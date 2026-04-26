# Installation

## Status

CADIS has no installable runtime release yet.

This document defines the intended install paths for future releases.

## From Source

After implementation begins:

```bash
git clone https://github.com/cadis-ai/cadis.git
cd cadis
cargo build --release
```

Expected binaries:

```text
target/release/cadis
target/release/cadisd
```

## Local State

CADIS stores local state in:

```text
~/.cadis
```

Expected contents:

```text
config.toml
logs/
sessions/
workers/
worktrees/
approvals.json
```

## First Run Target

The first supported runtime flow will be:

```bash
cadisd
cadis status
cadis chat "hello"
```

## Package Targets Later

- Linux tarball.
- Debian package.
- AppImage or similar desktop package.
- Homebrew formula for macOS later.
- Windows installer later.

