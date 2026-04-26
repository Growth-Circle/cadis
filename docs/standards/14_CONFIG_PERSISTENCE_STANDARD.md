# Config and Persistence Standard

## 1. Purpose

This standard defines CADIS configuration, local state, persistence, migration, and redaction rules.

`cadisd` owns durable state. Clients may cache view state, but they are not authoritative.

## 2. CADIS Home

Default home:

```text
~/.cadis/
```

Default layout:

```text
~/.cadis/
|-- config.toml
|-- sessions/
|-- workers/
|-- logs/
|-- worktrees/
|-- approvals.json
`-- tokens/
```

Rules:

- `CADIS_HOME` may override the default home.
- Paths in config may use `~`, but internal code should normalize before use.
- Runtime-created files should use restrictive permissions where possible.
- Worktrees live under CADIS home unless explicitly configured otherwise.

## 3. Config Format

- User config is TOML.
- Default path is `~/.cadis/config.toml`.
- Config examples must not contain raw secrets.
- Unknown fields should be rejected or warned consistently by config version policy.
- Environment variables may override selected fields documented in `docs/16_CONFIG_REFERENCE.md`.

Required config areas:

- daemon
- transport
- agents
- policy
- models
- Telegram
- HUD
- voice
- agent display names
- agent model selections

## 4. Secret Configuration

- Provider keys are referenced through environment variable names such as `OPENAI_API_KEY`.
- Telegram token is referenced through `TELEGRAM_BOT_TOKEN`.
- Raw keys must not be written to config examples, logs, diagnostics, protocol events, or tests.
- Future keychain support must preserve the same no-log rule.
- Redaction applies before serialization for logs and diagnostic output.

## 5. Policy Configuration

Allowed policy values:

```text
allow
ask
deny
```

Rules:

- Destructive, external, privileged, or secret-reading actions default to conservative behavior.
- Invalid policy values fail config validation.
- Policy config changes must not bypass runtime classification.
- Reloaded policy must be applied atomically.

## 6. Durable UI Preferences

HUD and voice preferences are daemon-backed config/state, not browser-local authority.

Examples:

- HUD theme
- background opacity
- hotkey
- always-on-top
- chat thinking and fast preferences
- voice enabled state
- voice provider, ID, rate, pitch, volume, auto-speak, and maximum spoken characters
- agent display names
- per-agent model selections

Rules:

- Accepted changes emit `ui.preferences.updated`, `agent.renamed`, or model-related events.
- UI may cache preferences only for rendering.
- Rename and model selection must survive HUD restart.

## 7. Event Logs

- Event logs are JSONL.
- Session logs and worker logs are separate.
- Logs include event IDs and session IDs where applicable.
- Tool lifecycle logs include tool call IDs.
- Approval logs include approval IDs and final resolution.
- Logs must be redacted before write.
- Debug mode may increase detail but must still redact secrets.

## 8. Atomic Writes

State files must be written atomically:

```text
write temporary file -> flush -> rename into place
```

Rules:

- Partial writes must not corrupt authoritative state.
- Recovery should ignore or quarantine incomplete temporary files.
- Append-only JSONL logs may use append semantics, but state snapshots require atomic replacement.
- Tests should cover interrupted or malformed state where practical.

## 9. Crash Recovery

CADIS should persist enough metadata to recover or explain:

- incomplete sessions
- pending or resolved approvals
- worker records
- worktree paths
- recent daemon events
- config version

Rules:

- Recovery must fail closed for uncertain approval state.
- Recovered state should emit events or snapshots so clients can rebuild view state.
- Corrupt state must produce actionable diagnostics without leaking secrets.

## 10. Migrations

- Storage format changes require an ADR.
- Config and state files should carry a version once migrations begin.
- Migrations must be idempotent or detect already-migrated state.
- Backup or rollback strategy is required for destructive migrations.
- Migration logs must be redacted.

## 11. Validation

Config and persistence changes require tests for:

- TOML parsing
- defaults
- environment overrides
- invalid values
- redaction
- atomic writes
- JSONL append behavior
- approval persistence
- UI preference persistence when applicable
