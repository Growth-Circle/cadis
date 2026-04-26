---
name: cadis-policy-security
description: Use when implementing or reviewing approval policy, risk classes, sandbox rules, secret redaction, tool permissioning, threat model, or security-sensitive behavior.
---

# CADIS Policy and Security

## Read First

- `docs/14_SECURITY_THREAT_MODEL.md`
- `docs/10_RISK_REGISTER.md`
- `docs/04_TRD.md`
- `SECURITY.md`

## Rules

- Deny by default when risk cannot be classified.
- Approval state is centralized in `cadisd`.
- First valid approval response wins.
- Expired approvals fail closed.
- UI, Telegram, voice, and CLI clients must not execute tools directly.
- Redact secrets before logging.

## Risk Classes

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

## Required Tests

- allow
- deny
- expiry
- duplicate response
- race resolution
- redaction
- outside-workspace behavior
- shell timeout and cancellation

