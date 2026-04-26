# Code Standard

## 1. Purpose

This standard defines code quality rules for CADIS Rust crates, binaries, and tests.

CADIS follows the Rust API Guidelines where practical and adapts them to a daemon-first local runtime.

## 2. Baseline Rules

- Use stable Rust unless a maintainer-approved decision record allows otherwise.
- Keep `unsafe` forbidden by default. Any exception requires a crate-specific ADR, safety comment, and tests.
- Treat warnings as defects in CI once implementation begins.
- Prefer explicit, typed data structures over loosely typed maps for protocol, config, policy, tools, and persistence.
- Keep public APIs small, documented, and hard to misuse.
- Avoid global mutable state except for process-wide constants or test-only fixtures.
- Do not introduce core runtime dependencies on Node.js, browser APIs, or UI frameworks.

## 3. Workspace and Crate Boundaries

The proposed workspace structure in `docs/04_TRD.md` is authoritative until changed by ADR.

Expected ownership:

| Area | Crate responsibility |
| --- | --- |
| protocol | typed requests, responses, events, versioning, serialization |
| core | sessions, event bus contracts, orchestration traits |
| daemon | process runtime, transport binding, lifecycle wiring |
| cli | command parsing and protocol client behavior |
| models | provider traits, capabilities, provider error mapping |
| tools | native tool registry and tool implementations |
| policy | risk classification, approval decisions, policy state |
| store | config, JSONL logs, state persistence, redaction before write |
| agents/workers | agent sessions, scheduling, worktree isolation |

Rules:

- Adapters may call the protocol client; they must not bypass `cadisd`.
- Tool execution must flow through policy.
- Store code must not depend on UI crates.
- Protocol types must not depend on daemon internals.
- Crates should expose capabilities through traits or typed structs, not stringly typed side channels.

## 4. Rust API Quality

Public Rust APIs must follow these rules:

- Names are clear, consistent, and domain-specific.
- Types encode invariants where reasonable, especially IDs, risk classes, protocol versions, and content kinds.
- Constructors validate data that cannot be valid by convention alone.
- Error types distinguish caller errors, policy denials, transport failures, provider failures, persistence failures, and internal bugs.
- Public functions document side effects, blocking behavior, cancellation behavior, and security-sensitive behavior.
- Builders are acceptable for complex config or provider options, but simple structs are preferred for protocol payloads.
- Serialization formats must be stable enough for tests and debugging.

## 5. Error Handling

- Use `Result<T, E>` for recoverable failures.
- Do not panic for user input, model output, provider responses, malformed config, transport input, or tool arguments.
- Panics are acceptable only for impossible internal invariants and test code.
- Attach actionable context to errors before crossing crate boundaries.
- Redact secrets before formatting errors for events, logs, CLI output, or UI surfaces.
- Preserve machine-readable error codes for protocol and tool failures.

## 6. Async and Blocking Work

- Tokio is the default async runtime unless changed by ADR.
- The daemon main loop must not block on worker execution, shell commands, provider streams, or disk-heavy persistence.
- Blocking filesystem or process work must be isolated with appropriate async boundaries.
- Long-running operations must support cancellation when technically possible.
- Channels and queues must have bounded or documented backpressure behavior.

## 7. Logging and Events

- Use structured logging.
- Logs must include enough context to correlate with protocol events without leaking secrets.
- Do not log raw prompts, diffs, tool arguments, or provider errors at high verbosity unless the debug mode explicitly allows it and redaction has run.
- Event IDs, session IDs, tool call IDs, and approval IDs must use typed wrappers in core code when practical.

## 8. Dependencies

- Prefer well-maintained crates with clear licenses, active maintenance, and limited transitive risk.
- New dependencies in security-sensitive crates require review for license, maintenance, and attack surface.
- Do not add a dependency for trivial logic.
- Pin behavior through tests when using crates for serialization, config parsing, transport, redaction, or process execution.

## 9. Tests

Code touching these areas must include targeted tests or an explicit test-gap note:

- protocol serialization and compatibility
- config parsing and environment override behavior
- redaction
- policy decisions
- approval state transitions
- tool schema validation
- persistence atomicity
- event ordering and correlation fields

## 10. Review Gates

Changes require stricter review when they touch:

- shell execution
- file writes
- outside-workspace access
- secret handling
- policy decisions
- approval resolution
- provider credential loading
- JSONL logs or state migrations
- local transport exposure
