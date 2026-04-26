# CADIS HUD

Tauri + React desktop HUD for the CADIS daemon.

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
