# Decision Log

This file records architecture and product decisions. It is intentionally lightweight until the project needs full ADR files.

## Accepted Decisions

### ADR-001: Start from a fresh monorepo

Status: Accepted.

Decision: CADIS starts as a clean Rust monorepo, not as an OpenClaw backend or direct source import.

Reason:

- The core must be fast and deeply controllable.
- UI and backend coupling should be avoided.
- License and architecture risks are lower at the start.

Consequence:

- More implementation work upfront.
- Cleaner long-term boundaries.

### ADR-002: Use `cadisd` as the core authority

Status: Accepted.

Decision: `cadisd` owns sessions, events, agents, tools, policy, and persistence.

Reason:

- Multiple interfaces need one source of truth.
- Approvals must be central.
- Tool execution must not be duplicated across clients.

### ADR-003: Keep interfaces thin

Status: Accepted.

Decision: CLI, Telegram, HUD, voice, and Android are clients of the daemon protocol.

Reason:

- Prevents inconsistent behavior.
- Keeps security and policy centralized.
- Makes interface development parallel later.

### ADR-004: Rust-first core

Status: Accepted.

Decision: Core runtime components are implemented in Rust.

Reason:

- Performance.
- Strong typing.
- Good fit for daemon, CLI, protocol, and native tools.
- Avoids Node dependency in core.

### ADR-005: Apache-2.0 baseline license

Status: Accepted.

Decision: Use Apache-2.0 for original CADIS baseline.

Reason:

- Permissive open-source license.
- Includes patent grant.
- Common for infrastructure projects.

Consequence:

- Imported source code must be license-compatible.
- Notices must be preserved.

### ADR-006: Runtime before HUD

Status: Accepted.

Decision: Build daemon, protocol, CLI, model stream, tools, policy, persistence, and agents before full HUD.

Reason:

- Prevents UI-first architecture.
- Makes all future interfaces easier.
- Keeps performance measurable.

### ADR-007: Native tools before MCP bridge

Status: Accepted.

Decision: Core tools are native Rust tools. MCP can be added later as an extension layer.

Reason:

- Native tools are easier to secure, test, and display in UI.
- Core tool behavior should not depend on external servers.

### ADR-008: Worktree isolation for coding workers

Status: Accepted.

Decision: Coding workers use git worktrees before patch application.

Reason:

- Prevents parallel edits from corrupting the main working tree.
- Makes patch review easier.
- Supports tester/reviewer workers.

### ADR-009: Logs are JSONL with redaction

Status: Accepted.

Decision: Event logs are append-only JSONL, with secrets redacted before write.

Reason:

- Easy to inspect.
- Good fit for event streams.
- Works before a database exists.

### ADR-010: Use RamaClaw as the canonical HUD reference

Status: Accepted.

Decision: CADIS desktop HUD adapts the RamaClaw orbital HUD design, feature set, and interaction model as the canonical UI reference.

Reason:

- RamaClaw already contains working UI behavior for config, voice, themes, model selection, agent rename, approvals, orbital agents, and desktop window controls.
- CADIS needs those features without inheriting OpenClaw as the core runtime.
- A documented adaptation contract prevents the UI port from losing behavior.

Consequence:

- CADIS must add protocol support for UI preferences, agent rename, per-agent model selection, voice preview, and window preferences.
- OpenClaw paths and localStorage ownership must be replaced by `cadisd` protocol and `~/.cadis` state.
- UI toolkit remains a separate pending decision.

### ADR-011: Use Serde JSON for the initial protocol crate

Status: Accepted.

Decision: `cadis-protocol` uses `serde` and `serde_json` for the first typed request, response, and event contract.

Reason:

- CADIS needs a stable JSON shape for CLI, daemon tests, HUD integration, Telegram, and future adapters.
- Serde is the Rust ecosystem standard for strongly typed serialization.
- JSON keeps early protocol examples easy to inspect and compare in tests.

Consequence:

- Protocol compatibility tests must cover serialized JSON shapes.
- Public protocol changes must update docs and tests together.
- Higher-performance encodings can be evaluated later without replacing the typed contract.

### ADR-012: Use egui for the first native HUD prototype

Status: Accepted.

Decision: The first CADIS HUD prototype uses `eframe/egui` as a Rust-native desktop client.

Reason:

- CADIS needs a window that can run immediately from the Rust workspace.
- The HUD must remain a protocol client and must not add Node.js to the daemon.
- `egui` supports custom-drawn orbital UI, low-radius panels, config controls, and a single Cargo-built binary.

Consequence:

- This is an operational prototype path, not a guarantee of final 100% RamaClaw visual parity.
- Tauri + React and Dioxus remain valid future options if the project optimizes for exact RamaClaw parity or Rust-first WebView UI.
- HUD state still belongs to `cadisd`; the UI only caches view state.

## Pending Decisions

### ADR-P001: First model provider

Options:

- OpenAI first.
- Ollama first.
- Custom HTTP first.

Current recommendation:

- Implement the trait so OpenAI and Ollama are both natural, then choose one for the first working path based on available keys and local setup.

### ADR-P002: Initial local transport

Options:

- Unix socket.
- WebSocket.
- Stdio.
- Unix socket plus stdio test mode.

Current recommendation:

- Unix socket for Linux runtime, stdio mode for tests.

### ADR-P003: GUI framework

Options:

- Dioxus Desktop.
- Tauri + React.
- Slint.

Current recommendation:

- Use the accepted `egui` prototype for the first runnable desktop HUD. If fastest 100% RamaClaw visual parity is required later, use Tauri + React for the HUD client while keeping `cadisd` as the core. If Rust-first WebView UI consistency becomes more important, evaluate Dioxus.

### ADR-P004: TTS provider

Options:

- Rust Edge TTS provider.
- OS native speech.
- Piper offline.
- Node compatibility wrapper.

Current recommendation:

- Rust TTS trait first, provider stub, then Edge TTS or OS-native provider. Node wrapper stays optional compatibility only.

### ADR-P005: Codex-derived capability path

Options:

- Direct fork.
- Extract compatible internals.
- Reimplement compatible behavior.
- Adapter to installed Codex.

Current recommendation:

- Do not import or fork Codex CLI code during v0.1.
- Use an optional adapter to the installed official Codex CLI for ChatGPT-plan
  auth experiments.
- Keep `cadisd` as the authority boundary; the adapter must not read or persist
  `~/.codex/auth.json`.
- Revisit direct integration only after daemon, protocol, policy, and tool
  runtime are stable.

## Decision Rules

Require a new decision record when a change:

- changes daemon authority boundaries
- changes protocol compatibility
- adds risky tool behavior
- changes approval semantics
- imports third-party source code
- changes license
- moves core logic into an adapter
- changes storage format
