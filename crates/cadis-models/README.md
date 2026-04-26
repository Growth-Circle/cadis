# cadis-models

Model provider adapters for the CADIS runtime.

The desktop MVP includes OpenAI, Ollama, and official Codex CLI adapters plus a
local fallback provider that requires no credentials. The Codex CLI adapter uses
`codex exec`; authenticate separately with the official CLI instead of storing
ChatGPT tokens in CADIS.
