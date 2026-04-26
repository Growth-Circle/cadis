# Memory Concept

## Purpose

This document adapts Wulan's CADIS memory concept into the CADIS architecture roadmap.
It defines how long-lived memory should work once CADIS moves beyond the current
desktop MVP.

Memory is future work. It is not part of the v0.1 runtime contract unless a later
decision record changes scope.

## Contribution Context

- Contributor: Wulan.
- Source contribution: `memory-concept.md`, design draft dated 2026-04-26.
- Status: concept, future work.
- Scope: daemon-owned memory architecture for sessions, agents, projects, tasks,
  delegation, and user preferences.

Wulan's central framing is:

```text
memory store != model context
```

CADIS should store memory outside the prompt, then inject only a bounded,
ranked, compressed memory capsule into an agent turn.

## Problem

A multi-agent runtime cannot safely treat memory as one giant transcript, one
vector database, or one ungoverned prompt prefix. CADIS needs memory that is:

- local-first and inspectable
- scoped per user, project, agent, task, and session
- reconstructible after crashes
- protected by ACL and redaction
- ranked and budgeted before context injection
- auditable when memory affects an answer

## Goals

- Remember stable user preferences across sessions.
- Remember project-specific facts, decisions, bug patterns, and procedures.
- Let agents and subagents work without polluting each other's context.
- Let parent agents receive child summaries and memory candidates, not raw logs.
- Keep persona and policy stable while allowing memory to adapt.
- Keep retrieval bounded by latency, token budget, scope, freshness, and evidence.
- Preserve local-first operation while allowing optional external providers later.

## Non-Goals

- No complex memory graph in v0.1.
- No automatic injection of all history into every model call.
- No client-owned memory in HUD, CLI, Telegram, or voice adapters.
- No remote memory provider as the default source of truth.
- No silent mutation of persona, policy, or security rules.
- No persistence of secrets, raw credentials, or unredacted sensitive artifacts.

## Proposed CADIS Model

CADIS should implement memory as daemon infrastructure owned by `cadisd`.
Agents may request, propose, and use memory, but the runtime controls:

- scope
- access
- retrieval
- ranking
- token budget
- validation
- promotion
- conflict handling
- persistence
- observability

The core loop should be:

```text
observe -> candidate -> validate -> promote -> retrieve -> compile capsule -> use
```

Every durable memory write should have provenance. A project fact, bug pattern,
or procedure is not accepted simply because a model said it; it must carry
evidence such as a session id, task id, file path, tool result, or commit.

## Memory Layers

CADIS should treat memory as layered state:

| Layer | Name | Purpose |
| --- | --- | --- |
| L0 | Turn scratchpad | Temporary plan and tool-result working state. |
| L1 | Session memory | Current conversation or task session summary. |
| L2 | Task memory | Worker goal, artifacts, result, status, and findings. |
| L3 | Daily memory | Low-friction daily observations and consolidation input. |
| L4 | Project memory | Repository facts, decisions, bugs, playbooks, constraints. |
| L5 | Agent memory | Role-specific lessons and agent-private facts. |
| L6 | User/global memory | Stable user preferences and long-term facts. |
| L7 | Procedure memory | Reusable skills and workflows learned from successful work. |
| L8 | Provider memory | Optional local or remote semantic providers. |
| L9 | Memory ledger | Append-only provenance and replay log. |

Project memory should outrank generic memory during repository work, unless the
generic memory is an explicit user preference or correction.

## Persistence

The recommended local-first stack is:

```text
Markdown files
  + SQLite metadata and FTS
  + append-only JSONL ledger
  + optional local vector index
  + artifact store for logs, transcripts, diffs, and test output
```

Suggested layout:

```text
~/.cadis/
|-- memory/
|   |-- ledger.jsonl
|   |-- candidates.jsonl
|   |-- global/
|   |   |-- USER.md
|   |   |-- PREFERENCES.md
|   |   `-- DECISIONS.md
|   |-- daily/
|   |   `-- 2026-04-26.md
|   |-- projects/
|   |   `-- <project-id>/
|   |       |-- MEMORY.md
|   |       |-- DECISIONS.md
|   |       |-- PLAYBOOKS.md
|   |       `-- BUGS.md
|   |-- delegation/
|   |-- providers/
|   `-- indexes/
|       `-- cadis-memory.sqlite
|-- agents/
|   `-- <agent-id>/
|       |-- PERSONA.md
|       |-- POLICY.toml
|       `-- MEMORY.md
`-- tasks/
    |-- task_<id>.json
    `-- task_<id>.jsonl
```

Markdown gives users an inspectable source. SQLite gives CADIS fast filtering,
dedupe, ACL checks, freshness checks, and FTS. The ledger gives replay,
debugging, and crash recovery.

## Runtime Flow

Each agent turn should eventually use this flow:

1. Route the request to the target agent.
2. Build a memory query plan using agent, project, task, and user scope.
3. Prefetch bounded memory hits from local indexes and optional providers.
4. Compile a memory capsule within token and latency budgets.
5. Run the model/tool loop.
6. Extract memory candidates from the turn summary, tool results, and user corrections.
7. Queue candidates for validation, promotion, or rejection.
8. Emit memory observability events.

The model receives the compiled capsule, not raw memory files.

## Multi-Agent Rules

- Agent memory is private by default.
- Shared memory requires an explicit grant.
- Child agents return summaries, artifacts, decisions, memory candidates, and
  confidence, not full hidden transcripts.
- Parent agents may receive delegation memory after runtime validation.
- Recursion depth, child count, and memory grants must respect agent budgets.
- `Mneme` is the recommended memory curator agent name.

## Data Model

The first Rust implementation should model at least:

```text
MemoryRecord
MemoryScope
MemoryKind
MemoryStatus
MemorySource
MemoryAcl
MemoryHit
MemoryCandidate
MemoryQueryPlan
MemoryCapsule
```

Important statuses:

- `candidate`
- `confirmed`
- `superseded`
- `rejected`
- `archived`
- `expired`

Important kinds:

- user preference
- user fact
- project fact
- decision
- procedure
- bug pattern
- environment fact
- tool convention
- task summary
- delegation result
- correction
- warning
- contradiction

## Protocol Implications

Future protocol work should add memory requests and events without letting
clients bypass daemon authority.

Candidate requests:

```text
memory.search
memory.propose
memory.inspect
memory.promote
memory.reject
memory.conflicts
memory.compact
context.preview
context.explain
```

Candidate events:

```text
memory.candidate.created
memory.promoted
memory.superseded
memory.rejected
memory.used
memory.conflict.detected
context.compiled
```

HUD, CLI, Telegram, and voice clients should only request or display these
operations. `cadisd` remains the authority for ACL, promotion, persistence, and
context compilation.

## Config Implications

Future configuration may look like:

```toml
[memory]
enabled = true
backend = "sqlite"
ledger = "~/.cadis/memory/ledger.jsonl"
human_files = true
auto_promote_daily = true
auto_promote_user_preferences = "explicit_only"
auto_promote_project_facts = "with_evidence"
persona_mutation = "proposal_only"
policy_mutation = "deny"

[memory.retrieval]
keyword_top_k = 20
vector_top_k = 20
rerank_top_k = 8
max_injected_tokens = 6000
provider_timeout_ms = 200
stale_while_revalidate = true
```

These keys are not accepted config yet. They should move into
`docs/16_CONFIG_REFERENCE.md` only after implementation and decision approval.

## Security And Privacy

Memory increases the blast radius of bad persistence. CADIS must apply these
rules before enabling durable memory:

- redact secrets before ledger, Markdown, SQLite, and vector indexing
- deny direct persona or policy mutation by agents
- require provenance for project facts and procedures
- mark conflicts and superseded entries instead of silently deleting them
- keep external provider memory optional and disabled by default
- never let retrieval failure crash the active user request
- expose memory usage and omitted memory summaries for debugging

Embeddings and semantic indexes can leak sensitive facts even when source text
is hidden. They must inherit the same local-first and redaction policy.

## MVP Acceptance Path

Suggested memory phases:

| Phase | Outcome |
| --- | --- |
| M0 | `cadis-memory` crate, record types, JSONL ledger, file-backed root. |
| M1 | Agent capsules, context compiler, memory contract injection. |
| M2 | SQLite metadata, FTS search, scope filtering, token-budget packing. |
| M3 | Candidate extraction, validation, promotion, dedupe, conflicts. |
| M4 | Delegation memory grants and shared task memory pools. |
| M5 | Async providers and optional local vector recall. |
| M6 | CLI/HUD observability, memory stats, context preview, context explain. |

The first useful CLI should be:

```bash
cadis memory propose ...
cadis memory search "auth middleware" --project <project-id>
cadis memory candidates list
cadis memory promote <memory-id>
cadis context preview --agent codex --task <task-id>
```

## Open Questions

- Which SQLite/vector dependency should be accepted under the license policy?
- Should user-global memory require explicit opt-in on first run?
- What memory operations need user approval versus Mneme review?
- How should memory be exported, imported, and deleted?
- What is the minimum useful HUD memory panel for desktop alpha?
- How should memory ACL violations appear in logs and UI?

## References

- Wulan, `memory-concept.md`, CADIS memory design contribution, 2026-04-26.
- `docs/05_ARCHITECTURE.md`
- `docs/06_IMPLEMENTATION_PLAN.md`
- `docs/11_DECISIONS.md`
- `docs/14_SECURITY_THREAT_MODEL.md`
- `docs/15_PROTOCOL_DRAFT.md`
- `docs/16_CONFIG_REFERENCE.md`
- `docs/standards/03_ARCHITECTURE_STANDARD.md`
- `docs/standards/04_SECURITY_STANDARD.md`
- `docs/standards/14_CONFIG_PERSISTENCE_STANDARD.md`
