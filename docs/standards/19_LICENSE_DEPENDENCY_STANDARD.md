# License and Dependency Standard

## Purpose

This standard defines how CADIS accepts dependencies, imported source, generated code, and notices.

CADIS uses Apache-2.0 for original project code. Dependencies and imports must preserve that legal and operational baseline.

## License Baseline

- Original CADIS code is Apache-2.0.
- `LICENSE` must contain the Apache-2.0 license text.
- `NOTICE` must preserve required attribution notices.
- License changes require a decision record in `docs/11_DECISIONS.md`.
- Third-party source imports require license review before code is committed.

## Dependency Rules

Dependencies should be added only when they provide clear value that is difficult to achieve with the standard library or existing project stack.

Before adding a dependency, contributors must document:

- purpose
- crate or package name
- version requirement
- license
- maintenance status
- security history if relevant
- runtime footprint
- whether it affects `cadisd`
- whether it touches tools, policy, credentials, networking, or persistence

Prefer:

- well-maintained Rust crates for core runtime behavior
- minimal transitive dependency trees
- typed APIs over stringly wrappers
- optional features for non-core integrations
- native Rust tools before external bridges

Avoid:

- required Node.js dependencies in the core daemon
- dependencies that force hosted-service coupling
- unmaintained packages for security-sensitive code
- large frameworks for small protocol or CLI tasks
- libraries with unclear provenance or generated blobs

## Approved and Restricted License Guidance

Generally acceptable after review:

- Apache-2.0
- MIT
- BSD-2-Clause
- BSD-3-Clause
- ISC
- Unicode-DFS-2016 for Unicode data where applicable

Requires maintainer review and explicit rationale:

- MPL-2.0
- EPL-2.0
- LGPL
- dual-license packages
- custom permissive licenses

Not acceptable for CADIS source imports without a major governance decision:

- GPL-only source
- AGPL-only source
- proprietary source
- code with missing or ambiguous license
- copied snippets from unknown sources

This guidance is not legal advice. When uncertain, do not import the code.

## Source Import Rules

Before importing source from another project:

1. Add or update a decision record in `docs/11_DECISIONS.md`.
2. Confirm license compatibility.
3. Confirm whether notices are required.
4. Prefer adapter or clean reimplementation when architecture or license is unclear.
5. Keep imported code isolated and identifiable.
6. Record provenance in comments or documentation where useful.
7. Update `NOTICE` when required.

CADIS must not import OpenClaw or RamaClaw source as a shortcut without decision and license review. They may be used as references only when the resulting CADIS implementation preserves daemon-first boundaries.

## Generated Code

Generated code may be committed when:

- the generator and inputs are documented
- regeneration is deterministic enough for review
- generated files are clearly identifiable
- licenses for generator output are compatible
- generated code does not hide security-sensitive behavior

Generated protocol code should be reviewed for compatibility, stability, and source-of-truth clarity.

## Dependency Checks

Required before binary release:

- dependency license audit
- transitive dependency review for core runtime crates
- vulnerability audit where tooling is available
- duplicate or abandoned dependency review
- `NOTICE` update check

Recommended Rust tooling once the workspace is active:

```text
cargo tree
cargo deny check
cargo audit
```

The exact tools may change, but the release must retain equivalent license and vulnerability visibility.

## Cadis-Specific Expectations

- Dependencies in `cadisd` carry more risk than dependencies in optional clients.
- Dependencies used by policy, tools, shell execution, persistence, logging, credentials, networking, or model streaming require stricter review.
- Model providers should be isolated behind traits so provider SDK churn does not spread through the core.
- UI dependencies must not become required to run the daemon.
- Optional bridges such as MCP should remain extension layers unless a later decision changes the architecture.

## References

- `LICENSE`
- `NOTICE`
- `docs/09_OPEN_SOURCE_STANDARD.md`
- `docs/11_DECISIONS.md`
- `docs/14_SECURITY_THREAT_MODEL.md`
