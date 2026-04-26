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
| P3 | Daemon Core | `cadisd` lifecycle, sessions, request-scoped events |
| P4 | CLI Client | `cadis` can talk to daemon |
| P5 | Model Provider Path | Providers answer through runtime |
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
- P5 model subset: local fallback provider plus optional Ollama, OpenAI API, and Codex CLI adapters.
- P8 persistence subset: `~/.cadis` layout, JSONL event logs, redaction before
  persistence, and store-level atomic JSON metadata helpers under
  `~/.cadis/state`.
- Workspace architecture docs: accepted target design for profile homes, agent
  homes, project workspaces, worker worktrees, workspace grants, denied paths,
  and project `.cadis/media/` assets in `docs/27_WORKSPACE_ARCHITECTURE.md`.
- Orchestrator baseline: daemon-owned `@agent` routing, `orchestrator.route`
  events, route-time `agent.status.changed` events, request-driven
  `agent.spawn`, and spawn limits.
- P13 HUD subset: Tauri + React `apps/cadis-hud` desktop app, orbital shell,
  chat command panel, agent cards, mention picker, config dialog, six themes,
  model controls, rename dialog, local mic debug, HUD-local voice doctor,
  Edge TTS hooks, approval stack rendering, and optional Wulan Arc avatar.
- Wulan avatar subset: native engine direction in
  `docs/26_WULAN_AVATAR_ENGINE.md` plus `crates/cadis-avatar` for
  renderer-neutral avatar modes, gestures, face-tracking privacy state, renderer
  backend intent, and renderable frame contracts.
- Workspace architecture baseline: profile layout initialization, persistent
  daemon-known agent home initialization, persistent workspace registry/grants,
  workspace protocol/CLI commands, workspace doctor with profile/agent file
  diagnostics, agent-scoped `tool.call` grants, and safe-read tool execution
  behind active grants.
- Voice debug baseline: HUD-local mic doctor, WebAudio analyser telemetry, and
  WebAudio PCM fallback when WebKit `MediaRecorder` produces zero audio chunks.

Still pending:

- Native mutating file/shell tools.
- Agent runtime beyond the current route-and-answer path; existing
  `agent.spawn` is client-requested, not agent-driven.
- Daemon startup wiring for durable session, agent, worker, and approval
  recovery. The store-level atomic write and fail-safe recovery helpers exist.
- Worker lifecycle, isolated worktrees, Telegram/mobile adapters, daemon-owned
  production voice output, and code work window.
- Denied-path enforcement for all mutating tools, checkpoint/rollback manager,
  dedicated profile/agent doctor commands, and media asset manifests.
- Future daemon-owned memory architecture from `25_MEMORY_CONCEPT.md`, including
  memory records, scoped retrieval, provenance ledger, candidate promotion, and
  memory capsules.

## 2.2 Next Execution Plan

The next work should run as parallel tracks with clear ownership. Each track must
keep `cadisd` as runtime authority and keep the HUD as a protocol client.

### Track A - Protocol and Event Bus

Owner: core/protocol agents.

Tasks:

- Add a daemon event bus with fan-out to connected clients.
- Add a persistent `session.subscribe` stream for HUD and CLI clients.
- Stop holding the runtime mutex while model providers are generating.
- Emit `session.started`, `orchestrator.route`, `agent.status.changed`,
  `message.delta`, and `message.completed` as they happen.
- Add integration tests for two clients receiving the same session events.

Exit criteria:

- HUD receives visible progress before model completion.
- CLI can subscribe to a live session.
- One slow request does not block unrelated status or agent list requests.

### Track B - Model Provider Readiness and Streaming

Owner: model-provider agents.

Tasks:

- Add provider readiness and capability metadata to the model catalog.
- Make effective provider/model visible to clients.
- Route agent-selected model IDs into provider selection instead of treating
  `agent.model.set` as UI-only state.
- Add streaming callback support for providers that can stream.
- Keep echo provider available only as an explicit fallback state.

Exit criteria:

- HUD can show whether the active model is real, fallback, or unavailable.
- Per-agent model selection changes which provider/model answers.
- OpenAI/Ollama/Codex CLI errors surface as structured daemon errors.

### Track C - Orchestrator, Agents, and Workers

Owner: agent-runtime and worker agents.

Tasks:

- Introduce an `AgentSession` state machine with route, task, result, timeout,
  budget, cancellation, and parent-child metadata.
- Extend the current request-driven spawn limits into agent-driven spawn:
  max depth, max children per agent, and global agent cap.
- Implement agent-driven spawn as a daemon-authorized action, not HUD logic.
- Add a worker registry with `worker.started`, `worker.log.delta`,
  `worker.completed`, `worker.failed`, and `worker.cancelled`.
- Implement `worker.tail` from daemon-owned worker logs.

Exit criteria:

- An agent can request a subagent through the orchestrator and the daemon enforces
  limits.
- HUD worker tree is driven by daemon worker events.
- Worker logs can be tailed from CLI and HUD.

### Track D - Policy, Approval, and Tool Runtime

Owner: policy/tool agents.

Tasks:

- Implement approval persistence and `approval.respond`.
- Gate shell/file write/git apply operations through central policy.
- Add first native tools: `file.search`, `file.read`, `shell.run`, `git.status`,
  `git.diff`.
- Add timeouts, cancellation, and redaction boundaries.

Exit criteria:

- Risky tools fail closed without approval.
- Approval decisions survive client reconnects.
- Tool events are visible in CLI/HUD and redacted in JSONL logs.

### Track E - Voice as a Daemon-Owned Capability

Owner: voice agents.

Tasks:

- Define voice provider config for `edge`, `openai`, and `system` providers.
- Move `voice.preview` and `voice.stop` toward daemon-owned execution while HUD
  remains a local capture/playback bridge where platform APIs require it.
- Separate STT language from TTS voice selection.
- Add a voice doctor covering mic permission, MediaRecorder, whisper binary,
  whisper model, Node helper, and audio player.
- Keep HUD-local WebAudio PCM capture as a fallback when WebKit
  `MediaRecorder` emits zero chunks even though analyser input is live.
- Promote the current HUD-local voice doctor results into daemon-visible status
  once voice becomes daemon-owned.
- Handle daemon voice events in HUD.

Exit criteria:

- Empty transcript, missing dependency, and blocked mic states are visible and
  actionable.
- Voice preview behavior matches daemon protocol events.
- Speech policy can block code, diffs, logs, and long tool output.

### Track F - Persistence and Recovery

Owner: store/recovery agents.

Tasks:

- Store session metadata, agent metadata, worker metadata, and approval metadata
  with atomic writes. Store-level helpers are implemented under
  `~/.cadis/state`; daemon runtime integration remains pending.
- Load durable state on daemon start.
- Keep append-only JSONL as audit log, not the only runtime state.
- Add recovery tests for partial writes and stale worker/session records.

Exit criteria:

- Spawned agents, selected models, active sessions, and pending approvals survive
  daemon restart.
- Corrupt or partial state files fail safe with clear diagnostics.

### Track G - Wulan Native Avatar Engine

Owner: HUD/native-renderer agents.

Tasks:

- Keep the current Three.js Wulan Arc avatar as a lazy-loaded prototype and
  fallback reference.
- Define a renderer-neutral avatar render state derived from daemon events and
  daemon-owned `hud.avatar_style` preferences.
- Extend `crates/cadis-avatar` from a renderer-neutral state engine into an
  adapter-ready renderer contract.
- Spike a focused Rust/wgpu renderer before considering Bevy.
- Port prototype primitives: portrait texture, alpha cutoff, hologram shader,
  particles, reticles, eye overlay, mouth overlay, and state colors.
- Add body gestures for idle breath, listening lean, nod, gaze shift, approval
  hand cue, speaking emphasis, coding focus, thinking scan, and error recoil.
- Keep face tracking optional, off by default, local-only, permission-gated, and
  disabled without breaking scripted gestures.
- Add reduced-motion and renderer-failure fallback to the default CADIS orb.

Exit criteria:

- Native Wulan can render idle, listening, thinking, speaking, coding, waiting,
  and error states from mock and daemon-derived HUD state.
- Avatar choice remains daemon-backed and no avatar engine code executes tools,
  approvals, model calls, memory retrieval, or policy decisions.
- Bevy is only reconsidered through a decision record if the focused wgpu path
  cannot meet accepted avatar requirements.

### Track H - Workspace Architecture

Owner: workspace/profile/worker agents.

Tasks:

- Implement `CADIS_HOME` and `CADIS_PROFILE_HOME` resolution against the target
  layout in `27_WORKSPACE_ARCHITECTURE.md`. Baseline profile home initialization
  now exists for the default profile.
- Add agent homes with `AGENT.toml`, persona/instruction/memory files, and
  machine-enforced `POLICY.toml`. Baseline templates and typed policy metadata
  now exist for daemon-known agents.
- Extend the implemented workspace registry/grants with full alias management,
  richer expiry UX, and denied-path checks for mutating tools.
- Add project `.cadis/workspace.toml`, `.cadis/worktrees/`, `.cadis/artifacts/`,
  and `.cadis/media/` conventions. Store-level `.cadis/workspace.toml` support
  and doctor checks for metadata mismatch now exist.
- Route coding workers into git worktrees and persist worker artifacts under the
  profile home.
- Add doctor checks for duplicated roots, broad grants, symlink escapes, secret
  paths, stale worktrees, corrupt JSONL, and oversized memory/persona files.
  Workspace doctor now includes a baseline for missing, corrupt, and oversized
  agent-home files.

Exit criteria:

- Tool calls without workspace grants fail closed or request approval.
- Agent home is never treated as the default project cwd.
- Coding workers edit only their worker worktree unless explicitly approved.
- Project `.cadis/media/` manifests record generated media provenance without
  storing secrets or raw transcripts.

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
- Provide store-level durable metadata files:
  `state/sessions/<session-id>.json`, `state/agents/<agent-id>.json`,
  `state/workers/<worker-id>.json`, and
  `state/approvals/<approval-id>.json`.
- Store session metadata.
- Store approval metadata.
- Implement redaction.
- Use atomic writes for state.

Exit criteria:

- Session events survive daemon restart as logs.
- Secret redaction tests pass.
- Store-level partial and corrupt state recovery tests pass.

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

## 13.1 Workspace Architecture Phases

Goal: implement the accepted profile, agent, workspace, and worktree design in
`27_WORKSPACE_ARCHITECTURE.md`.

Status: partially implemented. The current implementation initializes the
default profile home, persists workspace registry/grants, exposes workspace
protocol/CLI commands, and gates safe-read tools behind active grants. Agent
homes, worker worktree creation, checkpoint rollback, and full denied-path
coverage remain future phases.

Tasks:

- W0: add typed terminology for `ProfileHome`, `AgentHome`,
  `ProjectWorkspace`, `WorkerWorktree`, `SandboxRoot`, and `WorkspaceGrant`.
- W1: centralize home/profile path resolution and directory initialization.
  Baseline complete for default profile initialization.
- W2: add profile create/list/use/import/export flows.
- W3: add agent home templates and agent capsule loading.
- W4: add workspace registry, aliases, grants, path guards, and denied paths.
  Baseline complete for registry/grants, broad-root rejection, and safe-read
  path guards.
- W5: add worker worktree creation, artifact storage, and cleanup rules.
- W6: add checkpoint/rollback before destructive or mutating operations.
- W7: integrate workspace, grant, worker, checkpoint, and rollback events.
- W8: add deterministic channel and cwd/project routing.
- W9: add doctor and migration checks for the full layout.
  Baseline complete for duplicate registered roots, missing project metadata,
  and registry/metadata ID mismatch.

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

- Maintain the Tauri + React desktop app as the production-oriented HUD while
  the Rust-native prototype remains a reference path.
- Connect to daemon protocol.
- Show chat stream.
- Show daemon status.
- Show agent tree.
- Show worker cards.
- Show approval cards.
- Show voice controls.
- Keep the default CADIS orb available.
- Keep the current Three.js Wulan Arc avatar optional while native Wulan is
  developed.
- Implement the CADIS-native Wulan avatar engine according to
  `docs/26_WULAN_AVATAR_ENGINE.md`.

Exit criteria:

- HUD can monitor and control active session.
- HUD approval resolves daemon approval.
- Wulan rendering failures fall back to the CADIS orb without blocking the HUD.

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
