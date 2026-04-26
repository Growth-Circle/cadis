# CADIS

CADIS is a Rust-first, local-first, model-agnostic multi-agent runtime for coordinating AI agents, tools, approvals, desktop HUDs, Telegram control, voice output, and isolated coding workflows.

The project name is written as `cadis` for packages, binaries, directories, and commands. The product display name is `CADIS`.

## Status

Early implementation baseline. The first Rust crate, `cadis-protocol`, now defines the typed protocol contract; no production daemon runtime has been implemented yet.

This repository starts from a clean architecture instead of using OpenClaw as the core backend. Codex-style coding capabilities may be integrated later through a separate compatibility or extraction layer after license, design, and performance review.

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

## Initial Scope

Linux desktop is the first target.

The first engineering milestone is not a full GUI. The first milestone is a fast daemon, typed event protocol, local CLI client, basic model streaming, native tool dispatch, and central approval policy.

## Repository Layout

```text
cadis/
|-- apps/                  # Desktop/mobile/server application entrypoints later
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
