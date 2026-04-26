# Product Requirements Document

## 1. Overview

CADIS is a local-first multi-agent runtime that gives one user a fast command layer over AI models, local tools, code work, approvals, Telegram, desktop UI, and voice output.

CADIS should not feel like a slow chatbot wrapper. It should feel like a responsive local control system that streams status immediately, runs work in the background, opens the right surface for the right content, and asks for approval before risky actions.

## 2. Problem Statement

Existing AI assistants often fail in real work because:

- They are tied to one interface.
- They block the main conversation during long-running tasks.
- They mix normal answers, code diffs, terminal output, and approval prompts in one chat feed.
- They lack a central policy engine for tools and approvals.
- They are slow because orchestration and UI are coupled.
- They are hard to extend across local models, cloud models, Telegram, desktop UI, and voice.
- They do not isolate parallel code edits safely.

## 3. Product Thesis

Users will trust and use local AI agents more if the system behaves like an operating layer, not like a single chatbot. The agent must be fast, observable, interruptible, policy-gated, and reachable from multiple interfaces.

## 4. Target Users

### Primary Persona: Local Power User

- Uses Linux desktop.
- Works across several projects.
- Wants a fast AI control system from terminal, Telegram, HUD, and voice.
- Cares about privacy, local state, and avoiding heavy backends.

### Secondary Persona: Developer Operator

- Wants coding agents that edit safely.
- Needs diffs, test output, worktree isolation, review agents, and patch approval.
- Wants the main chat to stay readable.

### Secondary Persona: AI Runtime Contributor

- Wants clean Rust crates, typed protocol, testable policy, and provider abstraction.
- Needs clear architecture and open-source contribution rules.

### Future Persona: Remote Controller User

- Wants to trigger and approve work from Telegram or Android while the daemon runs on a desktop machine.

## 5. Product Goals

- Make local AI orchestration fast and visible.
- Support multiple model providers behind one interface.
- Coordinate main agents, subagents, workers, and tools.
- Route content to the correct surface.
- Keep risky actions policy-gated.
- Keep code work isolated and visually inspectable.
- Provide open-source-quality docs, governance, security policy, and roadmap.

## 6. User Experience Principles

- First event should be immediate.
- Progress should stream before final answers.
- Normal answers can be spoken.
- Code, diffs, logs, and tests must be visual.
- Approval prompts must include risk summary, command or action, target, and consequence.
- Telegram should be useful for remote control, not a full IDE.
- HUD should be dense, operational, and focused.
- CLI should remain scriptable and predictable.

## 7. Primary User Journeys

### Journey A: Local Chat

1. User starts `cadisd`.
2. User runs `cadis chat "summarize this project"`.
3. CLI connects to daemon.
4. Daemon creates a session.
5. Model provider streams response.
6. CLI shows message deltas and final summary.
7. Event log is persisted.

### Journey B: Coding Task

1. User asks CADIS to fix a bug in a repository.
2. Daemon classifies task as code-heavy.
3. Code work session is created.
4. Coding worker runs in an isolated git worktree.
5. Events stream to CLI/HUD/Telegram.
6. Tests run.
7. Diff and test output are shown in code window.
8. Voice speaks only a short summary.
9. User approves applying the patch.

### Journey C: Remote Telegram Approval

1. User sends a Telegram command to start work.
2. Telegram adapter forwards request to `cadisd`.
3. Agent proposes a risky shell command.
4. Policy engine creates approval request.
5. HUD, CLI, and Telegram receive the same approval state.
6. User approves in Telegram.
7. First response wins and all surfaces update.

### Journey D: Voice Output

1. User asks a normal question.
2. Daemon routes final answer to voice if voice is enabled.
3. User asks a code-heavy question.
4. Voice speaks a concise status summary and directs detailed output to code window.

## 8. MVP Scope

### Included in MVP

- Rust workspace baseline.
- `cadisd` daemon skeleton.
- Typed event protocol.
- Local client connection.
- `cadis` CLI.
- Streaming model provider trait.
- One working provider implementation.
- File read/search and shell run tools.
- Central policy engine.
- CLI approval flow.
- Local persistence and JSONL logs.
- Basic agent session abstraction.

### Excluded from MVP

- Full HUD.
- Full Telegram feature set.
- Android client.
- Voice input.
- Marketplace.
- Full Codex integration.
- Cross-machine execution.
- Plugin SDK.

## 9. Product Requirements

| ID | Requirement |
| --- | --- |
| PRD-001 | CADIS must run as a local daemon named `cadisd`. |
| PRD-002 | CADIS must expose a CLI named `cadis`. |
| PRD-003 | All interfaces must communicate with `cadisd` through a documented local protocol. |
| PRD-004 | The daemon must emit status events before final model output. |
| PRD-005 | The runtime must support multiple model providers through a shared abstraction. |
| PRD-006 | Risky tool calls must use central approval policy. |
| PRD-007 | Approval state must be shared across all connected interfaces. |
| PRD-008 | Coding tasks must support separate visual output surfaces. |
| PRD-009 | Parallel coding work must use isolation before patch application. |
| PRD-010 | Persistent logs must redact secrets. |
| PRD-011 | The first target platform must be Linux desktop. |
| PRD-012 | Open-source files and contribution rules must exist before public release. |

## 10. Performance Targets

| Metric | Target |
| --- | --- |
| Time to first daemon status event | Under 100 ms after request accepted |
| Runtime overhead before model call | Under 50 ms in normal path |
| Tool dispatch overhead p95 | Under 25 ms excluding tool execution |
| Stream relay overhead | Under 20 ms per event under normal load |
| Main agent blocked by worker | Never by design |
| Approval fan-out | All active clients receive request within 100 ms locally |

## 11. Product Metrics

- Time to first event.
- Tool dispatch latency.
- Approval response latency.
- Session crash recovery success rate.
- Number of model providers passing conformance tests.
- Number of tool policies covered by tests.
- Coding task patch approval completion rate.
- Secret redaction test pass rate.

## 12. Release Criteria

### v0.1 Pre-Alpha

- Daemon starts.
- CLI can send a message and receive streaming events.
- Event protocol has tests.
- One model provider streams.
- File read/search and shell tools work with policy classes.
- Logs persist with redaction.
- CLI approval flow works.

### v0.2 Alpha

- Telegram adapter can send commands, status, and approvals.
- Basic voice output works.
- Agent session abstraction supports code-heavy routing.

### v0.3 Desktop Alpha

- Linux HUD can show chat, agent tree, approvals, and status.
- Code work window can show diffs, logs, and test results.

## 13. Dependencies

- Rust stable.
- Async runtime, likely Tokio.
- Local IPC transport.
- Model provider SDKs or direct HTTP clients.
- Git for worktree isolation.
- Optional `teloxide` for Telegram.
- Optional Dioxus for desktop UI.
- Optional TTS backend provider.

## 14. Open Product Questions

- Should the first public brand always be `CADIS`, or should docs also use `C.A.D.I.S.`?
- Should the first model provider be OpenAI, Ollama, or custom HTTP?
- Should the daemon protocol start with Unix socket only, WebSocket only, or both?
- Should voice output be enabled in v0.1 or v0.2?
- Should the first GUI be Dioxus Desktop or a simpler terminal UI?

