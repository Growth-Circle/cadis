# Architecture Standard

## 1. Purpose

This standard defines the architectural rules for CADIS.

CADIS is daemon-first. `cadisd` is the authority for sessions, agents, tools, policy, approvals, event routing, persistence, and model interaction. Every user interface is a client.

## 2. Core Rule

All operational behavior must pass through the daemon protocol.

Adapters may:

- render state
- collect user input
- submit protocol requests
- subscribe to events
- display approval prompts
- cache local view state for rendering

Adapters must not:

- execute tools directly
- own approval state
- mutate agent runtime state outside the daemon
- persist authoritative session or agent state in browser storage
- bypass policy for convenience
- implement core orchestration logic

## 3. Runtime Boundary

Core components:

- daemon process
- local protocol
- event bus
- session registry
- agent runtime
- worker scheduler
- model provider abstraction
- native tool runtime
- policy and approval engine
- persistence and redaction layer

Adapter components:

- CLI
- HUD
- code work window
- Telegram adapter
- voice client
- future Android remote
- optional MCP bridge

The optional MCP bridge is an extension path, not the core tool runtime.

## 4. Request Flow

The required fast path is:

```text
client request -> local protocol -> daemon session -> event bus -> model/tool stream
```

Rules:

- A client request must identify protocol version and request ID.
- The daemon accepts, rejects, or routes the request.
- State changes are emitted as events.
- Durable events are written through the store after redaction.
- Clients update from daemon events, not from optimistic local authority.

## 5. Tool and Approval Flow

Tool execution must follow this sequence:

```text
tool request -> policy classification -> approval if required -> execution -> lifecycle events -> redacted persistence
```

Rules:

- Model-generated tool calls are untrusted input.
- Policy classifies before execution.
- Denied, expired, corrupted, or missing approval state fails closed.
- Approval state is centralized and first valid response wins.
- Tool lifecycle events include tool call ID and session ID when applicable.

## 6. Worker Isolation

Parallel coding agents must not edit the same target working directory directly.

Default workflow:

```text
task -> worker -> CADIS-controlled git worktree -> edit/test/diff -> review -> user approval -> apply
```

Rules:

- Worktrees live under CADIS-controlled storage unless explicitly configured.
- Worker output streams through daemon events.
- Patch application to a target workspace is policy-gated.
- Cleanup is explicit or governed by policy.
- The main session event loop must remain responsive while workers run.

## 7. UI Architecture

The HUD and code work window communicate only through the daemon protocol.

Rules:

- HUD state is derived from events and daemon-backed preferences.
- Durable HUD preferences live in daemon config/state.
- Approval cards remain visible until `approval.resolved`.
- Agent rename and per-agent model selection must persist through daemon state.
- Voice playback follows content-kind routing and must not speak code, diffs, terminal logs, or secrets.

## 8. Transport Architecture

Initial transport is local-only:

- Unix domain socket for Linux MVP.
- Stdio for tests and scripted clients.
- WebSocket later for HUD and remote relay only after protocol behavior is stable.

Rules:

- Protocol version is mandatory.
- Incompatible clients are rejected.
- Local transport must not be exposed remotely by default.
- Reconnect behavior must recover recent state where available.

## 9. Persistence Architecture

Default CADIS home:

```text
~/.cadis/
|-- config.toml
|-- sessions/
|-- workers/
|-- logs/
|-- worktrees/
|-- approvals.json
`-- tokens/
```

Rules:

- State writes are atomic.
- Event logs are JSONL.
- Session and worker logs are separate.
- Secrets are redacted before persistence.
- Crash recovery uses persisted metadata where possible.

## 10. Architecture Changes

An ADR in `docs/11_DECISIONS.md` is required for:

- changing the daemon-first authority model
- changing local transport strategy
- changing protocol compatibility rules
- changing storage formats
- changing tool execution or policy flow
- adding direct tool execution to any adapter
- changing the UI framework
- importing substantial source from another project

## 11. Review Questions

Every architecture review should ask:

- Does this keep `cadisd` authoritative?
- Does every risky operation pass through policy?
- Can clients reconstruct state from events and snapshots?
- Are secrets redacted before leaving their trust boundary?
- Can the main daemon loop stay responsive?
- Is this behavior testable without a GUI?
