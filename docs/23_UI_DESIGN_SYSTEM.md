# UI Design System

## 1. Direction

CADIS desktop HUD adopts the RamaClaw orbital HUD design language:

- dark transparent desktop HUD
- central animated orb
- agent satellites around the center
- dense operational status
- mono labels and low-radius cards
- hue-swappable OKLCH theme system
- code, voice, approvals, workers, and chat visible without marketing layout

## 2. Visual Principles

- Operational, not decorative.
- Dense but readable.
- Motion communicates state.
- Color communicates state and theme.
- Panels are compact and low-radius.
- Chat remains secondary to the orchestration HUD.
- Long code output belongs in code work window, not main HUD.

## 3. Theme Tokens

All theme palettes derive from one hue token:

```css
:root {
  --hue: 210;
  --bg: rgba(8, 7, 6, 0.78);
  --bg-solid: #050403;
  --panel: oklch(0.16 0.025 var(--hue) / 0.78);
  --panel-2: oklch(0.20 0.03 var(--hue) / 0.82);
  --border: oklch(0.32 0.06 var(--hue) / 0.55);
  --border-d: oklch(0.24 0.045 var(--hue) / 0.45);
  --text: oklch(0.86 0.02 var(--hue));
  --dim: oklch(0.62 0.04 var(--hue));
  --faint: oklch(0.45 0.04 var(--hue));
  --accent: oklch(0.78 0.16 var(--hue));
  --accent-d: oklch(0.62 0.14 var(--hue));
  --ok: oklch(0.78 0.16 145);
  --warn: oklch(0.78 0.16 70);
  --err: oklch(0.70 0.18 25);
}
```

Theme presets:

| Key | Label | Hue | Use |
| --- | --- | --- | --- |
| `arc` | ARC REACTOR | 210 | default cool blue |
| `amber` | AMBER | 38 | warm HUD mode |
| `phosphor` | PHOSPHOR | 145 | terminal green |
| `violet` | VIOLET | 290 | cyberdeck violet |
| `alert` | ALERT | 18 | incident mode |
| `ice` | ICE | 235 | focus mode |

## 4. Typography

Use two font stacks:

```css
--mono: "JetBrains Mono", ui-monospace, monospace;
--sans: "Inter", system-ui, sans-serif;
```

Guidelines:

- Mono for status, labels, telemetry, buttons, IDs, and command surfaces.
- Sans for longer readable content.
- Avoid oversized hero typography.
- Keep button text short.
- Use dynamic sizing for central orb brand text.

## 5. Layout

Base shell:

```text
window chrome: 30px
status bar:   44px
orbital HUD:  flexible
chat panel:   280px
```

Orbital canvas:

```text
logical size: 1920x1080
aspect ratio: 16:9
central orb: near center
agent slots: 12 perimeter slots
```

RamaClaw implementation uses 12 non-overlapping slots:

```text
top row:      4 agents
side columns: 4 agents
bottom row:   4 agents
```

CADIS should keep this slot approach because it prevents overlap and keeps the central orb clear.

## 6. Core Components

### `CadisWindowChrome`

Responsibilities:

- drag region
- title
- configure button
- always-on-top toggle
- minimize
- close

### `CadisStatusBar`

Responsibilities:

- system brand
- daemon connection
- main model
- active/waiting/idle counts
- optional latency and telemetry

### `CadisOrbitalHud`

Responsibilities:

- 16:9 canvas
- spokes
- rings
- agent placement
- central orb
- meta ring

### `CadisOrb`

Responsibilities:

- state animation
- central display name
- state label
- ring animation
- voice waveform bars
- context action for rename

States:

```text
idle
listening
thinking
speaking
working
waiting
```

### `CadisAgentCard`

Responsibilities:

- icon
- display name
- status
- role
- current task
- model/detail
- worker tree
- context action for rename

### `CadisWorkerTree`

Responsibilities:

- list child workers under parent agent
- show worker status
- show worker ID
- show last text
- show agent-session step progress when available
- show compact worker progress without expanding card height unpredictably
- collapse/expand

### `CadisChatPanel`

Responsibilities:

- message log
- quick commands
- mic button
- config shortcuts
- composer
- voice status
- auto-scroll

### `CadisApprovalCard`

Responsibilities:

- risk class
- agent
- command/action
- workspace
- reason
- expiry
- approve/deny
- wait for daemon resolution

### `CadisConfigDialog`

Tabs:

- Voice
- Models
- Appearance
- Window

### `CadisAgentRenameDialog`

Responsibilities:

- edit display name
- validate and normalize
- submit to daemon
- show disconnected warning if needed

### `CadisFirstRunWizard`

Steps:

- theme
- voice mode
- Telegram fallback
- approval timeout
- hotkey

## 7. Voice Catalog

Initial curated voices:

| ID | Label | Locale | Gender |
| --- | --- | --- | --- |
| `id-ID-ArdiNeural` | Ardi (Indonesian, Male) | id-ID | Male |
| `id-ID-GadisNeural` | Gadis (Indonesian, Female) | id-ID | Female |
| `ms-MY-OsmanNeural` | Osman (Malay, Male) | ms-MY | Male |
| `ms-MY-YasminNeural` | Yasmin (Malay, Female) | ms-MY | Female |
| `en-US-AvaNeural` | Ava (US, Female) | en-US | Female |
| `en-US-AndrewNeural` | Andrew (US, Male) | en-US | Male |
| `en-US-EmmaNeural` | Emma (US, Female) | en-US | Female |
| `en-US-BrianNeural` | Brian (US, Male) | en-US | Male |
| `en-GB-SoniaNeural` | Sonia (GB, Female) | en-GB | Female |
| `en-GB-RyanNeural` | Ryan (GB, Male) | en-GB | Male |

Default:

```text
id-ID-GadisNeural
```

Voice prefs:

```text
voice_id
rate       -50..50, step 5
pitch      -50..50, step 5
volume     -50..50, step 5
auto_speak boolean
provider   edge/os/piper/stub
```

## 8. Interaction Rules

- Right-click central orb opens main agent rename.
- Right-click agent card opens agent rename.
- Theme changes live.
- Background opacity changes live.
- Model selection sends daemon update immediately.
- Voice test stops current speech before starting preview.
- Chat auto-speak waits for final response.
- Approval card remains until daemon resolution.
- Disconnected UI keeps controls visible but disables unsafe actions.

## 9. Accessibility and Responsiveness

- All icon buttons need accessible labels.
- Theme swatches need labels/tooltips.
- Sliders need visible values.
- Dialogs close on backdrop and explicit close button.
- Text must not overflow cards; compact long model names.
- Central orb name dynamically sizes.
- Minimum desktop size: 1200x760.
- Preferred desktop size: 1600x1000.
- Target screenshot parity: 1920x1080.

## 10. Screenshots and Visual QA

Required screenshot checks:

- HUD empty connected state.
- HUD disconnected state.
- HUD with active agents and workers.
- Config dialog Voice tab.
- Config dialog Models tab.
- Config dialog Appearance tab.
- Agent rename dialog.
- Approval stack with two cards.
- All six themes.

Canvas/pixel checks:

- central orb is nonblank and centered
- agent cards do not overlap central orb
- chat panel does not cover agent cards
- approval cards do not cover config dialog
- text fits in status bar at 1200px width
