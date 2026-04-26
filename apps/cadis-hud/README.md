# CADIS HUD

Tauri + React desktop HUD for the CADIS daemon. It provides the orbital agent
view, chat routing, voice controls, model settings, approvals, and live worker
state as a protocol client. Runtime authority stays in `cadisd`.

![CADIS desktop HUD](../../docs/assets/readme/cadis-hud-desktop.png)

## Avatar

The center HUD avatar is configurable from Settings -> Appearance:

- `CADIS Orb`: the default RamaClaw-style orb.
- `Wulan Arc`: a contributed hologram avatar adapted from
  `wulan-contribute/cadis-arc-avatar-sample`.

The selected avatar is persisted through `cadisd` as `hud.avatar_style`. The
Wulan Arc path lazy-loads its Three.js scene so the default HUD path stays
lightweight. The Wulan Arc scene keeps the portrait readable inside the HUD and
adds lightweight eye blink/gaze and mouth pulse overlays for live-state feedback.

The long-term Wulan direction is a CADIS-native avatar engine documented in
`../../docs/26_WULAN_AVATAR_ENGINE.md`. The current Three.js scene is the
migration prototype. The preferred native path is a focused Rust/wgpu renderer;
Bevy is deferred unless CADIS later accepts a broader 3D scene-engine decision.
Optional face tracking must remain off by default, permission-gated, local-only,
and non-persistent.

## Run

```bash
corepack enable
pnpm install
pnpm tauri:dev
```

The HUD talks to `cadisd` through the CADIS protocol. On startup the renderer
calls the native `cadis_events_subscribe` Tauri command, which opens an
`events.subscribe` Unix-socket connection, keeps it open for live daemon
events, and forwards each newline-delimited protocol frame to the renderer as a
Tauri event. One-shot requests such as `message.send`, `models.list`, and
`ui.preferences.set` still use `cadis_request`.

Socket discovery order for both commands:

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

CI runs the same frontend hygiene checks from this directory:

```bash
pnpm lint
pnpm typecheck
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
```

No credentials are stored in this app. ChatGPT Plus/Pro access is delegated to
the official Codex CLI login used by `cadisd` when the model provider is
`codex-cli`. OpenAI API access uses environment variables handled by the daemon.
Use `.env.example` for local placeholders only; do not commit real `.env`,
Codex auth, provider keys, or generated Tauri bundles.

## Voice Input

The mic button records locally in the webview and sends WAV audio to the Tauri
side for `whisper-cli` transcription. Configure these paths if needed:

```bash
export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
export CADIS_WHISPER_LANGUAGE="id"
```

On Linux, the HUD installs a WebKitGTK permission handler for audio capture.
If the OS portal still blocks the mic, allow microphone access for CADIS in the
system prompt/settings and click the mic again.
