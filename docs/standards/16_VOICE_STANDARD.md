# CADIS Voice Standard

## 1. Purpose

This standard defines CADIS voice behavior for text-to-speech, voice preferences, previews, HUD state, and safe content routing. Voice is an optional feature and must not become a dependency for core daemon, CLI, tool, or policy operation.

## 2. Ownership

Voice preferences are durable daemon-owned configuration.

The HUD may keep ephemeral preview state, but it must use daemon requests for durable changes and voice actions unless a later architecture decision explicitly makes preview HUD-owned.

## 3. Provider Contract

CADIS must define a TTS provider trait before adding production voice engines.

Provider responsibilities:

- synthesize short speakable text
- report supported voices
- apply rate, pitch, and volume preferences
- support stop or cancellation where possible
- return structured errors
- avoid logging sensitive text unnecessarily

Initial provider options:

- `edge`
- `os`
- `piper`
- `stub`

The `stub` provider is required for deterministic tests.

## 4. Voice Preferences

Voice preferences belong in `~/.cadis/config.toml`.

Required fields:

```text
enabled
provider
voice_id
rate
pitch
volume
auto_speak
max_spoken_chars
```

Recommended ranges:

```text
rate       -50..50, step 5
pitch      -50..50, step 5
volume     -50..50, step 5
```

Default voice:

```text
id-ID-GadisNeural
```

## 5. Curated Voice Catalog

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

The catalog should remain curated and small for the HUD. Advanced provider voice discovery may be added separately.

## 6. Speakable Content Policy

CADIS must speak only content that is useful and safe to hear.

| Content Kind | Speak |
| --- | --- |
| chat | yes if short and auto-speak is enabled |
| summary | yes |
| approval | risk summary only |
| error | short actionable error |
| code | no |
| diff | no |
| terminal_log | no |
| test_result | short summary only |

Long answers must be summarized before speaking. Long code, diffs, logs, and raw command output must not be spoken.

## 7. Auto-Speak Rules

Auto-speak must:

- wait for final assistant output
- respect `enabled`
- respect `auto_speak`
- respect `max_spoken_chars`
- skip code, diff, and terminal logs
- stop current speech before starting a new preview or explicit speech action

Streaming deltas must not be spoken one by one.

## 8. Approval Speech

Approval speech is allowed only as a concise risk summary.

It may include:

- action category
- agent display name
- workspace
- risk class
- short reason

It must not read long commands, secrets, diffs, or full logs aloud.

## 9. HUD Voice UI

The HUD voice UI must include:

- voice state machine: idle, listening, thinking, speaking
- voice selector
- rate slider
- pitch slider
- volume slider
- auto-speak toggle
- engine label
- test voice button
- stop test button
- error message
- last engine success hint

Voice preview should use the main agent display name in the sample text.

## 10. Protocol

Voice requests:

```text
voice.preview
voice.stop
ui.preferences.set
```

Voice events:

```text
voice.preview.started
voice.preview.completed
voice.preview.failed
ui.preferences.updated
```

Voice preview failure must be visible in the HUD and structured in logs.

## 11. Privacy and Logging

Voice logs must not store unnecessary spoken text.

Rules:

- Do not log provider credentials.
- Do not log full text for code, diffs, logs, or secret-bearing content.
- Redact before persistence.
- Prefer metadata such as content kind, character count, provider, voice ID, and outcome.

## 12. Testing Requirements

Required tests:

- voice preference serialization
- default voice selection
- curated catalog shape
- speakable content routing
- long answer summarization trigger
- code, diff, and log suppression
- approval risk summary routing
- preview started/completed/failed events
- stop behavior
- HUD control state mapping
