# RamaClaw UI Adaptation Guide

## 1. Purpose

CADIS will adapt the RamaClaw HUD UI direction as the canonical desktop HUD experience.

The target is 100% product and interaction parity, not necessarily 100% source-code reuse. RamaClaw is a Tauri + React implementation tied to OpenClaw. CADIS is a daemon-first Rust runtime. The UI must therefore be adapted through CADIS protocol, config, state, and policy boundaries.

## 2. Source References

Set this locally when auditing or porting:

```bash
export RAMACLAW_HUD_SOURCE=/path/to/ramaclaw-hud
export RAMACLAW_SPEC_SOURCE=/path/to/ramaclaw-specs
```

Important source files:

```text
$RAMACLAW_HUD_SOURCE/src/App.tsx
$RAMACLAW_HUD_SOURCE/src/lib/store.ts
$RAMACLAW_HUD_SOURCE/src/lib/gateway.ts
$RAMACLAW_HUD_SOURCE/src/lib/agents-roster.ts
$RAMACLAW_HUD_SOURCE/src/styles/themes.ts
$RAMACLAW_HUD_SOURCE/src/styles/globals.css
$RAMACLAW_HUD_SOURCE/src/ui/orbital/OrbitalHUD.tsx
$RAMACLAW_HUD_SOURCE/src/ui/orbital/RamaOrb.tsx
$RAMACLAW_HUD_SOURCE/src/ui/orbital/AgentWidget.tsx
$RAMACLAW_HUD_SOURCE/src/ui/orbital/WorkerTree.tsx
$RAMACLAW_HUD_SOURCE/src/ui/chat/ChatPanel.tsx
$RAMACLAW_HUD_SOURCE/src/ui/approvals/ApprovalCard.tsx
$RAMACLAW_HUD_SOURCE/src/ui/settings/ConfigDialog.tsx
$RAMACLAW_HUD_SOURCE/src/ui/settings/AgentRenameDialog.tsx
$RAMACLAW_HUD_SOURCE/src/ui/settings/VoiceConfig.tsx
$RAMACLAW_HUD_SOURCE/src/ui/settings/ModelsConfig.tsx
$RAMACLAW_HUD_SOURCE/src/ui/settings/ThemePicker.tsx
$RAMACLAW_HUD_SOURCE/src/ui/WindowChrome.tsx
$RAMACLAW_HUD_SOURCE/src/ui/wizard/Wizard.tsx
$RAMACLAW_HUD_SOURCE/src-tauri/src/lib.rs
$RAMACLAW_HUD_SOURCE/src-tauri/tauri.conf.json

$RAMACLAW_SPEC_SOURCE/RamaClaw Desktop HUD.html
$RAMACLAW_SPEC_SOURCE/shared-ui.jsx
$RAMACLAW_SPEC_SOURCE/variation-orbital.jsx
$RAMACLAW_SPEC_SOURCE/backend-spec.jsx
$RAMACLAW_SPEC_SOURCE/agents-data.jsx
$RAMACLAW_SPEC_SOURCE/uploads/pasted-1777098520186-0.png
```

Before publishing CADIS publicly, rewrite this section to point to a public design package, screenshot set, or committed UI reference files.

## 3. Adaptation Definition

100% adaptation means CADIS preserves:

- orbital HUD composition
- central CADIS orb behavior
- agent satellite cards
- worker tree under each agent
- status bar
- bottom chat and voice command panel
- pending approval stack
- unified config window
- agent rename flow
- voice picker and voice test flow
- model picker per agent
- theme picker and opacity control
- frameless transparent desktop window behavior
- first-run wizard intent
- gateway event semantics, translated to CADIS protocol

100% adaptation does not mean:

- keep OpenClaw paths
- keep OpenClaw gateway topic names
- keep browser localStorage as source of truth
- make Node.js a CADIS core dependency
- force Tauri if the final CADIS HUD uses Dioxus

## 4. UI Surfaces

### HUD Shell

The main shell must compose:

- custom window chrome
- status bar
- orbital canvas
- approval stack overlay
- chat panel
- config dialog
- agent rename dialog

RamaClaw reference composition:

```text
WindowChrome
StatusBar
OrbitalHUD
ApprovalStack
ChatPanel
ConfigDialog
AgentRenameDialog
```

CADIS equivalent:

```text
CadisWindowChrome
CadisStatusBar
CadisOrbitalHud
CadisApprovalStack
CadisChatPanel
CadisConfigDialog
CadisAgentRenameDialog
```

### Orbital HUD

The orbital HUD must use a 16:9 logical coordinate system. RamaClaw uses 1920x1080 and places the central orb around the visual center, with 12 perimeter slots for agents.

CADIS should keep:

- central orb
- two faint orbital rings
- dashed spokes from orb to agents
- non-overlapping perimeter agent slots
- model, context, mode, and voice readouts around orb
- state-driven orb animation

### Status Bar

The status bar must show:

- product/system name using current main agent display name
- daemon/gateway connection state
- selected main model
- active/waiting/idle counts
- optional system stats later

### Chat and Voice Panel

The bottom panel must show:

- streaming chat log
- user, assistant, and system messages
- voice status
- quick action chips
- mic button
- voice settings button
- model settings button
- textarea command composer

CADIS must replace OpenClaw wording with CADIS wording.

### Approval Stack

Approval cards must remain synchronized with daemon approval state. A click must send a response to `cadisd`; the card is removed only after `approval.resolved` arrives.

### Config Dialog

Config dialog tabs:

- Voice
- Models
- Appearance
- Window

This unified dialog replaces separate one-off panels.

### Agent Rename Dialog

Agent rename opens from right-click/context action on:

- central orb for main agent
- agent satellite card for subagents

The rename must:

- trim whitespace
- collapse repeated whitespace
- cap at 32 characters
- fall back to `CADIS` for main or role default for subagents if blank
- persist through daemon config/state
- emit `agent.renamed` event

## 5. Feature Adaptation Map

| RamaClaw feature | RamaClaw source | CADIS adaptation |
| --- | --- | --- |
| Zustand HUD store | `src/lib/store.ts` | `cadisd` owns durable state; UI keeps ephemeral view model |
| Local voice prefs | `ramaclaw.voicePrefs.v2` | `~/.cadis/config.toml` and `ui.preferences.updated` |
| Local agent names | `ramaclaw.agentNames.v1` | `agent.rename` request plus daemon persistence |
| Local background opacity | `ramaclaw.backgroundOpacity.v1` | `hud.appearance.background_opacity` |
| Local chat prefs | `ramaclaw.chatPreferences.v1` | `chat.preferences` in daemon config/session |
| Local agent models | `ramaclaw.agentModels.v1` | per-agent model config in daemon |
| OpenClaw model catalog | `models.list.response` | `models.list` response from `cadisd` |
| OpenClaw approvals | `approval.respond` | `approval.resolve` or `approval.respond` normalized by CADIS protocol |
| OpenClaw gateway | `ramaclaw-hud.v1` | `cadis-hud.v1` or CADIS protocol version |
| Tauri Edge TTS command | `edge_tts_speak` | CADIS voice provider command or daemon voice service |

## 6. Required CADIS Protocol Additions

Add these requests:

```text
agent.rename
agent.model.set
agent.specialist.set
models.list
ui.preferences.get
ui.preferences.set
voice.preview
voice.stop
window.preference.set
```

Add or confirm these events:

```text
agent.renamed
agent.model.changed
agent.specialist.changed
models.list.response
ui.preferences.updated
voice.preview.started
voice.preview.completed
voice.preview.failed
window.preference.updated
```

Existing events that must drive HUD:

```text
daemon.status
session.started
message.delta
message.completed
agent.spawned
agent.status.changed
agent.task.changed
worker.started
worker.log.delta
worker.completed
approval.requested
approval.resolved
orchestrator.route
```

## 7. State Ownership Rules

CADIS must not copy RamaClaw's localStorage-first model.

Use this split:

| State | Owner | UI cache allowed |
| --- | --- | --- |
| daemon connection | UI | yes |
| active chat draft | UI | yes |
| transcript view scroll | UI | yes |
| selected config tab | UI | yes |
| theme | daemon config | yes |
| background opacity | daemon config | yes |
| voice prefs | daemon config | yes |
| agent display names | daemon config/session store | yes |
| per-agent model | daemon config | yes |
| approvals | daemon | yes, event-derived only |
| workers | daemon | yes, event-derived only |
| agent statuses | daemon | yes, event-derived only |

## 8. Visual Requirements

CADIS HUD must preserve:

- dark transparent glass window
- OKLCH hue-based theming
- low-radius panels
- mono operational labels
- central high-fidelity orb
- subtle grid, scanlines, dashed spokes
- compact readable cards
- glow used for state only, not decoration

The UI should feel like an operational desktop HUD, not a marketing dashboard.

## 9. Technical Strategy

There are two viable implementation paths.

### Path A: Exact UI First With Tauri + React

Use if the priority is fastest visual parity.

Pros:

- can reuse most RamaClaw UI code
- easiest to match CSS, layout, animations
- faster to validate screenshots

Cons:

- adds Node/React/Tauri to HUD app
- conflicts with earlier Dioxus preference
- must ensure core remains Rust daemon and UI remains a client

### Path B: Port To Dioxus With RamaClaw As Reference

Use if the priority is Rust-first UI consistency.

Pros:

- matches existing CADIS Rust-first direction
- avoids React as long-term UI dependency
- cleaner Rust workspace story

Cons:

- slower to reach exact visual parity
- all CSS/animation behavior must be reimplemented
- screenshot parity will require more work

Recommendation:

- Keep `cadisd` core independent.
- Decide UI toolkit with an ADR before implementation.
- If the user wants "100% UI" fastest, use Tauri + React for HUD v0.6 while keeping daemon protocol clean.
- If the user wants "100% Rust" strongest, port to Dioxus and accept slower visual parity.

## 10. Acceptance Criteria

CADIS UI adaptation is complete when:

- screenshot parity passes for main HUD at 1600x1000 and 1920x1080
- all six themes work live
- config dialog has Voice, Models, Appearance, Window tabs
- main and subagent rename works and persists
- voice picker lists bilingual voices and preview works
- per-agent model picker updates daemon
- background opacity changes live and persists
- approval card waits for daemon resolution before disappearing
- workers appear under their parent agent
- chat panel sends messages through `cadisd`
- auto-speak follows CADIS content routing policy
- no UI client executes tools directly
- no OpenClaw file path remains in CADIS runtime docs or code
