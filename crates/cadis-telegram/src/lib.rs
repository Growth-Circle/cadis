//! Telegram bot adapter for C.A.D.I.S.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from Telegram Bot API interactions.
#[derive(Debug)]
pub enum TelegramError {
    Http(reqwest::Error),
    Api(String),
}

impl fmt::Display for TelegramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "telegram http error: {e}"),
            Self::Api(msg) => write!(f, "telegram api error: {msg}"),
        }
    }
}

impl std::error::Error for TelegramError {}

impl From<reqwest::Error> for TelegramError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

// ---------------------------------------------------------------------------
// Telegram API types (minimal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub data: Option<String>,
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Commands parsed from Telegram bot updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramCommand {
    Status,
    Agents,
    Workers,
    Spawn { agent_name: String },
    Approve { approval_id: String },
    Deny { approval_id: String },
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Adapter that bridges Telegram bot updates to the C.A.D.I.S. daemon.
pub struct TelegramAdapter {
    pub daemon_url: String,
    bot_token: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for TelegramAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramAdapter")
            .field("daemon_url", &self.daemon_url)
            .field("bot_token", &"[REDACTED]")
            .finish()
    }
}

impl TelegramAdapter {
    pub fn new(daemon_url: String, bot_token: String) -> Self {
        Self {
            daemon_url,
            bot_token,
            client: reqwest::Client::new(),
        }
    }

    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    /// Build a Telegram Bot API URL for the given method.
    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.bot_token)
    }

    /// Long-poll for updates from the Telegram Bot API.
    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>, TelegramError> {
        let resp: ApiResponse<Vec<Update>> = self
            .client
            .get(self.api_url("getUpdates"))
            .query(&[("offset", offset), ("timeout", 30)])
            .send()
            .await?
            .json()
            .await?;
        if resp.ok {
            Ok(resp.result.unwrap_or_default())
        } else {
            Err(TelegramError::Api(
                resp.description.unwrap_or_else(|| "unknown error".into()),
            ))
        }
    }

    /// Send a plain text message.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), TelegramError> {
        let resp: ApiResponse<serde_json::Value> = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
            .send()
            .await?
            .json()
            .await?;
        if resp.ok {
            Ok(())
        } else {
            Err(TelegramError::Api(
                resp.description.unwrap_or_else(|| "unknown error".into()),
            ))
        }
    }

    /// Send a message with an inline keyboard (pre-serialized JSON).
    pub async fn send_message_with_keyboard(
        &self,
        chat_id: i64,
        text: &str,
        keyboard_json: &str,
    ) -> Result<(), TelegramError> {
        let keyboard: serde_json::Value =
            serde_json::from_str(keyboard_json).map_err(|e| TelegramError::Api(e.to_string()))?;
        let resp: ApiResponse<serde_json::Value> = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "reply_markup": keyboard,
            }))
            .send()
            .await?
            .json()
            .await?;
        if resp.ok {
            Ok(())
        } else {
            Err(TelegramError::Api(
                resp.description.unwrap_or_else(|| "unknown error".into()),
            ))
        }
    }

    /// Acknowledge a callback query so the Telegram client stops showing a spinner.
    pub async fn answer_callback_query(
        &self,
        callback_query_id: &str,
    ) -> Result<(), TelegramError> {
        let resp: ApiResponse<bool> = self
            .client
            .post(self.api_url("answerCallbackQuery"))
            .json(&serde_json::json!({ "callback_query_id": callback_query_id }))
            .send()
            .await?
            .json()
            .await?;
        if resp.ok {
            Ok(())
        } else {
            Err(TelegramError::Api(
                resp.description.unwrap_or_else(|| "unknown error".into()),
            ))
        }
    }

    /// Simple long-polling loop. Calls `handler` for each parsed command with its chat_id.
    pub async fn poll_loop(&self, handler: impl Fn(TelegramCommand, i64)) {
        let mut offset: i64 = 0;
        loop {
            let updates = match self.get_updates(offset).await {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("cadis-telegram poll error: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            for update in &updates {
                offset = update.update_id + 1;
                let (text, chat_id) = if let Some(msg) = &update.message {
                    (msg.text.as_deref(), msg.chat.id)
                } else if let Some(cb) = &update.callback_query {
                    (
                        cb.data.as_deref(),
                        cb.message.as_ref().map(|m| m.chat.id).unwrap_or(0),
                    )
                } else {
                    continue;
                };
                if let Some(text) = text {
                    if let Some(cmd) = handle_update(text) {
                        handler(cmd, chat_id);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command parser (existing)
// ---------------------------------------------------------------------------

/// Parse a Telegram update text into a command.
pub fn handle_update(update: &str) -> Option<TelegramCommand> {
    let text = update.trim();
    let mut parts = text.splitn(2, ' ');
    let cmd = parts.next()?;
    let arg = parts.next().map(|s| s.trim().to_string());

    match cmd {
        "/status" => Some(TelegramCommand::Status),
        "/agents" => Some(TelegramCommand::Agents),
        "/workers" => Some(TelegramCommand::Workers),
        "/spawn" => Some(TelegramCommand::Spawn { agent_name: arg? }),
        "/approve" => Some(TelegramCommand::Approve { approval_id: arg? }),
        "/deny" => Some(TelegramCommand::Deny { approval_id: arg? }),
        _ => None,
    }
}

/// Inline keyboard JSON for approve/deny buttons.
pub fn format_approval_buttons(approval_id: &str) -> String {
    #[derive(Serialize)]
    struct InlineKeyboard {
        inline_keyboard: Vec<Vec<Button>>,
    }
    #[derive(Serialize)]
    struct Button {
        text: String,
        callback_data: String,
    }

    let kb = InlineKeyboard {
        inline_keyboard: vec![vec![
            Button {
                text: "✅ Approve".into(),
                callback_data: format!("/approve {approval_id}"),
            },
            Button {
                text: "❌ Deny".into(),
                callback_data: format!("/deny {approval_id}"),
            },
        ]],
    };
    serde_json::to_string(&kb).expect("serialization cannot fail")
}

/// Security guidance for bot token handling.
pub fn bot_token_security_note() -> &'static str {
    "Store the Telegram bot token in CADIS_TELEGRAM_BOT_TOKEN or a secrets manager. \
     Never commit tokens to version control or include them in logs."
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_commands() {
        assert_eq!(handle_update("/status"), Some(TelegramCommand::Status));
        assert_eq!(handle_update("/agents"), Some(TelegramCommand::Agents));
        assert_eq!(handle_update("/workers"), Some(TelegramCommand::Workers));
    }

    #[test]
    fn parse_commands_with_args() {
        assert_eq!(
            handle_update("/spawn coder"),
            Some(TelegramCommand::Spawn {
                agent_name: "coder".into()
            })
        );
        assert_eq!(
            handle_update("/approve abc-123"),
            Some(TelegramCommand::Approve {
                approval_id: "abc-123".into()
            })
        );
        assert_eq!(
            handle_update("/deny abc-123"),
            Some(TelegramCommand::Deny {
                approval_id: "abc-123".into()
            })
        );
    }

    #[test]
    fn missing_arg_returns_none() {
        assert_eq!(handle_update("/spawn"), None);
        assert_eq!(handle_update("/approve"), None);
        assert_eq!(handle_update("/deny"), None);
    }

    #[test]
    fn unknown_command_returns_none() {
        assert_eq!(handle_update("/help"), None);
        assert_eq!(handle_update("hello"), None);
    }

    #[test]
    fn approval_buttons_valid_json() {
        let json = format_approval_buttons("req-42");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let kb = v["inline_keyboard"].as_array().expect("array");
        assert_eq!(kb.len(), 1);
        assert_eq!(kb[0].as_array().expect("row").len(), 2);
        assert!(kb[0][0]["callback_data"]
            .as_str()
            .unwrap()
            .contains("req-42"));
    }

    #[test]
    fn debug_redacts_token() {
        let adapter = TelegramAdapter::new("http://localhost".into(), "secret-token".into());
        let dbg = format!("{:?}", adapter);
        assert!(!dbg.contains("secret-token"));
        assert!(dbg.contains("REDACTED"));
    }

    #[test]
    fn security_note_not_empty() {
        assert!(!bot_token_security_note().is_empty());
    }

    // --- New tests: URL construction ---

    #[test]
    fn api_url_get_updates() {
        let adapter = TelegramAdapter::new("http://localhost".into(), "123:ABC".into());
        assert_eq!(
            adapter.api_url("getUpdates"),
            "https://api.telegram.org/bot123:ABC/getUpdates"
        );
    }

    #[test]
    fn api_url_send_message() {
        let adapter = TelegramAdapter::new("http://localhost".into(), "tok".into());
        assert_eq!(
            adapter.api_url("sendMessage"),
            "https://api.telegram.org/bottok/sendMessage"
        );
    }

    #[test]
    fn api_url_answer_callback() {
        let adapter = TelegramAdapter::new("http://localhost".into(), "my-token".into());
        assert_eq!(
            adapter.api_url("answerCallbackQuery"),
            "https://api.telegram.org/botmy-token/answerCallbackQuery"
        );
    }

    #[test]
    fn api_url_uses_bot_token_accessor() {
        let adapter = TelegramAdapter::new("http://localhost".into(), "secret".into());
        let url = adapter.api_url("getMe");
        assert!(url.contains(adapter.bot_token()));
        assert!(url.starts_with("https://api.telegram.org/bot"));
        assert!(url.ends_with("/getMe"));
    }
}
