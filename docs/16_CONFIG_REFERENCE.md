# Configuration Reference

## 1. Default Location

The desktop MVP reads user config from:

```text
~/.cadis/config.toml
```

`CADIS_HOME` can move that directory. CADIS creates this local state layout on
first run:

```text
~/.cadis/
|-- config.toml
|-- profiles/
|   `-- default/
|       |-- profile.toml
|       |-- .gitignore
|       |-- agents/
|       |-- memory/
|       |-- skills/
|       |-- workspaces/
|       |   |-- registry.toml
|       |   |-- aliases.toml
|       |   `-- grants.jsonl
|       |-- workers/
|       |-- sessions/
|       |-- artifacts/
|       |-- checkpoints/
|       |-- sandboxes/
|       |-- eventlog/
|       |-- channels/
|       |-- secrets/
|       |-- logs/
|       |-- locks/
|       `-- run/
|-- logs/
|   |-- <session-id>.jsonl
|   `-- daemon.jsonl
|-- state/
|   |-- sessions/
|   |   `-- <session-id>.json
|   |-- agents/
|   |   `-- <agent-id>.json
|   |-- workers/
|   |   `-- <worker-id>.json
|   `-- approvals/
|       `-- <approval-id>.json
|-- worktrees/
|-- run/
|-- tokens/
|-- sessions/
|-- workers/
`-- approvals/
```

`logs/` is append-only audit history. `state/` is the durable restart metadata
baseline for runtime recovery. The top-level `sessions/`, `workers/`, and
`approvals/` directories are reserved compatibility directories; new JSON
metadata belongs under `state/`.

Profile-aware helpers now also initialize `~/.cadis/profiles/default/`. The
profile tree is the CADIS-standard home for profile-local agents, memory,
workspace registry metadata, worker records, session records, artifacts,
checkpoints, event logs, channels, and secrets. The store crate keeps creating
the existing top-level paths so current daemon/core/CLI code continues to work
while profile-aware runtime pieces are added.

## 2. Desktop MVP Config

The current implementation supports these keys:

```toml
cadis_home = "~/.cadis"
log_level = "info"

# Optional. If unset, CADIS uses $XDG_RUNTIME_DIR/cadis/cadisd.sock when
# available, otherwise ~/.cadis/run/cadisd.sock.
# socket_path = "~/.cadis/run/cadisd.sock"

[profile]
# Default profile initialized under ~/.cadis/profiles/<profile>/.
default_profile = "default"

[model]
# auto tries Ollama first and falls back to the local credential-free provider.
# Supported values: "auto", "codex-cli", "openai", "ollama", "echo".
# Per-agent selections may use "provider/model", for example "ollama/llama3.2"
# or "openai/gpt-5.2"; unsupported providers are rejected by cadisd.
provider = "auto"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"

[hud]
theme = "arc"
# Supported today: "orb", "wulan_arc".
# Future native value after renderer integration: "wulan_native".
avatar_style = "orb"
background_opacity = 90
always_on_top = false

[voice]
enabled = false
voice_id = "id-ID-GadisNeural"
rate = 0
pitch = 0
volume = 0
auto_speak = false

[agent_spawn]
# Applies to client-requested agent.spawn and explicit orchestrator /worker actions.
max_depth = 2
max_children_per_parent = 4
max_total_agents = 32

[orchestrator]
# Enables explicit /worker, /spawn, /route, and /delegate message actions.
worker_delegation_enabled = true
default_worker_role = "Worker"
```

An example file is available at `config/cadis.example.toml`.

## 3. Profile Home Layout

The store crate provides `CadisHome` and `ProfileHome` helpers for daemon-first
profile state:

```text
~/.cadis/profiles/<profile>/
|-- profile.toml
|-- .gitignore
|-- agents/
|-- memory/
|-- skills/
|-- workspaces/
|   |-- registry.toml
|   |-- aliases.toml
|   `-- grants.jsonl
|-- workers/
|-- sessions/
|-- artifacts/
|-- checkpoints/
|-- sandboxes/
|-- eventlog/
|-- channels/
|-- secrets/
|-- logs/
|-- locks/
`-- run/
```

`profile.toml` is initialized from a safe template. `.gitignore` excludes
secrets, channel state, sessions, workers, checkpoints, sandboxes, logs, locks,
runtime files, private keys, tokens, and SQLite sidecar files. Template
initialization is non-destructive: existing profile files are not overwritten.

The profile home is not an execution workspace and is not a sandbox. Project
roots must be registered separately in the workspace registry before
profile-aware tool/runtime code grants file, shell, git, or worker access.

## 4. Workspace Registry

Profile workspace metadata lives at:

```text
~/.cadis/profiles/<profile>/workspaces/registry.toml
```

The store crate loads and writes this TOML shape:

```toml
[[workspace]]
id = "example-project"
kind = "project"
root = "~/Project/example-project"
vcs = "git"
owner = "rama"
trusted = true
worktree_root = ".cadis/worktrees"
artifact_root = ".cadis/artifacts"
checkpoint_policy = "enabled"

[[workspace.alias]]
workspace_id = "example-project"
aliases = ["example", "demo"]
```

Supported `kind` values are `project`, `documents`, `sandbox`, and `worktree`.
Supported `vcs` values are `git` and `none`. Supported `checkpoint_policy`
values are `enabled` and `disabled`. Helper APIs expand `~` in workspace roots
after loading and write registry updates atomically with redaction applied to
serialized TOML.

Workspace grants are persisted as redacted JSONL. Grant creation appends one
record; revocation rewrites the active grant set so stale grants do not revive
after daemon restart:

```text
~/.cadis/profiles/<profile>/workspaces/grants.jsonl
```

Each grant records `grant_id`, `profile_id`, optional `agent_id`,
`workspace_id`, `root`, `access`, `created_at`, optional `expires_at`, `source`,
and optional redacted `reason`. Supported access values are `read`, `write`,
`exec`, and `admin`. Supported source values are `route`, `user`, `policy`, and
`worker_spawn`.

These store helpers only persist metadata. Enforcement remains daemon/runtime
owned: file, shell, git, and worker tools must resolve an active grant before
using a project root.

## 5. Native Avatar Config Contract

`crates/cadis-avatar` defines the Rust config contract for the future native
Wulan renderer. These keys are documented now for privacy review and are not yet
read by the desktop MVP config loader:

```toml
[avatar]
renderer = "wgpu_native" # "headless", "wgpu_native", "bevy_scene"
renderer_fallback = "orb" # "orb", "static_wulan_texture"
reduced_motion = false
max_delta_ms = 250

[avatar.face_tracking]
mode = "off" # "off", "permission_required", "local_only"
permission_required = true
camera_indicator_required = true
one_click_disable_required = true
min_confidence_percent = 35

[avatar.privacy]
local_only_face_tracking = true
persist_raw_face_frames = false
persist_face_landmarks = false
allow_remote_face_tracking = false
allow_face_identity = false
```

Rules:

- Face tracking is off by default.
- The current `wgpu-renderer` crate feature is a native adapter spike that builds
  render plans from `AvatarFrame`; the desktop MVP config loader does not yet
  instantiate a GPU surface from these keys.
- Native Wulan renderer failure falls back to the CADIS orb by default and must
  not block HUD launch.
- Enabling face tracking requires explicit permission, a visible camera-active
  indicator, and a one-click disable action.
- Face tracking data must stay local and must not be persisted by default.
- Identity recognition, matching, embeddings, or biometric templates require a
  separate security and privacy decision before any config key can enable them.

## 6. Agent Spawn Limits

`[agent_spawn]` configures the daemon baseline for request-driven
`agent.spawn` and explicit orchestrator worker-spawn actions:

- `max_depth`: maximum child depth below a root agent. A child of `main` is depth 1.
- `max_children_per_parent`: maximum direct children under one parent.
- `max_total_agents`: maximum registered agents, including built-in agents.

The desktop MVP defaults are conservative: depth 2, 4 children per parent, and
32 total agents. Implicit model-driven recursive spawning remains reserved for a
later runtime track.

## 7. Orchestrator Routing

`[orchestrator]` configures daemon-owned message routing behavior. This is not a
HUD setting.

- `worker_delegation_enabled`: enables explicit `/worker`, `/spawn`, `/route`, and `/delegate` message actions handled by `cadisd`.
- `default_worker_role`: role used by `/worker <task>` when no `Role: task` prefix is supplied.

Direct `@agent` mention targeting remains enabled independently of this flag.
Explicit worker spawn actions still use the `[agent_spawn]` depth, child, and
total-agent limits.

## 8. Model Provider Behavior

- `auto`: tries Ollama at `ollama_endpoint`, then falls back to the local echo provider.
- `codex-cli`: delegates to the installed official Codex CLI with `codex exec`.
  Authenticate the CLI separately with `codex login` for ChatGPT Plus/Pro access.
  CADIS does not read, copy, or persist `~/.codex/auth.json`.
- `openai`: sends chat requests to the OpenAI Chat Completions API. It requires
  `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` in the daemon environment.
- `ollama`: requires a running Ollama server and returns an error event if unavailable.
- `echo`: uses the credential-free local fallback.

Per-agent model selections set through `agent.model.set` are routed by `cadisd`
when they include a supported provider prefix:

```text
auto/llama3.2
ollama/llama3.2
openai/gpt-5.2
codex-cli/gpt-5.4
echo/cadis-local-fallback
```

Plain provider names such as `echo` or `ollama` select that provider with the
configured model. Plain model names select the configured default provider with
that model where the provider supports model overrides. Message events include
the effective provider and model used for the response.

The `models.list` protocol response exposes conservative readiness metadata for
clients. `readiness = "fallback"` and `fallback = true` identify entries such as
`echo` or fallback-capable `auto` behavior so clients can distinguish a real
provider from the local fallback. Providers that need a daemon, login, API key,
or local service are reported as `requires_configuration` until CADIS has active
provider probing.

The OpenAI API key is not a config key. Do not put API keys, bearer tokens, or
auth headers in `~/.cadis/config.toml`, examples, or logs.

## 9. Environment Variables

```text
CADIS_HOME
CADIS_LOG_LEVEL
CADIS_MODEL_PROVIDER
CADIS_SOCKET
CADIS_HUD_SOCKET
VITE_CADIS_SOCKET_PATH
OPENAI_API_KEY
CADIS_OPENAI_API_KEY
CODEX_API_KEY
CADIS_CODEX_BIN
CADIS_CODEX_MODEL
CADIS_CODEX_EXTRA_ARGS
CADIS_WHISPER_CLI
WHISPER_CLI
CADIS_WHISPER_MODEL
WHISPER_MODEL
CADIS_WHISPER_LANGUAGE
WHISPER_LANGUAGE
CADIS_HUD_NODE
XDG_RUNTIME_DIR
TELEGRAM_BOT_TOKEN
```

The desktop MVP reads `CADIS_HOME`, `CADIS_LOG_LEVEL`, `CADIS_MODEL_PROVIDER`,
`CADIS_OPENAI_API_KEY`, `OPENAI_API_KEY`, `CADIS_CODEX_BIN`,
`CADIS_CODEX_MODEL`, and `CADIS_CODEX_EXTRA_ARGS`.

Socket discovery uses `CADIS_HUD_SOCKET` for the Tauri HUD, `CADIS_SOCKET` as a
shared client override, and `XDG_RUNTIME_DIR` for the default daemon socket
under `$XDG_RUNTIME_DIR/cadis/cadisd.sock`. `VITE_CADIS_SOCKET_PATH` is a
development-only renderer seed for the browser preview; daemon state remains
authoritative.

HUD voice input reads `CADIS_WHISPER_CLI`, `WHISPER_CLI`,
`CADIS_WHISPER_MODEL`, `WHISPER_MODEL`, `CADIS_WHISPER_LANGUAGE`, and
`WHISPER_LANGUAGE`. `CADIS_HUD_NODE` can point the Tauri side at a specific
Node.js binary for local voice helper execution.

`CODEX_API_KEY` is consumed by the official Codex CLI when that CLI is
configured for API-key auth; CADIS does not read it directly. `TELEGRAM_BOT_TOKEN`
is reserved for the planned Telegram adapter and must stay empty in examples
until that adapter exists.

## 10. Secret Rules

- Do not store raw API keys in committed files.
- Do not commit `~/.codex/auth.json`; treat it as a password-equivalent file.
- Prefer `~/.cadis/config.toml` for local runtime config and keep provider keys in environment variables or a future OS keychain integration.
- Do not write resolved secrets to logs.
- Event logs pass through CADIS redaction before JSONL persistence.
- Workspace registry writes and grant JSONL appends also pass through CADIS
  redaction before persistence.
- Redact values from keys containing `api_key`, `token`, `secret`, or `authorization`.
- Do not commit `.env`, generated auth JSON, HAR traces, crash dumps, logs, local sockets, or Tauri bundle output.

## 11. Durable State Files

The store crate provides atomic JSON helpers for these exact durable files:

```text
~/.cadis/state/sessions/<session-id>.json
~/.cadis/state/agents/<agent-id>.json
~/.cadis/state/workers/<worker-id>.json
~/.cadis/state/approvals/<approval-id>.json
```

Path components are redaction-safe: only ASCII letters, digits, `-`, and `_`
are preserved, and every other character is replaced with `_`. Empty IDs become
`unnamed`.

State writes use a private temporary file in the same directory:

```text
~/.cadis/state/<kind>/.<safe-id>.json.tmp.<pid>.<counter>.<nanos>
```

The store writes redacted pretty JSON, syncs the temporary file, renames it over
the target `.json`, and syncs the parent directory. Recovery only reads final
`.json` files. Partial temp files are ignored, and corrupt final JSON files are
skipped with diagnostics instead of becoming trusted runtime state.
