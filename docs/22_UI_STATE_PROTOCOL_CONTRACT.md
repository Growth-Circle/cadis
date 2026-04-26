# UI State and Protocol Contract

## 1. Purpose

This document defines the daemon-backed state contract required to adapt the RamaClaw HUD into CADIS without making the UI the owner of core state.

## 2. Core Rule

The HUD may cache state for rendering, but `cadisd` owns durable state and all authoritative operational state.

The UI must never execute tools, approve actions locally, mutate agent runtime state directly, or treat local browser storage as the source of truth.

## 3. View Model

The HUD may keep this ephemeral view model:

```text
HudViewState
|-- connection
|-- active_config_tab
|-- chat_draft
|-- local_scroll_positions
|-- selected_agent_id
|-- rename_dialog_target
|-- pending_local_voice_preview
`-- event_derived_snapshot
```

`event_derived_snapshot` is rebuilt from daemon events and snapshots.

## 4. Durable Preferences

Durable preferences belong in daemon config/state:

```toml
[hud]
theme = "arc"
background_opacity = 82
hotkey = "Super+Space"
always_on_top = false

[hud.chat]
thinking = false
fast = true

[voice]
enabled = false
provider = "edge"
voice_id = "id-ID-GadisNeural"
rate = 0
pitch = 0
volume = 0
auto_speak = true
max_spoken_chars = 800

[agents.display_names]
main = "CADIS"
coder = "Codex"

[agents.models]
main = "openai/gpt-5.5"
coder = "openai/gpt-5.5"
```

## 5. Requests

### `message.send`

Sent when the user submits text in the chat panel.

```json
{
  "type": "message.send",
  "session_id": null,
  "agent_id": "main",
  "channel": "hud-chat",
  "text": "fix the auth bug",
  "model": "openai/gpt-5.5",
  "agent_models": {
    "main": "openai/gpt-5.5",
    "coder": "openai/gpt-5.5"
  },
  "preferences": {
    "thinking": false,
    "fast": true
  }
}
```

### `agent.rename`

Sent after the rename dialog submits.

```json
{
  "type": "agent.rename",
  "agent_id": "main",
  "display_name": "CADIS"
}
```

Rules:

- daemon normalizes or validates again
- daemon persists accepted name
- daemon emits `agent.renamed`
- UI updates from event

### `agent.model.set`

Sent when a per-agent model selector changes.

```json
{
  "type": "agent.model.set",
  "agent_id": "coder",
  "model": "ollama/qwen2.5-coder"
}
```

### `models.list`

Sent by HUD on connect or config dialog open.

```json
{
  "type": "models.list"
}
```

### `approval.respond`

Sent when user clicks approve or deny.

```json
{
  "type": "approval.respond",
  "approval_id": "apr_123",
  "verdict": "approve"
}
```

The UI must not remove the card immediately.

### `ui.preferences.set`

Sent when the user changes theme, opacity, chat prefs, window prefs, or other HUD preferences.

```json
{
  "type": "ui.preferences.set",
  "patch": {
    "hud": {
      "theme": "ice",
      "background_opacity": 75
    }
  }
}
```

### `voice.preview`

Sent when user clicks voice test.

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

### `voice.stop`

Sent when user clicks stop during preview or speech.

```json
{
  "type": "voice.stop"
}
```

## 6. Events

### `daemon.status`

Drives connection and status bar.

```json
{
  "type": "daemon.status",
  "state": "connected",
  "latency_ms": 3,
  "version": "0.1.0"
}
```

### `models.list.response`

Updates model catalog and default model.

```json
{
  "type": "models.list.response",
  "models": ["openai/gpt-5.5", "ollama/qwen2.5-coder"],
  "default_model": "openai/gpt-5.5",
  "agent_models": {
    "main": "openai/gpt-5.5",
    "coder": "ollama/qwen2.5-coder"
  }
}
```

### `agent.status.changed`

Drives agent card status dot and counts.

```json
{
  "type": "agent.status.changed",
  "agent_id": "coder",
  "status": "working"
}
```

Allowed UI statuses:

```text
working
idle
waiting
spawning
completed
failed
cancelled
```

### `agent.task.changed`

Drives agent card current task.

```json
{
  "type": "agent.task.changed",
  "agent_id": "coder",
  "verb": "Editing",
  "target": "src/auth/session.rs",
  "detail": "Refactoring token refresh logic"
}
```

### `agent.renamed`

Confirms rename and updates all surfaces.

```json
{
  "type": "agent.renamed",
  "agent_id": "main",
  "display_name": "CADIS"
}
```

### `message.delta`

Streams assistant text.

```json
{
  "type": "message.delta",
  "session_id": "ses_123",
  "agent_id": "main",
  "agent_name": "CADIS",
  "text": "I found the failing test",
  "content_kind": "chat",
  "final": false
}
```

### `message.completed`

Marks final assistant output.

```json
{
  "type": "message.completed",
  "session_id": "ses_123",
  "agent_id": "main",
  "agent_name": "CADIS",
  "text": "I found the failing test and opened the code window.",
  "content_kind": "summary"
}
```

### `approval.requested`

Creates approval card.

```json
{
  "type": "approval.requested",
  "approval_id": "apr_123",
  "risk_class": "git-force-push",
  "agent_id": "coder",
  "title": "Approval needed",
  "reason": "Force push to protected branch",
  "command": "git push --force origin main",
  "workspace": "/home/user/Project/app",
  "expires_at": "2026-04-26T12:05:00Z"
}
```

### `approval.resolved`

Removes or updates approval card.

```json
{
  "type": "approval.resolved",
  "approval_id": "apr_123",
  "verdict": "deny",
  "resolved_by": "telegram"
}
```

### `worker.event`

Updates worker tree and optional transient worker card.

```json
{
  "type": "worker.event",
  "worker_id": "worker_auth_01",
  "parent_agent_id": "coder",
  "status": "running",
  "cli": "cadis",
  "cwd": "/home/user/Project/app",
  "text": "running cargo test",
  "updated_at": 1770000000000
}
```

### `orchestrator.route`

Adds route transparency row.

```json
{
  "type": "orchestrator.route",
  "id": "route_123",
  "source": "hud-chat",
  "target": "coder",
  "reason": "@coder prefix"
}
```

### `ui.preferences.updated`

Confirms settings persisted by daemon.

```json
{
  "type": "ui.preferences.updated",
  "preferences": {
    "hud": {
      "theme": "arc",
      "background_opacity": 82
    }
  }
}
```

### `voice.preview.started`, `voice.preview.completed`, `voice.preview.failed`

Drive voice test UI.

```json
{
  "type": "voice.preview.completed",
  "provider": "edge",
  "voice_id": "id-ID-GadisNeural"
}
```

## 7. RamaClaw Topic Mapping

| RamaClaw topic/message | CADIS request/event |
| --- | --- |
| `user.message` | `message.send` |
| `agent.model` | `agent.model.set` |
| `agent.rename` | `agent.rename` |
| `approval.respond` | `approval.respond` |
| `models.list` | `models.list` |
| `models.list.response` | `models.list.response` |
| `agent.*.status` | `agent.status.changed` |
| `agent.*.task` | `agent.task.changed` |
| `session.*.message` | `message.delta` / `message.completed` |
| `worker.*.event` | `worker.event` |
| `approval.requested` | `approval.requested` |
| `approval.resolved` | `approval.resolved` |
| `orchestrator.route` | `orchestrator.route` |

## 8. Connection Behavior

HUD connection behavior should preserve RamaClaw's operational feel:

- connect to local daemon only
- perform protocol handshake
- subscribe to event stream
- request model list after handshake
- reconnect with exponential backoff
- keep last subscription set
- never log tokens

CADIS replacement discovery:

```text
1. CADIS_HUD_SOCKET or CADIS_HUD_URL
2. ~/.cadis/cadisd.sock
3. ~/.cadis/hud-gateway.port, if WebSocket mode is enabled
4. dev override
```

## 9. Voice Routing Policy

The HUD must speak only speakable content:

| Content kind | Speak |
| --- | --- |
| chat | yes if short and auto-speak enabled |
| summary | yes |
| approval | risk summary only |
| error | short actionable error |
| code | no |
| diff | no |
| terminal_log | no |
| test_result | short summary only |

## 10. Validation

Protocol adaptation is valid when:

- HUD can render from a mock CADIS event stream.
- All RamaClaw UI features have CADIS request/event equivalents.
- UI preferences persist through daemon config, not localStorage.
- Approval card lifecycle is server-confirmed.
- Rename and model selection survive HUD restart.
- Disconnection and reconnect behavior are visible and tested.

