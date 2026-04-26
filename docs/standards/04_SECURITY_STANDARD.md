# Security Standard

## 1. Purpose

This standard defines CADIS security posture for local runtime development and open-source readiness.

CADIS follows an OpenSSF-style posture: secure defaults, reviewed dependencies, documented vulnerability reporting, tested controls, and clear release gates.

## 2. Security Principles

- Deny by default when policy cannot classify an action.
- Fail closed when approval state is missing, expired, duplicated, or corrupted.
- Treat model output, provider responses, Telegram messages, protocol input, and tool arguments as untrusted.
- Centralize policy and approval enforcement in `cadisd`.
- Redact secrets before logs, events, errors, snapshots, or diagnostics leave protected code paths.
- Prefer least privilege and local-only defaults.
- Make security-sensitive behavior observable without exposing secrets.

## 3. Protected Assets

CADIS must protect:

- provider API keys
- Telegram bot tokens
- local files and source repositories
- shell access
- approval state
- model context
- logs and persisted events
- worktrees and generated patches
- local protocol socket
- optional local face tracking signals and camera-derived avatar controls

## 4. Trust Boundaries

Primary boundary:

```text
user/client input -> daemon protocol -> daemon core -> policy -> tool runtime -> OS/filesystem/network
```

Rules:

- Validation repeats at the daemon even when clients validate first.
- Tool arguments are invalid until schema validation and policy classification pass.
- Provider errors are mapped and redacted before reaching users.
- Remote adapters such as Telegram are never trusted as local authority.
- UI clients cannot execute tools or resolve approvals without daemon confirmation.

## 5. Risk Classes

The standard risk classes are:

```text
safe-read
workspace-edit
network-access
secret-access
system-change
dangerous-delete
outside-workspace
git-push-main
git-force-push
sudo-system
```

Every tool declares risk class, expected side effects, workspace behavior, network use, and whether secrets may be read.

## 6. Approval Security

- Approval requests are created before execution.
- Approval records include ID, risk class, tool call ID, session ID, summary, expiration, and source.
- First valid response wins.
- Duplicate responses are audited and ignored.
- Denied and expired approvals do not execute.
- Approval resolution is persisted.
- All connected clients receive final approval state.

## 7. Secret Handling

- Store raw secrets in environment variables or an OS keychain when implemented.
- Do not store raw provider keys in config examples, event logs, tests, screenshots, or docs.
- Redact values whose names end in `_KEY`, `_TOKEN`, `_SECRET`, or `_PASSWORD`.
- Redact known provider key formats where practical.
- Do not echo resolved environment secrets in `doctor`, debug mode, or protocol traces.
- Never include secrets in panic messages or public issue templates.

## 8. Tool Runtime Security

- Shell execution requires policy decision.
- Outside-workspace writes are blocked or approval-gated.
- Dangerous deletes require explicit approval.
- Secret reads require explicit approval.
- Sudo or system-level changes require explicit approval.
- Tool execution must support timeout and cancellation.
- Tool failures include actionable metadata after redaction.
- Tool tests cover allow, deny, timeout, cancellation, and malformed arguments.

## 9. Local Protocol Security

- Local-only transport is the default.
- Unix sockets should use restrictive permissions.
- Incompatible protocol versions are rejected.
- Unknown request types are rejected.
- Debug protocol logging is opt-in and redacted.
- Remote relay or WebSocket modes require a security review before public alpha.

## 10. Avatar and Face Tracking Privacy

Wulan face tracking is optional and must be treated as privacy-sensitive local
camera processing.

Rules:

- Face tracking is off by default and must not be required for Wulan
  expressiveness.
- Camera access requires explicit user permission, a visible active-camera
  indicator, and a one-click disable action.
- Face tracking data must remain local to the renderer process unless a future
  security decision explicitly changes that boundary.
- Raw frames, derived landmarks, embeddings, identity labels, biometric
  templates, and confidence traces must not be written to logs, diagnostics,
  crash reports, telemetry, memory, or provider context by default.
- Permission denial, camera unavailable, and low-confidence frames must fall
  back to scripted avatar gestures.
- `crates/cadis-avatar` privacy validation must reject non-local,
  persisted-by-default, or identity-recognition face tracking config.

## 11. Supply Chain Security

Before public alpha:

- CI runs formatting, linting, tests, and dependency/license checks.
- Dependencies are reviewed for license, maintenance, and known risk.
- Release artifacts use checksums.
- Vulnerability reporting path in `SECURITY.md` is current.
- Known limitations are documented.

Preferred checks align with OpenSSF Scorecard categories where practical:

- branch protection
- CI tests
- dependency update hygiene
- pinned GitHub Actions
- license clarity
- vulnerability reporting
- secret scanning

## 12. Security Gates

Pre-alpha gates:

- redaction tests pass
- policy deny/allow tests pass
- approval allow, deny, expire, and duplicate response tests pass
- shell tool cannot execute without policy decision
- logs include event IDs and session IDs without raw secrets

Public alpha gates:

- threat model reviewed and updated
- Telegram adapter reviewed if enabled
- worktree cleanup reviewed
- dependency license and security audit completed
- documented limitations published

## 13. Review Requirements

Changes require security-focused review when they touch:

- `cadis-policy`
- `cadis-tools`
- `cadis-store`
- shell execution
- provider credential loading
- redaction
- approval resolution
- local transport binding
- worktree cleanup
- logs and debug mode
