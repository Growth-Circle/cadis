# Platform Baseline

## 1. Purpose

This document defines CADIS platform support during the beta runtime
slice. It separates primary runtime targets from source validation so docs and
CI do not overstate current support.

## 2. Support Matrix

| Platform | Current status | CI baseline | Runtime claim |
| --- | --- | --- | --- |
| Linux desktop | Primary runtime and HUD target | Full Ubuntu CI, Rust workspace checks, HUD frontend checks, and Tauri native shell check | Supported source-built MVP target |
| macOS | Full test suite, no packaged runtime yet | Full Rust workspace format, clippy, and `cargo test --workspace` on `macos-latest` | No packaged runtime or HUD support claim yet |
| Windows | Full test suite (TCP transport), no packaged runtime yet | Full Rust workspace clippy and `cargo test --workspace` on `windows-latest`; Unix socket tests are `#[cfg(unix)]` only | No packaged daemon, HUD, or audio runtime claim yet |
| Android | Future remote controller target | None | Not a local runtime target |

Linux remains the first platform for `cadisd`, the CLI, local Unix socket
transport, the Tauri HUD, HUD voice capture/playback bridges, and desktop
runtime behavior.

macOS validation proves that the full Rust workspace compiles, lints, and passes
the complete test suite on `macos-latest`. Unix socket integration tests are
gated behind `#[cfg(unix)]` and run natively on macOS. It does not mean the
daemon, CLI, HUD packaging, voice bridge, sandboxing, or shell behavior is
supported for users on macOS.

Windows validation now runs the full Rust workspace test suite on
`windows-latest`. Unix socket integration tests are gated behind `#[cfg(unix)]`
and are automatically skipped. TCP transport is used where applicable. Packaged
daemon, HUD, voice bridge, and shell behavior are not yet supported for users on
Windows.

## 3. Platform Test Notes

Both macOS and Windows now run the full workspace test suite. Unix socket
integration tests are gated behind `#[cfg(unix)]` and are automatically skipped
on Windows. macOS runs them natively since it supports Unix sockets.

## 4. Workflow Commands

The platform workflow lives at
`.github/workflows/platform-baseline.yml`. It does not require secrets, model
provider credentials, microphone access, camera access, signing keys, or release
tokens.

macOS full test:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Windows full test:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

## 4.1 Release Workflow

The release workflow at `.github/workflows/release.yml` builds binaries for:

- `x86_64-unknown-linux-gnu` (native)
- `aarch64-unknown-linux-gnu` (cross-compiled)

Release artifacts include `cadis` and `cadisd` binaries with SHA-256 checksums.
The Tauri HUD is not included in release artifacts and must be built from source.

## 5. Promotion Requirements

Before CADIS can claim runtime support beyond Linux, the relevant platform must
have:

- local daemon transport that does not depend on Unix sockets where unsupported
- shell execution adapters with explicit policy and approval behavior
- path normalization and denied-path enforcement tests
- sandbox and permission behavior documented for that platform
- audio capture/playback or a documented unsupported voice state
- CI coverage that builds and tests the claimed runtime crates
- installation and known-limitation docs for that platform

Live provider tests and real device checks remain opt-in local validation and
must not become default pull request requirements while they need credentials or
hardware.
