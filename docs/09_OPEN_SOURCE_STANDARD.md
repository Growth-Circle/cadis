# Open Source Standard

## 1. Repository Requirements

Required before public release:

- `README.md`
- `LICENSE`
- `NOTICE`
- `CONTRIBUTING.md`
- `CODE_OF_CONDUCT.md`
- `SECURITY.md`
- `CHANGELOG.md`
- issue templates
- pull request template
- CI workflow
- product and technical docs
- clear roadmap

## 2. License Policy

Baseline license: Apache-2.0.

Rules:

- Do not import third-party source code without license review.
- Preserve upstream notices when required.
- Record major imports in `NOTICE`.
- Keep generated code identifiable when committed.
- Audit dependency licenses before binary release.

## 3. Governance

Initial governance is maintainer-led.

Maintainer responsibilities:

- keep roadmap realistic
- review security-sensitive changes carefully
- protect local-first principle
- require tests for policy, tools, and persistence
- enforce license review before source imports

Contributor expectations:

- small pull requests
- clear issue links
- tests or explicit test-gap notes
- security impact notes for risky areas
- no secrets in logs, docs, tests, or examples

## 4. Decision Process

Architecture-level changes require a decision record in `docs/11_DECISIONS.md`.

ADR-required changes include:

- transport protocol changes
- model provider contract changes
- tool execution model changes
- policy and approval behavior changes
- source imports from other projects
- license changes
- UI framework changes
- storage format changes

## 5. Branch and Release Model

Recommended branch model:

- `main`: always releasable or clearly pre-alpha stable
- feature branches: short-lived
- release tags: `v0.x.y`

Recommended release stages:

- `v0.1`: daemon pre-alpha
- `v0.3`: agent runtime alpha
- `v0.6`: desktop preview
- `v0.9`: beta
- `v1.0`: stable local runtime

## 6. CI Standard

Planning baseline:

- repository hygiene checks

After Rust code starts:

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- docs link check if practical
- macOS source-validation baseline and Windows portable-crate baseline, as
  documented in `docs/28_PLATFORM_BASELINE.md`
- dependency license audit before releases

## 7. Security Standard

Security-sensitive areas:

- shell execution
- file writes
- outside-workspace access
- secret reads
- network access
- approval resolution
- logging
- provider key handling
- worktree cleanup

Requirements:

- threat model before beta
- redaction tests before pre-alpha
- approval race tests before alpha
- policy bypass tests before alpha
- private vulnerability reporting path before public adoption

## 8. Documentation Standard

Public docs should be:

- English-first for open-source reach
- explicit about status and limitations
- free of private paths and secrets
- updated with behavior changes
- linked from README where practical

Required docs by v0.1:

- install from source
- run daemon
- run CLI chat
- configure first provider
- explain logs
- explain approvals

## 9. Issue Management

Recommended labels:

```text
bug
enhancement
docs
security
protocol
daemon
cli
policy
tools
models
agents
telegram
voice
hud
good first issue
help wanted
blocked
```

## 10. Pull Request Review Standard

Review should check:

- behavior correctness
- tests
- security impact
- performance impact
- dependency impact
- API stability
- docs impact
- license impact

Changes touching `cadis-policy`, `cadis-tools`, `cadis-store`, shell execution, or credential handling require stricter review.

## 11. Public Communication

The project should avoid overclaiming while pre-alpha.

Use accurate status labels:

- planning
- pre-alpha
- alpha
- beta
- stable

Do not claim production readiness before:

- policy tests pass
- persistence is reliable
- secret redaction is tested
- install and recovery docs exist
- known limitations are published
