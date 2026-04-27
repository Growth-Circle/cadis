# Workspace Architecture

Status: Baseline accepted and partially implemented. CADIS now initializes
profile homes, creates daemon-known agent homes from templates, persists the
workspace registry and active grants under the default profile, exposes
`workspace list/register/grant/revoke/doctor` through the protocol and CLI, and
keeps tool execution behind daemon-resolved workspace grants. Real worker
worktree creation/cleanup, checkpoint rollback, workspace-local skill
enforcement, artifact production, and denied-path enforcement for mutating tools
remain future work.

## 1. Purpose

CADIS separates durable identity, runtime state, project files, and isolated
coding execution. This prevents agents from drifting into the wrong directory,
keeps secrets and sessions out of project repositories, and lets many coding
workers operate on the same project without editing the same checkout.

The required concepts are:

| Concept | Purpose | Example |
| --- | --- | --- |
| Profile home | One CADIS identity/environment with config, channels, agents, memory, sessions, skills, workers, logs, and artifacts | `~/.cadis/profiles/default/` |
| Agent home | One persistent agent's persona, instructions, memory, skills, and policy | `~/.cadis/profiles/default/agents/rama/` |
| Project workspace | A real user project root that tools may access only after a grant | `~/Project/chatbot-ai-saas/` |
| Worker worktree | An isolated git checkout for one coding worker/task | `~/Project/chatbot-ai-saas/.cadis/worktrees/w-auth-01/` |

Rule: agent home is not project cwd, and profile home is not a sandbox.

## 2. Implemented Now

The current desktop MVP implements only part of this architecture:

- `CADIS_HOME` defaults to `~/.cadis`.
- `~/.cadis/config.toml` can be loaded.
- JSONL event logs are written with redaction boundaries.
- Store-level atomic JSON metadata helpers exist under `~/.cadis/state`.
- Safe-read tool baselines canonicalize paths and reject outside-workspace reads.
- Worker protocol/event types and HUD worker display concepts exist.

Implemented baseline:

- `~/.cadis/profiles/<profile>/` profile homes.
- Persistent daemon-known agent homes under each profile with `AGENT.toml`,
  persona/instruction/user/memory/tool guidance files, typed `POLICY.toml`
  metadata, `SKILL_POLICY.toml`, and agent-local memory/skill/prompt folders.
- Profile-local workspace registry and grant files.
- Protocol/CLI workspace commands: `list`, `register`, `grant`, `revoke`, and
  `doctor`.
- Tool calls require a registered workspace and matching active grant before
  safe-read execution or approval flow.
- `workspace doctor` includes profile/agent-home diagnostics for missing,
  corrupt, and oversized agent files.
- Worker events include planned worktree/artifact metadata.
- Store helpers now resolve project worker worktree paths and metadata files
  under `<project>/.cadis/worktrees/`, resolve worker artifact paths under
  `profiles/<profile>/artifacts/workers/`, and let `workspace doctor` report
  stale worker worktree metadata or missing artifact roots.
- Worker execution setup now creates a git worktree for session-bound project
  workspaces, moves project-local worker worktree metadata through `ready`,
  `review_pending`, and `cleanup_pending`, and writes profile-scoped worker
  artifacts for review.
- `worker.cleanup` records cleanup intent for terminal CADIS-owned worker
  worktrees without deleting files and rejects unknown, missing, or non-owned
  paths.

Still future:

- Worker worktree file removal.
- Real worker command/test execution and durable worker runtime logs.
- Checkpoint and rollback manager.
- Dedicated profile and agent doctor commands.
- Workspace-local skills and project `.cadis/` metadata enforcement.

## 3. Target Home Layout

Top-level CADIS home:

```text
~/.cadis/
|-- config.toml                    # global non-secret defaults
|-- profiles/                      # independent CADIS environments
|-- global-cache/                  # shared cache; safe to delete
|-- plugins/                       # installed CADIS plugins/extensions
|-- bin/                           # optional managed helper binaries
|-- logs/                          # daemon-level redacted logs
|-- run/                           # sockets, pid files, selected ports
`-- VERSION
```

Profile home:

```text
~/.cadis/profiles/default/
|-- profile.toml                   # profile config and feature flags
|-- .env                           # secrets fallback; chmod 0600; never committed
|-- secrets/                       # encrypted or OS-keyring-backed secret handles
|-- channels/                      # Telegram/HUD/mobile state, no plaintext secrets
|-- agents/                        # persistent agent homes
|-- memory/                        # profile-wide memory
|-- skills/                        # profile-level skills
|-- workspaces/                    # workspace registry, aliases, grants
|-- workers/                       # worker state records and streams
|-- sessions/                      # profile session JSONL and metadata
|-- artifacts/                     # patches, test reports, generated files
|-- checkpoints/                   # shadow repos for rollback
|-- sandboxes/                     # temporary isolated roots
|-- eventlog/                      # append-only event streams
|-- cron/                          # scheduled jobs
|-- logs/                          # profile logs, redacted
`-- locks/                         # state mutation locks
```

Agent home:

```text
~/.cadis/profiles/default/agents/rama/
|-- AGENT.toml
|-- PERSONA.md
|-- INSTRUCTIONS.md
|-- USER.md
|-- MEMORY.md
|-- TOOLS.md                       # guidance only, not permission policy
|-- POLICY.toml                    # hard permissions and sandbox defaults
|-- SKILL_POLICY.toml
|-- skills/
|-- memory/
|   |-- daily/
|   |-- decisions.md
|   `-- delegation.md
|-- prompts/
|-- sessions/                      # symlink or index into profile sessions
`-- README.md
```

## 4. Project Metadata

Project files stay in the user's project. CADIS metadata inside a project lives
under `.cadis/` and must not contain secrets:

```text
<project>/.cadis/
|-- workspace.toml
|-- skills/
|-- artifacts/
|-- worktrees/
`-- media/
```

`workspace.toml` records project-local defaults such as workspace ID, VCS type,
worktree root, artifact root, and media root. It does not grant access by
itself; the daemon must still resolve a workspace grant.

Current baseline: `cadis-store` can load and write this project-local metadata,
and `workspace doctor` reports missing files, registry ID mismatch, absolute
project-local roots, and duplicate registered roots. Creation/initialization UX
remains future work.

## 5. Media Assets Convention

Project-scoped media assets generated or curated by CADIS should use:

```text
<project>/.cadis/media/
|-- input/                         # user-provided or copied references
|-- generated/                     # generated images, audio, video, sprites
|-- thumbnails/
|-- manifests/
`-- exports/
```

Rules:

- Media files that are source assets for the project may be moved into the
  project proper only through an explicit write/apply action.
- Generated media manifests must record source prompt or task ID, producing
  agent/worker ID, model/provider if known, license/source notes, and target use.
- Secrets, provider tokens, raw channel tokens, and private session transcripts
  must never be stored in project `.cadis/media/`.
- Large binary media should be ignored by default unless the project explicitly
  opts into tracking it.

## 6. Workspace Registry and Grants

Project roots are registered in profile state, not guessed repeatedly:

```text
~/.cadis/profiles/default/workspaces/
|-- registry.toml
|-- aliases.toml
`-- grants.jsonl
```

Example:

```toml
[[workspace]]
id = "chatbot-ai-saas"
kind = "project"
root = "~/Project/chatbot-ai-saas"
vcs = "git"
owner = "rama"
trusted = true
worktree_root = ".cadis/worktrees"
artifact_root = ".cadis/artifacts"
media_root = ".cadis/media"
checkpoint_policy = "enabled"
```

Every `file.*`, `shell.*`, `git.*`, and `worker.*` operation must resolve a
workspace grant before execution:

```text
WorkspaceGrant {
  profile_id,
  agent_id,
  workspace_id,
  root,
  access: read | write | exec | admin,
  expires_at,
  source: route | user | policy | worker_spawn
}
```

If no grant exists, the tool fails closed or requests approval. Grants are
runtime policy records; project `.cadis/workspace.toml` is only metadata.

## 7. Denied Paths

Path resolution must canonicalize paths, reject symlink escapes, verify the
granted root, check access mode, and then enforce denied paths.

Minimum denied paths:

```text
~/.ssh
~/.aws
~/.gnupg
~/.config/gcloud
~/.cadis/profiles/*/.env
~/.cadis/profiles/*/secrets
~/.cadis/profiles/*/channels/*/tokens
/etc
/dev
/proc
/sys
```

Broad grants to `/`, `$HOME`, system directories, cloud credential directories,
or profile secret directories are invalid unless a future admin mode explicitly
defines a safer exception.

## 8. Worker Worktrees

Coding workers use git worktrees by default:

```text
main project checkout  -> user-owned stable branch
worker worktree        -> isolated branch per task
checkpoint shadow repo -> rollback safety net
final patch/PR         -> reviewed before merge
```

Default worker worktree path:

```text
<project>/.cadis/worktrees/<worker-id>/
```

Worker state and artifacts live under the profile:

```text
~/.cadis/profiles/default/workers/<worker-id>.toml
~/.cadis/profiles/default/workers/<worker-id>.jsonl
~/.cadis/profiles/default/artifacts/workers/<worker-id>/
```

Project-local worker worktree metadata lives beside the project worktree root:

```text
<project>/.cadis/worktrees/
|-- <worker-id>/                  # planned/actual git worktree checkout
`-- .metadata/
    `-- <worker-id>.toml          # worker ID, workspace ID, path, branch, base ref, state, artifact root
```

`workspace doctor` treats these metadata files as diagnostics only. It warns
when a recorded worktree path or profile artifact root is stale/missing; it does
not create, remove, or mutate git worktrees. Worktree creation and cleanup
planning are owned by the daemon worker runtime when a session-bound worker
starts or reaches a terminal cleanup flow. Cleanup planning moves metadata to
`cleanup_pending`; actual file removal remains a later explicit executor.

Workers receive write/exec grants only for their worktree unless the user
explicitly approves broader access. The parent project checkout remains
read-only by default.

## 9. Skill Precedence

Target precedence, highest first:

1. Agent-local: `profiles/<profile>/agents/<agent>/skills/`
2. Workspace-local: `<project>/.cadis/skills/`
3. Profile-local: `profiles/<profile>/skills/approved/`
4. Managed plugins: `~/.cadis/plugins/*/skills/`
5. Bundled CADIS skills

Generated skills should start as candidates and become active only after review
or explicit policy.

## 10. Future Work Phases

The workspace architecture should be implemented in this order:

| Phase | Outcome |
| --- | --- |
| W0 | Finalize terminology and protocol types |
| W1 | CADIS home resolver and directory skeleton |
| W2 | Profile manager baseline implemented |
| W3 | Agent home manager/templates baseline implemented |
| W4 | Workspace registry and grants |
| W5 | Worker worktree manager |
| W6 | Checkpoint and rollback manager |
| W7 | Event log and session store integration |
| W8 | Channel bindings and deterministic routing |
| W9 | Doctor, migration, and templates; profile/agent file diagnostics baseline implemented |
