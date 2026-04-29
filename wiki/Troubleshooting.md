# Troubleshooting

## Daemon won't start

**Symptom**: `cadisd` exits immediately or prints an error on startup.

- Run `cadisd --check` to validate config before starting.
- Check `~/.cadis/config.toml` for TOML syntax errors.
- Ensure the socket directory exists and is writable. The default is `$XDG_RUNTIME_DIR/cadis/` or `~/.cadis/run/`.
- Check if another `cadisd` instance is already running: `pgrep cadisd`. Kill the stale process or remove the leftover socket file.
- Check logs at `~/.cadis/logs/daemon.jsonl` for error details.

## CLI can't connect

**Symptom**: `cadis status` returns a connection error.

- Verify `cadisd` is running: `pgrep cadisd`.
- Ensure the CLI and daemon use the same socket. Override with `--socket`:
  ```bash
  cadis --socket /tmp/cadis-test.sock status
  ```
- If using a custom `CADIS_SOCKET`, ensure it matches the daemon's socket path.
- Run `cadis doctor` for a full environment check.

## HUD shows disconnected

**Symptom**: The Tauri HUD launches but shows a disconnected state.

- Ensure `cadisd` is running before launching the HUD.
- The HUD resolves the socket from (in order): `CADIS_HUD_SOCKET`, `CADIS_SOCKET`, `~/.cadis/config.toml`, `$XDG_RUNTIME_DIR/cadis/cadisd.sock`, `~/.cadis/run/cadisd.sock`.
- For development, pass the socket explicitly:
  ```bash
  CADIS_HUD_SOCKET=/tmp/cadis-hud.sock pnpm tauri:dev
  ```
- Check the browser devtools console (F12) for WebSocket errors.
- Ensure Tauri system dependencies are installed (WebKitGTK 4.1, etc.).

## Voice not working

**Symptom**: HUD voice input or TTS output doesn't work.

- Use the HUD Settings → Voice tab doctor to check all dependencies.
- For mic input, ensure `whisper-cli` is installed and the model file exists:
  ```bash
  export CADIS_WHISPER_CLI="$HOME/.local/bin/whisper-cli"
  export CADIS_WHISPER_MODEL="$HOME/.local/share/cadis/whisper-models/ggml-base.bin"
  ```
- On Linux, check desktop portal mic permissions — allow mic access for C.A.D.I.S. in system settings.
- For TTS output, ensure an audio player is available on the system.
- Verify `[voice].enabled = true` in `config.toml`.

## Model errors

**Symptom**: `cadis chat` returns an error instead of a model response.

**Ollama issues**:
- Ensure Ollama is running: `curl http://127.0.0.1:11434/api/tags`
- Ensure the model is pulled: `ollama pull llama3.2`
- Check `ollama_endpoint` in config matches the running Ollama instance.

**OpenAI issues**:
- Ensure `CADIS_OPENAI_API_KEY` or `OPENAI_API_KEY` is set in the daemon environment (not in `config.toml`).
- Check for `model_auth_missing` or `model_auth_failed` error codes in the response.
- Verify `openai_model` is a valid model name.

**Codex CLI issues**:
- Ensure the Codex CLI is installed: `codex --version`
- Re-authenticate: `codex login`
- C.A.D.I.S. does not read `~/.codex/auth.json` directly — the Codex CLI must be independently authenticated.

**General**:
- Run `cadis models` to see which providers are ready vs. `requires_configuration`.
- Use `provider = "auto"` to fall back gracefully when a provider is unavailable.
- Check `cadis doctor` for environment diagnostics.
