---
name: cadis-ramaclaw-ui
description: Use when implementing or reviewing CADIS HUD, RamaClaw UI adaptation, config window, agent rename, theme picker, voice/model settings, orbital HUD, approval cards, screenshot parity, or desktop window behavior.
---

# CADIS RamaClaw UI

## Read First

- `docs/20_RAMACLAW_UI_ADAPTATION.md`
- `docs/21_UI_FEATURE_PARITY_CHECKLIST.md`
- `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`
- `docs/23_UI_DESIGN_SYSTEM.md`
- `docs/11_DECISIONS.md`

## Rules

- Preserve RamaClaw feature parity.
- Replace OpenClaw paths and protocol names with CADIS equivalents.
- Keep durable state in `cadisd`, not browser localStorage.
- UI must be a protocol client only.
- Approval cards disappear only after daemon resolution.
- Code, diffs, logs, and tests must not flood the main chat.

## Must Preserve

- orbital HUD
- central CADIS orb
- status bar
- chat and voice panel
- approval stack
- unified config dialog
- agent rename
- six themes
- background opacity
- voice picker and test
- per-agent model selector
- worker tree

## Validation

- Screenshot parity at 1600x1000 and 1920x1080.
- All six themes.
- No OpenClaw text or path remains.
- No overlapping text or cards.

Use the installed `playwright` and `screenshot` skills for UI validation.

