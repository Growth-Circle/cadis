# CADIS Model Provider Standard

## 1. Purpose

This standard defines the contract for model providers in CADIS. Providers supply model output, but they do not own sessions, policy, tools, approvals, persistence, or UI state.

## 2. Provider Boundary

The daemon calls model providers through a common `ModelProvider` abstraction.

Provider implementations are responsible for:

- request translation
- streaming response handling
- provider error mapping
- capability metadata
- authentication lookup
- cancellation support where available

Provider implementations must not:

- execute tools directly
- approve actions
- persist raw secrets in event logs
- mutate daemon state outside provider result APIs
- require UI-specific logic

## 3. Initial Providers

CADIS should support these providers according to the implementation plan:

| Provider | Priority |
| --- | --- |
| OpenAI | P1 |
| Ollama | P1 |
| Anthropic | P2 |
| Gemini | P2 |
| OpenRouter | P2 |
| Custom HTTP | P2 |

One provider is sufficient to prove the first streaming path, but the abstraction must not hard-code one vendor.

## 4. Capability Metadata

Each provider and model must expose capability metadata.

Recommended fields:

- provider ID
- model ID
- display name
- streaming support
- tool-call support
- JSON/schema support
- vision support, if applicable
- context window, if known
- local or remote execution
- cost metadata, if configured
- rate-limit hints, if known

The HUD and CLI may display capability data, but the daemon remains authoritative.

## 5. Configuration

Provider config belongs in `~/.cadis/config.toml` with environment variable overrides where appropriate.

Rules:

- Do not store raw provider keys in event logs.
- Prefer environment variables for secrets.
- Config parsing must produce actionable errors.
- Missing optional providers must not prevent daemon startup.
- `cadis doctor` should report provider readiness without exposing secrets.

## 6. Streaming Contract

Providers must emit normalized model events:

```text
model.started
model.delta
model.completed
model.failed
model.cancelled
```

The daemon maps provider events into CADIS session events such as `message.delta` and `message.completed`.

Streaming rules:

- Preserve token order.
- Support cancellation where the upstream API permits it.
- Convert provider-specific stop reasons to CADIS stop metadata.
- Bound memory usage for long streams.
- Surface partial output carefully when errors occur.

## 7. Error Mapping

Provider errors must become structured CADIS errors.

Required categories:

- authentication failure
- authorization failure
- missing model
- invalid request
- rate limited
- timeout
- network failure
- provider unavailable
- malformed provider response
- cancelled

Errors must include actionable metadata but must not leak credentials, headers, or full sensitive request bodies.

## 8. Model Catalog

The daemon must support `models.list` for UI and CLI clients.

Catalog behavior:

- include configured providers and models
- identify the default model
- include per-agent model assignments
- preserve current configured values even if a model is temporarily missing
- make provider unavailability visible without deleting configuration

## 9. Provider Conformance

Every provider must pass conformance tests before being marked supported.

Required conformance tests:

- streams deltas in order
- emits completed event
- maps authentication error
- maps rate limit error
- maps malformed response
- supports cancellation or reports unsupported clearly
- redacts credentials in logs
- exposes capability metadata
- handles provider timeout

Live provider tests must be opt-in and skipped by default in CI.

## 10. Custom HTTP Provider

The custom HTTP provider is for advanced users and integration testing.

Requirements:

- explicit endpoint configuration
- explicit auth header configuration through secret references
- strict response schema
- timeout
- cancellation if supported by transport
- no arbitrary code execution

Custom HTTP provider errors must use the same error mapping as built-in providers.

## 11. UI Requirements

HUD model UI must:

- list available models from the daemon
- show the default model
- support per-agent model selection
- preserve the selected value if absent from the current catalog
- send updates through `agent.model.set`
- update the central orb meta ring after daemon confirmation

The UI must not fetch model catalogs directly from vendors.

## 12. Testing Requirements

Required tests:

- config parsing for each provider
- missing secret handling
- `models.list` response shape
- default model selection
- per-agent model assignment
- mock streaming success
- cancellation
- provider error mapping
- redaction of provider keys
- HUD catalog mapping
