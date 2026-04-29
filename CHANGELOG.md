# Changelog

All notable changes to CADIS will be documented in this file.

The format follows Keep a Changelog style, and the project will use Semantic Versioning once the first release exists.

## [Unreleased]

## [1.1.2] - 2026-04-29

### Added
- npm distribution: `npm install -g @growthcircle/cadis` with platform-specific binary packages
- Platform packages: @growthcircle/cadis-{linux-x64,linux-arm64,darwin-x64,darwin-arm64,win32-x64}
- GitHub Actions npm-publish workflow (triggers on release)

### Fixed
- Release workflow proceeds when HUD build fails (binary builds are sufficient)

## [1.1.0] - 2026-04-29

### Added
- Cross-platform TCP transport for daemon and CLI (Windows/macOS/Linux)
- macOS path conventions (~/Library/Application Support/cadis)
- Windows path conventions (%APPDATA%\cadis)
- Cross-platform shell adapter (cmd.exe on Windows, /bin/sh on Unix)
- macOS and Windows HUD bundle targets (dmg, nsis)
- macOS and Windows CI upgraded to full test suite
- 9 UI Feature Parity items verified and checked off
- Config dialog render test
- Known limitations updated for cross-platform status

### Changed
- Daemon supports --tcp-port flag for TCP transport
- CLI supports --tcp flag for TCP connection
- Platform baseline upgraded: macOS full tests, Windows full tests
- cadis-core module extraction noted in known limitations

## [1.0.0] - 2026-04-29

### Added

- Telegram adapter: DaemonBridge wiring to cadisd via Unix socket (status, agents, approve, deny, chat).
- HUD: disconnect safety — visual dimming and disabled controls when daemon is unreachable.
- HUD: patch.created and test.result event rendering in Code Work Panel.
- HUD: 7 new frontend tests (themes, agent rename dialog, voice preferences serialization).
- Core module extraction: orchestrator, tools, voice, and workspace split from monolithic lib.rs.
- Wiki pages: Home, Getting Started, Configuration, FAQ, Troubleshooting.
- Screenshot parity verification script.

### Changed

- Bump all workspace crate versions from 0.9.2 to 1.0.0.
- Bump HUD package version from 0.9.2 to 1.0.0.
- cadis-core lib.rs reduced from 15,032 to 12,520 lines via module extraction.
- Known limitations updated for v0.9.2 shipping state (AppImage/deb available).

### Fixed

- Open-source cleanup: removed RamaClaw fallback paths from Tauri lib.rs.
- Replaced private developer paths in docs with generic placeholders.

### Security

- Verified no committed secrets or API keys (only intentional test fixtures).

## [0.9.2] - 2026-04-29

### Added

- Implement `SessionUnsubscribe` protocol request (was returning not_implemented).
- Telegram adapter: real HTTP Bot API connection with `get_updates`, `send_message`, `send_message_with_keyboard`, `answer_callback_query`, and `poll_loop`.
- CLI unit tests: 34 new tests covering arg parsing and utility functions.
- Daemon unit tests: 8 new tests covering bounded_replay, event_bus, and arg parsing.
- UI Feature Parity Checklist: audited sections 3–20 against actual HUD source, ~150 items verified.
- Open-source cleanup: replace RamaClaw brand references with CADIS.
- HUD: quick action chips (yes, no, cancel, expand) in chat composer.
- HUD: approval card risk summary and expiry countdown display.
- HUD: orb meta ring updates after model change.
- `max_steps_per_session` default raised from 1 to 8.
- `cargo-deny` license check added to CI workflow.

### Changed

- Bump all workspace crate versions from 0.1.0 to 0.9.2.
- Bump HUD package version from 0.1.0 to 0.9.2.

### Fixed

- `SessionUnsubscribe` no longer returns an error response.

### Security

- Dependency license audit via `cargo-deny` now runs in CI.

## [0.9.1] - 2026-04-29

### Added

- SessionUnsubscribe protocol implementation.
- Telegram adapter HTTP connection to Bot API.
- UI Feature Parity checklist audit (404/404 master checklist).
- Daemon and CLI test coverage expansion.
- Known limitations documentation.

## [0.9.0] - 2026-04-28

### Added

- Local daemon runtime (`cadisd`) with Unix socket NDJSON protocol.
- CLI client (`cadis`) with status, doctor, models, agents, chat, approve, deny, spawn, worker, workspace, voice, events, and session commands.
- Tauri + React desktop HUD with orbital shell, chat, agent tree, approval cards, voice controls, code work panel, and six themes.
- Multi-agent runtime with orchestrator routing, agent spawn, and worker isolation via git worktrees.
- Model provider layer: Ollama (native NDJSON streaming), OpenAI API (SSE streaming), Codex CLI adapter, and local echo fallback.
- Native tool runtime: file.read, file.search, file.patch, shell.run, git.status, git.diff with approval gates.
- Policy engine with risk classification, approval expiry, first-response-wins, denied paths, and secret fail-closed.
- JSONL event persistence with credential redaction.
- Crash recovery for sessions, agents, workers, approvals, and AgentSession state.
- Daemon-owned Edge TTS voice provider with speech policy (blocks code/diff/log speech).
- Wulan avatar state engine (renderer-neutral) with gesture set and wgpu render plan spike.
- Workspace architecture: profile homes, agent homes, workspace registry, grants, and worker worktrees.
- Platform baseline CI for Linux (full), macOS (source validation), and Windows (portable crates).
- Telegram adapter crate (protocol types, not yet connected to live bot).
- Comprehensive documentation: 28 docs, 20 standards, architecture, protocol, and security threat model.

### Security

- Credential redaction before JSONL persistence.
- Shell environment filtering via allowlist.
- Secret-file gating and denied-path enforcement.
- Approval expiry recheck before execution.
- `.gitignore` coverage for credentials, logs, sockets, and crash dumps.
