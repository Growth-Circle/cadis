# CADIS Tool Runtime Standard

## 1. Purpose

This standard defines how native CADIS tools are registered, validated, executed, observed, and governed. Tools are a privileged runtime surface. They must be designed for safety, auditability, and predictable behavior.

## 2. Tool Registry

All tools must be registered in the daemon-owned tool registry before use.

Each tool must declare:

- stable name
- version or compatibility marker
- input schema
- output schema
- risk class
- workspace constraints
- timeout behavior
- cancellation support
- event fields emitted during execution

Initial tools:

- `file.read`
- `file.search`
- `file.patch`
- `shell.run`
- `git.status`
- `git.diff`
- `git.worktree.create`
- `git.worktree.remove`

Current implementation baseline:

- `file.read`, `file.search`, and `git.status` are native safe-read tools.
- `shell.run`, `file.write`, `file.patch`, `git.diff`,
  `git.worktree.create`, and `git.worktree.remove` are classified and
  approval-gated placeholders.
- Unknown tool names are denied before execution.
- Approval-gated placeholders fail closed after approval resolution until their
  execution backends are implemented.
- Worker orchestration may emit planned worktree and artifact metadata without
  invoking `git.worktree.create`; that event metadata is intent, not filesystem
  mutation.

## 3. Naming

Tool names must be stable protocol identifiers.

Rules:

- Use dotted names: `domain.action`.
- Do not encode implementation language in the name.
- Do not rename a tool without a compatibility plan.
- Experimental tools must be clearly marked in capability metadata.

## 4. Schema Requirements

Tool inputs and outputs must be structured data.

Rules:

- Validate input before policy evaluation and execution.
- Reject unknown or incompatible fields when they affect safety.
- Return structured errors with codes, messages, and actionable metadata.
- Include normalized paths separately from user-supplied paths when path safety matters.
- Avoid ad hoc string parsing when a structured API is available.

## 5. Risk Classes

Every tool call must have a risk classification.

Example classes:

| Risk Class | Examples | Default |
| --- | --- | --- |
| safe-read | file reads inside workspace, git status | auto-allow |
| bounded-write | patch inside workspace | policy decision |
| command | shell command without obvious destructive behavior | policy decision |
| secret-access | environment or config secret access | approval required |
| outside-workspace-write | writing outside allowed roots | approval required |
| dangerous-delete | recursive delete, destructive cleanup | approval required |
| system-change | sudo, package manager, service mutation | approval required |
| protected-git-write | push to protected branches, force push | approval required |

Risk classification must be conservative. If classification is ambiguous, choose the higher-risk class.

## 6. Workspace Boundaries

Tools must enforce workspace constraints in the daemon.

Rules:

- Resolve and normalize paths before access.
- Reject path traversal outside allowed roots unless policy explicitly allows it.
- Symlinks must not bypass workspace boundaries.
- Resolve registered workspace IDs and active workspace grants before execution.
- Agent-scoped grants require matching `tool.call.agent_id`; global grants omit
  `agent_id`.
- Outside-workspace writes require approval.
- Reads that may expose secrets require policy review even if inside workspace.
- Temporary test directories must be isolated from user state.

## 7. Execution Lifecycle

Every tool call must emit lifecycle events:

```text
tool.started
tool.output.delta
tool.completed
tool.failed
tool.cancelled
```

Events must include:

- `session_id`
- `agent_id`, when applicable
- `tool_call_id`
- tool name
- risk class
- started and completed timestamps
- cwd or workspace, when applicable

Output events must be bounded. Long stdout, stderr, diffs, and logs should be chunked, summarized, or routed to appropriate log storage.

## 8. Shell Tool

`shell.run` is high risk and must be policy-gated.

Requirements:

- explicit command and args representation where possible
- explicit cwd
- timeout
- cancellation
- stdout and stderr capture
- exit code capture
- environment filtering
- no implicit secret injection
- structured failure for timeout, spawn failure, and nonzero exit

Commands that mutate system state, use `sudo`, install packages, delete files, alter protected git refs, or access secrets must require approval.

## 9. File Tools

`file.read` and `file.search` should be safe by default only inside allowed workspaces and after secret policy checks.

The initial `file.read` and `file.search` implementation resolves paths with
canonical filesystem paths and rejects reads outside the request workspace.
Search skips unreadable files and large files, redacts result lines, and bounds
the returned match list.

`file.patch` requirements:

- patch must be previewable
- patch must apply only to intended files
- conflicts must be explicit
- writes must be atomic where practical
- generated patches must not overwrite unrelated edits
- outside-workspace writes require approval

Patch application must preserve user changes and should fail closed on mismatched context.

## 10. Git Tools

Git tools must prefer explicit commands or library calls with clear output.

Rules:

- `git.status` and `git.diff` are safe-read by default.
- The current native baseline implements only `git.status` and runs
  `git status --short --branch` under the daemon after workspace path
  normalization.
- Worktree creation must validate repository state.
- Worktree cleanup must avoid deleting paths not created by CADIS.
- `git.worktree.create` must consume daemon worktree intent, not ad hoc client
  paths, and must emit a later event that moves the worktree state from
  `planned` to `active` only after successful creation.
- `git.worktree.remove` must require a CADIS-owned worker/worktree record and
  preserve artifacts unless an explicit cleanup policy says otherwise.
- Push, force-push, rebase, reset, and branch deletion are not initial native tool actions unless policy and approval coverage exist.

## 11. Cancellation and Timeouts

All long-running tools must support cancellation or have a hard timeout.

Rules:

- Timeouts must produce `tool.failed` with timeout metadata.
- Cancellation must produce `tool.cancelled`.
- Child processes must be cleaned up when possible.
- The agent must receive a structured result after cancellation.

## 12. Redaction and Logging

Tool logs must pass through redaction before persistence.

Never log:

- raw provider keys
- shell environment secrets
- unredacted auth headers
- private tokens in command output

When redaction changes output, preserve enough context for debugging without exposing the secret.

## 13. Testing Requirements

Required tests:

- registry rejects duplicate tool names
- schema validation accepts valid inputs and rejects unsafe ones
- risk classification for representative commands and paths
- workspace path normalization
- symlink boundary handling
- shell timeout and cancellation
- file patch conflict handling
- tool lifecycle event ordering
- redaction before persistence
- policy denial prevents execution
