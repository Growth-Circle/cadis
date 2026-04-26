# Implementation Plan

## 1. Build Strategy

CADIS should be implemented from the inside out:

```text
protocol -> daemon -> CLI -> model stream -> tools -> policy -> persistence
-> agent runtime -> workers -> Telegram -> voice -> HUD -> code window
```

This order prevents the project from becoming a UI shell before the core runtime is fast and testable.

## 2. Phase Summary

| Phase | Name | Outcome |
| --- | --- | --- |
| P0 | Repository Foundation | Publishable open-source baseline |
| P1 | Rust Workspace Skeleton | Workspace and crate boundaries exist |
| P2 | Protocol and Events | Typed protocol and event log model |
| P3 | Daemon Core | `cadisd` lifecycle, sessions, event bus |
| P4 | CLI Client | `cadis` can talk to daemon |
| P5 | Model Streaming | One provider streams through runtime |
| P6 | Tool Runtime | Native tool registry and first tools |
| P7 | Policy and Approvals | Central risk policy and CLI approvals |
| P8 | Persistence | Config, state, JSONL logs, redaction |
| P9 | Agent Sessions | Agent abstraction and task lifecycle |
| P10 | Worker Isolation | Git worktree worker flow |
| P11 | Telegram Adapter | Remote command and approval surface |
| P12 | Voice Output | TTS trait and speech policy |
| P13 | HUD | Linux desktop control surface |
| P14 | Code Work Window | Diff, logs, tests, patch approval |
| P15 | Alpha Hardening | Tests, docs, packaging, release |

## 2.1 Current Desktop MVP Scope

Implemented in the first runnable baseline:

- P1 workspace skeleton crates: `cadis-core`, `cadis-daemon`, `cadis-cli`, `cadis-store`, `cadis-policy`, and `cadis-models`.
- P3 daemon subset: `cadisd --version`, `cadisd --check`, Unix socket transport, stdio test mode, config load, health status, session registry, and event emission.
- P4 CLI subset: `cadis status`, `cadis doctor`, `cadis models`, `cadis chat`, JSON frame output, and `cadis daemon` launcher.
- P5 model subset: local fallback provider plus optional Ollama adapter.
- P8 persistence subset: `~/.cadis` layout, JSONL event logs, and redaction before persistence.
- P13 HUD prototype subset: native Rust `cadis-hud` window, orbital shell, status bar, chat command panel, config tabs, six themes, model controls, rename dialog, voice preview hooks, and approval stack rendering.

Still pending:

- Native file/shell tools.
- Approval storage and tool gating.
- Agent runtime beyond the main chat path.
- Worker isolation, Telegram, production voice output, full HUD parity, and code work window.

## 3. P0 - Repository Foundation

Goal: make the project ready to publish as an open-source repository.

Tasks:

- Create `README.md`.
- Create `LICENSE`, `NOTICE`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`.
- Create issue templates and PR template.
- Create documentation set: PRD, BRD, FRD, TRD, architecture, roadmap, checklist, risk, decisions.
- Create workspace placeholder.
- Define license and import policy.

Exit criteria:

- Required files exist.
- No private secrets or local-only project assumptions in public docs.
- First implementation sprint is clear.

## 4. P1 - Rust Workspace Skeleton

Goal: create a compilable Rust workspace with empty but meaningful crate boundaries.

Crates:

- `cadis-protocol`
- `cadis-core`
- `cadis-daemon`
- `cadis-cli`
- `cadis-store`
- `cadis-policy`

Tasks:

- Add workspace members.
- Add crate README files.
- Add basic error type strategy.
- Add lint baseline.
- Add formatting and clippy CI.
- Add `cargo test` CI.

Exit criteria:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## 5. P2 - Protocol and Events

Goal: define the typed language spoken by daemon and clients.

Tasks:

- Define protocol version type.
- Define request messages.
- Define response messages.
- Define event enum.
- Define event metadata.
- Add serde serialization.
- Add compatibility tests.
- Add JSON examples.

Core types:

```text
ClientRequest
DaemonResponse
CadisEvent
SessionId
AgentId
ToolCallId
ApprovalId
ContentKind
RiskClass
```

Exit criteria:

- Protocol types serialize and deserialize.
- Unknown event handling strategy is tested.
- Event examples are documented.

## 6. P3 - Daemon Core

Goal: implement `cadisd` with lifecycle, event bus, and session registry.

Tasks:

- Create daemon binary.
- Add config loader.
- Add local transport stub.
- Add event bus.
- Add session registry.
- Add shutdown handling.
- Add health status.
- Add structured logging.

Exit criteria:

```bash
cadisd --version
cadisd --check
cadisd
```

Daemon should emit:

- daemon started
- config loaded
- transport listening
- daemon stopping

## 7. P4 - CLI Client

Goal: implement `cadis` commands for local control.

Commands:

```text
cadis daemon
cadis status
cadis chat <message>
cadis run --cwd <path> <task>
cadis approve <id>
cadis deny <id>
cadis doctor
```

Tasks:

- Build CLI parser.
- Connect to local transport.
- Print event stream.
- Add JSON output option for scripting.
- Add exit code policy.
- Add basic `doctor` checks.

Exit criteria:

```bash
cadis status
cadis chat "hello"
```

## 8. P5 - Model Streaming

Goal: stream model output through daemon events.

Tasks:

- Define `ModelProvider` trait.
- Define `ModelRequest`.
- Define `ModelEvent`.
- Define provider capabilities.
- Implement one provider.
- Add cancellation.
- Map provider errors.
- Add conformance tests.

Recommended first provider decision:

- For internet/cloud-first testing: OpenAI.
- For local-first testing: Ollama.

The project should support both early, but only one is required to prove the runtime path.

Exit criteria:

- `cadis chat "hello"` streams `message.delta`.
- Provider errors become CADIS errors.
- Session completes cleanly.

## 9. P6 - Tool Runtime

Goal: execute native tools through a registry.

Tasks:

- Define tool trait.
- Define input/output schema strategy.
- Define risk class per tool.
- Implement `file.read`.
- Implement `file.search`.
- Implement `shell.run`.
- Add timeout and cancellation.
- Emit tool lifecycle events.

Exit criteria:

- A test agent or CLI command can call file and shell tools.
- Tool lifecycle events appear in logs.
- Tool errors are structured.

## 10. P7 - Policy and Approvals

Goal: require central approval for risky actions.

Tasks:

- Implement policy decision engine.
- Add default policy config.
- Add approval request store.
- Add first-response-wins resolution.
- Add CLI prompt or command approval.
- Deny execution when approval is denied or expired.
- Add tests for race conditions.

Exit criteria:

- `shell.run` can be policy-gated.
- CLI approval unblocks execution.
- CLI denial prevents execution.
- Approval state is logged.

## 11. P8 - Persistence

Goal: make daemon state durable and auditable.

Tasks:

- Create `~/.cadis` directory structure.
- Load `config.toml`.
- Append JSONL event logs.
- Store session metadata.
- Store approval metadata.
- Implement redaction.
- Use atomic writes for state.

Exit criteria:

- Session events survive daemon restart as logs.
- Secret redaction tests pass.
- Partial state write tests pass where feasible.

## 12. P9 - Agent Sessions

Goal: create a stable agent runtime boundary.

Tasks:

- Define `AgentSession`.
- Define agent roles.
- Define task input and result.
- Add lifecycle events.
- Add budget and timeout.
- Add cancellation.
- Add basic tool-call loop if provider supports tool calls.
- Add text protocol fallback for models without native tool calls later.

Exit criteria:

- A main agent can answer.
- An agent can request a tool.
- Agent status is visible.

## 13. P10 - Worker Isolation

Goal: isolate long-running and coding work.

Tasks:

- Define worker scheduler.
- Add git worktree creation.
- Add worker log stream.
- Add worker cleanup.
- Add patch collection.
- Add apply approval flow.

Exit criteria:

- Coding worker edits in separate worktree.
- Diff can be generated.
- Patch is not applied without approval.

## 14. P11 - Telegram Adapter

Goal: control CADIS from Telegram.

Tasks:

- Add optional `cadis-telegram` crate.
- Connect adapter to daemon protocol.
- Support `/status`.
- Support `/spawn`.
- Support `/approve`.
- Support `/deny`.
- Push summaries.
- Push approval cards/buttons.

Exit criteria:

- Telegram message starts a CADIS session.
- Approval can be resolved from Telegram.
- Telegram adapter contains no core agent logic.

## 15. P12 - Voice Output

Goal: speak the right content and avoid speaking the wrong content.

Tasks:

- Define TTS provider trait.
- Define speech policy.
- Implement voice on/off.
- Implement provider stub.
- Implement first provider.
- Route summaries to voice.
- Block long code, diffs, and logs from speech.

Exit criteria:

- Normal answer can be spoken.
- Code-heavy answer speaks only short summary.
- Approval speaks risk summary.

## 16. P13 - HUD

Goal: provide a Linux desktop control surface.

Tasks:

- Create Dioxus desktop app.
- Connect to daemon protocol.
- Show chat stream.
- Show daemon status.
- Show agent tree.
- Show worker cards.
- Show approval cards.
- Show voice controls.

Exit criteria:

- HUD can monitor and control active session.
- HUD approval resolves daemon approval.

## 17. P14 - Code Work Window

Goal: keep coding work visual and separate from chat.

Tasks:

- Open/focus code work window for code-heavy tasks.
- Show file tree.
- Show diff.
- Show terminal output.
- Show test results.
- Provide apply/discard actions.
- Link to external editor.

Exit criteria:

- Coding output does not flood main chat.
- User can inspect diff and approve patch.

## 18. P15 - Alpha Hardening

Goal: make CADIS usable by early external users.

Tasks:

- Add packaging script.
- Add release checks.
- Add install docs.
- Add provider setup docs.
- Add security review checklist.
- Add dependency license audit.
- Add crash recovery tests.
- Add performance benchmarks for event relay and tool dispatch.

Exit criteria:

- Tagged pre-alpha release is publishable.
- Setup is documented.
- Known limitations are explicit.

## 19. Suggested Sprint Order

### Sprint 1

- P1 workspace skeleton.
- P2 protocol types.
- P3 daemon starts.
- P4 CLI status.

### Sprint 2

- CLI chat request.
- Event bus streaming.
- JSONL logs.
- One provider streaming.

### Sprint 3

- Tool registry.
- File and shell tools.
- Policy engine.
- CLI approvals.

### Sprint 4

- Agent session abstraction.
- Basic tool-call loop.
- Worker skeleton.
- Worktree proof of concept.

### Sprint 5

- Telegram adapter.
- Voice trait and stub.
- Release docs and packaging.

### Sprint 6

- HUD prototype.
- Code work window prototype.
- Multi-agent limits.

## 20. Implementation Rules

- No UI logic may bypass daemon protocol.
- No tool may execute without risk classification.
- No provider key may enter logs.
- No direct import of third-party source code without ADR.
- No recursive agent spawning by default.
- No full Android local runtime before desktop alpha.
