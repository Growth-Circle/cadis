# Installation

## Status

CADIS has no packaged release yet. The desktop MVP can be built and run from
source on Linux.

## From Source

```bash
git clone https://github.com/Growth-Circle/cadis.git
cd cadis
cargo build --release
```

Core binaries:

```text
target/release/cadis
target/release/cadisd
```

The primary desktop HUD lives in `apps/cadis-hud` and is built with pnpm and
Tauri. The Rust workspace may also build older native HUD prototype artifacts,
but packaged HUD work should use the Tauri app unless a decision record changes
that.

The renderer-neutral Wulan avatar state engine is a library crate,
`crates/cadis-avatar`; it is not an installed binary.

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

Launch the native desktop HUD from source:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm tauri:dev
```

For a custom socket:

```bash
target/release/cadisd --socket /tmp/cadis-hud.sock --dev-echo
cd apps/cadis-hud
CADIS_HUD_SOCKET=/tmp/cadis-hud.sock pnpm tauri:dev
```

To build the HUD frontend and native Tauri app locally:

```bash
cd apps/cadis-hud
pnpm build
pnpm tauri:build
```

The HUD is a client of `cadisd`; it does not store credentials or execute tools
directly.

Voice dependencies are optional unless you use HUD speech or mic input. The HUD
Voice settings include a local doctor that checks mic status, `whisper-cli`, the
Whisper model path, the Node helper used for Edge TTS, and local audio players.

The default provider mode is `auto`: CADIS tries Ollama at
`http://127.0.0.1:11434` and falls back to a local credential-free response if
Ollama is not ready. When Ollama is running, CADIS streams native NDJSON deltas
from `/api/generate` into daemon `message.delta` events.

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

Do not store the API key in `config.toml`. OpenAI responses stream through
Chat Completions server-sent events before the final `message.completed` event.

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

Do not copy local `.env` files, Codex auth JSON, logs, sockets, crash dumps,
HAR traces, or Tauri bundle output into commits or release source archives.

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
- Protocol types exist for orchestrator route, `session.subscribe`, and worker
  events. Live event fan-out exists for daemon-wide and session-filtered
  streams, but worker execution is not implemented yet.
- Telegram, production daemon-owned voice output, full HUD parity, and code work window are not implemented yet.
- The Tauri HUD is source-built for now; packaged desktop artifacts are not published yet.

## Package Targets Later

- Linux tarball.
- Debian package.
- AppImage or similar desktop package.
- Homebrew formula for macOS later.
- Windows installer later.
