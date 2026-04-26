# CADIS Approval Policy Standard

## 1. Purpose

This standard defines the central approval and policy requirements for CADIS. Policy is a daemon-owned safety boundary. No client, adapter, UI, model provider, agent, or tool may bypass it.

## 2. Core Rules

- `cadisd` owns policy decisions.
- Policy must run before risky tool execution.
- Safe reads may be auto-allowed by default.
- Secret access requires approval.
- Outside-workspace writes require approval.
- Dangerous deletes require approval.
- Sudo and system changes require approval.
- Protected git writes require approval.
- Approval resolution is first-response-wins.
- Approval requests and resolutions must be persisted when persistence is enabled.

## 3. Policy Decision Flow

Every action that may cross a safety boundary must follow this flow:

```text
validate request
classify risk
evaluate policy
auto-allow, auto-deny, or request approval
persist approval request when needed
wait for resolution
execute only after approved
persist resolution and execution result
```

Policy must fail closed. Unknown or malformed risky actions must not execute.

## 4. Decision Types

Allowed policy decisions:

| Decision | Meaning |
| --- | --- |
| allow | Execute without interactive approval |
| deny | Reject without execution |
| require_approval | Pause and request approval |

A decision must include:

- action identity
- risk class
- reason
- matching rule, when applicable
- required approval metadata, when applicable

## 5. Approval Request Contract

Approval requests must include:

- `approval_id`
- `session_id`
- `agent_id`, when applicable
- action or command
- cwd or workspace
- risk class
- concise reason
- risk summary
- expiry timestamp, when applicable
- available responses

The request must contain enough information for CLI, HUD, Telegram, and logs to show the same decision context.

## 6. Approval Resolution

Resolution rules:

- First valid response wins.
- Later responses must be rejected or reported as already resolved.
- Denied approvals must prevent execution.
- Expired approvals must prevent execution.
- Approval state must not depend on UI-local state.
- Clients must remove approval cards only after `approval.resolved`.

Resolution events must include:

- `approval_id`
- verdict
- resolver surface, when known
- resolved timestamp
- final state

## 7. Defaults

Recommended default behavior:

| Action | Default |
| --- | --- |
| File read inside workspace | allow |
| File search inside workspace | allow |
| Git status or diff | allow |
| Patch inside workspace | require policy decision; approval may be configurable |
| Shell command | require policy decision |
| Outside-workspace write | require approval |
| Secret access | require approval |
| Dangerous delete | require approval |
| Sudo or system mutation | require approval |
| Protected git push or force push | require approval |

Open-source defaults should be conservative and understandable.

Current implementation baseline:

- `file.read`, `file.search`, and `git.status` are auto-allowed only after
  daemon-side tool classification and workspace path validation.
- `shell.run`, write tools, and mutating git/worktree placeholders require
  approval and are persisted under daemon-owned state.
- Unknown tools are denied.
- Approved risky placeholders still fail closed with `tool.failed` until the
  corresponding execution backend is implemented.
- `approval.respond` uses daemon state and persisted approval records; the first
  valid pending response wins, later responses are rejected as already resolved.

## 8. Configuration

Policy configuration belongs in `~/.cadis/config.toml`.

Configuration may define:

- approval timeout
- allowed safe-read roots
- shell command allowlist or denylist
- protected branch patterns
- tool-specific approval requirements
- adapter permissions
- default deny rules

Environment overrides may be supported for development, but must not weaken policy silently in production-like use.

## 9. Multi-Surface Approvals

Approvals may be shown in multiple surfaces:

- CLI
- HUD
- Telegram adapter
- future desktop notifications

All surfaces must send resolution to the daemon. The daemon must arbitrate first-response-wins and emit one authoritative resolution event.

Clients must handle the case where another surface resolves the approval first.

## 10. Audit and Persistence

When persistence is enabled, CADIS must persist:

- approval request
- policy decision
- resolver surface
- resolution verdict
- timestamps
- linked session, agent, and tool call IDs

Persisted data must be redacted. Raw provider keys, auth headers, and environment secrets must never be written to approval logs.

The baseline persists one JSON approval record per approval under
`~/.cadis/state/approvals`. Records include request metadata, risk class,
expiration, decision, redacted reason, and resolution timestamp.

## 11. UX Requirements

Approval prompts must be concise but complete.

They should show:

- action
- workspace or cwd
- risk class
- reason
- affected target
- expiry
- approve and deny choices

The HUD approval card must remain visible until `approval.resolved` arrives from the daemon.

## 12. Testing Requirements

Required tests:

- safe read auto-allow
- outside-workspace write requires approval
- secret access requires approval
- dangerous delete requires approval
- sudo/system change requires approval
- protected git write requires approval
- approval allow executes action
- approval deny prevents action
- expiry prevents action
- duplicate responses are first-response-wins
- concurrent CLI and HUD responses resolve once
- request and resolution persistence
- redaction in approval logs
