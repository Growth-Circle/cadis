//! Telegram bot adapter for C.A.D.I.S.

use serde::Serialize;

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

/// Adapter that bridges Telegram bot updates to the C.A.D.I.S. daemon.
pub struct TelegramAdapter {
    pub daemon_url: String,
    bot_token: String,
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
        }
    }

    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }
}

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
}
