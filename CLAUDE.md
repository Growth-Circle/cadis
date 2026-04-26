# CLAUDE.md

This file gives Claude and Claude-style coding agents project-specific instructions.

## Read First

Read `AGENT.md` before doing implementation work.

For most tasks, also read:

- `README.md`
- `docs/06_IMPLEMENTATION_PLAN.md`
- `docs/07_MASTER_CHECKLIST.md`
- `docs/11_DECISIONS.md`
- `docs/standards/00_STANDARD_INDEX.md`

## Operating Rules

- Keep changes scoped.
- Prefer existing project docs and decisions over inventing new direction.
- Update docs when behavior or architecture changes.
- Never bypass `cadisd` authority.
- Never put tool execution or approval logic in UI clients.
- Never log raw secrets.
- Never import external source code without license review and a decision record.
- Treat RamaClaw and OpenClaw as references, not CADIS core dependencies.

## Skill Routing

Use these local skill files when relevant:

- `skills/cadis-rust-core/SKILL.md`
- `skills/cadis-protocol/SKILL.md`
- `skills/cadis-policy-security/SKILL.md`
- `skills/cadis-tool-runtime/SKILL.md`
- `skills/cadis-model-provider/SKILL.md`
- `skills/cadis-ramaclaw-ui/SKILL.md`
- `skills/cadis-voice/SKILL.md`
- `skills/cadis-open-source/SKILL.md`

For UI work, preserve the RamaClaw feature set documented in:

- `docs/20_RAMACLAW_UI_ADAPTATION.md`
- `docs/21_UI_FEATURE_PARITY_CHECKLIST.md`
- `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`
- `docs/23_UI_DESIGN_SYSTEM.md`

## Implementation Priorities

1. Protocol and events.
2. Daemon and CLI.
3. Model streaming.
4. Tools and policy.
5. Persistence.
6. Agent runtime.
7. Worker isolation.
8. Telegram and voice.
9. HUD and code work window.

## Validation

Run relevant checks before finalizing work.

If Rust code exists:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If UI code exists:

```bash
pnpm test
pnpm typecheck
pnpm lint
```

Only run commands that are valid for the current repository state.

## Communication

Lead with findings, changes, and verification. Mention blockers and test gaps clearly.
