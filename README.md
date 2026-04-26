<p align="center">
  <img src="icon.png" alt="CADIS logo" width="132" />
</p>

<h1 align="center">CADIS</h1>

<p align="center">
  Local-first multi-agent runtime for desktop work, voice, tools, approvals, and code orchestration.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
  <img alt="Rust first" src="https://img.shields.io/badge/runtime-Rust-orange.svg">
  <img alt="Local first" src="https://img.shields.io/badge/local--first-yes-brightgreen.svg">
  <img alt="Linux desktop MVP" src="https://img.shields.io/badge/target-Linux%20desktop-6f42c1.svg">
</p>

<p align="center">
  <img src="docs/assets/readme/cadis-hud-desktop.png" alt="CADIS desktop HUD with orbital agents, voice chat, and model routing" width="920" />
</p>

<p align="center">
  <sub>CADIS HUD: local daemon status, orbital agents, voice I/O, model routing, and approval-ready desktop control.</sub>
</p>

CADIS is a Rust-first, local-first, model-agnostic runtime for coordinating AI
agents across a desktop HUD, CLI, tools, voice, approvals, and isolated coding
workflows.

The daemon, `cadisd`, owns runtime authority. Every UI, voice, Telegram, mobile,
or CLI surface is just a protocol client.

```text
HUD / CLI / Voice / Telegram / Android
                |
              cadisd
                |
     agents, models, tools, policy, store
```

## Why CADIS

Modern AI tools are powerful, but the control plane is often scattered across
browser tabs, CLIs, background scripts, and private app state. CADIS pulls that
work into one local daemon with typed events, explicit approvals, and a desktop
HUD built for repeated daily use.

- **Local-first:** sessions, events, policy, and orchestration live on your machine.
- **Model-agnostic:** use Ollama, OpenAI-compatible APIs, or the official Codex CLI adapter.
- **Daemon-owned:** UI clients do not own agent runtime logic.
- **Approval-oriented:** risky actions belong behind a central policy engine.
- **Voice-aware:** short assistant replies can be spoken; long code/logs stay visual.
- **Open-source baseline:** clean docs, typed protocol, and a contributor-friendly layout.

## Current Status

CADIS is an early desktop MVP. The repository includes:

- `cadisd`: local daemon and protocol authority
- `cadis`: CLI client for status, models, agents, spawn, chat, and doctor checks
- `apps/cadis-hud`: Tauri + React RamaClaw-style HUD
- `crates/cadis-avatar`: renderer-neutral Wulan avatar state engine contract
- typed protocol events for messages, models, agents, approvals, workspaces,
  orchestrator routing, and workers
- JSONL event persistence with redaction boundaries
- profile-local workspace registry/grants for safe-read tools
- optional Ollama, OpenAI API, and Codex CLI model adapters
- official Codex CLI adapter for ChatGPT Plus/Pro login flows
- HUD-local Edge TTS playback, `whisper-cli` voice input, and voice doctor
  preflight

Planned work still includes production-grade mutating tool execution, full
policy coverage, richer worker isolation, Telegram/mobile clients,
daemon-owned production voice, and code work windows. The target workspace
architecture is partially implemented; persistent agent homes, real worker
worktree creation, checkpoint rollback, and project media manifests remain next.

## Quick Start

Build the workspace:

```bash
cargo build --release
```

Start the daemon:

```bash
target/release/cadisd --check
target/release/cadisd
```

Use the CLI:

```bash
target/release/cadis status
target/release/cadis doctor
target/release/cadis models
target/release/cadis agents
target/release/cadis chat "hello"
```

Run the desktop HUD:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm tauri:dev
```

The HUD discovers the daemon socket from `CADIS_HUD_SOCKET`, `CADIS_SOCKET`,
`~/.cadis/config.toml`, `$XDG_RUNTIME_DIR/cadis/cadisd.sock`, or
`~/.cadis/run/cadisd.sock`.

## Models

Default mode is `auto`: CADIS tries Ollama at `http://127.0.0.1:11434`, then
falls back to a local credential-free response if Ollama is not running.

For OpenAI API billing, set `[model].provider = "openai"` and provide
`CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` in the daemon environment.

For ChatGPT Plus/Pro through Codex:

```bash
codex login
```

Then set:

```toml
[model]
provider = "codex-cli"
```

CADIS does not store ChatGPT credentials; the official Codex CLI owns that auth.

## Voice

The HUD can speak final CADIS replies through Edge TTS. For mic input, the HUD
records locally and asks Tauri to transcribe via `whisper-cli`. On WebKitGTK,
CADIS also records WebAudio PCM in parallel, so voice can still transcribe when
`MediaRecorder` sees the mic but produces zero chunks.

```bash
export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
```

On Linux, CADIS installs a WebKitGTK audio permission handler for the HUD. If
your desktop portal blocks the mic, allow microphone access for CADIS in system
settings and click the mic again.

The HUD Settings -> Voice tab includes a local voice doctor that checks renderer
mic status, WebAudio analyser/PCM fallback telemetry, `whisper-cli`, the
configured Whisper model, Node helper execution, and available audio players.

## Repository Layout

```text
cadis/
|-- apps/                  # Tauri HUD and future apps
|-- config/                # Example agents, tools, and policy config
|-- crates/                # Rust daemon, CLI, protocol, model, store crates
|-- docs/                  # Product, architecture, protocol, and standards docs
|   `-- assets/            # Documentation images and README media
|-- examples/              # Example configs and usage flows
|-- skills/                # Project-local contributor skills
|-- AGENT.md               # Canonical guide for coding agents
|-- Cargo.toml             # Rust workspace manifest
|-- SECURITY.md
`-- LICENSE
```

## Documentation

- [Project Charter](docs/00_PROJECT_CHARTER.md)
- [Architecture](docs/05_ARCHITECTURE.md)
- [Implementation Plan](docs/06_IMPLEMENTATION_PLAN.md)
- [Master Checklist](docs/07_MASTER_CHECKLIST.md)
- [Protocol Draft](docs/15_PROTOCOL_DRAFT.md)
- [Configuration Reference](docs/16_CONFIG_REFERENCE.md)
- [Developer Setup](docs/17_DEVELOPER_SETUP.md)
- [Installation](docs/18_INSTALLATION.md)
- [RamaClaw UI Adaptation](docs/20_RAMACLAW_UI_ADAPTATION.md)
- [UI State Protocol Contract](docs/22_UI_STATE_PROTOCOL_CONTRACT.md)
- [UI Design System](docs/23_UI_DESIGN_SYSTEM.md)
- [Memory Concept](docs/25_MEMORY_CONCEPT.md)
- [Wulan Avatar Engine](docs/26_WULAN_AVATAR_ENGINE.md)
- [Workspace Architecture](docs/27_WORKSPACE_ARCHITECTURE.md)
- [Open Source Standard](docs/09_OPEN_SOURCE_STANDARD.md)

## Security

CADIS is built to keep credentials out of git and logs. Local auth artifacts,
tokens, `.env` files, JSONL traces, sockets, and crash diagnostics are ignored
by default. See [SECURITY.md](SECURITY.md) for reporting and handling guidance.

## Contributing

Start with [AGENT.md](AGENT.md), [CONTRIBUTING.md](CONTRIBUTING.md), and
[docs/standards/00_STANDARD_INDEX.md](docs/standards/00_STANDARD_INDEX.md).

## License

CADIS is licensed under the Apache License 2.0.
