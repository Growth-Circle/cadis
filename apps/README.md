# Apps

Application entrypoints live here when they are not Rust workspace crates.

Current apps:

- `cadis-hud`: Tauri + React desktop HUD adapted from the RamaClaw visual system,
  wired to `cadisd` through the CADIS protocol.

Related Rust workspace prototype:

- `crates/cadis-hud`: native egui HUD prototype retained for Rust-first runtime experiments.

Planned apps:

- `cadis-mobile-android`: Android remote controller later.
- `cadis-server`: optional future remote relay or managed service, not part of v0.1 core.
