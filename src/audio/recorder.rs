//! Per-user audio recorder with PCM recording
//!
//! Records Discord voice audio as decoded PCM and saves as WAV files

use dashmap::DashMap;
use serenity::model::id::UserId;
use songbird::events::context_data::VoiceTick;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Audio configuration constants
/// These must match the songbird Config (DecodeMode::Decode with Mono/16kHz)
const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;

#[derive(Error, Debug)]
pub enum RecorderError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("No audio data available")]
    NoData,
}

/// Per-user audio buffer storing decoded PCM samples
struct UserAudioBuffer {
    /// Decoded PCM i16 samples (mono, 16kHz)
    pcm_samples: Vec<i16>,
    /// Timestamp when recording started for this user
    start_time: u64,
}

impl UserAudioBuffer {
    fn new() -> Self {
        Self {
            pcm_samples: Vec::new(),
            start_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    fn add_samples(&mut self, samples: &[i16]) {
        self.pcm_samples.extend_from_slice(samples);
    }

    fn is_empty(&self) -> bool {
        self.pcm_samples.is_empty()
    }

    fn take_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.pcm_samples)
    }
}

/// User-specific audio recorder
///
/// Collects decoded PCM audio from Discord and saves as WAV files
pub struct UserRecorder {
    /// Per-user audio buffers
    user_buffers: DashMap<UserId, UserAudioBuffer>,
    /// Temporary audio directory
    temp_dir: PathBuf,
    /// Session timestamp for unique filenames
    session_timestamp: u64,
}

impl UserRecorder {
    /// Create a new recorder
    pub fn new<P: AsRef<Path>>(temp_dir: P) -> Result<Self, RecorderError> {
        let temp_dir = temp_dir.as_ref().to_path_buf();
        
        // Ensure temp directory exists
        fs::create_dir_all(&temp_dir)?;
        
        let session_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(Self {
            user_buffers: DashMap::new(),
            temp_dir,
            session_timestamp,
        })
    }

    /// Process incoming voice tick from Songbird
    ///
    /// Uses decoded PCM samples (requires DecodeMode::Decode)
    pub fn process_voice_tick(&self, tick: &VoiceTick) {
        for (ssrc, data) in &tick.speaking {
            // Use SSRC as temporary User ID (u32 -> u64)
            let user_id = UserId::new(*ssrc as u64);

            // Use decoded PCM voice data (available with DecodeMode::Decode)
            if let Some(decoded) = &data.decoded_voice {
                if !decoded.is_empty() {
                    self.add_pcm_samples(user_id, decoded);
                }
            }
        }
    }

    /// Add decoded PCM samples for a specific user
    pub fn add_pcm_samples(&self, user_id: UserId, samples: &[i16]) {
        if samples.is_empty() {
            return;
        }
        
        let mut entry = self.user_buffers.entry(user_id).or_insert_with(UserAudioBuffer::new);
        entry.add_samples(samples);
        
        // Print a dot for activity (like Python version)
        print!(".");
        std::io::stdout().flush().ok();
    }

    /// Flush all user audio to WAV files
    ///
    /// Returns a map of user_id -> file_path for saved audio
    pub async fn flush_audio(&self) -> Result<HashMap<UserId, PathBuf>, RecorderError> {
        let mut saved_files = HashMap::new();
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Collect user IDs to process
        let user_ids: Vec<UserId> = self.user_buffers.iter().map(|r| *r.key()).collect();

        for user_id in user_ids {
            if let Some((_, mut buffer)) = self.user_buffers.remove(&user_id) {
                if buffer.is_empty() {
                    continue;
                }

                let samples = buffer.take_samples();
                let filename = format!(
                    "{}_{}_{}", 
                    self.session_timestamp,
                    user_id.get(),
                    current_time
                );
                
                // Save as WAV file
                match self.save_wav_audio(&filename, &samples) {
                    Ok(path) => {
                        info!("Saved {} PCM samples for user {} to {:?}", samples.len(), user_id, path);
                        saved_files.insert(user_id, path);
                    }
                    Err(e) => {
                        error!("Failed to save audio for user {}: {}", user_id, e);
                    }
                }
            }
        }

        if saved_files.is_empty() {
            return Err(RecorderError::NoData);
        }

        Ok(saved_files)
    }

    /// Save PCM samples as a WAV file
    fn save_wav_audio(&self, filename: &str, samples: &[i16]) -> Result<PathBuf, RecorderError> {
        let wav_path = self.temp_dir.join(format!("{}.wav", filename));
        
        let mut file = File::create(&wav_path)?;
        
        let data_size = (samples.len() * 2) as u32; // i16 = 2 bytes each
        let byte_rate = SAMPLE_RATE * CHANNELS as u32 * BITS_PER_SAMPLE as u32 / 8;
        let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
        
        // RIFF header
        file.write_all(b"RIFF")?;
        file.write_all(&(36 + data_size).to_le_bytes())?;
        file.write_all(b"WAVE")?;
        
        // fmt sub-chunk
        file.write_all(b"fmt ")?;
        file.write_all(&16u32.to_le_bytes())?;          // Sub-chunk size (16 for PCM)
        file.write_all(&1u16.to_le_bytes())?;            // Audio format (1 = PCM)
        file.write_all(&CHANNELS.to_le_bytes())?;        // Channels
        file.write_all(&SAMPLE_RATE.to_le_bytes())?;     // Sample rate
        file.write_all(&byte_rate.to_le_bytes())?;       // Byte rate
        file.write_all(&block_align.to_le_bytes())?;     // Block align
        file.write_all(&BITS_PER_SAMPLE.to_le_bytes())?; // Bits per sample
        
        // data sub-chunk
        file.write_all(b"data")?;
        file.write_all(&data_size.to_le_bytes())?;
        
        // Write PCM samples as little-endian i16
        for &sample in samples {
            file.write_all(&sample.to_le_bytes())?;
        }
        
        info!("Saved WAV file: {:?} ({} samples, {:.1}s)", 
            wav_path, samples.len(), 
            samples.len() as f64 / SAMPLE_RATE as f64);
        
        Ok(wav_path)
    }

    /// Clear all buffers without saving
    pub fn clear(&self) {
        self.user_buffers.clear();
    }

    /// Check if there's any audio data buffered
    pub fn has_data(&self) -> bool {
        self.user_buffers.iter().any(|r| !r.value().is_empty())
    }

    /// Get the number of users currently being recorded
    pub fn user_count(&self) -> usize {
        self.user_buffers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_recorder_basic() {
        let temp = tempdir().unwrap();
        let recorder = UserRecorder::new(temp.path()).unwrap();
        
        let user_id = UserId::new(12345);
        // Add PCM samples instead of raw Opus
        recorder.add_pcm_samples(user_id, &[100, -100, 200, -200]);
        recorder.add_pcm_samples(user_id, &[300, -300, 400, -400]);
        
        assert!(recorder.has_data());
        assert_eq!(recorder.user_count(), 1);
        
        let files = recorder.flush_audio().await.unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains_key(&user_id));
        
        // Verify the file ends in .wav
        let path = files.get(&user_id).unwrap();
        assert_eq!(path.extension().unwrap(), "wav");
    }
}
