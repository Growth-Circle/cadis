# CADIS Project Charter

## Document Control

- Project: CADIS
- Full name: Coordinated Agentic Distributed Intelligence System
- Repository name: cadis
- Status: Planning baseline
- Version: 0.1.0-planning
- Date: 2026-04-26
- Primary target: Linux desktop
- Later targets: Windows, macOS, Android remote client. Current
  macOS/Windows validation status is defined in `docs/28_PLATFORM_BASELINE.md`.

## Executive Summary

CADIS is a Rust-first, local-first, model-agnostic runtime for coordinating many AI agents, subagents, native tools, approvals, user interfaces, and voice output through one local daemon.

The project starts from a clean monorepo because the previous OpenClaw-backed approach is considered too slow and too coupled for the desired core runtime. CADIS should feel immediate even when model inference is slow. The daemon must stream early events, keep tool dispatch lightweight, and isolate coding work from normal chat.

## Mission

Build a fast open-source AI command system that can safely coordinate real work across code, files, shell commands, models, voice, Telegram, and desktop UI while keeping control local to the user.

## Product Statement

CADIS is a local multi-agent operating layer:

```text
One local daemon.
Many agents.
Many interfaces.
Any model.
Any tool.
Safe approvals.
Fast streaming.
```

## Strategic Reset

The new direction replaces the earlier "OpenClaw as backend/core" assumption.

### Old Direction

- UI or assistant shell backed by OpenClaw.
- Core behavior tied to an existing backend.
- Risk of slow orchestration, slow event relay, and difficult deep changes.

### New Direction

- Fresh Rust monorepo.
- `cadisd` daemon as the core.
- Interfaces are thin clients.
- Native Rust tool registry and policy engine.
- Codex-style coding features are isolated in a later compatibility or extraction layer.
- Open-source standards are part of the first commit, not an afterthought.

## Guiding Principles

1. Fast by default.
2. Rust-first in the core.
3. Local-first for state, orchestration, and logs.
4. Model-agnostic from the first model abstraction.
5. Interface-agnostic through one daemon protocol.
6. Safe by design through central policy and approval.
7. Code-heavy work belongs in visual code windows.
8. Long-running work must not block the main agent.
9. Public repository quality matters from day one.
10. Optional integrations must not become core dependencies.

## Goals

- Create a clean open-source project baseline.
- Define product, business, functional, and technical requirements.
- Define an implementation plan that can be executed from an empty repository.
- Build a Rust workspace architecture suitable for daemon, CLI, model providers, tools, agents, Telegram, voice, HUD, and code work windows.
- Keep the first release tightly scoped to Linux desktop.

## Non-Goals

- No OpenClaw dependency in the core runtime.
- No full SaaS backend in v0.1.
- No Android local coding runtime in v0.1.
- No plugin marketplace in v0.1.
- No complex memory graph in v0.1; see `25_MEMORY_CONCEPT.md` for Wulan's future memory architecture contribution.
- No IDE replacement in v0.1.
- No automatic cross-machine distributed execution in v0.1.
- No voice wake-word in v0.1.

## Stakeholders

| Stakeholder | Need |
| --- | --- |
| Primary user | Fast local AI assistant that can actually work across tools and code |
| Maintainer | Clean architecture, testable Rust crates, low dependency risk |
| Contributor | Clear docs, issue templates, roadmap, contribution rules |
| Security reviewer | Central policy, audit logs, approval guarantees |
| Power user | Telegram control, voice output, code work isolation |
| Future integrator | Stable protocol, model provider contract, tool registry |

## Success Criteria

### Planning Success

- PRD, BRD, FRD, TRD, architecture, implementation plan, roadmap, checklist, risks, and decisions exist.
- Open-source baseline files exist.
- The repository can be initialized and published without major cleanup.

### MVP Success

- `cadisd` starts locally.
- `cadis chat "hello"` streams daemon events.
- One model provider can stream output.
- Native file and shell tools work behind policy.
- Approval requests can be resolved through CLI.
- Event logs are persisted as JSONL.
- Tool execution and secret redaction have tests.

### Product Success

- Telegram can control CADIS remotely.
- HUD can show chat, agent status, approvals, and voice controls.
- Coding tasks open a code work window instead of flooding chat.
- Parallel coding agents use isolated git worktrees.
- Model provider can be swapped without changing UI or tools.

## Core Decisions

| Area | Decision |
| --- | --- |
| Repository | Fresh Rust monorepo |
| Project name | `cadis` |
| Product display name | CADIS |
| Core backend | `cadisd` daemon |
| CLI | `cadis` |
| Core language | Rust |
| First platform | Linux desktop |
| First UI | CLI, then Telegram, then Dioxus HUD |
| Voice | Rust TTS abstraction; online Edge TTS provider first if feasible |
| Agent isolation | Git worktrees for coding agents |
| Persistence | `~/.cadis` local directory |
| Logs | JSONL event logs with redaction |
| License | Apache-2.0 baseline |
| OpenClaw | Not part of core |
| Codex-derived capability | Later adapter or extracted crate after review |

## Delivery Philosophy

The runtime comes first. Interfaces should be added only after the daemon protocol, event stream, policy model, and tool execution path are clean.

The recommended order:

```text
protocol -> daemon -> CLI -> model stream -> tools -> approvals -> agent runtime
-> Telegram -> voice -> HUD -> code work window -> multi-agent tree
```

## Documentation Map

- `01_PRD.md`: what the product must do and for whom.
- `02_BRD.md`: why the project should exist and what business outcomes matter.
- `03_FRD.md`: detailed functional requirements.
- `04_TRD.md`: non-functional and engineering requirements.
- `05_ARCHITECTURE.md`: system structure and flows.
- `06_IMPLEMENTATION_PLAN.md`: phased build plan.
- `07_MASTER_CHECKLIST.md`: execution checklist.
- `08_ROADMAP.md`: release path.
- `09_OPEN_SOURCE_STANDARD.md`: community and publishing standards.
- `10_RISK_REGISTER.md`: risk tracking.
- `11_DECISIONS.md`: architecture decisions and open questions.
- `25_MEMORY_CONCEPT.md`: Wulan-contributed memory concept adapted as future work.
