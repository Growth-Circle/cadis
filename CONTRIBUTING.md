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

## For Maintainers: PR Review Workflow

After every push to `main`, maintainers must triage open PRs before tagging
any release. See `docs/standards/22_PR_REVIEW_WORKFLOW_STANDARD.md` for the
full workflow. Summary:

1. `gh pr list --state open` — review every PR
2. For each: merge, apply manually (credit author), request changes, or close
3. Verify CI is green
4. Only then tag a release

## Design Changes

For architecture-level changes, open or update an ADR in `docs/11_DECISIONS.md` before implementation.

