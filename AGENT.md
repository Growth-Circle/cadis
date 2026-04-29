# CADIS Agent Guide

This file is the contributor and AI-agent operating guide for CADIS.

## Project Identity

CADIS is a Rust-first, local-first, model-agnostic multi-agent runtime.

Core rule:

```text
cadisd owns runtime authority.
All UI, Telegram, voice, and mobile clients are protocol clients.
```

Do not rebuild CADIS as an OpenClaw backend, a UI-first app, or a hosted SaaS. OpenClaw and RamaClaw are migration references only.

## Current Status

CADIS has a production-ready daemon (`cadisd`) and CLI (`cadis`) with a native `cadis-hud` prototype. The daemon runs on a Tokio async runtime with TCP and Unix socket transports. Key capabilities:

- **12 native tool backends**: `file.read`, `file.search`, `file.list`, `file.write`, `file.patch`, `shell.run`, `git.status`, `git.diff`, `git.log`, `git.commit`, `git.worktree.create`, `git.worktree.remove`
- **Agent orchestration**: model-driven sub-agent spawning, token tracking, and budget enforcement across 13 built-in agents with specialist personas
- **Full approval engine**: risk classification with 10 risk classes, audit trails, and expiry-checked approval state
- **Worker runtime**: worktree isolation for safe parallel code work
- **5 model providers**: Echo, Ollama, OpenAI, CodexCli, and Auto — all with native streaming
- **Durable state persistence**: crash recovery with consistent on-disk state
- **Output filter pipeline** for 60-90% token reduction (inspired by [RTK](https://github.com/rtk-ai/rtk))
- **Semantic-boundary truncation** and **trigram search index** (inspired by [QMD](https://github.com/tobi/qmd))

Start from:

- `README.md`
- `docs/00_PROJECT_CHARTER.md`
- `docs/06_IMPLEMENTATION_PLAN.md`
- `docs/07_MASTER_CHECKLIST.md`
- `docs/11_DECISIONS.md`
- `docs/standards/00_STANDARD_INDEX.md`

For UI work, also read:

- `docs/20_RAMACLAW_UI_ADAPTATION.md`
- `docs/21_UI_FEATURE_PARITY_CHECKLIST.md`
- `docs/22_UI_STATE_PROTOCOL_CONTRACT.md`
- `docs/23_UI_DESIGN_SYSTEM.md`

## Non-Negotiable Rules

- Keep core runtime Rust-first.
- Keep `cadisd` as the authority for sessions, agents, tools, policy, approvals, and persistence.
- Do not put core agent logic in HUD, Telegram, voice, Android, or CLI clients.
- Do not execute tools outside the policy engine.
- Do not log secrets.
- Do not import third-party source code without a decision record and license review.
- Do not use OpenClaw paths or config as CADIS runtime defaults.
- Do not make Node.js a core daemon dependency.
- Keep risky actions behind central approval.
- Follow `docs/standards/21_RELEASE_NOTES_STANDARD.md` for every release — update `RELEASE_NOTES.md` before tagging.
- Follow `docs/standards/22_PR_REVIEW_WORKFLOW_STANDARD.md` — after every push, check open PRs, triage them, and verify CI before releasing.

## Working Order

Follow this build order unless a decision record changes it:

```text
protocol
daemon
CLI
model streaming
tools
policy
persistence
agent runtime
workers
Telegram
voice
HUD
code work window
multi-agent tree
```

## Recommended Skills

Use project-local skills from `skills/`:

- `cadis-rust-core`: Rust workspace, daemon, CLI, store.
- `cadis-protocol`: protocol and event contract changes.
- `cadis-policy-security`: approval, risk, sandbox, threat model.
- `cadis-tool-runtime`: native tools and execution lifecycle.
- `cadis-model-provider`: model provider integration.
- `cadis-ramaclaw-ui`: RamaClaw HUD adaptation.
- `cadis-voice`: voice, TTS, STT, speech policy.
- `cadis-open-source`: docs, release, contribution hygiene.

Installed global skills that are relevant:

- `cli-creator`
- `doc`
- `playwright`
- `screenshot`
- `security-best-practices`
- `security-threat-model`
- `security-ownership-map`
- `speech`
- `transcribe`
- `gh-fix-ci`
- `gh-address-comments`

OpenAI docs are already available as a system skill.

## Task Routing

Use this quick routing table:

| Task | Read first |
| --- | --- |
| daemon or CLI | `skills/cadis-rust-core/SKILL.md` |
| protocol events | `skills/cadis-protocol/SKILL.md` |
| approval or sandbox | `skills/cadis-policy-security/SKILL.md` |
| file, shell, git tools | `skills/cadis-tool-runtime/SKILL.md` |
| model provider | `skills/cadis-model-provider/SKILL.md` |
| HUD, config window, theme, agent rename | `skills/cadis-ramaclaw-ui/SKILL.md` |
| TTS, STT, wake word, speech routing | `skills/cadis-voice/SKILL.md` |
| docs, release, GitHub hygiene | `skills/cadis-open-source/SKILL.md` |

## Testing Expectations

Before claiming completion, run the smallest relevant validation:

- docs-only: check links and grep for stale OpenClaw wording when relevant.
- Rust code: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
- protocol: serialization and compatibility tests.
- policy/tools: allow, deny, expiry, cancellation, redaction, and timeout tests.
- UI: screenshot parity and no-overlap checks.

If a command cannot be run, state why.

## Documentation Expectations

When behavior changes, update the related docs:

- requirements: `docs/03_FRD.md`
- architecture: `docs/05_ARCHITECTURE.md`
- implementation: `docs/06_IMPLEMENTATION_PLAN.md`
- checklist: `docs/07_MASTER_CHECKLIST.md`
- protocol: `docs/15_PROTOCOL_DRAFT.md`
- config: `docs/16_CONFIG_REFERENCE.md`
- decisions: `docs/11_DECISIONS.md`
- standards: `docs/standards/`

## Source Imports

Before importing code from any external project:

1. Record a decision in `docs/11_DECISIONS.md`.
2. Check license compatibility.
3. Preserve notices.
4. Prefer adapter or reimplementation if licensing or architecture is unclear.
