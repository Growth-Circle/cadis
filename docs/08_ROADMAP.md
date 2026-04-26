# Roadmap

## Roadmap Principles

- Prove the daemon before building rich UI.
- Prove one provider before adding many providers.
- Prove policy before expanding tools.
- Prove worktree isolation before allowing parallel coding agents.
- Keep early releases small and honest.

## v0.0 - Planning Baseline

Status: in progress.

Deliverables:

- Open-source repository baseline.
- Product and technical docs.
- Implementation plan.
- Risk register.
- Decision log.

Exit criteria:

- Project can be published as a clean repository.
- First implementation sprint can begin.

## v0.1 - Daemon Pre-Alpha

Goal: local daemon and CLI can stream a simple model-backed chat.

Deliverables:

- Rust workspace crates.
- `cadisd` daemon.
- `cadis` CLI.
- Protocol and event types.
- Local transport.
- Session lifecycle.
- One model provider.
- JSONL event logs.

Exit criteria:

```bash
cadisd
cadis status
cadis chat "hello"
```

## v0.2 - Tools and Policy Alpha

Goal: CADIS can execute safe local tools and require approval for risky tools.

Deliverables:

- Native tool registry.
- File tools.
- Shell tool.
- Policy engine.
- CLI approvals.
- Redaction.
- Persistence.

Exit criteria:

- Safe reads execute automatically.
- Risky shell command asks for approval.
- Denial prevents execution.
- Approval state is logged.

## v0.3 - Agent Runtime Alpha

Goal: CADIS supports agent sessions, role metadata, and basic tool-call loop.

Deliverables:

- Agent session abstraction.
- Main agent.
- Coding agent skeleton.
- Agent lifecycle events.
- Budgets and timeouts.
- Cancellation.

Exit criteria:

- Agent can call tools through policy.
- Agent status is visible in CLI.

## v0.4 - Worker Isolation Alpha

Goal: code-heavy tasks run in isolated worktrees.

Deliverables:

- Worker scheduler.
- Git worktree creation.
- Worker logs.
- Diff generation.
- Patch approval flow.

Exit criteria:

- Coding worker can produce a diff.
- Patch is not applied without approval.

## v0.5 - Telegram and Voice Preview

Goal: remote control and speech output work for basic flows.

Deliverables:

- Telegram adapter.
- `/status`, `/spawn`, `/approve`, `/deny`.
- TTS provider trait.
- Speech policy.
- First voice provider or stub.

Exit criteria:

- Telegram can start a session and resolve approval.
- Voice speaks normal answer or short summary.

## v0.6 - Linux HUD Preview

Goal: desktop HUD can monitor and control daemon sessions.

Deliverables:

- Dioxus desktop app.
- Chat stream.
- Agent tree.
- Worker cards.
- Approval cards.
- Voice controls.

Exit criteria:

- HUD can show active session and resolve approvals.

## v0.7 - Code Work Window Preview

Goal: code-heavy tasks have a dedicated visual surface.

Deliverables:

- Code work window.
- Diff viewer.
- Terminal log panel.
- Test result panel.
- Apply/discard controls.

Exit criteria:

- Coding work no longer floods main chat.

## v0.8 - Multi-Agent Preview

Goal: controlled multi-agent tree works under limits.

Deliverables:

- Agent spawn.
- Agent kill.
- Agent tail.
- Result collection.
- Depth and child limits.
- Budget limits.

Exit criteria:

- Multiple workers can run without blocking main agent.

## v0.9 - Desktop Beta

Goal: Linux desktop beta is useful for real local work.

Deliverables:

- Stabilized daemon.
- Stabilized CLI.
- HUD and code window usable.
- Telegram and voice optional.
- Two or more model providers.
- Security docs.
- Install docs.

Exit criteria:

- External users can install, run, and complete a small coding task.

## v1.0 - Stable Local Runtime

Goal: stable Linux local runtime with clear extension points.

Deliverables:

- Stable protocol subset.
- Stable CLI commands.
- Stable policy behavior.
- Stable local storage format or migration path.
- Release artifacts.
- Security response process.

Exit criteria:

- CADIS can be recommended for daily local use with documented limitations.

## Later Roadmap

- Windows desktop.
- macOS desktop.
- Android remote controller.
- Daemon-owned memory runtime based on Wulan's `25_MEMORY_CONCEPT.md`.
- Offline TTS provider.
- Additional model providers.
- MCP extension bridge.
- Plugin SDK.
- Optional remote relay.
- Team policy packs.
