# cadis-hud-legacy (deprecated)

> **This crate is a deprecated native eframe/egui HUD prototype.**
> The canonical CADIS HUD is the Tauri + React app at `apps/cadis-hud`.

This crate remains in the workspace for reference but is no longer the
recommended HUD surface. The Tauri HUD provides the full desktop experience
including voice I/O, orbital agent visualization, and cross-platform transport
(Unix socket + TCP).

## Migration

Build and run the canonical HUD instead:

```bash
cd apps/cadis-hud
corepack enable
pnpm install
pnpm tauri:dev     # development
pnpm tauri:build   # production bundle
```
