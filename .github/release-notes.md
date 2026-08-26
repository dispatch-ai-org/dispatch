## Experimental Developer Preview

Dispatch v0.1.0 is an early local-first release for developers who want to run the same software task through multiple coding-agent harnesses and evaluate the resulting evidence.

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
