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

- [x] Decide HUD toolkit: Tauri + React for fastest parity or Dioxus for Rust-first UI.
- [x] Record toolkit decision in `docs/11_DECISIONS.md`.
- [x] Define `cadis-hud.v1` protocol subset or reuse CADIS protocol directly.
- [x] Decide voice ownership: daemon owns status, doctor, preview protocol,
  stop protocol, and speech policy; HUD remains the local capture/playback
  bridge where desktop APIs require it.
- [ ] Decide how first-run wizard writes config.

## 4. Shell and Desktop Window

- [x] Transparent frameless window.
- [x] Default size around 1600x1000.
- [x] Minimum size around 1200x760.
- [x] Custom drag chrome.
- [x] Window configure button.
- [x] Pin or always-on-top toggle.
- [x] Minimize button.
- [x] Close button.
- [x] Background opacity preference.
- [x] Linux package behavior documented.

## 5. Status Bar

- [x] Main agent display name in brand label.
- [x] Daemon connection state.
- [x] Main model label.
- [x] Active agent count.
- [x] Waiting agent count.
- [x] Idle agent count.
- [ ] Optional latency display.
- [ ] Optional system stats later.

## 6. Orbital HUD

- [x] 16:9 logical canvas.
- [x] Central CADIS orb.
- [x] Faint orbital rings.
- [x] Dashed spokes.
- [x] 12 non-overlapping agent slots.
- [x] Agent satellite cards.
- [x] Worker tree under agent card.
- [x] Main orb state labels: idle, listening, thinking, speaking, working, waiting.
- [x] Model/context/mode/voice meta ring.
- [x] Orb animation state changes.
- [x] Agent click or context menu opens actions.
- [x] Agent rename opens from context action.

## 7. Agent Cards

- [x] Show agent icon.
- [x] Show display name.
- [x] Show status dot.
- [x] Show status label.
- [x] Show role.
- [x] Show current task verb.
- [x] Show current task target.
- [x] Show current task detail.
- [x] Show model detail when idle.
- [x] Show worker count when workers exist.
- [x] Show nested workers.
- [ ] Support transient worker cards if desired.

## 8. Agent Rename

- [x] Rename main agent from central orb.
- [x] Rename subagent from agent card.
- [x] Normalize whitespace.
- [x] Max length 32 characters.
- [x] Empty input falls back to default.
- [x] Send `agent.rename` to daemon.
- [ ] Persist display name in daemon config/state.
- [ ] Emit `agent.renamed`.
- [x] Update status bar and chat labels immediately after confirmed event.
- [x] If daemon is disconnected, queue or save pending preference with visible warning.

## 9. Chat Panel

- [x] Message log.
- [x] User messages.
- [x] Assistant messages.
- [x] System messages.
- [x] Streaming assistant placeholder.
- [x] Composer textarea.
- [x] Enter to send.
- [x] Shift+Enter newline.
- [x] Quick chips: yes, no, cancel, expand.
- [x] Agent route chip or command prefix.
- [x] Voice settings shortcut.
- [x] Model settings shortcut.
- [x] Send disabled when disconnected.
- [x] Disconnected placeholder references CADIS daemon, not OpenClaw.

## 10. Voice UI

- [x] Voice state machine: idle, listening, thinking, speaking.
- [x] Curated bilingual voice catalog.
- [x] Indonesian voices.
- [x] English voices.
- [x] Malay fallback voices.
- [x] Voice selector.
- [x] Rate slider.
- [x] Pitch slider.
- [x] Volume slider.
- [x] Auto-speak toggle.
- [x] Engine label.
- [x] Test voice button.
- [x] Stop test button.
- [x] Error message.
- [x] Last engine success hint.
- [x] Voice preview uses main agent display name.
- [x] Auto-speak only final assistant message.
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
- [x] Main orb meta ring updates after model change.

## 12. Appearance UI

- [x] Six themes: arc, amber, phosphor, violet, alert, ice.
- [x] Hue-driven OKLCH tokens.
- [x] Theme swatches.
- [x] Active theme visual state.
- [x] Live theme update without restart.
- [x] Theme persists through daemon config.
- [x] Background opacity slider.
- [x] Background opacity live update.
- [x] Background opacity persists.

## 13. Approval UI

- [x] Approval stack overlay.
- [x] Newest approvals first.
- [x] Card shows rule or risk class.
- [x] Card shows agent.
- [x] Card shows command/action.
- [x] Card shows cwd/workspace.
- [x] Card shows reason.
- [x] Card shows risk summary.
- [x] Card shows expiry if available.
- [x] Deny button.
- [x] Approve button.
- [x] Button click sends daemon response.
- [x] Card removed only after `approval.resolved`.
- [ ] First-response-wins state is reflected if another surface resolves.

## 14. Config Dialog

- [x] One modal for all config.
- [x] Voice tab.
- [x] Models tab.
- [x] Appearance tab.
- [x] Window tab.
- [x] Close on backdrop.
- [x] Close button.
- [x] Done button.
- [x] Tab state persists for current UI session.
- [x] Config writes route through daemon.

## 15. First-Run Wizard

- [x] Theme step.
- [x] Voice mode step.
- [x] Telegram fallback step.
- [x] Approval timeout step.
- [x] Hotkey step.
- [x] Save to `~/.cadis/config.toml` or daemon config API.
- [x] Skip gracefully outside desktop runtime.
- [x] Existing config suppresses wizard.

## 16. Gateway and Protocol

- [x] Replace OpenClaw discovery with CADIS daemon discovery.
- [x] Remove `~/.openclaw` reads.
- [x] Use `~/.cadis` config/state.
- [x] Subscribe to CADIS event stream.
- [x] Handle daemon status.
- [x] Handle model list response.
- [x] Handle agent status.
- [x] Handle agent task.
- [x] Handle streaming messages.
- [x] Handle approvals.
- [x] Handle worker lifecycle.
- [x] Handle worker worktree and artifact metadata.
- [ ] Handle `patch.created` and `test.result` summaries.
- [x] Handle route log.
- [x] Send message through `message.send`.
- [x] Send model update.
- [x] Send agent rename.
- [x] Send approval response.

## 17. Prototype Validation

- [x] HUD prototype can run against mock CADIS events without a full agent runtime for worker progress.
- [x] Prototype view model contains only ephemeral UI state plus event-derived snapshots.
- [x] Prototype does not use localStorage, browser storage, or UI files as authoritative durable state.
- [x] Prototype shows daemon connected, disconnected, and reconnecting states.
- [x] Prototype renders active, idle, waiting, failed, and cancelled agent states.
- [x] Prototype renders worker lifecycle updates through `worker.*` daemon events.
- [x] Prototype renders worker worktree state and artifact references without
  reading arbitrary local files.
- [x] Prototype renders approval lifecycle from `approval.requested` to `approval.resolved`.
- [x] Prototype keeps approval card visible after click until daemon resolution event.
- [x] Prototype confirms rename and model changes only after daemon events/responses.
- [ ] Prototype disables unsafe actions while disconnected.

## 18. Code Work Artifact View

- [x] Open read-only code work panel for selected worker output.
- [x] Render worker summary from daemon worker state.
- [x] Render patch artifact reference from daemon worker artifact metadata.
- [ ] Render changed files from `changed-files.json`.
- [x] Render test report artifact reference/status from daemon worker metadata.
- [x] Render bounded terminal log summaries from `worker.log.delta`.
- [ ] Keep apply action as a daemon request that requires patch approval.
- [x] Keep cleanup/discard action as a daemon request that requires cleanup approval.
- [x] Confirm HUD/code work panel never executes tools directly.
- [ ] Confirm parent checkout patch apply still goes through approval-gated
  `file.patch` or a future patch-apply tool.

## 19. Testing

- [ ] Theme helper tests.
- [x] Agent name normalization tests.
- [ ] Agent rename protocol test.
- [ ] Voice prefs serialization tests.
- [ ] Config dialog render test.
- [x] Approval card waits for resolved event.
- [x] Gateway reconnect/backoff test.
- [x] Protocol event mapping tests.
- [x] Code work artifact view reducer/render tests.
- [ ] Screenshot parity: 1600x1000. <!-- Manual verification required — see scripts/screenshot-parity.sh -->
- [ ] Screenshot parity: 1920x1080. <!-- Manual verification required — see scripts/screenshot-parity.sh -->
- [x] No OpenClaw text/path remains in UI.

## 20. Open-Source Cleanup

- [x] Replace RamaClaw brand text with CADIS.
- [x] Replace OpenClaw wording with CADIS daemon wording.
- [x] Replace private source paths with public references or remove them.
- [ ] Recreate icons for CADIS.
- [ ] Confirm asset licensing.
- [x] Ensure no provider keys or local config values are committed.
