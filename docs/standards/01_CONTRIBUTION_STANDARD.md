# Contribution Standard

## Purpose

This standard defines how contributions enter CADIS without weakening the daemon-first architecture, local-first trust model, or open-source maintainability.

CADIS is early-stage. Contributions should make the runtime easier to reason about before they make it larger.

## Core Rules

- `cadisd` owns sessions, agents, tools, policy, approvals, persistence, and event ordering.
- CLI, HUD, Telegram, voice, and Android code must remain protocol clients.
- Core runtime code should be Rust-first.
- Risky tool behavior must go through the central policy and approval engine.
- Source imports from external projects require license review and a decision record before code lands.
- Secrets must not appear in logs, docs, test fixtures, examples, screenshots, or issue content.
- Changes should be small enough for a reviewer to understand the behavior, risk, and rollback path.

## Required Contributor Files

The repository must keep these contributor-facing files current:

- `README.md`
- `CONTRIBUTING.md`
- `CODE_OF_CONDUCT.md`
- `SECURITY.md`
- `LICENSE`
- `NOTICE`
- `CHANGELOG.md`
- `AGENT.md`
- `docs/11_DECISIONS.md`
- `docs/12_TEST_STRATEGY.md`
- `docs/standards/`

GitHub project hygiene should include:

- issue templates for bugs, features, security-sensitive reports, and documentation gaps
- pull request template with test, security, documentation, dependency, and license prompts
- labels for `bug`, `enhancement`, `docs`, `security`, `protocol`, `daemon`, `cli`, `policy`, `tools`, `models`, `agents`, `telegram`, `voice`, `hud`, `good first issue`, `help wanted`, and `blocked`

## Contribution Workflow

1. Read `README.md`, `AGENT.md`, and the relevant project-local skill in `skills/`.
2. Confirm whether the change affects daemon authority, protocol compatibility, policy, tools, persistence, providers, UI state, or licensing.
3. Open or reference an issue for behavior changes.
4. Keep the pull request scoped to one concern.
5. Add tests for changed behavior, or state the explicit test gap and why it remains.
6. Update docs when behavior, configuration, architecture, or public workflows change.
7. Add or update a decision record in `docs/11_DECISIONS.md` for architecture-level changes.

## Pull Request Requirements

Every pull request must state:

- what changed
- why it changed
- user-visible behavior, if any
- daemon or protocol impact
- security impact
- dependency and license impact
- test commands run
- documentation updated or intentionally not needed

## Commit Style

Use clear commit messages. Conventional prefixes are recommended when they add clarity:

```text
feat: add daemon event bus skeleton
fix: prevent unsafe shell approval bypass
docs: clarify provider setup
test: cover approval resolution race
chore: update CI toolchain
```

Commits should not mix unrelated behavior, formatting, and generated output. Security-sensitive fixes should use neutral public wording when disclosure timing matters.

## Definition of Done

A contribution is done when:

- behavior matches the issue or documented intent
- daemon authority boundaries are preserved
- relevant tests pass or a maintainer accepts the explicit test gap
- docs and examples match the new behavior
- security, dependency, and license impacts are stated
- no secrets, private paths, or local-only assumptions are introduced
- CI is green or the remaining failure is unrelated and documented

Required checks for Rust changes:

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`

Platform baseline status and commands are documented in
`docs/28_PLATFORM_BASELINE.md`.

Required checks for docs-only changes:

- local spell/readability review
- link and path spot check
- search for stale OpenClaw wording where relevant
- confirmation that no private paths, tokens, or secrets were added

Required checks for protocol, policy, and tool changes:

- serialization or compatibility tests for protocol changes
- allow, deny, expiry, cancellation, and race tests for approval changes
- redaction tests for logs and persisted events
- workspace-boundary tests for file, shell, git, and patch tools
- timeout and cancellation tests for long-running tools

## Review Standard

Reviewers should prioritize:

- correctness and failure behavior
- daemon authority boundaries
- policy bypass risk
- secret exposure risk
- compatibility with existing protocol and config expectations
- tests that would fail before the change
- operational simplicity for local-first users
- dependency weight, license, and maintenance risk

Changes touching `cadisd`, protocol crates, approval logic, tool execution, persistence, credentials, shell execution, or workspace isolation require stricter review and should not be rubber-stamped.

## Cadis-Specific Expectations

- Do not move orchestration into a client to make an interface easier to build.
- Do not add a second approval path for convenience.
- Do not make Node.js, a browser runtime, or a hosted service a required core daemon dependency.
- Prefer typed protocol events over ad hoc client-specific state.
- Prefer native Rust tools before bridges to external tool servers.
- Keep long code, diffs, logs, and test output out of voice-first flows.
- Use git worktrees for isolated coding workers once coding-worker behavior exists.
- Record decision-making when a change affects long-term extensibility.

## References

- `README.md`
- `AGENT.md`
- `CONTRIBUTING.md`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/11_DECISIONS.md`
- `docs/24_CONTRIBUTOR_SKILLS.md`
