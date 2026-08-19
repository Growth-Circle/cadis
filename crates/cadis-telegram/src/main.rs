//! CADIS Telegram bot binary entry point.
//!
//! This binary starts a Telegram bot that forwards commands to the CADIS daemon.
//! It requires a bot token from @BotFather and connects to a running cadisd instance.
//!
//! # Usage
//!
//! ```bash
//! # Set required environment variables
//! export CADIS_TELEGRAM_BOT_TOKEN="your_bot_token_from_botfather"
//! export CADIS_TELEGRAM_ALLOWED_CHATS="123456789,987654321"  # Optional: comma-separated chat IDs
//! export CADIS_DAEMON_URL="127.0.0.1:7433"  # Optional: defaults to 127.0.0.1:7433
//!
//! # Start the bot
//! cadis-telegram
//! ```
//!
//! # Security
//!
//! - Store bot token in `CADIS_TELEGRAM_BOT_TOKEN` environment variable, never commit it
//! - Use `CADIS_TELEGRAM_ALLOWED_CHATS` to restrict bot access to specific chat IDs
//! - If allowed_chats is empty, the bot accepts commands from ANY chat (not recommended)

use cadis_telegram::{bot_token_security_note, DaemonBridge, TelegramAdapter};
use std::env;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Print security note on startup
    eprintln!("{}", bot_token_security_note());
    eprintln!();

    // Load bot token from environment (required)
    let bot_token = env::var("CADIS_TELEGRAM_BOT_TOKEN").map_err(|_| {
        "CADIS_TELEGRAM_BOT_TOKEN environment variable is required.\n\
         Get a bot token from @BotFather on Telegram.\n\
         Example: export CADIS_TELEGRAM_BOT_TOKEN=\"123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11\""
    })?;

    // Load daemon URL (defaults to localhost TCP)
    let daemon_url = env::var("CADIS_DAEMON_URL").unwrap_or_else(|_| "127.0.0.1:7433".to_string());

    // Load allowed chat IDs (optional: empty = allow all)
    let allowed_chat_ids: Vec<i64> = env::var("CADIS_TELEGRAM_ALLOWED_CHATS")
        .ok()
        .and_then(|s| {
            if s.trim().is_empty() {
                None
            } else {
                Some(
                    s.split(',')
                        .filter_map(|id| id.trim().parse::<i64>().ok())
                        .collect(),
                )
            }
        })
        .unwrap_or_default();

    if allowed_chat_ids.is_empty() {
        eprintln!("⚠️  WARNING: CADIS_TELEGRAM_ALLOWED_CHATS is not set.");
        eprintln!("⚠️  Bot will accept commands from ANY Telegram chat.");
        eprintln!("⚠️  Set CADIS_TELEGRAM_ALLOWED_CHATS=\"chat_id1,chat_id2\" to restrict access.");
        eprintln!();
    } else {
        eprintln!("✓ Allowed chat IDs: {:?}", allowed_chat_ids);
    }

    eprintln!("✓ Daemon URL: {}", daemon_url);
    eprintln!(
        "✓ Bot token: {}...{}",
        &bot_token[..8],
        &bot_token[bot_token.len() - 4..]
    );
    eprintln!();
    eprintln!("Starting CADIS Telegram bot...");
    eprintln!("Send /status to your bot to verify it's working.");
    eprintln!();

    // Create adapter and bridge
    let adapter = TelegramAdapter::new(daemon_url.clone(), bot_token, allowed_chat_ids);
    let bridge = DaemonBridge::new(daemon_url);

    // Start polling loop (blocks forever)
    adapter.poll_and_bridge(&bridge).await;

    Ok(())
}
