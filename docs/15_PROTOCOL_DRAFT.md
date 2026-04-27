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
`agent.list.response`, `ui.preferences.updated`, and `session.updated` for known
sessions.

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
safe read-only execution
for `file.read`, `file.search`, and `git.status`; risky placeholders such as
`shell.run`, `file.write`, and `file.patch` create an approval request after the
workspace grant check and do not execute until a later runtime implements the
gated action.

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
  "uptime_seconds": 3
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
workspace.list.response
workspace.registered
workspace.grant.created
workspace.grant.revoked
workspace.doctor.response
models.list.response
ui.preferences.updated
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
patch.created
test.result
voice.preview.started
voice.preview.completed
voice.preview.failed
voice.started
voice.completed
```

`models.list.response` payloads include conservative provider readiness metadata:

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

`tool.completed` may include a redacted structured `output` object. The current
safe-read outputs are:

- `file.read`: `path`, `content`, `truncated`
- `file.search`: `query`, `matches[]`, `truncated`
- `git.status`: `cwd`, `status`

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

- Unix socket for Linux.
- Stdio for tests.
- Newline-delimited JSON frames.

Later:

- WebSocket for HUD and remote relay.

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
