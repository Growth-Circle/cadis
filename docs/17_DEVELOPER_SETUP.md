# Developer Setup

## 1. Status

CADIS now has a desktop MVP runtime:

- `cadisd` local daemon over a Unix socket.
- `cadis` CLI client.
- `cadis status`, `cadis doctor`, `cadis models`, and `cadis chat`.
- JSONL event logs under `~/.cadis/logs`.
- Redaction before event persistence.
- Ollama optional model adapter with local fallback.
- OpenAI optional model adapter using env-only API keys.
- Official Codex CLI optional adapter using `codex exec`.
- Native `cadis-hud` desktop prototype.

Tools, approval-gated execution, workers, Telegram, production voice output, and full HUD parity are still planned work.

## 2. Requirements

- Rust stable with `rustfmt` and `clippy`.
- Git.
- Linux desktop for the first target.
- Optional Ollama for real local model responses.
- Optional `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` for OpenAI responses.
- Optional official Codex CLI for ChatGPT Plus/Pro-backed Codex responses.

Cloud provider credentials are not required unless `[model].provider = "openai"`.
ChatGPT-plan auth is handled by the official Codex CLI, not by CADIS.

## 3. Clone

```bash
git clone https://github.com/cadis-ai/cadis.git
cd cadis
```

## 4. Build And Validate

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## 5. Daemon Commands

```bash
cargo run -p cadis-daemon -- --version
cargo run -p cadis-daemon -- --check
cargo run -p cadis-daemon --
```

Use a temporary socket for isolated testing:

```bash
cargo run -p cadis-daemon -- --socket /tmp/cadis-test.sock --dev-echo
```

## 6. CLI Commands

In another terminal:

```bash
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock status
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock doctor
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock models
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock chat "hello"
```

JSON frame output:

```bash
cargo run -p cadis-cli -- --socket /tmp/cadis-test.sock --json chat "hello"
```

## 7. Run Desktop HUD

Start `cadisd` first:

```bash
cargo run -p cadis-daemon -- --socket /tmp/cadis-hud.sock --dev-echo
```

In another terminal:

```bash
cargo run -p cadis-hud -- --socket /tmp/cadis-hud.sock
```

Equivalent env-based launch:

```bash
CADIS_HUD_SOCKET=/tmp/cadis-hud.sock cargo run -p cadis-hud
```

The HUD is a protocol client. It connects to `cadisd` over the Unix socket and
does not own durable runtime state or credentials.

## 8. Optional Ollama Setup

Install and start Ollama, then pull a model:

```bash
ollama pull llama3.2
```

Create `~/.cadis/config.toml`:

```toml
[model]
provider = "ollama"
ollama_model = "llama3.2"
ollama_endpoint = "http://127.0.0.1:11434"
```

Use `provider = "auto"` to fall back to the local credential-free provider when
Ollama is not running.

## 9. Optional OpenAI Setup

Keep the API key in the daemon environment:

```bash
export CADIS_OPENAI_API_KEY="..."
```

Create or update `~/.cadis/config.toml`:

```toml
[model]
provider = "openai"
openai_model = "gpt-5.2"
openai_base_url = "https://api.openai.com/v1"
```

Do not store the API key in `config.toml`.

## 10. Development Rules

## 10. Optional Codex CLI Setup

Install and authenticate the official Codex CLI:

```bash
npm i -g @openai/codex
codex login
```

Then create or update `~/.cadis/config.toml`:

```toml
[model]
provider = "codex-cli"
```

CADIS calls `codex exec` in read-only ephemeral mode. It does not fork Codex
CLI and does not read or persist `~/.codex/auth.json`.

## 11. Development Rules

- Add a crate only when it has a clear responsibility.
- Keep protocol types stable and tested.
- Keep core code independent from UI frameworks.
- Keep provider integrations modular.
- Add security tests before expanding risky tool behavior.
- Do not commit real provider keys, Telegram tokens, JSONL logs, diagnostics, or local CADIS state.
