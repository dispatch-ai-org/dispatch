# Attached work and `serve`: technical reference

This page describes what `src/orchestrator/attach.rs` and `src/orchestrator/serve.rs`
do: `dispatch attach`, `dispatch finish` and `dispatch serve`, the three commands that
let Dispatch observe, judge and — only if you ask — apply work done by an agent it did
not launch. For the short version and the commands, see the
[README](../README.md#attach-your-own-agent) and the
[product guide](product-guide.md#using-your-own-agent). Coherence itself (the model,
the verdicts, the fixture matrix) is documented in [coherence.md](coherence.md); this
page is about how attached work is fed into that same model. Attach and `serve` are
part of the experimental developer preview.

## The owner-loop model

The unit that keeps native Dispatch work coherent is an **owner loop**: one foreground
process that holds the run's operation lock, runs the watcher, persists verdicts, and
applies through `auto_apply`. Attached work gets the same owner loop, not a different
mechanism:

- **`dispatch attach -- <agent command>`** (wrapped attach) *is* the owner loop of the
  Work it creates: it starts the agent under the terminal, watches the integration
  world while the agent runs, and on exit freezes the patch, verifies it, and applies
  it if authorized. Claude, Codex, Cursor or anything else stays exactly itself;
  Dispatch prints nothing to that terminal while it runs.
- **`dispatch serve`** is the owner loop for Work that has no live owner: an already
  running agent attached with `--workspace` and no command, or wrapped Work whose
  wrapper process died. It also renders the project view. One foreground process per
  integration root.

No daemon is required for a standalone run, wrapped attach, or the TUI. `serve` is
needed only to keep an already-running foreign agent observed and to apply its work
once it is ready. Native runs and attached Work coexist by sharing state (SQLite, run
directories) and locks (the per-source apply lock), not by routing native runs through
`serve`: both paths call the same `evaluate`, `gate` and `auto_apply`.

## Repository and integration-root identity

- **Integration root**: the checkout `attach`/`serve` watch and apply into. It becomes
  the attached run's `source_path`, so the world, `world::observe` and the per-source
  apply lock are the ones native runs on that source already use. `--root` defaults to
  the repository's main worktree (the first entry of `git worktree list --porcelain`)
  when the workspace is a linked Git worktree; it is required for a plain directory.
  `serve` uses its current directory or `--root`.
- **Repository key**: `sha256(canonical git-common-dir)`, recorded on the attachment
  as `repo_key`. `attach` refuses a workspace whose common dir differs from the root's:
  *"workspace belongs to a different repository than the integration root."* Two main
  worktrees of the same repository are, by design, two separate worlds; the attachment
  names the one it targets.
- **Same-checkout attach** (workspace equals the integration root) is refused:
  *"attach needs a separate worktree; run git worktree add."* Δ attribution and
  self-application are undefined when the workspace and the root are the same tree, so
  this is refused rather than left ambiguous.
- **Plain directories**: only the wrapped form can attach a plain (non-Git) directory;
  S0 is a snapshot taken at attach time (below). Foreign attach of a plain directory
  always requires Git and is refused otherwise.

## The record: an ordinary `RunRecord`

Attached Work is a `RunRecord` with `mode: RunMode::Attached` (`"attached"` in JSON)
and exactly one candidate. This is the whole trick that keeps coherence from forking
into two implementations: `check`, `explain`, `status`, `accept`, `reject`, `apply`,
`auto_apply`, the watcher, `gate` and `apply_validated` all take a `RunRecord` and a
candidate label, and none of them need to know whether Dispatch launched the agent.

| `RunRecord` field | Attached value |
|---|---|
| `source_path`, `source_kind`, `source_git_head`, `source_fingerprint` | the integration root at attach time |
| `baseline_path`, `baseline_commit` | S0, materialized under `runs/<id>/baseline` (below) |
| `candidates[0]` | `workspace_path` = the external worktree or directory; `diff_path` = `runs/<id>/delta.patch`; `harness_id` = `attachment.agent` or `"external"`; `label` = `"A"`; `checks` filled at finish |
| `attempts[0]` | one record, `role: "attached"`, same `harness_id`, `resource: None`, every model field `None` (never guessed) |
| `environment` | `execution_backend: "local"`; `unsafe_local` = the `--allow-unsafe-local` acknowledgement; the rest copied from the root's `dispatch.yml` |
| `execution` | `None`: no invocation budget, no questions |
| `task` | `--task` text, or `"attached work in <workspace basename>"`; `exact_prompt` equals `task` |
| `attachment: Option<AttachmentRecord>` | provenance, capabilities, process identities, timestamps (below); `None` for every native run |
| `outcome` while active | `lifecycle: Working, work_result: Pending, verification: NotRun, review: NotRequested, phase: Executing` |

`RunMode::Attached` needs **migration 21**, which rebuilds `runs` (see "Migration 21"
below). A binary built before this exists refuses a state directory that has already
been migrated to schema 21.

What attached Work cannot do: `dispatch refresh` (refused — see "Review of attached
work" below).

## S0 and confidence

S0 must be "the integration world as the agent saw it when its work began," because
every fact compares S0 against the world now, and Δ is workspace minus S0.

| Attach form | S0 | Δ | Provenance / confidence |
|---|---|---|---|
| Git worktree, wrapped or foreign | the tree of `merge_base(root HEAD, workspace HEAD)`, exported and turned into a private baseline repository under `runs/<id>/baseline` | everything in the workspace that differs from S0 (committed and uncommitted, untracked included, ignore rules honored) | `BaselineProvenance::GitMergeBase { commit }`, **`AttachConfidence::Full`**: edits made before attach are in Δ because S0 is a real commit |
| Git worktree, no merge base with the root | refused: *"no common history between the workspace and the integration root."* | — | — |
| Plain directory, wrapped only | a snapshot of the workspace taken at attach (`source::create_snapshot`) | workspace minus that snapshot | `BaselineProvenance::SnapshotAtAttach`, **`AttachConfidence::Partial`**: edits made *before* attach are invisible to Δ, and this is what the record says |
| Plain directory, foreign | refused: *"a plain directory can be attached only by wrapping the agent command."* | — | — |

The baseline is materialized on disk, not referenced by pointer, so it survives a
later rebase or garbage collection in the user's own repository, and `world::observe`,
`facts`, `gate` and `apply_validated` see exactly the `baseline_path`/`baseline_commit`
shape they already handle for native runs.

**Where this is visible today.** `attachment.provenance` and `attachment.confidence`
are stored on every attached run (`runs/<id>/metadata.json`) and carried in the
`attach.created` event payload. The *foreign* form of `attach` prints them once, at
creation:

```text
ATTACHED <run-id>
Workspace  <path>
Root       <path>
S0         commit <sha> (merge base), full confidence
Next: dispatch finish <run-id> when the agent is done
```

or, for a plain-directory snapshot, `S0  snapshot at attach, partial confidence`. The
*wrapped* form prints nothing at attach (rule: Dispatch is silent in that terminal —
see "Terminal and signals" below), so for wrapped attach the provenance line is not
printed at attach time; `dispatch explain <id>` shows it afterwards for either form.
For an attached run, `explain` prints an **Attached work** section instead of a
selection explanation (there is none): the workspace and root, the S0 line
(`S0: commit <sha> (merge base with the root)` or `S0: snapshot of the workspace at
attach; earlier edits are not attributed`), `confidence`, `agent`, `owner`, the
capability flags, and `finished: <reason>` once finished. It is followed by the
ordinary **Coherence** section when the world moved or the verdict is not `CONTINUE`,
and by the one-line `Coherence: CONTINUE — world unchanged` verdict otherwise. The
`serve` view's five fixed columns (below) do not include provenance. Programmatically,
read the run's stored record or its `attach.created` event.

## Δ and "finished"

- **Live Δ**, while a Work is still active: the watcher writes `delta-live.patch` on
  every world movement, exactly like a native allocation run's watcher, for mid-run
  verdicts only. It is never validated and never applied.
- **Finished** means Δ is frozen: `collect_diff(baseline, workspace, delta.patch)`,
  then `checks.verify` runs **in the workspace itself** — it is the user's own
  worktree, and the `--allow-unsafe-local` acknowledgement (or the one given at attach)
  carries the authority for that. The candidate's `checks` and `diff_stats` are set,
  `refresh_outcome` computes `work_result`/`verification`, and the run becomes
  `review: Pending`.
  - **Wrapped**: finishes automatically the moment the agent process exits, whatever
    its exit code. A non-zero exit still freezes Δ; the candidate becomes `Failed`
    only if Δ itself cannot be collected (an unreadable workspace), otherwise
    `Completed` with the agent's exit code recorded on the candidate.
  - **Foreign**: only an explicit `dispatch finish <id>`. There is no heuristic ("no
    edits for N minutes") that decides an agent is done; a human, or the agent's own
    driving script, must say so.
- After finish, the record behaves like any other Ready result: `check`, `accept`,
  `reject`, `auto_apply` apply to it unchanged.
- Δ attribution is per workspace — there is no per-process attribution inside one
  workspace — which is why same-checkout attach is refused and why one workspace holds
  at most one active attached Work at a time (`attach --workspace` on an
  already-attached workspace refuses with the existing id: *"workspace already
  attached as `<id>`."*).

## Capabilities: observe, signal, control, integrate

Four booleans, recorded on `attachment.capabilities` and enforced by the owner loop
that holds the Work:

| Capability | Wrapped attach | Foreign attach | Native run |
|---|---|---|---|
| **OBSERVE** (watch the world, evaluate, record verdicts) | yes | yes, by `serve` | yes |
| **SIGNAL** (surface stale/invalid state: events, `status`, the `serve` view) | yes | yes | yes |
| **CONTROL** (stop/cancel the process) | the wrapper's own child only, on the wrapper's own SIGTERM/SIGHUP | never | the existing supervisor |
| **INTEGRATE** (apply automatically when eligible) | only with `--auto-apply` at attach | only with `--auto-apply` at attach, executed by `serve` | session or invocation policy |

`observe` and `signal` are always `true`; `control` is `true` only for the wrapped
form; `integrate` mirrors `--auto-apply`.

**No foreign process is ever signaled or killed.** `--pid` on the foreign form and the
wrapper's own child PID are recorded purely for liveness (`identity_state`: PID plus
start time plus boot id) — never as a target for `kill`. `mid_run: stop` (the
coherence configuration key that lets Dispatch cancel a *native* attempt on an invalid
verdict) does not apply to attached work at all: neither the wrapped owner loop nor
`serve` implement it. A verdict on attached work is recorded, never enforced against
the agent. The agent never needs to know Dispatch exists; Dispatch writes no marker
files into its workspace.

## The three commands

```text
dispatch attach [--root <path>] [--workspace <path>] [--task <text>] [--agent <name>]
                [--allow-unsafe-local] [--auto-apply] -- <command> [args...]
dispatch attach --workspace <path> [--root <path>] [--pid <n>] [--task <text>]
                [--agent <name>] [--allow-unsafe-local] [--auto-apply]
dispatch finish <run-id> [--allow-unsafe-local]
dispatch serve  [--root <path>] [--json]
```

| Flag | Meaning |
|---|---|
| `--workspace <path>` | the worktree/directory to observe; default the current directory |
| `--root <path>` | the integration root; default the repository's main worktree for a linked Git worktree, required for a plain directory |
| `--task <text>` | describes the attached work; never sent to the agent |
| `--agent <name>` | free-text label (`claude`, `codex`, `cursor`, anything else); never guessed — `None` becomes `"external"` |
| `--pid <n>` | the foreign agent's process ID, for liveness only, never signaled |
| `--allow-unsafe-local` | explicitly allows `checks.verify` to run on the host at `finish` time |
| `--auto-apply` | sets `capabilities.integrate`: apply automatically once the work is ready and coherent |
| `-- <command> [args...]` | selects the **wrapped** form: everything after `--` is the agent command Dispatch spawns and owns for the session |
| `dispatch finish <run-id> [--allow-unsafe-local]` | foreign work only: freeze Δ, run `checks.verify` in the workspace, become an ordinary Ready result |
| `dispatch serve [--root <path>] [--json]` | one foreground process per integration root |

Whether `attach` is wrapped or foreign is decided by whether a command follows `--`:
with one, it is `run_wrapped`; without one, it is the plain `create` (foreign) form.

**Refusals, in the order they are checked**, with the exact messages `attach` prints
on `stderr` and exits non-zero:

1. workspace equals root — *"attach needs a separate worktree; run git worktree add"*
2. different repository — *"workspace belongs to a different repository than the
   integration root"*
3. Git workspace with no merge base — *"no common history between the workspace and
   the integration root"*
4. plain-directory foreign attach — *"a plain directory can be attached only by
   wrapping the agent command"*
5. `checks.verify` configured without the acknowledgement — *"finish runs your
   checks.verify on the host; pass --allow-unsafe-local"*
6. the workspace is already attached — *"workspace already attached as `<id>`"*

`dispatch finish` on a run that is not an active, single-candidate attachment refuses
with *"attached work `<id>` is not active"* or *"attached work `<id>` does not have
exactly one candidate."*

## Terminal and signals (the wrapped form)

The wrapped owner loop is deliberately not the `Executor` path that Dispatch uses for
runs it launches itself: no piped stdout/stderr, no token accounting, no timeout —
Dispatch does not own this agent's execution, only its observation.

- The agent inherits the wrapper's stdio and stays in the wrapper's own process group
  (`std::process::Command` is used without `process_group(0)`), so terminal job
  control and Ctrl+C behave exactly as if the shell had started the agent directly.
  Nothing is written to the terminal while the child is alive.
- Before the agent is spawned, the wrapper installs `tokio` signal handlers for
  **SIGTERM** and **SIGHUP** that forward the same signal to the child (`libc::kill`,
  best-effort — a race with the child's own exit is a no-op).
- Also before spawning, the wrapper sets **SIGINT** and **SIGQUIT** to `SIG_IGN` for
  its own lifetime, the same thing `time(1)` does, so a Ctrl+C during the run reaches
  the interruptible child and not the wrapper. Because `SIG_IGN` is inherited across
  `exec`, the child resets both to `SIG_DFL` in `pre_exec` — after `fork`, before
  `exec` — so it execs interruptible with the default disposition, exactly as if the
  shell had started it. Both dispositions are restored to default in the wrapper once
  the agent has exited.
- The SIGTERM/SIGHUP handlers are installed before the agent is spawned specifically
  to close a race: a signal arriving after `spawn` but before the handler was
  installed would otherwise kill the wrapper by its default disposition and orphan the
  agent instead of forwarding the signal to it.
- On exit, the wrapper finishes the Work (freezes Δ, runs `checks.verify`), applies it
  if `--auto-apply` was given, and only then prints its own single line — after the
  agent has fully released the terminal. The process exit code is the agent's own exit
  code, when the wrapper's own bookkeeping (creation, then finishing) succeeded; a
  bookkeeping failure exits 1 instead (creation failures never start the agent at all).

## `serve`

`dispatch serve [--root <path>] [--json]` loops until Ctrl+C, SIGTERM or SIGHUP.
Nothing is applied or launched on start or exit.

- **Identity and lock.** `locks/serve-<sha256(root)>.lock` (`flock`). A second `serve`
  on the same root refuses to start: *"already serving this root."* A dead `serve`
  releases the lock by itself — no stale socket, no PID file.
- **Every tick** (`coherence.poll_secs`, minimum 1 second):
  1. Read `world::signal(root)` and compare it with the previous tick's; adopt any
     orphaned Work (next bullet).
  2. If the world moved, or something was adopted this tick: re-evaluate every active
     attached run whose live owner state is not `Live` (a live wrapper is that
     wrapper's own business, not `serve`'s) — observe the world against *that run's*
     own baseline, evaluate, drop `analysis_uncertain`-only reasons the same way the
     mid-run watcher does (a person may be mid-edit), and persist through
     `apply::persist_verdict`.
  3. For every attached run that is now `Finished`, `Ready`, unreviewed, unapplied and
     has `capabilities.integrate`, call `auto_apply` (serialized by the same
     per-source lock every applier uses). After its own successful apply, `serve`
     re-observes immediately instead of waiting for the next tick — the apply is its
     own doorbell.
  4. Render the project view (below). This step runs on every tick, independent of
     whether `serve` itself changed anything this tick, because a run attached,
     finished or applied by *another* process changes the view without any verdict or
     apply of `serve`'s own.
- **Adoption.** An active attached run whose stored `owner_state` is `Live` but whose
  owner process (`attachment.owner`, the wrapper) is now gone (`identity_state`:
  `Gone`/`Reused`) is adopted: `serve` takes over its observation, commits
  `attach.adopted {owner_state: "adopted"}`, and sets `owner_state: Adopted`. A run
  whose operation lock is held by someone else is skipped, not forced — adoption never
  overrides a live owner. Nothing is finished, applied or relaunched by adoption
  itself; only observation resumes.
- **The view.** One line per run in this root — attached and native together, sorted
  by id — read from the stored DB projections, never recomputed for display beyond
  what the tick above already evaluated:

  ```text
  <first 8 of id> · <native|attached> <agent> · S0 <what it began against> · <verdict>[: <first reason>] · <state> · <verification>[ · review <review>]
  01M37J7H · native claude · S0 snapshot at eec41cc7 · unmoved · ready · checks passed · review pending
  01M37K2A · attached codex · S0 merge-base 1c9e0a47 (full) · REFRESH: fact_broken: pub fn validate… · blocked · checks passed · review pending
  ```

  The agent is the one that did the work, for native runs too. S0 is `snapshot at
  <project commit>` for native work on a Git source (the working tree as it was at
  that commit), `directory snapshot` for a plain directory, and `merge-base <commit>
  (full)` or `snapshot at attach (partial)` for attached work. The verdict is
  `CONTINUE`, `REFRESH` or `STOP` from the stored validity; `unmoved` when the source
  has not changed (including a result applied to an unmoved source); `overridden`
  when a human applied it over a REFRESH; or `not checked` when nothing has been
  evaluated yet. State is `working`, `question` (the run waits for your
  `dispatch answer`), `ready`, `blocked`, `applied` (`applied by auto-apply` when policy applied it) or
  `finished`.

  A run is shown while it is still active, or for up to an hour after it finished.
  On a TTY the block is redrawn in place; otherwise lines are appended. It **renders
  every tick but prints only what changed** since the last redraw — the whole block is
  only ever redrawn if at least one line in it differs from what is already on screen.
  `--json` instead emits one `{"type":"work", "run_id", "agent", "verdict", "state",
  "reason", "origin", "s0", "verification", "review", "applied_by"}` object per run
  whose displayed fields changed (the last five since 0.4.3), and one
  `{"type":"world", "digest"}` object whenever the observed world moved.

## The gate rule for attached runs

`coherence::gate` has one rule that exists only for attached work: **it never takes
the unmoved-fingerprint shortcut.** For a run Dispatch launched, the whole-tree
fingerprint recorded at creation was taken from the same tree S0 was copied from, so
an equal fingerprint today reliably means an unmoved world and the gate can skip
straight to `Legacy` (the existing all-or-nothing apply). An attached run's S0 is a
merge-base *commit*, while its `source_fingerprint` is the root's *working tree* at
attach time — the two can already differ even though nothing has moved since, so the
shortcut would silently skip evaluation and the integration checks. Attached runs
(`run.mode == RunMode::Attached`) therefore always evaluate through `evaluate_run` and,
when it says `Continue`, through L2 as usual; only `coherence.accept: strict` still
takes them straight to `Legacy`, exactly as it does for native runs.

This was found and fixed during a 0.4.0 real-agent trial (see "Evidence" in the [claims
table](coherence-validation.md)): the first real-agent attach applied without ever
computing a verdict because of this shortcut, and `dispatch explain` showed no
coherence section for an applied run — the regression test lives in
`tests/attach_cli.rs`.

## `empty_delta`

`auto_apply`'s eligibility stage (see [coherence.md](coherence.md#stage-1-eligibility))
skips a finished, otherwise-eligible run whose frozen `delta.patch` is empty, with
reason `empty_delta`, instead of applying it as "0 files changed." This matters
specifically for attached work: a foreign or wrapped agent that made no edits, or an
agent that only touched files the source's ignore rules exclude, still produces a
Ready result with nothing to apply. The run stays reviewable; nothing is recorded as
applied. This was also found during a 0.4.0 real-agent trial, fixed alongside the gate rule
above.

## Review of attached work

`dispatch accept`/`dispatch reject`, and the review menu's equivalents, record a
review of attached work exactly as they do for native work (since 0.4.1): one
`goal_feedback_revisions` row with the reasons and the verbatim explanation,
`outcome.review`, and a `review.accepted`/`review.rejected` event. The review and, on
accept, `apply_locked(..., ApplyAuthority::Human)` run under one hold of the run's
operation lock.

- `load_latest_unresolved_single` (the target of a bare `dispatch accept`/`dispatch
  reject` with no id) also returns an attached run whose review is still `Pending`.
- **The delivered-result guard.** A review is a judgment of a delivered result; an
  attached run that has not been finished yet has no Δ to judge. Reviewing an active
  attachment refuses: *"review requires a delivered result; finish attached work
  `<id>` first."*
- `dispatch refresh` has no meaning for attached work — there is no Dispatch task to
  relaunch — and is refused: *"attached work has no Dispatch task to refresh; finish
  or reject it."*

## Events

In addition to the existing coherence, application and review events (see
[coherence.md](coherence.md#events)), attached work commits:

| Event | When | Payload |
|---|---|---|
| `attach.created` | `attach` creates the Work (both forms) | `{"attachment": <AttachmentRecord>}` |
| `attach.started` | the wrapped form's agent process has been spawned | `{"agent_process": <ProcessIdentity>}` |
| `attach.finished` | `finish` (either explicit or on agent exit) freezes Δ | `{"reason": <FinishReason>}` |
| `attach.adopted` | `serve` takes over observation of an orphaned run | `{"owner_state": "adopted"}` |

An attached run also commits the ordinary `run.created`/`run.finished` events, and
`coherence.checked`/`coherence.invalidated`, `result.applied`/`application.failed`,
and `auto_apply.skipped`/`auto_apply.blocked` exactly as a native run does, all through
the shared `apply::persist_verdict` (see below).

## Shared verdict persistence

`orchestrator::apply::persist_verdict(state, db, run, validity)` is the one function
that turns a `Validity` into a stored verdict: it calls `remember_validity` (stamping
`first_invalid_at` the first time a run becomes invalid) and commits
`coherence.checked` (for `Continue`) or `coherence.invalidated` (otherwise) through the
same transition path every other event uses. The allocation-run mid-run watcher
(`native::apply_watch`), the wrapped attach owner loop, and `serve` all call it
directly; none of them re-implement `mid_run: stop` — `apply_watch` is the only caller
that still does, and only for native allocation runs.

## Durable versus recomputed

- **Durable**: the Work record (`runs/<id>/metadata.json`, including the
  `AttachmentRecord`), the materialized S0 baseline, the frozen `delta.patch`, every
  event, and `attachment.owner_state`/`finished_at`/`finish_reason`.
- **Recomputed on every use**: the world, symbol tables, facts and the verdict shown by
  `check`, `status` and `explain` — exactly as for
  native runs. `serve`'s view line reads the *last stored* verdict; it does not
  recompute one for display beyond what its own tick already evaluated and persisted.

## Restart and crash behavior

- **`serve` restart.** Active attached Work is read back from the database
  (`mode = attached`, `lifecycle != Finished`). The wrapper's stored `ProcessIdentity`
  (and the agent's, if known) is re-checked with `identity_state`: `ExactLive` is left
  alone (a live wrapper owns it); a stored `Live` owner that is now `Gone`/`Reused` is
  adopted (above); a foreign attachment with no owner, or an owner whose liveness
  cannot be told (`Unknown`), is observed but not marked adopted. Nothing is finished,
  applied or relaunched by a restart.
- **A wrapper crash** leaves its Work `Working` with a stale `owner_state: Live` until
  `serve` next observes that root and adopts it (or until a human runs `dispatch
  finish` directly), whichever happens first.
- **Reattaching** a process is not a separate operation: `attach --workspace` on a
  workspace that already has an active attached run refuses with that run's id
  (refusal 6 above) rather than creating a second Work for the same workspace.
- **SQLite unavailable or busy.** The wrapper keeps the agent running regardless and
  retries persistence on the next watcher tick, logging to `stderr` only after the
  child has exited; `serve` reports at most one error per tick and retries the next
  one. Neither ever kills or pauses an agent because of a persistence failure.
- **Crash consistency** is the same as for native runs: WAL plus `synchronous = FULL`;
  apply is not crash-atomic (a crash between `git apply` and the database update can
  leave a patched source with an unapplied-looking run — pre-existing, not specific to
  attach).
- **Concurrent projection writers.** An owner loop, `serve`, and every unlocked
  `load_run` reader that repairs a stale projection can all rewrite the same
  `metadata.json`. `state::write_atomically` stages each write through a **uniquely
  named** temporary file before an atomic rename, so two writers can never consume
  each other's temporary file mid-write; whichever rename lands last leaves a
  complete, current projection. (Found by the S6 simultaneous-integration scenario,
  which failed 30-50% of runs with a shared temporary name before this fix.)

## Coexistence with standalone runs

- `dispatch run`, `dispatch`, `dispatch accept` are unchanged when no attachment
  exists on a source.
- With attachments present, native runs and attached Work observe the same world,
  take the same per-source lock, and produce the same kinds of events. A native
  session's own watcher notices a landing by `serve` on its next poll, and vice versa.
- Nothing in the standalone path reads the `serve` lock or requires `serve` to be
  running.
- Attached Work appears in `status --json` like any other run, with
  `mode: "attached"`. Attach is always a foreground, human-typed command.

## Migration 21

`RunMode::Attached` requires schema 21 (`(21, "attached_work_mode", ...)` in
`src/db.rs`), which rebuilds `runs` the way migration 13 did: rename to `runs_v20`,
recreate `runs` with the same 28 columns plus a widened
`CHECK (run_mode IN ('legacy', 'routed', 'allocation', 'comparison', 'attached'))`,
copy every row across, drop `runs_v20`, then recreate migration 19's
`private_runs_source_window` index and `private_decision_immutable` trigger, which the
rename leaves bound to the old table name. `migrate()` turns `PRAGMA foreign_keys` off
for the duration of this migration (as it already does for 13), because otherwise
`DROP TABLE runs_v20` fails on any database with a row in `attempts`, `control_runs` or
`planned_goals` that references a run. `schema_version` becomes 21; opening a
historical-schema database creates a `dispatch.schema-20-*.db` backup first, as for
every other historical-schema upgrade. An older binary refuses a schema-21 state
directory, exactly as it already refuses any newer schema.

## Known limits

- No socket and no daemon: coordination is SQLite (WAL, revision-fenced projections)
  plus `flock` files; `serve` discovers new or changed Work on its next tick (at most
  `poll_secs`, default 10 s). This is adequate for work measured in minutes to hours,
  not for sub-second push updates.
- `serve` never finishes, launches, kills or refreshes anything. Wrapped attach and a
  human (`dispatch finish`) remain the only things that finish a Work.
- No foreign process is ever signaled. `--pid` and the wrapper's own child PID are
  liveness-only.
- `mid_run: stop` does not apply to attached work in this version; a `Stop` verdict on
  attached work is recorded, not enforced.
- One workspace holds exactly one active attached Work; there is no per-process
  attribution inside a workspace shared by more than one agent.
- A plain directory cannot honor `.gitignore` (same limit as native runs on a plain
  directory); its S0 confidence is `Partial` regardless.
- `validate_candidate_tree`'s existing size limits apply to `collect_diff` on an
  attached workspace exactly as they do to a native candidate; a very large worktree
  with build output can make `finish` slow. Measured on this repository's own
  1.7 GB `target/` worktree: about 25 ms for 3,968 files, which is why the limits are
  unchanged for this release.
- Attached results never become routing or allocation evidence; they carry no
  observed-model or resource data (`resource: None`, every model field `None`).
- The `serve` view does not print S0 provenance or confidence; `dispatch explain`
  does (see "Where this is visible today" above), and the stored record and the
  `attach.created` event carry them for programs.
