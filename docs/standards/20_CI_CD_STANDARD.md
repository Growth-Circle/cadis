# CI/CD Standard

## Purpose

This standard defines the checks and automation CADIS should use to keep the repository releasable, secure, and aligned with daemon-first architecture.

CI should catch correctness, formatting, security, documentation, dependency, and release issues before they reach users.

## CI Principles

- Run fast checks on every pull request.
- Run deeper checks on merge to `main` and before release.
- Keep checks reproducible locally where practical.
- Treat security-sensitive failures as blocking.
- Do not require secrets for normal pull request validation.
- Do not allow CI to publish artifacts from untrusted pull requests.
- Keep release credentials scoped and protected.

## Required Pull Request Checks

GitHub repository rulesets enforce the current required checks on protected
refs. Rulesets should be changed through a reviewed branch and then updated in
GitHub repository settings or the GitHub Rulesets API.

Active repository rulesets:

- `CADIS main branch protection` targets `refs/heads/main`, blocks deletion
  and non-fast-forward updates, requires pull requests, requires resolved
  review threads, and requires the CI status checks listed below.
- `CADIS release tag protection` targets `refs/tags/v*` and blocks deletion
  and non-fast-forward updates for release tags.

Required `main` status checks:

- `Automated PR review`
- `Repository hygiene`
- `Rust workspace`
- `HUD frontend`
- `macOS Rust source baseline`
- `Windows portable crate baseline`

Baseline checks:

- automated public-safe PR metadata/diff guardrails for committed secrets,
  private local paths, generated artifacts, and local runtime/agent state
- repository hygiene check
- Markdown and documentation path check where tooling exists
- no committed secrets
- redacted `gitleaks detect --source .` scan before public release branches
- no generated artifacts unless expected
- no forbidden runtime defaults such as OpenClaw paths

Rust checks once crates exist:

```text
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Focused Rust checks may be used while iterating, for example
`cargo test -p cadis-avatar` for native avatar state changes, but required CI
should keep the full workspace checks green.

HUD checks once the Tauri app exists:

```text
cd apps/cadis-hud
pnpm lint
pnpm typecheck
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
```

The HUD job should install Linux Tauri dependencies and must not require model
provider credentials, microphone access, camera access, or audio devices.
Voice doctor behavior should be covered by deterministic unit tests where
possible; live mic and player checks remain local smoke tests.

Platform baseline checks:

- macOS may run Rust workspace source validation with `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
  selected portable/core crate tests.
- Windows must check only portable crates until daemon/CLI transport, shell,
  path, sandbox, state-store permission, HUD, and audio adapters exist.

Current Windows portable crate commands:

```text
cargo check -p cadis-protocol -p cadis-policy -p cadis-store -p cadis-models -p cadis-avatar --all-targets --all-features
cargo clippy -p cadis-protocol -p cadis-policy -p cadis-store -p cadis-models -p cadis-avatar --all-targets --all-features -- -D warnings
cargo test -p cadis-protocol -p cadis-policy -p cadis-models -p cadis-avatar --all-targets --all-features
```

Security and dependency checks once tooling is configured:

```text
cargo audit
cargo deny check
```

Protocol, policy, and tool checks should include:

- serialization and compatibility tests
- approval allow and deny tests
- approval expiry and first-response-wins tests
- cancellation and timeout tests
- redaction tests before log persistence
- workspace-boundary tests for file, shell, git, and patch tools

## Required Release Checks

Release workflows must run:

- all pull request checks
- clean checkout build
- dependency license audit
- vulnerability audit
- changelog check
- release notes check
- artifact build on supported platforms
- artifact checksum generation
- smoke tests for packaged binaries

Release workflows should not publish unless the tag matches the expected version format:

```text
v0.x.y
v0.x.y-alpha.N
v0.x.y-beta.N
v0.x.y-rc.N
```

## Workflow Files

Recommended workflows:

- `.github/workflows/pr-autoreview.yml`
- `.github/workflows/ci.yml`
- `.github/workflows/platform-baseline.yml`
- `.github/workflows/security.yml`
- `.github/workflows/docs.yml`
- `.github/workflows/release.yml`

Recommended templates:

- `.github/pull_request_template.md`
- `.github/ISSUE_TEMPLATE/bug_report.md`
- `.github/ISSUE_TEMPLATE/feature_request.md`
- `.github/ISSUE_TEMPLATE/security_report.md`
- `.github/ISSUE_TEMPLATE/docs_report.md`

## Cadis-Specific CI Gates

CI should protect daemon-first boundaries with targeted checks as the codebase grows:

- clients must not call tools directly when the daemon protocol should mediate
- clients must not own approval state
- clients must not own persisted session truth
- risky tools must declare risk class and policy requirements
- logs and persisted events must pass redaction tests
- model providers must stream through core provider traits
- UI and voice clients must consume daemon state through typed protocol events
- HUD voice checks must stay preflight/diagnostic unless daemon-owned voice
  execution is explicitly implemented
- native avatar crates must remain renderer/state boundaries and must not call
  tools, approvals, models, memory, or policy APIs
- platform checks must not imply unsupported runtime coverage; Windows remains
  portable-crate-only until transport, shell, path, sandbox, HUD, and audio
  adapters are implemented

Some of these checks may start as review checklist items and become automated as package boundaries stabilize.

## Secrets and Permissions

CI must not expose:

- model provider API keys
- signing keys
- release tokens
- private package credentials
- maintainer personal tokens

Rules:

- Pull request checks from forks must run without privileged secrets.
- Release signing should happen only on protected tags or approved release jobs.
- Logs must redact secrets and tokens.
- CI should fail closed when required release credentials are missing.

## Local Reproduction

Contributors should be able to reproduce core CI with:

```text
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cd apps/cadis-hud
pnpm lint
pnpm typecheck
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
```

Platform baseline CI is documented in `docs/28_PLATFORM_BASELINE.md`.
Additional documented commands should be added as CI expands, including docs checks, dependency checks, and package smoke tests.

## Failure Handling

When CI fails:

- identify the failing job and command
- reproduce locally when possible
- fix the smallest responsible change
- do not bypass failing security, policy, protocol, or release checks without maintainer approval
- record persistent infrastructure issues if they block contributors

## References

- `AGENT.md`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/12_TEST_STRATEGY.md`
- `docs/14_SECURITY_THREAT_MODEL.md`
- `docs/28_PLATFORM_BASELINE.md`
- `docs/standards/07_RELEASE_STANDARD.md`
