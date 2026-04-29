# Stable CLI Reference — v1.0

CLI binary: `cadis`. Communicates with the `cadisd` daemon over a Unix socket.

## Global flags

| Flag | Short | Description |
|------|-------|-------------|
| `--json` | | Print raw NDJSON server frames instead of human-readable output |
| `--socket <PATH>` | | Override the Unix socket path to `cadisd` |
| `--help` | `-h` | Print help and exit |
| `--version` | `-V` | Print version and exit |

Global flags must appear **before** the subcommand.

---

## Commands

### status — stable

Show daemon status (version, uptime, model provider, sessions, voice state).

```bash
cadis status
cadis --json status
```

### doctor — stable

Check local config, daemon connectivity, and voice subsystem health.

```bash
cadis doctor
cadis --json doctor
```

### chat — stable

Send a one-shot chat message to the daemon.

```bash
cadis chat "hello world"
cadis --json chat "summarize this project"
```

**Arguments:** `<MESSAGE>` (required, may be multiple words joined by space)

### models — stable

List configured model providers and their readiness.

```bash
cadis models
cadis --json models
```

### agents — stable

List daemon-owned agents.

```bash
cadis agents
cadis --json agents
```

### approve — stable

Approve a pending approval request.

```bash
cadis approve <APPROVAL_ID>
```

**Arguments:** `<APPROVAL_ID>` (required)

### deny — stable

Deny a pending approval request.

```bash
cadis deny <APPROVAL_ID>
```

**Arguments:** `<APPROVAL_ID>` (required)

### spawn — stable

Spawn a new child or subagent.

```bash
cadis spawn worker
cadis spawn worker --name w1 --model gpt-4
cadis spawn researcher --parent agent_main --name r1
```

**Arguments:** `<ROLE>` (required)

| Flag | Description |
|------|-------------|
| `--name <NAME>` | Display name for the new agent |
| `--parent <AGENT_ID>` | Parent agent ID (default: main) |
| `--model <MODEL>` | Provider/model identifier |

### events — stable

Subscribe to daemon runtime events (streams until interrupted).

```bash
cadis events
cadis events --snapshot
cadis events --replay 50
cadis events --since evt_abc123
cadis events --no-snapshot
```

| Flag | Description |
|------|-------------|
| `--snapshot` | Print one state snapshot and exit |
| `--replay <COUNT>` | Replay up to COUNT buffered events before live (default: 128) |
| `--since <EVENT_ID>` | Replay retained events after EVENT_ID |
| `--no-snapshot` | Subscribe without initial state snapshot |

### worker tail — stable

Stream recent log output from a worker.

```bash
cadis worker tail <WORKER_ID>
cadis worker tail w1 --lines 50
```

**Arguments:** `<WORKER_ID>` (required)

| Flag | Description |
|------|-------------|
| `--lines <COUNT>` | Number of log lines to return |

### worker result — stable

Retrieve the final result of a completed worker.

```bash
cadis worker result <WORKER_ID>
cadis --json worker result w1
```

**Arguments:** `<WORKER_ID>` (required)

### workspace list — stable

List registered workspaces.

```bash
cadis workspace list
cadis workspace list --grants
```

| Flag | Description |
|------|-------------|
| `--grants` | Include workspace grants in the output |

### workspace doctor — stable

Run health checks on a workspace.

```bash
cadis workspace doctor
cadis workspace doctor --workspace my-project
cadis workspace doctor --root /home/user/project
cadis workspace doctor my-project
```

| Flag | Description |
|------|-------------|
| `--workspace <ID>` | Workspace ID to check (also accepted as positional) |
| `--root <PATH>` | Workspace root path to check |

### voice status — stable

Show daemon-visible voice subsystem status.

```bash
cadis voice
cadis voice status
```

### voice doctor — stable

Show voice doctor checks and local bridge preflight state.

```bash
cadis voice doctor
cadis --json voice doctor
```

---

## Experimental commands

These commands may change or be removed in future v1.x releases.

### run — experimental

Send a desktop task as a chat request, optionally scoped to a directory.

```bash
cadis run "build all"
cadis run --cwd /tmp build all
```

**Arguments:** `<TASK>` (required, remaining args joined)

| Flag | Description |
|------|-------------|
| `--cwd <PATH>` | Working directory context for the task |

### worker cleanup — experimental

Not yet implemented as a CLI subcommand. Worker worktree cleanup is managed through the daemon protocol.

### workspace register — experimental

Register a new workspace with the daemon.

```bash
cadis workspace register my-project /home/user/project
cadis workspace register my-project /home/user/project --kind sandbox --untrusted
```

**Arguments:** `<ID>` (required), `<ROOT>` (required)

| Flag | Description |
|------|-------------|
| `--kind <KIND>` | `project`, `documents`, `sandbox`, or `worktree` (default: project) |
| `--alias <ALIAS>` | Alias for the workspace (repeatable) |
| `--vcs <VCS>` | VCS type (default: git) |
| `--no-vcs` | Mark workspace as having no VCS |
| `--trusted` / `--untrusted` | Trust level (default: trusted) |
| `--worktree-root <PATH>` | Worktree root (default: `.cadis/worktrees`) |
| `--artifact-root <PATH>` | Artifact root (default: `.cadis/artifacts`) |

### workspace grant — experimental

Grant workspace access to an agent.

```bash
cadis workspace grant my-project --access read,write --agent agent_w1
```

**Arguments:** `<WORKSPACE_ID>` (required)

| Flag | Description |
|------|-------------|
| `--agent <AGENT_ID>` | Agent to grant access to |
| `--access <LIST>` | Comma-separated: `read`, `write`, `exec`, `admin` |
| `--source <SOURCE>` | Grant source label (default: `user`) |

### workspace revoke — experimental

Revoke a workspace grant.

```bash
cadis workspace revoke --grant grant_abc123
cadis workspace revoke --workspace my-project --agent agent_w1
cadis workspace revoke my-project
```

| Flag | Description |
|------|-------------|
| `--grant <GRANT_ID>` | Specific grant ID to revoke |
| `--workspace <ID>` | Workspace ID (also accepted as positional) |
| `--agent <AGENT_ID>` | Agent whose grants to revoke |

---

## Internal commands

These are not part of the public CLI contract.

| Command | Description |
|---------|-------------|
| `daemon [ARGS...]` | Launch `cadisd` from sibling path or `$PATH` |
| `tool [OPTIONS] <NAME>` | Direct daemon tool call (internal/debug) |
| `session subscribe <ID>` | Subscribe to a session event stream (internal/debug) |

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (connection failure, invalid input, daemon rejection) |

All errors print a message to stderr in the format: `cadis: <message>`.

---

## JSON output format (`--json`)

When `--json` is passed, the CLI prints one NDJSON line per server frame instead of human-readable output. Each line is a `ServerFrame` — either a `Response` or an `Event`.

### Response frame

```json
{
  "type": "response",
  "payload": {
    "request_id": "req_12345_1700000000000",
    "response": {
      "type": "daemon.status",
      "status": "running",
      "version": "0.9.0",
      "protocol_version": "1",
      "cadis_home": "/home/user/.cadis",
      "sessions": 1,
      "model_provider": "ollama",
      "uptime_seconds": 3600
    }
  }
}
```

### Event frame

```json
{
  "type": "event",
  "payload": {
    "event_id": "evt_abc123",
    "session_id": "sess_001",
    "type": "message.completed",
    ...
  }
}
```

### Rejection frame

```json
{
  "type": "response",
  "payload": {
    "request_id": "req_12345_1700000000000",
    "response": {
      "type": "request.rejected",
      "code": "invalid_request",
      "message": "unknown command",
      "retryable": false
    }
  }
}
```

For streaming commands (`events`, `session subscribe`), frames are emitted as they arrive, one per line.
