# Changelog

All notable changes to CADIS will be documented in this file.

The format follows Keep a Changelog style, and the project will use Semantic Versioning once the first release exists.

## Unreleased

### Added

- Initial planning baseline.
- Product, business, functional, technical, architecture, roadmap, and open-source governance documents.
- Open-source repository standard files.
- GitHub discussion template tailored for C.A.D.I.S. product, architecture, tool-runtime, policy, and UX conversations.
- Desktop MVP runtime with `cadisd`, `cadis`, Unix socket NDJSON frames, status/chat/doctor commands, optional Ollama model adapter, local fallback responses, JSONL event logs, and redaction.
- Native `cadis-hud` prototype with orbital HUD shell, status bar, chat command panel, config tabs, theme controls, model controls, voice preview hooks, rename dialog, and approval stack UI.
- Example desktop MVP config at `config/cadis.example.toml`.
- Daemon Unix socket integration coverage for live `session.subscribe` fan-out to
  two clients while status and agent-list requests remain responsive during
  paused message generation.
- Runtime tool definitions now declare richer contract metadata (description, side effects, timeout, workspace scope, cancellation behavior, and secret/network posture), with approval summaries reflecting that contract.
- Agent Runtime baseline with daemon-owned `AgentSession` lifecycle events, per-route timeout and step-budget metadata, cancellation metadata, and explicit `/worker` or `/spawn` orchestration through core spawn limits.
- Native safe-read `git.diff` execution, pending approval restart recovery, and
  daemon/HUD voice lifecycle event handling.
- Platform baseline documentation and GitHub Actions coverage for macOS Rust
  source validation and Windows portable-crate validation.

### Changed

- Refreshed the README to present the project publicly as **C.A.D.I.S.** (`Coordinated Agentic Distributed Intelligence System`) with clearer pre-alpha positioning and a more open-source-ready structure.

### Security

- Expanded `.gitignore` coverage for local state, secrets, diagnostic files, crash dumps, and credential exports.
- Added credential redaction before persisted event logs.
