# Technical Requirements Document

## 1. Technical Strategy

CADIS will be built as a fresh Rust monorepo. The daemon and protocol define the product. UI clients, Telegram, voice, and model providers are adapters around that core.

The architecture must keep the fast path small:

```text
client request -> local protocol -> daemon session -> event bus -> model/tool stream
```

## 2. Target Platforms

| Platform | Phase | Notes |
| --- | --- | --- |
| Linux desktop | First | Primary development and MVP target |
| Windows | Later | Requires shell, sandbox, path, and audio adapter work |
| macOS | Later | Requires sandbox and audio adapter work |
| Android | Later | Remote controller first, not local coding runtime |

## 3. Proposed Workspace Structure

```text
crates/
|-- cadis-protocol/       # typed requests, responses, events
|-- cadis-avatar/         # renderer-independent Wulan avatar state engine
|-- cadis-core/           # session, event bus, orchestration contracts
|-- cadis-daemon/         # cadisd binary/runtime
|-- cadis-cli/            # cadis CLI
|-- cadis-models/         # model provider traits and implementations
|-- cadis-tools/          # native tool registry
|-- cadis-policy/         # approval and sandbox policy
|-- cadis-store/          # persistence and logs
|-- cadis-agents/         # agent session abstraction and roles
|-- cadis-workers/        # worktree and worker scheduling
|-- cadis-telegram/       # optional Telegram adapter
|-- cadis-voice/          # optional TTS/STT abstraction
|-- cadis-ui/             # Dioxus shared UI components later
`-- cadis-code-window/    # code work UI later
```

The workspace starts empty and adds crates only when implementation begins. This keeps the planning baseline honest and avoids fake compile targets.

## 4. Core Runtime Requirements

- Rust stable.
- Async runtime: Tokio unless a later ADR chooses otherwise.
- Structured errors with context.
- Protocol versioning from the first protocol crate.
- `unsafe` forbidden in workspace lints unless a crate-specific ADR allows it.
- Public types documented when exported.
- No core dependency on Node.js.
- Optional integrations behind features or separate crates.

Avatar-specific rule:

- `cadis-avatar` may define renderer-neutral Wulan state, body gesture state,
  face pose inputs, privacy config, and a direct-wgpu renderer contract.
- `cadis-avatar` must not depend on HUD frameworks, Tauri, `wgpu`, Bevy, camera
  APIs, microphone APIs, model providers, or daemon internals.
- Concrete `wgpu` or Bevy renderer adapters must live behind separate features
  or crates once implementation starts.

## 5. Transport Requirements

Initial transport should support local-only communication.

Options:

- Unix domain socket for Linux MVP.
- WebSocket later for HUD, browser-style clients, and remote relay.
- StdIO mode for testing and scripted clients.

Required behavior:

- Client identifies protocol version.
- Daemon rejects incompatible clients.
- Connection can subscribe to one session or global daemon events.
- Event stream supports backpressure strategy.
- Disconnected clients can reconnect and query recent state.

## 6. Event Requirements

Every event must include:

- event ID
- event type
- timestamp
- session ID when applicable
- source component
- protocol version

Important event categories:

- session lifecycle
- message delta and completion
- agent lifecycle
- tool lifecycle
- approval request and resolution
- code work output
- test result
- voice output
- errors

## 7. Data and Persistence Requirements

Default path:

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

- Write state atomically using temporary file plus rename.
- Use JSONL for append-only event logs.
- Redact secrets before writing.
- Do not store raw API keys in logs.
- Keep session and worker logs separate.
- Store enough metadata for crash recovery.

## 8. Security Requirements

### Tool Boundaries

Every tool call must declare:

- tool name
- arguments schema
- workspace path if applicable
- risk class
- expected side effects
- whether network access is needed
- whether secrets may be read

### Risk Classes

```text
safe-read
workspace-edit
network-access
secret-access
system-change
dangerous-delete
outside-workspace
git-push-main
git-force-push
sudo-system
```

### Approval Requirements

- Approval state is centralized.
- First valid response wins.
- Resolution is persisted.
- All connected surfaces receive the result.
- Denied actions must not execute.
- Expired approvals must fail closed.

## 9. Performance Requirements

| Area | Requirement |
| --- | --- |
| Event creation | Low allocation where practical |
| Tool dispatch | p95 under 25 ms excluding tool work |
| Stream relay | p95 under 20 ms per event locally |
| First status | under 100 ms after request accepted |
| Main loop | never blocked by worker execution |
| Logs | async append or bounded buffered write |

## 10. Model Provider Requirements

Provider trait should support:

- streaming text
- structured tool calls
- capability metadata
- cancellation
- provider-specific options
- token or usage metadata when available
- typed error mapping

Capabilities:

```text
tool_calling
vision
json_schema
reasoning_effort
prompt_cache
max_context_tokens
local_model
streaming
```

## 11. Tool Runtime Requirements

Native tools are preferred over MCP for core functionality. MCP can be an extension layer later.

Initial native tools:

- `file.read`
- `file.search`
- `file.patch`
- `shell.run`
- `git.status`
- `git.diff`
- `git.worktree.create`
- `git.worktree.remove`

Tool execution must be:

- cancellable
- observable
- policy-gated
- logged with redaction
- tested with success, failure, denial, and timeout cases

## 12. Coding Work Isolation

Parallel coding agents must not edit the same working directory directly.

Default flow:

```text
task -> create worker -> create git worktree -> edit -> test -> diff -> review -> user approval -> apply
```

Requirements:

- Worktree path must live under CADIS-controlled directory unless configured otherwise.
- Worker lifecycle events must include non-destructive worktree intent metadata
  before any `git.worktree.create` backend exists or runs.
- Worktree intent must identify the planned worktree root, worker-specific path,
  branch name, base ref when known, lifecycle state, and cleanup policy.
- Worker lifecycle events must include artifact locations for patch, test report,
  summary, changed-files manifest, and memory-candidate outputs.
- Patch application requires approval when it changes tracked files.
- Worktree cleanup must be explicit or policy-controlled.
- Worker output must stream without blocking the main session.

## 13. UI Technical Requirements

The first GUI should be delayed until daemon behavior is stable.

When implemented:

- Dioxus Desktop is the preferred default.
- UI communicates only through daemon protocol.
- HUD does not execute tools directly.
- Code work window uses structured events for diffs, logs, and tests.
- UI must show approval risk clearly.
- Optional Wulan avatar rendering consumes daemon-derived HUD state through
  `cadis-avatar` frames. The default orb must remain available as fallback.
- Bevy is deferred for Wulan unless a separate decision accepts a broader 3D
  scene engine.

## 14. Testing Requirements

### Unit Tests

- protocol serialization
- policy decisions
- redaction
- config loading
- tool schema validation
- event ordering utilities

### Integration Tests

- daemon starts and accepts client
- CLI sends chat request
- tool request flows through policy
- approval denies execution
- approval allows execution
- logs are persisted and redacted

### Conformance Tests

- model provider streaming contract
- tool registry contract
- event protocol compatibility

## 15. Release Engineering Requirements

- CI should run formatting, lint, tests, and docs checks once code exists.
- Release artifacts should include checksums.
- Changelog must be updated for every release.
- Dependency licenses must be audited before public binary release.
- Versioning should use SemVer after first release.

## 16. Documentation Requirements

- Public docs in English for open-source readiness.
- Architecture decisions tracked in `docs/11_DECISIONS.md`.
- Security-sensitive behavior documented before implementation.
- Examples must avoid real secrets and private machine paths.
