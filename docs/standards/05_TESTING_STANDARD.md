# CADIS Testing Standard

## 1. Purpose

This standard defines the minimum testing expectations for CADIS. It applies to protocol, daemon, CLI, model providers, tool runtime, policy, persistence, agents, voice, HUD, and release packaging.

CADIS is a local automation system that can read files, run commands, request approvals, persist logs, and coordinate agents. Testing must therefore prioritize correctness, safety, auditability, and deterministic behavior before UI polish.

## 2. Principles

- Tests must be deterministic by default.
- Safety-critical behavior must have direct tests, not only end-to-end coverage.
- Mock providers are required for CI; live model providers are optional and must be opt-in.
- Temporary directories must be used for filesystem tests.
- No test may require a real API key, personal config, or global daemon state.
- Tests must assert structured errors and events, not only success or failure.
- Regression tests must be added for every fixed safety bug.
- Large open-source practice applies: fast unit tests, focused integration tests, explicit fixtures, reproducible CI, and clear failure output.

## 3. Required Test Layers

### Unit Tests

Use unit tests for pure or narrow behavior:

- protocol serialization and deserialization
- version compatibility
- config parsing and defaults
- redaction functions
- policy decisions
- risk classification
- tool schema validation
- event formatting
- agent limits and budgeting
- voice routing decisions
- HUD state reducers and normalization helpers

### Integration Tests

Use integration tests where components interact:

- daemon startup and health checks
- CLI-to-daemon communication
- session lifecycle
- approval request and resolution
- tool execution through policy
- JSONL persistence
- provider streaming through mock servers
- cancellation and timeouts
- reconnect and subscription recovery

### Conformance Tests

Conformance tests are required for stable extension points:

- local protocol compatibility
- model provider contract
- tool registry contract
- approval policy contract
- event schema compatibility
- HUD protocol mapping

Provider and tool conformance fixtures must be reusable by downstream providers.

### End-to-End Tests

End-to-end tests are required for release candidates:

- start daemon
- connect CLI
- send chat request
- stream model response from mock provider
- request a tool call
- approve or deny action
- persist logs
- shut down cleanly

E2E tests should be few, stable, and representative.

## 4. Safety-Critical Coverage

The following areas require tests before the related feature is considered complete:

| Area | Required Coverage |
| --- | --- |
| Approval | allow, deny, expiry, duplicate responses, first-response-wins, race handling |
| Shell | safe command, risky command, timeout, cancellation, cwd validation, stderr capture |
| File tools | inside workspace, outside workspace, missing file, permission denied, patch conflict |
| Secrets | env redaction, provider key redaction, config redaction, log redaction |
| Persistence | atomic write, JSONL append, corrupt state recovery, no raw provider keys |
| Protocol | unknown compatible event, incompatible version rejection, schema examples |
| Workers | worktree create, diff, cleanup, missing git repo, failed cleanup reporting |
| UI approvals | card remains until `approval.resolved`, cross-surface resolution updates |

## 5. Test Data Rules

- Fake tokens must use obvious prefixes such as `cadis_test_`, `sk-test-`, or `dummy_`.
- Real provider keys must never be committed or printed in CI.
- Fixture repositories must be small and generated or vendored intentionally.
- Snapshot tests must avoid local absolute paths unless normalized.
- Golden JSON fixtures must include protocol version fields where applicable.
- Tests that write under `~/.cadis` must redirect CADIS home to a temporary directory.

## 6. CI Requirements

The default CI gate for Rust code is:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

When the HUD exists, UI CI must include:

```bash
npm test
npm run lint
npm run build
```

Equivalent commands are acceptable if the final HUD toolkit is not Node-based.

CI must publish clear logs for failing tests and must not upload secrets, local config, or raw event logs containing unredacted data.

## 7. Performance Tests

Performance tests must measure:

- daemon startup time
- time to first event
- event bus relay latency
- tool dispatch overhead
- JSONL append overhead
- approval fan-out latency
- provider streaming overhead with mock provider
- HUD event reducer throughput

Benchmarks should start as local developer commands. They should become CI checks only after stable thresholds and low variance are established.

## 8. UI and Visual QA

HUD testing is required once UI work begins:

- render tests for config dialog tabs
- reducer tests for daemon event mapping
- reconnect and backoff tests
- agent rename normalization tests
- approval card lifecycle tests
- screenshot parity at 1600x1000 and 1920x1080
- minimum size check at 1200x760
- all six themes
- no OpenClaw text or paths in CADIS UI

Screenshot checks must verify that the orb is nonblank, agent cards do not overlap the central orb, chat does not cover agent cards, approval cards do not cover dialogs, and text fits in the status bar.

## 9. Release Test Bar

Each release must document the test commands that passed.

Minimum release gates:

- v0.1: protocol serialization, daemon start, CLI status, mock chat, JSONL write, redaction
- v0.2: file tools, shell tool, policy allow/deny, approval race, approval persistence
- v0.3: agent lifecycle, tool-call loop, timeout, cancellation
- v0.4: worktree create/diff/apply flow, patch approval, worker cleanup
- v0.5: Telegram command parsing, Telegram approval resolution, voice content routing
- HUD alpha: protocol mapping, approval lifecycle, config persistence, visual parity screenshots

## 10. Flaky Test Policy

Flaky tests are release blockers when they cover policy, approvals, tools, persistence, protocol compatibility, or redaction.

Allowed responses:

- fix the race or nondeterminism
- quarantine the test with an issue and owner if non-critical
- replace real time with a controllable clock
- replace network calls with local mock servers

Disabling a failing safety test without a replacement is not allowed.
