//! Settings commands: /settings set_mode, /settings set_interval

use serenity::all::{
    CommandInteraction, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use std::sync::Arc;
use tracing::info;

use crate::database::{AnalysisMode, Database};

/// Register settings commands
pub fn register() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("settings")
            .description("Botの設定を変更します")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "set_mode",
                    "分析モードを変更します (debate / summary)",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "mode",
                        "分析モード",
                    )
                    .required(true)
                    .add_string_choice("debate", "debate")
                    .add_string_choice("summary", "summary"),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "set_interval",
                    "分析間隔（秒）を変更します",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "seconds",
                        "間隔（秒）",
                    )
                    .required(true)
                    .min_int_value(60)
                    .max_int_value(3600),
                ),
            ),
    ]
}

/// Handle /settings command
pub async fn handle(
    ctx: &Context,
    command: &CommandInteraction,
    db: Arc<Database>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let guild_id = command.guild_id.ok_or("Must be used in a guild")?;

    // Defer response to avoid "Unknown interaction" timeout
    command.defer(&ctx.http).await?;

    let options = &command.data.options();
    if options.is_empty() {
        respond(ctx, command, "サブコマンドを指定してください。").await?;
        return Ok(());
    }

    let subcommand = &options[0];
    let subcommand_name: &str = &subcommand.name;
    
    match subcommand_name {
        "set_mode" => {
            // Get the mode value from subcommand options
            if let serenity::all::ResolvedValue::SubCommand(sub_opts) = &subcommand.value {
                if let Some(mode_opt) = sub_opts.first() {
                    if let serenity::all::ResolvedValue::String(mode_str) = &mode_opt.value {
                        if let Some(mode) = AnalysisMode::from_str(mode_str) {
                            db.set_analysis_mode(guild_id.get(), mode)?;
                            respond(
                                ctx,
                                command,
                                &format!("✅ 分析モードを '{}' に変更しました。", mode.as_str()),
                            ).await?;
                            info!("Guild {} set mode to {}", guild_id, mode.as_str());
                        } else {
                            respond(
                                ctx,
                                command,
                                "❌ モードは 'debate' または 'summary' を指定してください。",
                            ).await?;
                        }
                    }
                }
            }
        }
        "set_interval" => {
            // Get the seconds value from subcommand options
            if let serenity::all::ResolvedValue::SubCommand(sub_opts) = &subcommand.value {
                if let Some(sec_opt) = sub_opts.first() {
                    if let serenity::all::ResolvedValue::Integer(seconds) = &sec_opt.value {
                        let seconds = *seconds as u64;
                        if seconds < 60 {
                            respond(
                                ctx,
                                command,
                                "❌ 間隔は最短60秒です。",
                            ).await?;
                        } else {
                            db.set_recording_interval(guild_id.get(), seconds)?;
                            respond(
                                ctx,
                                command,
                                &format!(
                                    "✅ 分析間隔を {}秒 ({:.1}分) に変更しました。",
                                    seconds,
                                    seconds as f64 / 60.0
                                ),
                            ).await?;
                            info!("Guild {} set interval to {}s", guild_id, seconds);
                        }
                    }
                }
            }
        }
        _ => {
            respond(ctx, command, "不明なサブコマンドです。").await?;
        }
    }

    Ok(())
}

/// Helper to send a response (using EditInteractionResponse since we deferred)
async fn respond(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
) -> Result<(), serenity::Error> {
    use serenity::all::EditInteractionResponse;
    
    command.edit_response(
        &ctx.http,
        EditInteractionResponse::new().content(content),
    ).await.map(|_| ())
}
