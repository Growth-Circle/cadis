# cadis-avatar

CADIS-native avatar state engine and renderer contract.

This crate is the Rust boundary for the Wulan avatar work. It does not depend on
the HUD, Tauri, Three.js, wgpu, Bevy, camera APIs, or model providers. Instead it
normalizes daemon/HUD state into renderable avatar frames:

- mode mapping: idle, listening, thinking, speaking, coding, approval, error
- body gesture state with priority, interruption, elapsed time, and reduced-motion flags
- face expression state
- optional local-only face tracking input
- direct wgpu-first renderer contract data without linking the `wgpu` crate
- renderer-failure fallback state for the default CADIS orb or a static Wulan texture
- deferred Bevy contract metadata behind the future `bevy-renderer` feature

`AvatarRendererContract` now includes an `AvatarFallbackContract` so native
renderer adapters can turn render failures into serializable HUD fallback state
without blocking HUD launch. The default fallback target is the CADIS orb.

The current `apps/cadis-hud` Wulan Arc scene remains the visual prototype. This
crate is the migration target for a future CADIS-native avatar renderer.
