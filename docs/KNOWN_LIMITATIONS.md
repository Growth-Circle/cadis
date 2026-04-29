# Known Limitations

This document lists known limitations of the current C.A.D.I.S. v0.9.2 beta.

## Platform

- **Windows and macOS support is new and less tested than Linux.** The daemon,
  CLI, and HUD build and pass CI on all three platforms, but Linux remains the
  primary development and runtime target. Edge cases on Windows and macOS may
  exist.
- **aarch64 Linux not natively tested.** aarch64 Linux binaries are
  cross-compiled but not natively tested in CI.

## Networking

- **No remote relay.** All communication is local Unix socket. There is no
  remote daemon access, multi-machine relay, or cloud orchestration.

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
- **No Windows or macOS installer.** Linux users can use the AppImage or .deb
  from GitHub Releases.
- **cadis-core lib.rs partial extraction done.** Major modules have been
  extracted into separate files, but some subsystems still carry significant
  inline logic. Further decomposition is ongoing.
- **Worker artifact view in HUD is read-only.** Apply/discard actions route
  through daemon approval but the full patch-apply flow needs more work.

## Configuration

- **Default `max_steps_per_session=8`.** Increase further for complex
  multi-step agent workflows that need more than 8 tool-call rounds.
