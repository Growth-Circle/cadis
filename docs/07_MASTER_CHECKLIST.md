# Master Checklist

## 1. Repository Foundation

- [x] Create project folder.
- [x] Add README.
- [x] Add Apache-2.0 license.
- [x] Add NOTICE.
- [x] Add CONTRIBUTING.
- [x] Add SECURITY.
- [x] Add CODE_OF_CONDUCT.
- [x] Add CHANGELOG.
- [x] Add issue templates.
- [x] Add discussion template.
- [x] Add PR template.
- [x] Add CI hygiene workflow.
- [x] Add Rust workspace placeholder.
- [x] Add environment example.
- [x] Add AGENT.md.
- [x] Add CLAUDE.md.
- [x] Add project-local skills.
- [x] Initialize git repository.
- [x] Create initial commit.
- [x] Create remote repository.
- [x] Push initial baseline.

## 2. Product Documentation

- [x] Project charter.
- [x] Blueprint normalization.
- [x] PRD.
- [x] BRD.
- [x] FRD.
- [x] TRD.
- [x] Architecture.
- [x] Implementation plan.
- [x] Roadmap.
- [x] Open-source standard.
- [x] Risk register.
- [x] Decision log.
- [x] User installation guide.
- [x] Developer setup guide.
- [x] Provider configuration guide.
- [x] Security threat model.
- [x] API/protocol reference.
- [x] RamaClaw UI adaptation guide.
- [x] UI feature parity checklist.
- [x] UI state/protocol contract.
- [x] UI design system.
- [x] Contributor skills guide.
- [x] Wulan avatar engine plan.
- [x] Workspace architecture plan.
- [x] Large open-source project standards index.
- [x] Contribution standard.
- [x] Code standard.
- [x] Architecture standard.
- [x] Security standard.
- [x] Testing standard.
- [x] Documentation standard.
- [x] Release standard.
- [x] Governance standard.
- [x] Protocol standard.
- [x] Agent standard.
- [x] Tool runtime standard.
- [x] Approval policy standard.
- [x] Model provider standard.
- [x] Config and persistence standard.
- [x] UI HUD standard.
- [x] Voice standard.
- [x] Performance standard.
- [x] Observability standard.
- [x] License and dependency standard.
- [x] CI/CD standard.

## 3. Workspace Skeleton

- [x] Create `crates/cadis-protocol`.
- [x] Create `crates/cadis-core`.
- [x] Create `crates/cadis-daemon`.
- [x] Create `crates/cadis-cli`.
- [x] Create `crates/cadis-store`.
- [x] Create `crates/cadis-policy`.
- [x] Add workspace members.
- [x] Add shared lint config.
- [x] Add formatting check.
- [x] Add clippy check.
- [x] Add test CI.

## 4. Protocol

- [x] Define protocol version.
- [x] Define event metadata.
- [x] Define session IDs.
- [x] Define agent IDs.
- [x] Define tool call IDs.
- [x] Define approval IDs.
- [x] Define request enum.
- [x] Define response enum.
- [x] Define event enum.
- [x] Define content kind.
- [x] Define risk class.
- [x] Add serde support.
- [x] Add JSON examples.
- [x] Add compatibility tests.

## 5. Daemon

- [x] Create `cadisd` binary.
- [x] Add daemon config loader.
- [x] Add daemon health status.
- [x] Add local transport listener.
- [ ] Add event bus.
- [ ] Add event fan-out to multiple clients.
- [x] Add `session.subscribe` protocol/request baseline.
- [ ] Add live persistent `session.subscribe` stream.
- [ ] Avoid blocking daemon mutex during model generation.
- [x] Add session registry.
- [ ] Add shutdown handling.
- [x] Add structured logging.
- [ ] Add daemon integration test.

## 6. CLI

- [x] Create `cadis` binary.
- [x] Add `cadis daemon`.
- [x] Add `cadis status`.
- [x] Add `cadis chat`.
- [x] Add `cadis run`.
- [ ] Add `cadis approve`.
- [ ] Add `cadis deny`.
- [x] Add `cadis doctor`.
- [x] Add JSON output mode.
- [ ] Add CLI integration tests.

## 7. Model Provider Layer

- [x] Define `ModelProvider` trait.
- [ ] Define provider capabilities.
- [x] Define streaming event type.
- [ ] Add real provider streaming callback support.
- [ ] Add provider readiness and effective model metadata.
- [ ] Apply per-agent model selection to provider routing.
- [ ] Define cancellation behavior.
- [x] Define provider error mapping.
- [x] Implement first provider.
- [ ] Add provider conformance tests.
- [x] Add provider config docs.
- [ ] Add second provider.

## 8. Tool Runtime

- [ ] Define tool trait.
- [x] Define tool registry.
- [x] Define tool schema strategy.
- [x] Define tool lifecycle events.
- [x] Implement `file.read`.
- [x] Implement `file.search`.
- [ ] Implement `file.patch`.
- [ ] Implement `shell.run`.
- [x] Implement `git.status`.
- [ ] Implement `git.diff`.
- [ ] Add timeouts.
- [ ] Add cancellation.
- [x] Add tests for success and failure.

## 9. Policy and Approval

- [ ] Define policy config.
- [x] Define default risk rules.
- [ ] Add approval request type.
- [ ] Add approval resolution type.
- [ ] Implement first-response-wins.
- [ ] Implement approval expiry.
- [ ] Gate shell execution.
- [ ] Gate outside-workspace writes.
- [ ] Gate secret access.
- [ ] Gate dangerous delete.
- [ ] Add race condition tests.
- [ ] Add denial tests.

## 10. Persistence and Logs

- [x] Create `~/.cadis` layout.
- [x] Load `config.toml`.
- [x] Write session metadata.
- [x] Write agent metadata.
- [x] Write worker metadata for daemon-planned worker delegations.
- [x] Write JSONL event logs.
- [ ] Write approval state.
- [x] Add store-level atomic JSON state helpers under `~/.cadis/state`.
- [x] Implement atomic writes for store-level JSON metadata.
- [x] Implement redaction.
- [x] Add crash recovery metadata for daemon session/agent metadata.
- [x] Add daemon recovery for stale non-terminal worker metadata.
- [x] Add redaction tests.
- [x] Add store-level recovery tests for partial and corrupt metadata files.
- [x] Add daemon persistence integration tests for session/agent restart recovery.
- [x] Add daemon persistence integration tests for worker restart recovery.

## 11. Agent Runtime

- [ ] Define `AgentSession`.
- [ ] Define agent roles.
- [ ] Define agent lifecycle.
- [ ] Implement main agent.
- [x] Implement daemon-owned `@agent` routing baseline.
- [x] Implement client-driven `agent.spawn` baseline.
- [ ] Add agent-driven spawn through daemon-approved action.
- [x] Add request-driven spawn max depth, max children, and global cap.
- [x] Add route-time agent status events baseline.
- [ ] Add full lifecycle agent status events.
- [ ] Add budget limits.
- [ ] Add timeout limits.
- [ ] Add cancellation.
- [ ] Add tool-call loop.
- [ ] Add model fallback behavior.

## 12. Worker Isolation

- [ ] Define worker scheduler.
- [ ] Define worker state.
- [ ] Implement daemon worker registry.
- [ ] Implement `worker.tail`.
- [ ] Create git worktree.
- [ ] Stream worker logs.
- [ ] Add worker cancellation.
- [ ] Generate worker diff.
- [ ] Run tests in worker.
- [ ] Request patch approval.
- [ ] Apply approved patch.
- [ ] Cleanup worktree.
- [ ] Add worker isolation tests.

## 13. Telegram Adapter

- [ ] Choose Telegram crate.
- [ ] Create adapter crate.
- [ ] Connect to daemon protocol.
- [ ] Implement `/status`.
- [ ] Implement `/agents`.
- [ ] Implement `/workers`.
- [ ] Implement `/spawn`.
- [ ] Implement `/approve`.
- [ ] Implement `/deny`.
- [ ] Implement approval buttons.
- [ ] Add security notes for bot token.

## 14. Voice Output

- [ ] Define TTS provider trait.
- [ ] Define speech policy.
- [ ] Add voice on/off config.
- [ ] Add explicit TTS provider config (`edge`, `openai`, `system`).
- [x] Separate HUD STT language from TTS voice.
- [x] Add HUD-local voice doctor/preflight.
- [x] Add WebAudio PCM fallback for WebKit MediaRecorder zero-chunk mic capture.
- [ ] Promote voice doctor/preflight into daemon-visible status.
- [ ] Handle daemon voice events in HUD.
- [ ] Add provider stub.
- [ ] Implement first provider.
- [ ] Speak short normal answers.
- [ ] Summarize long answers.
- [ ] Block code/diff/log speech.
- [ ] Speak approval risk summary.
- [ ] Add speech routing tests.

## 15. HUD

- [x] Choose first production-oriented HUD framework: Tauri + React.
- [x] Create desktop app skeleton.
- [x] Connect to daemon protocol.
- [x] Show chat stream.
- [x] Show agent tree.
- [x] Add `@agent` mention picker baseline.
- [ ] Show worker progress.
- [x] Show approval cards.
- [x] Add voice controls.
- [x] Add local mic debug telemetry.
- [x] Add voice doctor UI in Settings.
- [x] Add Wulan Arc avatar option.
- [x] Document CADIS-native Wulan avatar engine direction.
- [x] Add `cadis-avatar` renderer-neutral avatar state crate.
- [x] Add status bar.
- [x] Add desktop packaging notes.
- [ ] Validate HUD prototype against RamaClaw adaptation contract.
- [ ] Render HUD from a mock CADIS daemon event stream.
- [x] Confirm HUD is protocol-client only and does not execute tools directly.
- [ ] Confirm durable HUD preferences are daemon-backed, not browser/local UI storage.
- [x] Confirm disconnected state references CADIS daemon, not OpenClaw.
- [ ] Confirm approval cards remain visible until `approval.resolved`.
- [x] Confirm chat sends through `message.send`.
- [x] Confirm agent rename sends `agent.rename` and updates only from `agent.renamed`.
- [x] Confirm model changes send `agent.model.set`.
- [x] Confirm theme and opacity changes route through `ui.preferences.set`.
- [x] Confirm avatar style changes route through `ui.preferences.set`.
- [x] Define renderer-neutral Wulan avatar render state.
- [ ] Connect native Wulan renderer to `cadis-avatar` frames.
- [ ] Spike focused Rust/wgpu Wulan renderer.
- [ ] Reconsider Bevy only through a decision record if wgpu is insufficient.
- [ ] Port Wulan portrait shader, particles, reticles, eye overlay, and mouth overlay from the Three.js prototype.
- [ ] Add Wulan body gesture set: idle breath, listening lean, nod, gaze shift, approval hand cue, speaking emphasis, coding focus, thinking scan, and error recoil.
- [ ] Add reduced-motion behavior for Wulan gestures.
- [ ] Keep optional face tracking off by default, local-only, permission-gated, and visibly indicated when active.
- [ ] Confirm Wulan native renderer failure falls back to the CADIS orb.
- [ ] Capture HUD screenshot parity at 1200x760, 1600x1000, and 1920x1080.
- [ ] Confirm no overlapping cards, status text, chat panel, approval stack, or central orb text.

## 15.1 Next Multi-Agent Execution Tracks

- [ ] Track A: daemon event bus and live session subscription.
- [ ] Track B: provider readiness, effective model metadata, and provider streaming.
- [ ] Track C: `AgentSession`, agent-driven spawn, limits, and worker registry.
- [ ] Track D: policy-backed tools and approval persistence.
- [ ] Track E: daemon-owned voice provider path, STT language setting, and voice doctor.
- [ ] Track F: durable metadata and restart recovery for sessions, agents, workers, and approvals.
- [x] Track F store baseline: atomic JSON helpers and fail-safe metadata recovery.
- [x] Track F daemon baseline: session/agent metadata survives runtime restart and cancelled sessions are removed.
- [x] Track F daemon worker baseline: worker metadata survives runtime restart and stale running workers recover as failed.
- [ ] Track G: CADIS-native Wulan avatar engine.
- [ ] Track H: profile homes, agent homes, workspace registry, grants, and worker worktrees.
- [x] Track H baseline: default profile layout plus persistent workspace registry/grants.

## 15.2 Workspace Architecture

- [x] Document profile home, agent home, project workspace, and worker worktree terms.
- [x] Document implemented-now vs future workspace architecture status.
- [x] Document workspace grants and fail-closed tool behavior.
- [x] Document denied paths for path resolution.
- [x] Document project `.cadis/media/` asset convention.
- [x] Define workspace protocol/types.
- [x] Implement `CADIS_HOME` and default `CADIS_PROFILE_HOME` resolver.
- [ ] Implement profile home manager.
- [ ] Implement agent home manager and templates.
- [x] Implement workspace registry and aliases baseline.
- [x] Implement workspace grants with expiry baseline.
- [ ] Enforce denied paths across file, shell, git, and worker tools.
- [x] Reject broad workspace roots and enforce safe-read workspace path guards.
- [ ] Implement project `.cadis/workspace.toml` support.
- [ ] Implement worker worktree creation under project `.cadis/worktrees/`.
- [ ] Persist worker artifacts under profile `artifacts/workers/`.
- [ ] Add project `.cadis/media/` manifests for generated media.
- [ ] Add workspace/profile/agent doctor checks.

## 16. Code Work Window

- [ ] Detect code-heavy task.
- [ ] Open code work window.
- [ ] Show diff viewer.
- [ ] Show terminal logs.
- [ ] Show test results.
- [ ] Show file tree.
- [ ] Add apply action.
- [ ] Add discard action.
- [ ] Add external editor action.
- [ ] Add code window routing tests.

## 17. Multi-Agent Tree

- [ ] Define tree data model.
- [ ] Enforce max depth.
- [ ] Enforce max children.
- [ ] Enforce max global agents.
- [ ] Enforce budget.
- [ ] Support spawn.
- [ ] Support kill.
- [ ] Support tail.
- [ ] Support result collection.
- [ ] Add fan-out tests.

## 18. Release Readiness

- [ ] Add install docs.
- [ ] Add build docs.
- [ ] Add release workflow.
- [ ] Add checksum generation.
- [ ] Add dependency license audit.
- [ ] Add threat model.
- [ ] Add benchmark suite.
- [ ] Add known limitations.
- [ ] Tag pre-alpha release.

## 19. RamaClaw UI Adaptation

- [x] Audit RamaClaw HUD code.
- [x] Audit RamaClaw design specs.
- [x] Document UI adaptation strategy.
- [x] Document feature parity checklist.
- [x] Document UI state and protocol contract.
- [x] Document UI design system.
- [x] Decide HUD toolkit.
- [x] Add `agent.rename` to protocol implementation.
- [x] Add `agent.model.set` to protocol implementation.
- [x] Add `ui.preferences.*` to protocol implementation.
- [x] Add `voice.preview` and `voice.stop` to protocol implementation.
- [ ] Add HUD preference config.
- [x] Add six theme presets to UI implementation.
- [x] Add unified config dialog to UI implementation.
- [x] Add agent rename dialog to UI implementation.
- [x] Add voice selector and preview to UI implementation.
- [x] Add per-agent model selector to UI implementation.
- [ ] Add screenshot parity tests.
- [x] HUD prototype preserves orbital shell: status bar, central orb, 12 agent slots, chat panel, approval stack, config dialog, and rename dialog.
- [x] HUD prototype preserves six-theme appearance system: `arc`, `amber`, `phosphor`, `violet`, `alert`, `ice`.
- [x] HUD prototype demonstrates Voice, Models, Appearance, and Window config tabs.
- [x] HUD prototype demonstrates worker tree rendering under parent agents.
- [x] HUD prototype demonstrates voice preview UI without speaking code, diffs, logs, or test output.
- [ ] HUD prototype passes open-source cleanup scan for OpenClaw runtime paths, private RamaClaw source paths, provider keys, and committed local config values.
