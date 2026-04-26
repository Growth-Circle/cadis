---
name: cadis-model-provider
description: Use when implementing or reviewing CADIS model provider traits, provider capabilities, streaming responses, tool-call support, OpenAI, Ollama, Anthropic, Gemini, OpenRouter, LM Studio, or custom HTTP providers.
---

# CADIS Model Provider

## Read First

- `docs/01_PRD.md`
- `docs/04_TRD.md`
- `docs/06_IMPLEMENTATION_PLAN.md`

## Rules

- Keep model providers behind a shared trait.
- Expose capabilities.
- Support streaming and cancellation.
- Map provider errors into CADIS errors.
- Do not leak API keys into events or logs.
- Do not hardcode one provider into core orchestration.

## Capabilities

```text
tool_calling
vision
json_schema
reasoning_effort
prompt_cache
max_context_tokens
local_model
streaming
```

## Provider Order

Prove one cloud provider and one local provider early. Good defaults are OpenAI and Ollama.

For OpenAI-specific work, also use the `openai-docs` system skill.

