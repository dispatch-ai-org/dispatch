## Experimental Developer Preview

Dispatch v0.1.2 makes agent selection the normal local workflow: tell Dispatch the task, review the proposed change, and explicitly accept or reject it.

### Task-first workflow

- Run `dispatch run "Fix the retry race"` from your project; no init, dataset import, Cloud account, or routing flag is required.
- Use `dispatch explain`, `dispatch diff`, `dispatch accept`, and `dispatch reject` without copying run IDs.
- Deliberately override agent choice with `--agent codex` or `--agent cursor`.

### Honest, local selection

- A bundled 508-byte public-prior snapshot supports offline cold start. Optional `dispatch data refresh` retrieves compact normalized public data without authentication or raw benchmark downloads.
- Evidence-based selection requires compatible, nonzero evidence for at least two execution-eligible agents. Missing evidence remains unknown, not zero performance.
- Otherwise Dispatch uses its deterministic default order. A sole eligible agent is labeled "Only available agent."
- The current bundled snapshot contains Codex-only evidence, so it does not establish comparative superiority over Cursor or Claude Code.
- Existing isolated candidate execution, configured verification, and explicit safe apply remain the execution path. Normal runs never require Cloud or fetch benchmark data.
- Prediction, mechanical outcome, and explicit human acceptance remain separate. Local evidence inspection does not influence routing; Cloud contribution remains explicit and opt-in.

### Experimental / limitations

- `--allow-unsafe-local` permits host execution; it is not a security sandbox.
- Install and authenticate a current supported coding-agent CLI. The final hosted smoke test used Codex CLI 0.153.4.
- Verification runs only when configured; a completed agent process is not proof of task quality.
- The Docker execution core exists, but turnkey Cursor/Codex images and container-compatible authentication are not provided.
- Advanced multi-candidate comparison and explicit dataset imports remain available outside the primary workflow.
- macOS archives are unsigned and not notarized.
