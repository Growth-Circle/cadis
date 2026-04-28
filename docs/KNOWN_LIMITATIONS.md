# Known Limitations

This document lists known limitations of the current C.A.D.I.S. pre-alpha.

## Platform

- **No Windows runtime.** The daemon, CLI transport, shell adapter, HUD, and
  audio paths are Linux-only. Windows CI validates portable crates only.
- **macOS is source-validation only.** No packaged runtime or HUD support.

## Networking

- **No remote relay.** All communication is local Unix socket. There is no
  remote daemon access, multi-machine relay, or cloud orchestration.

## Voice

- **No production voice.** Edge TTS runs as a subprocess bridge through the
  HUD. Daemon-owned TTS provider execution is not implemented. Whisper
  transcription depends on a local `whisper-cli` binary.

## Clients

- **No Telegram client yet.** The Telegram adapter is planned but not
  implemented.
- **No mobile client.** Android and iOS surfaces are future work.

## Runtime

- **Single-threaded tool execution.** Tool calls execute sequentially within
  the daemon. Parallel tool execution and async cancellation are not
  implemented yet.
- **No packaged installer.** All binaries are built from source.
