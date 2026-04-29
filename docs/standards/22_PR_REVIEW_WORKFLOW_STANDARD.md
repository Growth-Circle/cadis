# PR Review Workflow Standard

## Purpose

This standard defines the mandatory pull request review workflow that every
maintainer and AI agent must follow after pushing to `main`. It ensures
community contributions are acknowledged, evaluated, and integrated before
any release is tagged.

Skipping this workflow risks shipping broken releases, ignoring contributor
work, and accumulating stale PRs that discourage future contributions.

## The Rule

> **After every push to `main`, check open PRs before doing anything else.**

This is non-negotiable. No release tag may be created while actionable PRs
remain unreviewed.

## Workflow

### Step 1: Check Open PRs

After every push (or at minimum before every release), run:

```bash
gh pr list --state open
```

### Step 2: Triage Each PR

For every open PR, determine one of these actions:

| Action | When | How |
|--------|------|-----|
| **Merge** | PR is correct, tests pass, no conflicts | `gh pr merge <N> --squash` |
| **Apply manually** | PR has conflicts but the fix is valid | Cherry-pick the meaningful changes, credit the author in the commit message |
| **Request changes** | PR has issues but the intent is good | Comment with specific feedback, keep PR open |
| **Close** | PR is outdated, superseded, or out of scope | Close with a respectful comment explaining why |
| **Defer** | PR is valid but not urgent | Label `deferred`, comment with timeline |

### Step 3: Credit Contributors

When applying PR changes manually (because of conflicts), always:

1. Mention the PR number in the commit message: `(from #N)`
2. Credit the author: `Co-authored-by: Name <email>` or mention in commit body
3. Comment on the PR explaining it was applied and in which commit

### Step 4: Verify CI

After merging or applying PRs:

```bash
gh run list --limit 3          # check CI status
gh run view <ID> --json conclusion --jq '.conclusion'
```

All required checks must pass before tagging a release.

### Step 5: Release Gate

Before creating a release tag, confirm:

- [ ] `gh pr list --state open` shows zero actionable PRs
- [ ] All CI workflows on `main` are green
- [ ] `RELEASE_NOTES.md` is updated and follows `21_RELEASE_NOTES_STANDARD.md`
- [ ] Version numbers are bumped in all crates and `npm/cadis/package.json`
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --lib --bins` passes

Only then:

```bash
git tag -a v{version} -m "v{version} — {summary}"
git push origin v{version}
```

## PR Response Time

| PR Type | Target Response | Action |
|---------|----------------|--------|
| Security fix | < 24 hours | Review + merge or request changes |
| Bug fix | < 48 hours | Review + merge or request changes |
| Feature | < 1 week | Triage + initial review |
| Documentation | < 48 hours | Review + merge |
| CI/infrastructure | < 48 hours | Review + merge |

## PR Labels

Use these labels to track PR state:

- `ready-for-review` — PR is complete and waiting for review
- `changes-requested` — reviewer asked for changes
- `approved` — approved, ready to merge
- `deferred` — valid but not merging now
- `superseded` — replaced by another PR or direct commit
- `community` — from an external contributor (prioritize response)

## Anti-Patterns

Do NOT:

- Push to `main` and immediately tag a release without checking PRs
- Close PRs without explanation
- Ignore PRs from community contributors for more than a week
- Merge PRs that break CI
- Apply PR changes without crediting the author
- Let PRs accumulate past 10 open items without triage

## Checklist for AI Agents

When an AI agent is asked to commit, push, and release, it must:

1. `git push`
2. `gh pr list --state open` — review every PR
3. For each PR: merge, apply, request changes, close, or defer
4. `gh run list --limit 3` — verify CI green
5. Only then proceed to release tagging

## References

- `docs/standards/01_CONTRIBUTION_STANDARD.md`
- `docs/standards/07_RELEASE_STANDARD.md`
- `docs/standards/20_CI_CD_STANDARD.md`
- `docs/standards/21_RELEASE_NOTES_STANDARD.md`
- `CONTRIBUTING.md`
