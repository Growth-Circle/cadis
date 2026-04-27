# Platform Baseline

## 1. Purpose

This document defines CADIS platform support during the pre-alpha runtime
slice. It separates primary runtime targets from source validation so docs and
CI do not overstate current support.

## 2. Support Matrix

| Platform | Current status | CI baseline | Runtime claim |
| --- | --- | --- | --- |
| Linux desktop | Primary runtime and HUD target | Full Ubuntu CI, Rust workspace checks, HUD frontend checks, and Tauri native shell check | Supported source-built MVP target |
| macOS | Source validation and baseline only | Rust workspace format and clippy, plus selected portable/core crate tests on `macos-latest` | No packaged runtime or HUD support claim yet |
| Windows | Portable crate validation only | Selected portable crates check and clippy, plus pure portable crate tests on `windows-latest` | No daemon, CLI transport, HUD, shell, storage permission, or audio runtime claim yet |
| Android | Future remote controller target | None | Not a local runtime target |

Linux remains the first platform for `cadisd`, the CLI, local Unix socket
transport, the Tauri HUD, HUD voice capture/playback bridges, and desktop
runtime behavior.

macOS validation proves that the Rust workspace remains source-portable enough
to format, compile, lint, and run selected portable/core tests under Unix-like
desktop conditions. It intentionally avoids the Linux-first daemon socket
integration tests until local transport paths are platform-adapted. It does not
mean the daemon, CLI, HUD packaging, voice bridge, sandboxing, or shell behavior
is supported for users on macOS.

Windows validation is deliberately narrower. Until CADIS has Windows transport,
shell, path, sandbox, storage permission, and audio adapters, Windows CI must
not build or test the daemon, CLI, core runtime, HUD crates, Tauri app, or
state-store permission behavior as a supported runtime path.

## 3. Portable Windows Crate Set

The Windows baseline checks only crates that should remain independent of local
daemon transport and desktop shell assumptions:

- `cadis-protocol`
- `cadis-policy`
- `cadis-store`
- `cadis-models`
- `cadis-avatar`

The Windows baseline intentionally excludes:

- `cadis-core`, while runtime shell/git/workspace behavior is still
  Linux-first.
- `cadis-daemon`, until non-Unix local transport exists.
- `cadis-cli`, until the client can use a Windows transport adapter.
- `crates/cadis-hud`, because the legacy native HUD uses Unix socket client
  code.
- `apps/cadis-hud`, until the Tauri shell has Windows transport, audio, and
  packaging adapters.

## 4. Workflow Commands

The platform workflow lives at
`.github/workflows/platform-baseline.yml`. It does not require secrets, model
provider credentials, microphone access, camera access, signing keys, or release
tokens.

macOS source baseline:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p cadis-protocol -p cadis-policy -p cadis-store -p cadis-models -p cadis-avatar -p cadis-core --all-targets --all-features
```

Windows portable crate baseline:

```bash
cargo check -p cadis-protocol -p cadis-policy -p cadis-store -p cadis-models -p cadis-avatar --all-targets --all-features
cargo clippy -p cadis-protocol -p cadis-policy -p cadis-store -p cadis-models -p cadis-avatar --all-targets --all-features -- -D warnings
cargo test -p cadis-protocol -p cadis-policy -p cadis-models -p cadis-avatar --all-targets --all-features
```

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
