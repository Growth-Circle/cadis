# C.A.D.I.S.

**Coordinated Agentic Distributed Intelligence System**

Local-first multi-agent runtime for desktop work, native tools, approvals, voice, and isolated coding workflows.

## Install

```bash
npm install -g cadis
```

## Usage

```bash
# Start the daemon
cadisd

# Use the CLI
cadis status
cadis doctor
cadis models
cadis agents
cadis chat "hello"
```

Windows users: the daemon defaults to TCP transport (`127.0.0.1:7433`).

## Supported platforms

- Linux x64
- Linux arm64
- macOS x64 (Intel)
- macOS arm64 (Apple Silicon)
- Windows x64

## Documentation

See https://github.com/Growth-Circle/cadis for full documentation.

## License

Apache-2.0
