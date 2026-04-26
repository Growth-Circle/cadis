---
name: cadis-voice
description: Use when implementing or reviewing CADIS voice output, TTS providers, STT, wake word, voice preferences, speech routing policy, auto-speak, voice preview, or RamaClaw voice migration.
---

# CADIS Voice

## Read First

- `docs/03_FRD.md`
- `docs/16_CONFIG_REFERENCE.md`
- `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`
- `docs/23_UI_DESIGN_SYSTEM.md`

## Rules

- Voice is controlled by daemon policy and content kind.
- Do not speak long code, diffs, terminal logs, or raw test output.
- Approval speech is a short risk summary only.
- Voice preview must be cancellable.
- Voice settings persist through CADIS config.
- Node-based Edge TTS can be an optional compatibility path, not core daemon dependency.

## Initial Voice Preferences

- `voice_id`
- `rate`
- `pitch`
- `volume`
- `auto_speak`
- `provider`
- `max_spoken_chars`

## Validation

- Preview start, stop, success, and failure.
- Auto-speak final response only.
- Speech routing by content kind.
- Redaction before speech if sensitive data is possible.

Use installed `speech` and `transcribe` skills when the task involves audio workflows.

