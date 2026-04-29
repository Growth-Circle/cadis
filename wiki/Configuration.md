# Configuration

## Config file location

C.A.D.I.S. reads user config from `~/.cadis/config.toml`. Override the base directory with `CADIS_HOME`.

An example file is at [`config/cadis.example.toml`](https://github.com/Growth-Circle/cadis/blob/main/config/cadis.example.toml).

## Model provider setup

### Auto (default)

Tries Ollama first, falls back to a local credential-free response:

```toml
[model]
provider = "auto"
```

### Ollama

Install [Ollama](https://ollama.ai), pull a model, then configure:

```bash
ollama pull llama3.2
```

```toml
[model]
provider = "ollama"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
```

### OpenAI API

Set the API key in the daemon environment (never in `config.toml`):

```bash
export CADIS_OPENAI_API_KEY="sk-..."
```

```toml
[model]
provider = "openai"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"
```

### Codex CLI (ChatGPT Plus/Pro)

Install and authenticate the official Codex CLI:

```bash
npm i -g @openai/codex
codex login
```

```toml
[model]
provider = "codex-cli"
```

C.A.D.I.S. does not store ChatGPT credentials; the Codex CLI owns authentication.

## Voice setup

```toml
[voice]
enabled = false
provider = "edge"          # "edge", "openai", or "system"
voice_id = "id-ID-GadisNeural"
stt_language = "auto"
auto_speak = false
max_spoken_chars = 800
```

For microphone input, set whisper environment variables:

```bash
export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
```

The HUD Settings → Voice tab includes a local voice doctor to check all dependencies.

## Workspace registration

Register project workspaces in `~/.cadis/profiles/default/workspaces/registry.toml`:

```toml
[[workspace]]
id = "my-project"
kind = "project"
root = "~/Project/my-project"
vcs = "git"
trusted = true
```

Supported `kind` values: `project`, `documents`, `sandbox`, `worktree`. Tools require an active workspace grant before accessing a project root.

## Key environment variables

| Variable | Purpose |
|---|---|
| `CADIS_HOME` | Override `~/.cadis` base directory |
| `CADIS_SOCKET` | Override daemon socket path |
| `CADIS_OPENAI_API_KEY` | OpenAI API key for the daemon |
| `CADIS_WHISPER_CLI` | Path to `whisper-cli` binary |
| `CADIS_WHISPER_MODEL` | Path to Whisper model file |
| `CADIS_HUD_SOCKET` | Override socket for the Tauri HUD |

## Full reference

See [docs/16_CONFIG_REFERENCE.md](https://github.com/Growth-Circle/cadis/blob/main/docs/16_CONFIG_REFERENCE.md) for the complete configuration reference including agent spawn limits, orchestrator routing, avatar config, and durable state paths.
