# Getting Started

## Prerequisites

- **Rust** stable 1.75+ ([rustup](https://rustup.rs/))
- **Git**
- **Linux desktop** (primary supported platform)

For the HUD (optional):
- Node.js 20+, pnpm 10.x (`corepack enable`)
- Tauri system deps (Debian/Ubuntu):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
  ```

## Install from release

Download `cadisd` and `cadis` from [GitHub Releases](https://github.com/Growth-Circle/cadis/releases), then make them executable:

```bash
chmod +x cadisd cadis
sudo mv cadisd cadis /usr/local/bin/
```

## Install from source

```bash
git clone https://github.com/Growth-Circle/cadis.git
cd cadis
cargo build --release
```

Binaries are at `target/release/cadisd` and `target/release/cadis`.

## Start the daemon

```bash
cadisd --check   # verify config
cadisd           # start
```

Or from a source build:

```bash
target/release/cadisd --check
target/release/cadisd
```

## First CLI chat

In another terminal:

```bash
cadis status          # check daemon is running
cadis doctor          # verify environment
cadis models          # list available models
cadis chat "hello"    # send a message
```

The default provider mode is `auto`: C.A.D.I.S. tries Ollama at `http://127.0.0.1:11434` and falls back to a local credential-free response if Ollama is unavailable.

## Run the HUD

Start `cadisd` first, then:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm tauri:dev
```

For a custom socket:

```bash
cadisd --socket /tmp/cadis-hud.sock --dev-echo
CADIS_HUD_SOCKET=/tmp/cadis-hud.sock pnpm tauri:dev
```

The HUD connects to `cadisd` over a Unix socket. It resolves the socket from `CADIS_HUD_SOCKET`, `CADIS_SOCKET`, `~/.cadis/config.toml`, `$XDG_RUNTIME_DIR/cadis/cadisd.sock`, or `~/.cadis/run/cadisd.sock`.

## Next steps

- [[Configuration]] — Set up model providers, voice, and workspaces
- [[FAQ]] — Common questions
- [[Troubleshooting]] — If something isn't working
