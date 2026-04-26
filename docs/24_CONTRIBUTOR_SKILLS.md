# Contributor Skills

## Purpose

This document explains which skills CADIS contributors should use and why.

CADIS needs two types of skills:

- global installed skills for common workflows
- project-local skills for CADIS-specific architecture and product rules

## Installed Global Skills

The following curated skills were installed into `~/.codex/skills`:

| Skill | Why CADIS needs it |
| --- | --- |
| `cli-creator` | Build and review the `cadis` CLI experience. |
| `doc` | Maintain product, technical, and contributor docs. |
| `playwright` | Validate HUD behavior and screenshot flows. |
| `screenshot` | Inspect UI output during visual parity checks. |
| `security-best-practices` | Review secure defaults for tools, shell, storage, and config. |
| `security-threat-model` | Maintain and expand the CADIS threat model. |
| `security-ownership-map` | Assign security-sensitive ownership areas. |
| `speech` | Work on TTS and voice output behavior. |
| `transcribe` | Work on STT, wake word, and transcription workflows. |
| `gh-fix-ci` | Diagnose GitHub Actions failures after CI exists. |
| `gh-address-comments` | Address review comments on GitHub PRs. |

OpenAI docs are available as a preinstalled system skill and should be used for OpenAI provider work.

## Project-Local Skills

Project-local skills live in `skills/`.

| Skill | Use when |
| --- | --- |
| `cadis-rust-core` | Implementing daemon, CLI, store, sessions, or crate boundaries. |
| `cadis-protocol` | Changing requests, events, content routing, or UI protocol. |
| `cadis-policy-security` | Working on approval, risk classes, sandboxing, redaction, or threat model. |
| `cadis-tool-runtime` | Implementing file, shell, git, patch, worktree, or other native tools. |
| `cadis-model-provider` | Adding or reviewing model providers and streaming behavior. |
| `cadis-ramaclaw-ui` | Working on HUD, config window, rename, themes, voice/model settings, or visual parity. |
| `cadis-voice` | Working on TTS, STT, wake word, auto-speak, and speech policy. |
| `cadis-open-source` | Working on docs, release, CI, issue templates, changelog, or governance. |

## Recommended Skill Combos

| Task | Skills |
| --- | --- |
| Build `cadis` CLI | `cadis-rust-core`, `cli-creator` |
| Add protocol event | `cadis-protocol` |
| Add shell tool | `cadis-tool-runtime`, `cadis-policy-security`, `security-best-practices` |
| Add OpenAI provider | `cadis-model-provider`, `openai-docs` |
| Port RamaClaw HUD | `cadis-ramaclaw-ui`, `playwright`, `screenshot` |
| Add voice preview | `cadis-voice`, `speech` |
| Add STT | `cadis-voice`, `transcribe` |
| Update threat model | `cadis-policy-security`, `security-threat-model` |
| Fix CI | `gh-fix-ci` |
| Prepare public release | `cadis-open-source`, `doc`, `security-ownership-map` |

## Contributor Rule

For any non-trivial task, contributors should read `AGENT.md`, then the one project-local skill that matches the task. Avoid loading every skill at once.

