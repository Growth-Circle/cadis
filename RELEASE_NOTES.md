# C.A.D.I.S. v0.9.0 — Desktop Beta

**Release date:** 2026-04-28
**Maturity:** Beta
**Platform:** Linux desktop (x86\_64, aarch64 cross-compiled)
**License:** Apache-2.0

## What is C.A.D.I.S.?

C.A.D.I.S. (Coordinated Agentic Distributed Intelligence System) is a local-first, Rust-first, model-agnostic runtime for coordinating AI agents across a desktop HUD, CLI, voice, approvals, native tools, and isolated coding workflows. The daemon (`cadisd`) is the single runtime authority — every interface is a protocol client, not a separate backend.

## Highlights

- **Local daemon runtime** — `cadisd` exposes a Unix socket NDJSON protocol that owns all agent orchestration, tool policy, and approval state.
- **Multi-agent orchestration** — orchestrator routing, agent spawn, and worker isolation via git worktrees with queue-based concurrency scheduling.
- **Desktop HUD** — Tauri + React shell with orbital agent view, chat, agent tree, approval cards, voice controls, code work panel, and six themes.
- **Native tool runtime** — `file.read`, `file.search`, `file.patch`, `shell.run`, `git.status`, `git.diff` with daemon-side approval gates, denied-path enforcement, and secret-file gating.
- **Model provider layer** — Ollama (native NDJSON streaming), OpenAI API (SSE streaming), Codex CLI adapter for ChatGPT Plus/Pro, and local echo fallback.
- **Policy and approval engine** — risk classification, approval expiry, first-response-wins resolution, credential redaction, and shell environment allowlisting.
- **Voice I/O** — daemon-owned Edge TTS with speech policy (blocks code/diff/log), HUD-local playback, and `whisper-cli` transcription bridge.
- **Crash recovery** — sessions, agents, workers, approvals, and AgentSession state survive daemon restarts via JSONL event persistence.

## Install

### From GitHub Releases (Linux binary)

```bash
# Download from https://github.com/Growth-Circle/cadis/releases/tag/v0.9.0
chmod +x cadis cadisd
./cadisd --check
./cadisd &
./cadis status
./cadis chat "hello"
```

### From Source

```bash
git clone https://github.com/Growth-Circle/cadis.git
cd cadis
cargo build --release
target/release/cadisd --check
```

### Run the HUD

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm tauri:dev
```

## Known Limitations

- **Linux-only runtime.** Windows has no daemon/CLI/HUD support; macOS is source-validation only.
- **aarch64 cross-compiled, not natively tested.** aarch64 Linux binaries are cross-compiled but lack native CI testing.
- **Local-only networking.** All communication is Unix socket — no remote relay, multi-machine, or cloud orchestration.
- **No production voice.** Edge TTS runs as a subprocess bridge; Whisper transcription requires a local `whisper-cli` binary.
- **No Telegram or mobile clients.** The Telegram adapter crate exists but is not connected to a live bot. No Android/iOS surfaces.
- **Sequential tool calls per session.** Workers run concurrently, but individual tool calls within a session are sequential. Async cancellation is not yet implemented.
- **No packaged installer or HUD binary.** All artifacts are bare binaries; the Tauri HUD must be built from source.
- **Default `max_steps_per_session=1`.** Increase to 10–20 in config for real multi-step agent workflows.

## Artifacts

| File | Platform |
|------|----------|
| `cadis-x86_64-unknown-linux-gnu` | Linux x86\_64 |
| `cadisd-x86_64-unknown-linux-gnu` | Linux x86\_64 |
| `cadis-aarch64-unknown-linux-gnu` | Linux aarch64 |
| `cadisd-aarch64-unknown-linux-gnu` | Linux aarch64 |
| `*.sha256` | Checksums |

## Checks Performed

- `cargo fmt --all -- --check` — formatting
- `cargo clippy --workspace --all-targets -- -D warnings` — lint
- `cargo test --workspace` — 250+ tests passing
- HUD lint, typecheck, test, and build (`pnpm lint && pnpm tsc && pnpm test && pnpm build`)
- `cargo-deny` license audit — no disallowed licenses
- Daemon smoke test — socket connect, status, chat round-trip
- Documentation review — 28 docs, 20 standards verified

## Security

Report vulnerabilities privately per [SECURITY.md](SECURITY.md). C.A.D.I.S. keeps credentials out of git and logs — local auth artifacts, tokens, `.env` files, JSONL traces, sockets, and crash diagnostics are ignored by default.

## What's Next

**v1.0 stable:** protocol freeze, stable storage format, packaged installers, Telegram client, async tool cancellation, and production voice pipeline.

---

# C.A.D.I.S. v0.9.1 — RC Prep

**Date:** 2026-04-29
**Maturity:** Beta (RC prep)

## Changes

- **SessionUnsubscribe** — implemented in daemon protocol.
- **Telegram adapter** — HTTP connection to Bot API added (adapter was previously command-parser only).
- **UI Feature Parity** — checklist audit completed; master checklist now 404/404.
- **Test coverage** — daemon and CLI test suites expanded.
- **Known limitations** — documented in `docs/KNOWN_LIMITATIONS.md`.