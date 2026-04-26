# CADIS Agent Standard

## 1. Purpose

This standard defines how CADIS agents are modeled, executed, observed, limited, and displayed. It aligns with the daemon-first architecture: agents are runtime entities owned by `cadisd`, not by the CLI, HUD, Telegram adapter, or any model provider.

## 2. Core Model

Each agent session must have stable identifiers and explicit lifecycle state.

Required concepts:

- `SessionId`
- `AgentId`
- parent agent ID, when applicable
- role
- display name
- status
- current task
- model selection
- budget and limits
- event stream position

The main agent is required. Coding, reviewer, and tester agents are optional roles that must use the same runtime contract.

## 3. Agent Roles

| Role | Purpose | Initial Priority |
| --- | --- | --- |
| main | User-facing orchestration and final response | P0 |
| coder | Code edits, tool use, implementation tasks | P1 |
| reviewer | Review for defects, regressions, and test gaps | P2 |
| tester | Test planning and execution assistance | P2 |

Role names are protocol values. Display names are user preferences and may change through `agent.rename`.

## 4. Lifecycle

Allowed statuses:

```text
spawning
idle
working
waiting
completed
failed
cancelled
```

Agents must emit lifecycle events when they are created, start work, wait on approvals or dependencies, complete, fail, or are cancelled.

Rules:

- An agent must not silently disappear from the registry.
- Failures must include actionable metadata.
- Cancellation must propagate to owned tools and workers where possible.
- Workers must not block the main agent event loop.
- Agent status events must include `session_id` and `agent_id`.

## 5. Task State

Agent task state must be concise and renderable in CLI, HUD, and logs.

Required task fields:

- verb, such as `Reading`, `Editing`, `Testing`, or `Waiting`
- target, such as a file path, subsystem, or command
- detail, suitable for compact UI display

Task updates must not include secrets, full logs, or long diffs. Long content belongs in tool output logs or the code work window.

## 6. Delegation and Limits

CADIS must enforce central limits before spawning or delegating:

- maximum depth
- maximum children per agent
- per-agent budget
- per-session budget
- allowed roles
- allowed tools by role, when configured

Limit violations must produce structured errors and visible events.

Delegation should be explicit. Route decisions must be observable through events such as `orchestrator.route` so users can see why work moved to another agent.

Baseline orchestrator behavior:

- Direct leading `@agent` mentions route to existing agents by ID, display name, or role.
- Explicit `/route @agent ...` and `/delegate @agent ...` actions route to an existing agent and emit worker lifecycle events for the delegated unit.
- Explicit `/worker ...` and `/spawn ...` actions create a child agent under `main`, then route the task to that child.
- Explicit worker-spawn actions must pass the same maximum depth, maximum children, and maximum total-agent checks as `agent.spawn`.
- Implicit model-driven recursive spawning is not enabled until a later runtime track defines policy, budget, and recovery behavior.

## 7. Model Selection

Agents may use different models when configured.

Rules:

- The daemon owns per-agent model selection.
- The HUD may request changes through `agent.model.set`.
- Model changes must persist when configured as durable preferences.
- Missing catalog entries must not erase the current configured value.
- Provider capabilities must be checked before assigning a model to a task that needs streaming, tools, or other specific features.

## 8. Tool Use

Agents must call tools only through the CADIS tool runtime.

Rules:

- Agents must not execute shell commands directly.
- Agents must not bypass policy or approval checks.
- Every tool call must have a `ToolCallId`.
- Tool lifecycle events must be linked to the agent and session.
- Tool failures must be surfaced to the agent as structured results.
- Risky tool calls must pause on approval instead of continuing optimistically.

## 9. Approval Behavior

When an agent needs approval:

- agent status should become `waiting`
- the approval request must identify the agent
- the agent may resume only after an approved resolution
- denied or expired approvals must produce a clear agent-visible failure or alternate path

First-response-wins approval resolution is authoritative. Agents must consume the daemon resolution event, not local UI button state.

## 10. Persistence and Recovery

CADIS must persist enough metadata to audit sessions and recover incomplete work when feasible.

Persist:

- session metadata
- agent creation and lifecycle events
- approvals requested and resolved
- tool lifecycle events
- final status

Do not persist:

- raw provider keys
- unredacted secrets
- unnecessary full prompts if policy later restricts retention

Incomplete session recovery should be conservative. Unknown or partially recovered agents should be marked failed or interrupted rather than resumed unsafely.

## 11. HUD Requirements

The HUD must render agents from daemon state and events.

Agent cards must support:

- display name
- status dot and label
- role
- current task verb, target, and detail
- model or idle detail
- nested workers
- context action for rename

Rename behavior:

- trim whitespace
- collapse repeated whitespace
- cap display name at 32 characters
- use `CADIS` for blank main-agent names
- use role defaults for blank subagent names
- persist through daemon state
- update UI only from `agent.renamed`

## 12. Testing Requirements

Required agent tests:

- lifecycle event ordering
- max depth enforcement
- max children enforcement
- budget exhaustion
- cancellation propagation
- approval wait and resume
- denial and expiry behavior
- model assignment validation
- rename normalization
- HUD event mapping for status and task updates
