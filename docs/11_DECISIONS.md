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

### ADR-013: Adopt Wulan Arc as an optional HUD avatar

Status: Accepted.

Decision: The Tauri HUD can offer the contributed Wulan Arc hologram avatar as
an optional center avatar alongside the default CADIS orb.

Reason:

- The existing RamaClaw-style orb remains the default visual and preserves HUD
  parity.
- Wulan Arc gives the center avatar a more expressive state surface for
  listening, thinking, speaking, coding, and error states.
- The implementation stays isolated to `apps/cadis-hud` and is lazy-loaded so
  Three.js is not required by the daemon or the default orb path.
- Eye blink/gaze and mouth pulse are implemented as lightweight overlay
  animation, not as a full facial rig or lip-sync model.

License review:

- The sample came from the local `wulan-contribute/cadis-arc-avatar-sample`
  contribution and does not include a separate LICENSE or NOTICE file.
- The new runtime dependencies used by the HUD avatar path, `three`,
  `@react-three/fiber`, and `@react-three/drei`, are MIT licensed.

Consequence:

- HUD avatar choice is stored as daemon-owned UI preference
  `hud.avatar_style`.
- Contributors should keep future avatar variants as optional HUD assets and
  avoid moving browser/WebGL dependencies into `cadisd`.

### ADR-014: Build Wulan as a CADIS-native avatar engine

Status: Accepted.

Decision: CADIS will treat the current Three.js Wulan Arc avatar as a prototype
and design the long-term Wulan avatar as a native Rust rendering capability,
preferably a focused `wgpu` renderer rather than Bevy for the first production
path.

Reason:

- The avatar should remain local-first, fast, and daemon-driven without making
  browser WebGL the permanent animation boundary.
- Wulan's initial visual needs are narrow: portrait compositing, hologram
  shader, particles, reticles, eye/mouth overlays, body gestures, and
  state-driven transitions.
- A focused `wgpu` layer gives CADIS direct control over frame budget,
  deterministic fixtures, fallback behavior, and dependency surface.
- Bevy is better reserved for a future decision if CADIS needs a broader 3D
  scene engine, skeletal rigs, physics-like interaction, or a game-style ECS.

Consequence:

- `docs/26_WULAN_AVATAR_ENGINE.md` is the canonical Wulan native-engine plan.
- `crates/cadis-avatar` owns the renderer-independent state engine, body
  gesture state model, local-only face tracking privacy config, and
  dependency-free direct-wgpu contract.
- The Tauri/React/Three.js Wulan Arc path remains a lazy-loaded HUD prototype
  until native parity is demonstrated.
- No heavy `wgpu` or Bevy dependency is wired into the state crate yet.
- Face tracking is optional, off by default, local-only, and requires explicit
  permission plus visible camera-active UI.
- Avatar state and preferences remain daemon-owned protocol state; renderer
  animation state remains disposable HUD state.

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

### ADR-P006: RamaClaw-Parity HUD App

Decision:

- Add `apps/cadis-hud` as the production-oriented Tauri + React desktop HUD.
- Keep `crates/cadis-hud` as the lightweight Rust prototype while the richer HUD
  uses the existing RamaClaw visual language.
- The Tauri HUD is a protocol client only. It talks to `cadisd` through the
  `cadis_request` command and does not own credentials, sessions, approvals, or
  model provider state.

Rationale:

- The user already has a working RamaClaw visual design and wants CADIS to use
  that interaction model on desktop.
- `cadisd` remains the authority boundary, so ChatGPT Plus/Pro access continues
  through the official Codex CLI adapter instead of importing or reading Codex
  credentials from the HUD.

Consequences:

- Frontend dependencies are isolated under `apps/cadis-hud`.
- Generated frontend output, `node_modules`, Tauri build output, local sockets,
  logs, and credential-like files stay ignored.
- UI preference changes are sent through `ui.preferences.set`; agent rename and
  model selection are confirmed by daemon events.

### ADR-P007: Wulan Memory Architecture

Options:

- Simple session summaries only.
- File-backed memory without indexing.
- SQLite/FTS memory with a provenance ledger.
- External memory provider as the primary store.

Current recommendation:

- Treat Wulan's memory concept in `docs/25_MEMORY_CONCEPT.md` as the future
  CADIS memory direction.
- Keep memory daemon-owned: `cadisd` controls ACL, retrieval, ranking,
  promotion, persistence, and context compilation.
- Start with Markdown plus JSONL ledger plus SQLite metadata/FTS before adding
  vector retrieval or external providers.
- Keep external providers optional and additive; local memory remains the source
  of truth.
- Keep complex memory outside v0.1 unless this pending decision becomes accepted.

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
