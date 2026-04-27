# Test Strategy

## 1. Testing Philosophy

CADIS must be tested most heavily where mistakes are expensive:

- policy decisions
- approval resolution
- tool execution
- redaction
- persistence
- protocol compatibility
- worker isolation

UI tests matter later, but core safety tests come first.

## 2. Test Layers

### Unit Tests

Use for:

- protocol serialization
- config parsing
- redaction functions
- policy decisions
- risk classification
- tool schema validation
- event formatting

### Integration Tests

Use for:

- daemon and CLI communication
- session lifecycle
- approval flow
- tool execution through policy
- log persistence
- provider streaming with mock server

### Conformance Tests

Use for:

- model provider contract
- tool registry contract
- local protocol compatibility
- event schema compatibility
- renderer-neutral avatar frame compatibility

### End-to-End Tests

Use for:

- start daemon
- send CLI chat
- stream response
- request tool
- approve tool
- persist log
- launch HUD against daemon protocol mock or local socket
- verify HUD settings remain daemon-backed

## 3. Security Test Matrix

| Area | Required Tests |
| --- | --- |
| Approval | allow, deny, expire, duplicate response, race |
| Shell | safe command, risky command, timeout, cancellation |
| File | inside workspace, outside workspace, missing file, permission denied |
| Secrets | env var redaction, token redaction, config redaction |
| Logs | no raw provider keys, event IDs present, session IDs present |
| Worktrees | create, diff, cleanup, conflict, missing git repo |

## 4. Performance Tests

Measure:

- time to first event
- event bus relay latency
- tool dispatch overhead
- JSONL append overhead
- daemon startup time
- approval fan-out latency

Performance tests should run locally first and become CI benchmarks later only if stable enough.

## 5. Test Data Rules

- Never commit real API keys.
- Use fake tokens with obvious prefixes.
- Use temporary directories for filesystem tests.
- Use mock providers for deterministic model tests.
- Use small fixture repositories for worktree tests.

## 6. Minimum Test Bar by Release

### v0.1

- protocol serialization
- daemon starts
- CLI status
- CLI chat with mock provider
- JSONL log write
- redaction
- HUD frontend lint, typecheck, unit tests, build, and Tauri crate check
- HUD voice doctor preflight can report missing `whisper-cli`, missing model,
  blocked mic, Node helper, and audio player states without secrets
- `cadis-avatar` unit tests cover mode-to-gesture mapping, face-tracking privacy,
  reduced-motion behavior, and renderer frame shape

### v0.2

- file tools
- shell tool
- policy allow/deny
- approval race
- approval persistence

### v0.3

- agent lifecycle
- tool-call loop
- timeout
- cancellation

### v0.4

- worktree create/diff/apply flow
- patch approval
- worker cleanup

### v0.5

- Telegram command parsing
- Telegram approval resolution
- voice content routing

## 7. CI Coverage

CI must keep the desktop HUD and Rust daemon paths separate enough that a
frontend dependency failure is visible:

- repository hygiene checks required public files
- Rust checks run `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`
- native avatar checks are covered by workspace Rust checks; `cargo test -p cadis-avatar` is the focused local check for avatar-only changes
- HUD checks run from `apps/cadis-hud` with pnpm: `pnpm lint`, `pnpm typecheck`, `pnpm test`, and `pnpm build`
- HUD native shell check runs `cargo check --manifest-path apps/cadis-hud/src-tauri/Cargo.toml --locked`
- browser preview and Playwright E2E are local or later-stage checks until stable enough for required CI
- current orchestrator coverage should include route events, agent status events,
  request-driven spawn limits, and live `session.subscribe` fan-out to multiple
  session-filtered subscribers
- current daemon socket coverage should include two clients receiving the same
  session events and unrelated `daemon.status` / `agent.list` requests
  completing while message generation is in flight
