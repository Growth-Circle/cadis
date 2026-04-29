# C.A.D.I.S. Wiki

**Coordinated Agentic Distributed Intelligence System**

C.A.D.I.S. is a local-first, Rust-first, model-agnostic runtime for coordinating AI agents across a desktop HUD, CLI, voice, approvals, native tools, and isolated coding workflows.

The daemon (`cadisd`) is the runtime authority. Every interface — CLI, HUD, voice, or future clients — is a protocol client.

```
HUD / CLI / Voice / Telegram / Android
                |
              cadisd
                |
     agents, models, tools, policy, store
```

## Quick links

- [[Getting Started]] — Install, build, run the daemon, first chat, launch the HUD
- [[Configuration]] — `~/.cadis/config.toml`, model providers, voice, workspaces
- [[FAQ]] — Common questions about design, models, approvals, and platform support
- [[Troubleshooting]] — Fixes for daemon, CLI, HUD, voice, and model issues

## Key design principles

- **Local-first**: state, logs, approvals, and orchestration live on your machine.
- **Rust-first core**: critical runtime paths stay in Rust.
- **Model-agnostic**: Ollama, OpenAI API, Codex CLI, or local fallback.
- **Policy-gated**: risky actions pass centralized approval and audit paths.
- **Interface-agnostic**: the daemon owns behavior; clients render and control it.

## Current status

C.A.D.I.S. is in **beta** (v0.9). Linux desktop is the primary target. macOS has source-validation CI. Windows is limited to portable-crate validation.

## Resources

- [GitHub Repository](https://github.com/Growth-Circle/cadis)
- [Architecture](https://github.com/Growth-Circle/cadis/blob/main/docs/05_ARCHITECTURE.md)
- [Protocol Draft](https://github.com/Growth-Circle/cadis/blob/main/docs/15_PROTOCOL_DRAFT.md)
- [Master Checklist](https://github.com/Growth-Circle/cadis/blob/main/docs/07_MASTER_CHECKLIST.md)
