# CADIS UI HUD Standard

## 1. Purpose

This standard defines the CADIS desktop HUD requirements. CADIS adapts the RamaClaw orbital HUD as the canonical desktop UI reference while preserving CADIS daemon ownership of protocol, state, policy, and tools.

The target is product and interaction parity, not source-code parity.

## 2. Architecture

The HUD is a client of `cadisd`.

Rules:

- The daemon owns durable state and operational state.
- The HUD may keep ephemeral render state.
- The HUD must not execute tools.
- The HUD must not approve actions locally.
- The HUD must not treat browser local storage as authoritative.
- All config writes must route through daemon protocol.
- All agent, model, approval, and voice state must be derived from daemon responses and events.

## 3. Required Shell

The main HUD shell must include:

- custom window chrome
- status bar
- orbital HUD
- approval stack overlay
- chat panel
- unified config dialog
- agent rename dialog

Component names should follow CADIS naming:

```text
CadisWindowChrome
CadisStatusBar
CadisOrbitalHud
CadisApprovalStack
CadisChatPanel
CadisConfigDialog
CadisAgentRenameDialog
```

## 4. Window Requirements

Desktop behavior:

- transparent frameless window
- preferred size around 1600x1000
- minimum size around 1200x760
- custom drag chrome
- configure button
- always-on-top toggle
- minimize
- close
- background opacity preference

Linux is the initial target. Toolkit selection may be Tauri + React for fastest parity or a Rust-first toolkit if it preserves the interaction contract.

## 5. Orbital HUD

The orbital HUD must use a 16:9 logical layout based on 1920x1080.

Required elements:

- central CADIS orb
- two faint orbital rings
- dashed spokes from orb to agents
- 12 non-overlapping perimeter slots
- agent satellite cards
- nested worker tree under each agent
- model, context, mode, and voice readouts around the orb
- state-driven orb animation

Allowed orb states:

```text
idle
listening
thinking
speaking
working
waiting
```

## 5.1 Wulan Avatar Engine

The default center avatar remains the CADIS orb. Wulan is an optional avatar
style and must follow `docs/26_WULAN_AVATAR_ENGINE.md`.

Rules:

- Wulan avatar selection must be daemon-backed through `hud.avatar_style`.
- The current Three.js Wulan Arc implementation is a HUD prototype and migration
  reference, not the long-term native engine boundary.
- `crates/cadis-avatar` owns the renderer-independent state engine and exposes
  `AvatarFrame`, `BodyGestureState`, `AvatarPrivacy`, and direct-wgpu uniform
  contract data without depending on `wgpu` or Bevy.
- The preferred native renderer is a focused Rust/wgpu engine. The renderer
  adapter should consume `WgpuAvatarUniforms` and `WgpuRendererContract`
  directly once the heavy renderer dependency is introduced.
- Bevy remains optional and deferred behind the future `bevy-renderer` feature
  unless CADIS accepts a broader 3D scene engine.
- Wulan must render from daemon-derived HUD state and disposable renderer state;
  it must not own sessions, agents, models, tools, policy, approvals, voice, or
  memory.
- Wulan must provide scripted body gestures for idle, listening, thinking,
  speaking, coding, approval, and error states.
- Body gestures must carry priority metadata so safety and approval states can
  interrupt decorative animation.
- Reduced-motion mode must disable large body gestures and preserve readable
  state with minimal color, opacity, or mouth/reticle changes.
- Renderer failure must fall back to the CADIS orb without blocking the HUD.

Optional face tracking:

- off by default
- explicit permission required before camera access
- local-only processing
- visible camera-active indicator
- no persisted frames, landmarks, embeddings, or biometric templates
- graceful fallback to scripted gestures when unavailable or denied

## 6. Status Bar

The status bar must show:

- current main agent display name
- daemon connection state
- selected main model
- active agent count
- waiting agent count
- idle agent count
- optional latency
- optional system stats later

Disconnected state must reference the CADIS daemon, not OpenClaw.

## 7. Chat Panel

The chat and voice panel must include:

- streaming chat log
- user, assistant, and system messages
- composer textarea
- Enter to send
- Shift+Enter for newline
- quick action chips
- voice status
- mic button
- voice settings button
- model settings button

The send action must use `message.send`. Sending must be disabled when disconnected unless an explicit offline queue is implemented.

## 8. Approval Stack

Approval cards must show:

- risk class or rule
- agent
- action or command
- cwd or workspace
- reason
- risk summary
- expiry, when available
- approve and deny buttons

Button clicks must send `approval.respond`. Cards must remain visible until `approval.resolved` arrives from the daemon.

## 9. Config Dialog

CADIS must use one unified config dialog with these tabs:

- Voice
- Models
- Appearance
- Window

The selected tab may be UI-local session state. Saved preferences must go through `ui.preferences.set` or a more specific daemon request.

## 10. Agent Rename

Rename opens from:

- right-click or context action on the central orb for the main agent
- right-click or context action on an agent card for subagents

Rules:

- trim whitespace
- collapse repeated whitespace
- cap at 32 characters
- blank main-agent name falls back to `CADIS`
- blank subagent name falls back to role default
- submit through `agent.rename`
- persist in daemon state
- update surfaces from `agent.renamed`

## 11. Theme and Appearance

The HUD must support six themes:

| Key | Label |
| --- | --- |
| `arc` | ARC REACTOR |
| `amber` | AMBER |
| `phosphor` | PHOSPHOR |
| `violet` | VIOLET |
| `alert` | ALERT |
| `ice` | ICE |

Themes must be hue-driven OKLCH token sets. Theme and opacity changes must update live and persist through daemon config.

## 12. Accessibility and Responsiveness

Requirements:

- icon buttons have accessible labels
- theme swatches have labels or tooltips
- sliders show visible values
- dialogs close through backdrop and explicit close button
- text must not overflow cards
- long model names must compact gracefully
- central orb display name must resize dynamically
- no overlapping UI at 1200x760, 1600x1000, or 1920x1080

## 13. Protocol Requirements

The HUD must use CADIS protocol requests and events:

Requests:

```text
message.send
agent.rename
agent.model.set
models.list
approval.respond
ui.preferences.get
ui.preferences.set
voice.preview
voice.stop
window.preference.set
```

Events:

```text
daemon.status
models.list.response
agent.status.changed
agent.task.changed
agent.renamed
message.delta
message.completed
approval.requested
approval.resolved
worker.event
orchestrator.route
ui.preferences.updated
voice.preview.started
voice.preview.completed
voice.preview.failed
```

## 14. Open-Source Cleanup

Before public release:

- replace RamaClaw brand text with CADIS where used in implementation
- replace OpenClaw wording with CADIS daemon wording
- remove private source paths from user-facing docs
- confirm asset licensing
- recreate icons as CADIS assets when needed
- verify no provider keys or local config values are committed

## 15. Testing Requirements

Required HUD tests:

- theme helper tests
- agent name normalization tests
- agent rename protocol test
- voice prefs serialization tests
- config dialog render test
- approval card waits for resolved event
- reconnect and backoff test
- protocol event mapping tests
- screenshot parity at 1600x1000
- screenshot parity at 1920x1080
- minimum size check at 1200x760
- no OpenClaw text or path remains in UI
