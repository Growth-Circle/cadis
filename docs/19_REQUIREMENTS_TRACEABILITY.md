# Requirements Traceability Matrix

## Purpose

This matrix links product requirements to functional requirements and implementation phases. It exists so the checklist can become GitHub issues without losing the original product intent.

## Matrix

| Product requirement | Functional coverage | Implementation phase | Verification |
| --- | --- | --- | --- |
| PRD-001: local daemon named `cadisd` | FRD-DAEMON-001 to FRD-DAEMON-009 | P3 | daemon startup integration test |
| PRD-002: CLI named `cadis` | FRD-CLI-001 to FRD-CLI-010 | P4 | CLI command tests |
| PRD-003: interfaces use daemon protocol | FRD-PROTO-001 to FRD-PROTO-009 | P2, P4, P11, P13 | protocol compatibility tests |
| PRD-004: stream status before final output | FRD-PROTO-002, FRD-OBS-001 to FRD-OBS-005 | P2, P3, P5 | time-to-first-event test |
| PRD-005: multiple model providers | FRD-MODEL-001 to FRD-MODEL-010 | P5 | provider conformance tests |
| PRD-006: risky tools use approval policy | FRD-POLICY-001 to FRD-POLICY-010 | P7 | allow/deny/expire tests |
| PRD-007: approval state shared across clients | FRD-PROTO-004, FRD-POLICY-009, FRD-TG-008, FRD-HUD-004 | P7, P11, P13 | multi-client approval test |
| PRD-008: code tasks support visual output | FRD-CODE-001 to FRD-CODE-005 | P14 | code window routing test |
| PRD-009: parallel coding uses isolation | FRD-AGENT-004, FRD-WORKER items in checklist | P10 | worktree isolation test |
| PRD-010: logs redact secrets | FRD-STORE-004, FRD-STORE-007 | P8 | redaction tests |
| PRD-011: Linux desktop first | FRD-HUD-001 | P13 | Linux build and smoke test |
| PRD-012: open-source files before release | Repository foundation checklist | P0 | repo hygiene CI |

## Release Blocking Requirements

### v0.1

- PRD-001
- PRD-002
- PRD-003
- PRD-004
- PRD-005 with one provider
- PRD-010

### v0.2

- PRD-006
- PRD-007 for CLI

### v0.4

- PRD-008
- PRD-009

### Public Alpha

- PRD-006
- PRD-007 across CLI and Telegram
- PRD-010
- threat model updated
- dependency license audit completed

