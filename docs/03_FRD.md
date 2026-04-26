# Functional Requirements Document

## 1. Scope

This document defines functional requirements for CADIS from planning baseline through the first desktop alpha. Requirements are grouped by subsystem and use stable IDs for backlog tracking.

## 2. Daemon

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-DAEMON-001 | Provide a local daemon binary named `cadisd`. | P0 |
| FRD-DAEMON-002 | Start, stop, and report health status. | P0 |
| FRD-DAEMON-003 | Accept local client connections. | P0 |
| FRD-DAEMON-004 | Create sessions for user requests. | P0 |
| FRD-DAEMON-005 | Emit lifecycle events for sessions. | P0 |
| FRD-DAEMON-006 | Route events to subscribed clients. | P0 |
| FRD-DAEMON-007 | Maintain an in-memory registry of active sessions and agents. | P1 |
| FRD-DAEMON-008 | Recover incomplete sessions from persisted metadata when possible. | P2 |
| FRD-DAEMON-009 | Support graceful shutdown with event flush. | P1 |

## 3. Local Protocol

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-PROTO-001 | Define typed request, response, and event structures. | P0 |
| FRD-PROTO-002 | Support streaming message deltas. | P0 |
| FRD-PROTO-003 | Support session subscription. | P0 |
| FRD-PROTO-004 | Support approval request and resolution messages. | P0 |
| FRD-PROTO-005 | Support agent status events. | P1 |
| FRD-PROTO-006 | Support tool lifecycle events. | P0 |
| FRD-PROTO-007 | Version the protocol. | P0 |
| FRD-PROTO-008 | Reject unknown incompatible protocol versions. | P1 |
| FRD-PROTO-009 | Provide JSON serialization for debugging. | P0 |

## 4. CLI

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-CLI-001 | Provide a user binary named `cadis`. | P0 |
| FRD-CLI-002 | `cadis daemon` starts the daemon. | P0 |
| FRD-CLI-003 | `cadis chat <message>` sends a chat request. | P0 |
| FRD-CLI-004 | `cadis run --cwd <path> <task>` starts a task in a workspace. | P1 |
| FRD-CLI-005 | `cadis status` shows daemon status. | P0 |
| FRD-CLI-006 | `cadis approve <id>` resolves approval as approved. | P0 |
| FRD-CLI-007 | `cadis deny <id>` resolves approval as denied. | P0 |
| FRD-CLI-008 | `cadis agent list` shows active agents. | P1 |
| FRD-CLI-009 | `cadis worker tail <id>` streams worker events. | P2 |
| FRD-CLI-010 | `cadis doctor` checks config, providers, transport, and storage. | P1 |

## 5. Model Providers

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-MODEL-001 | Define a provider trait for streaming model events. | P0 |
| FRD-MODEL-002 | Expose provider capability metadata. | P0 |
| FRD-MODEL-003 | Support provider configuration from local config. | P0 |
| FRD-MODEL-004 | Support OpenAI provider. | P1 |
| FRD-MODEL-005 | Support Ollama provider. | P1 |
| FRD-MODEL-006 | Support Anthropic provider. | P2 |
| FRD-MODEL-007 | Support Gemini provider. | P2 |
| FRD-MODEL-008 | Support OpenRouter provider. | P2 |
| FRD-MODEL-009 | Support custom HTTP provider. | P2 |
| FRD-MODEL-010 | Provide conformance tests for streaming and errors. | P1 |

## 6. Tool Runtime

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-TOOL-001 | Define a native tool registry. | P0 |
| FRD-TOOL-002 | Tools must declare name, schema, risk class, and workspace constraints. | P0 |
| FRD-TOOL-003 | Implement `file.read`. | P0 |
| FRD-TOOL-004 | Implement `file.search`. | P0 |
| FRD-TOOL-005 | Implement `file.patch`. | P1 |
| FRD-TOOL-006 | Implement `shell.run`. | P0 |
| FRD-TOOL-007 | Implement `git.status`. | P1 |
| FRD-TOOL-008 | Implement `git.diff`. | P1 |
| FRD-TOOL-009 | Implement `git.worktree.create`. | P2 |
| FRD-TOOL-010 | Implement `git.worktree.remove`. | P2 |
| FRD-TOOL-011 | Tool results must emit lifecycle events. | P0 |
| FRD-TOOL-012 | Tool failures must include actionable error metadata. | P0 |

## 7. Approval and Policy

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-POLICY-001 | Provide a central policy engine. | P0 |
| FRD-POLICY-002 | Classify actions by risk class. | P0 |
| FRD-POLICY-003 | Auto-allow safe reads by default. | P0 |
| FRD-POLICY-004 | Require approval for outside-workspace writes. | P0 |
| FRD-POLICY-005 | Require approval for secret access. | P0 |
| FRD-POLICY-006 | Require approval for dangerous deletes. | P0 |
| FRD-POLICY-007 | Require approval for sudo/system changes. | P0 |
| FRD-POLICY-008 | Require approval for git push to protected branches. | P1 |
| FRD-POLICY-009 | Support first-response-wins approval resolution. | P0 |
| FRD-POLICY-010 | Persist approval request and resolution. | P1 |

## 8. Agent Runtime

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-AGENT-001 | Define `AgentSession` abstraction. | P0 |
| FRD-AGENT-002 | Stream agent lifecycle events. | P0 |
| FRD-AGENT-003 | Support main agent role. | P0 |
| FRD-AGENT-004 | Support coding agent role. | P1 |
| FRD-AGENT-005 | Support reviewer agent role. | P2 |
| FRD-AGENT-006 | Support tester agent role. | P2 |
| FRD-AGENT-007 | Enforce max depth. | P1 |
| FRD-AGENT-008 | Enforce max children per agent. | P1 |
| FRD-AGENT-009 | Enforce per-agent budget. | P1 |
| FRD-AGENT-010 | Prevent workers from blocking the main agent event loop. | P0 |

## 9. Persistence

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-STORE-001 | Use `~/.cadis` as default local home. | P0 |
| FRD-STORE-002 | Persist session logs as JSONL. | P0 |
| FRD-STORE-003 | Persist worker logs separately. | P1 |
| FRD-STORE-004 | Redact secrets before writing logs. | P0 |
| FRD-STORE-005 | Use atomic writes for state files. | P0 |
| FRD-STORE-006 | Store config in TOML. | P0 |
| FRD-STORE-007 | Never store raw provider keys in event logs. | P0 |

## 10. Telegram Adapter

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-TG-001 | Provide optional Telegram adapter crate. | P2 |
| FRD-TG-002 | Support `/status`. | P2 |
| FRD-TG-003 | Support `/agents`. | P2 |
| FRD-TG-004 | Support `/workers`. | P2 |
| FRD-TG-005 | Support `/approve <id>`. | P2 |
| FRD-TG-006 | Support `/deny <id>`. | P2 |
| FRD-TG-007 | Support `/spawn <task>`. | P2 |
| FRD-TG-008 | Push approval messages with buttons when supported. | P2 |
| FRD-TG-009 | Telegram adapter must not contain core agent logic. | P0 |

## 11. Voice

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-VOICE-001 | Define TTS provider trait. | P2 |
| FRD-VOICE-002 | Support voice on/off state. | P2 |
| FRD-VOICE-003 | Speak normal short answers. | P2 |
| FRD-VOICE-004 | Summarize long answers before speaking. | P2 |
| FRD-VOICE-005 | Do not speak long code, diffs, or logs. | P2 |
| FRD-VOICE-006 | Speak approval risk summaries. | P2 |

## 12. HUD and Code Work Window

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-HUD-001 | Provide Linux HUD after daemon and CLI stabilize. | P3 |
| FRD-HUD-002 | Show chat stream. | P3 |
| FRD-HUD-003 | Show agent tree. | P3 |
| FRD-HUD-004 | Show approval cards. | P3 |
| FRD-HUD-005 | Show worker progress. | P3 |
| FRD-HUD-006 | Control voice mode. | P3 |
| FRD-CODE-001 | Provide separate code work window. | P3 |
| FRD-CODE-002 | Show diffs. | P3 |
| FRD-CODE-003 | Show terminal logs. | P3 |
| FRD-CODE-004 | Show test results. | P3 |
| FRD-CODE-005 | Support apply/discard patch. | P3 |

## 13. RamaClaw UI Adaptation

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-UI-001 | CADIS HUD must adapt the RamaClaw orbital HUD as the canonical desktop UI reference. | P3 |
| FRD-UI-002 | HUD must include a unified config dialog with Voice, Models, Appearance, and Window tabs. | P3 |
| FRD-UI-003 | HUD must support agent display-name rename for main and subagents. | P3 |
| FRD-UI-004 | Agent rename must persist through daemon state, not browser-only storage. | P3 |
| FRD-UI-005 | HUD must support six hue-based themes: arc, amber, phosphor, violet, alert, ice. | P3 |
| FRD-UI-006 | HUD must support background opacity setting. | P3 |
| FRD-UI-007 | HUD must support curated bilingual voice selection. | P3 |
| FRD-UI-008 | HUD must support voice rate, pitch, volume, auto-speak, test, and stop controls. | P3 |
| FRD-UI-009 | HUD must support per-agent model selection. | P3 |
| FRD-UI-010 | HUD must show agent cards with status, role, task, detail/model, and nested workers. | P3 |
| FRD-UI-011 | HUD must keep approval cards visible until daemon emits approval resolution. | P3 |
| FRD-UI-012 | HUD must replace all OpenClaw paths, labels, and assumptions with CADIS equivalents. | P3 |

## 14. Configuration

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-CONFIG-001 | Load config from `~/.cadis/config.toml`. | P0 |
| FRD-CONFIG-002 | Support provider config. | P0 |
| FRD-CONFIG-003 | Support policy config. | P1 |
| FRD-CONFIG-004 | Support agent limits. | P1 |
| FRD-CONFIG-005 | Support environment variable overrides. | P0 |

## 15. Observability

| ID | Requirement | Priority |
| --- | --- | --- |
| FRD-OBS-001 | Emit structured logs. | P0 |
| FRD-OBS-002 | Emit traceable event IDs. | P0 |
| FRD-OBS-003 | Include session ID in all session events. | P0 |
| FRD-OBS-004 | Include tool call ID in all tool lifecycle events. | P0 |
| FRD-OBS-005 | Provide debug mode for local protocol events. | P1 |
