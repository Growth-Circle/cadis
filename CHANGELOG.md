# Changelog

All notable changes to CADIS will be documented in this file.

The format follows Keep a Changelog style, and the project will use Semantic Versioning once the first release exists.

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
