# CADIS HUD

Tauri + React desktop HUD for the CADIS daemon. It provides the orbital agent
view, chat routing, voice controls, model settings, approvals, and live worker
state as a protocol client. Runtime authority stays in `cadisd`.

## Avatar

The center HUD avatar is configurable from Settings -> Appearance:

- `CADIS Orb`: the default RamaClaw-style orb.
- `Wulan Arc`: a contributed hologram avatar adapted from
  `wulan-contribute/cadis-arc-avatar-sample`.

The selected avatar is persisted through `cadisd` as `hud.avatar_style`. The
Wulan Arc path lazy-loads its Three.js scene so the default HUD path stays
lightweight. The Wulan Arc scene keeps the portrait readable inside the HUD and
adds lightweight eye blink/gaze and mouth pulse overlays for live-state feedback.

## Run

```bash
pnpm install
pnpm tauri:dev
```

The HUD talks to `cadisd` through the `cadis_request` Tauri command and the
CADIS protocol. Socket discovery order:

1. explicit `socketPath` argument from the renderer
2. `CADIS_HUD_SOCKET`
3. `CADIS_SOCKET`
4. `~/.cadis/config.toml` `socket_path`
5. `$XDG_RUNTIME_DIR/cadis/cadisd.sock`
6. `~/.cadis/run/cadisd.sock`

For browser-only preview:

```bash
pnpm dev
```

Browser preview cannot call Tauri commands, so it renders the shell and remains
disconnected until launched with Tauri.

## Build

```bash
pnpm build
pnpm tauri:build
```

No credentials are stored in this app. ChatGPT Plus/Pro access is delegated to
the official Codex CLI login used by `cadisd` when the model provider is
`codex-cli`. OpenAI API access uses environment variables handled by the daemon.

## Voice Input

The mic button records locally in the webview and sends WAV audio to the Tauri
side for `whisper-cli` transcription. Configure these paths if needed:

```bash
export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
```

On Linux, the HUD installs a WebKitGTK permission handler for audio capture.
If the OS portal still blocks the mic, allow microphone access for CADIS in the
system prompt/settings and click the mic again.
