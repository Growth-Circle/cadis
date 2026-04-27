# Protocol Draft

## 1. Purpose

The CADIS protocol is the typed contract between clients and `cadisd`.

Initial goals:

- local-first
- stream-friendly
- versioned
- simple to debug
- stable enough for CLI, Telegram, HUD, and tests

## 2. Envelope

Every client request should include:

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "session.create",
  "payload": {}
}
```

Every daemon event should include:

```json
{
  "protocol_version": "0.1",
  "event_id": "evt_...",
  "session_id": "ses_...",
  "timestamp": "2026-04-26T00:00:00Z",
  "source": "cadisd",
  "type": "message.delta",
  "payload": {}
}
```

The desktop MVP transport sends newline-delimited JSON frames from daemon to
client:

```json
{
  "frame": "response",
  "payload": {
    "protocol_version": "0.1",
    "request_id": "req_...",
    "type": "daemon.status.response",
    "payload": {}
  }
}
```

```json
{
  "frame": "event",
  "payload": {
    "protocol_version": "0.1",
    "event_id": "evt_...",
    "timestamp": "2026-04-26T00:00:00Z",
    "source": "cadisd",
    "type": "daemon.started",
    "payload": {}
  }
}
```

Client-to-daemon frames are one `RequestEnvelope` per line. Daemon-to-client
frames are `ServerFrame` values with `response` or `event`.

## 3. Request Types

```text
events.subscribe
events.snapshot
daemon.status
session.create
session.cancel
session.subscribe
session.unsubscribe
message.send
tool.call
approval.respond
agent.list
agent.rename
agent.model.set
agent.spawn
agent.kill
workspace.list
workspace.register
workspace.grant
workspace.revoke
workspace.doctor
worker.tail
models.list
ui.preferences.get
ui.preferences.set
voice.status
voice.doctor
voice.preflight
voice.preview
voice.stop
config.reload
```

`events.subscribe` keeps the connection open after the immediate
`request.accepted` response. The daemon sends, in order:

1. current snapshot events when `include_snapshot` is true
2. up to `replay_limit` retained events after `since_event_id`, when available
3. live runtime events fanned out from requests handled after subscription

Example:

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "hud_...",
  "type": "events.subscribe",
  "payload": {
    "since_event_id": "evt_000120",
    "replay_limit": 128,
    "include_snapshot": true
  }
}
```

`events.snapshot` is a one-shot request for daemon-owned state. The desktop MVP
snapshot is represented as normal event frames, currently including
`agent.list.response`, `ui.preferences.updated`, `session.updated` for known
sessions, `agent.session.*` snapshots for recovered or in-memory per-route
AgentSession records, and worker lifecycle snapshots for workers known to the
in-memory daemon worker registry or recovered durable worker metadata. Recovery
diagnostics for corrupt or skipped durable metadata are emitted as redacted
`daemon.error` events; partial temporary files are ignored and do not produce
events.

`worker.tail` is a one-shot request for recent daemon-owned worker log lines.
The desktop MVP replays log lines from the in-memory worker registry as
`worker.log.delta` events. `lines` is optional; when absent the daemon returns
up to 64 recent lines, capped at 1000. Unknown workers are rejected with
`worker_not_found`.

`worker.started`, `worker.completed`, `worker.failed`, and `worker.cancelled`
may include worktree and artifact metadata. Failed worker events include optional
`error_code` and redacted `error`; cancelled worker events include optional
`cancellation_requested_at`. For session-bound project workspaces, the daemon
worker runtime creates `<project>/.cadis/worktrees/<worker-id>/`, emits the
active worktree path in `worker.started`, and writes profile-scoped artifacts
before `worker.completed` or `worker.failed`. Terminal worker events move active
worktrees to `review_pending` or `cleanup_pending` according to cleanup policy;
patch apply and cleanup remain separate approval-gated flows.

Example:

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "worker.tail",
  "payload": {
    "worker_id": "worker_000001",
    "lines": 20
  }
}
```

`session.subscribe` keeps the connection open after the immediate
`request.accepted` response. The daemon sends the current `session.updated`
snapshot when `include_snapshot` is true, then bounded replay and live events
whose event envelope has the requested `session_id`. It does not deliver
daemon-global events such as agent rosters or UI preferences.

Example:

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "session.subscribe",
  "payload": {
    "session_id": "ses_...",
    "since_event_id": "evt_000120",
    "replay_limit": 128,
    "include_snapshot": true
  }
}
```

`tool.call` requests daemon-owned native tool execution. Tool calls must resolve
a registered workspace and an active workspace grant before execution or
approval flow proceeds. `agent_id` is optional; when present, it lets daemon tool
execution satisfy agent-scoped workspace grants. The initial baseline supports
safe read-only execution for `file.read`, `file.search`, `git.status`, and
`git.diff`; risky placeholders such as `shell.run`, `file.write`, and
`file.patch` create an approval request after the workspace grant check and fail
closed after approval until a later runtime implements the gated action.

Example:

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "tool.call",
  "payload": {
    "session_id": "ses_...",
    "agent_id": "codex",
    "tool_name": "file.read",
    "input": {
      "workspace_id": "example-project",
      "path": "README.md"
    }
  }
}
```

Track D approved execution target:

```text
tool.call
validate input and resolve workspace
classify risk and evaluate policy
tool.requested
approval.requested, if required
approval.respond
approval.resolved
revalidate approval, grant, denied paths, secret posture, and session/worker state
tool.started
tool.output.delta, optional and bounded
tool.completed, tool.failed, or future tool.cancelled
```

Approval is not a client-side execution grant. After `approval.respond` approves
a risky tool, `cadisd` must revalidate the current daemon state before
execution. If the approval expired, the workspace grant changed, a target path is
denied, secret access is not explicitly authorized, the session was cancelled,
or the execution backend is unavailable, the daemon emits `approval.resolved`
followed by `tool.failed`.

Current v0.1 behavior for approved risky placeholders is
`tool.failed.error.code = "tool_execution_blocked"`. Denied approvals use
`approval_denied`; expired approvals use `approval_expired`. Async tool
cancellation is still future work; until a typed `tool.cancelled` event lands in
the protocol crate, cancellation must not be marked complete in the checklist.

`shell.run` input must resolve to a registered workspace or CADIS-owned worker
worktree before execution. The execution backend must use an explicit cwd,
filtered environment, bounded stdout/stderr, exit-code reporting, timeout, and
cleanup on cancellation. It must not inject secrets implicitly.

`file.patch` input must be previewable before approval and must apply only to
daemon-normalized workspace-relative paths. The backend must fail closed on path
traversal, symlink escape, denied path, mismatched context, or concurrent user
edits. Patch writes should be atomic where practical.

Worker integration uses the same protocol flow. Worker command/test execution
runs inside the worker worktree only after Track D tool approval support exists.
Applying a worker artifact to the parent workspace is a separate `file.patch` or
future patch-apply tool call with its own approval; `worker.completed` alone
does not authorize parent-checkout mutation.

`workspace.register` adds or replaces a profile-local project workspace registry
entry. CADIS persists the registry under
`~/.cadis/profiles/<profile>/workspaces/registry.toml` and rejects overly broad
or protected roots such as `/`, the user home directory, `CADIS_HOME`, and known
secret/system directories.

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "workspace.register",
  "payload": {
    "workspace_id": "example-project",
    "kind": "project",
    "root": "/home/user/project",
    "aliases": ["example"],
    "vcs": "git",
    "trusted": true,
    "worktree_root": ".cadis/worktrees",
    "artifact_root": ".cadis/artifacts"
  }
}
```

`workspace.grant` creates an active grant for a registered workspace and persists
it under `~/.cadis/profiles/<profile>/workspaces/grants.jsonl`. Empty `access`
defaults to `read`. Grants without `agent_id` apply to the default local runtime
context; grants with `agent_id` require matching `tool.call.agent_id`.

```json
{
  "protocol_version": "0.1",
  "request_id": "req_...",
  "client_id": "cli_...",
  "type": "workspace.grant",
  "payload": {
    "workspace_id": "example-project",
    "agent_id": "codex",
    "access": ["read", "exec"],
    "source": "user"
  }
}
```

`workspace.list` returns known workspaces and, when `include_grants` is true,
active grants. `workspace.revoke` removes a specific grant by `grant_id` or
matching grants by `workspace_id` and optional `agent_id`. `workspace.doctor`
checks registry presence, root existence, and active grants for a workspace.

## 3.1 Response Types

Immediate response types:

```text
request.accepted
request.rejected
daemon.status.response
```

`daemon.status.response` payload:

```json
{
  "status": "ok",
  "version": "0.1.0",
  "protocol_version": "0.1",
  "cadis_home": "/home/user/.cadis",
  "socket_path": "/run/user/1000/cadis/cadisd.sock",
  "sessions": 0,
  "model_provider": "auto",
  "uptime_seconds": 3,
  "voice": {
    "enabled": false,
    "state": "disabled",
    "provider": "edge",
    "voice_id": "id-ID-GadisNeural",
    "stt_language": "auto",
    "max_spoken_chars": 800,
    "bridge": "hud-local"
  }
}
```

## 4. Event Types

```text
daemon.started
daemon.stopping
daemon.error
session.started
session.updated
session.completed
session.failed
message.delta
message.completed
agent.spawned
agent.list.response
agent.renamed
agent.model.changed
agent.status.changed
agent.completed
agent.session.started
agent.session.updated
agent.session.completed
agent.session.failed
agent.session.cancelled
workspace.list.response
workspace.registered
workspace.grant.created
workspace.grant.revoked
workspace.doctor.response
models.list.response
ui.preferences.updated
voice.status.updated
voice.doctor.response
voice.preflight.response
orchestrator.route
tool.requested
tool.started
tool.completed
tool.failed
approval.requested
approval.resolved
worker.started
worker.log.delta
worker.completed
worker.failed
worker.cancelled
patch.created
test.result
voice.preview.started
voice.preview.completed
voice.preview.failed
voice.started
voice.completed
```

`models.list.response` payloads include conservative provider readiness metadata:

`agent.session.*` events are emitted by the daemon-owned Agent Runtime baseline
for each routed agent task. They are in-memory for the desktop MVP and carry the
route, task, result, timeout, budget, cancellation, and parent-child metadata
needed by clients to render lifecycle state without owning orchestration:

```json
{
  "type": "agent.session.started",
  "agent_session_id": "ags_000001",
  "session_id": "ses_...",
  "route_id": "route_000001",
  "agent_id": "coder",
  "parent_agent_id": "main",
  "task": "run focused tests",
  "status": "running",
  "timeout_at": "2026-04-26T00:15:00Z",
  "budget_steps": 1,
  "steps_used": 0
}
```

Allowed AgentSession statuses are `started`, `running`, `completed`, `failed`,
`cancelled`, `timed_out`, and `budget_exceeded`. `agent.session.completed` adds
an optional redacted `result`. Terminal failure/cancellation events add optional
`error_code`, `error`, and `cancellation_requested_at` fields as applicable.
The current baseline enforces a per-route step budget before provider execution
and records timeout deadlines. AgentSession metadata is written atomically under
`state/agent-sessions/` and recovered on daemon restart so snapshots can replay
the current AgentSession state. `session.cancel` marks active AgentSessions as
`cancelled` and daemon provider callbacks now return provider-boundary
`Cancel` for that pending generation, preventing a later provider response from
turning the AgentSession back into `failed` or `completed`. Tool-loop
cancellation and broader async interrupts remain later runtime work.

`workspace.list.response` payload:

```json
{
  "workspaces": [
    {
      "workspace_id": "example-project",
      "kind": "project",
      "root": "/home/user/project",
      "aliases": ["example"],
      "vcs": "git",
      "trusted": true,
      "worktree_root": ".cadis/worktrees",
      "artifact_root": ".cadis/artifacts"
    }
  ],
  "grants": [
    {
      "grant_id": "grant_000001",
      "agent_id": "codex",
      "workspace_id": "example-project",
      "root": "/home/user/project",
      "access": ["read", "exec"],
      "source": "user"
    }
  ]
}
```

`workspace.doctor.response` payload:

```json
{
  "checks": [
    {
      "name": "registry",
      "status": "ok",
      "message": "1 workspace(s) registered"
    },
    {
      "name": "workspace.grants",
      "status": "ok",
      "message": "1 active grant(s)"
    }
  ]
}
```

```json
{
  "models": [
    {
      "provider": "auto",
      "model": "llama3.2",
      "display_name": "Auto (Ollama llama3.2, then local fallback)",
      "capabilities": ["streaming", "local_fallback"],
      "readiness": "fallback",
      "effective_provider": "ollama",
      "effective_model": "llama3.2",
      "fallback": true
    },
    {
      "provider": "echo",
      "model": "cadis-local-fallback",
      "display_name": "CADIS local fallback",
      "capabilities": ["offline"],
      "readiness": "fallback",
      "effective_provider": "echo",
      "effective_model": "cadis-local-fallback",
      "fallback": true
    }
  ]
}
```

`readiness` is one of `ready`, `fallback`, `requires_configuration`, or
`unavailable`. `fallback: true` means the entry is not a real model provider.
For `auto`, CADIS reports the configured Ollama model as the primary effective
model and marks the entry as fallback-capable because runtime requests can still
fall back to `echo` if Ollama is not ready. `models.list` uses daemon config so
clients can display the configured Ollama/OpenAI model IDs instead of generic
placeholders.

Model-backed `message.delta` and `message.completed` payloads may include a
`model` object:

```json
{
  "content_kind": "chat",
  "content": "Done",
  "agent_id": "codex",
  "agent_name": "Codex",
  "model": {
    "requested_model": "echo/cadis-local-fallback",
    "effective_provider": "echo",
    "effective_model": "cadis-local-fallback",
    "fallback": false
  }
}
```

`requested_model` is the agent-selected provider/model ID when present.
`effective_provider` and `effective_model` are the provider and model that
actually served the request. When fallback occurs, `fallback` is true and
`fallback_reason` may contain a redacted reason suitable for logs and clients.

For `message.send`, `cadisd` first publishes daemon-owned session, route, and
agent status events, then runs provider generation outside the runtime mutex.
Providers with stream callbacks cause `message.delta` events to be fanned out
as callbacks arrive; providers without native streaming still produce the same
typed events after their blocking response returns.

Ollama and OpenAI use provider-native streams in the current baseline: Ollama
reads `/api/generate` NDJSON chunks and OpenAI reads Chat Completions
server-sent events. The provider router preserves these native paths for
per-agent selections such as `ollama/llama3.2` or `openai/gpt-5.2`. Callback
cancellation stops reading the provider stream and returns a non-retryable
`model_cancelled` error. Codex CLI still uses the callback-compatible wrapper
around `codex exec` output because token-level CLI streaming is not part of the
adapter contract yet.

Model provider failures use the normal `ErrorPayload` shape on `session.failed`
events:

```json
{
  "code": "provider_unavailable",
  "message": "Ollama request failed",
  "retryable": true
}
```

Clients should treat `code` and `retryable` as the machine-readable fields.
Provider error messages are for display only and must be redacted before they
reach protocol events or logs. Common Track B provider codes are
`model_auth_missing`, `model_auth_failed`, `provider_client_error`,
`provider_unavailable`, `provider_rate_limited`, `provider_http_error`,
`model_not_found`, `model_request_rejected`, `provider_response_invalid`,
`provider_response_empty`, and `codex_cli_*`.

## 5. Content Kind

```text
chat
summary
code
diff
terminal_log
test_result
approval
error
```

Clients use content kind for routing and display.

## 6. Approval Payload Draft

```json
{
  "approval_id": "apr_...",
  "session_id": "ses_...",
  "tool_call_id": "tool_...",
  "risk_class": "sudo-system",
  "title": "Approval needed",
  "summary": "Run a system-level command",
  "command": "sudo systemctl restart docker",
  "workspace": "/home/user/Project/example",
  "expires_at": "2026-04-26T00:05:00Z"
}
```

## 7. Tool Lifecycle Draft

```text
tool.requested
approval.requested, if needed
approval.resolved, if needed
tool.started
tool.completed or tool.failed
```

Safe-read tools emit `tool.requested`, `tool.started`, and then
`tool.completed` or `tool.failed`. Approval-gated placeholders emit
`tool.requested` and `approval.requested`; `approval.respond` emits
`approval.resolved` and `tool.failed` because risky execution is intentionally
blocked in this baseline.

The approved execution target keeps the same ordering, but inserts daemon-side
revalidation after `approval.resolved` and before `tool.started`. A successful
approved tool emits exactly one terminal event. Timeout is represented as
`tool.failed` with timeout metadata. Future async tool cancellation should emit a
typed terminal cancellation event once the protocol crate supports it.

`tool.completed` may include a redacted structured `output` object. The current
safe-read outputs are:

- `file.read`: `path`, `content`, `truncated`
- `file.search`: `query`, `matches[]`, `truncated`
- `git.status`: `cwd`, `status`
- `git.diff`: `cwd`, `pathspec`, `diff`, `truncated`

## 8. Compatibility Rules

- Protocol version is required.
- Unknown request types are rejected.
- Unknown event types are ignored by clients if marked compatible later.
- Breaking changes require version bump.
- Debug JSON examples must be kept in docs or tests.

## 9. Transport Candidates

Initial:

- Unix socket NDJSON request/response frames
- `events.subscribe` over the same socket for long-lived local event streams
- `session.subscribe` over the same socket for session-filtered event streams

The desktop daemon keeps an in-memory bounded replay buffer for recent runtime
events. The baseline buffer is process-local and is not a durable event store.

- Unix socket for the Linux runtime/HUD target.
- Stdio for tests.
- Newline-delimited JSON frames.

Later:

- WebSocket for HUD and remote relay.
- Windows and macOS runtime transport adapters are not supported yet; see
  `docs/28_PLATFORM_BASELINE.md`.

## 10. HUD Request Drafts

### `message.send`

```json
{
  "type": "message.send",
  "session_id": null,
  "target_agent_id": "codex",
  "content": "@codex run the focused tests",
  "content_kind": "chat"
}
```

`target_agent_id` is optional. If it is absent, `cadisd` may resolve a leading
`@agent` mention against agent ID, display name, or role. If a client supplies
`target_agent_id` from a matching leading mention, `cadisd` strips that mention
from the prompt sent to the provider while preserving the request `content` as
the client-authored input.

When the resolved target is the main orchestrator, `cadisd` also recognizes
explicit, text-level routing actions without changing protocol version `0.1`:

```text
/route @codex run focused tests
/delegate @codex review the patch
/worker Reviewer: inspect the patch
/spawn Tester: run the focused tests
```

`/route` and `/delegate` route to an existing agent and emit
`orchestrator.route` plus worker lifecycle events for the delegated unit.
`/worker` and `/spawn` create a child agent under `main`, subject to the same
spawn limits as `agent.spawn`, then route the task to that child. Direct leading
`@agent` mentions keep their existing behavior and do not require these explicit
actions.

### `approval.respond`

```json
{
  "type": "approval.respond",
  "approval_id": "apr_...",
  "decision": "approved",
  "reason": "Approved from CLI"
}
```

### `agent.rename`

```json
{
  "type": "agent.rename",
  "agent_id": "main",
  "display_name": "CADIS"
}
```

### `agent.model.set`

```json
{
  "type": "agent.model.set",
  "agent_id": "coder",
  "model": "ollama/qwen2.5-coder"
}
```

### `agent.spawn`

```json
{
  "type": "agent.spawn",
  "role": "Coding",
  "parent_agent_id": "main",
  "display_name": "Builder",
  "model": "codex-cli/chatgpt-plan"
}
```

The daemon assigns the new `agent_id` and confirms with `agent.spawned`.
Client-requested spawning and explicit `/worker` or `/spawn` orchestration are
bounded by daemon config. The desktop MVP defaults allow child depth 2, 4 direct
children per parent, and 32 total registered agents including built-in agents.
Explicit orchestration is daemon-owned: `/worker` and `/spawn` requests create
an AgentSession, call the same core spawn path as `agent.spawn`, and enforce the
same depth, child, and global caps before any provider response is produced.
Rejections use:

- `agent_spawn_depth_limit_exceeded`
- `agent_spawn_children_limit_exceeded`
- `agent_spawn_total_limit_exceeded`

Implicit model-driven spawning is reserved for a later runtime track.

### `ui.preferences.set`

```json
{
  "type": "ui.preferences.set",
  "patch": {
    "hud": {
      "theme": "arc",
      "avatar_style": "wulan_arc",
      "background_opacity": 82
    }
  }
}
```

### `voice.status`, `voice.doctor`, and `voice.preflight`

`voice.status` returns the daemon-visible voice state as a
`voice.status.updated` event. `voice.doctor` returns `voice.doctor.response`
with daemon checks plus the last local bridge preflight when one has been
reported.

The daemon remains the owner of voice preferences and policy state. HUD/Tauri
remains the local capture/playback bridge for microphone permissions,
`MediaRecorder`, WebAudio PCM fallback, whisper execution, and native audio
playback.

Supported daemon-visible TTS provider IDs are `edge`, `openai`, and `system`.
The `stub` provider is available for deterministic tests. In this slice all
provider implementations are daemon-local stubs: they validate policy and emit
voice lifecycle events without calling external APIs or reading secrets.

```json
{
  "type": "voice.preflight",
  "surface": "cadis-hud",
  "summary": "ready",
  "checks": [
    {
      "name": "microphone",
      "status": "ok",
      "message": "1 input visible"
    },
    {
      "name": "webaudio.pcm_fallback",
      "status": "ok",
      "message": "PCM fallback available when MediaRecorder emits zero chunks"
    }
  ]
}
```

The daemon applies speech policy before provider dispatch. Final assistant
messages may emit `voice.started` and `voice.completed` only after
`message.completed`, only when `enabled` and `auto_speak` are true, and only for
speakable content. Code, diffs, terminal logs, and long raw tool or test output
must not produce voice playback events.

`status` values are `ok`, `warn`, or `error`. The daemon also accepts HUD-local
aliases such as `pass` and `fail` and normalizes them before emitting
`voice.preflight.response`.

### `voice.preview`

```json
{
  "type": "voice.preview",
  "text": "Halo, saya CADIS. Audio test berhasil.",
  "prefs": {
    "voice_id": "id-ID-GadisNeural",
    "rate": 0,
    "pitch": 0,
    "volume": 0
  }
}
```
