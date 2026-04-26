# Installation

## Status

CADIS has no packaged release yet. The desktop MVP can be built and run from
source on Linux.

## From Source

```bash
git clone https://github.com/cadis-ai/cadis.git
cd cadis
cargo build --release
```

Expected binaries:

```text
target/release/cadis
target/release/cadisd
target/release/cadis-hud
```

## First Run

Start the daemon:

```bash
target/release/cadisd --check
target/release/cadisd
```

In another terminal:

```bash
target/release/cadis status
target/release/cadis doctor
target/release/cadis models
target/release/cadis chat "hello"
```

Launch the native desktop HUD:

```bash
target/release/cadis-hud
```

For a custom socket:

```bash
target/release/cadisd --socket /tmp/cadis-hud.sock --dev-echo
target/release/cadis-hud --socket /tmp/cadis-hud.sock
```

Equivalent env-based launch:

```bash
CADIS_HUD_SOCKET=/tmp/cadis-hud.sock target/release/cadis-hud
```

The HUD is a client of `cadisd`; it does not store credentials or execute tools
directly.

The default provider mode is `auto`: CADIS tries Ollama at
`http://127.0.0.1:11434` and falls back to a local credential-free response if
Ollama is not ready.

To use OpenAI, keep the API key in the daemon environment and set the provider
in `~/.cadis/config.toml`:

```bash
export CADIS_OPENAI_API_KEY="..."
```

```toml
[model]
provider = "openai"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"
```

Do not store the API key in `config.toml`.

To use a ChatGPT Plus/Pro subscription through Codex, authenticate the official
Codex CLI first and then select the adapter provider:

```bash
npm i -g @openai/codex
codex login
```

```toml
[model]
provider = "codex-cli"
```

CADIS does not fork Codex CLI and does not read or copy `~/.codex/auth.json`.

## Local State

CADIS stores local state in:

```text
~/.cadis
```

Current desktop MVP contents:

```text
config.toml
logs/
sessions/
workers/
worktrees/
run/
tokens/
approvals/
```

The daemon socket defaults to `$XDG_RUNTIME_DIR/cadis/cadisd.sock` when
`XDG_RUNTIME_DIR` exists, otherwise `~/.cadis/run/cadisd.sock`.

## Optional Ollama Config

Create `~/.cadis/config.toml`:

```toml
[model]
provider = "ollama"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
```

Then run:

```bash
ollama pull llama3.2
target/release/cadis chat "hello"
```

## Known Limitations

- No packaged installer yet.
- No native file/shell tool runtime yet.
- Approval commands exist in the CLI protocol surface but approval storage and tool gating are not implemented yet.
- Telegram, production voice output, full HUD parity, and code work window are not implemented yet.

## Package Targets Later

- Linux tarball.
- Debian package.
- AppImage or similar desktop package.
- Homebrew formula for macOS later.
- Windows installer later.
