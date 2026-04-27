# Risk Register

## Risk Summary

CADIS has high potential but also high risk because it will execute local tools, manage credentials, edit code, run shell commands, and coordinate autonomous agents. The core risk strategy is to keep authority centralized and observable.

## Register

| ID | Risk | Impact | Likelihood | Mitigation | Owner |
| --- | --- | --- | --- | --- | --- |
| R-001 | Scope expands before daemon is stable | High | High | Follow implementation order; UI after runtime | Maintainer |
| R-002 | OpenClaw assumptions leak into new core | Medium | Medium | Keep core fresh; use adapters only if needed | Maintainer |
| R-003 | Third-party source import causes license issue | High | Medium | ADR and license review before import | Maintainer |
| R-004 | Tool approval bypass | Critical | Medium | Central policy engine; tests; no adapter tool execution | Security |
| R-005 | Secrets appear in logs or commits | Critical | Medium | Redaction before write; tests; safe examples; secret scan before publish | Security |
| R-006 | Shell tool damages system | Critical | Medium | Risk classes; approvals; workspace constraints | Core |
| R-007 | Agent fan-out consumes resources | High | Medium | Depth, child, global, timeout, and budget limits | Core |
| R-008 | Parallel agents conflict on files | High | Medium | Mandatory worktree isolation for coding workers | Workers |
| R-009 | Model provider changes break runtime | Medium | Medium | Provider abstraction and conformance tests | Models |
| R-010 | UI becomes core logic | High | Medium | Protocol-only UI rule; architecture review | UI |
| R-011 | Telegram bot token leak | High | Low | Env config, no logs, docs, redaction | Integrations |
| R-012 | Voice speaks sensitive content | High | Medium | Speech policy; content kind routing | Voice |
| R-013 | Persistence corruption | Medium | Medium | Atomic writes; append-only logs; recovery metadata | Store |
| R-014 | Local protocol incompatible too early | Medium | Medium | Version protocol from first crate | Protocol |
| R-015 | Dependency bloat slows runtime | Medium | Medium | Dependency review; optional features | Maintainer |
| R-016 | Desktop framework choice delays core | Medium | Medium | Delay HUD until runtime is stable | UI |
| R-017 | Users expect production readiness too early | Medium | High | Clear status labels and known limitations | Maintainer |
| R-018 | Windows/macOS differences are underestimated | Medium | Medium | Linux-first scope; platform baseline matrix; adapters later | Platform |
| R-019 | Model tool-call formats differ too much | Medium | Medium | Capability metadata; fallback protocol | Models |
| R-020 | Tests lag behind security behavior | High | Medium | Require tests for policy, tools, store | Maintainer |

## Top Risks to Address Before v0.1

1. Approval bypass.
2. Secret leakage.
3. Runtime/UI scope creep.
4. Unclear protocol boundaries.
5. Provider abstraction too coupled to one vendor.

## Top Risks to Address Before Public Alpha

1. Worktree cleanup and patch application safety.
2. Telegram approval race behavior.
3. Voice content leakage.
4. Dependency license audit.
5. Crash recovery expectations.

## Risk Review Cadence

- Review this document before each minor release.
- Add new risks when adding integrations, providers, or tools.
- Treat security-sensitive risks as release blockers.
