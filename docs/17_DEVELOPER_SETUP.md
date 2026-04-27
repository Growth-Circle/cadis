# Developer Setup

## 1. Status

CADIS now has a desktop MVP runtime:

- `cadisd` local daemon over a Unix socket.
- `cadis` CLI client.
- `cadis status`, `cadis doctor`, `cadis models`, and `cadis chat`.
- JSONL event logs under `~/.cadis/logs`.
- Redaction before event persistence.
- Store-level atomic JSON metadata helpers under `~/.cadis/state`.
- Ollama optional model adapter with local fallback.
- OpenAI optional model adapter using env-only API keys.
- Official Codex CLI optional adapter using `codex exec`.
- In-memory worker registry and `worker.tail` replay for route-time worker
  delegation logs.
- Tauri `cadis-hud` desktop prototype under `apps/cadis-hud`.
- HUD-local voice doctor preflight for mic, `whisper-cli`, Whisper model, Node
  helper, and audio player checks.
- `cadis-avatar` renderer-neutral Wulan avatar state crate.

Tools, approval-gated execution, live event fan-out, worker execution,
Telegram, production daemon-owned voice output, and full HUD parity are still
planned work.

The durable state helper baseline exists in `cadis-store`; full daemon startup
recovery and runtime writes for sessions, agents, workers, and approvals remain
pending core/runtime integration.

## 2. Requirements

- Rust stable with `rustfmt` and `clippy`.
- Git.
- Node.js 20 or newer with Corepack for the Tauri HUD frontend.
- pnpm 10.x; the HUD package pins the exact version through `packageManager`.
- Linux desktop for the first target.
- Tauri Linux development packages for HUD native checks: WebKitGTK 4.1,
  Ayatana AppIndicator, librsvg, and patchelf.
- Optional Ollama for real local model responses.
- Optional `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` for OpenAI responses.
- Optional official Codex CLI for ChatGPT Plus/Pro-backed Codex responses.

Cloud provider credentials are not required unless `[model].provider = "openai"`.
ChatGPT-plan auth is handled by the official Codex CLI, not by CADIS.

## 3. Clone

```bash
git clone https://github.com/Growth-Circle/cadis.git
cd cadis
```

## 4. Build And Validate

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Focused native avatar validation:

```bash
cargo test -p cadis-avatar
```

HUD frontend validation:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
```

## 5. Daemon Commands

```bash
cargo run -p cadis-daemon -- --version
cargo run -p cadis-daemon -- --check
cargo run -p cadis-daemon --
```

Use a temporary socket for isolated testing:

```bash
cargo run -p cadis-daemon -- --socket /tmp/cadis-test.sock --dev-echo
```

## 6. CLI Commands

In another terminal:

```bash
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock status
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock doctor
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock models
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock chat "hello"
```

JSON frame output:

```bash
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock --json chat "hello"
```

## 7. Run Desktop HUD

Start `cadisd` first:

```bash
cargo run -p cadis-daemon -- --socket /tmp/cadis-hud.sock --dev-echo
```

In another terminal:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
CADIS_HUD_SOCKET=/tmp/cadis-hud.sock pnpm tauri:dev
```

For a browser-only shell preview:

```bash
VITE_CADIS_SOCKET_PATH=/tmp/cadis-hud.sock pnpm dev
```

The Tauri HUD is the main desktop client. It connects to `cadisd` over the Unix
socket and does not own durable runtime state or credentials. Browser preview
cannot invoke Tauri commands, so it is only useful for renderer work.

## 8. Voice Doctor

The HUD Settings -> Voice tab has a local doctor for desktop voice dependencies.
It reports renderer mic status, MediaRecorder availability, WebAudio analyser
and PCM fallback telemetry, `whisper-cli`, the configured Whisper model, Node
helper execution for Edge TTS, and audio player availability.

Useful local environment overrides:

```bash
export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
export CADIS_WHISPER_LANGUAGE="id"
```

This is a HUD-local preflight today. Daemon-owned voice provider execution and
daemon-visible voice status are still planned.

## 9. Optional Ollama Setup

Install and start Ollama, then pull a model:

```bash
ollama pull llama3.2
```

Create `~/.cadis/config.toml`:

```toml
[model]
provider = "ollama"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
```

Use `provider = "auto"` to fall back to the local credential-free provider when
Ollama is not running.

## 10. Optional OpenAI Setup

Keep the API key in the daemon environment:

```bash
export CADIS_OPENAI_API_KEY="..."
```

Create or update `~/.cadis/config.toml`:

```toml
[model]
provider = "openai"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"
```

Do not store the API key in `config.toml`.

## 11. Optional Codex CLI Setup

Install and authenticate the official Codex CLI:

```bash
npm i -g @openai/codex
codex login
```

Then create or update `~/.cadis/config.toml`:

```toml
[model]
provider = "codex-cli"
```

CADIS calls `codex exec` in read-only ephemeral mode. It does not fork Codex
CLI and does not read or persist `~/.codex/auth.json`.

## 12. Event And Orchestrator Notes

The daemon currently emits route-time session, orchestrator, message delta,
message completed, and agent status events as follow-up frames for a request.
It also supports request-driven `agent.spawn` with configured depth, child, and
total-agent limits.

`events.subscribe` streams daemon-wide events. `session.subscribe` streams the
current session snapshot, bounded replay, and live events filtered to one
session ID. `worker.tail` can replay recent in-memory daemon worker logs.
Persistent worker recovery and daemon worker execution are not implemented yet.

## 13. Durable State Notes

The store crate owns the durable metadata path contract:

```text
~/.cadis/state/sessions/<session-id>.json
~/.cadis/state/agents/<agent-id>.json
~/.cadis/state/agent-sessions/<agent-session-id>.json
~/.cadis/state/workers/<worker-id>.json
~/.cadis/state/approvals/<approval-id>.json
```

Use `StateStore` for new metadata writes. It sanitizes IDs for paths, writes
redacted JSON through temp-file-plus-rename, ignores partial temp files during
recovery, and reports corrupt final JSON as diagnostics.
`cadis-core` currently reloads session, agent, and AgentSession metadata on
runtime startup. Recovered AgentSession records are replayed in
`events.snapshot`; corrupt final AgentSession JSON is reported as a redacted
`daemon.error` diagnostic.

## 14. Development Rules

- Add a crate only when it has a clear responsibility.
- Keep protocol types stable and tested.
- Keep core code independent from UI frameworks.
- Keep provider integrations modular.
- Add security tests before expanding risky tool behavior.
- Do not commit real provider keys, Telegram tokens, Codex auth JSON, `.env`
  files, JSONL logs, diagnostics, Tauri bundles, or local CADIS state.
