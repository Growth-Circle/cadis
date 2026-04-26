# Protocol Standard

## 1. Purpose

This standard defines the CADIS local protocol rules for requests, responses, events, schemas, and compatibility.

The protocol is the typed contract between clients and `cadisd`.

## 2. Protocol Principles

- Versioned from the first implementation.
- Local-first and stream-friendly.
- Debuggable as JSON.
- Strongly typed in Rust.
- Stable enough for CLI, tests, Telegram, HUD, and future clients.
- Owned by the daemon, not by any UI.

## 3. Envelope Requirements

Every request includes:

- `protocol_version`
- `request_id`
- `client_id`
- `type`
- `payload`

Every event includes:

- `protocol_version`
- `event_id`
- `timestamp`
- `source`
- `type`
- `payload`
- `session_id` when applicable

Tool lifecycle events also include `tool_call_id`. Approval events include `approval_id`.

## 4. Naming Rules

- Request and event names use lowercase dot-separated names.
- Names identify domain and action, for example `session.create` or `approval.resolved`.
- Do not overload one event type with unrelated payload shapes.
- Do not rename public protocol fields without a compatibility plan.
- Avoid abbreviations unless already part of the domain.

## 5. Request Rules

Supported request families include:

- daemon status
- session create, cancel, subscribe, unsubscribe
- message send
- approval response
- agent list, rename, model set, spawn, kill
- worker tail
- models list
- UI preferences get and set
- voice preview and stop
- config reload

Rules:

- Unknown request types are rejected.
- Malformed payloads are rejected with machine-readable errors.
- Clients cannot request direct tool execution unless the daemon exposes a policy-gated request for it.
- Requests that mutate durable state must return or emit confirmation through daemon events.

## 6. Event Rules

Events are the source of truth for client state.

Required event families:

- daemon lifecycle and errors
- session lifecycle
- message delta and completion
- agent lifecycle, rename, model, status, and task changes
- model catalog response
- tool lifecycle
- approval requested and resolved
- worker lifecycle and log deltas
- patch and test result output
- UI preference updates
- voice preview and playback lifecycle

Rules:

- Events must be serializable and durable enough for JSONL logs.
- Clients may ignore unknown compatible event types after compatibility rules are defined.
- Events that expose user content, tool arguments, errors, or provider data must be redacted before persistence or debug output.
- Message, diff, terminal log, and test output must declare content kind where routed to clients.

## 7. Content Kind

Allowed content kinds:

```text
chat
summary
code
diff
terminal_log
test_result
approval
error
```

Clients use content kind for routing:

- voice may speak short `chat`, `summary`, approval risk summaries, and short actionable errors
- voice must not speak `code`, `diff`, or `terminal_log`
- code window owns full diffs, terminal logs, and test output
- Telegram receives summaries for verbose technical output

## 8. Compatibility

- `protocol_version` is required on every request and event.
- Incompatible clients are rejected during handshake or first request.
- Breaking changes require a protocol version bump.
- Additive fields must have default behavior for old clients.
- Removing fields, changing meanings, changing enum variants, or changing event ordering is breaking unless explicitly version-gated.
- Debug JSON examples must be covered by docs or tests.

## 9. Ordering and Correlation

- Events have unique event IDs.
- Session events include session ID.
- Tool events include tool call ID.
- Approval events include approval ID and tool call ID when applicable.
- Timestamps use an unambiguous UTC format.
- Event ordering utilities must be tested where clients rely on ordering.

## 10. Approval Protocol

Approval request payloads include:

- approval ID
- session ID
- tool call ID
- risk class
- title
- summary or reason
- command or operation details when safe to show
- workspace when applicable
- expiration

Rules:

- Clients submit approval responses to the daemon.
- Clients do not remove approval cards until `approval.resolved`.
- First valid response wins.
- Denied, expired, or invalid approvals fail closed.

## 11. HUD and UI State Protocol

HUD requests and events must follow `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`.

Rules:

- UI preferences persist through daemon config/state.
- Agent rename emits `agent.renamed`.
- Per-agent model changes emit `agent.model.changed` or the accepted equivalent.
- Model catalogs are requested through `models.list`.
- Voice preview and stop are daemon protocol requests.

## 12. Validation

Protocol changes require:

- serialization tests
- compatibility tests for version behavior
- malformed payload tests
- event schema tests for required correlation fields
- redaction tests for debug logging and persisted events
- mock client coverage for CLI and HUD-critical flows when applicable
