# Glossary

## Agent

An AI-controlled worker that can reason, call tools, produce events, and return results.

## Agent Home

The profile-scoped directory for one persistent agent's identity, instructions,
memory, skills, and policy. Example:
`~/.cadis/profiles/default/agents/rama/`. Agent home is not the project cwd.

## Agent Tree

A parent-child structure of agents and subagents. CADIS limits depth and fan-out to avoid uncontrolled resource usage.

## Approval

A user decision required before executing a risky action.

## CADIS

The product display name for Coordinated Agentic Distributed Intelligence System.

## `cadis`

The command name, package name, and repository name.

## `cadisd`

The local daemon that owns sessions, tools, policy, agents, and persistence.

## Code Work Window

A dedicated visual window for code-heavy output such as diffs, terminal logs, tests, and patch approval.

## Content Kind

Metadata that tells clients how to route output. Examples: chat, summary, code, diff, terminal log, test result, approval, error.

## Event Bus

The internal daemon mechanism that broadcasts structured events to clients, logs, and adapters.

## HUD

Desktop control surface for chat, status, agents, approvals, voice controls, and worker progress.

## Local-First

The core runtime, state, orchestration, and logs live on the user's machine.

## Profile Home

The root directory for one CADIS identity/environment, including profile config,
agents, memory, skills, sessions, channels, workers, artifacts, event logs, and
redacted logs. Example: `~/.cadis/profiles/default/`. A profile home is a state
boundary, not a filesystem sandbox.

## Project Workspace

A registered user project root that tools may read, write, or execute within
only after the daemon resolves a workspace grant. Example:
`~/Project/chatbot-ai-saas/`.

## Model Provider

An adapter that sends requests to an AI model backend and streams model events back to CADIS.

## Policy Engine

The central component that decides whether an action is allowed, denied, or requires approval.

## Risk Class

A label that describes the risk of a tool call or action, such as safe-read, workspace-edit, secret-access, or sudo-system.

## Session

A user-visible interaction or task tracked by the daemon.

## Tool Runtime

The subsystem that validates, executes, cancels, logs, and reports native CADIS tools.

## Worker Worktree

A git worktree created for one coding worker/task, usually under
`<project>/.cadis/worktrees/<worker-id>/`, so parallel workers do not edit the
same checkout.

## Worktree

A separate git working tree used to isolate coding workers from the main repository checkout.

## Workspace Grant

A daemon-owned runtime authorization that binds a profile, agent, project
workspace, root path, access level, expiry, and source. File, shell, git, and
worker tools must fail closed or request approval when no matching grant exists.
