# C.A.D.I.S. v1.2.0 — Async runtime, memory system, and zero-config launch

**Release date:** 2026-04-30
**Maturity:** beta

## Highlights

v1.2.0 is the largest feature release since the initial beta. The daemon now runs
on a fully async Tokio runtime with streaming tool output, a persistent memory
system, and intelligent output filtering that reduces token usage by 60–90%. The
CLI launches the daemon and HUD automatically — no manual setup required. This
release also introduces 88 stable error codes, three new tool backends, and
community-contributed CI and npm hardening.

## What's New

- daemon: Migrate to Tokio async runtime for all daemon operations ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Add streaming delta delivery via Tokio mpsc channels — agents receive incremental model output instead of waiting for full responses ([`df8b664`](https://github.com/Growth-Circle/cadis/commit/df8b664) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Add config reload handler — daemon reloads configuration without restart ([`c7c9792`](https://github.com/Growth-Circle/cadis/commit/c7c9792) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Wire checkpoint system for agent state persistence ([`c7c9792`](https://github.com/Growth-Circle/cadis/commit/c7c9792) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Register 88 stable error codes for consistent, machine-readable error reporting ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- tools: Add `file.list` tool backend — list directory contents with glob filtering ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- tools: Add `git.log` tool backend — retrieve commit history with range and format options ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- tools: Add `git.commit` tool backend — stage and commit changes with message and scope ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- tools: Add tool loop — agents chain multiple tool calls iteratively within a single turn, enabling multi-step reasoning ([`df8b664`](https://github.com/Growth-Circle/cadis/commit/df8b664) by [@RamaAditya49](https://github.com/RamaAditya49))
- store: Add `cadis-memory` crate — persistent memory system for agents to store and retrieve context across sessions ([`df8b664`](https://github.com/Growth-Circle/cadis/commit/df8b664) by [@RamaAditya49](https://github.com/RamaAditya49))
- store: Add trigram search index for fast fuzzy matching over memory entries, inspired by QMD ([`fe71a26`](https://github.com/Growth-Circle/cadis/commit/fe71a26) by [@RamaAditya49](https://github.com/RamaAditya49))
- models: Add output filter pipeline for 60–90% token reduction — strips boilerplate, deduplicates, and compresses model output before storage, inspired by RTK ([`506fbc8`](https://github.com/Growth-Circle/cadis/commit/506fbc8) by [@RamaAditya49](https://github.com/RamaAditya49))
- models: Add semantic truncation — intelligently truncates long outputs preserving meaning boundaries instead of cutting at byte offsets, inspired by QMD ([`fe71a26`](https://github.com/Growth-Circle/cadis/commit/fe71a26) by [@RamaAditya49](https://github.com/RamaAditya49))
- cli: Add zero-config launch — `cadis` starts the daemon and HUD automatically if not already running ([`a9ae836`](https://github.com/Growth-Circle/cadis/commit/a9ae836) by [@RamaAditya49](https://github.com/RamaAditya49))
- cli: Add interactive chat fallback — `cadis chat` drops into a REPL when no message argument is provided ([`c7c9792`](https://github.com/Growth-Circle/cadis/commit/c7c9792) by [@RamaAditya49](https://github.com/RamaAditya49))
- cli: Add `cadis profile` commands for managing user and agent profiles ([`c7c9792`](https://github.com/Growth-Circle/cadis/commit/c7c9792) by [@RamaAditya49](https://github.com/RamaAditya49))

## Improvements

- daemon: Enhance agent orchestration with model-driven spawn — the orchestrator selects which specialist agent to spawn based on model routing hints ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Add per-agent token tracking for usage monitoring and budget enforcement ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- tools: Expand tool backend count from 9 to 12 with `file.list`, `git.log`, and `git.commit` ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))

## Bug Fixes

- daemon: Guard `std::fs` import with `cfg(unix)` to fix Windows clippy failure ([`a823942`](https://github.com/Growth-Circle/cadis/commit/a823942) by [@RamaAditya49](https://github.com/RamaAditya49))
- daemon: Gate unix-only test imports behind `cfg(unix)` to fix Windows CI ([`4ef43d9`](https://github.com/Growth-Circle/cadis/commit/4ef43d9) by [@RamaAditya49](https://github.com/RamaAditya49))
- hud: Generate macOS and Windows icons for Tauri builds ([`0675606`](https://github.com/Growth-Circle/cadis/commit/0675606) by [@RamaAditya49](https://github.com/RamaAditya49))

## Infrastructure & CI

- Add security audit workflow with `cargo audit` and `gitleaks` secret scanning ([`3410104`](https://github.com/Growth-Circle/cadis/commit/3410104) by [@RamaAditya49](https://github.com/RamaAditya49))
- Fix gitleaks version pinning in security workflow ([`4ef43d9`](https://github.com/Growth-Circle/cadis/commit/4ef43d9) by [@RamaAditya49](https://github.com/RamaAditya49))
- Apply CI fixes from community PRs: stabilize cross-platform builds ([#41](https://github.com/Growth-Circle/cadis/pull/41), [#43](https://github.com/Growth-Circle/cadis/pull/43) by [@DeryFerd](https://github.com/DeryFerd))
- Add SHA-256 checksum verification to npm install script ([#44](https://github.com/Growth-Circle/cadis/pull/44) by [@DeryFerd](https://github.com/DeryFerd))
- Add `cadis-hud` binary download to npm postinstall ([#45](https://github.com/Growth-Circle/cadis/pull/45) by [@DeryFerd](https://github.com/DeryFerd))
- Add auto npm publish step to release workflow ([`0675606`](https://github.com/Growth-Circle/cadis/commit/0675606) by [@RamaAditya49](https://github.com/RamaAditya49))

## Documentation

- Update known limitations and implementation plan for v1.1.3 ([`ea9e3d7`](https://github.com/Growth-Circle/cadis/commit/ea9e3d7) by [@RamaAditya49](https://github.com/RamaAditya49))
- Add release notes standard ([`docs/standards/21_RELEASE_NOTES_STANDARD.md`](docs/standards/21_RELEASE_NOTES_STANDARD.md) by [@RamaAditya49](https://github.com/RamaAditya49))

## Contributors

Thanks to everyone who contributed to this release!

- [@RamaAditya49](https://github.com/RamaAditya49) — Tokio migration, tool loop, streaming, memory system, output filters, semantic truncation, trigram search, zero-config launch, interactive CLI, profile commands, agent orchestration, error codes, checkpoint wiring, config reload
- [@DeryFerd](https://github.com/DeryFerd) — CI stabilization ([#41](https://github.com/Growth-Circle/cadis/pull/41), [#43](https://github.com/Growth-Circle/cadis/pull/43)), npm SHA-256 verification ([#44](https://github.com/Growth-Circle/cadis/pull/44)), HUD download in npm ([#45](https://github.com/Growth-Circle/cadis/pull/45))

## Installation

### npm (all platforms)

```bash
npm install -g @growthcircle/cadis@1.2.0
```

### Shell (Linux / macOS)

```bash
curl -fsSL https://github.com/Growth-Circle/cadis/releases/download/v1.2.0/cadis-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadis
curl -fsSL https://github.com/Growth-Circle/cadis/releases/download/v1.2.0/cadisd-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadisd
chmod +x cadis cadisd
sudo mv cadis cadisd /usr/local/bin/
```

### PowerShell (Windows)

```powershell
$v = "1.2.0"
$cadisDir = "$env:LOCALAPPDATA\cadis"; New-Item -ItemType Directory -Force -Path $cadisDir | Out-Null
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/download/v$v/cadis-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadis.exe"
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/download/v$v/cadisd-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadisd.exe"
```

### Build from source

```bash
cargo install cadis --version 1.2.0
```

### Verify

```bash
cadis --version
cadisd --check
```

<details>
<summary>SHA-256 checksums</summary>

```text
(checksums will be added after release binaries are built)
```

</details>

## Full Changelog

[v1.1.3...v1.2.0](https://github.com/Growth-Circle/cadis/compare/v1.1.3...v1.2.0)
