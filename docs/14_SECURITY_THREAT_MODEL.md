# Security Threat Model

## 1. Scope

This threat model covers the local CADIS runtime:

- daemon
- CLI
- local protocol
- tools
- policy and approvals
- persistence
- model providers
- Telegram adapter
- voice output
- HUD and code work window later

## 2. Assets

| Asset | Why it matters |
| --- | --- |
| Provider API keys | Can incur cost or expose account access |
| Telegram bot token | Can allow remote control abuse |
| Local files | CADIS can read and edit project files |
| Shell access | Commands can change or damage the machine |
| Git repositories | Agents can change source code |
| Logs | May reveal secrets, commands, prompts, or paths |
| Future memory store | May preserve user facts, project facts, summaries, embeddings, or tool history |
| Approval state | Incorrect state can allow risky actions |
| Model context | May contain private code or user data |

## 3. Trust Boundaries

```text
User input
  -> client adapter
  -> daemon protocol
  -> daemon core
  -> policy engine
  -> tool runtime
  -> local OS / network / filesystem
```

Important boundaries:

- model output is untrusted
- Telegram messages are remote input
- tool arguments are untrusted until validated
- provider responses are untrusted
- logs must be redacted before persistence

## 4. Threats

| ID | Threat | Mitigation |
| --- | --- | --- |
| T-001 | Prompt injection asks model to run dangerous command | Tool calls require policy and approval |
| T-002 | Model attempts to read secrets | Secret-access risk class requires approval |
| T-003 | Tool writes outside workspace | Outside-workspace policy gate |
| T-004 | Shell command damages system | Risk classification, approval, timeout |
| T-005 | Approval race causes double execution | First-response-wins state machine |
| T-006 | Telegram token is logged | Redaction before logging |
| T-007 | Provider key appears in error | Error redaction and provider error mapping |
| T-008 | Agent fan-out exhausts resources | Depth, children, global, timeout, budget limits |
| T-009 | Parallel edits corrupt repo | Git worktree isolation |
| T-010 | Voice speaks private code or secrets | Content kind routing and speech policy |
| T-011 | UI bypasses policy | UI clients cannot execute tools directly |
| T-012 | Local protocol exposed beyond machine | Local-only transport by default |
| T-013 | Future memory stores sensitive facts or stale secrets | Redact before memory persistence, enforce ACL, keep provider memory optional |

## 5. Security Requirements

- Deny by default when policy cannot classify an action.
- Fail closed when approval state is missing, expired, or corrupted.
- Redact secrets before writing logs.
- Never expose raw provider keys through events.
- Treat model-generated tool calls as untrusted input.
- Make tool execution cancellable.
- Keep audit events for approvals and tools.
- Future memory writes must be daemon-owned, provenance-backed, and redacted
  before Markdown, JSONL, SQLite, or vector indexing. See `25_MEMORY_CONCEPT.md`.

## 6. Pre-Alpha Security Gates

- Redaction tests pass.
- Secret scan or equivalent credential-leak check passes before publishing.
- Approval allow, deny, expire, and duplicate response tests pass.
- Shell tool cannot run without policy decision.
- Outside-workspace write is blocked or approval-gated.
- Logs contain event IDs and session IDs.

## 7. Public Alpha Security Gates

- Threat model updated.
- Telegram adapter reviewed.
- Worktree cleanup reviewed.
- Dependency license and security audit completed.
- Known limitations published.
