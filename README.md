# Dispatch

**Dispatch v0.1.1 — Experimental Developer Preview**

Dispatch runs the same software task through multiple coding-agent harnesses against equivalent source states, captures comparable execution evidence, and lets the developer inspect and choose the result they prefer.

Dispatch v0.1.1 does not route work by default or decide which agent is best. An explicit experimental `--route` mode can select one locally runnable harness from cached benchmark evidence; ordinary runs still use the harnesses the developer names.

Runs, evaluations, and routing observations are stored locally. Nothing is uploaded to Dispatch Cloud unless the user accepts the applicable contribution scope and then explicitly runs `dispatch sync`.

## Why Dispatch exists

Coding harnesses differ by task: they can have different latency, completion behavior, reported token use, and implementation choices. Dispatch records those differences on the developer's actual software work.

```text
task
  ↓
Codex / Cursor / other harnesses
  ↓
equivalent independent candidates
  ↓
verification + runtime + usage + diffs
  ↓
human evaluation
  ↓
durable evidence
```

Dispatch treats automated checks and operational measurements as evidence. It does not combine them into a universal quality score; human preference remains the v0.1.1 quality signal.

## What v0.1.1 does

- Runs locally against ordinary directories, Git repositories, and linked worktrees; GitHub, a remote repository, an account, and Dispatch cloud are not required.
- Freezes the source into an internal Git baseline and gives every harness its own candidate workspace.
- Runs heterogeneous candidates concurrently with per-candidate timeout supervision.
- Supports Codex CLI and Cursor Agent as the v0.1.1 real harnesses, plus deterministic offline fakes for workflow testing.
- Runs command-level verification against the frozen baseline and every completed candidate.
- Preserves stdout, stderr, structured harness output, checks, workspaces, and patches.
- Records runtime, reported usage, cost when directly available, changed files, and diff statistics.
- Supports blind Candidate A/B comparison, candidate inspection, and A/B/Tie/Neither evaluation.
- Stores optional structured reasons and unrestricted human explanations verbatim.
- Persists normalized records in SQLite and large artifacts on the filesystem.
- Explicitly imports local SWE-bench and Terminal-Bench/Harbor snapshots as cached experimental routing priors.
- Can explicitly route one run to one locally runnable real harness while preserving the normal execution and verification path.
- Records routed-run predictions, mechanical outcomes, and any later existing evaluation as separate durable local observations.
- Leaves the original source unchanged through the evaluation flow; `dispatch apply` is explicit and rejects source drift.

## Install

Prebuilt archives are the preferred installation path. All builds are available on the [latest GitHub Release](https://github.com/dispatch-ai-org/dispatch/releases/latest).

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

Ensure `$HOME/.local/bin` is on your `PATH`. Dispatch also requires Git and `curl` at runtime.

### Install from source

Advanced users with the current stable Rust toolchain and Git can install directly from the public repository:

```bash
cargo install --git https://github.com/dispatch-ai-org/dispatch --locked
dispatch version
```

Dispatch is not published to crates.io.

Install and authenticate the harness CLIs you intend to use. `dispatch doctor` reports what it can find, but it does not validate account quotas, remote service availability, or the contents of a Docker image.

## Security first

> **Local execution is not a security sandbox.**

Real agents and project checks run on the host only when each command includes `--allow-unsafe-local`. They run with the operating-system permissions of the Dispatch process, can access host files reachable by that user, and receive `HOME`, so installed harnesses can use their local authentication. CPU and memory settings are recorded but are not enforced by the local backend.

Dispatch clears most child environment variables. Additional names are forwarded only when they appear in `execution.forwarded_env` and the run also includes `--allow-forwarded-env`; forwarded values are redacted from persisted logs. This reduces accidental exposure but does not make local execution a sandbox.

The local backend is the supported v0.1.1 real-harness backend. Docker execution is experimental: container lifecycle, candidate isolation, resource limits, timeout handling, verification, and cleanup have been validated with test harnesses, but v0.1.1 does not provide turnkey container images or container-compatible authentication for Cursor or Codex. The image must already contain the harnesses and project toolchain; the default `ubuntu:24.04` value is only a placeholder.

## Quick start with Codex and Cursor

Install and authenticate the Codex and Cursor harness CLIs, then create the project configuration:

```bash
dispatch init ./my-project
```

Before the first run, replace the empty `checks.verify` list in `my-project/dispatch.yml` with the project's real verification command. For a Rust project:

```yaml
checks:
  verify:
    - cargo test
```

Use the command that actually verifies the project, such as `pytest`, `npm test`, or `go test ./...`. Dispatch is most useful when every candidate is checked against the same project verification command. Tests, builds, and lint are evidence—not Dispatch's automatic judgment of code quality—and a passing check does not select a winner. The human evaluation remains the outcome.

Check the environment and review the reported verification state before running:

```bash
dispatch doctor ./my-project
```

Run both harnesses against fresh copies of the same baseline:

```bash
dispatch run ./my-project \
  --task "Fix the retry race in the worker." \
  --harnesses codex,cursor \
  --backend local \
  --timeout 300 \
  --max-parallel 2 \
  --allow-unsafe-local
```

To opt into a single-harness routed run using only locally cached evidence:

```bash
dispatch run ./my-project \
  --task "Fix the retry race in the worker." \
  --route \
  --allow-unsafe-local
```

`--route` and `--harnesses` are mutually exclusive. Routed execution is local-only in v0.1.1 because Dispatch must establish executable availability before selecting a harness.

Review the resulting run without submitting an evaluation:

```bash
dispatch compare <run-id>
dispatch diff <run-id> A
dispatch diff <run-id> B --stat
dispatch inspect <run-id> A
dispatch show <run-id>
```

Record a human decision only after review:

```bash
dispatch compare <run-id> \
  --winner B \
  --reason correctness \
  --reason cleaner-change \
  --explanation "B fits the existing design better."
```

Interactive evaluation is available with `dispatch compare <run-id> --evaluate`. Valid outcomes are a candidate label, `tie`, or `neither`. Applying a result is a separate optional action:

```bash
dispatch apply <run-id> B
```

A predictively routed single-candidate result uses explicit acceptance feedback instead of blind comparison:

```bash
dispatch evaluate <run-id> \
  --outcome accept \
  --reason correctness \
  --explanation "The routed result is ready to use."
```

Use `--outcome reject` when the disclosed result is not acceptable for the task. Repeating identical feedback is a no-op; submitting changed feedback replaces the routed human signal on the same observation. This does not alter the routing prediction or mechanical verification result.

Optional Cloud contribution is also separate. After configuring the developer-preview ingestion token described below, the reviewed flow is:

```bash
dispatch sync enable
dispatch sync preview <run-id> [--type evaluation|routing-observation]
dispatch sync
```

For an offline workflow check, reuse the run command but omit `--harnesses`; the default `fake-good,fake-bad` pair exercises snapshotting, execution, comparison, persistence, and apply without harness credentials or network access. Keep `--allow-unsafe-local` when configured project checks run on the host.

## Source and verification behavior

Dispatch snapshots the exact local source tree into an internal Git baseline, including dirty and untracked content. It then creates an independent full workspace for each candidate. Harnesses are pointed at those candidate workspaces, not at the original source tree.

A local backend still permits a harness to reach other host paths, which is why real local runs require `--allow-unsafe-local`. The original source is changed only by explicit `dispatch apply`, which first checks that the source still matches the run's recorded fingerprint.

If `checks.baseline` is empty, Dispatch runs `checks.verify` against the frozen baseline as evidence. A failing baseline is recorded and displayed but does not stop candidate execution: repair tasks commonly begin with failing tests. Dispatch then runs the verification commands independently against every completed candidate and retains per-command stdout and stderr.

Verification is mechanical evidence, not an automatic quality decision: `PASS` does not mean winner. Human preference remains the evaluation outcome.

New files that remain ignored by a candidate's final `.gitignore` stay in its inspectable workspace but are omitted from `diff.patch` and `apply`. This prevents common test/build caches from entering an applied change in v0.1.1.

## Commands

```text
dispatch init [path] [--force]
dispatch doctor [source] [--config path]
dispatch recommend [source] (--task text | --task-file path)
dispatch run [source] (--task text | --task-file path)
  [--harnesses codex,cursor | --route] [--config path]
  [--backend local|docker] [--timeout seconds] [--max-parallel count]
  [--allow-unsafe-local] [--allow-forwarded-env]
dispatch status [run-id]
dispatch history [--limit count]
dispatch show <run-id>
dispatch diff <run-id> <candidate> [--stat | --name-only]
dispatch inspect <run-id> <candidate> [--shell]
dispatch compare <run-id> [--evaluate]
  [--winner A|B|tie|neither] [--reason value]...
  [--explanation text | --explanation-file path|-]
dispatch evaluate <run-id> --outcome accept|reject [--reason value]...
  [--explanation text | --explanation-file path|-]
dispatch apply <run-id> <candidate>
dispatch datasets import swe-bench <path>
dispatch datasets import terminal-bench <path>
dispatch sync enable|disable|status
dispatch sync preview <run-id> [--type evaluation|routing-observation]
dispatch sync token set <token>
dispatch sync token status|clear
dispatch sync
dispatch version
```

Run IDs may be abbreviated when the prefix is unique. `--state-dir` is global and `DISPATCH_HOME` provides the same override; the default state root is `~/.dispatch`.

Structured evaluation reasons are optional. Current canonical values are `correctness`, `completeness`, `architecture`, `maintainability`, `readability`, `tests`, `edge-cases`, `cleaner-change`, `performance`, `cost`, `latency`, and `other`. `cleaner-change` remains specific to candidate comparison and is not valid for routed acceptance feedback. Freeform explanations are unrestricted and stored verbatim.

## Local benchmark priors

`dispatch datasets import swe-bench <path>` explicitly imports a local SWE-bench Verified snapshot. The directory must contain `metadata.yaml`, `instances.jsonl`, and `results/results.json`. Dispatch preserves those exact files in its local dataset cache and stores only normalized aggregate evidence in SQLite for the experimental Router. Import and routing perform no benchmark network requests.

`dispatch datasets import terminal-bench <path>` imports a completed Harbor job directory containing `config.json` and per-trial `<trial>/result.json` files. Dispatch caches those exact normalization inputs, keeps Harbor agent and model identities separate, and records Terminal-Bench morphology as unknown. No normal Dispatch command contacts Harbor or Terminal-Bench.

The Router treats an unknown prior dimension as compatible fallback evidence for a known task, never as an exact wildcard. Conflicting known values are rejected, and an unknown task dimension cannot consume a known prior value. Specificity is the number of dimensions the prior establishes and matches. If several priors are compatible with one harness, Dispatch selects one by greater specificity, then attempts, then stable provenance identity; it does not sum unrelated benchmark sources.

`dispatch recommend <source> --task <text>` classifies the local source and task, then displays compatible cached evidence without executing or detecting a harness. It considers the supported real adapters (`claude`, `codex`, and `cursor`) regardless of whether they are installed or customized in configuration; fake adapters are excluded. The command performs no network requests, leaves the source unchanged, and exits successfully with an explicit message when no compatible evidence exists.

`dispatch run <source> --task <text> --route --allow-unsafe-local` uses the same classification and Router semantics, skips predicted adapters that are not locally executable under the effective configuration, and selects exactly one scored real adapter. The selected adapter then enters the same candidate, executor, verification, artifact, and cleanup path as an explicit one-harness run. No evidence, or no runnable predicted adapter, is a pre-execution error; Dispatch never substitutes an unscored harness. The task features and selected prior are stored separately from the resulting mechanical execution and any later human evaluation.

Once a routed candidate reaches a terminal state, Dispatch records one local routing observation keyed to that run. It snapshots the prediction provenance, actual harness/model identity, process state, and configured verification statuses without turning any of them into a quality label. Missing verification and missing human evaluation remain unknown, and `dispatch show <run-id>` displays the observation. `dispatch evaluate <run-id> --outcome accept|reject` records a separate human acceptance signal on that same observation; acceptance is never inferred from verification, and rejection never changes verification. The existing blind Candidate/Tie/Neither comparison model is retained separately because it does not faithfully represent acceptance of a disclosed single routed candidate. These observations and their human signals are not Router priors and are not included in Cloud sync.

The SWE-bench importer preserves the upstream `tags.agent` and single `tags.model` identities separately and accepts only pass@1 submissions without translating agent identity. Harbor maps only its verified `codex` and `cursor-cli` integrations to Dispatch `codex` and `cursor`; other Harbor agent names remain unchanged. A prior is usable only for its stored harness identity.

## Optional contribution sync

Uploads are off by default. Normal local startup, `doctor`, runs, recommendation, comparison, routed evaluation, history, and apply do not contact Dispatch Cloud.

Early developer-preview ingestion requires a server-issued token. Configure that separate submission permission locally with `dispatch sync token set <server-issued-token>`; storing a token does not enable sharing.

The consent, review, and transmission sequence is exactly:

```bash
dispatch sync enable
dispatch sync preview <run-id> [--type evaluation|routing-observation]
dispatch sync
```

`dispatch sync enable` records the current versioned consent locally, creates or preserves the contributor identity, and prepares eligible historical evaluations and routing observations in the same local outbox. It does not upload data. Consent version 1 authorizes only `evaluation-v1`; existing version-1 users retain evaluation sync but routing observations remain ineligible until they explicitly run `sync enable` again to accept version 2 and `routing-observation-v1`.

`dispatch sync preview <run-id>` requires the applicable consent and prints the exact eligible HTTP body to stdout without transmitting it; the record type is identified separately. If a routed run also has a blind evaluation, select the body explicitly with `--type evaluation` or `--type routing-observation`. Bare `dispatch sync` is the only transmission step for pending and retryable failed records.

The contributor identity is a random local ULID that is not derived from an account, machine, path, or hardware data.

The token is a lightweight submission permission, not an account, identity, or proof that an evaluation is genuine. It is stored in the local SQLite state, is never printed after storage, and is not encrypted beyond filesystem protections on the Dispatch state directory. `dispatch sync disable` stops uploads but does not clear it; use `dispatch sync token clear` explicitly. `dispatch sync status` and `dispatch sync token status` inspect local state without contacting Cloud.

Token possession and sharing consent are independent. Configuring a token never enables uploads, and enabling sync without a token cannot upload. Local execution and evaluation never require either one.

`evaluation-v1` shares the exact task description; Dispatch, OS, architecture, and backend metadata; harness/version/model identity; execution, usage, diff-count, and verification results; and the blind human outcome, structured reasons, and freeform explanation. `routing-observation-v1` separately shares task morphology, the prediction snapshot and benchmark provenance, actual harness/model identity, mechanical outcome, and optional routed accept/reject feedback. Routed human explanations are uploaded verbatim when present. This is contributed data, not anonymous telemetry; task text and explanations may contain proprietary context. The ingestion token is an HTTP header and never appears in either body.

Both V1 payloads exclude source files, snapshots, patches, logs, environment names and values, credentials, Git identity/remotes and fingerprints, verification command text, exact harness prompts, changed-file names, candidate errors, and absolute local paths. Routing observations additionally exclude task text. Richer source or diff sharing is not a v0.1.1 consent scope. The public contracts are [`schemas/evaluation-v1.json`](schemas/evaluation-v1.json) and [`schemas/routing-observation-v1.json`](schemas/routing-observation-v1.json).

The default Cloud base URL is `https://api.rundispatch.sh`. `DISPATCH_CLOUD_URL` preserves the development/testing override, and plain HTTP is accepted only for loopback testing. The SQLite outbox retains failed uploads for explicit retry; local evaluation never depends on network success.

The client uses the system `curl` with bounded timeouts. Evaluation records go to `POST /v1/evaluations`; routing records go to `POST /v1/routing-observations`. Both send `Authorization: Bearer <token>` and the stable record ID as `Idempotency-Key`. HTTP 201 and idempotent HTTP 200 responses mark a record synced. Transient failures remain retryable; `422 idempotency_conflict` remains locally visible as a durable conflict and is not treated as synced.

Before its first successful upload, a pending or failed routing payload is refreshed from the current local observation during explicit preview or sync preparation, so newly added human feedback is not silently omitted. A local human-evaluation change invalidates an earlier preview, so preview again after changing it. After a routing observation is synced, later local feedback remains local: Dispatch refuses to mutate or resend the immutable Cloud record under the same observation ID.

## Configuration

`dispatch init` creates `dispatch.yml` without replacing an existing file unless `--force` is supplied. Dispatch also discovers `dispatch.yaml`, `.dispatch.yml`, and `.dispatch.yaml` at the source root.

```yaml
execution:
  backend: local
  timeout_secs: 1800
  cpus: 2
  memory: 4g
  max_parallel: 3
  docker_image: ubuntu:24.04
  forwarded_env: []

checks:
  baseline: []
  # Use your project's verification commands.
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

Each harness can also specify an `executable` path. Repository configuration cannot grant itself local execution or environment forwarding; those require command-line acknowledgement for each run. Verification entries are shell commands by design.

## Evaluation and blinding

Comparison exposes candidate verification, runtime, reported usage, change size, logs, patches, and retained workspaces. The evaluator may choose A, B, Tie, or Neither and may attach structured reasons, unrestricted explanation text, both, or neither.

Harness names are omitted from the normal comparison view until an evaluation is successfully persisted. This is **explicit-label blinding**, not side-channel-proof anonymity. Timing, token semantics, code style, local metadata, or harness-specific artifacts may allow a developer to infer identity. Dispatch does not claim cryptographic blinding and does not use an LLM judge.

## Telemetry and artifacts

Harnesses do not report usage identically. Cursor and Codex may expose different input, cache, and output categories. Dispatch stores the reported task-token value together with its `token_semantics` instead of pretending the values are perfectly interchangeable. Transaction-level dollar cost is recorded only when the harness directly reports it; unknown values remain unknown.

Normalized data lives at `~/.dispatch/dispatch.db`. Per-run evidence lives under `~/.dispatch/runs/<run-id>/`, including:

```text
task.md                 exact task
config.snapshot.yml     effective configuration
metadata.json           inspectable run record
events.jsonl            append-only run events
baseline/               frozen source state
candidates/<id>/
  workspace/            retained candidate tree
  prompt.txt            exact harness prompt
  stdout.log            bounded head-and-tail capture
  stderr.log            bounded head-and-tail capture
  harness.jsonl         structured harness events
  diff.patch            candidate delta
  checks/               verification logs
```

Stdout and stderr are capped at a marked 16 MiB head-and-tail capture per stream. Dispatch also rejects escaping source symlinks, very large candidate trees/files, oversized patches, and host-side Git post-processing that exceeds its safety timeout.

## Current boundaries

- Experimental developer preview, not a stable 1.0 or hosted service.
- The local backend is the supported real-harness backend in v0.1.1. Docker real-harness execution remains experimental and requires a suitable image and container-compatible authentication.
- Real-harness release testing currently covers Codex CLI and Cursor Agent; installation and authentication are external prerequisites.
- Local real-harness execution requires explicit unsafe acknowledgement.
- Verification is only as meaningful as the project's configured commands.
- Token accounting is harness-specific; no normalized cross-harness cost model exists.
- Optional Cloud contribution is off by default and requires explicit versioned scope consent plus a developer-preview ingestion token; only bare `dispatch sync` transmits.
- Experimental success-ratio ranking affects execution only when a developer explicitly supplies `--route`; no default routing, retry/escalation, automatic winner, bundled cloud service, web UI, or universal quality score exists.
- No cloud service is required for local use.

## Architecture and development

```text
Rust — Dispatch core
├── CLI and run orchestration
├── harness adapters
├── process supervision and execution backends
├── source snapshots and candidate workspaces
├── verification and diff/artifact capture
├── evaluation
├── explicit opt-in evaluation sync
└── SQLite + filesystem persistence
```

Rust owns local execution. If a networked product is introduced later, Go may own coordination, aggregate learning, and routing services, but cloud must never become a prerequisite for a normal local run.

Dispatch deliberately favors a small execution core over an internal platform. New production code should solve observed problems rather than speculative ones. Detailed guardrails for human and coding-agent contributors live in [`AGENTS.md`](AGENTS.md).

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

The end-to-end suite uses temporary non-Git projects and deterministic fake harnesses; it needs no account, network access, or paid agent.
