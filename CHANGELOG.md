# Changelog

All notable changes to CADIS will be documented in this file.

The format follows Keep a Changelog style, and the project will use Semantic Versioning once the first release exists.

## Unreleased

### Added

- Initial planning baseline.
- Product, business, functional, technical, architecture, roadmap, and open-source governance documents.
- Open-source repository standard files.
- Desktop MVP runtime with `cadisd`, `cadis`, Unix socket NDJSON frames, status/chat/doctor commands, optional Ollama model adapter, local fallback responses, JSONL event logs, and redaction.
- Native `cadis-hud` prototype with orbital HUD shell, status bar, chat command panel, config tabs, theme controls, model controls, voice preview hooks, rename dialog, and approval stack UI.
- Example desktop MVP config at `config/cadis.example.toml`.

### Security

- Expanded `.gitignore` coverage for local state, secrets, diagnostic files, crash dumps, and credential exports.
- Added credential redaction before persisted event logs.
