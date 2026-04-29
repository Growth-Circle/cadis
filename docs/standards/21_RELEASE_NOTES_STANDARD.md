# Release Notes Standard

## Purpose

This standard defines the format, structure, and rules for writing C.A.D.I.S.
release notes. It complements `07_RELEASE_STANDARD.md` (which covers the release
process, gates, and checks) by specifying how release notes are written and
presented to users.

Consistent release notes help users understand what changed, whether they need to
act, and who contributed.

## Release Title Format

Every GitHub release and `CHANGELOG.md` entry uses this title format:

```text
v{major}.{minor}.{patch} — {one-line summary}
```

Examples:

```text
v0.3.0 — Agent runtime alpha with multi-agent orchestration
v0.3.1 — Fix daemon deadlock on concurrent tool requests
v1.0.0 — Stable local runtime with full policy enforcement
```

The one-line summary is present tense, lowercase after the dash, and describes
the release theme — not an exhaustive list.

## Semver Rules

| Release type | When to use | Example |
| --- | --- | --- |
| **Patch** (`x.y.Z`) | Bug fixes only. No new features, no breaking changes. | `v0.3.1` |
| **Minor** (`x.Y.0`) | New features, backward-compatible improvements. May include bug fixes. | `v0.4.0` |
| **Major** (`X.0.0`) | Breaking changes to protocol, config, CLI, or public API. | `v1.0.0` |

Pre-release suffixes follow `07_RELEASE_STANDARD.md`:

```text
v0.4.0-alpha.1
v0.4.0-beta.2
v0.4.0-rc.1
```

## Required Sections

Release notes contain the following sections in order. Omit empty sections
except Highlights, Installation, and Full Changelog which are always present.

### 1. Highlights

Two to three sentences summarizing the release for non-technical readers. State
the theme, the most important change, and any required user action. Link to a
migration guide or blog post if one exists.

### 2. What's New

New features grouped by area. Each item uses a conventional-commit-style scope
prefix and links to its PR or commit. Attribute the contributor inline.

Areas: `daemon`, `cli`, `tools`, `hud`, `protocol`, `voice`, `avatar`, `store`,
`policy`, `models`, `workspace`.

### 3. Improvements

Enhancements to existing features: performance gains, UX refinements, refactors
that change observable behavior. Same format as What's New.

### 4. Bug Fixes

Bug fixes grouped by area. Same format.

### 5. Breaking Changes

Present only when breaking changes exist. Each entry explains:

1. What changed.
2. Why it changed.
3. What users must do (migration steps).

Place this section prominently — it must be impossible to miss.

### 6. Infrastructure & CI

Changes to CI pipelines, build scripts, release automation, dependency upgrades,
and security hardening. Include dependency version bumps here.

### 7. Documentation

New or updated documentation with links.

### 8. Contributors

A warm credits paragraph thanking all contributors by GitHub handle with profile
links. Every contributor who authored or co-authored a merged PR in the release
is listed.

### 9. Installation

Install and upgrade instructions for all distribution channels: npm, shell
script, PowerShell, and cargo from source. Include binary checksums in a
collapsible details block.

### 10. Full Changelog

A GitHub compare link from the previous tag to the current tag.

## Formatting Rules

### Entry format

Every changelog entry follows this format:

```text
- {scope}: {present-tense description} ([#{number}](url) by [@handle](profile))
```

Example:

```text
- daemon: Add agent health-check endpoint ([#100](https://github.com/Growth-Circle/cadis/pull/100) by [@RamaAditya49](https://github.com/RamaAditya49))
```

### Present tense

Use present tense for all entries: "Add", "Fix", "Remove", "Improve", "Update".
Not "Added", "Fixed", "Removed".

### Linking

Every item links to its PR or commit. Do not list changes without traceability.

### Contributor attribution

- Each entry includes `by @handle` inline.
- The Contributors section lists all contributors for the release.
- Use full GitHub profile URLs: `[@handle](https://github.com/handle)`.

### Scope prefixes

Group entries by area using scope prefixes. Valid scopes match the areas listed
in the What's New section above. Use lowercase.

### Checksums

Binary releases include SHA-256 checksums in a collapsible block:

```markdown
<details>
<summary>SHA-256 checksums</summary>

```text
abc123...  cadis-x86_64-unknown-linux-gnu
def456...  cadis-aarch64-apple-darwin
ghi789...  cadis-x86_64-pc-windows-msvc.exe
abc123...  cadisd-x86_64-unknown-linux-gnu
def456...  cadisd-aarch64-apple-darwin
ghi789...  cadisd-x86_64-pc-windows-msvc.exe
```

</details>
```

### Experimental features

Changes to experimental or unstable features (voice, avatar engine) use an
`[Experimental]` prefix in the entry description so users understand the
stability contract:

```text
- voice: [Experimental] Add wake-word detection via Whisper ([#200](...) by [@handle](...))
```

## Template

Use this template for every release. Replace placeholders. Remove empty sections
(except Highlights, Installation, Full Changelog).

````markdown
# C.A.D.I.S. v{major}.{minor}.{patch} — {one-line summary}

**Release date:** {YYYY-MM-DD}
**Maturity:** {pre-alpha | alpha | beta | rc | stable}

## Highlights

{Two to three sentences for non-technical readers. State the theme, the biggest
change, and any required user action.}

## What's New

- {scope}: {Description} ([#{number}]({url}) by [@{handle}]({profile}))
- {scope}: {Description} ([#{number}]({url}) by [@{handle}]({profile}))

## Improvements

- {scope}: {Description} ([#{number}]({url}) by [@{handle}]({profile}))

## Bug Fixes

- {scope}: {Description} ([#{number}]({url}) by [@{handle}]({profile}))

## Breaking Changes

> [!WARNING]
> This release contains breaking changes. Read the migration steps below.

- **{What changed}** — {Why it changed}. {What users must do}.
  ([#{number}]({url}))

## Infrastructure & CI

- {Description} ([#{number}]({url}) by [@{handle}]({profile}))

## Documentation

- {Description} ([#{number}]({url}) by [@{handle}]({profile}))

## Contributors

Thanks to {all contributors} for their contributions to this release!

{List each contributor: [@{handle}]({profile})}

## Installation

### npm (all platforms)

```bash
npm install -g @growthcircle/cadis@{version}
```

### Shell (Linux / macOS)

```bash
curl -fsSL https://github.com/Growth-Circle/cadis/releases/download/v{version}/cadis-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadis
curl -fsSL https://github.com/Growth-Circle/cadis/releases/download/v{version}/cadisd-$(uname -m | sed 's/arm64/aarch64/')-$([ "$(uname)" = "Darwin" ] && echo apple-darwin || echo unknown-linux-gnu) -o cadisd
chmod +x cadis cadisd
sudo mv cadis cadisd /usr/local/bin/
```

### PowerShell (Windows)

```powershell
$v = "{version}"
$cadisDir = "$env:LOCALAPPDATA\cadis"; New-Item -ItemType Directory -Force -Path $cadisDir | Out-Null
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/download/v$v/cadis-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadis.exe"
Invoke-WebRequest "https://github.com/Growth-Circle/cadis/releases/download/v$v/cadisd-x86_64-pc-windows-msvc.exe" -OutFile "$cadisDir\cadisd.exe"
```

### Build from source

```bash
cargo install cadis --version {version}
```

### Verify

```bash
cadis --version
cadisd --check
```

<details>
<summary>SHA-256 checksums</summary>

```text
{checksum}  cadis-x86_64-unknown-linux-gnu
{checksum}  cadis-aarch64-unknown-linux-gnu
{checksum}  cadis-x86_64-apple-darwin
{checksum}  cadis-aarch64-apple-darwin
{checksum}  cadis-x86_64-pc-windows-msvc.exe
{checksum}  cadisd-x86_64-unknown-linux-gnu
{checksum}  cadisd-aarch64-unknown-linux-gnu
{checksum}  cadisd-x86_64-apple-darwin
{checksum}  cadisd-aarch64-apple-darwin
{checksum}  cadisd-x86_64-pc-windows-msvc.exe
```

</details>

## Full Changelog

[v{prev}...v{version}](https://github.com/Growth-Circle/cadis/compare/v{prev}...v{version})
````

## Example: Patch Release

```markdown
# C.A.D.I.S. v0.3.1 — Fix daemon deadlock on concurrent tool requests

**Release date:** 2026-05-01
**Maturity:** alpha

## Highlights

This patch fixes a deadlock in the daemon when two agents request the same tool
simultaneously. All alpha users should upgrade.

## Bug Fixes

- daemon: Fix deadlock when two agents request the same tool concurrently ([#98](https://github.com/Growth-Circle/cadis/pull/98) by [@RamaAditya49](https://github.com/RamaAditya49))
- cli: Fix exit code on connection timeout ([#102](https://github.com/Growth-Circle/cadis/pull/102) by [@DeryFerd](https://github.com/DeryFerd))

## Contributors

Thanks to [@RamaAditya49](https://github.com/RamaAditya49) and
[@DeryFerd](https://github.com/DeryFerd) for their contributions to this
release!

## Installation

### npm (all platforms)

```bash
npm install -g @growthcircle/cadis@0.3.1
```

### Verify

```bash
cadis --version
cadisd --check
```

## Full Changelog

[v0.3.0...v0.3.1](https://github.com/Growth-Circle/cadis/compare/v0.3.0...v0.3.1)
```

## Design Rationale

This standard synthesizes practices from Tokio, Tauri, and Next.js:

- **Keep-a-Changelog categories** (Tokio): scannable, Rust ecosystem convention.
- **Inline attribution per entry** (Tauri): connects people to their work.
- **Warm credits section** (Next.js): builds community for a project seeking contributors.
- **Highlights section**: helps users of a multi-component project quickly assess relevance.
- **Breaking Changes first**: impossible to miss, with migration steps inline.
- **Install/Upgrade section**: necessary for multi-channel distribution (npm, cargo, binary).
- **No build logs in notes**: cargo audit runs in CI, not pasted into release notes.
- **Experimental prefix**: signals stability contract for voice, avatar, and other unstable features.

## References

- `07_RELEASE_STANDARD.md` — release process, gates, and checks
- `01_CONTRIBUTION_STANDARD.md` — commit message format and PR workflow
- `20_CI_CD_STANDARD.md` — release automation
- [Keep a Changelog](https://keepachangelog.com/)
- [Conventional Commits](https://www.conventionalcommits.org/)
- [Semantic Versioning](https://semver.org/)
