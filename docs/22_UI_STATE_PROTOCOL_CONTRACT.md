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
avatar_style = "orb"
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

### `events.subscribe`

Sent when the HUD establishes a daemon connection that should receive runtime
events without polling.

```json
{
  "type": "events.subscribe",
  "since_event_id": "evt_000120",
  "replay_limit": 128,
  "include_snapshot": true
}
```

The daemon responds with `request.accepted`, then sends snapshot events, bounded
replay, and live events on the same connection. HUD should rebuild
`event_derived_snapshot` from those events and keep the last seen `event_id` for
reconnect.

In the Tauri HUD, the renderer remains a protocol client and does not open the
Unix socket directly. It calls the native `cadis_events_subscribe` command with
the same protocol request envelope. The native side keeps the socket open,
reads newline-delimited `ServerFrame` JSON from `cadisd`, and emits each frame
to the renderer as a `cadis-frame` Tauri event. If the socket closes
unexpectedly, native emits `cadis-subscription-closed`; the renderer marks the
gateway disconnected and reconnects with bounded backoff using the last seen
`event_id` as `since_event_id`.

The HUD may still use `cadis_request` for one-shot commands such as
`models.list`, `daemon.status`, `message.send`, and preference or approval
requests. Authoritative state changes must arrive through daemon events before
the UI treats them as applied.

### `events.snapshot`

Sent when the HUD needs a one-shot daemon-owned state snapshot.

```json
{
  "type": "events.snapshot"
}
```

The current desktop MVP snapshot is encoded as event frames, including
`agent.list.response`, `ui.preferences.updated`, and `session.updated`.

### `session.subscribe`

Sent when a client wants only one session's events rather than the daemon-wide
event stream.

```json
{
  "type": "session.subscribe",
  "session_id": "ses_...",
  "replay_limit": 128,
  "include_snapshot": true
}
```

The daemon responds with `request.accepted`, then sends the current
`session.updated` event, bounded replay, and live events whose envelope
`session_id` matches the request. The HUD may use this for focused session panes
while keeping daemon-wide `events.subscribe` as the main state feed.

### `message.send`

Sent when the user submits text in the chat panel.

```json
{
  "type": "message.send",
  "session_id": null,
  "target_agent_id": "codex",
  "content": "@codex fix the auth bug",
  "content_kind": "chat"
}
```

`target_agent_id` is optional. HUD may include it as a hint when a leading
`@agent` mention resolves locally, but `cadisd` remains authoritative and emits
`orchestrator.route` for the final route.

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
      "avatar_style": "wulan_arc",
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

### `voice.status`, `voice.doctor`, and `voice.preflight`

Sent by the HUD to read daemon-visible voice state and to publish local bridge
preflight results. The HUD still owns platform capture/playback mechanics; it
does not become authoritative for durable voice preferences, speech policy, or
agent routing.

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
      "name": "MediaRecorder",
      "status": "warn",
      "message": "recorder available; WebAudio PCM fallback remains armed"
    }
  ]
}
```

The local bridge preflight must include microphone permission/API state,
`MediaRecorder`, analyser, WebAudio PCM fallback, whisper binary/model,
Node helper, and audio player checks when available.

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

### `session.started`

Creates a visible session progress row before model output arrives.

```json
{
  "type": "session.started",
  "session_id": "ses_123",
  "title": "Fix auth test"
}
```

### `agent.list.response`

Replaces the seeded roster with daemon-owned agent state.

```json
{
  "type": "agent.list.response",
  "agents": [
    {
      "agent_id": "codex",
      "role": "Coding",
      "display_name": "Codex",
      "parent_agent_id": null,
      "model": "codex-cli/chatgpt-plan",
      "status": "idle"
    }
  ]
}
```

### `agent.spawned`

Adds a newly created agent or subagent to the HUD roster.

```json
{
  "type": "agent.spawned",
  "agent_id": "coding_1",
  "role": "Coding",
  "display_name": "Builder",
  "parent_agent_id": "main",
  "model": "codex-cli/chatgpt-plan",
  "status": "idle"
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

The optional `task` field on `agent.status.changed` drives the current task
summary. A separate `agent.task.changed` event is reserved for a later protocol
version.

### `agent.session.started` / `agent.session.updated` / terminal events

Tracks daemon-owned per-route agent runtime state. HUD may display these as
task details under the agent card, but it must treat `cadisd` as authoritative
for timeout, budget, cancellation, result, and parent-child metadata.

```json
{
  "type": "agent.session.started",
  "agent_session_id": "ags_000001",
  "session_id": "ses_123",
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

Terminal events are `agent.session.completed`, `agent.session.failed`, and
`agent.session.cancelled`. Status values are `started`, `running`, `completed`,
`failed`, `cancelled`, `timed_out`, and `budget_exceeded`.

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
  "delta": "I found the failing test",
  "content_kind": "chat",
  "agent_id": "main",
  "agent_name": "CADIS"
}
```

### `message.completed`

Marks final assistant output.

```json
{
  "type": "message.completed",
  "session_id": "ses_123",
  "content_kind": "summary",
  "content": "I found the failing test and opened the code window.",
  "agent_id": "main",
  "agent_name": "CADIS"
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

### `worker.started` / `worker.log.delta` / `worker.completed`

Updates worker tree and optional transient worker card.

```json
{
  "type": "worker.started",
  "worker_id": "worker_auth_01",
  "agent_id": "coding_1",
  "parent_agent_id": "coder",
  "status": "running",
  "cli": "cadis",
  "cwd": "/home/user/Project/app",
  "summary": "running cargo test"
}
```

`worker.log.delta` carries `worker_id`, `delta`, and optional `agent_id` /
`parent_agent_id`. `worker.completed` carries the same metadata plus optional
`summary`. `worker.failed` and `worker.cancelled`, when emitted by later daemon
phases, must flow through the same reducer. `worker.tail` returns recent
daemon-owned log lines as `worker.log.delta` events for an existing worker;
clients should apply those events through the same worker reducer used for live
updates.

HUD worker progress is derived from daemon events only. The worker tree may
combine `agent.session.*` progress (`steps_used` / `budget_steps`) with
`worker.*` status, log tail, worktree metadata, and artifact paths, but it must
not create, execute, cancel, or approve workers locally.

### `orchestrator.route`

Adds route transparency row.

```json
{
  "type": "orchestrator.route",
  "id": "route_123",
  "source": "hud-chat",
  "target_agent_id": "coder",
  "target_agent_name": "Codex",
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
      "avatar_style": "orb",
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

### `voice.status.updated`, `voice.doctor.response`, `voice.preflight.response`

Drive the voice status and doctor rows in the HUD config dialog.

```json
{
  "type": "voice.status.updated",
  "enabled": false,
  "state": "disabled",
  "provider": "edge",
  "voice_id": "id-ID-GadisNeural",
  "stt_language": "auto",
  "max_spoken_chars": 800,
  "bridge": "hud-local"
}
```

`voice.doctor.response` and `voice.preflight.response` wrap the same status with
`checks[]`, each containing `name`, `status`, and `message`. The HUD maps
daemon `ok`, `warn`, and `error` statuses to its existing pass/warn/fail doctor
presentation.

Daemon-visible TTS provider IDs are `edge`, `openai`, and `system`; `stub` is
reserved for deterministic tests. Current daemon providers are local stubs that
validate speech policy and emit lifecycle events without external API calls.

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
- send `events.subscribe` with the last seen event ID when available
- request model list after handshake
- reconnect with exponential backoff
- keep last subscription set
- never log tokens

CADIS replacement discovery:

```text
1. explicit socketPath argument passed to the Tauri cadis_request command
2. CADIS_HUD_SOCKET
3. CADIS_SOCKET
4. socket_path in ~/.cadis/config.toml
5. $XDG_RUNTIME_DIR/cadis/cadisd.sock when XDG_RUNTIME_DIR exists
6. ~/.cadis/run/cadisd.sock
7. Future CADIS_HUD_URL or hud-gateway.port if WebSocket mode is enabled
```

`VITE_CADIS_SOCKET_PATH` may seed browser-preview development state, but it is
not an authoritative runtime configuration source.

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

`cadisd` applies this policy before provider dispatch. Auto-speak waits for the
final `message.completed` event and may then emit `voice.started` and
`voice.completed` for short speakable content. Code, diffs, terminal logs, and
long raw tool or test output must not emit voice playback events.

## 10. Validation

Protocol adaptation is valid when:

- HUD can render from a mock CADIS event stream.
- HUD shows `session.started`, `orchestrator.route`,
  `agent.status.changed`, and `message.delta` progress before
  `message.completed`.
- HUD worker progress renders from the mock daemon worker stream fixture without
  a running agent runtime.
- All RamaClaw UI features have CADIS request/event equivalents.
- UI preferences persist through daemon config, not localStorage.
- Approval card lifecycle is server-confirmed.
- Rename and model selection survive HUD restart.
- Disconnection and reconnect behavior are visible and tested.
- `apps/cadis-hud` passes pnpm lint, typecheck, unit tests, frontend build, and
  `src-tauri` cargo check in CI.
