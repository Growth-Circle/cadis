# CADIS

CADIS is a Rust-first, local-first, model-agnostic multi-agent runtime for coordinating AI agents, tools, approvals, desktop HUDs, Telegram control, voice output, and isolated coding workflows.

The project name is written as `cadis` for packages, binaries, directories, and commands. The product display name is `CADIS`.

## Status

Desktop MVP implementation baseline. The repository now includes the typed protocol crate, a local `cadisd` daemon, a `cadis` CLI client, a native `cadis-hud` desktop prototype, a Tauri/React CADIS HUD app, JSONL event persistence with redaction, optional Ollama/OpenAI model adapters, a credential-free local fallback, and an official Codex CLI adapter for ChatGPT-plan auth.

Tools, approval-gated shell/file execution, workers, Telegram, full voice runtime, and code work windows are still planned work.

This repository starts from a clean architecture instead of using OpenClaw as the core backend. CADIS does not fork Codex CLI for v0.1; it can call the installed official CLI as an adapter while keeping daemon authority in `cadisd`.

## Product Direction

CADIS is designed around one local daemon and many interfaces:

```text
Telegram  -> cadisd
HUD       -> cadisd
CLI       -> cadisd
Voice     -> cadisd
Android   -> cadisd
```

The daemon owns orchestration, events, tools, policy, persistence, sessions, and approvals. Interfaces are clients only and must not contain core agent logic.

## Quick Start

Build from source:

```bash
cargo build --release
```

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
target/release/cadis-hud
```

Run the RamaClaw-style desktop HUD:

```bash
cd apps/cadis-hud
pnpm install
pnpm tauri:dev
```

The Tauri HUD discovers the daemon socket from `CADIS_HUD_SOCKET`, `CADIS_SOCKET`,
`~/.cadis/config.toml`, `$XDG_RUNTIME_DIR/cadis/cadisd.sock`, or
`~/.cadis/run/cadisd.sock`.

The default model mode is `auto`: CADIS tries Ollama at `http://127.0.0.1:11434` and falls back to a local credential-free response if Ollama is not running. To use OpenAI API billing, set `[model].provider = "openai"` and provide `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` in the daemon environment. To use ChatGPT Plus/Pro through Codex, install the official Codex CLI, run `codex login`, then set `[model].provider = "codex-cli"`.

## Initial Scope

Linux desktop is the first target.

The current MVP proves the daemon, typed event protocol, local CLI client, native HUD prototype, model response path, and event persistence. Native tool dispatch and central approval policy are next.

## Repository Layout

```text
cadis/
|-- apps/                  # Tauri HUD and future desktop/mobile/server apps
|-- config/                # Example agents, tools, and policy config
|-- crates/                # Rust workspace crates later
|-- docs/                  # Product, business, functional, technical docs
|-- examples/              # Example configs and usage flows later
|-- .github/               # Open-source workflow and issue templates
|-- Cargo.toml             # Workspace manifest placeholder
|-- CONTRIBUTING.md
|-- SECURITY.md
`-- LICENSE
```

## Core Principles

- Fast by default: emit status immediately and keep runtime overhead low.
- Rust-first: daemon, CLI, protocol, tools, scheduler, policy, and persistence should be native Rust.
- Local-first: primary state and orchestration live on the user's machine.
- Model-agnostic: support OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio, and custom HTTP providers.
- Interface-agnostic: Telegram, HUD, CLI, Android, and voice clients use the same daemon protocol.
- Safe by design: risky actions go through one approval engine.
- Code is visual, not spoken: long code, diffs, logs, and test output belong in a code work window.

## Documentation

- [Project Charter](docs/00_PROJECT_CHARTER.md)
- [Blueprint](docs/BLUEPRINT.md)
- [PRD](docs/01_PRD.md)
- [BRD](docs/02_BRD.md)
- [FRD](docs/03_FRD.md)
- [Technical Requirements](docs/04_TRD.md)
- [Architecture](docs/05_ARCHITECTURE.md)
- [Implementation Plan](docs/06_IMPLEMENTATION_PLAN.md)
- [Master Checklist](docs/07_MASTER_CHECKLIST.md)
- [Roadmap](docs/08_ROADMAP.md)
- [Open Source Standard](docs/09_OPEN_SOURCE_STANDARD.md)
- [Risk Register](docs/10_RISK_REGISTER.md)
- [Decisions](docs/11_DECISIONS.md)
- [Test Strategy](docs/12_TEST_STRATEGY.md)
- [Glossary](docs/13_GLOSSARY.md)
- [Threat Model](docs/14_SECURITY_THREAT_MODEL.md)
- [Protocol Draft](docs/15_PROTOCOL_DRAFT.md)
- [Configuration Reference](docs/16_CONFIG_REFERENCE.md)
- [Developer Setup](docs/17_DEVELOPER_SETUP.md)
- [Installation](docs/18_INSTALLATION.md)
- [Requirements Traceability](docs/19_REQUIREMENTS_TRACEABILITY.md)
- [RamaClaw UI Adaptation](docs/20_RAMACLAW_UI_ADAPTATION.md)
- [UI Feature Parity Checklist](docs/21_UI_FEATURE_PARITY_CHECKLIST.md)
- [UI State Protocol Contract](docs/22_UI_STATE_PROTOCOL_CONTRACT.md)
- [UI Design System](docs/23_UI_DESIGN_SYSTEM.md)
- [Contributor Skills](docs/24_CONTRIBUTOR_SKILLS.md)
- [Project Standards](docs/standards/00_STANDARD_INDEX.md)

## Contributor Agent Guides

- [AGENT.md](AGENT.md)
- [CLAUDE.md](CLAUDE.md)
- [Project skills](skills/README.md)

## License

CADIS is licensed under the Apache License 2.0. No third-party source code has been imported into this baseline.
