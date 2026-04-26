# Configuration Reference

## 1. Default Location

The desktop MVP reads user config from:

```text
~/.cadis/config.toml
```

`CADIS_HOME` can move that directory. CADIS creates this local state layout on
first run:

```text
~/.cadis/
|-- config.toml
|-- logs/
|-- sessions/
|-- workers/
|-- worktrees/
|-- run/
|-- tokens/
`-- approvals/
```

## 2. Desktop MVP Config

The current implementation supports these keys:

```toml
cadis_home = "~/.cadis"
log_level = "info"

# Optional. If unset, CADIS uses $XDG_RUNTIME_DIR/cadis/cadisd.sock when
# available, otherwise ~/.cadis/run/cadisd.sock.
# socket_path = "~/.cadis/run/cadisd.sock"

[model]
# auto tries Ollama first and falls back to the local credential-free provider.
# Supported values: "auto", "codex-cli", "openai", "ollama", "echo".
provider = "auto"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"

[hud]
theme = "arc"
avatar_style = "orb"
background_opacity = 90
always_on_top = false

[voice]
enabled = false
voice_id = "id-ID-GadisNeural"
rate = 0
pitch = 0
volume = 0
auto_speak = false
```

An example file is available at `config/cadis.example.toml`.

## 3. Model Provider Behavior

- `auto`: tries Ollama at `ollama_endpoint`, then falls back to the local echo provider.
- `codex-cli`: delegates to the installed official Codex CLI with `codex exec`.
  Authenticate the CLI separately with `codex login` for ChatGPT Plus/Pro access.
  CADIS does not read, copy, or persist `~/.codex/auth.json`.
- `openai`: sends chat requests to the OpenAI Chat Completions API. It requires
  `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` in the daemon environment.
- `ollama`: requires a running Ollama server and returns an error event if unavailable.
- `echo`: uses the credential-free local fallback.

The OpenAI API key is not a config key. Do not put API keys, bearer tokens, or
auth headers in `~/.cadis/config.toml`, examples, or logs.

## 4. Environment Variables

```text
CADIS_HOME
CADIS_LOG_LEVEL
CADIS_MODEL_PROVIDER
OPENAI_API_KEY
CADIS_OPENAI_API_KEY
CODEX_API_KEY
CADIS_CODEX_BIN
CADIS_CODEX_MODEL
CADIS_CODEX_EXTRA_ARGS
TELEGRAM_BOT_TOKEN
```

The desktop MVP reads `CADIS_HOME`, `CADIS_LOG_LEVEL`, `CADIS_MODEL_PROVIDER`,
`CADIS_OPENAI_API_KEY`, `OPENAI_API_KEY`, `CADIS_CODEX_BIN`,
`CADIS_CODEX_MODEL`, and `CADIS_CODEX_EXTRA_ARGS`. `CODEX_API_KEY` is consumed
by the official Codex CLI when that CLI is configured for API-key auth; CADIS
does not read it directly. Other provider key variables are reserved for future
model adapters and examples must keep their values empty.

## 5. Secret Rules

- Do not store raw API keys in committed files.
- Do not commit `~/.codex/auth.json`; treat it as a password-equivalent file.
- Prefer `~/.cadis/config.toml` for local runtime config and keep provider keys in environment variables or a future OS keychain integration.
- Do not write resolved secrets to logs.
- Event logs pass through CADIS redaction before JSONL persistence.
- Redact values from keys containing `api_key`, `token`, `secret`, or `authorization`.
