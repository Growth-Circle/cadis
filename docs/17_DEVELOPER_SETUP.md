# Developer Setup

## 1. Status

CADIS is currently a planning baseline. Runtime crates will be added in the implementation phase.

## 2. Requirements

Planned requirements:

- Rust stable
- Git
- Linux desktop for first target
- Optional model provider credentials
- Optional Ollama for local model testing
- Optional Telegram bot token for adapter testing

## 3. Clone

```bash
git clone https://github.com/cadis-ai/cadis.git
cd cadis
```

## 4. Expected Commands After P1

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## 5. Expected Commands After P4

```bash
cargo run -p cadis-daemon -- --check
cargo run -p cadis-cli -- status
cargo run -p cadis-cli -- chat "hello"
```

## 6. Development Rules

- Add a crate only when it has a clear responsibility.
- Keep protocol types stable and tested.
- Keep core code independent from UI frameworks.
- Keep provider integrations modular.
- Add security tests before expanding risky tool behavior.

