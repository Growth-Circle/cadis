# Storage Format and Migration — v1.0

## 1. `~/.cadis` Directory Layout

```text
~/.cadis/
├── config.toml                        # Global daemon configuration (TOML)
├── profiles/
│   └── {profile_id}/                  # One profile home per identity
│       ├── profile.toml
│       ├── agents/{agent_id}/         # Persistent agent homes
│       ├── workspaces/
│       │   ├── registry.toml          # Workspace registry
│       │   ├── aliases.toml           # Workspace aliases
│       │   └── grants.jsonl           # Append-only workspace grants
│       ├── artifacts/workers/{id}/    # Worker artifact outputs
│       ├── checkpoints/               # Checkpoint snapshots
│       ├── sessions/                  # Profile session data
│       ├── workers/                   # Worker state records
│       ├── memory/                    # Profile-wide memory
│       ├── skills/                    # Profile-level skills
│       ├── eventlog/                  # Append-only event streams
│       ├── secrets/                   # Encrypted/keyring secret handles
│       ├── logs/                      # Profile-scoped redacted logs
│       ├── locks/                     # State mutation locks
│       └── run/                       # Profile runtime files
├── state/
│   ├── sessions/{session_id}.json     # Session metadata
│   ├── agents/{agent_id}.json         # Agent metadata
│   ├── agent-sessions/{id}.json       # AgentSession metadata
│   ├── workers/{worker_id}.json       # Worker metadata
│   └── approvals/{approval_id}.json   # Approval metadata
├── logs/
│   ├── {session_id}.jsonl             # Per-session redacted event log
│   └── daemon.jsonl                   # Daemon-level events (no session)
├── run/
│   └── cadisd.sock                    # Daemon Unix socket
├── global-cache/                      # Shared cache (safe to delete)
├── plugins/                           # Installed plugins/extensions
└── bin/                               # Managed helper binaries
```

All directories under `~/.cadis` are created with mode `0700`. All files are
written with mode `0600`. Writes use atomic rename via temporary files to
prevent partial reads.

The socket path resolves as: `$CADIS_SOCKET` → `config.toml socket_path` →
`$XDG_RUNTIME_DIR/cadis/cadisd.sock` → `~/.cadis/run/cadisd.sock`.

## 2. State Metadata Formats

Each state record is a single pretty-printed JSON file under `~/.cadis/state/`.
File names use redaction-safe IDs: path separators and special characters are
replaced with `_`. Secret-looking values are replaced with `[REDACTED]` before
writing.

### Session (`state/sessions/{session_id}.json`)

```json
{
  "session_id": "ses_abc123",
  "title": "User chat session",
  "cwd": "/home/user/project"
}
```

### Agent (`state/agents/{agent_id}.json`)

```json
{
  "agent_id": "main",
  "role": "Orchestrator",
  "display_name": "CADIS",
  "parent_agent_id": null,
  "model": "auto",
  "status": "ready",
  "specialist_id": "cadis-main",
  "specialist_label": "Main Orchestrator",
  "persona": "..."
}
```

### AgentSession (`state/agent-sessions/{id}.json`)

```json
{
  "agent_session_id": "ags_xyz789",
  "session_id": "ses_abc123",
  "route_id": "route_1",
  "agent_id": "main",
  "parent_agent_id": null,
  "task": "Respond to user message",
  "status": "running",
  "timeout_at": "2026-04-29T01:15:00Z",
  "budget_steps": 8,
  "steps_used": 2,
  "result": null,
  "error_code": null,
  "error": null,
  "cancellation_requested_at": null
}
```

### Worker (`state/workers/{worker_id}.json`)

```json
{
  "worker_id": "worker_1",
  "session_id": "ses_abc123",
  "agent_id": "coder",
  "parent_agent_id": "main",
  "agent_session_id": "ags_xyz789",
  "status": "running",
  "cli": null,
  "cwd": "/home/user/project/.cadis/worktrees/worker_1",
  "summary": "Implement auth module",
  "error_code": null,
  "error": null,
  "cancellation_requested_at": null,
  "worktree": { "workspace_id": "my-project", "branch": "cadis/worker_1/auth" },
  "artifacts": { "root": "~/.cadis/profiles/default/artifacts/workers/worker_1" },
  "updated_at": "2026-04-29T01:00:00Z"
}
```

### Approval (`state/approvals/{approval_id}.json`)

```json
{
  "approval_id": "apr_1",
  "session_id": "ses_abc123",
  "tool_call_id": "tool_1",
  "tool_name": "shell.run",
  "risk_class": "system_change",
  "title": "Approval needed",
  "summary": "Run command in workspace",
  "command": "make build",
  "workspace": "/home/user/project",
  "requested_at": "2026-04-29T01:00:00Z",
  "expires_at": "2026-04-29T01:05:00Z",
  "state": "pending",
  "decision": null,
  "reason": null,
  "resolved_at": null
}
```

`state` values: `pending`, `resolved`, `expired`.
`decision` values (when resolved): `approved`, `denied`.

## 3. JSONL Event Log Format

Event logs are append-only JSONL files under `~/.cadis/logs/`. Each line is one
redacted `EventEnvelope`:

```json
{"protocol_version":"0.9","event_id":"evt_001","timestamp":"2026-04-29T01:00:00Z","source":"cadisd","session_id":"ses_abc123","event":"message.complete","data":{...}}
```

Fields:

| Field | Type | Description |
|---|---|---|
| `protocol_version` | string | Protocol version (e.g. `"0.9"`) |
| `event_id` | string | Daemon-generated unique event ID |
| `timestamp` | string | UTC ISO-8601 timestamp |
| `source` | string | Source component, usually `"cadisd"` |
| `session_id` | string? | Session ID when event belongs to a session |
| `event` | string | Event type (flattened via serde) |

Events without a session ID are written to `daemon.jsonl`. Events with a
session ID are written to `{session_id}.jsonl`. All secret-looking values are
redacted before writing.

## 4. Workspace Registry Format

### Registry (`profiles/{id}/workspaces/registry.toml`)

```toml
[[workspace]]
id = "my-project"
kind = "project"
root = "~/Project/my-project"
vcs = "git"
owner = "rama"
trusted = true
worktree_root = ".cadis/worktrees"
artifact_root = ".cadis/artifacts"
checkpoint_policy = "enabled"

[[workspace.alias]]
workspace_id = "my-project"
aliases = ["myproj", "mp"]
```

`kind` values: `project`, `documents`, `sandbox`, `worktree`.
`vcs` values: `none`, `git`.
`checkpoint_policy` values: `enabled`, `disabled`.

### Grants (`profiles/{id}/workspaces/grants.jsonl`)

Append-only JSONL. Each line is one `WorkspaceGrantRecord`:

```json
{"grant_id":"grant_1","profile_id":"default","agent_id":"main","workspace_id":"my-project","root":"/home/user/Project/my-project","access":["read","write"],"created_at":"2026-04-29T01:00:00Z","expires_at":"2026-04-29T02:00:00Z","source":"user","reason":"User approved workspace access"}
```

`access` values: `read`, `write`, `exec`, `admin`.
`source` values: `route`, `user`, `policy`, `worker_spawn`.

## 5. Project `.cadis/` Layout

```text
<project>/.cadis/
├── workspace.toml                     # Project-local workspace metadata
├── .gitignore                         # Ignores worktrees/, artifacts/, tmp/, logs/
├── worktrees/
│   ├── {worker_id}/                   # Git worktree checkout
│   └── .metadata/
│       └── {worker_id}.toml           # Worktree lifecycle metadata
├── artifacts/                         # Workspace-local artifacts
└── media/
    └── manifest.json                  # Media provenance manifest
```

### `workspace.toml`

```toml
workspace_id = "my-project"
kind = "project"
vcs = "git"
worktree_root = ".cadis/worktrees"
artifact_root = ".cadis/artifacts"
media_root = ".cadis/media"
```

### Worker worktree metadata (`.metadata/{worker_id}.toml`)

```toml
worker_id = "worker_1"
workspace_id = "my-project"
worktree_path = ".cadis/worktrees/worker_1"
branch_name = "cadis/worker_1/auth"
base_ref = "HEAD"
state = "ready"
artifact_root = "/home/user/.cadis/profiles/default/artifacts/workers/worker_1"
```

`state` values: `planned`, `ready`, `review_pending`, `cleanup_pending`, `removed`.

### Media manifest (`media/manifest.json`)

```json
{
  "entries": [
    {
      "path": "generated/image.png",
      "generated_by": "agent/main",
      "tool": "image.generate",
      "created_at": "2026-04-29T01:00:00Z",
      "description": "Generated concept art"
    }
  ]
}
```

## 6. State Migration Policy

The v1.0 storage format is **stable**. Future v1.x releases will include
migration scripts if the format changes.

**v0.x → v1.0 migration:** Delete `~/.cadis/state/` and restart the daemon.
Session and agent-session state is ephemeral and rebuilt on daemon startup.
Worker and approval state from v0.x is not carried forward.

No migration is needed for profile homes, workspace registries, or agent homes
— these formats are forward-compatible from v0.9.

## 7. Backup and Restore

To back up a C.A.D.I.S. installation:

```bash
cp ~/.cadis/config.toml /backup/cadis/config.toml
cp -a ~/.cadis/profiles/ /backup/cadis/profiles/
```

To restore:

```bash
cp /backup/cadis/config.toml ~/.cadis/config.toml
cp -a /backup/cadis/profiles/ ~/.cadis/profiles/
```

The `state/` directory is ephemeral and does not need to be backed up. The
`logs/` directory can optionally be preserved for audit. The `run/` directory
contains runtime sockets and should not be backed up.
