# Dispatch

**Dispatch v0.1.2 — Experimental Developer Preview**

Tell Dispatch what you want changed. Dispatch chooses an available coding agent using observed performance data, runs it in an isolated candidate workspace, verifies the result when configured, and lets you review and accept it.

```text
task
  ↓
available coding agents + local public performance evidence
  ↓
one selected agent
  ↓
isolated execution + configured verification
  ↓
review → accept or reject
```

Dispatch works locally and offline. A release contains a compact public-evidence snapshot, normal runs never fetch benchmark data, and Dispatch Cloud is optional.

## Quick start

Install and authenticate at least one supported coding-agent CLI: Claude Code, Codex CLI, or Cursor Agent. Then, from a repository:

```bash
cd my-project
dispatch run "Fix the retry race"
```

Local agents run with your operating-system permissions. Dispatch asks for confirmation before execution; scripts can use `--allow-unsafe-local` after accepting that risk.

Review and apply the result without copying a run ID:

```bash
dispatch diff
dispatch accept
```

Or reject it without changing the source tree:

```bash
dispatch reject
```

Dispatch uses benchmark performance to choose only when at least two execution-eligible agents have compatible, nonzero evidence. It compares that evidenced subset using the existing Router; agents without evidence remain unknown, not inferior. Otherwise it uses the first available agent in the fixed order Claude Code → Codex → Cursor, independently of benchmark availability. A sole eligible agent is labeled “Only available agent.”

To deliberately override Dispatch's choice:

```bash
dispatch run "Fix the retry race" --agent cursor
```

To inspect the last choice in more detail:

```bash
dispatch explain
```

No `init`, `doctor`, dataset import, Cloud account, or routing flag is required for this workflow.

## What Dispatch reports

An evidence-based selection is described as observed benchmark performance, not as confidence or a probability of success. For example, with two compatible fixture results:

```text
Agent
  Cursor
Selection
  Evidence-based
Why:
  Cursor had the strongest relevant observed benchmark performance
  among eligible agents with compatible evidence.
Available benchmark evidence:
  Cursor: 158/330
  Codex: 123/330
```

The current bundled snapshot contains only Codex evidence. On a fresh installation with Codex and Cursor available, that is not comparative evidence; the fixed default selects Codex:

```text
Agent
  Codex
Selection
  Dispatch default
Why:
  Dispatch did not have enough comparable public performance data
  to make an evidence-based choice.
  Available benchmark evidence did not determine the selection.
Available benchmark evidence:
  Codex: 123/330
  Cursor: no compatible public evidence
```

After execution, Dispatch leads with the task, selected agent, selection basis, verification result, and next action. Mechanical verification, human acceptance, and the routing prediction remain separate facts. A passing check never silently accepts or applies a result.

## Install

Prebuilt archives are available on the [latest GitHub Release](https://github.com/dispatch-ai-org/dispatch/releases/latest).

### macOS Apple Silicon

The macOS archive is unsigned and not notarized.

```bash
curl -LO https://github.com/dispatch-ai-org/dispatch/releases/latest/download/dispatch-macos-arm64.tar.gz
curl -LO https://github.com/dispatch-ai-org/dispatch/releases/latest/download/SHA256SUMS
grep 'dispatch-macos-arm64.tar.gz' SHA256SUMS | shasum -a 256 -c -
tar -xzf dispatch-macos-arm64.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 0755 dispatch "$HOME/.local/bin/dispatch"
"$HOME/.local/bin/dispatch" version
```

### Linux x86_64

The Linux archive targets glibc-based x86_64 systems; it is not a static musl build.

```bash
curl -LO https://github.com/dispatch-ai-org/dispatch/releases/latest/download/dispatch-linux-x86_64.tar.gz
curl -LO https://github.com/dispatch-ai-org/dispatch/releases/latest/download/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf dispatch-linux-x86_64.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 0755 dispatch "$HOME/.local/bin/dispatch"
"$HOME/.local/bin/dispatch" version
```

Ensure `$HOME/.local/bin` is on `PATH`. Dispatch also requires Git and `curl` at runtime.

### Install from source

With a current stable Rust toolchain:

```bash
cargo install --git https://github.com/dispatch-ai-org/dispatch --locked
dispatch version
```

Dispatch is not published to crates.io.

## Safety and source behavior

> **Local execution is not a security sandbox.**

Real agents and project checks run with the permissions of the Dispatch process and receive `HOME` so installed harnesses can use local authentication. Dispatch clears most other child environment variables. Configured extra variables require the separate `--allow-forwarded-env` acknowledgement and their values are redacted from persisted logs.

Dispatch freezes the source into an internal Git baseline and gives the selected agent an independent candidate workspace. It does not run the agent directly in the original tree. `dispatch accept` uses the existing safe apply path and rejects source drift; `dispatch reject` never applies candidate changes.

If verification is configured, the same commands run against the candidate and their output is retained. Without configured checks, Dispatch reports `Verification: Not configured`. Verification is mechanical evidence, not a universal code-quality judgment.

The local backend is the supported real-agent path in v0.1.2. Docker execution is advanced and experimental: users must provide a suitable image containing the agent and project toolchain.

## Core commands

```text
dispatch run "<task>" [--source path] [--agent claude|codex|cursor]
dispatch status [run-id]
dispatch diff [run-id] [candidate]
dispatch accept [run-id]
dispatch reject [run-id]
dispatch explain [run-id]
dispatch history [--limit count]
dispatch version
```

Without a run ID, `status`, `diff`, `accept`, `reject`, and `explain` resolve the latest relevant single-result run for the current source tree. Explicit run IDs remain available for history and debugging. `--task-file path|-` reads a task from a file or stdin, and `--source path` overrides the current directory.

## Public routing data

Dispatch releases embed `PublicPriorSnapshotV1`, a small normalized snapshot containing only the evidence the Router needs: benchmark provenance, harness and model identity, task morphology, successes, and attempts. It contains no raw benchmark tasks, source code, trajectories, patches, logs, or local Dispatch observations.

On first routing use, the bundled snapshot is installed idempotently into the existing SQLite state. Routing remains entirely local. A newer compact snapshot can be fetched explicitly:

```bash
dispatch data refresh
```

Refresh is optional. It validates the entire versioned snapshot before transactionally replacing the current distributed-public rows. An invalid response or offline Cloud leaves the prior working snapshot intact and never blocks a run. No raw Harbor, Terminal-Bench, or SWE-bench data is downloaded by this path.

The detailed `dispatch recommend`, `dispatch evidence`, and `dispatch datasets` commands remain advanced research and maintainer surfaces. Manual import still accepts local SWE-bench Verified and Harbor/Terminal-Bench results, preserves their raw normalization inputs for reproducibility, and keeps agent and model identity distinct. Maintainers can export only normalized public priors with:

```bash
dispatch datasets export-public-priors public-priors-v1.json
```

Export is deterministic for identical normalized input and excludes local observations and private run data.

## Configuration and verification

Configuration is optional. Dispatch discovers `dispatch.yml`, `dispatch.yaml`, `.dispatch.yml`, or `.dispatch.yaml` at the source root. `dispatch init` can create a starting file for advanced setup.

```yaml
execution:
  backend: local
  timeout_secs: 1800
  max_parallel: 3
  forwarded_env: []

checks:
  baseline: []
  verify:
    - cargo test

harnesses:
  claude:
    model: null
    extra_args: []
  codex:
    model: null
    extra_args: []
  cursor:
    model: null
    extra_args: []
```

Use the command that actually verifies the project, such as `pytest`, `npm test`, or `go test ./...`. Repository configuration cannot grant itself local-execution or environment-forwarding permission.

## Advanced comparison and inspection

The normal path selects one agent. Existing multi-candidate tournament behavior remains available through the hidden advanced `--harnesses` option and the `compare`, `inspect`, `show`, and explicit `apply` commands.

Blind comparison has different semantics from single-result acceptance: it records Candidate A/B/Tie/Neither preference without exposing the harness label first. Routed or manually overridden single results use accept/reject semantics instead. Dispatch never converts one model into the other.

`dispatch evidence local <source>` derives observational counts from durable local routing observations and the latest append-only human feedback revision. It does not rank agents, modify public priors, or affect Router decisions. These counts describe tasks selected by Dispatch's prior policy and optional developer feedback; they are not unbiased or calibrated estimates.

## Optional data contribution

Uploads are off by default. Normal runs, review, acceptance, recommendation, history, and public-data refresh do not upload observations.

Contribution requires both a separate ingestion token and explicit versioned consent:

```bash
dispatch sync token set <server-issued-token>
dispatch sync enable
dispatch sync preview <run-id> --type routing-observation
dispatch sync
```

`sync enable` records consent and prepares eligible local records; it does not transmit them. `sync preview` prints the exact versioned body. Bare `dispatch sync` is the only upload action. Existing legacy `evaluation-v1` consent does not authorize `routing-observation-v1`; the expanded scope must be accepted explicitly.

Contributed routing observations preserve prediction, actual agent, mechanical outcome, and optional explicit accept/reject feedback independently. Human explanations are uploaded verbatim when present and may contain proprietary information. This is contributed data, not anonymous telemetry.

V1 routing payloads exclude task text, source files and paths, snapshots, patches, logs, changed filenames, artifact paths, Git remotes, environment data, credentials, verification command text, and exact agent prompts. The public wire contracts live under [schemas/](schemas/). The durable SQLite outbox retains retryable failures and immutable idempotency identities.

## Local state and artifacts

Normalized state lives in `~/.dispatch/dispatch.db`; `--state-dir` or `DISPATCH_HOME` overrides that location. Per-run evidence lives under `~/.dispatch/runs/<run-id>/`, including the frozen baseline, candidate workspace, bounded stdout/stderr, structured agent output, checks, patch, events, and inspectable metadata.

No Cloud service is required to create, execute, inspect, accept, or reject a task. Public-data refresh and contribution sync are explicit, separate network actions.

## Current boundaries

- Experimental developer preview, not a stable 1.0 service.
- Real-agent release testing centers on Codex CLI and Cursor Agent; agent installation, authentication, quotas, and provider availability remain external prerequisites.
- Public evidence is observational benchmark evidence, not ground truth, a quality label, confidence, or calibrated probability.
- If no compatible public evidence exists, the deterministic fallback order is the existing real-adapter order: Claude Code, then Codex, then Cursor, restricted to agents detected as locally executable.
- Local empirical observations do not influence routing yet.
- No automatic retry, escalation, task decomposition, agent racing in normal mode, ML, embeddings, LLM judging, background refresh, or Cloud routing lookup exists.

## Development

Rust owns local classification, routing, execution, persistence, review, and the explicit sync client. Dispatch Cloud is optional and distributes one compact public snapshot plus explicit contribution endpoints.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Detailed contributor guardrails live in [AGENTS.md](AGENTS.md).
