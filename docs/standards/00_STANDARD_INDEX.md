# CADIS Standards Index

## Purpose

This folder defines the standards required to run CADIS like a serious large open-source project.

These standards are mandatory for contributors and AI agents unless a maintainer-approved decision record says otherwise.

## Standards Set

| No | Standard | Purpose |
| --- | --- | --- |
| 01 | `01_CONTRIBUTION_STANDARD.md` | Contributor workflow, issues, PRs, review, commits, definition of done |
| 02 | `02_CODE_STANDARD.md` | Rust/code quality, crate boundaries, errors, logging, dependency use |
| 03 | `03_ARCHITECTURE_STANDARD.md` | Daemon-first architecture, adapter boundaries, ADR/RFC process |
| 04 | `04_SECURITY_STANDARD.md` | Security posture, vulnerability handling, secrets, supply chain |
| 05 | `05_TESTING_STANDARD.md` | Unit, integration, e2e, security, protocol, UI, performance tests |
| 06 | `06_DOCUMENTATION_STANDARD.md` | Docs structure, writing rules, examples, public-safe documentation |
| 07 | `07_RELEASE_STANDARD.md` | Versioning, changelog, artifacts, checksums, release gates |
| 08 | `08_GOVERNANCE_STANDARD.md` | Maintainers, ownership, decision-making, roadmap, conduct |
| 09 | `09_PROTOCOL_STANDARD.md` | Request/event naming, schema, versioning, compatibility |
| 10 | `10_AGENT_STANDARD.md` | Agent roles, spawning, budgets, worker isolation |
| 11 | `11_TOOL_RUNTIME_STANDARD.md` | Native tool schema, risk class, lifecycle, timeout, cancellation |
| 12 | `12_APPROVAL_POLICY_STANDARD.md` | Multi-surface approvals, first-response-wins, fail-closed rules |
| 13 | `13_MODEL_PROVIDER_STANDARD.md` | Provider trait, capabilities, streaming, errors, conformance |
| 14 | `14_CONFIG_PERSISTENCE_STANDARD.md` | `~/.cadis`, TOML config, JSONL logs, migrations, redaction |
| 15 | `15_UI_HUD_STANDARD.md` | RamaClaw UI parity, HUD state, theme, config window, screenshots |
| 16 | `16_VOICE_STANDARD.md` | TTS, STT, wake word, speech routing, voice preview |
| 17 | `17_PERFORMANCE_STANDARD.md` | Latency, overhead, startup, memory, benchmark gates |
| 18 | `18_OBSERVABILITY_STANDARD.md` | Logs, event IDs, trace IDs, redacted diagnostics, debug mode |
| 19 | `19_LICENSE_DEPENDENCY_STANDARD.md` | License policy, dependency approval, NOTICE, source imports |
| 20 | `20_CI_CD_STANDARD.md` | CI checks, required status, security scans, release automation |

## Priority Tiers

### Tier 0: Must Exist Before Runtime Implementation

- contribution
- code
- architecture
- security
- testing
- protocol
- tool runtime
- approval policy
- license/dependency
- CI/CD

### Tier 1: Must Exist Before Public Pre-Alpha

- documentation
- release
- governance
- config/persistence
- observability
- model provider

### Tier 2: Must Exist Before Desktop Alpha

- agent
- UI/HUD
- voice
- performance

## External Best-Practice Baseline

CADIS standards are informed by:

- OpenSSF Best Practices Badge and OpenSSF Scorecard for security posture and repository health.
- SLSA for supply-chain integrity and release provenance.
- Rust API Guidelines for Rust public API quality.
- Semantic Versioning for version behavior after public APIs are declared.
- Keep a Changelog for human-readable changelog structure.
- Conventional Commits for structured commit messages.
- GitHub community health guidance for contributor-facing project files.

## Cadis-Specific Baseline

The external baseline is adapted to CADIS constraints:

- `cadisd` is the authority.
- UI, Telegram, voice, Android, and CLI are clients.
- Tools must go through policy.
- Approvals fail closed.
- Logs are JSONL and redacted.
- Runtime state is local-first.
- Model providers are replaceable.
- RamaClaw is the canonical HUD reference.
- Code-heavy work is visual, not spoken.
- Linux is the primary runtime/HUD target; macOS and Windows validation status
  is defined in `docs/28_PLATFORM_BASELINE.md`.

## How To Use This Folder

When a task touches a standard area:

1. Read the relevant standard first.
2. Read the corresponding product/technical docs.
3. Make the change.
4. Update checklist and docs if behavior changes.
5. Run the relevant validation.
