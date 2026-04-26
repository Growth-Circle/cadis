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
- [x] Add PR template.
- [x] Add CI hygiene workflow.
- [x] Add Rust workspace placeholder.
- [x] Add environment example.
- [x] Add AGENT.md.
- [x] Add CLAUDE.md.
- [x] Add project-local skills.
- [x] Initialize git repository.
- [x] Create initial commit.
- [ ] Create remote repository.
- [ ] Push initial baseline.

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
- [ ] Create `crates/cadis-core`.
- [ ] Create `crates/cadis-daemon`.
- [ ] Create `crates/cadis-cli`.
- [ ] Create `crates/cadis-store`.
- [ ] Create `crates/cadis-policy`.
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

- [ ] Create `cadisd` binary.
- [ ] Add daemon config loader.
- [ ] Add daemon health status.
- [ ] Add local transport listener.
- [ ] Add event bus.
- [ ] Add session registry.
- [ ] Add shutdown handling.
- [ ] Add structured logging.
- [ ] Add daemon integration test.

## 6. CLI

- [ ] Create `cadis` binary.
- [ ] Add `cadis daemon`.
- [ ] Add `cadis status`.
- [ ] Add `cadis chat`.
- [ ] Add `cadis run`.
- [ ] Add `cadis approve`.
- [ ] Add `cadis deny`.
- [ ] Add `cadis doctor`.
- [ ] Add JSON output mode.
- [ ] Add CLI integration tests.

## 7. Model Provider Layer

- [ ] Define `ModelProvider` trait.
- [ ] Define provider capabilities.
- [ ] Define streaming event type.
- [ ] Define cancellation behavior.
- [ ] Define provider error mapping.
- [ ] Implement first provider.
- [ ] Add provider conformance tests.
- [ ] Add provider config docs.
- [ ] Add second provider.

## 8. Tool Runtime

- [ ] Define tool trait.
- [ ] Define tool registry.
- [ ] Define tool schema strategy.
- [ ] Define tool lifecycle events.
- [ ] Implement `file.read`.
- [ ] Implement `file.search`.
- [ ] Implement `file.patch`.
- [ ] Implement `shell.run`.
- [ ] Implement `git.status`.
- [ ] Implement `git.diff`.
- [ ] Add timeouts.
- [ ] Add cancellation.
- [ ] Add tests for success and failure.

## 9. Policy and Approval

- [ ] Define policy config.
- [ ] Define default risk rules.
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

- [ ] Create `~/.cadis` layout.
- [ ] Load `config.toml`.
- [ ] Write session metadata.
- [ ] Write JSONL event logs.
- [ ] Write approval state.
- [ ] Implement atomic writes.
- [ ] Implement redaction.
- [ ] Add crash recovery metadata.
- [ ] Add redaction tests.
- [ ] Add persistence tests.

## 11. Agent Runtime

- [ ] Define `AgentSession`.
- [ ] Define agent roles.
- [ ] Define agent lifecycle.
- [ ] Implement main agent.
- [ ] Add agent status events.
- [ ] Add budget limits.
- [ ] Add timeout limits.
- [ ] Add cancellation.
- [ ] Add tool-call loop.
- [ ] Add model fallback behavior.

## 12. Worker Isolation

- [ ] Define worker scheduler.
- [ ] Define worker state.
- [ ] Create git worktree.
- [ ] Stream worker logs.
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
- [ ] Add provider stub.
- [ ] Implement first provider.
- [ ] Speak short normal answers.
- [ ] Summarize long answers.
- [ ] Block code/diff/log speech.
- [ ] Speak approval risk summary.
- [ ] Add speech routing tests.

## 15. HUD

- [ ] Choose final UI framework.
- [ ] Create desktop app skeleton.
- [ ] Connect to daemon protocol.
- [ ] Show chat stream.
- [ ] Show agent tree.
- [ ] Show worker progress.
- [ ] Show approval cards.
- [ ] Add voice controls.
- [ ] Add status bar.
- [ ] Add desktop packaging notes.

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
- [ ] Decide HUD toolkit.
- [ ] Add `agent.rename` to protocol implementation.
- [ ] Add `agent.model.set` to protocol implementation.
- [ ] Add `ui.preferences.*` to protocol implementation.
- [ ] Add `voice.preview` and `voice.stop` to protocol implementation.
- [ ] Add HUD preference config.
- [ ] Add six theme presets to UI implementation.
- [ ] Add unified config dialog to UI implementation.
- [ ] Add agent rename dialog to UI implementation.
- [ ] Add voice selector and preview to UI implementation.
- [ ] Add per-agent model selector to UI implementation.
- [ ] Add screenshot parity tests.
