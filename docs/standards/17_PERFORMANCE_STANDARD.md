# CADIS Performance Standard

## 1. Purpose

This standard defines performance expectations for CADIS. The goal is a responsive local daemon and UI without compromising safety, correctness, or auditability.

## 2. Principles

- Optimize measured bottlenecks.
- Keep the daemon responsive under tool, provider, and worker activity.
- Do not block the main event loop on worker execution.
- Prefer bounded queues, timeouts, and backpressure over unbounded memory growth.
- Preserve complete safety checks even when optimizing.
- Performance tests must be reproducible before they become CI gates.

## 3. Key Metrics

CADIS should measure:

- daemon startup time
- time to first event
- local protocol round-trip latency
- event bus relay latency
- model time to first delta, excluding provider latency where possible
- tool dispatch overhead
- shell spawn overhead
- JSONL append overhead
- approval fan-out latency
- cancellation latency
- HUD event reducer throughput
- HUD frame responsiveness under active streams

## 4. Initial Targets

Initial targets are guidance, not hard release gates until measured on supported platforms.

| Metric | Target |
| --- | --- |
| `cadisd --check` | under 500 ms on a typical developer machine |
| daemon ready after start | under 2 seconds |
| local status request | under 100 ms |
| event bus relay overhead | under 20 ms p95 in local tests |
| tool dispatch overhead before command execution | under 50 ms |
| approval fan-out after policy decision | under 100 ms |
| JSONL append overhead | under 10 ms p95 for normal events |
| HUD interaction response | no visible lag for normal controls |

Targets must be revised with benchmark data.

## 5. Daemon Responsiveness

The daemon must remain responsive while:

- a model stream is active
- a shell command is running
- a worker is active
- an approval is waiting
- JSONL logs are being appended
- multiple clients are subscribed

Long-running operations must be asynchronous or isolated so they do not block health checks, cancellation, approval resolution, or event delivery.

## 6. Event Bus

Event delivery must be bounded and observable.

Rules:

- Use bounded buffers or explicit backpressure.
- Slow clients must not stall the daemon globally.
- Event IDs must allow clients to detect gaps.
- Large outputs must be chunked or summarized.
- Dropped events, if ever allowed for non-critical streams, must be explicit.

Safety-critical events such as approvals, resolution, tool lifecycle, and session completion must not be silently dropped.

## 7. Model Streaming

Model streaming should optimize perceived responsiveness:

- emit session and model start events promptly
- forward deltas in order
- avoid buffering the entire response before display
- support cancellation
- bound memory for long responses
- map provider stalls to timeout or progress metadata where appropriate

Provider network latency must be measured separately from CADIS runtime overhead.

## 8. Tool Runtime

Tool execution performance must not weaken policy.

Rules:

- policy evaluation runs before execution
- timeouts are required for long-running tools
- stdout and stderr capture must be bounded
- large outputs must be summarized or chunked
- cancellation must be checked and propagated
- child process cleanup must not block shutdown indefinitely

## 9. Persistence

Persistence must be durable and efficient.

Requirements:

- append JSONL events without rewriting full logs
- use atomic writes for state files
- flush critical events before graceful shutdown
- redact before writing
- avoid blocking the daemon on slow disk operations where practical

Recovery should favor correctness over speed.

## 10. HUD Performance

The HUD must stay responsive while rendering active agents, workers, messages, and approvals.

Requirements:

- render from an event-derived snapshot
- avoid recomputing full history on every frame
- virtualize or compact long logs when needed
- keep the 16:9 orbital layout stable
- prevent text overflow and layout shifts
- maintain responsive controls at minimum size 1200x760

Screenshot and pixel checks must confirm that important elements are nonblank and non-overlapping.

## 11. Benchmarking

Benchmark commands should be easy to run locally and documented near implementation.

Benchmarks should cover:

- protocol serialization
- event fan-out
- JSONL append
- policy evaluation
- tool dispatch
- mock provider streaming
- HUD reducer throughput

Benchmarks must isolate external services. Live model-provider benchmarks are optional and must be labeled as environment-dependent.

## 12. Observability

Performance diagnostics should use structured logs and metrics fields:

- event ID
- session ID
- agent ID
- tool call ID
- queue wait time
- execution duration
- provider duration
- persistence duration
- client delivery duration

Debug mode may expose local protocol timing, but must still redact secrets.

## 13. Regression Policy

Performance regressions are release blockers when they affect:

- approval resolution
- cancellation
- daemon health checks
- event delivery for safety-critical events
- startup reliability
- UI ability to show current safety state

Non-critical regressions should be tracked with benchmark output, platform details, and a proposed threshold.
