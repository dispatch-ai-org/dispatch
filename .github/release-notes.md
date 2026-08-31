## Experimental Developer Preview

Dispatch v0.1.1 is a local-first patch release that points optional evaluation sync at the production Dispatch Cloud endpoint and makes the verified first-run path explicit.

### Fixed

- Use `https://api.rundispatch.sh` as the default Cloud base URL while preserving development and loopback overrides.
- Clarify that `dispatch sync enable` records consent and prepares data locally but does not upload it.
- Prompt new projects to configure a real verification command before their first evaluation.

### Supported

- Local backend with Cursor Agent and Codex CLI
- Local Git and non-Git source directories
- Independent candidate workspaces and concurrent execution
- Baseline and candidate verification
- Blind comparison and durable human evaluation
- SQLite and filesystem persistence
- Explicit opt-in evaluation sync to Dispatch Cloud

### Experimental / limitations

- `--allow-unsafe-local` permits host execution; it is not a security sandbox.
- The Docker execution core exists, but turnkey Cursor/Codex images and container-compatible authentication are not provided.
- Dispatch does not automatically route work or choose a winner.
- Dispatch Cloud is optional; normal local runs require no account or hosted service.
- macOS archives are unsigned and not notarized.
