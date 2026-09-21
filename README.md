# Dispatch

**Dispatch 0.1.3-rc.1 — Local release candidate / experimental developer preview**

Local artifacts are available for review. The publication gate remains blocked by
an unresolved deadline-fixture failure; see the [exact validation results](docs/product-rc-validation.md).

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

Dispatch keeps orchestration and evidence local. Provider execution requires the provider’s service. A release contains a compact public-evidence snapshot, normal runs never fetch benchmark data, and Dispatch Cloud is optional.

## Interactive dogfood loop

From a source directory, open Dispatch. Missing resources lead to guided setup
and preserve your goal. You can also configure resources first:

```bash
dispatch setup                 # provider login guidance and explicit funding consent
dispatch setup --checks        # approve a known project check
dispatch                      # direct goal → work → verification → review
```

Enter the outcome, then approve local execution for that goal. Dispatch displays
allocation, work, verification, and any bounded recovery. A durable clarification
appears directly in the session; submitting its answer continues automatically.
At review, enter `d` for the diff, `a` to accept and safely apply, `r` to reject,
`i` for artifact details, or `n` to leave the result pending and start another goal.
These actions retain the displayed run, question, and candidate identities.
Verification and human acceptance remain separate.

The compact inline view preserves scrollback and uses the terminal's background.
Enter submits, Alt+Enter inserts a newline, bracketed paste inserts without
submitting, Ctrl+C cancels, and Ctrl+D exits an empty editor. During work,
Ctrl+C waits for process cleanup; EOF/hangup also requests controlled cancellation.
Input entered while work is active is discarded rather than queued as another goal.

Use `dispatch --plain` for ordinary line input, `--ascii` for ASCII graph marks,
and `--no-color` (or `NO_COLOR`) for native colors. `TERM=dumb` selects plain mode.
Plain input is line-oriented; the integrated editor supports multiline paste.
`dispatch --no-retry` disables automatic recovery for the interactive session.
Bare invocation without terminal input and output prints help and exits 2.

Use `/resources` for Accounts / Resources and `/checks` for project checks.
`dispatch resources` prints cached configuration status without a prompt or model
call. Discovery never authorizes spending. Codex and Claude use their own supported
CLI login; no Dispatch account is required. Confirm the exact model, effort and
included funding. Claude assertions expire after at most 24 hours and have an
explicit refresh path. Changed/rejected funding epochs need fresh owner consent.
No paid fallback, credits, account switch or private-policy activation is automatic.
See [setup and review](docs/product-guide.md), [visual system](docs/design-system.md)
and [candidate validation](docs/product-rc-validation.md).

For an explicitly multipart goal, opt in with `/plan <goal>` in the composer or
`dispatch run /path/to/project --task 'Goal' --plan`. Planning uses one planner,
up to four sequential tasks and one shared extra under one deadline, with one
final review. It requires an owner-approved check contract and suitable included
profiles. Ordinary goals remain direct. See [bounded planned work](docs/planning.md).

## Machine control

`dispatch control --stdio` provides scoped, foreground JSONL requests, replayable
events, semantic waits, and durable retries. A human provisions the scope with
`dispatch control-grant`; the client inherits a private grant handle. Human
acceptance and application remain in the existing review workflow.

See the [protocol and standalone client guide](docs/control-protocol.md) and
[Phase 5 validation report](docs/phase5-validation.md). Neither requires Herdr.

## One-shot CLI

The included-resource path supports configured Codex and Claude contracts. Legacy explicit harness overrides remain advanced; they do not inherit an included-funding guarantee. From a repository or plain directory:

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

The legacy one-shot path without allocation profiles uses benchmark performance to choose only when at least two execution-eligible agents have compatible, nonzero evidence. It compares that evidenced subset using the existing Router; agents without evidence remain unknown, not inferior. Otherwise it uses the first available agent in the fixed order Claude Code → Codex → Cursor, independently of benchmark availability. A sole eligible agent is labeled “Only available agent.”

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

## Install this candidate

Use the tested local archive and explicit executable paths in
[install / upgrade / uninstall](docs/release-install.md). The current candidate
has not been published. Older GitHub release downloads are not this build.

The validated runtime scope for this pass is macOS arm64. Linux has an existing
CI recipe but was not run here. The macOS package is unsigned and not notarized.
Source builds use `cargo build --release --locked`; Git and the project’s actual
check tools must be installed. No provider tool or font is installed by Dispatch.

## Safety and source behavior

> **Local execution is not a security sandbox.**

Real agents and project checks run with the permissions of the Dispatch process and receive `HOME` so installed harnesses can use local authentication. Dispatch clears most other child environment variables. Configured extra variables require the separate `--allow-forwarded-env` acknowledgement and their values are redacted from persisted logs.

Dispatch freezes the source into an internal Git baseline and gives the selected agent an independent candidate workspace. It does not run the agent directly in the original tree. `dispatch accept` uses the existing safe apply path and rejects source drift; `dispatch reject` never applies candidate changes.

If verification is configured, the same commands run against the candidate and their output is retained. Without configured checks, Dispatch reports `Verification: Not configured`. Verification is mechanical evidence, not a universal code-quality judgment. A completed harness invocation is reported as `Ready for review`; failed checks are reported as `Verification failed`, never as completed or verified work.

The local backend is the supported real-agent path in this candidate. Docker execution is advanced and experimental: users must provide a suitable image containing the agent and project toolchain.

## Core commands

```text
dispatch run "<task>" [--source path] [--agent claude|codex|cursor]
dispatch run "<task>" --json|--jsonl
dispatch status [run-id] [--json|--jsonl]
dispatch diff [run-id] [candidate]
dispatch accept [run-id]
dispatch reject [run-id]
dispatch explain [run-id]
dispatch check [run-id] [--json]
dispatch refresh [run-id]
dispatch history [--limit count]
dispatch version
```

Without a run ID, `status`, `diff`, `accept`, `reject`, and `explain` resolve the latest relevant single-result run for the current source tree. Explicit run IDs remain available for history and debugging. `--task-file path|-` reads a task from a file or stdin, and `--source path` overrides the current directory.

`run --json` writes one versioned result object to stdout. `run --jsonl` writes each committed, sequenced event followed by a final result object; `status --jsonl` replays that committed journal. Machine modes require non-interactive authorization flags when local execution needs acknowledgement, keeping stdout parseable.

Advanced automation can follow committed events without controlling execution:

```bash
dispatch events <run-id> --after 0 --until attention --timeout 30
dispatch events <run-id> --after <cursor> --until finished --timeout 30
```

Output is JSON Lines with a final committed cursor. `attention` includes human
waiting or a finished outcome; `finished` waits for the core lifecycle to finish
(including a result awaiting review). Timeout exits 124 without answering,
launching, or repairing anything. Omit `--until` to replay through the current
journal. Events now include their committed `payload.outcome`, so a transient
question cannot disappear merely because it was answered before the next poll.
A cursor ahead of the journal is rejected. Historical events written before
Phase 4 remain readable, but cannot reconstruct transient states they did not
record; current state is still available.

One-shot exit codes are:

- `0`: a result is ready for review and verification passed or was not configured;
- `3`: a result is ready for review, but configured verification failed;
- `5`: required subscription capacity is deferred without a model launch;
- `1`: execution or orchestration failed;
- `2`: command-line usage error.

The result keeps execution, verification, review, and application as separate fields. Attempt records also keep requested, resolved, and harness-observed model/effort values separate; an unknown or mismatched observed identity is not replaced by configuration.

### Model allocation

`dispatch run --agent codex --model <id> --effort <level> "<task>"` selects one
declared Codex resource through the normal isolated execution path. Dispatch
validates the choice against the user-level `$DISPATCH_HOME/resources.yml`
file (normally `~/.dispatch/resources.yml`); project configuration cannot
enable allocation.

To opt into deterministic light/standard/strong selection for ordinary runs,
create that file with `allocation_enabled: true` and the verified profiles you
actually have:

```yaml
version: 1
allocation_enabled: true
capacity:
  codex_probe: true
  probe_timeout_secs: 5
  freshness_secs: 300
  admission: true
  lease_secs: 20
  heartbeat_secs: 5
  aging_secs: 60
profiles:
  - provider: openai
    funding_source: chatgpt-plus
    harness: codex
    model: your-included-model-id
    effort: low
    service_mode: standard
    runtime: local
    pool: chatgpt-codex
    provider_buckets: [your-observed-codex-limit-id]
    tier: light
    included: true
    no_overage_verified: true
    authorization_revision: 1
```

The deterministic policy uses standard when task type or scope is unknown; ordinary
requests do not need to name a file or use a special opening phrase. Recognized
feature/refactor work and known broad scope still require strong. Unknown inputs
remain recorded as unknown and do not qualify a task for light. Explicit model
constraints and funding checks still apply.

Add suitable `standard` or `strong` profiles when available. `no_overage_verified` is an
explicit assertion that the account or invocation cannot fall through to paid
overage; an included model name or visible credits are not enough. Profiles
without that assertion are shown in `dispatch explain` but are ineligible.
Before launching a selected model, Dispatch uses the optional read-only Codex
account/rate-limit probe to check fresh authentication, plan, credit, service,
pool, and window facts. A reported change invalidates the earlier assertion;
unavailable quota telemetry remains explicitly unknown and does not fabricate
capacity. Reset timestamps only schedule another observation.

`authorization_revision` is an explicit local funding-authorization epoch. If
fresh evidence reports a changed account, plan, service tier, credit capability,
or pool/window identity, that epoch remains rejected on later runs even though
the raw observation is retained. Increment it only after affirmatively checking
and accepting the new subscription-only account state. When an applicable
window enters Reserve, background and standard work are deferred with exit 5;
only an explicitly urgent (`--priority urgent`) request may proceed.

Profiles backed by the same subscription allowance must use the same `pool`
and opaque `provider_buckets` mapping. Foreground Dispatch processes sharing a
state directory use a fair, fenced SQLite admission lease and initially allow
one active model invocation per pool. The lease is released after child cleanup
is confirmed and before local verification or review. This coordinates only
cooperating local Dispatch processes; other applications, machines, and mixed
provider activity remain outside its guarantee and quota changes are not
attributed to an individual attempt without supporting evidence. `dispatch
explain` and JSON output retain the detailed field-level observation.
Setting `capacity.admission: false` does not bypass shared coordination for an
allocation run. Dispatch rejects that run; disable allocation after coordinated
owners have drained if a single uncoordinated session is intentionally desired.
Allocation decisions and goal feedback remain local and never enter the v1
routing sync envelope. Remove the file or set `allocation_enabled: false` to
return ordinary runs to legacy agent routing; explicit model controls remain
available when they match a verified profile.


Claude Code can use the same allocation, admission, recovery and review paths.
Its included-only profiles additionally require private, time-bound account and
invocation evidence; configuring a login or a model name alone is insufficient.
See the [Phase 6 setup/support matrix](docs/phase6-validation.md) before enabling
one. A bounded live smoke passed with Claude Code 2.1.274, personal Pro,
`claude-sonnet-5` at medium effort, and usage credits disabled; this is specific
to the tested account state and configuration. Keep Codex first in profile order
to preserve that default. Either provider can be configured alone, and explicit
`--agent` choices never fall back.

### Bounded recovery and clarification

Allocation runs allow at most two model invocations in total. After a target
check fails, Dispatch may make one stronger attempt if that exact check passed
on the original baseline, all baseline checks passed, and a suitable verified
profile satisfies the original constraints. `--no-retry` disables this automatic
recovery. An explicit fixed model/effort is never silently overridden. Ordinary
pre-existing baseline check failures permit initial work but disable recovery;
missing tools and baseline infrastructure failures stop before model work.

Every attempt uses fresh authorization, shared admission and a new fence. Failed
workspaces and per-attempt evidence are retained. Recovery starts from the
original baseline with bounded failure diagnostics; the final diff remains
relative to that baseline. Mixed attribution is explicit and stays local.
`--timeout` is one goal-wide deadline covering baseline checks, attempts,
verification and human waiting; it does not reset for recovery or continuation.

Codex may request an essential clarification using this exact JSON envelope in
its final `agent_message` event, followed by a successful process exit:

```json
{"dispatch_checkpoint":{"version":1,"question":"Which behavior is required?","choices":["A","B"]}}
```

Dispatch validates the report after child cleanup and admission release, then
persists a question with `lifecycle=waiting` and `waiting_on=human`. It never
pauses a live model process. Malformed, non-final or unsuccessful reports do not
create a question. Answer or cancel the specific question with its current
revision (available in `dispatch status <run-id> --json`):

```bash
dispatch answer <run-id> <question-id> --revision 1 --answer "A" --json
dispatch cancel <run-id> <question-id> --revision 1 --json
```

Answers are authorized by the local run owner's OS identity and accepted once.
The answer command owns a fresh foreground continuation using the original
baseline and answer. That invocation consumes the remaining slot in the same
two-invocation budget. A second checkpoint cannot create a third invocation.
There is no daemon, automatic crash replay or separate human `resume` command.
Pending questions and all attempt evidence survive process exit and reload.
JSON/JSONL includes each attempt, its bindings and checks, the final delivery
pointer, elapsed time, normalized failures, and question state.

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
- Phase 7 private evidence supports explicit owner-controlled trials; no private policy is activated automatically.
- Direct allocation has bounded recovery. Task decomposition is opt-in and sequential; no agent racing, learned planner, ML, embeddings, LLM judging, background refresh, daemon, or Cloud routing lookup is added. Planning effectiveness remains unmeasured.

## Development

Rust owns local classification, routing, execution, persistence, review, and the explicit sync client. Dispatch Cloud is optional and distributes one compact public snapshot plus explicit contribution endpoints.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Detailed contributor guardrails live in [AGENTS.md](AGENTS.md).
