# UI Feature Parity Checklist

## 1. Goal

This checklist tracks RamaClaw-to-CADIS UI parity. A checked item means the CADIS implementation preserves the user-visible behavior and maps it to CADIS daemon protocol/state.

## 2. Source Audit

- [x] Audit `ramaclaw-hud` app composition.
- [x] Audit config/settings window.
- [x] Audit agent rename flow.
- [x] Audit voice picker and TTS/STT behavior.
- [x] Audit theme system.
- [x] Audit orbital HUD layout.
- [x] Audit gateway topics and outgoing commands.
- [x] Audit approval card behavior.
- [x] Audit wizard and desktop window behavior.
- [x] Audit RamaClaw design specs.

## 3. Architecture Decisions

- [ ] Decide HUD toolkit: Tauri + React for fastest parity or Dioxus for Rust-first UI.
- [ ] Record toolkit decision in `docs/11_DECISIONS.md`.
- [ ] Define `cadis-hud.v1` protocol subset or reuse CADIS protocol directly.
- [x] Decide voice ownership: daemon owns status, doctor, preview protocol,
  stop protocol, and speech policy; HUD remains the local capture/playback
  bridge where desktop APIs require it.
- [ ] Decide how first-run wizard writes config.

## 4. Shell and Desktop Window

- [ ] Transparent frameless window.
- [ ] Default size around 1600x1000.
- [ ] Minimum size around 1200x760.
- [ ] Custom drag chrome.
- [ ] Window configure button.
- [ ] Pin or always-on-top toggle.
- [ ] Minimize button.
- [ ] Close button.
- [ ] Background opacity preference.
- [ ] Linux package behavior documented.

## 5. Status Bar

- [ ] Main agent display name in brand label.
- [ ] Daemon connection state.
- [ ] Main model label.
- [ ] Active agent count.
- [ ] Waiting agent count.
- [ ] Idle agent count.
- [ ] Optional latency display.
- [ ] Optional system stats later.

## 6. Orbital HUD

- [ ] 16:9 logical canvas.
- [ ] Central CADIS orb.
- [ ] Faint orbital rings.
- [ ] Dashed spokes.
- [ ] 12 non-overlapping agent slots.
- [ ] Agent satellite cards.
- [ ] Worker tree under agent card.
- [ ] Main orb state labels: idle, listening, thinking, speaking, working, waiting.
- [ ] Model/context/mode/voice meta ring.
- [ ] Orb animation state changes.
- [ ] Agent click or context menu opens actions.
- [ ] Agent rename opens from context action.

## 7. Agent Cards

- [ ] Show agent icon.
- [ ] Show display name.
- [ ] Show status dot.
- [ ] Show status label.
- [ ] Show role.
- [ ] Show current task verb.
- [ ] Show current task target.
- [ ] Show current task detail.
- [ ] Show model detail when idle.
- [x] Show worker count when workers exist.
- [x] Show nested workers.
- [ ] Support transient worker cards if desired.

## 8. Agent Rename

- [ ] Rename main agent from central orb.
- [ ] Rename subagent from agent card.
- [ ] Normalize whitespace.
- [ ] Max length 32 characters.
- [ ] Empty input falls back to default.
- [ ] Send `agent.rename` to daemon.
- [ ] Persist display name in daemon config/state.
- [ ] Emit `agent.renamed`.
- [ ] Update status bar and chat labels immediately after confirmed event.
- [ ] If daemon is disconnected, queue or save pending preference with visible warning.

## 9. Chat Panel

- [ ] Message log.
- [ ] User messages.
- [ ] Assistant messages.
- [ ] System messages.
- [ ] Streaming assistant placeholder.
- [ ] Composer textarea.
- [ ] Enter to send.
- [ ] Shift+Enter newline.
- [ ] Quick chips: yes, no, cancel, expand.
- [ ] Agent route chip or command prefix.
- [ ] Voice settings shortcut.
- [ ] Model settings shortcut.
- [ ] Send disabled when disconnected.
- [ ] Disconnected placeholder references CADIS daemon, not OpenClaw.

## 10. Voice UI

- [ ] Voice state machine: idle, listening, thinking, speaking.
- [ ] Curated bilingual voice catalog.
- [ ] Indonesian voices.
- [ ] English voices.
- [ ] Malay fallback voices.
- [ ] Voice selector.
- [ ] Rate slider.
- [ ] Pitch slider.
- [ ] Volume slider.
- [ ] Auto-speak toggle.
- [ ] Engine label.
- [ ] Test voice button.
- [ ] Stop test button.
- [ ] Error message.
- [ ] Last engine success hint.
- [ ] Voice preview uses main agent display name.
- [ ] Auto-speak only final assistant message.
- [ ] Code/diff/log content is not spoken.

## 11. Model UI

- [ ] List available models from daemon.
- [ ] Show default model.
- [ ] Per-agent model selector.
- [ ] Preserve current value if missing from catalog.
- [ ] Push model update to daemon.
- [ ] Persist per-agent model.
- [ ] Thinking mode toggle.
- [ ] Fast response toggle.
- [ ] Main orb meta ring updates after model change.

## 12. Appearance UI

- [ ] Six themes: arc, amber, phosphor, violet, alert, ice.
- [ ] Hue-driven OKLCH tokens.
- [ ] Theme swatches.
- [ ] Active theme visual state.
- [ ] Live theme update without restart.
- [ ] Theme persists through daemon config.
- [ ] Background opacity slider.
- [ ] Background opacity live update.
- [ ] Background opacity persists.

## 13. Approval UI

- [ ] Approval stack overlay.
- [ ] Newest approvals first.
- [ ] Card shows rule or risk class.
- [ ] Card shows agent.
- [ ] Card shows command/action.
- [ ] Card shows cwd/workspace.
- [ ] Card shows reason.
- [ ] Card shows risk summary.
- [ ] Card shows expiry if available.
- [ ] Deny button.
- [ ] Approve button.
- [ ] Button click sends daemon response.
- [ ] Card removed only after `approval.resolved`.
- [ ] First-response-wins state is reflected if another surface resolves.

## 14. Config Dialog

- [ ] One modal for all config.
- [ ] Voice tab.
- [ ] Models tab.
- [ ] Appearance tab.
- [ ] Window tab.
- [ ] Close on backdrop.
- [ ] Close button.
- [ ] Done button.
- [ ] Tab state persists for current UI session.
- [ ] Config writes route through daemon.

## 15. First-Run Wizard

- [ ] Theme step.
- [ ] Voice mode step.
- [ ] Telegram fallback step.
- [ ] Approval timeout step.
- [ ] Hotkey step.
- [ ] Save to `~/.cadis/config.toml` or daemon config API.
- [ ] Skip gracefully outside desktop runtime.
- [ ] Existing config suppresses wizard.

## 16. Gateway and Protocol

- [ ] Replace OpenClaw discovery with CADIS daemon discovery.
- [ ] Remove `~/.openclaw` reads.
- [ ] Use `~/.cadis` config/state.
- [ ] Subscribe to CADIS event stream.
- [ ] Handle daemon status.
- [ ] Handle model list response.
- [ ] Handle agent status.
- [ ] Handle agent task.
- [ ] Handle streaming messages.
- [ ] Handle approvals.
- [x] Handle worker lifecycle.
- [ ] Handle worker worktree and artifact metadata.
- [ ] Handle `patch.created` and `test.result` summaries.
- [ ] Handle route log.
- [ ] Send message through `message.send`.
- [ ] Send model update.
- [ ] Send agent rename.
- [ ] Send approval response.

## 17. Prototype Validation

- [x] HUD prototype can run against mock CADIS events without a full agent runtime for worker progress.
- [ ] Prototype view model contains only ephemeral UI state plus event-derived snapshots.
- [ ] Prototype does not use localStorage, browser storage, or UI files as authoritative durable state.
- [ ] Prototype shows daemon connected, disconnected, and reconnecting states.
- [ ] Prototype renders active, idle, waiting, failed, and cancelled agent states.
- [x] Prototype renders worker lifecycle updates through `worker.*` daemon events.
- [ ] Prototype renders worker worktree state and artifact references without
  reading arbitrary local files.
- [ ] Prototype renders approval lifecycle from `approval.requested` to `approval.resolved`.
- [ ] Prototype keeps approval card visible after click until daemon resolution event.
- [ ] Prototype confirms rename and model changes only after daemon events/responses.
- [ ] Prototype disables unsafe actions while disconnected.

## 18. Code Work Artifact View

- [ ] Open read-only code work window for code-heavy worker output.
- [ ] Render worker summary artifact.
- [ ] Render patch preview from `patch.diff` or daemon-provided patch summary.
- [ ] Render changed files from `changed-files.json`.
- [ ] Render test results from `test-report.json` and `test.result`.
- [ ] Render bounded terminal log summaries from `worker.log.delta`.
- [ ] Keep apply action as a daemon request that requires patch approval.
- [ ] Keep cleanup/discard action as a daemon request that requires cleanup approval.
- [ ] Confirm HUD/code work window never executes tools directly.
- [ ] Confirm parent checkout patch apply still goes through approval-gated
  `file.patch` or a future patch-apply tool.

## 19. Testing

- [ ] Theme helper tests.
- [ ] Agent name normalization tests.
- [ ] Agent rename protocol test.
- [ ] Voice prefs serialization tests.
- [ ] Config dialog render test.
- [ ] Approval card waits for resolved event.
- [ ] Gateway reconnect/backoff test.
- [ ] Protocol event mapping tests.
- [ ] Code work artifact view reducer/render tests.
- [ ] Screenshot parity: 1600x1000.
- [ ] Screenshot parity: 1920x1080.
- [ ] No OpenClaw text/path remains in UI.

## 20. Open-Source Cleanup

- [ ] Replace RamaClaw brand text with CADIS.
- [ ] Replace OpenClaw wording with CADIS daemon wording.
- [ ] Replace private source paths with public references or remove them.
- [ ] Recreate icons for CADIS.
- [ ] Confirm asset licensing.
- [ ] Ensure no provider keys or local config values are committed.
