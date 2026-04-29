<p align="center">
  <img src="icon.png" alt="C.A.D.I.S. logo" width="132" />
</p>

<h1 align="center">C.A.D.I.S.</h1>

<p align="center"><strong>Coordinated Agentic Distributed Intelligence System</strong></p>

<p align="center">
  Local-first multi-agent runtime for desktop work, native tools, approvals, voice, and isolated coding workflows.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
  <a href="https://github.com/Growth-Circle/cadis/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Growth-Circle/cadis/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/Growth-Circle/cadis/actions/workflows/platform-baseline.yml"><img alt="Platform" src="https://github.com/Growth-Circle/cadis/actions/workflows/platform-baseline.yml/badge.svg?branch=main"></a>
  <img alt="Status: Beta" src="https://img.shields.io/badge/status-beta-blue.svg">
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.75%2B-orange?logo=rust&logoColor=white">
  <img alt="TypeScript" src="https://img.shields.io/badge/typescript-5.x-blue?logo=typescript&logoColor=white">
  <img alt="Tauri" src="https://img.shields.io/badge/tauri-2.x-24C8D8?logo=tauri&logoColor=white">
  <img alt="Local first" src="https://img.shields.io/badge/local--first-yes-brightgreen.svg">
  <img alt="Platform" src="https://img.shields.io/badge/platform-Linux%20%C2%B7%20macOS%20%C2%B7%20Windows-6f42c1?logo=desktop&logoColor=white">
  <a href="https://www.npmjs.com/package/@growthcircle/cadis"><img alt="npm" src="https://img.shields.io/npm/v/@growthcircle/cadis?logo=npm&logoColor=white&label=npm"></a>
  <a href="https://github.com/Growth-Circle/cadis/discussions"><img alt="Discussions" src="https://img.shields.io/github/discussions/Growth-Circle/cadis?logo=github&label=discussions"></a>
  <a href="https://github.com/Growth-Circle/cadis/issues"><img alt="Issues" src="https://img.shields.io/github/issues/Growth-Circle/cadis?logo=github"></a>
  <a href="https://github.com/Growth-Circle/cadis/pulls"><img alt="PRs" src="https://img.shields.io/github/issues-pr/Growth-Circle/cadis?logo=github&label=PRs"></a>
</p>

<p align="center">
  <a href="docs/07_MASTER_CHECKLIST.md"><img alt="Milestone" src="https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fraw.githubusercontent.com%2FGrowth-Circle%2Fcadis%2Fmain%2Fdocs%2Fprogress.json&query=%24.milestone&label=milestone&style=flat&color=58a6ff&logo=target&logoColor=white"></a>
  <a href="docs/07_MASTER_CHECKLIST.md"><img alt="Checklist" src="https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fraw.githubusercontent.com%2FGrowth-Circle%2Fcadis%2Fmain%2Fdocs%2Fprogress.json&query=%24.checklist.done&suffix=%2F404%20done&label=checklist&style=flat&color=30a14e"></a>
</p>

<p align="center">
  <img src="docs/assets/readme/cadis-hud-desktop.png" alt="C.A.D.I.S. desktop HUD with orbital agents, voice chat, and model routing" width="920" />
</p>

<p align="center">
  <sub>C.A.D.I.S. HUD: local daemon status, orbital agents, voice I/O, model routing, and approval-aware desktop control.</sub>
</p>

C.A.D.I.S. is a **Rust-first, local-first, model-agnostic** runtime where one
daemon (`cadisd`) owns all agent orchestration, tool policy, and approval state.
CLI, HUD, voice, and future surfaces are protocol clients — not separate backends.

```text
HUD / CLI / Voice / Telegram / Android
                |
              cadisd
                |
     agents, models, tools, policy, store
```

## Installation

### npm (all platforms)

```bash
npm install -g @growthcircle/cadis
```

### Shell (Linux / macOS)

```bash
curl -fsSL https://github.com/Growth-Circle/cadis/releases/latest/download/cadis-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadis
curl -fsSL https://github.com/Growth-Circle/cadis/releases/latest/download/cadisd-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadisd
chmod +x cadis cadisd
sudo mv cadis cadisd /usr/local/bin/
```

### PowerShell (Windows)

```powershell
$cadisDir = "$env:LOCALAPPDATA\cadis"; New-Item -ItemType Directory -Force -Path $cadisDir | Out-Null
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/latest/download/cadis-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadis.exe"
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/latest/download/cadisd-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadisd.exe"
[Environment]::SetEnvironmentVariable("Path", "$cadisDir;$([Environment]::GetEnvironmentVariable('Path', 'User'))", "User")
```

### Build from source

```bash
git clone https://github.com/Growth-Circle/cadis.git && cd cadis
cargo build --release    # requires Rust 1.75+
```

### Verify

```bash
cadis --version
cadisd --check
```

## Quick start

```bash
# Start the daemon
cadisd

# In another terminal
cadis status          # daemon health
cadis doctor          # diagnostics
cadis models          # available models
cadis agents          # running agents
cadis chat "hello"    # talk to an agent
```

> **Windows**: the daemon uses TCP by default (`127.0.0.1:7433`). You can also
> start it explicitly with `cadisd --tcp-port 7433` and use `cadis --tcp status`.

## Features

- **Daemon-owned runtime** — `cadisd` is the single authority for agents, tools, policy, and state
- **Multi-agent orchestration** — main orchestrator + specialist agents + isolated code workers
- **Native tool runtime** — file, git, shell, and workspace tools with approval gates
- **Policy and approvals** — risk classification, audit trails, and expiry-checked approval state
- **Model-agnostic** — Ollama, OpenAI API, and Codex CLI adapters with native streaming
- **Voice I/O** — Edge TTS output + Whisper transcription input via the HUD
- **Desktop HUD** — Tauri + React orbital interface with model routing and agent control
- **Avatar engine** — renderer-neutral Wulan identity/presence/expression contract
- **Cross-platform** — Linux, macOS, and Windows with TCP transport fallback

## Model provider setup

Default mode is `auto`: tries Ollama at `http://127.0.0.1:11434`, then falls
back to a local credential-free response. For OpenAI, set `CADIS_OPENAI_API_KEY`
or `OPENAI_API_KEY`. For ChatGPT Plus/Pro via Codex CLI:

```bash
codex login
```

```toml
[model]
provider = "codex-cli"
```

See the [Configuration Reference](docs/16_CONFIG_REFERENCE.md) for all options.

## Documentation

📖 [Wiki](https://github.com/Growth-Circle/cadis/wiki) · [Architecture](docs/05_ARCHITECTURE.md) · [Protocol](docs/15_PROTOCOL_DRAFT.md) · [CLI Reference](docs/30_STABLE_CLI.md) · [Config Reference](docs/16_CONFIG_REFERENCE.md) · [Developer Setup](docs/17_DEVELOPER_SETUP.md) · [Platform Baseline](docs/28_PLATFORM_BASELINE.md)

<details>
<summary>All documentation</summary>

- [Project Charter](docs/00_PROJECT_CHARTER.md)
- [Architecture](docs/05_ARCHITECTURE.md)
- [Implementation Plan](docs/06_IMPLEMENTATION_PLAN.md)
- [Master Checklist](docs/07_MASTER_CHECKLIST.md)
- [Open Source Standard](docs/09_OPEN_SOURCE_STANDARD.md)
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
- [Platform Baseline](docs/28_PLATFORM_BASELINE.md)
- [Protocol Freeze](docs/29_PROTOCOL_FREEZE.md)
- [Stable CLI Reference](docs/30_STABLE_CLI.md)
- [Storage Format and Migration](docs/31_STORAGE_FORMAT.md)

</details>

## Repository layout

```text
cadis/
├── apps/                  # Tauri HUD and future apps
├── config/                # Example configuration
├── crates/                # Rust daemon, CLI, protocol, policy, store, models
├── docs/                  # Product, architecture, protocol, and standards docs
│   └── assets/            # Documentation images and README media
├── examples/              # Example configs and usage flows
├── skills/                # Project-local contributor skills
├── AGENT.md               # Coding-agent guidance
├── Cargo.toml             # Rust workspace manifest
├── SECURITY.md
└── LICENSE
```

## Contributors

Thanks to everyone who has contributed to C.A.D.I.S.!

<!-- ALL-CONTRIBUTORS-LIST:START -->
<table>
  <tr>
    <td align="center"><a href="https://github.com/RamaAditya49"><img src="https://avatars.githubusercontent.com/u/213913142?v=4" width="80" style="border-radius:50%;" alt="RamaAditya49"/><br /><sub><b>Rama Aditya</b></sub></a></td>
    <td align="center"><a href="https://github.com/shintaaurelia"><img src="https://avatars.githubusercontent.com/u/271351134?v=4" width="80" style="border-radius:50%;" alt="shintaaurelia"/><br /><sub><b>Shinta Aurelia</b></sub></a></td>
    <td align="center"><a href="https://github.com/wulanrlestari"><img src="https://avatars.githubusercontent.com/u/271922236?v=4" width="80" style="border-radius:50%;" alt="wulanrlestari"/><br /><sub><b>Wulan R Lestari</b></sub></a></td>
    <td align="center"><a href="https://github.com/DeryFerd"><img src="https://avatars.githubusercontent.com/u/109969595?v=4" width="80" style="border-radius:50%;" alt="DeryFerd"/><br /><sub><b>DeryFerd</b></sub></a></td>
  </tr>
</table>
<!-- ALL-CONTRIBUTORS-LIST:END -->

## Contributing

Start with [AGENT.md](AGENT.md), [CONTRIBUTING.md](CONTRIBUTING.md), and
[docs/standards/00_STANDARD_INDEX.md](docs/standards/00_STANDARD_INDEX.md).

If you want to propose a product direction, protocol change, tool-runtime
change, or UX concept, use GitHub Discussions first so design feedback can land
before implementation drift.

## Security

C.A.D.I.S. is built to keep credentials out of git and logs. Local auth
artifacts, tokens, `.env` files, JSONL traces, sockets, and crash diagnostics
are ignored by default. See [SECURITY.md](SECURITY.md) for reporting and
handling guidance.

## License

C.A.D.I.S. is licensed under the Apache License 2.0.
