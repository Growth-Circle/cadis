# Known Limitations

This document lists known limitations of the current C.A.D.I.S. v0.9 beta.

## Platform

- **No Windows runtime.** The daemon, CLI transport, shell adapter, HUD, and
  audio paths are Linux-only. Windows CI validates portable crates only.
- **macOS and Windows are source-validation only.** No runtime, HUD, or audio
  adapters exist for either platform.
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

- **Telegram adapter connects to Bot API but not yet to cadisd.** The adapter
  can poll Telegram and parse commands, but the bridge to daemon protocol is
  not wired yet.
- **No mobile client.** Android and iOS surfaces are future work.

## Runtime

- **Sequential tool calls per session.** Workers run concurrently via queue
  scheduling, but individual tool calls within a session execute sequentially.
  Async cancellation is not implemented yet; no cancellation propagation to
  running subprocesses.
- **No packaged installer.** All binaries are built from source.
- **No HUD packaging in release artifacts.** The Tauri HUD must be built from
  source.
- **cadis-core lib.rs is monolithic (~15K lines).** Module extraction is
  planned.
- **Worker artifact view in HUD is read-only.** Apply/discard actions route
  through daemon approval but the full patch-apply flow needs more work.

## Configuration

- **Default `max_steps_per_session=8`.** Increase further for complex
  multi-step agent workflows that need more than 8 tool-call rounds.
