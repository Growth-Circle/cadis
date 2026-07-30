# Crates

Rust workspace crates for C.A.D.I.S.

## Core runtime

- `cadis-protocol` — typed requests, responses, events, versioning, and serialization
- `cadis-core` — daemon runtime: agents, tools, orchestration, voice, and workspace logic
- `cadis-daemon` — `cadisd` binary and transport listeners
- `cadis-cli` — `cadis` command-line client
- `cadis-store` — local config, state layout, redaction, and JSONL logs
- `cadis-policy` — risk classification, approvals policy, and path guards
- `cadis-models` — model provider adapters and routing helpers

## Clients and surfaces

- `cadis-hud` — legacy eframe HUD prototype (canonical desktop HUD is `apps/cadis-hud`)
- `cadis-telegram` — Telegram adapter and daemon bridge
- `cadis-avatar` — Wulan avatar state engine and renderer contracts

## Supporting crates

- `cadis-memory` — memory subsystem helpers
- `cadis-output-filter` — output filtering and redaction utilities

Each crate should keep a single clear responsibility. Add new crates only when a boundary is stable enough to justify a separate package.
