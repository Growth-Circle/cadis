# Protocol Freeze — v1.0 Stable Subset

This document defines the stable protocol surface for C.A.D.I.S. protocol
version `0.1`. Types listed here will not receive breaking changes in any
v1.x release without a major version bump.

## Protocol version

Current: **`0.1`** (`CURRENT_PROTOCOL_VERSION`).

## Transport format

NDJSON (newline-delimited JSON) over a Unix domain socket. Each message is
exactly one JSON object per line. Clients send `RequestEnvelope` objects; the
daemon replies with `ServerFrame` objects.

## ServerFrame envelope

Every line sent by `cadisd` is a `ServerFrame`, tagged by `frame`:

| Frame        | Description                                      |
|--------------|--------------------------------------------------|
| `response`   | Immediate `ResponseEnvelope` for a client request |
| `event`      | Asynchronous `EventEnvelope` from the daemon      |

```json
{ "frame": "response", "payload": { "protocol_version": "0.1", "request_id": "…", "type": "…", "payload": {…} } }
{ "frame": "event",    "payload": { "protocol_version": "0.1", "event_id": "…", "timestamp": "…", "source": "cadisd", "type": "…", "payload": {…} } }
```

## Stable request set — `ClientRequest`

| Wire type                  | Variant               | Description                                              |
|----------------------------|-----------------------|----------------------------------------------------------|
| `events.subscribe`         | EventsSubscribe       | Subscribe to daemon runtime events with optional replay  |
| `events.snapshot`          | EventsSnapshot        | Request a one-shot daemon runtime state snapshot         |
| `daemon.status`            | DaemonStatus          | Query daemon health and runtime status                   |
| `session.create`           | SessionCreate         | Create a new session                                     |
| `session.cancel`           | SessionCancel         | Cancel an existing session                               |
| `session.subscribe`        | SessionSubscribe      | Subscribe to a session's event stream                    |
| `session.unsubscribe`      | SessionUnsubscribe    | Unsubscribe from a session's event stream                |
| `message.send`             | MessageSend           | Send a user message to the daemon                        |
| `tool.call`                | ToolCall              | Request daemon-owned tool execution                      |
| `approval.respond`         | ApprovalRespond       | Respond to a pending approval request                    |
| `agent.list`               | AgentList             | List all known agents                                    |
| `agent.rename`             | AgentRename           | Rename an agent's display name                           |
| `agent.model.set`          | AgentModelSet         | Set an agent's model provider/identifier                 |
| `agent.specialist.set`     | AgentSpecialistSet    | Set an agent's specialist persona                        |
| `agent.spawn`              | AgentSpawn            | Spawn a new agent                                        |
| `agent.kill`               | AgentKill             | Kill a running agent                                     |
| `agent.tail`               | AgentTail             | Tail an agent's recent session events                    |
| `workspace.list`           | WorkspaceList         | List registered workspaces and active grants             |
| `workspace.register`       | WorkspaceRegister     | Register or replace a project workspace                  |
| `workspace.grant`          | WorkspaceGrant        | Grant an agent access to a workspace                     |
| `workspace.revoke`         | WorkspaceRevoke       | Revoke one or more workspace grants                      |
| `workspace.doctor`         | WorkspaceDoctor       | Run workspace registry health checks                     |
| `worker.tail`              | WorkerTail            | Tail a worker's log stream                               |
| `worker.result`            | WorkerResult          | Collect a worker terminal result summary                 |
| `worker.cleanup`           | WorkerCleanup         | Request worker worktree cleanup planning                 |
| `models.list`              | ModelsList            | List available model descriptors                         |
| `ui.preferences.get`       | UiPreferencesGet      | Get daemon-owned UI preferences                          |
| `ui.preferences.set`       | UiPreferencesSet      | Patch daemon-owned UI preferences                        |
| `voice.status`             | VoiceStatus           | Query daemon-visible voice capability status             |
| `voice.doctor`             | VoiceDoctor           | Run daemon-visible voice diagnostics                     |
| `voice.preflight`          | VoicePreflight        | Report local bridge preflight checks to the daemon       |
| `voice.preview`            | VoicePreview          | Preview voice output with optional preferences           |
| `voice.stop`               | VoiceStop             | Stop current voice output                                |
| `config.reload`            | ConfigReload          | Reload daemon configuration from disk                    |
| `daemon.shutdown`          | DaemonShutdown        | Request graceful daemon shutdown                         |

## Stable event set — `CadisEvent`

| Wire type                       | Variant                  | Description                                              |
|---------------------------------|--------------------------|----------------------------------------------------------|
| `daemon.started`                | DaemonStarted            | Daemon process started                                   |
| `daemon.stopping`              | DaemonStopping           | Daemon is shutting down                                  |
| `daemon.error`                  | DaemonError              | Daemon-level error                                       |
| `session.started`               | SessionStarted           | Session was created                                      |
| `session.updated`               | SessionUpdated           | Session state changed                                    |
| `session.completed`             | SessionCompleted         | Session completed                                        |
| `session.failed`                | SessionFailed            | Session failed                                           |
| `message.delta`                 | MessageDelta             | Streaming message content delta                          |
| `message.completed`             | MessageCompleted         | Message generation completed                             |
| `agent.spawned`                 | AgentSpawned             | Agent was spawned                                        |
| `agent.list.response`           | AgentListResponse        | Agent roster snapshot                                    |
| `agent.renamed`                 | AgentRenamed             | Agent display name changed                               |
| `agent.model.changed`           | AgentModelChanged        | Agent model provider changed                             |
| `agent.specialist.changed`      | AgentSpecialistChanged   | Agent specialist persona changed                         |
| `agent.status.changed`          | AgentStatusChanged       | Agent lifecycle status changed                           |
| `agent.completed`               | AgentCompleted           | Agent completed its task                                 |
| `agent.killed`                  | AgentKilled              | Agent was killed by a client                             |
| `agent.session.started`         | AgentSessionStarted      | Agent runtime session started                            |
| `agent.session.updated`         | AgentSessionUpdated      | Agent runtime session state changed                      |
| `agent.session.completed`       | AgentSessionCompleted    | Agent runtime session completed                          |
| `agent.session.failed`          | AgentSessionFailed       | Agent runtime session failed                             |
| `agent.session.cancelled`       | AgentSessionCancelled    | Agent runtime session was cancelled                      |
| `workspace.list.response`       | WorkspaceListResponse    | Workspace registry snapshot                              |
| `workspace.registered`          | WorkspaceRegistered      | Workspace was registered                                 |
| `workspace.grant.created`       | WorkspaceGrantCreated    | Workspace grant was created                              |
| `workspace.grant.revoked`       | WorkspaceGrantRevoked    | Workspace grant was revoked                              |
| `workspace.doctor.response`     | WorkspaceDoctorResponse  | Workspace doctor result                                  |
| `models.list.response`          | ModelsListResponse       | Model catalog snapshot                                   |
| `ui.preferences.updated`        | UiPreferencesUpdated     | UI preferences changed                                   |
| `voice.status.updated`          | VoiceStatusUpdated       | Voice capability status changed                          |
| `voice.doctor.response`         | VoiceDoctorResponse      | Voice diagnostics result                                 |
| `voice.preflight.response`      | VoicePreflightResponse   | Bridge voice preflight was recorded                      |
| `orchestrator.route`            | OrchestratorRoute        | Orchestrator routed a request to an agent                |
| `tool.requested`                | ToolRequested            | Tool execution was requested                             |
| `tool.started`                  | ToolStarted              | Tool execution started                                   |
| `tool.completed`                | ToolCompleted            | Tool execution completed                                 |
| `tool.failed`                   | ToolFailed               | Tool execution failed                                    |
| `approval.requested`            | ApprovalRequested        | Approval is required for a risky action                  |
| `approval.resolved`             | ApprovalResolved         | Approval was resolved                                    |
| `worker.started`                | WorkerStarted            | Worker started                                           |
| `worker.log.delta`              | WorkerLogDelta           | Worker log content delta                                 |
| `worker.completed`              | WorkerCompleted          | Worker completed                                         |
| `worker.failed`                 | WorkerFailed             | Worker failed                                            |
| `worker.cancelled`              | WorkerCancelled          | Worker was cancelled                                     |
| `worker.cleanup.requested`      | WorkerCleanupRequested   | Worker cleanup was requested and recorded                |
| `patch.created`                 | PatchCreated             | Patch was created                                        |
| `test.result`                   | TestResult               | Test result emitted                                      |
| `voice.preview.started`         | VoicePreviewStarted      | Voice preview started                                    |
| `voice.preview.completed`       | VoicePreviewCompleted    | Voice preview completed                                  |
| `voice.preview.failed`          | VoicePreviewFailed       | Voice preview failed                                     |
| `voice.started`                 | VoiceStarted             | Voice playback started                                   |
| `voice.completed`               | VoiceCompleted           | Voice playback completed                                 |

## Stable response set — `DaemonResponse`

| Wire type                  | Variant          | Description                                              |
|----------------------------|------------------|----------------------------------------------------------|
| `request.accepted`         | RequestAccepted  | Request was accepted; follow-up state arrives via events  |
| `daemon.status.response`   | DaemonStatus     | Current daemon health and runtime status                 |
| `request.rejected`         | RequestRejected  | Request was rejected before execution                    |

## Stability guarantees

The types, wire names, and payload shapes listed above are frozen for the v1.x
release line. Specifically:

- No variant will be removed or renamed.
- No required payload field will be removed or change type.
- New optional fields may be added to existing payloads.
- New variants may be added to `ClientRequest`, `CadisEvent`, and `DaemonResponse`.
- Clients must tolerate unknown event types and unknown fields.

A breaking change to any frozen type requires a major protocol version bump.

## What is NOT frozen

The following are explicitly outside the stability contract and may change
between any releases:

- Internal daemon behavior and implementation details
- Model provider selection, fallback logic, and provider adapters
- Tool execution internals, shell environment filtering, and sandbox policy
- UI rendering, HUD layout, and client-side presentation
- JSONL persistence format and redaction boundaries
- Worker orchestration scheduling and concurrency limits
- Avatar engine internals and renderer adapters
- Configuration file schema beyond what the protocol exposes
