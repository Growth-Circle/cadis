# Documentation Standard

## Purpose

This standard defines how CADIS documentation stays accurate, useful, and aligned with the daemon-first architecture.

Documentation is part of the product surface. It must describe the system CADIS is building, not a UI-first fork, hosted SaaS, or OpenClaw backend.

## Core Rules

- Use `CADIS` for the product name and `cadis` for packages, binaries, directories, and commands.
- State current maturity honestly: planning, pre-alpha, alpha, beta, or stable.
- Make `cadisd` authority clear whenever runtime behavior is described.
- Describe interfaces as clients of the daemon protocol.
- Do not document OpenClaw paths, defaults, or architecture as CADIS defaults.
- Do not include secrets, private paths, local tokens, API keys, internal hostnames, or personal account details.
- Keep docs English-first for broad open-source access.

## Required Documentation Set

The repository should maintain:

- `README.md` for project identity, status, layout, and primary links
- `AGENT.md` for contributor and AI-agent operating rules
- `CONTRIBUTING.md` for contribution workflow
- `SECURITY.md` for vulnerability reporting and security baseline
- `CHANGELOG.md` for notable changes
- `docs/00_PROJECT_CHARTER.md` through active product and technical docs
- `docs/11_DECISIONS.md` for accepted and pending architecture decisions
- `docs/12_TEST_STRATEGY.md` for validation expectations
- `docs/13_GLOSSARY.md` for shared terms
- `docs/14_SECURITY_THREAT_MODEL.md` for security assumptions and abuse paths
- `docs/15_PROTOCOL_DRAFT.md` for daemon protocol concepts
- `docs/16_CONFIG_REFERENCE.md` for user-facing configuration
- `docs/17_DEVELOPER_SETUP.md` and `docs/18_INSTALLATION.md` for local setup
- `docs/standards/` for operational standards
- `skills/` docs for repeatable CADIS-specific workflows

## Required Updates

Update documentation in the same change when code or policy changes affect:

- daemon behavior
- CLI commands or flags
- protocol requests, events, or compatibility
- tool risk classes, permissions, or approval behavior
- model provider setup
- configuration keys or defaults
- persisted state, logs, or recovery behavior
- install, build, or release steps
- public contribution, governance, or support processes
- security assumptions, boundaries, or mitigations

Architecture-level changes require a corresponding update or new entry in `docs/11_DECISIONS.md`.

## Writing Rules

- Start each document with a clear title and purpose.
- Prefer direct, testable statements over broad claims.
- Mark future work as future work; do not imply it already exists.
- Use short sections and concrete lists for procedures.
- Use fenced code blocks for commands, config, event examples, and logs.
- Include expected outcomes for setup and verification commands.
- Name required files and checks explicitly.
- Keep examples local-first and privacy-preserving.
- Use stable terms from `docs/13_GLOSSARY.md` where available.

## Cadis-Specific Content Rules

When documenting architecture, include:

- `cadisd` as the single runtime authority
- daemon-owned sessions, events, tools, policy, approvals, and persistence
- thin protocol clients for CLI, HUD, Telegram, voice, and Android
- model-agnostic provider boundaries
- local-first state and user control

When documenting tools or approvals, include:

- risk class
- workspace boundary
- approval requirement
- cancellation behavior
- logging and redaction expectations
- user-visible status event expectations

When documenting UI or voice behavior, include:

- protocol events used by the client
- daemon-owned source of truth
- what content should remain visual instead of spoken
- accessibility and interruption expectations where relevant

## Documentation Checks

Before merging docs changes, contributors should run or manually complete:

- link and relative path spot check
- heading and numbering review
- stale status review
- search for private paths and placeholder secrets
- search for stale OpenClaw defaults where relevant
- check that public commands match the current workspace layout

Recommended future automated checks:

- Markdown lint
- link checker
- spelling checker with project dictionary
- documentation examples smoke test
- generated CLI help comparison

## References

- `README.md`
- `AGENT.md`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/11_DECISIONS.md`
- `docs/13_GLOSSARY.md`
- `docs/24_CONTRIBUTOR_SKILLS.md`
