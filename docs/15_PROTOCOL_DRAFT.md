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
daemon.status
session.create
session.cancel
session.subscribe
session.unsubscribe
message.send
approval.respond
agent.list
agent.rename
agent.model.set
agent.spawn
agent.kill
worker.tail
models.list
ui.preferences.get
ui.preferences.set
voice.preview
voice.stop
config.reload
```

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
agent.renamed
agent.model.changed
agent.status.changed
agent.completed
models.list.response
ui.preferences.updated
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

## 8. Compatibility Rules

- Protocol version is required.
- Unknown request types are rejected.
- Unknown event types are ignored by clients if marked compatible later.
- Breaking changes require version bump.
- Debug JSON examples must be kept in docs or tests.

## 9. Transport Candidates

Initial:

- Unix socket for Linux.
- Stdio for tests.
- Newline-delimited JSON frames.

Later:

- WebSocket for HUD and remote relay.

## 10. HUD Request Drafts

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

### `ui.preferences.set`

```json
{
  "type": "ui.preferences.set",
  "patch": {
    "hud": {
      "theme": "arc",
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
