# Security Policy

CADIS will execute tools, edit files, call models, and coordinate agents. Security is a core product requirement, not a later hardening step.

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.9.x   | ✅ Current beta — security fixes applied |
| 1.0.x   | ✅ Will receive security fixes on release |
| < 0.9   | ❌ No longer supported |

## Reporting a Vulnerability

**Use GitHub Security Advisories.** File a private vulnerability report at:

> <https://github.com/Growth-Circle/cadis/security/advisories/new>

Private vulnerability reporting is enabled on this repository. Do not open
public issues for security vulnerabilities.

We aim to **acknowledge reports within 48 hours** and provide an initial
assessment within 7 days. Critical issues affecting approval bypass, command
execution, or credential leakage will be prioritized for expedited fixes.

Report categories include but are not limited to:

- approval bypass
- command execution
- filesystem escape
- credential leakage
- prompt injection leading to tool misuse
- unsafe network access
- log redaction failure
- sandbox failure

## Security Baselines

- Risky actions require central approval.
- Secrets must never be logged.
- OpenAI API keys must come from `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY`, never from committed config, examples, or logs.
- ChatGPT/Codex account auth must stay with the official Codex CLI. Treat `~/.codex/auth.json` as password-equivalent and never commit, copy, or log it.
- Tool calls must declare risk class and workspace boundary.
- Approval resolution must be atomic and first-response-wins.
- Shell execution must be traceable, cancellable, and policy-gated.
- Persistent logs should use redaction before write.

## Accidental Credential Commit

If a credential, token, event log, diagnostic bundle, or crash dump is committed:

1. Revoke or rotate the affected credential immediately.
2. Remove the file from the index with `git rm --cached` when it should stay local.
3. Rewrite public history when the secret reached a shared remote.
4. Invalidate provider, Telegram, or integration tokens that may have been exposed.
5. Add or tighten an ignore pattern, redaction rule, or secret scan before continuing.
