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
| Profile and agent homes | May contain durable memory, instructions, policy, sessions, and channel state |
| Shell access | Commands can change or damage the machine |
| Git repositories | Agents can change source code |
| Workspace grants | Incorrect grants can expose broad filesystem access |
| Project media assets | Generated or copied media may contain private references, prompts, or provenance |
| Logs | May reveal secrets, commands, prompts, or paths |
| Future memory store | May preserve user facts, project facts, summaries, embeddings, or tool history |
| Approval state | Incorrect state can allow risky actions |
| Model context | May contain private code or user data |
| Optional face tracking data | Camera-derived signals may reveal biometric or environmental information |

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
- profile homes and agent homes are state boundaries, not sandboxes
- project `.cadis/` metadata is untrusted input until validated by the daemon
- worker worktrees are scoped execution roots, not approval bypasses
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
| T-014 | Optional avatar face tracking leaks camera frames, landmarks, or biometrics | Face tracking is off by default, explicit opt-in, local-only, non-persistent, and guarded by visible active-camera UI plus one-click disable |
| T-015 | Risky native tool executes after malformed or missing approval state | Unknown tools are denied, risky placeholders create persisted approvals, expired or missing approvals fail closed, and approved risky placeholders do not execute in the baseline |
| T-016 | Agent home is accidentally used as project cwd and leaks memory or policy files to tools | Agent home and project workspace are separate typed records; file/shell/git tools require workspace grants |
| T-017 | Broad or stale workspace grant exposes `$HOME`, `/`, system paths, or cloud credential directories | Grant validation rejects broad roots, applies expiry, canonicalizes paths, rejects symlink escape, and checks denied paths |
| T-018 | Worker edits parent checkout instead of isolated branch | Coding workers default to project `.cadis/worktrees/<worker-id>/` and receive write/exec grants only for that worktree |
| T-019 | Project `.cadis/media/` stores secrets, raw transcripts, or untracked private provenance | Media manifests are redacted, secrets and raw transcripts are forbidden, and large binaries are ignored unless explicitly tracked |

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
- Optional Wulan face tracking must remain local to the renderer process. Raw
  frames, landmarks, embeddings, identity labels, and biometric templates must
  not be sent to model providers, remote relays, logs, diagnostics, crash
  reports, or telemetry by default.
- Workspace grants must be resolved before file, shell, git, or worker tools
  execute. Missing, expired, corrupt, or mismatched grants fail closed.
- Denied paths must include SSH/GPG/cloud credential directories, profile
  `.env` files, profile secret stores, channel token directories, and system
  paths such as `/etc`, `/dev`, `/proc`, and `/sys`.
- Project `.cadis/media/` may hold generated media and manifests, but must not
  contain provider tokens, raw channel tokens, secrets, or raw session
  transcripts.

## 6. Pre-Alpha Security Gates

- Redaction tests pass.
- Secret scan or equivalent credential-leak check passes before publishing.
- Approval allow, deny, expire, and duplicate response tests pass.
- Shell tool cannot run without policy decision.
- Outside-workspace write is blocked or approval-gated.
- Logs contain event IDs and session IDs.

Current baseline status:

- Safe-read native tools are limited to `file.read`, `file.search`, and
  `git.status`.
- Safe file tools resolve canonical paths and reject outside-workspace reads.
- `shell.run` and write/mutating placeholders require persisted approval but
  still fail closed after approval until execution backends are implemented.
- Full agent home, worker worktree, checkpoint, media-manifest, and mutating-tool
  denied-path enforcement remains future work. The current baseline already
  persists workspace grants, rejects broad workspace roots, and blocks
  safe-read symlink/path escape.

## 7. Public Alpha Security Gates

- Threat model updated.
- Telegram adapter reviewed.
- Worktree cleanup reviewed.
- Dependency license and security audit completed.
- Known limitations published.
