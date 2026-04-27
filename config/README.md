# Config

Example CADIS configuration files live here.

Runtime user configuration defaults to:

```text
~/.cadis/config.toml
```

Do not commit real provider keys, Telegram tokens, or local private paths.

Voice config is daemon-owned. Supported visible TTS provider IDs are `edge`,
`openai`, and `system`; `stub` is reserved for deterministic tests. Current
provider implementations are local stubs and must not include API keys.

The desktop MVP example is:

- [cadis.example.toml](cadis.example.toml)
