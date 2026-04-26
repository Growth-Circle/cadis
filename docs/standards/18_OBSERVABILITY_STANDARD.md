# Observability Standard

## 1. Purpose

This standard defines logs, event IDs, tracing, diagnostics, and debug behavior for CADIS.

Observability must make daemon behavior explainable without leaking secrets.

## 2. Principles

- Events are the primary runtime signal.
- Logs are structured and redacted.
- Every user-visible operation can be correlated to request, session, agent, tool, approval, or worker IDs.
- Debug mode is explicit and still safe.
- Observability must not block the daemon main loop.

## 3. Required Correlation Fields

Every event includes:

- event ID
- event type
- timestamp
- source component
- protocol version

Session events include:

- session ID

Tool events include:

- tool call ID
- session ID when applicable
- risk class when applicable

Approval events include:

- approval ID
- tool call ID when applicable
- session ID when applicable
- verdict when resolved

Worker events include:

- worker ID
- parent agent ID when applicable
- workspace or worktree path after redaction rules

## 4. Event Families

CADIS must emit structured events for:

- daemon lifecycle
- session lifecycle
- message deltas and completions
- agent lifecycle and status
- tool requested, started, completed, and failed
- approval requested and resolved
- worker lifecycle and log deltas
- patch creation
- test results
- UI preference updates
- voice preview and playback
- daemon and protocol errors

## 5. Structured Logs

Rules:

- Logs are machine-readable where practical.
- Event logs are JSONL.
- Human CLI output is not the only diagnostic source.
- Logs should include stable error codes for protocol, policy, tool, provider, and persistence failures.
- High-cardinality content such as full prompts, diffs, and terminal output belongs in event streams or worker logs with redaction, not ordinary daemon log lines.

## 6. Redaction

Redaction is mandatory before persistence or diagnostic output.

Redact:

- provider API keys
- Telegram bot tokens
- values ending in `_KEY`, `_TOKEN`, `_SECRET`, or `_PASSWORD`
- raw secrets from config or environment
- provider error details that echo credentials
- sensitive file contents discovered through tool output

Rules:

- Redaction tests are release gates.
- Redaction failures are security bugs.
- Debug mode cannot disable redaction.

## 7. Debug Mode

Debug mode may expose:

- protocol request and event envelopes
- timing information
- routing decisions
- policy classification details
- provider capability metadata
- tool lifecycle summaries

Debug mode must not expose:

- raw provider keys
- Telegram tokens
- unredacted environment values
- full secret-bearing files
- raw approval credentials or private tokens

## 8. Metrics and Timing

CADIS should measure:

- time to first status event
- event bus relay latency
- tool dispatch overhead excluding tool work
- JSONL append overhead
- daemon startup time
- approval fan-out latency
- worker queue delay

Performance targets from `docs/04_TRD.md` remain authoritative until changed by ADR.

## 9. Error Events

Error events should include:

- stable error code
- human-readable summary
- component
- correlation IDs
- retryability where known
- redacted context
- suggested next action when practical

Errors must not include raw secrets or unredacted provider payloads.

## 10. Client Diagnostics

Clients may display daemon diagnostics, but the daemon remains authoritative.

Rules:

- CLI `status` and `doctor` commands should use daemon protocol where possible.
- HUD connection state derives from daemon status and transport health.
- Telegram receives concise summaries, not full logs.
- Voice receives only content approved by speech routing policy.

## 11. Validation

Observability changes require tests for:

- required event fields
- event ID uniqueness
- session ID propagation
- tool call ID propagation
- approval ID propagation
- redaction in logs and debug protocol output
- JSONL event persistence
- malformed event handling where applicable
