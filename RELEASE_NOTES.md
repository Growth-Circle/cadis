# C.A.D.I.S. v1.2.2 — Polished HUD, draggable agents, output filtering

**Release date:** 2026-04-30
**Maturity:** beta

## Highlights

v1.2.2 delivers a fully refactored native HUD with glassmorphism visuals,
draggable agent cards, an improved chat experience, and a debug mode. The
output filter pipeline now includes semantic-boundary truncation and a trigram
search index for faster file search on large projects. Windows CI stability
is improved and npm publishing is fixed.

## What's New

### HUD — Full Refactor
- hud: Split monolithic main.rs into 5 modules — connection, theme, types, widgets, app ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7) by [@RamaAditya49](https://github.com/RamaAditya49))
- hud: Glassmorphism panels with semi-transparent fill, border glow, and rounded corners ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- hud: Animated pulsing orb with glow effect and status-based color ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- hud: Adaptive framerate — 60fps when animating, 10fps when idle ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- hud: Draggable agent cards — reposition agents freely in the orbital view ([`de55b8c`](https://github.com/Growth-Circle/cadis/commit/de55b8c))
- hud: Hide/show agents — close button (×) on each card, agent tray to restore ([`de55b8c`](https://github.com/Growth-Circle/cadis/commit/de55b8c))
- hud: Chat timestamps, agent name badges, code block rendering, typing indicator ([`de55b8c`](https://github.com/Growth-Circle/cadis/commit/de55b8c))
- hud: Debug mode tab in Settings — FPS, event log, connection status, agent details ([`de55b8c`](https://github.com/Growth-Circle/cadis/commit/de55b8c))
- hud: Event subscription with replay for real-time updates ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- hud: @mention targeting in chat input ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))

### Output Filtering
- filter: Semantic-boundary truncation — break at headings, code fences, and function boundaries instead of raw byte limits, inspired by [QMD](https://github.com/tobi/qmd) ([`fe71a26`](https://github.com/Growth-Circle/cadis/commit/fe71a26))
- filter: Trigram search index for fast file.search on large workspaces (>100 files), inspired by [QMD](https://github.com/tobi/qmd) ([`fe71a26`](https://github.com/Growth-Circle/cadis/commit/fe71a26))

## Improvements

- hud: Chat history capped at 500 messages to prevent memory growth ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- hud: Graceful shutdown sends DaemonShutdown when HUD started the daemon ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- build: Release profile with opt-level=z, LTO, strip, codegen-units=1 for smaller binaries ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))

## Bug Fixes

- hud: Gate `PathBuf` import with `#[cfg(unix)]` for Windows compilation ([`adf1e84`](https://github.com/Growth-Circle/cadis/commit/adf1e84))
- ci: Limit Windows tests to portable crates per CI/CD standard ([`dcbd674`](https://github.com/Growth-Circle/cadis/commit/dcbd674))
- ci: Fix npm auto-publish — restore direct publish job in release workflow ([`a7ca5c7`](https://github.com/Growth-Circle/cadis/commit/a7ca5c7))
- ci: Fix gitleaks version resolution (fetch latest dynamically) ([`4ef43d9`](https://github.com/Growth-Circle/cadis/commit/4ef43d9))

## Documentation

- Add PR Review Workflow standard (`docs/standards/22_PR_REVIEW_WORKFLOW_STANDARD.md`) ([`13e79bd`](https://github.com/Growth-Circle/cadis/commit/13e79bd))
- Add Release Notes standard (`docs/standards/21_RELEASE_NOTES_STANDARD.md`) ([`23824b5`](https://github.com/Growth-Circle/cadis/commit/23824b5))
- Update architecture doc with output filter pipeline and search index sections ([`fe71a26`](https://github.com/Growth-Circle/cadis/commit/fe71a26))

## Contributors

- [@RamaAditya49](https://github.com/RamaAditya49) — HUD refactor, output filtering, CI fixes, standards
- [@DeryFerd](https://github.com/DeryFerd) — PRs #41, #43, #44, #45 applied in v1.2.1

## Installation

```bash
npm install -g @growthcircle/cadis
cadis
```

## Full Changelog

[v1.2.1...v1.2.2](https://github.com/Growth-Circle/cadis/compare/v1.2.1...v1.2.2)
