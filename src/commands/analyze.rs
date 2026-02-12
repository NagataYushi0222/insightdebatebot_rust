//! Analyze commands: /analyze_start, /analyze_stop, /analyze_now

use serenity::all::{
    CommandInteraction, Context, CreateCommand, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditInteractionResponse, CreateMessage,
};
use std::sync::Arc;
use tracing::info;

use crate::session::SessionManager;

/// Register analyze commands
pub fn register() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("analyze_start")
            .description("ボイスチャットの分析を開始します"),
        CreateCommand::new("analyze_stop")
            .description("分析を終了し、ボイスチャットから退出します"),
        CreateCommand::new("analyze_now")
            .description("すぐにレポートを作成します（分析間隔を待たずに実行）"),
        CreateCommand::new("analyze_debug")
            .description("Botの権限と状態を確認するデバッグコマンド"),
    ]
}

/// Handle /analyze_debug command
pub async fn handle_debug(
    ctx: &Context,
    command: &CommandInteraction,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    command.defer(&ctx.http).await?;

    let guild_id = command.guild_id.ok_or("Must be used in a guild")?;
    let bot_user = ctx.http.get_current_user().await?;
    
    let mut info = format!("Debug Info:\nBot: {} ({})\nGuild: {}\n", bot_user.name, bot_user.id, guild_id);

    // Get user's voice channel
    let maybe_channel_id = ctx.cache.guild(guild_id)
        .and_then(|g| g.voice_states.get(&command.user.id).and_then(|vs| vs.channel_id));

    if let Some(channel_id) = maybe_channel_id {
        info.push_str(&format!("Target Voice Channel: {}\n", channel_id));
        
        // Check permissions
        // Note: permissions_in requires Guild cache. We use guild-level perms for now or try to get it.
        // First get member (async)
        if let Ok(member) = ctx.http.get_member(guild_id, bot_user.id).await {
            if let Some(guild) = ctx.cache.guild(guild_id) {
                 // Calculate permissions (no await here)
                 if let Ok(perms) = guild.user_permissions_in(channel_id, &member) {
                     info.push_str(&format!("Permissions in channel:\n"));
                     info.push_str(&format!("  - CONNECT: {}\n", perms.contains(serenity::model::permissions::Permissions::CONNECT)));
                     info.push_str(&format!("  - SPEAK: {}\n", perms.contains(serenity::model::permissions::Permissions::SPEAK)));
                     info.push_str(&format!("  - ADMINISTRATOR: {}\n", perms.contains(serenity::model::permissions::Permissions::ADMINISTRATOR)));
                 } else {
                     info.push_str("Could not calculate channel permissions (user_permissions_in failed).\n");
                 }
            } else {
                 info.push_str("Guild not in cache.\n");
            }
        } else {
             info.push_str("Could not fetch Bot Member from API.\n");
        }
    } else {
        info.push_str("User is NOT in a voice channel. Please join one.\n");
    }
    
    respond_edit(ctx, command, &format!("```\n{}\n```", info)).await?;
    Ok(())
}

/// Handle /analyze_start command
pub async fn handle_start(
    ctx: &Context,
    command: &CommandInteraction,
    session_manager: Arc<SessionManager>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Defer immediately to avoid "Unknown interaction"
    command.defer(&ctx.http).await?;

    let guild_id = command.guild_id.ok_or("Must be used in a guild")?;
    
    // Get user's voice channel from guild cache
    let maybe_channel_id = {
        let guild = ctx.cache.guild(guild_id).ok_or("Guild not in cache")?;
        guild
            .voice_states
            .get(&command.user.id)
            .and_then(|vs| vs.channel_id)
    };

    let voice_channel_id = match maybe_channel_id {
        Some(id) => id,
        None => {
            respond_edit(ctx, command, "ボイスチャットに参加してからコマンドを実行してください。").await?;
            return Ok(());
        }
    };

    // Check if already recording
    if session_manager.get_session(guild_id).is_some() {
        respond_edit(ctx, command, "既に分析を実行中です。").await?;
        return Ok(());
    }

    // Get songbird manager
    let manager = songbird::get(ctx).await.ok_or("Songbird not registered")?;

    // Force leave if already connected (Handling ghost connections)
    if manager.get(guild_id).is_some() {
        let _ = manager.remove(guild_id).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Join voice channel
    let call = manager.join(guild_id, voice_channel_id).await?;

    // Create session
    let _session_arc = session_manager.create_session(guild_id, command.channel_id, call).await?;
    
    // Register all users currently in the voice channel
    /* TEMPORARILY DISABLED FOR DEBUGGING
    {
        // 1. Collect member data first (CacheRef is not Send, so can't be held across await)
        let members_to_register: Vec<(u64, String)> = {
            let guild = ctx.cache.guild(guild_id).ok_or("Guild not in cache")?;
            
            guild.voice_states.iter()
                .filter(|(_, vs)| vs.channel_id == Some(voice_channel_id))
                .map(|(user_id, _)| {
                    // Use nickname (server display name) if available, else global name
                    let name = guild.members.get(user_id)
                        .map(|m| m.display_name().to_string())
                        .unwrap_or_else(|| format!("User_{}", user_id));
                    (user_id.get(), name)
                })
                .collect()
        };

        // 2. Register users (now safe to await)
        let session = session_arc.read().await;
        for (user_id_u64, name) in members_to_register {
            session.register_user(serenity::model::id::UserId::new(user_id_u64), name);
        }
    }
    */
    
    // Start analysis loop
    session_manager.start_analysis_loop(guild_id, ctx.http.clone());

    // Get channel name for response
    let channel_name = ctx.cache.channel(voice_channel_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let response = EditInteractionResponse::new()
        .content(format!(
            "{} の分析を開始しました。プライバシー保護のため、録音・分析が行われることを参加者に周知してください。",
            channel_name
        ));
    command.edit_response(&ctx.http, response).await?;

    info!("Started recording in guild {} channel {}", guild_id, voice_channel_id);
    Ok(())
}

/// Handle /analyze_stop command
pub async fn handle_stop(
    ctx: &Context,
    command: &CommandInteraction,
    session_manager: Arc<SessionManager>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Defer immediately
    command.defer(&ctx.http).await?;

    let guild_id = command.guild_id.ok_or("Must be used in a guild")?;

    // Check if recording
    if session_manager.get_session(guild_id).is_none() {
        respond_edit(ctx, command, "分析は実行されていません。").await?;
        return Ok(());
    }

    respond_edit(ctx, command, "🔄 最終レポートを作成して終了します。しばらくお待ちください...").await?;

    // Cleanup session (runs final analysis)
    session_manager.cleanup_session(guild_id, ctx.http.clone()).await?;

    // Leave voice channel
    let manager = songbird::get(ctx).await.ok_or("Songbird not registered")?;
    let _ = manager.leave(guild_id).await;

    let msg = CreateMessage::new().content("✅ 分析を終了しました。お疲れ様でした！");
    command.channel_id.send_message(&ctx.http, msg).await?;

    info!("Stopped recording in guild {}", guild_id);
    Ok(())
}

/// Handle /analyze_now command
pub async fn handle_now(
    ctx: &Context,
    command: &CommandInteraction,
    session_manager: Arc<SessionManager>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Defer immediately
    command.defer(&ctx.http).await?;
    
    let guild_id = command.guild_id.ok_or("Must be used in a guild")?;

    // Check if recording
    if session_manager.get_session(guild_id).is_none() {
        respond_edit(ctx, command, "分析は実行されていません。先に /analyze_start を実行してください。").await?;
        return Ok(());
    }

    respond_edit(ctx, command, "🔄 手動分析を開始しました...").await?;

    // Force analysis
    if let Err(e) = session_manager.force_analysis(guild_id, ctx.http.clone()).await {
        let msg = CreateMessage::new().content(format!("⚠️ エラー: {}", e));
        command.channel_id.send_message(&ctx.http, msg).await?;
    }

    Ok(())
}

/// Helper to send a deferred response
async fn respond_edit(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
) -> Result<(), serenity::Error> {
    command.edit_response(&ctx.http, EditInteractionResponse::new().content(content)).await?;
    Ok(())
}
