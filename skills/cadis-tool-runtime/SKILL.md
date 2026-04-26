---
name: cadis-tool-runtime
description: Use when implementing native CADIS tools such as file, shell, git, patch, browser bridge, tool registry, tool schemas, lifecycle events, timeout, cancellation, or tool result handling.
---

# CADIS Tool Runtime

## Read First

- `docs/03_FRD.md`
- `docs/04_TRD.md`
- `docs/05_ARCHITECTURE.md`
- `docs/14_SECURITY_THREAT_MODEL.md`

## Rules

- Every tool declares name, schema, risk class, side effects, timeout, and workspace behavior.
- Every tool goes through policy before execution.
- Every tool emits lifecycle events.
- Tool results must be structured.
- Tool output must be redacted before logging.

## Initial Tools

- `file.read`
- `file.search`
- `file.patch`
- `shell.run`
- `git.status`
- `git.diff`
- `git.worktree.create`
- `git.worktree.remove`

## Validation

Test success, failure, denial, timeout, cancellation, and redaction.

