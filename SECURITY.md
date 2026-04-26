# Security Policy

CADIS will execute tools, edit files, call models, and coordinate agents. Security is a core product requirement, not a later hardening step.

## Supported Versions

No supported release exists yet. Security reports are still welcome during planning and pre-alpha implementation.

## Reporting a Vulnerability

Until a public security contact is created, report privately to the maintainers. Do not open public issues for vulnerabilities involving:

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
- Tool calls must declare risk class and workspace boundary.
- Approval resolution must be atomic and first-response-wins.
- Shell execution must be traceable, cancellable, and policy-gated.
- Persistent logs should use redaction before write.

