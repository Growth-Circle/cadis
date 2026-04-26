# Configuration Reference

## 1. Default Location

CADIS reads user config from:

```text
~/.cadis/config.toml
```

Environment variables may override selected fields.

## 2. Draft Config

```toml
[daemon]
home = "~/.cadis"
log_level = "info"
transport = "unix"

[transport.unix]
path = "~/.cadis/cadisd.sock"

[agents]
max_depth = 2
max_children_per_agent = 4
max_global_agents = 12
default_timeout_sec = 900
allow_recursive_spawn = false

[policy]
safe_read = "allow"
workspace_edit = "allow"
network_access = "ask"
secret_access = "ask"
system_change = "ask"
dangerous_delete = "ask"
outside_workspace = "ask"
git_push_main = "ask"
git_force_push = "ask"
sudo_system = "ask"

[models.default]
provider = "ollama"
model = "llama3.1"

[models.openai]
api_key_env = "OPENAI_API_KEY"
model = "gpt-4.1"

[models.ollama]
base_url = "http://localhost:11434"
model = "llama3.1"

[telegram]
enabled = false
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_chat_ids = []

[hud]
theme = "arc"
background_opacity = 82
hotkey = "Super+Space"
always_on_top = false

[hud.chat]
thinking = false
fast = true

[voice]
enabled = false
provider = "edge"
voice_id = "id-ID-GadisNeural"
rate = 0
pitch = 0
volume = 0
auto_speak = true
max_spoken_chars = 800

[agents.display_names]
main = "CADIS"

[agents.models]
main = "openai/gpt-5.5"
```

## 3. Secret Rules

- Store secrets in environment variables or an OS keychain later.
- Do not store raw API keys in config examples.
- Do not write resolved secrets to logs.
- Redact values from keys ending in `_KEY`, `_TOKEN`, `_SECRET`, or `_PASSWORD`.

## 4. Policy Values

Allowed values:

```text
allow
ask
deny
```

Default should be conservative where actions are destructive, external, or privileged.

## 5. Environment Variables

```text
CADIS_HOME
CADIS_LOG_LEVEL
OPENAI_API_KEY
ANTHROPIC_API_KEY
GEMINI_API_KEY
OPENROUTER_API_KEY
TELEGRAM_BOT_TOKEN
```

## 6. HUD Theme Keys

Allowed values:

```text
arc
amber
phosphor
violet
alert
ice
```

## 7. Voice Preference Ranges

```text
rate   -50..50, step 5
pitch  -50..50, step 5
volume -50..50, step 5
```

Initial voice catalog is documented in `docs/23_UI_DESIGN_SYSTEM.md`.
