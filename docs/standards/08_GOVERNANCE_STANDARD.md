# Governance Standard

## Purpose

This standard defines how CADIS decisions are made, reviewed, and communicated while the project is maintainer-led.

Governance should protect the architecture: one local daemon, many clients, central policy, and auditable tool execution.

## Governance Model

CADIS starts with maintainer-led governance.

Maintainers are responsible for:

- protecting the local-first model
- protecting `cadisd` authority boundaries
- keeping clients thin
- requiring security review for risky behavior
- enforcing license review before source imports
- keeping the roadmap honest
- keeping public status labels accurate
- making decision records visible

Contributors are responsible for:

- small, reviewable changes
- clear issue and pull request context
- tests or explicit test-gap notes
- docs updates for changed behavior
- security impact notes for risky areas
- dependency and license disclosure

## Decision Process

Most implementation changes can be reviewed through normal pull requests.

A decision record in `docs/11_DECISIONS.md` is required when a change:

- changes daemon authority boundaries
- changes protocol compatibility
- changes model provider contracts
- changes tool execution model
- changes approval semantics
- changes persistence format
- changes UI framework direction
- imports third-party source code
- changes license
- moves core logic into a client or adapter
- introduces a required hosted service

Decision records should include:

- status
- decision
- reason
- consequences
- alternatives considered when useful

## Review Authority

At least one maintainer review is required for:

- `cadisd`
- protocol crates or event contracts
- policy and approval logic
- tool runtime behavior
- shell, file, git, patch, or network tools
- credential handling
- logging and redaction
- persistence and migrations
- release workflow
- license or notice changes

Maintainers may require additional review for changes with broad compatibility or security impact.

## Conflict Resolution

When contributors disagree:

1. Restate the concrete technical decision.
2. Identify which CADIS principle is at stake.
3. Compare options against daemon authority, security, local-first operation, testability, and maintenance cost.
4. Record the accepted outcome in `docs/11_DECISIONS.md` if the result affects architecture or policy.

Personal preference is not enough to override daemon-first architecture or security requirements.

## Security Governance

Security-sensitive reports should not be handled in public issues when they involve:

- approval bypass
- command execution
- filesystem escape
- credential leakage
- prompt injection leading to tool misuse
- unsafe network access
- log redaction failure
- sandbox failure

Maintainers should triage private reports by:

- severity
- exploitability
- affected versions
- available workaround
- required fix owner
- disclosure timeline

Security fixes may be merged with limited public detail until users have a reasonable update path.

## Cadis-Specific Guardrails

The project should reject or redesign changes that:

- make a client the source of truth for sessions, tools, policy, approvals, or persistence
- bypass `cadisd` for risky actions
- duplicate approval decisions across interfaces
- require a cloud service for the core local runtime
- make Node.js a core daemon dependency
- import OpenClaw source without prior decision and license review
- hide major behavior behind undocumented config
- reduce observability for tool execution or approvals

## Public Communication

Public communication must be accurate about maturity.

Allowed status labels:

- planning
- pre-alpha
- alpha
- beta
- stable

Do not claim production readiness until security, persistence, recovery, install, and known limitation docs support that claim.

## References

- `AGENT.md`
- `CODE_OF_CONDUCT.md`
- `SECURITY.md`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/11_DECISIONS.md`
- `docs/14_SECURITY_THREAT_MODEL.md`
