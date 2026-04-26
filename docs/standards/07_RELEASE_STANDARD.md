# Release Standard

## Purpose

This standard defines how CADIS releases are prepared, validated, tagged, and communicated.

CADIS should release only what is true: a local-first daemon runtime at its current maturity level. Release notes must not overstate readiness.

## Release Model

CADIS uses semantic versioning once the first release exists.

Recommended stages:

- `v0.1`: daemon pre-alpha
- `v0.3`: agent runtime alpha
- `v0.6`: desktop preview
- `v0.9`: beta
- `v1.0`: stable local runtime

Tags should use:

```text
v0.x.y
```

Pre-release tags may use:

```text
v0.x.y-alpha.N
v0.x.y-beta.N
v0.x.y-rc.N
```

## Release Branch Rules

- `main` should remain releasable or clearly marked as pre-alpha stable.
- Feature branches should be short-lived.
- Release branches are optional before beta.
- Hotfix branches should be scoped to the release line they repair.
- Generated artifacts must be reproducible or clearly documented.

## Required Release Files

Before a public release, confirm these files are present and current:

- `README.md`
- `LICENSE`
- `NOTICE`
- `CONTRIBUTING.md`
- `CODE_OF_CONDUCT.md`
- `SECURITY.md`
- `CHANGELOG.md`
- `docs/17_DEVELOPER_SETUP.md`
- `docs/18_INSTALLATION.md`
- `docs/11_DECISIONS.md`
- `docs/12_TEST_STRATEGY.md`
- release workflow under `.github/workflows/`

Before binary releases, also confirm:

- dependency license report
- third-party notice review
- build provenance notes
- supported platform list
- known limitations
- upgrade and rollback notes when persistence exists

## Required Checks

Minimum checks before any release:

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- docs link/path spot check
- license audit
- changelog review
- release notes review for accurate maturity claims

Additional checks before releases that include runtime behavior:

- daemon start and stop smoke test
- CLI connection smoke test
- model stream smoke test where provider credentials are available
- approval allow and deny smoke test
- tool cancellation smoke test
- JSONL log redaction smoke test
- persistence migration or recovery test when storage exists

Additional checks before releases that include clients:

- client connects only through daemon protocol
- no duplicated tool or approval authority in clients
- visible status for long-running operations
- voice output avoids long code, diffs, logs, and test output

## Release Procedure

1. Confirm the intended version and maturity stage.
2. Review open blockers tagged `security`, `protocol`, `daemon`, `policy`, `tools`, and `release`.
3. Run required checks locally and in CI.
4. Update `CHANGELOG.md`.
5. Update installation, configuration, and known limitation docs.
6. Confirm `NOTICE` and dependency license status.
7. Create the release tag.
8. Publish artifacts through the release workflow.
9. Publish release notes with limitations and upgrade guidance.
10. Monitor issue reports after release.

## Release Notes Requirements

Release notes must include:

- version and date
- maturity status
- major features
- breaking changes
- security-relevant changes
- install or upgrade instructions
- known limitations
- checks performed
- artifact names and supported platforms

Do not claim production readiness before:

- policy tests pass
- persistence is reliable
- secret redaction is tested
- install and recovery docs exist
- known limitations are published
- private vulnerability reporting path exists

## Cadis-Specific Gates

Before `v0.1`, CADIS should have:

- `cadisd` starts locally
- typed event protocol skeleton
- CLI client connection path
- basic model streaming path
- native tool dispatch skeleton
- central approval policy skeleton

Before alpha, CADIS should have:

- approval race tests
- policy bypass tests
- workspace-boundary tests
- JSONL redaction tests
- protocol compatibility tests

Before beta, CADIS should have:

- updated threat model
- documented recovery behavior
- documented unsupported operations
- security contact
- dependency license report
- reproducible release workflow

## References

- `CHANGELOG.md`
- `docs/08_ROADMAP.md`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/11_DECISIONS.md`
- `docs/12_TEST_STRATEGY.md`
- `docs/17_DEVELOPER_SETUP.md`
- `docs/18_INSTALLATION.md`
