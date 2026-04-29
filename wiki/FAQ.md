# FAQ

## Why local-first?

C.A.D.I.S. keeps state, logs, approvals, and orchestration on your machine. No cloud account is required to run the daemon or use the CLI. This means your conversation history, agent state, and approval audit trail stay under your control. Cloud model providers (OpenAI, Codex CLI) are optional — the system works offline with Ollama or the built-in local fallback.

## What models work?

C.A.D.I.S. is model-agnostic. Supported providers:

- **Ollama** — any model Ollama supports (e.g., `llama3.2`, `mistral`, `codellama`). Responses stream natively.
- **OpenAI API** — any Chat Completions-compatible model (e.g., `gpt-4o`, `gpt-5.2`). Requires an API key. Responses stream via SSE.
- **Codex CLI** — ChatGPT Plus/Pro subscription models through the official Codex CLI. Authenticate with `codex login`.
- **Local fallback** — a credential-free echo provider for testing when no external model is available.

The default `auto` mode tries Ollama first, then falls back to the local provider.

## How do approvals work?

C.A.D.I.S. classifies tool actions by risk level. Low-risk read-only operations (file reads, git diff) execute directly. Risky actions (shell commands, file mutations) require explicit approval before execution. The daemon enforces:

- Workspace and input revalidation before execution
- Shell environment filtering via allowlist
- Secret-file gating and denied-path enforcement
- Approval expiry recheck before execution

Approval state is persisted and auditable through JSONL event logs with redaction boundaries.

## Can I use it on macOS or Windows?

**Linux desktop** is the primary supported platform for the full runtime, CLI, and HUD.

**macOS** is a Rust source-validation baseline. You can build and test the core Rust crates, but the daemon, CLI transport, and HUD are not packaged or fully validated for macOS yet.

**Windows** is limited to portable-crate validation (protocol, policy, store, models, avatar). Daemon, CLI transport, shell, path, sandbox, HUD, and audio adapters do not exist yet.

See [Platform Baseline](https://github.com/Growth-Circle/cadis/blob/main/docs/28_PLATFORM_BASELINE.md) for the full support matrix.

## How do I add a new agent?

Agents are managed through the daemon. You can spawn agents via:

1. **CLI**: `cadis agents` lists current agents.
2. **Protocol**: send an `agent.spawn` request through the daemon protocol.
3. **Orchestrator actions**: use `/worker <task>` or `/spawn` in chat messages when `worker_delegation_enabled = true`.

Agent spawn is governed by `[agent_spawn]` limits in `config.toml`:

```toml
[agent_spawn]
max_depth = 2
max_children_per_parent = 4
max_total_agents = 32
```

Each agent gets a home directory under `~/.cadis/profiles/default/agents/<agent>/` with identity (`AGENT.toml`), persona, instructions, memory, tools, and policy files.

## How do I use voice?

Enable voice in `config.toml`, set up `whisper-cli` for mic input, and use the HUD Settings → Voice tab to verify dependencies. See [[Configuration#voice-setup]] for details.

## Where is state stored?

All local state lives under `~/.cadis/` (or `$CADIS_HOME`):

- `config.toml` — user configuration
- `logs/` — JSONL event audit logs (redacted)
- `state/` — durable session, agent, worker, and approval metadata
- `profiles/` — profile-scoped agents, workspaces, artifacts, and memory
- `run/` — daemon socket
