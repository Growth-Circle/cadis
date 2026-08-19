# cadis-telegram

Telegram bot adapter for C.A.D.I.S. Routes Telegram commands to the daemon protocol and formats approval actions as inline keyboard buttons.

## Usage as Library

```rust
use cadis_telegram::{TelegramAdapter, DaemonBridge};

let adapter = TelegramAdapter::new(
    "127.0.0.1:7433".to_string(),
    "your_bot_token".to_string(),
    vec![123456789], // allowed chat IDs
);
let bridge = DaemonBridge::new("127.0.0.1:7433".to_string());
adapter.poll_and_bridge(&bridge).await;
```

## Usage as Binary

The `cadis-telegram` binary provides a standalone Telegram bot that connects to a running CADIS daemon.

### Installation

```bash
# Build from source
cargo build --release -p cadis-telegram

# Binary will be at target/release/cadis-telegram
```

### Configuration

Set required environment variables:

```bash
# Required: Bot token from @BotFather
export CADIS_TELEGRAM_BOT_TOKEN="123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"

# Optional: Restrict bot to specific Telegram chat IDs (recommended)
export CADIS_TELEGRAM_ALLOWED_CHATS="123456789,987654321"

# Optional: Daemon URL (defaults to 127.0.0.1:7433)
export CADIS_DAEMON_URL="127.0.0.1:7433"
```

### Running

```bash
# Start CADIS daemon first
cadisd

# Then start the Telegram bot
cadis-telegram
```

The bot will start polling for updates and forward commands to the daemon.

### Supported Commands

- `/status` - Check daemon status
- `/agents` - List running agents
- `/workers` - List active workers
- `/spawn <agent_name>` - Spawn a new agent
- `/approve <approval_id>` - Approve a pending action
- `/deny <approval_id>` - Deny a pending action
- Any other text - Send as chat message to daemon

### Security

⚠️ **Important Security Notes**:

- **Never commit bot tokens** - Always use environment variables
- **Restrict chat access** - Set `CADIS_TELEGRAM_ALLOWED_CHATS` to specific chat IDs
- **Use private chats** - Avoid adding bot to public groups
- **Token storage** - Store tokens securely (environment variables, secrets manager)

If `CADIS_TELEGRAM_ALLOWED_CHATS` is not set, the bot will accept commands from ANY Telegram chat. This is not recommended for production use.

### Example Deployment

```bash
#!/bin/bash
# telegram-bot.sh - Example deployment script

# Load secrets from secure location
export CADIS_TELEGRAM_BOT_TOKEN=$(cat /etc/cadis/telegram-token)
export CADIS_TELEGRAM_ALLOWED_CHATS="123456789"
export CADIS_DAEMON_URL="127.0.0.1:7433"

# Start bot (runs forever)
/usr/local/bin/cadis-telegram
```

### Systemd Service

```ini
# /etc/systemd/system/cadis-telegram.service
[Unit]
Description=CADIS Telegram Bot
After=cadisd.service
Requires=cadisd.service

[Service]
Type=simple
User=cadis
EnvironmentFile=/etc/cadis/telegram.env
ExecStart=/usr/local/bin/cadis-telegram
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
# /etc/cadis/telegram.env
CADIS_TELEGRAM_BOT_TOKEN=your_token_here
CADIS_TELEGRAM_ALLOWED_CHATS=123456789,987654321
CADIS_DAEMON_URL=127.0.0.1:7433
```

## Development

Run tests:

```bash
cargo test -p cadis-telegram
```

Check for issues:

```bash
cargo clippy -p cadis-telegram
```
