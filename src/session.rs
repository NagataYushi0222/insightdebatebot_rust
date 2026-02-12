//! Session management for guild recording sessions
//!
//! Handles per-guild voice recording sessions with periodic analysis

use crate::analyzer::Analyzer;
use crate::audio::{AudioProcessor, UserRecorder};
use crate::config::Config;
use crate::database::{AnalysisMode, Database, GuildSettings};
use dashmap::DashMap;
use serenity::all::{ChannelId, CreateMessage, CreateThread, GuildId, Http, UserId};
use songbird::Call;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, error, info, warn};

/// A recording session for a single guild
pub struct GuildSession {
    /// Guild ID
    pub guild_id: GuildId,
    /// Text channel to post reports
    pub text_channel_id: ChannelId,
    /// Audio recorder
    pub recorder: Arc<UserRecorder>,
    /// Voice call handle
    pub call: Arc<tokio::sync::Mutex<Call>>,
    /// User ID to display name mapping
    pub user_names: DashMap<UserId, String>,
    /// Context from previous analysis
    pub last_context: RwLock<String>,
    /// Recording task handle
    pub task_handle: Option<JoinHandle<()>>,
    /// Whether the session is active
    pub is_active: bool,
}

impl GuildSession {
    /// Create a new guild session
    pub fn new(
        guild_id: GuildId,
        text_channel_id: ChannelId,
        call: Arc<tokio::sync::Mutex<Call>>,
        temp_dir: &PathBuf,
    ) -> Result<Self, crate::audio::recorder::RecorderError> {
        let recorder = Arc::new(UserRecorder::new(temp_dir)?);

        Ok(Self {
            guild_id,
            text_channel_id,
            recorder,
            call,
            user_names: DashMap::new(),
            last_context: RwLock::new(String::new()),
            task_handle: None,
            is_active: true,
        })
    }

    /// Register a user's display name
    pub fn register_user(&self, user_id: UserId, name: String) {
        self.user_names.insert(user_id, name);
    }

    /// Get user display name
    pub fn get_user_name(&self, user_id: &UserId) -> String {
        self.user_names
            .get(user_id)
            .map(|r| r.value().clone())
            .unwrap_or_else(|| format!("User_{}", user_id))
    }

    /// Start the periodic analysis loop
    pub fn start_analysis_loop(
        session: Arc<RwLock<GuildSession>>,
        http: Arc<Http>,
        analyzer: Arc<Analyzer>,
        db: Arc<Database>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                // Get current interval from settings
                let (guild_id, is_active);
                {
                    let session = session.read().await;
                    guild_id = session.guild_id;
                    is_active = session.is_active;
                }

                if !is_active {
                    break;
                }

                let settings = db.get_guild_settings(guild_id.get()).unwrap_or_default();
                let interval_secs = settings.recording_interval;

                // Wait for interval
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;

                // Check if still active
                {
                    let session = session.read().await;
                    if !session.is_active {
                        break;
                    }
                }

                // Perform analysis
                if let Err(e) = perform_analysis(
                    session.clone(),
                    http.clone(),
                    analyzer.clone(),
                    db.clone(),
                    false,
                ).await {
                    warn!("Periodic analysis failed: {}", e);
                }
            }
        })
    }

    /// Stop the session
    pub async fn stop(&mut self) {
        self.is_active = false;
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}

/// Perform analysis on the current audio buffer
pub async fn perform_analysis(
    session: Arc<RwLock<GuildSession>>,
    http: Arc<Http>,
    analyzer: Arc<Analyzer>,
    db: Arc<Database>,
    is_final: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (guild_id, text_channel_id, recorder, user_names);
    let context;
    
    {
        let session = session.read().await;
        guild_id = session.guild_id;
        text_channel_id = session.text_channel_id;
        recorder = session.recorder.clone();
        user_names = session.user_names.clone();
        context = session.last_context.read().await.clone();
    }

    info!("[{}] Starting analysis (Final: {})", guild_id, is_final);

    // Flush audio to files
    let audio_files = match recorder.flush_audio().await {
        Ok(files) => files,
        Err(e) => {
            if is_final {
                info!("[{}] No audio to analyze for final report", guild_id);
            }
            return Err(Box::new(e));
        }
    };

    if audio_files.is_empty() {
        return Ok(());
    }

    // Convert WAV files to MP3 for better Gemini API compatibility
    let mut mp3_files: HashMap<UserId, PathBuf> = HashMap::new();
    for (user_id, wav_path) in &audio_files {
        match AudioProcessor::convert_to_mp3(wav_path).await {
            Ok(mp3_path) => {
                mp3_files.insert(*user_id, mp3_path);
            }
            Err(e) => {
                warn!("Failed to convert audio for user {}: {}", user_id, e);
                // Clean up the WAV file that failed conversion
                AudioProcessor::cleanup_files(&[wav_path.clone()]);
            }
        }
    }

    if mp3_files.is_empty() {
        return Ok(());
    }

    // Ensure MP3 files are always cleaned up, even if analysis or posting fails
    let result = perform_analysis_and_post(
        &mp3_files, &session, &http, &analyzer, &db, &user_names, &context, guild_id, text_channel_id, is_final,
    ).await;

    // Always clean up MP3 files
    let files_to_cleanup: Vec<PathBuf> = mp3_files.values().cloned().collect();
    AudioProcessor::cleanup_files(&files_to_cleanup);

    result
}

/// Inner function that performs analysis and posts results
/// Separated so that cleanup always happens in the caller
async fn perform_analysis_and_post(
    mp3_files: &HashMap<UserId, PathBuf>,
    session: &Arc<RwLock<GuildSession>>,
    http: &Arc<Http>,
    analyzer: &Arc<Analyzer>,
    db: &Arc<Database>,
    user_names: &DashMap<UserId, String>,
    context: &str,
    guild_id: GuildId,
    text_channel_id: ChannelId,
    is_final: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Build user map
    let user_map: HashMap<UserId, String> = mp3_files
        .keys()
        .map(|&uid| {
            let name = user_names
                .get(&uid)
                .map(|r| r.value().clone())
                .unwrap_or_else(|| format!("User_{}", uid));
            (uid, name)
        })
        .collect();

    // Get settings
    let settings = db.get_guild_settings(guild_id.get()).unwrap_or_default();
    let mode = settings.analysis_mode;

    // Send "analyzing" message
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
    let starter_text = if is_final {
        format!("🛑 **セッション終了** ({})", timestamp)
    } else {
        format!("📅 **自動分析** ({})", timestamp)
    };

    let starter_msg = text_channel_id.send_message(&http, CreateMessage::new().content(&starter_text)).await?;

    // Create thread for report
    let thread_name = if is_final {
        format!("議論分析レポート (最終) {}", timestamp)
    } else {
        format!("議論分析レポート {}", timestamp)
    };

    let thread_builder = CreateThread::new(thread_name);
    let thread = text_channel_id.create_thread_from_message(&http, starter_msg.id, thread_builder).await?;

    let analyzing_msg = CreateMessage::new()
        .content(format!("🔄 音声ファイルを分析中... (Mode: {})", mode.as_str()));
    thread.send_message(&http, analyzing_msg).await?;

    // Run analysis
    let report = match analyzer.analyze_discussion(mp3_files.clone(), &context, user_map, mode).await {
        Ok(r) => r,
        Err(crate::analyzer::AnalyzerError::RateLimitExceeded) => {
            "⚠️ 分析のリクエスト制限（Quota Limit）に達しました。".to_string()
        }
        Err(e) => {
            format!("分析中にエラーが発生しました: {}", e)
        }
    };

    // Update context
    {
        let session = session.read().await;
        let mut last_context = session.last_context.write().await;
        *last_context = if report.len() > 2000 {
            report[report.len()-2000..].to_string()
        } else {
            report.clone()
        };
    }

    // Post report
    let header = if is_final {
        "🏁 **最終分析レポート**\n"
    } else {
        "📊 **議論分析レポート**\n"
    };

    if report.len() + header.len() < 2000 {
        let msg = CreateMessage::new().content(format!("{}{}", header, report));
        thread.send_message(&http, msg).await?;
    } else {
        thread.send_message(&http, CreateMessage::new().content(header)).await?;
        for chunk in report.as_bytes().chunks(1900) {
            let chunk_str = String::from_utf8_lossy(chunk);
            thread.send_message(&http, CreateMessage::new().content(&*chunk_str)).await?;
        }
    }

    Ok(())
}

/// Session manager for all guilds
pub struct SessionManager {
    sessions: DashMap<GuildId, Arc<RwLock<GuildSession>>>,
    config: Arc<Config>,
    db: Arc<Database>,
    analyzer: Arc<Analyzer>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(config: Arc<Config>, db: Arc<Database>) -> Self {
        let analyzer = Arc::new(Analyzer::new(config.gemini_api_key.clone()));
        
        Self {
            sessions: DashMap::new(),
            config,
            db,
            analyzer,
        }
    }

    /// Get or create a session for a guild
    pub fn get_session(&self, guild_id: GuildId) -> Option<Arc<RwLock<GuildSession>>> {
        self.sessions.get(&guild_id).map(|r| r.value().clone())
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        guild_id: GuildId,
        text_channel_id: ChannelId,
        call: Arc<tokio::sync::Mutex<Call>>,
    ) -> Result<Arc<RwLock<GuildSession>>, crate::audio::recorder::RecorderError> {
        let session = GuildSession::new(
            guild_id,
            text_channel_id,
            call.clone(),
            &self.config.temp_audio_dir,
        )?;
        
        // Attach event handler to the voice call
        {
            let mut handler = call.lock().await;
            
            // Enable audio decoding: Opus → PCM i16 (mono, 16kHz)
            handler.set_config(
                songbird::Config::default()
                    .decode_mode(songbird::driver::DecodeMode::Decode)
                    .decode_channels(songbird::driver::Channels::Mono)
                    .decode_sample_rate(songbird::driver::SampleRate::Hz16000)
            );
            
            handler.add_global_event(
                songbird::CoreEvent::VoiceTick.into(),
                crate::bot::VoiceReceiver {
                    recorder: session.recorder.clone(),
                },
            );
            handler.add_global_event(
                songbird::CoreEvent::SpeakingStateUpdate.into(),
                crate::bot::VoiceReceiver {
                    recorder: session.recorder.clone(),
                },
            );
        }

        let session = Arc::new(RwLock::new(session));
        self.sessions.insert(guild_id, session.clone());
        
        Ok(session)
    }

    /// Start analysis loop for a session
    pub fn start_analysis_loop(&self, guild_id: GuildId, http: Arc<Http>) {
        if let Some(session) = self.get_session(guild_id) {
            let handle = GuildSession::start_analysis_loop(
                session.clone(),
                http,
                self.analyzer.clone(),
                self.db.clone(),
            );
            
            // Store handle
            tokio::spawn(async move {
                let mut session = session.write().await;
                session.task_handle = Some(handle);
            });
        }
    }

    /// Force analysis for a session
    pub async fn force_analysis(&self, guild_id: GuildId, http: Arc<Http>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(session) = self.get_session(guild_id) {
            perform_analysis(session, http, self.analyzer.clone(), self.db.clone(), false).await
        } else {
            Err("Session not found".into())
        }
    }

    /// Stop and cleanup a session
    pub async fn cleanup_session(&self, guild_id: GuildId, http: Arc<Http>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some((_, session)) = self.sessions.remove(&guild_id) {
            // Run final analysis
            perform_analysis(session.clone(), http, self.analyzer.clone(), self.db.clone(), true).await?;
            
            // Stop session
            let mut session = session.write().await;
            session.stop().await;
        }
        
        Ok(())
    }
}
