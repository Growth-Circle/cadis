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
- P5 model subset: local fallback provider plus optional Ollama, OpenAI API,
  and Codex CLI adapters. Ollama now streams native NDJSON deltas and OpenAI
  now streams Chat Completions SSE deltas through the shared provider callback;
  Codex CLI remains wrapped through `codex exec` output until the CLI exposes a
  stable token stream.
- P8 persistence subset: `~/.cadis` layout, JSONL event logs, redaction before
  persistence, and store-level atomic JSON metadata helpers under
  `~/.cadis/state`.
- Workspace architecture docs: accepted target design for profile homes, agent
  homes, project workspaces, worker worktrees, workspace grants, denied paths,
  and project `.cadis/media/` assets in `docs/27_WORKSPACE_ARCHITECTURE.md`.
- Platform baseline docs and CI: Linux remains the primary runtime/HUD target,
  macOS is Rust source validation, and Windows validates only portable crates
  until transport, shell, path, sandbox, HUD, and audio adapters exist. See
  `docs/28_PLATFORM_BASELINE.md`.
- Orchestrator baseline: daemon-owned `@agent` routing, `orchestrator.route`
  events, route-time `agent.status.changed` events, request-driven
  `agent.spawn`, and spawn limits.
- Agent Runtime baseline: durable per-route `AgentSession` records with route,
  task, result, timeout deadline, step budget, cancellation, and parent-child
  metadata exposed through `agent.session.*` lifecycle events and replayed in
  snapshot responses after daemon restart. `session.cancel` now propagates into
  active model provider streams through callback cancellation, so an in-flight
  provider response cannot later overwrite a cancelled AgentSession.
- Track C worker baseline: in-memory daemon worker registry, route-time
  `worker.log.delta` lifecycle logs, `events.snapshot` worker lifecycle
  snapshots, `worker.failed` / `worker.cancelled` terminal metadata,
  one-shot `worker.tail` replay, and compact `worker.result` collection that
  returns terminal worker/AgentSession summaries plus artifact paths without
  replaying raw logs. Worker completion now runs a bounded daemon-owned
  validation command inside the active worker worktree and records the command
  report in worker artifacts.
- P13 HUD subset: Tauri + React `apps/cadis-hud` desktop app, orbital shell,
  chat command panel, agent cards, mention picker, config dialog, six themes,
  model controls, rename dialog, local mic debug, HUD-local voice doctor,
  Edge TTS hooks, approval stack rendering, read-only code work panel for
  worker status/artifact metadata/log tail, and optional Wulan Arc avatar.
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
- Model readiness/routing baseline: `models.list` exposes configured
  provider/model IDs, conservative readiness, effective provider/model metadata,
  and fallback flags; `agent.model.set` selections are used by daemon provider
  routing for message generation instead of remaining HUD-only state.

Still pending:

- **Tool hardening.** `shell.run` has environment allowlist and approval gates
  but still needs typed async cancellation with propagation to running
  subprocesses. `file.patch` needs atomic temp-file writes and concurrent-edit
  protection.
- **Agent runtime.** Model-driven spawn, multi-step tool-call loops, and worker
  concurrency scheduling exist. Still pending: full async tool cancellation and
  cancellation propagation into active provider streams.
- **Worker lifecycle.** Worker worktrees, daemon-owned validation, and
  profile-scoped artifacts are implemented. Still pending: configurable
  test orchestration, worktree file cleanup/removal, and code work panel
  apply/discard actions beyond read-only view.
- **Clients.** Telegram adapter has DaemonBridge but is not production-tested.
  No mobile client yet.
- **Voice.** Edge TTS works as subprocess bridge through HUD. Daemon-owned
  production TTS provider and native Whisper integration are future work.
- **Memory.** Future daemon-owned memory architecture from
  `25_MEMORY_CONCEPT.md` (memory records, scoped retrieval, provenance ledger,
  candidate promotion, memory capsules).
- **Codex CLI streaming.** Current adapter streams normalized callback events
  from `codex exec` output; granularity is limited by the official CLI's stdout
  behavior.
- **HUD macOS bundle.** Icon assets need to be generated for Tauri macOS `.dmg`
  bundling. HUD works via `pnpm tauri:dev`.

## 2.2 Next Execution Plan

The next work should run as parallel tracks with clear ownership. Each track must
keep `cadisd` as runtime authority and keep the HUD as a protocol client.

### Track A - Protocol and Event Bus

Owner: core/protocol agents.

Tasks:

- Add a daemon event bus with fan-out to connected clients.
- Add a persistent `session.subscribe` stream for HUD and CLI clients.
  Baseline now supports session-filtered replay and live fan-out over the
  daemon socket. The daemon now publishes route/status progress before provider
  generation returns and fans out model deltas as provider callbacks arrive.
- Stop holding the runtime mutex while model providers are generating.
  Baseline now prepares daemon-owned message generation under the runtime mutex,
  releases it for provider work, and reacquires it only to mint authoritative
  event envelopes.
- Emit `session.started`, `orchestrator.route`, `agent.status.changed`,
  `message.delta`, and `message.completed` as they happen.
- Add integration tests for two clients receiving the same session events.
  Baseline now includes a real Unix socket integration test with two
  `session.subscribe` clients receiving the same route/status/message events
  while a separate client receives `daemon.status` and `agent.list` responses
  during a deliberately paused provider generation.

Exit criteria:

- HUD receives visible progress before model completion.
  Completed by the HUD live-progress acceptance fixture, which renders live
  `session.started`, `orchestrator.route`, `agent.status.changed`,
  `message.delta`, and `message.completed` frames through the React HUD before
  final completion.
- CLI can subscribe to a live session.
- One slow request does not block unrelated status or agent list requests.

### Track B - Model Provider Readiness and Streaming

Owner: model-provider agents.

Tasks:

- Add provider readiness and capability metadata to the model catalog.
- Make effective provider/model visible to clients.
- Route agent-selected model IDs into provider selection instead of treating
  `agent.model.set` as UI-only state.
- Add streaming callback support for providers that can stream. Baseline now
  includes native Ollama NDJSON streaming, native OpenAI Chat Completions SSE
  streaming, and router dispatch that preserves provider-native stream paths.
- Provider-boundary cancellation is defined as callback `Cancel` control,
  `model.cancelled`, and a non-retryable `model_cancelled` error. Ollama and
  OpenAI stop reading the upstream stream when callbacks request cancellation;
  Codex CLI still uses the default callback wrapper around process output.
- Keep echo provider available only as an explicit fallback state.

Exit criteria:

- HUD can show whether the active model is real, fallback, or unavailable.
- Per-agent model selection changes which provider/model answers.
- OpenAI/Ollama/Codex CLI errors surface as structured daemon errors.
- Ollama and OpenAI emit live `message.delta` events from native provider
  streams before final completion.

### Track C - Orchestrator, Agents, and Workers

Owner: agent-runtime and worker agents.

Tasks:

- Introduce an `AgentSession` state machine with route, task, result, timeout,
  budget, cancellation, and parent-child metadata. Initial baseline is
  in-memory and wraps the existing synchronous route-and-answer path.
- Extend the current request-driven spawn limits into agent-driven spawn:
  max depth, max children per agent, and global agent cap.
- Implement agent-driven spawn as a daemon-authorized action, not HUD logic.
  The current safe slice supports explicit daemon-owned `/worker` and `/spawn`
  orchestration through the same core spawn path; implicit model-driven spawn is
  still reserved for later runtime work.
- Add a worker registry with `worker.started`, `worker.log.delta`,
  `worker.completed`, `worker.failed`, and `worker.cancelled`.
- Implement `worker.tail` from daemon-owned worker logs.
- Implement `worker.result` from daemon-owned terminal worker metadata and
  linked AgentSession result summaries, excluding raw worker logs.
- Propagate `session.cancel` into active provider streams. Baseline now checks
  pending AgentSession cancellation inside daemon provider callbacks, returns
  `ModelStreamControl::Cancel`, and prevents cancelled generations from
  finalizing as failed or completed after the cancel event.
- Create worker worktrees and profile-scoped worker artifacts for explicit
  daemon-planned workers. Baseline now creates session-bound project worktrees
  and writes review artifacts. Worker completion now runs a bounded daemon-owned
  validation command inside the active worker worktree and stores the redacted
  command report in `summary.md` and `test-report.json`. Configurable worker
  commands/tests, cleanup removal, and parent patch application remain future
  work. Any future configurable command/test execution must go through
  daemon-owned policy and run inside the worker worktree, not HUD logic.

Exit criteria:

- An explicit orchestrator action can request a subagent and the daemon enforces
  limits.
- HUD worker tree is driven by daemon worker events.
- Worker logs can be tailed from CLI and HUD.
- Worker terminal result summaries and artifact paths can be collected without
  replaying raw logs.
- Cancelling a session stops the active provider stream at the next callback
  boundary and preserves the AgentSession as `cancelled`.
- Worker command execution baseline runs inside the CADIS-owned worker worktree
  and produces review artifacts without touching the parent checkout.

### Track D - Policy, Approval, and Tool Runtime

Owner: policy/tool agents.

Tasks:

- Implement approval persistence and `approval.respond`.
- Gate shell/file write/git apply operations through central policy.
- Add first native tools: `file.search`, `file.read`, `shell.run`, `git.status`,
  `git.diff`.
- Add timeouts, cancellation, and redaction boundaries.
- Baseline now includes tool contract metadata, safe-read `file.read` and
  `file.search`, `git.status`, `git.diff`, workspace grants, approval
  summaries, approval persistence/recovery, redaction boundaries, approved
  `shell.run` execution, and approved structured `file.patch` execution after
  workspace/input revalidation.
  Remaining Track D hardening includes minimal shell environment filtering,
  typed async tool cancellation, atomic patch writes, and broader
  concurrent-edit protection.

Execution semantics for the Track D execution baseline and next hardening
slice:

- Treat approval as authorization to attempt execution, not as execution
  itself. After an approval is granted, `cadisd` must revalidate approval
  expiry, workspace grant, normalized input, denied paths, secret-access policy,
  and current session/worker state before `tool.started`.
- `shell.run` must execute only inside a registered workspace or CADIS-owned
  worker worktree with `exec` or `admin` access, bounded stdout/stderr,
  exit-code reporting, timeout, and cancellation cleanup. The current baseline
  provides cwd resolution, bounded redacted output, exit code, and timeout
  process cleanup; a minimal environment allowlist and typed async cancellation
  remain required before broad worker command execution is complete.
- `file.patch` must apply only to normalized workspace-relative paths, preserve
  unrelated user edits, fail closed on context mismatch, symlink escape, denied
  path, or concurrent change, and write atomically where practical. The current
  baseline supports structured replace/write operations; atomic writes and
  richer concurrent-edit detection remain hardening work.
- Secret access fails closed by default. A tool that may read a secret-bearing
  path, environment value, config entry, or command output needs explicit policy
  support and approval metadata before execution.
- Timeouts must produce a terminal `tool.failed` result with timeout metadata.
  Cancellation remains incomplete until the protocol and runtime expose a typed
  terminal tool cancellation path and process/file-operation cleanup.

Worker integration sequence:

1. Track C/I creates an isolated worker worktree and profile-scoped artifacts.
2. Current worker command execution uses a bounded daemon-owned validation
   command with cwd inside the worker worktree. Future configurable worker
   commands/tests must use daemon-owned policy and stay inside the worker
   worktree. The HUD, Tauri shell, and code work panel must not run commands
   directly.
3. Worker output is collected into review artifacts, normally `summary.md`,
   `patch.diff`, `changed-files.json`, `test-report.json`, and
   `memory-candidates.jsonl`, with long stdout/stderr kept out of the main chat.
4. Applying a worker artifact to the parent workspace is a separate Track D
   `file.patch` or future patch-apply tool call with its own approval request.
   `worker.completed` never authorizes parent-checkout mutation.
5. Worker cleanup is a separate approved flow and must not delete paths without
   a CADIS-owned worker/worktree record. Terminal worker events may move
   worktrees to `review_pending` or `cleanup_pending`, but state planning is not
   deletion.

Exit criteria:

- Risky tools fail closed without approval.
- Approval decisions survive client reconnects.
- Tool events are visible in CLI/HUD and redacted in JSONL logs.
- Approved `shell.run` and `file.patch` execute only after daemon-side
  revalidation and emit one terminal result.
- Worker patch application cannot modify the parent checkout without a separate
  Track D approval.

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
- Green slice: expose `voice.status`, `voice.doctor`, and `voice.preflight`
  so CLI/HUD can see daemon-owned voice status while HUD/Tauri remains the
  local capture/playback bridge.
- Green slice: define the daemon TTS provider trait, local provider stubs for
  `edge`, `openai`, and `system`, and speech policy that blocks code, diffs,
  terminal logs, and long raw tool/test output before provider dispatch.
- Handle daemon voice events in HUD.
- Baseline now exposes daemon-visible voice status/doctor/preflight, keeps the
  HUD/Tauri bridge for local capture and playback, separates STT language from
  TTS voice selection, and applies speech policy before TTS provider dispatch.

Exit criteria:

- Empty transcript, missing dependency, and blocked mic states are visible and
  actionable.
- Voice preview behavior matches daemon protocol events.
- Speech policy can block code, diffs, logs, and long tool output.

### Track F - Persistence and Recovery

Owner: store/recovery agents.

Tasks:

- Store session metadata, agent metadata, AgentSession metadata, worker
  metadata, and approval metadata with atomic writes. Store-level helpers are
  implemented under `~/.cadis/state`; AgentSession records use
  `state/agent-sessions/<agent-session-id>.json`.
- Load durable session, agent, and AgentSession state on daemon start.
- Keep append-only JSONL as audit log, not the only runtime state.
- Add recovery tests for partial writes, corrupt AgentSession files, and stale
  worker/session records.

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
  and doctor checks for metadata mismatch now exist. Store-level worker worktree
  path/metadata helpers under project `.cadis/worktrees/` now exist.
- Route coding workers into git worktrees and persist worker artifacts under the
  profile home. Profile-scoped worker artifact path helpers now point at
  `profiles/<profile>/artifacts/workers/`; the first execution slice creates
  project-local git worktrees and writes summary, patch, changed-file, test
  report, and memory-candidate artifact files.
- Add doctor checks for duplicated roots, broad grants, symlink escapes, secret
  paths, stale worktrees, corrupt JSONL, and oversized memory/persona files.
  Workspace doctor now includes a baseline for missing, corrupt, and oversized
  agent-home files plus stale project worker worktree metadata and missing
  artifact roots.

Exit criteria:

- Tool calls without workspace grants fail closed or request approval.
- Agent home is never treated as the default project cwd.
- Coding workers edit only their worker worktree unless explicitly approved.
- Project `.cadis/media/` manifests record generated media provenance without
  storing secrets or raw transcripts.

### Track I - Worker Execution Runtime

Owner: worker-runtime agents.

Tasks:

- Turn planned worker metadata into daemon-owned execution setup.
- Create git worktrees under project `.cadis/worktrees/<worker-id>/` for
  session-bound project workspaces.
- Persist project-local worker worktree metadata with `ready` state once the
  worktree exists.
- Write profile-scoped worker artifacts: `summary.md`, `patch.diff`,
  `changed-files.json`, `test-report.json`, and `memory-candidates.jsonl`.
- Execute the daemon-owned worker validation command with cwd inside the active
  worker worktree; do not run commands from HUD, the code work panel, or the
  parent checkout.
- Collect the worker command report into worker artifacts and summarize bounded
  output for `worker.log.delta`, `worker.result`, and code work panel display.
- Add future configurable worker test commands through daemon-owned policy.
- Emit `worker.failed` and `worker.cancelled` with durable failure,
  cancellation, and cleanup-planning metadata.
- Move terminal worktree metadata to `review_pending` or `cleanup_pending`;
  actual worktree removal remains a separate approved cleanup flow requiring a
  CADIS-owned worker/worktree record.
- Keep the parent checkout untouched; patch application remains gated by Track D
  policy/approval work.
- Continue surfacing worker lifecycle and log events to CLI/HUD.

Exit criteria:

- A routed coding worker with a git workspace receives an isolated worktree.
- `worker.started` includes active worktree metadata, and `worker.completed`
  / `worker.failed` / `worker.cancelled` move active worktrees to their planned
  terminal cleanup state.
- Worker artifacts are written under the profile artifact root and are redacted
  before persistence.
- Worker command result and read-only `worker.result` collection can be
  inspected from daemon events and worker artifacts without mutating the parent
  checkout.

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
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
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
  `state/agent-sessions/<agent-session-id>.json`,
  `state/workers/<worker-id>.json`, and
  `state/approvals/<approval-id>.json`.
- Store session metadata.
- Store worker metadata for the current daemon-planned worker delegation
  baseline and recover stale non-terminal worker records as failed on daemon
  restart.
- Store AgentSession metadata.
- Store approval metadata.
- Implement redaction.
- Use atomic writes for state.

Exit criteria:

- Session events survive daemon restart as logs.
- AgentSession snapshots survive daemon restart as durable state.
- Secret redaction tests pass.
- Store-level partial and corrupt state recovery tests pass.

## 12. P9 - Agent Sessions

Goal: create a stable agent runtime boundary.

Tasks:

- Define `AgentSession`.
- Define agent roles.
- Define task input and result.
- Add lifecycle events.
- Add budget and timeout metadata.
- Add cancellation metadata.
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
- Add git worktree creation. Baseline now creates a project-local worktree for
  session-bound git workspaces before worker execution starts.
- Add worker log stream.
- Add worker cleanup. Baseline now exposes metadata-only `worker.cleanup`
  planning, requires CADIS-owned project worktree metadata, rejects
  missing/unknown/non-owned paths, and leaves actual file removal for a later
  approved cleanup executor.
- Add worker command execution. Baseline now runs a bounded daemon-owned
  validation command with cwd inside the worker worktree and records the result
  in artifacts; configurable command/test execution remains future work.
- Add patch collection. Baseline now writes `patch.diff`, `changed-files.json`,
  `test-report.json`, `summary.md`, and `memory-candidates.jsonl` artifact files
  under the profile worker artifact root.
- Add apply approval flow.

Exit criteria:

- Coding worker edits in separate worktree.
- Diff can be generated.
- Command output and test results are collected as worker artifacts.
- Patch is not applied without approval.
- Cleanup cannot delete a worktree without CADIS-owned metadata and approval.

## 13.1 Workspace Architecture Phases

Goal: implement the accepted profile, agent, workspace, and worktree design in
`27_WORKSPACE_ARCHITECTURE.md`.

Status: partially implemented. The current implementation initializes the
default profile home, persists workspace registry/grants, exposes workspace
protocol/CLI commands, gates safe-read tools behind active grants, initializes
daemon-known agent homes, creates worker worktrees, and writes worker artifacts.
Configurable worker test execution, cleanup removal, checkpoint rollback, and
full denied-path coverage remain future phases.

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
  Baseline worktree creation, artifact storage, and terminal cleanup planning
  states now exist; approved cleanup removal remains future work.
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

- First slice: open a read-only artifact view for worker output from daemon
  events and profile-scoped artifact references.
- Open/focus code work window for code-heavy tasks. Baseline now opens a HUD
  code work panel from worker cards.
- Show file tree.
- Show diff from `patch.diff` or daemon-provided patch preview. Baseline shows
  patch artifact references; inline diff content remains future work.
- Show terminal output summaries and bounded logs from `worker.log.delta` /
  artifact previews. Baseline shows recent daemon log tail.
- Show test results from `test-report.json` and `test.result` summaries.
  Baseline shows test-report artifact metadata/status.
- Provide apply/discard request actions that route back through daemon
  protocol; the window must not apply patches or delete worktrees directly.
  Baseline apply/discard controls are disabled placeholders.
- Link to external editor.

Exit criteria:

- Coding output does not flood main chat.
- User can inspect worker status, command summaries, recent log tail, and
  artifact references in a read-only view; inline diff/test artifact content
  remains future work.
- Parent checkout patch application still goes through approval-gated
  `file.patch` or a future patch-apply tool.
- The code work window does not execute tools; it is a protocol client of
  `cadisd`.

## 18. P15 - Alpha Hardening

Goal: make CADIS usable by early external users.

Tasks:

- Add packaging script.
- Add release checks.
- Add install docs.
- Add provider setup docs.
- Add platform support matrix and macOS/Windows baseline CI. Baseline now lives
  in `docs/28_PLATFORM_BASELINE.md` and
  `.github/workflows/platform-baseline.yml`.
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
