# Crates

Rust workspace crates for CADIS runtime and supporting tools.

## Core Crates

- `cadis-protocol` - typed requests, responses, events, versioning, serialization
- `cadis-avatar` - Wulan avatar state engine, gesture model, face-tracking privacy config, wgpu-first renderer
- `cadis-core` - core runtime primitives and agent orchestration
- `cadis-daemon` - main daemon (cadisd) with Tokio async runtime
- `cadis-cli` - command-line interface client
- `cadis-store` - persistent state storage and config management
- `cadis-policy` - approval engine and risk classification
- `cadis-models` - model provider abstraction (Ollama, OpenAI, CodexCli, Auto, Echo)
- `cadis-memory` - semantic memory and session persistence
- `cadis-output-filter` - 60-90% token reduction for context efficiency
- `cadis-telegram` - Telegram adapter integration
- `cadis-hud` - native HUD (apps/cadis-hud) using Tauri and React

Each crate has clear responsibility boundaries and is tested independently.
