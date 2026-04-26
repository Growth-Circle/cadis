# Contributing to CADIS

CADIS is early-stage. Contributions should keep the core fast, local-first, auditable, and Rust-first.

## Development Standards

- Prefer small, reviewable pull requests.
- Keep core runtime code in Rust unless a document explicitly marks a component as optional or compatibility-only.
- Do not put core agent logic in UI clients, Telegram handlers, or voice adapters.
- Keep security-sensitive behavior behind the central approval and policy engine.
- Add tests for protocol, policy, persistence, and tool execution behavior.
- Avoid adding new dependencies without explaining the operational and security tradeoff.

## Commit Style

Use clear conventional prefixes where useful:

```text
feat: add event bus skeleton
fix: prevent unsafe shell approval bypass
docs: clarify model provider contract
test: cover approval resolution race
chore: update CI toolchain
```

## Pull Request Checklist

- The change is scoped to one concern.
- Public interfaces are documented.
- Security impact is described when the change touches tools, shell, files, network, credentials, or approvals.
- Tests or a clear test gap are included.
- Logs do not expose secrets.
- Generated files are not committed unless required.

## Design Changes

For architecture-level changes, open or update an ADR in `docs/11_DECISIONS.md` before implementation.

