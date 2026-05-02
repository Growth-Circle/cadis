# Known Limitations

This document lists known limitations of C.A.D.I.S. v1.2.x.

## Platform

- **Windows and macOS support is new and less tested than Linux.** The daemon,
  CLI, and HUD build and pass CI on all three platforms, but Linux remains the
  primary development and runtime target. Edge cases on Windows and macOS may
  exist.
- **aarch64 Linux not natively tested.** aarch64 Linux binaries are
  cross-compiled but not natively tested in CI.
- **HUD macOS bundle fails on icon format.** The Tauri macOS `.dmg` bundle
  requires icon assets that are not yet generated. HUD works via
  `pnpm tauri:dev` on macOS.

## Networking

- **No remote relay.** The daemon communicates via local Unix socket (Linux /
  macOS) or local TCP (`127.0.0.1:7433` on Windows or via `--tcp-port`). There
  is no remote daemon access, multi-machine relay, or cloud orchestration.

## Voice

- **No production voice.** Edge TTS runs as a subprocess bridge through the
  HUD. Daemon-owned TTS provider execution is not implemented. Whisper
  transcription depends on a local `whisper-cli` binary.
- **TTS providers other than Edge TTS use stub implementations.** Only Edge TTS
  produces real audio output; other provider backends are stubs.

## Clients

- **Telegram adapter has DaemonBridge but is not production-tested.** The
  adapter can poll Telegram, parse commands, and bridge to `cadisd` via
  `DaemonBridge`, but the integration is early and not yet production-hardened.
- **No mobile client.** Android and iOS surfaces are future work.

## Runtime

- **Sequential tool calls per session.** Workers run concurrently via queue
  scheduling, but individual tool calls within a session execute sequentially.
  Async cancellation is not implemented yet; no cancellation propagation to
  running subprocesses.
- **cadis-core lib.rs partial extraction done.** Major modules have been
  extracted into separate files, but some subsystems still carry significant
  inline logic. Further decomposition is ongoing.
- **Worker artifact view in HUD is read-only.** Apply/discard actions route
  through daemon approval but the full patch-apply flow needs more work.
- **`file.patch` concurrent-edit detection is basic.** Structured file patching
  uses atomic temp-file writes with rollback, but concurrent-edit conflict
  detection remains simple and may not cover all race conditions.

## Configuration

- **Default `max_steps_per_session=8`.** Increase further for complex
  multi-step agent workflows that need more than 8 tool-call rounds.
