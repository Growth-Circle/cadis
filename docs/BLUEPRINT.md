# CADIS Blueprint

## Purpose

This blueprint normalizes the original C.A.D.I.S. idea into the new `cadis` project baseline.

The key change is strategic: CADIS starts as a clean Rust-first local runtime instead of using OpenClaw as the backend or core.

## Name

- Package and command name: `cadis`
- Daemon name: `cadisd`
- Product display name: CADIS
- Full name: Coordinated Agentic Distributed Intelligence System

## Positioning

CADIS is a Rust-first, local-first, model-agnostic multi-agent runtime. It coordinates agents, subagents, tools, approvals, voice output, Telegram control, desktop HUD, and isolated code work windows.

## System Shape

```text
One daemon: cadisd
Many interfaces: CLI, HUD, Telegram, voice, Android later
Many agents: main, coder, reviewer, tester, explorer, researcher, operator
Many models: OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio, custom HTTP
Many tools: files, shell, git, patch, browser later, integrations later
One policy engine: approvals, sandbox rules, risk classes
```

## Product Principles

1. Runtime first, UI second.
2. Native Rust core, optional compatibility layers.
3. Fast status streaming before final answers.
4. Central approvals for risky actions.
5. Tool execution must be observable and cancellable.
6. Code-heavy work belongs in code windows, not voice or chat walls.
7. Telegram is a control surface, not a separate agent runtime.
8. Model providers are replaceable.
9. Parallel coding agents use worktree isolation.
10. Open-source quality starts at planning.

## Initial Architecture

```text
cadis CLI
Telegram Adapter
HUD
Voice Adapter
Android Remote Later
      |
      v
Local Protocol
      |
      v
cadisd daemon
      |
      +-- Event Bus
      +-- Session Store
      +-- Approval and Policy Engine
      +-- Agent Orchestrator
      +-- Model Provider Layer
      +-- Native Tool Runtime
      +-- Persistence and Logs
```

## First Build Target

The first target is a Linux desktop runtime with CLI control. Telegram, voice, and HUD follow only after the daemon and event protocol are stable.

## Core Implementation Order

```text
repository baseline
protocol crate
event crate/types
daemon skeleton
CLI client
local transport
session lifecycle
model stream abstraction
one provider
tool registry
policy engine
approval flow
persistence
agent session abstraction
worktree worker isolation
Telegram adapter
voice output
HUD
code work window
multi-agent tree
```

## Technical Defaults

| Area | Default |
| --- | --- |
| Language | Rust |
| Async runtime | Tokio unless changed by ADR |
| Initial transport | Unix socket or stdio test mode |
| Config | TOML |
| Logs | JSONL |
| UI | Dioxus Desktop later |
| Telegram | `teloxide` candidate |
| TTS | Rust provider trait; Edge TTS provider candidate |
| License | Apache-2.0 baseline |

## Guardrails

- Do not build full HUD before daemon and protocol.
- Do not import Codex or other source code without license review.
- Do not make Node.js a core runtime dependency.
- Do not let Telegram, HUD, or voice execute tools directly.
- Do not allow tool execution without risk classification.
- Do not log raw secrets.
- Do not allow recursive agent fan-out by default.

## v0.1 Definition

CADIS v0.1 is successful when:

- `cadisd` starts.
- `cadis chat "hello"` streams events.
- one model provider works.
- file and shell tools exist.
- policy can require approval.
- CLI can approve or deny.
- session logs persist.
- redaction tests pass.

