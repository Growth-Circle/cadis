---
name: cadis-protocol
description: Use when changing CADIS protocol messages, event schemas, content routing, HUD protocol, approval payloads, model catalog events, or compatibility rules.
---

# CADIS Protocol

## Read First

- `docs/15_PROTOCOL_DRAFT.md`
- `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`
- `docs/05_ARCHITECTURE.md`

## Rules

- Version the protocol.
- Keep events typed and serializable.
- Include event ID, timestamp, source, and session ID when applicable.
- Do not use OpenClaw topic names as CADIS canonical names.
- Add compatibility tests for schema changes.
- Update docs before or with implementation.

## Required UI Requests

- `message.send`
- `agent.rename`
- `agent.model.set`
- `models.list`
- `ui.preferences.get`
- `ui.preferences.set`
- `voice.preview`
- `voice.stop`
- `approval.respond`

## Required Event Families

- session lifecycle
- message delta and completion
- agent status, task, rename, model
- worker lifecycle
- approval request and resolution
- UI preferences
- voice preview
- errors

## Validation

- Serialization tests.
- Unknown or incompatible version tests.
- Request and event examples in docs.

