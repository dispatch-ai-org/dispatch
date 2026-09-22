# Dispatch 0.3 plan: safe auto-apply, then attached external work

Status: planning document, no production change. Written 2026-09-22 against
`main` at `73947eb` (tag `v0.2.0`). The repository is the source of truth; where
this document and the code disagree later, the code wins and this document is
corrected.

Audience: the integrator (Fable) and the implementation models that receive the
work packages in part 10. Parts 1 to 9 are the design; part 10 is what to
delegate; part 13 is the executive summary.

---

## 1. Verified current state

| Item | Verified value |
|---|---|
| Branch / commit | `main` at `73947eb` (merge of `worktree-coherence-messaging`); tag `v0.2.0` at `e87d44b` |
| Crate version | `0.2.0`, edition 2024 |
| Schema version | 20 (`bounded_planning`); no coherence migration |
| Production source | 42,450 lines in `src/` (largest: `orchestrator.rs` 4,948; `db.rs` 4,123; `sync.rs` 2,498; `admission.rs` 2,053; `source.rs` 2,022) |
| Tests | 13,308 lines in `tests/`; 555 `#[test]` functions across unit and integration suites |
| `cargo fmt --check` | clean |
| `cargo test` | 647 passed, 0 failed, 2 ignored (timing-only matrix and one PTY fixture) |
| `cargo clippy --all-targets -- -D warnings` | clean |
| CI | `ci.yml` runs fmt, `cargo test --locked -- --test-threads=4`, clippy on ubuntu-22.04; `release.yml` builds macOS arm64 and Linux x86_64 on `v*` tags, verifies the tag equals the crate version, packages, smoke-tests `dispatch version` |
| Local dogfood state | `~/.dispatch` exists with 14 runs, `resources.yml`, `locks/` |
| Dependencies relevant here | `tokio` 1.53 with `full` (Unix sockets available if ever needed), `crossterm` 0.29 (parses Shift+Tab as `KeyCode::BackTab`), `ratatui-textarea` 0.9.2 (maps `BackTab` to a Tab insert, so it must be intercepted first), `libc` for `flock`, `rusqlite` bundled |

Every concept the assignment listed was checked against the code. All exist, with the
precise shapes recorded in part 2. Deviations from the assignment's description are in
part 3.

---

## 2. Architecture map (as implemented)

### 2.1 Work / run domain

There is no separate `Work` type. The unit is `RunRecord` (`src/models.rs`), with:

- **S0**: `baseline_path` (a private Git repository under the run directory, made by
  `source::create_snapshot` from the working tree, including dirty and untracked files),
  `baseline_commit`, and `source_fingerprint` (the strict whole-tree hash).
- **Δ**: the sole candidate's `diff_path` (`git diff --binary --full-index --no-renames`
  against S0, collected by `source::collect_diff` with a temporary index). Mid-run, a
  work-in-progress patch from `source::snapshot_delta`.
- **MustHold**: `CoherenceRecord.facts` exists but is never populated; facts are always
  recomputed from (S0, Δ) by `coherence::facts::derive_facts`.
- **Validity**: `CoherenceRecord.validity` (latest stored verdict), `first_invalid_at`,
  `refreshed_from`. `Decision` is `Continue | Refresh | Stop`.
- **Outcome**: `RunOutcome { lifecycle, work_result, verification, review, application, phase, waiting_on }`, version 1.
  - `VerificationState`: `NotConfigured | NotRun | Passed | Failed | Inconclusive`.
  - `ReviewState`: `NotRequested | Pending | Accepted | Rejected | Deferred`.
  - `ApplicationState`: `NotApplied | Applied | BlockedBySourceDrift | Failed`.
- `RunMode`: `Legacy | Routed | Allocation | Comparison`. SQLite enforces this set with a
  `CHECK` constraint on `runs.run_mode` (migration 13).
- `RunStatus` (older, still authoritative for `apply`): `ReadyForEvaluation | Evaluated | Applied | ...`.

### 2.2 Run lifecycle

`orchestrator::run_dispatch` (`src/orchestrator.rs:643`): validate → discover and freeze
config (`config.snapshot.yml` in the run dir) → `create_snapshot` → `run.created` →
baseline checks → then one of three drivers:

- `phase3::drive` for allocation runs (the normal path; one candidate, bounded
  recovery, clarification questions, the mid-run **watcher**);
- `planned::drive` for `--plan`;
- the legacy comparison loop for explicit `--harnesses`.

Finish: `refresh_outcome` sets `work_result: Ready`, `review: Pending`, `phase: Reviewing`
when a candidate completed; `run.finished` is committed. The process then returns the
record to its caller (CLI, TUI, control session, refresh).

### 2.3 Coherence engine (`src/coherence.rs`, `coherence/*`)

Pure and read-only. `evaluate(world, work)`:

1. L0: world unchanged or empty patch → `Continue`; `git apply --check --reverse` succeeds
   → `Stop` (`already_applied`); `git apply --check` fails → `Refresh` (`patch_conflict`).
2. L1: `facts::check` over Modified / Referenced / File facts for `.rs` and `.py`
   (tree-sitter), file-level fallback elsewhere; `analysis` is `symbols` or `files_only`.
3. L2 (`integration::verify_integration`): only at accept, only when the world moved,
   `coherence.integration_checks` is true, `checks.verify` is non-empty, and, on the
   local backend, the run itself recorded `unsafe_local`. It copies the current source
   (non-ignored files only) to a scratch tree under the run dir, applies Δ, runs the
   checks with the run's execution config, and returns `analysis: integration` on pass or
   `Refresh` on any failure or infrastructure error. **If it is skipped for lack of
   authority, the verdict passes through unchanged**, that is, a moved world can be
   applied on L0/L1 evidence alone. This is a deliberate property of human accept and
   the key thing auto-apply must not inherit.

`world::observe` hashes the current non-ignored tree against S0 and yields
`digest` (content-addressed, `dispatch-world-v1`) and `changes`. `world::signal` is the
cheap change doorbell (Git `HEAD`, `git status --porcelain=v2 -z`, size and mtime of the
listed files).

`live_validity(run)` recomputes the verdict for a finished, unapplied, single-candidate
run; `check`, `status`, `explain` and the control `result` all use it. Nothing trusts a
stored verdict at accept.

### 2.4 Accept, review and apply (`src/orchestrator.rs:3343-4216`)

- `review_target` (TUI): per-run `.operation.lock` (`flock`, non-blocking), exact
  `state_revision`, sole candidate, `Finished`/`Ready`, `review == Pending`, and
  `commands::ensure_machine_review_denied()` (any task-local machine actor is refused).
- `review_delivery(accept)` and `accept_or_reject_latest` (CLI): first record the human
  decision (`record_allocation_feedback_locked` → `goal_feedback_revisions` +
  `review.accepted|rejected` event + `outcome.review`; or `record_routing_evaluation_locked`
  for routed runs), then, on accept, `apply_locked`.
- `apply` (`dispatch apply <run> <candidate>`, hidden): per-run lock, then `apply_locked`
  with no feedback recorded.
- `apply_locked`: requires `RunStatus::ReadyForEvaluation | Evaluated`; takes the
  **per-source lock** `locks/source-<sha256(source_path)>.lock` (non-blocking); calls
  `coherence::gate`; on `Legacy` → `source::safe_apply` (fingerprint fence), on
  `Compatible(validity)` → `source::apply_validated(run, label, validity.world_digest)`
  (digest fence checked before the dry run and again after it), on `Blocked` →
  typed `CoherenceBlocked`. Success sets `applied_candidate`, `status: Applied`,
  **`review: Accepted`**, `application: Applied`, commits `result.applied` with
  `coherence` when the world moved. Failure sets `BlockedBySourceDrift` (typed block, or
  message matching) or `Failed`, stores the validity, commits `application.failed`.
- `gate`: unreadable `config.snapshot.yml` → `Legacy` (strict); `accept: strict` or
  fingerprint equal → `Legacy`; else `evaluate_run` then L2 → `Compatible | Blocked`.
  It runs while both locks are held and holds no SQLite transaction; L2 checks are not
  cancellable.

### 2.5 Mid-run watcher (`coherence/watch.rs`, `phase3::apply_watch`)

One `Watcher` per allocation attempt: `signal` every `poll_secs`; on movement, `observe` +
`snapshot_delta` + `evaluate` (L0/L1 only, `analysis_uncertain` dropped). It sends
`WatchMsg` over a channel; the owning loop applies it with `remember_validity` and commits
`coherence.invalidated | coherence.checked` (`coherence.stopped` then cancel in
`mid_run: stop`). Exactly one writer of the run's revision. Planned, legacy and
comparison runs have no watcher.

### 2.6 Refresh

`refresh_request` builds a new `RunRequest` for a finished, ready, unapplied run
(fresh reasons in a fixed addendum; same launch choices; acknowledgements required
again; `coherence.refreshed_from`). Always a human-typed launch.

### 2.7 Persistence and journal

- SQLite (`~/.dispatch/dispatch.db`), WAL, `synchronous=FULL`. `runs.run_projection_json`
  is the committed projection; `runs.state_revision` is bumped by every
  `commit_transition` with an optimistic check ("run state revision changed
  concurrently"). Events live in `events(run_id, sequence, event_type, payload_json,
  actor, ...)`; every payload carries the run's `outcome`. Actor is `machine:<principal>`
  under a control session, otherwise `orchestrator`. There is no `human` actor value
  today: human review events are recorded with actor `orchestrator`.
- Per-run projections `runs/<id>/metadata.json` and `events.jsonl` are repaired from the
  DB on load (`State::load_run`).
- `validate_terminal_write` forbids reopening cancelled or rejected work and mutating
  completed attempt evidence.
- Locks are `flock` files: `runs/<id>/.operation.lock`, `locks/source-<key>.lock`,
  `control-grants/<principal>.lock`. The non-Unix branch uses `create_new` files.

### 2.8 Control protocol (`src/control.rs`, `src/commands.rs`)

Foreground stdio JSONL, human-provisioned grant key passed as an inherited fd, one
writable session per principal (grant lock), fixed `Scope` (source, state root, config
digest, profiles, timeout, invocations, unsafe-local, factual delegation, plan). Operations:
`initialize, submit, status, result, capacity, events, subscribe, await, answer, cancel,
recover, artifact`. Review and application are explicitly denied to machine actors.
`result` carries a read-only `coherence` summary. Receipts make mutations idempotent.
Observation is DB polling (`event_page`, 25 ms), not notifications.

### 2.9 TUI (`src/presenter.rs`, `presenter/inspection.rs`, `presenter/handoff.rs`)

`session()` loops: goal prompt (`/resources`, `/checks`, `/plan`, `f` attest, `i`
details) → `run_goal` → `ui.work(run_dispatch)` (only Ctrl+C / Ctrl+D handled while
working; other input is drained and never queued) → `review_goal` → `review_action`
(typed letters + Enter: `d e a r n i`) → `review_delivery`. The `Editor` wraps
`ratatui-textarea`; the `draw(body, editor, hint)` API takes a hint string rendered in the
last row. Mode state is in-memory on `Ui`. `handoff.rs` shows the pattern for handing the
terminal to an external program and taking it back.

### 2.10 Process lifecycle

`executor.rs`: children spawn in their own process group, piped stdio, cancellation
token, kill of the group and confirmation. `admission::ProcessIdentity { pid, start,
boot, process_group }` with `identity_state` (`ExactLive | Gone | Reused | Unknown`)
is the reusable liveness primitive. Recovery of a dead foreground owner is explicit and
eligibility-checked (`recover_checkpoint`, `repair_abandoned`).

### 2.11 Sources and identity

`sources(path, kind, git_head, fingerprint)` keyed by canonical path. `inspect_source`
distinguishes `Git`, `GitWorktree` (via `commondir`), and `Directory`. "Latest run for
this source" is a path equality scan (`load_latest_for_source`). Two worktrees of one
repository are two unrelated sources today.

---

## 3. Discrepancies between the assignment's description and the code

| Assignment wording | What the code does |
|---|---|
| "MustHold / relied-upon facts" stored on Work | `CoherenceRecord.facts` is declared but never written; facts are recomputed every time from S0 and Δ. Correct per the design rule "recomputable, no stored index". Keep it that way. |
| "explicit human review separate from application state" | Mostly true, with one leak: `apply_locked` sets `review: Accepted` unconditionally, so `dispatch apply` (no human decision recorded) shows as accepted. Auto-apply must not reuse that line. |
| "existing application locks/fencing" | Present, but all lock acquisition is **non-blocking** (`LOCK_NB`). A second applier fails immediately instead of waiting. Auto-apply needs a bounded blocking variant. |
| "accept-time integration checks" | Skipped without error when the run lacks `unsafe_local` authority, when `checks.verify` is empty, or when disabled. For human accept this is fine; auto-apply must treat a skipped L2 on a moved world as insufficient evidence. |
| "mid-run coherence observation" | Allocation runs only; planned and legacy runs have none. Attached work will need the same watcher outside `phase3`. |
| "source-drift-safe application" | Digest-fenced (`apply_validated`) before and after the dry run; the window between the second check and the real `git apply` is inherent and documented. Apply is not crash-atomic (patched source, unapplied run) and remains so in this plan. |
| "control protocol" as a candidate service substrate | It is a per-grant foreground stdio interface with human review excluded by design; not a persistent service and not a general IPC layer. |
| "session" | The TUI has no persisted session object. Session state is the `Ui` struct's memory. |
| Events have a human actor | They do not; human review events carry actor `orchestrator`. Recorded as a known gap; not fixed in this plan. |
| README says `dispatch` selects "the best available agent" from evidence | True; unrelated to this plan and left alone. |

---

## 4. KEEP / EXTEND / HIDE / DEPRECATE / REPLACE

| Subsystem | Verdict | Reason |
|---|---|---|
| Coherence evaluator (`evaluate`, facts, symbols, world, integration) | **KEEP** | Reused verbatim by auto-apply and attached work. No fork. |
| `source::apply_validated` / `safe_apply` digest and fingerprint fences | **KEEP** | They are the final-boundary fence. |
| `apply_locked` / `gate` | **EXTEND** | Add an authority parameter, a structured outcome, a bounded blocking lock, and a stricter authorization predicate for policy application. Move to `orchestrator/apply.rs` (pure move plus the extension) to stop `orchestrator.rs` growing. |
| `OperationLock` | **EXTEND** | Add `acquire_wait(path, timeout)`. |
| Mid-run `Watcher` / `apply_watch` | **EXTEND** | Reuse for attached work; factor the persist step so `phase3` and the attach owner loop share it. |
| Review recording (`record_allocation_feedback*`, `record_routing_evaluation*`) | **KEEP** | Auto-apply never calls them. Human evidence stays human. |
| `refresh_request` | **KEEP** | No automatic refresh. Optional TUI entry point later. |
| Control protocol | **KEEP** | Untouched by the core work. Optional additive `auto_apply` submit field behind a new grant capability in a later packet. Not used as service IPC. |
| TUI | **EXTEND** (small) | Mode toggle, ambient indicator, one contextual review action, post-finish auto-apply flow. No dashboard. |
| CLI | **EXTEND** | `run --auto-apply`, `refresh --auto-apply`; later `attach`, `finish`, `serve`. |
| SQLite schema | **EXTEND** | Migration 21 only to admit `run_mode = 'attached'`. Everything else lives in projection JSON with `#[serde(default)]`. |
| Planned execution, admission, capacity, allocation, sync, private evidence | **KEEP** | Untouched. Attached work has no admission and produces no sync payload. |
| `sources` table / path identity | **KEEP** | Integration root is a path; repository identity is recorded on the attachment, not in a new table. |
| Anything to HIDE / DEPRECATE / REPLACE | none | No working infrastructure is replaced. `dispatch apply` keeps its behavior (a human-typed command). |

---

## 5. Auto-apply: design

### 5.1 Name and semantics

The public and internal term is **auto-apply**. It is never "auto-accept". Acceptance is a
human quality judgment; application is a mechanical source change. Auto-apply performs
the second without the first. The truthful state of an auto-applied run is:

```text
Verification: passed
Coherence:    CONTINUE (integration checks passed on the merged tree)   or   CONTINUE (world unchanged)
Application:  applied by auto-apply
Review:       not performed
```

Rules that follow:

1. Auto-apply never writes `review.accepted`, never calls `save_goal_feedback` or
   `save_routing_human_evaluation`, never changes `outcome.review`. `review` stays
   `Pending`.
2. A human may still accept or reject an auto-applied run afterwards. That records the
   review only (no second apply; rejection does not revert the source and says so). This
   post-hoc review is the signal for "false CONTINUE identified by a human".
3. The application actor is recorded: `RunOutcome.applied_by: Option<AppliedBy>` with
   `human | auto_apply` (additive, `#[serde(default)]`; `None` on old records and on
   unapplied runs). `ApplicationState::Applied` keeps meaning "the source was patched".
   The 12 existing `ApplicationState` match sites do not change meaning.
4. `apply_locked` gains `ApplyAuthority::{Human, AutoApply}`. Only `Human` sets
   `review: Accepted` (preserving today's behavior for `accept` and the hidden `apply`).
5. `is_ready_unapplied` already excludes applied runs, so `check`/`refresh` cannot act on
   an auto-applied run.

### 5.2 Eligibility and authorization policy (the explicit answers)

Auto-apply is attempted only by the process that owns the run's foreground execution,
immediately after `run_dispatch` returns, and only if that owner's live policy says so.
The attempt has three stages, each recorded.

**Stage 1, eligibility (no external work).** All must hold, else `auto_apply.skipped {reason}`:

| Condition | Rule | Skip reason |
|---|---|---|
| Delivered | `lifecycle Finished`, `work_result Ready`, exactly one candidate, `review Pending`, `application NotApplied`, `RunStatus::ReadyForEvaluation` | `not_ready` |
| Verification configured | `checks.verify` non-empty in the frozen config | `verification_not_configured` |
| Verification result | `outcome.verification == Passed` | `verification_failed`, `verification_inconclusive`, `verification_not_run` |
| Local authority for checks | if backend is `local`, `environment.unsafe_local` is true | `integration_checks_unavailable` |
| Planned delivery | allowed; `planning::verify_delivery` must pass | `delivery_unverifiable` |
| Comparison runs | never (several candidates) | `not_sole_candidate` |

"Verification not configured" is **never** eligible in this version. Without checks there
is no L2 and the only evidence would be "the agent exited zero". A human looks at those.
No knob relaxes this; if dogfood shows it is too strict, that is a product decision to
take later with evidence.

**Stage 2, gate under the locks.** Acquire the per-run lock and the per-source lock with
a bounded wait (part 5.3), then `coherence::gate`. The gate's result is authorized for
policy application only when:

| Gate result | Authorized? |
|---|---|
| `Legacy` with `fingerprint_tree(source) == source_fingerprint` (byte-identical world) | yes, via `safe_apply` |
| `Legacy` because `accept: strict` and the world moved | no: `blocked { reason: strict_mode_drift }` |
| `Legacy` because `config.snapshot.yml` is unreadable | no unless fingerprint equal (same as row 1); else `blocked { reason: config_unreadable }` |
| `Compatible(v)` with `!v.world_changed` | yes |
| `Compatible(v)` with `v.world_changed && v.analysis == Integration` | yes |
| `Compatible(v)` with `v.world_changed` and analysis `symbols`/`files_only` (L2 skipped or disabled) | **no**: `blocked { reason: integration_evidence_missing }` |
| `Blocked(Refresh ...)` including `analysis_uncertain` and `integration_check_failed` | no: `blocked`, verdict stored, `application: BlockedBySourceDrift`, review stays `Pending` |
| `Blocked(Stop ...)` | no: `blocked`, verdict stored; nothing is rejected automatically |

This yields the safety invariant: **a policy applies only a verdict whose evidence is
complete for the exact world it names: an unmoved world with passed candidate
verification, or a moved world whose merged tree passed the project's own checks.**

**Stage 3, apply.** `apply_validated` (or `safe_apply`) with the gate's digest. On the
typed fence failure "source changed during apply validation" (someone edited during L2),
re-run stage 2 **once**; a second fence failure ends as
`blocked { reason: world_moved_during_validation }`. No further loop, no sleep-and-retry.

Other explicit answers:

- **REFRESH**: not applied, never refreshed automatically. The run stays reviewable; the
  UI names `dispatch refresh`. Refresh spends an invocation and stays human-typed.
- **STOP**: not applied; the UI names `dispatch reject`. No automatic rejection.
- **Apply races another completed run**: serialized by the per-source lock; the loser
  evaluates against the winner's world after acquiring the lock (part 5.3).
- **Application failure** (Git error after the fence): `application.failed` with
  `applied_by: auto_apply`, `application: Failed`; policy is unchanged for later work
  (a Git failure is per-run, not a reason to silently turn the mode off), but the TUI
  shows the failure prominently and drops into review.
- **Process restart**: nothing. Policy lives in the owning process's memory. A run that
  finished under auto-apply but whose owner died before applying is an ordinary pending
  result. No stored flag authorizes future application. Recovery never applies.
- **Scope of the policy**: it is a live property of the foreground owner: the TUI session
  (toggle), or the CLI invocation (`--auto-apply`). It is not in `dispatch.yml` and not
  on the run record. What is recorded is the evidence of what happened
  (`applied_by`, events). One model, three surfaces (TUI, CLI, later control).

### 5.3 Final-boundary concurrency design

Existing fences are sufficient once acquisition blocks:

```text
acquire run lock (wait ≤ T)                   runs/<id>/.operation.lock
acquire source lock (wait ≤ T)                locks/source-<sha256(source_path)>.lock
observe world now  → digest D                 coherence::gate → evaluate_run
validate (L0, L1, then L2 on a scratch copy)  while holding both locks, no SQLite txn
apply_validated(D): observe == D, dry run, observe == D, git apply
commit result.applied (revision-fenced)       then release both locks
```

- `T` for the source lock is the run's `execution.timeout_secs` (L2 of another applier can
  take that long); for the run lock 5 s (a human review may hold it; the CLI reports "run
  has a foreground owner").
- `OperationLock::acquire_wait` uses `flock(LOCK_EX)` on a helper thread with a deadline
  (or a 100 ms `LOCK_NB` poll loop; either is acceptable, the poll loop is simpler and
  portable to the non-Unix branch).
- The lock key stays `sha256(source_path)`. Two sessions on one checkout share it. Two
  worktrees of one repository are different sources and rightly do not share it until
  attached work introduces the integration root (part 6).
- The residual window between the post-dry-run digest check and the real `git apply` is
  inherent to applying onto a live working tree and is unchanged.
- Nothing new is held across a SQLite write transaction. The two commits (`auto_apply.*`
  events and `result.applied`) are ordinary `commit_transition` calls.

The dogfood race (two sessions finish together): the second `auto_apply` waits on the
source lock, then observes the world the first one produced, and its verdict is by
construction about that world.

### 5.4 Data model and events

- `RunOutcome.applied_by: Option<AppliedBy>` (`human | auto_apply`).
- `RunResult.auto_apply: Option<AutoApplySummary { outcome: applied|blocked|skipped|failed, reason: Option<String>, coherence: Option<CoherenceSummary> }>` (JSON/JSONL output, additive).
- Events (all through `commit_transition`, so they carry `outcome`):
  - `auto_apply.skipped` `{reason}`
  - `auto_apply.blocked` `{reason, coherence?}` (verdict stored via `remember_validity`; `application: BlockedBySourceDrift` only when the gate blocked on a verdict, not for `verification`-type skips)
  - `result.applied` `{files_changed, coherence?, applied_by: "auto_apply"}` (same type as the human apply so every "applied" consumer sees it; `applied_by` distinguishes)
  - `application.failed` `{application, error, applied_by: "auto_apply"}`
- No new tables. No sync envelope change: auto-applied runs create no evaluation or
  feedback record until a human reviews them.

### 5.5 TUI interaction

Modeled on the useful part of Claude Code's permission modes: a mode, always visible,
cheap to flip, flippable mid-work, safe by default.

- `Ui.auto_apply: bool`, default `false` on every session start; never persisted.
- **Shift+Tab** toggles it on every screen: goal prompt, while working, review menu.
  `KeyCode::BackTab` is intercepted in `Editor::event`, in `Ui::work`'s input arm and
  in `review_action`'s key loop before the widget sees it (the textarea would insert a
  tab). `/auto-apply` (`on|off|toggle`) at the goal prompt does the same and is the
  path for `--plain` mode, where raw keys are not readable.
- **Indicator**: the hint row (last line) always carries the mode as its first segment:
  `⏸ review before apply · Shift+Tab` (secondary color) or
  `⏵⏵ auto-apply on · Shift+Tab to pause` (warning color). ASCII: `[review]` /
  `>> AUTO-APPLY ON`. In plain mode the toggle prints one line and prompts include
  `(auto-apply on)`. When on, the goal heading line is prefixed the same way, so the mode
  is visible both in the frame and in scrollback.
- **Contextual escalation** in the review menu: `[aa] Accept & apply, then auto-apply`.
  It records the human accept exactly as `a` does (this is a real human review), applies,
  and then sets `ui.auto_apply = true`. It never reinterprets a past result.
- **Post-finish flow** in `run_goal`: if the run is Ready and `ui.auto_apply`, call
  `orchestrator::auto_apply` inside the `present` scope with `block_in_place` (so
  committed events still reach the live view) and a static body "Auto-apply: validating
  against the current source… (checks on the merged tree can take as long as your
  verification)". Outcomes:
  - applied: commit one projection with the line `Auto-applied · review not performed ·
    Coherence: CONTINUE — …`; return to the goal prompt; `ui.reviewed` stays `None` (no
    attest offer, because no human review happened).
  - skipped / blocked / failed: commit the notice ("Auto-apply skipped: no checks
    configured", "Auto-apply blocked: REFRESH — fact_broken …") and enter `review_goal`
    as today with the notice shown above the actions.
- Ctrl+C during auto-apply: not cancellable (L2 is not cancellable today); the body says
  so. Bounded by the checks' timeouts.
- Review of an already auto-applied run (later, via `i` details or CLI): shows
  `Auto-applied · review pending`; `a`/`r` record review only.

What is not built: no settings screen, no persisted preference, no per-project default,
no portfolio view.

### 5.6 CLI and headless

- `dispatch run --auto-apply "<task>"` and `dispatch refresh --auto-apply [run]`. After the
  run returns Ready the same process calls `auto_apply` and prints one line:
  `Auto-applied Candidate A to <source> (2 files changed). Review not performed.` or
  `Not applied automatically: <reason>. Review with dispatch check/accept <id>.`
- Exit codes with the flag: `0` applied; new `6` "ready, not applied automatically
  (skipped or blocked)"; other codes unchanged. `--json`/`--jsonl` carry
  `result.auto_apply`.
- No `dispatch.yml` key. No environment variable.
- Two headless invocations on one checkout are the second dogfood shape; the locks make
  them safe.

### 5.7 Control protocol

Unchanged in the core packets. If later needed: `submit { auto_apply: true }` accepted
only when the grant was issued with a new `--allow-auto-apply` (an INTEGRATE authority in
the vocabulary of part 6.6); the run's `applied_by` is `auto_apply` and the events carry
actor `machine:<principal>`; review remains denied. Machine callers still cannot
impersonate human review because the review functions are never reachable from a
policy apply. This is an additive optional packet (A7), not on the critical path.

### 5.8 Returning to a safe state

Every non-applied outcome leaves the run reviewable and the source untouched. The mode
itself is only ever turned on by a human keystroke or flag in the owning process; there
is no persisted "on" state for a later process to inherit.

---

## 6. Attached external work and the repo-scoped service

### 6.1 The shape that fits this codebase

The unit that already keeps native work coherent is an **owner loop**: one foreground
process that holds the run's operation lock, runs the watcher, persists verdicts through
`commit_transition`, and applies through `apply_locked`. Attached work should get the
same owner loop, not a different mechanism. Two consequences:

1. **`dispatch attach -- <agent command>`** (wrapped attach) is itself the owner loop of
   that Work: it creates the Work record, starts the agent with the terminal, watches the
   integration world while the agent runs, and on exit freezes Δ, verifies, and applies if
   authorized. Claude stays Claude; Dispatch is silent in that terminal.
2. **`dispatch serve`** is the owner loop for Work that has no live owner: foreign
   attachments (`--workspace` without a command), and wrapped Work whose wrapper died. It
   also renders the project view. It is one foreground process per integration root.

No daemon is required for standalone runs, for wrapped attach, or for the TUI. `serve` is
required only to keep foreign attachments observed and to apply them.

Native runs and attached Work coexist by sharing **state** (SQLite, run dirs) and
**locks** (per-source apply lock), not by routing native runs through the service. That
is what makes behavior consistent regardless of who launched the agent: both paths call
`evaluate`, `gate`, `auto_apply`.

### 6.2 Repository and world identity

- **Integration root**: a checkout path (canonical). It is the `source_path` of every
  Work attached to it, so the world, `observe`, and the per-source apply lock are the
  ones native runs already use. For a Git repository it defaults to the main worktree
  (`git worktree list --porcelain`, first entry) when `attach` runs inside a linked
  worktree; `serve` uses its cwd or `--root`.
- **Repository key**: `sha256(canonical git-common-dir)`. Recorded on the attachment.
  `attach` refuses a workspace whose common dir differs from the root's ("different
  repository"). Two roots of one repository (rare) are two worlds by design; the
  attachment names which one it targets.
- **Same-checkout attach** (workspace == integration root) is refused in this version with
  the message to create a worktree. Δ attribution and self-application are undefined
  there; this is stated, not papered over.
- **Plain directories**: wrapped attach only, workspace must be a separate directory
  (usually a copy); S0 is a snapshot at attach time (part 6.4). Foreign attach requires
  Git.

### 6.3 Attached Work model: reuse `RunRecord`

Attached Work is a `RunRecord` with `mode: Attached` and one candidate. This is the single
decision that keeps the coherence implementation from forking: `check`, `explain`,
`status`, `accept`, `reject`, `apply`, `auto_apply`, the watcher, `gate` and
`apply_validated` all take a `RunRecord` and a candidate label.

Field mapping:

| RunRecord field | Attached value |
|---|---|
| `source_path`, `source_kind`, `source_git_head`, `source_fingerprint` | the integration root at attach time (fingerprint is informational; the gate uses the digest path) |
| `baseline_path`, `baseline_commit` | S0 materialized under `runs/<id>/baseline` (part 6.4) |
| `candidates[0]` | `workspace_path` = the external worktree; `diff_path` = `runs/<id>/delta.patch` (live copy overwritten by the watcher, frozen at finish); `harness_id` = `claude | codex | cursor | external`; `checks` filled at finish |
| `attempts[0]` | one record, `role: "attached"`, `harness_id` as above, `resource: None`, models unknown (`None`, never guessed) |
| `environment` | copied from the root's `dispatch.yml` at attach; `unsafe_local` = the `--allow-unsafe-local` acknowledgement if `checks.verify` is non-empty |
| `phase3` | `None` (no invocation budget, no admission, no questions) |
| `task` | `--task` text if given, else `"attached work in <workspace>"` |
| `attachment: Option<AttachmentRecord>` (new, `#[serde(default)]`) | provenance, capabilities, process identities, timestamps (part 6.4, 6.6) |
| `outcome` | `Working/Pending/NotRun/NotRequested` while active; `refresh_outcome` at finish |

`RunMode::Attached` requires **migration 21** rebuilding `runs` with the widened `CHECK`
(the same pattern as migration 13). Old binaries refuse newer schemas already, so the
upgrade/rollback story is the existing one.

What attached Work cannot do: `refresh` without a task (refused with a message);
allocation feedback (`record_allocation_feedback_locked` requires `Allocation`; the
review path must record attached review through `save_goal_feedback` with a distinct
event, or be limited to `accept`/`reject` recording `review.*` events only; the packet
S3 decides with Fable, the default is "review events only, no routing evidence", so
attached results never become routing evidence).

### 6.4 Workspace and baseline (S0) model

S0 must be "the integration world as the agent saw it when its work began", because
facts compare S0 against the world now, and Δ is workspace minus S0.

| Attach form | S0 | Δ | Provenance / confidence |
|---|---|---|---|
| Git worktree, wrapped or foreign | tree of `merge-base(root HEAD, workspace HEAD)`, exported with `git archive <commit> | tar` into `runs/<id>/baseline` and turned into a private baseline repo by the existing `initialize_internal_repository` | `snapshot_delta(baseline, workspace)`: committed and uncommitted changes since S0, untracked included, `.gitignore` honored | `git_merge_base { commit }`, **full**: edits before attach are in Δ because S0 is a real commit |
| Git worktree, HEAD unrelated to the root (no merge base) | refused | | |
| Plain directory, wrapped | snapshot of the workspace at attach (`create_snapshot`) | workspace minus that snapshot | `snapshot_at_attach`, **partial**: pre-attach edits are invisible and the record says so |
| Plain directory, foreign | refused (no honest S0) | | |

The baseline is materialized, not referenced, so `world::observe`, `facts`, `gate` and
`apply_validated` see exactly the `baseline_path`/`baseline_commit` shape they already
handle, and a later rebase or garbage collection in the user's repository cannot remove
S0.

Baseline provenance is stored on the attachment and printed by `explain` and the serve
view ("S0: commit abc123 (merge base)", "S0: snapshot at attach; earlier edits not
attributed").

### 6.5 Δ and "finished"

- **Live Δ** while active: the watcher writes `delta.patch` on every world movement (same
  as native runs' `delta-live.patch`), used for mid-run verdicts only.
- **Finished** means Δ is frozen: `collect_diff(baseline, workspace, delta.patch)`, then
  `checks.verify` run **in the workspace itself** (it is the user's own worktree; the
  wrapper carries the human's acknowledgement), results on the candidate, then
  `refresh_outcome` → `Ready`, `review: Pending`.
  - Wrapped: at agent exit (any exit code; a non-zero exit still freezes Δ and marks the
    candidate `Failed` if the workspace is unreadable, else `Completed` with the exit code
    recorded).
  - Foreign: explicit `dispatch finish <id>` (name open; "land" and "ready" were
    considered; "finish" mirrors the run event). No heuristic ("no edits for N minutes")
    decides that an agent is done.
- After finish the record is an ordinary Ready result: `check`, `accept`, `reject`,
  `auto_apply` apply unchanged.
- Δ attribution is per workspace, which is why same-checkout attach is refused.

`collect_diff` calls `validate_candidate_tree`, which walks the whole tree with the
existing size limits; on a real worktree with build output this may be slow or exceed the
limits. S2 measures this and either walks only non-ignored files for attached workspaces
or raises the limits with evidence. Fable decides on the measurement.

### 6.6 Capabilities: observe, signal, control, integrate

Recorded on the attachment as four booleans and enforced by the owner loop:

| Capability | Wrapped attach | Foreign attach | Native run |
|---|---|---|---|
| OBSERVE (watch the world, evaluate, record verdicts) | yes | yes (by `serve`) | yes |
| SIGNAL (surface stale/invalid state) | yes: events, `status`, serve view | yes | yes |
| CONTROL (stop/cancel the process) | own child only, on the wrapper's own SIGTERM/SIGHUP; **no `mid_run: stop`** for external agents in this version | never (the PID is informational; `identity_state` is used for liveness, never for `kill`) | existing supervisor |
| INTEGRATE (apply automatically when eligible) | only with `--auto-apply` at attach | only with `--auto-apply` at attach, executed by `serve` | session or invocation policy |

The agent never needs to know Dispatch exists. Signaling is Dispatch-side only in this
version; writing marker files into an agent's workspace is deliberately not done.

### 6.7 Commands

```text
dispatch attach [--root <path>] [--workspace <path>] [--task "..."] [--agent claude|codex|cursor]
                [--allow-unsafe-local] [--auto-apply] -- <command...>
dispatch attach --workspace <path> [--root <path>] [--pid <n>] [--task "..."] [--agent ...]
                [--allow-unsafe-local] [--auto-apply]
dispatch finish <work-id>          # foreign work: freeze Δ, verify, become Ready
dispatch serve [--root <path>] [--json]
```

Wrapped attach runtime behavior:

- The child inherits stdio and stays in the wrapper's process group (no `process_group(0)`),
  so terminal job control and Ctrl+C behave exactly as if the shell had started the
  agent. The wrapper ignores SIGINT/SIGQUIT while the child runs (as `time` does),
  forwards SIGTERM/SIGHUP to the child, and always records the exit. It writes nothing to
  the terminal while the child is alive.
- It is **not** the `Executor` path (which pipes stdio and captures logs). No stdout/stderr
  capture, no token accounting, no timeout: Dispatch does not own this agent.
- The watcher is the existing `Watcher` with `WatchSpec { source: root, workspace, baseline, delta_patch: runs/<id>/delta.patch, poll }`.
  Verdicts are persisted through the shared persist step (factored out of
  `phase3::apply_watch`) as `coherence.invalidated | coherence.checked` on this Work.
- On exit: finish (6.5) → if `--auto-apply`: `auto_apply(run)`; print one line after the
  agent has released the terminal.

`serve` runtime behavior:

- Identity: `locks/serve-<sha256(root)>.lock` (`flock`); a second `serve` on the same root
  exits with "already serving". A dead `serve` releases the lock by itself (no stale
  socket, no PID file).
- Loop every `coherence.poll_secs`: `world::signal(root)`; when it moved, or when a new
  active Work appeared, reevaluate every active attached Work whose owner is not
  `ExactLive` (wrapped Work with a live wrapper is the wrapper's business); persist
  verdicts. Then, for every Ready attached Work with INTEGRATE, call `auto_apply`
  (serialized by the source lock like everyone else). After its own successful apply it
  reevaluates immediately (the apply is the doorbell) instead of waiting a tick.
- View: one line per Work in this root, attached and native, from the DB projections:
  `id · agent · CONTINUE/REFRESH/STOP · working/ready/applied/blocked · first reason`.
  Refreshed in place on a TTY, appended as lines otherwise; `--json` emits one object per
  change. This is the whole of the "project view" for this version.
- Exit on Ctrl+C/SIGTERM. Nothing is applied or launched on exit or start.

### 6.8 IPC recommendation

**No socket in this version.** The coordination substrate is what the codebase already
trusts: SQLite (WAL, revision-fenced projections, event journal polled at 25–50 ms
elsewhere) plus `flock` files. `attach`, `finish`, `serve`, the TUI and the CLI all read
and write the same durable state; `serve` discovers new Work on its next tick (≤
`poll_secs`, default 10 s), which is adequate for work that takes minutes to hours.

Why not a Unix socket now: it would add a listener lifecycle, stale-socket cleanup,
a second framing layer next to the control protocol, and a Windows story, for a latency
gain no scenario in this plan needs. The control protocol is not reused as the service
transport because it is per-grant, stdio-bound, foreground-owned and excludes review and
apply by design; bending it into a repo-scoped bus would be the "unnatural abstraction"
the assignment warns about.

When a socket becomes justified (a live TUI client wanting push updates, or sub-second
reevaluation after a landing), it should be a thin notify-only doorbell ("world moved",
"work changed") that clients still confirm against the DB, so nothing depends on it.

### 6.9 Lifecycle and recovery

| Question | Answer |
|---|---|
| Authoritative integration world | the integration root working tree minus its ignore rules, as `world::observe` computes it; never a stored snapshot |
| New world revision | a changed `world::signal`, confirmed by a changed `observe().digest`; digests name content, not time |
| Manual edits detected | by the same signal/observe path; unrelated Work stays CONTINUE via L1 |
| Commits / branch movement | `HEAD` is part of the signal; a checkout that changes the tree is a world change; a commit that does not is not |
| Interaction with mid-run polling | one watcher per owner loop; `serve` polls once for all foreign Work; no shared watcher process |
| Active Work after `serve` restart | read back from the DB (`mode = attached`, `lifecycle != Finished`); the wrapper's `ProcessIdentity` and the agent's, if known, are re-checked with `identity_state`: `ExactLive` → leave to the owner; `Gone`/`Reused`/`Unknown` → `serve` adopts observation and marks `attachment.owner_state` honestly; nothing is finished, applied or relaunched |
| Reattach a process | not a separate operation: `attach --workspace` on a workspace that already has active Work refuses with the existing id |
| Dead processes | `identity_state` (pid + start time + boot id); never `kill` for foreign work |
| "Finished" for a foreign workspace | explicit `dispatch finish`; for wrapped, child exit |
| Δ ready | when frozen by finish |
| External agent changes vs unrelated workspace changes | Δ is everything in the workspace that differs from S0, ignore rules applied; there is no per-process attribution inside one workspace, which is why one workspace holds one Work |
| Baseline provenance | `attachment.provenance` and `confidence` |
| File watchers | not used; polling `git status` is sufficient and portable |
| Simultaneous auto-apply attempts | the per-source lock (part 5.3); `serve` and wrappers and native sessions all go through `auto_apply` |
| Manual source changes during final integration | digest fence; one re-validation; then blocked for a human |
| Planned Dispatch runs | unchanged; visible in the serve view as native runs |
| Attach and admission/capacity | none; attached work consumes no Dispatch-managed pool and records `resource: None`; capacity is `unknown`, never estimated |
| Durable vs recomputed | durable: the Work record, S0, frozen Δ, events, attachment metadata; recomputed: world, facts, live verdict |
| SQLite unavailable or locked | the busy timeout (5 s) and error return already in `Database`; the wrapper keeps the agent running and retries persistence on the next tick, logging to stderr only after the child exits; `serve` reports and retries next tick |
| Crash consistency | same as native runs: WAL + FULL sync; apply is not crash-atomic (documented); a wrapper crash leaves the Work `Working` with `owner_state: gone` for `serve` or a human `finish` |

### 6.10 Application serialization

One function, `orchestrator::apply::auto_apply`, is the only automatic applier in the
system, called by the TUI, the CLI, the attach wrapper and `serve`. It always takes the
run lock and the source lock with bounded waits and always validates after acquiring them.
There is no queue, no scheduler, no priority: whoever holds the source lock validates
against the current world and the rest wait or fail closed.

### 6.11 Standalone and service coexistence

- `dispatch run`, `dispatch`, `dispatch accept` are unchanged when no attachment exists.
- With attachments present, native runs and attached Work see the same world, take the
  same lock, and produce the same events. A native session's watcher notices a landing by
  `serve` on the next poll, and vice versa.
- Nothing in the standalone path reads the `serve` lock or requires `serve` to run.

### 6.12 Relationship to the existing control protocol

KEEP, unchanged. It remains the way a machine client submits and observes Dispatch-launched
work. Attached Work is visible through `status --json` and `history` like any run;
exposing it through the control protocol is not needed for the scenarios and is not
planned.

---

## 7. Metrics and learning (what becomes derivable)

All from the `events` table and run projections; no new metrics system.

| Question | Derivation |
|---|---|
| Concurrent Work | count of runs with `lifecycle != Finished` per `source_path` over time (event timestamps) |
| CONTINUE/REFRESH/STOP counts | `coherence.*` payloads, `auto_apply.blocked.coherence`, `result.applied.coherence` |
| False REFRESH identified by a human | `review.rejected` with the structured reason `verdict-disagreed` (new allowed reason label), or a `refresh` whose new run's verdict was CONTINUE on an unmoved world |
| Missed invalidation / false CONTINUE | post-hoc `review.rejected` on an `applied_by: auto_apply` run, with reason `broke-after-apply` (new label) |
| Time from invalidation to detection | `coherence.invalidated.timestamp` minus the world change time is not observable; report detection latency as bounded by `poll_secs`, and `first_invalid_at` versus attempt timestamps (existing) |
| Work stopped before more execution | `coherence.stopped` (existing) |
| Automatic applications, blocked ones and reasons | `result.applied` with `applied_by`, `auto_apply.blocked.reason`, `auto_apply.skipped.reason` |
| Application retries / failures | `application.failed` with `applied_by`; `world_moved_during_validation` reason |
| Refresh success | `refreshed_from` lineage and the new run's `result.applied` |
| Manual intervention | human `review.*` after `auto_apply.blocked`, or `accept` on a run whose owner had auto-apply skipped |
| Attached vs native | `run_mode` |
| Reasons people bypass Dispatch | `auto_apply.skipped.verification_not_configured` frequency; `strict` usage (existing) |

Two new structured reject reasons (`verdict-disagreed`, `broke-after-apply`) are the only
data-model addition for learning.

---

## 8. Effort and risk

Units: focused implementation days for a capable coding model with Fable specifying,
reviewing and integrating. Ranges reflect uncertainty; confidence is stated.

| Area | Estimate | Basis | Confidence |
|---|---|---|---|
| Auto-apply core (`apply.rs` extraction, authority, outcome, blocking lock, authorization predicate, retry-once, events, `applied_by`, post-hoc review guard) | 2–3 d | ~150 lines moved, ~300 new in one module; touches `apply_locked`, `accept_or_reject_latest`, `review_delivery`, `is_ready_unapplied` callers | high |
| CLI flags, exit code 6, `RunResult.auto_apply` | 0.5 d | `main.rs` arg structs and two call sites | high |
| TUI mode, indicator, `/auto-apply`, `aa`, post-finish flow, plain mode | 2–3 d | `presenter.rs` (2,122 lines) and `inspection.rs`; three key loops; PTY fixture tests exist (`tests/fixtures/phase4_session.py`) | medium (terminal input) |
| Concurrency and dogfood scenario tests (two sessions, cases 1–3, simultaneous, manual edit) | 2 d | fixture patterns in `coherence_recovery.rs` (gated fake agent with FIFO), `coherence_accept.rs` | medium |
| Auto-apply docs and release notes | 0.5–1 d | `coherence.md`, `product-guide.md`, README, `control-protocol.md` note, `release-install.md` | high |
| **Auto-apply total** | **7–10 d** | | |
| Migration 21 + `RunMode::Attached` + `AttachmentRecord` + presenter labels | 1–1.5 d | migration 13 pattern; `coherence_upgrade.rs` pattern | high |
| Source layer: `materialize_baseline_from_commit`, repo identity, merge base, `snapshot_delta`/`collect_diff` on an external worktree, tree-limit measurement | 2–3 d | `source.rs` internals reused; the unknown is `validate_candidate_tree` cost on real worktrees | medium |
| `attach` creation (both forms), `finish` | 2 d | reuses `run_dispatch`'s record construction; new validation rules | medium |
| Wrapped owner loop (inherited-stdio child, signal handling, watcher reuse, finish, auto-apply) | 2–3 d | `Watcher` reused; the new part is the child/signal handling outside `Executor` | medium-low (process/terminal) |
| `serve` (lock identity, poll loop, adoption, apply, view, restart) | 2–3 d | small loop over DB projections; view is text lines | medium |
| Scenario tests (attached Claude, attached + native, restart, simultaneous) | 2–3 d | fixture agent scripts; need `git worktree` fixtures | medium |
| Attach/serve docs, release | 1 d | | high |
| **Service/attach total** | **12–17 d** | | |
| **Overall** | **19–27 d** | | medium |

Critical path: A0 → A2 → A4 → A5 → dogfood (auto-apply releasable on its own) → S0 → S1‖S2 → S3 → S4‖S5 → S6 → release.

Highest-risk unknowns, in order:

1. **Process and terminal handling in the wrapper** (S4): keeping Claude/Codex fully
   interactive while a watcher runs in the same process; signal forwarding; exit
   detection. Mitigation: same process group, ignore SIGINT while the child runs, no
   terminal writes; a PTY test with a fixture "agent" that reads stdin.
2. **Workspace attribution and size** (S2): `collect_diff` limits on real worktrees;
   ignore handling with a foreign `.git` file. Mitigation: measure on this repository's
   own worktree before choosing the fix.
3. **Terminal input** (A4): Shift+Tab reliability across terminals and multiplexers
   (crossterm parses CSI Z; kitty protocol also maps it). Mitigation: `/auto-apply`
   command as the fallback in every mode; PTY test for both.
4. **Application race** (A2/A5): the retry-once rule and the blocking lock must not
   deadlock with a human review holding the run lock. Mitigation: 5 s run-lock wait and a
   clear "foreground owner" message; test with a held lock.
5. **Persistence/restart** (S5): adoption rules for orphaned wrapped Work. Mitigation:
   never finish or apply on adoption; label and wait for a human.
6. **Backward compatibility**: migration 21 rebuilds `runs`; `applied_by` and
   `attachment` are additive. Mitigation: extend `coherence_upgrade.rs` with a schema-20
   fixture.
7. **Cross-platform**: none of this adds a socket; `flock` and `identity_state` already
   have non-Unix branches of weaker quality; Windows stays uncertified as today.

Complexity multipliers to watch: `orchestrator.rs` size (hence the `apply.rs` split first),
and any temptation to let `serve` become a scheduler.

---

## 9. Validation matrix

| Scenario | Where | Assertion |
|---|---|---|
| Unmoved world, verification passed, auto-apply on | CLI test | applied; `applied_by: auto_apply`; `review: pending`; no `review.*`, no `goal_feedback_revisions` row |
| Moved world, unrelated file, checks pass on merged tree | CLI test | applied with `coherence.analysis: integration` |
| Moved world, L2 disabled or unauthorized | CLI test | `auto_apply.blocked { integration_evidence_missing }`; source unchanged; run reviewable |
| Verification not configured / failed / inconclusive | CLI test | `auto_apply.skipped {reason}`; review menu shown |
| REFRESH (`fact_broken`) / STOP (`already_applied`) | CLI test | blocked; verdict stored; nothing applied; nothing rejected |
| Two sessions, A lands, B re-evaluated: cases 1, 2, 3 | integration test with gated fake agents (FIFO pattern) | B applied / blocked REFRESH / blocked STOP respectively; exactly one `result.applied` per run |
| Simultaneous finish | same, both released together | second waits on source lock; its stored `world_digest` equals the world after the first apply |
| Manual edit during L2 | test writes a file while checks run | fence fails, one re-validation, then applied or `world_moved_during_validation` |
| Human review holds run lock | test | auto-apply reports "foreground owner", source unchanged |
| Owner dies after Ready before apply | test kills process | run pending, no apply on restart or `load_run` |
| Post-hoc accept/reject of an auto-applied run | CLI test | review recorded; no second apply; rejection does not touch the source |
| TUI: Shift+Tab toggles on prompt, during work, in review; indicator text; `/auto-apply` in `--plain`; `aa` action | PTY fixture | rendered strings and resulting DB state |
| TUI: session start defaults to off | PTY fixture | |
| Legacy run.json / schema 20 DB opened by new binary | upgrade test | `applied_by: None`, `attachment: None`, runs load and apply |
| Attach: same repo required; same checkout refused; unrelated repo refused; plain dir foreign refused | CLI tests | |
| Attach: S0 equals merge-base tree; pre-attach edits appear in Δ | integration test with `git worktree` | |
| Wrapped attach with fixture agent editing the worktree; another Work lands; verdict recorded for the attached Work | integration test | `coherence.invalidated` on the attached run when a fact breaks; CONTINUE otherwise |
| Wrapped attach exit → finish → verification → auto-apply (granted) | integration test | applied through the same `auto_apply` with `applied_by: auto_apply` |
| Foreign attach + `serve` + `finish` + apply | integration test | `serve` applies only Ready Work with INTEGRATE |
| `serve` restart with active Work; wrapper alive vs gone | integration test | adoption labels; nothing applied or launched |
| Second `serve` on the same root | test | refused; lock released when the first exits |
| Native run and attached Work on one root, both auto-apply | integration test | one applies, the other is revalidated against the new world |
| Standalone behavior unchanged | existing suites | all 647 tests still pass |
| Manual: TUI two sessions on a real repo; `attach -- claude` for one goal; `serve` view | dogfood log in `docs/` | |

---

## 10. Execution plan: work packages

Conventions for every packet: implement in a branch off `main`; run `cargo fmt --check`,
`cargo test`, `cargo clippy --all-targets -- -D warnings`; return the diff, the test
names added, the LOC delta of `src/`, and a short note of anything not done. Do not touch
files outside the listed ones without saying so. Escalate (do not decide) anything under
"escalate".

### Phase A: auto-apply (releasable alone as 0.3.0)

#### A0 — Apply module and authority interface (Fable, not delegated)

- Objective: move `apply`, `apply_locked`, `CoherenceBlocked` handling and `remember_validity`
  into `src/orchestrator/apply.rs`; add `ApplyAuthority { Human, AutoApply }`, the
  `ApplyOutcome` type, `AppliedBy` on `RunOutcome`, and the `auto_apply` function
  signature with a `todo!()`-free skeleton that returns `Skipped(not_ready)` for
  everything. Behavior of human accept must be byte-for-byte unchanged (existing tests
  are the proof).
- Why Fable: it fixes the interfaces every other packet depends on.
- Files: `src/orchestrator.rs`, new `src/orchestrator/apply.rs`, `src/models.rs`.
- Acceptance: full suite green; `git diff --stat` shows a move, not a rewrite.

#### A1 — Bounded blocking operation lock

- Objective: `OperationLock::acquire_wait(path, busy_message, timeout) -> Result<Self>`.
- Subsystem: `src/orchestrator.rs` (`OperationLock`), possibly moved next to `apply.rs`.
- Current interface: `acquire(path, busy_message)` with `flock(LOCK_EX|LOCK_NB)`.
- Change: poll `acquire` every 100 ms until `timeout`; same error text on expiry plus
  "after waiting Ns". Non-Unix branch: same loop over `create_new`.
- Invariants: no sleeping while holding another lock the caller did not already hold;
  no SQLite transaction involved.
- Tests: unit test with a lock held on a thread and released after 300 ms; timeout test.
- Escalate: any desire to change lock file locations or use `fcntl` locks.
- Can run concurrently with A0 (interface is fixed above).

#### A2 — `auto_apply` core

- Objective: implement stages 1–3 of part 5.2, the events of part 5.4, and the post-hoc
  review guard (accept/reject on an applied run records review only).
- Files: `src/orchestrator/apply.rs`, `src/orchestrator.rs` (`accept_or_reject_latest`,
  `review_delivery`), `src/coherence.rs` (if a helper is needed to expose gate evidence
  level), `src/presenter.rs` `label()` for the new states (text only).
- Relevant types: `AcceptGate`, `Validity.analysis`, `Validity.world_changed`,
  `RunOutcome`, `RunStatus`, `OperationLock::acquire_wait`.
- Intended change: `pub fn auto_apply(state: &State, run_id: &str) -> Result<ApplyOutcome>`
  returning `Applied { report, validity }`, `Blocked { reason, validity }`,
  `Skipped { reason }`, `Failed { error }`; every branch commits its event; `Applied`
  sets `applied_by: AutoApply` and leaves `review: Pending`.
- Invariants: never call `record_allocation_feedback*`/`record_routing_evaluation*`;
  never hold a SQLite transaction across `gate`; wait for locks, do not spin; retry the
  fence at most once; `review` unchanged.
- Non-goals: refresh, rejection, control protocol, TUI.
- Tests: `tests/auto_apply.rs` covering every row of the stage-2 table using the
  `fake-good` harness (unmoved world) and the gated `codex` fixture from
  `coherence_recovery.rs` (moved world with checks); post-hoc review test.
- Escalate: any case where the stage-2 table seems to require a different answer.

#### A3 — CLI surface

- Objective: `run --auto-apply`, `refresh --auto-apply`, exit code 6, `RunResult.auto_apply`.
- Files: `src/main.rs`, `src/orchestrator.rs` (`run_result`), `src/models.rs` (`AutoApplySummary`).
- Tests: `tests/coherence_cli.rs` additions: applied, blocked, skipped; JSON shape; exit codes.
- Escalate: any exit-code collision.
- Depends on A2.

#### A4 — TUI mode

- Objective: part 5.5 in full.
- Files: `src/presenter.rs` (`Ui`, `Editor::event`, `input_prompt`, `work`, `session`,
  `run_goal`, `review_goal`), `src/presenter/inspection.rs` (`review_action`),
  `src/presenter/theme.rs` (one style if needed).
- Relevant interfaces: `draw(body, editor, hint)`, `Input`, `KeyCode::BackTab`,
  `orchestrator::present`, `tokio::task::block_in_place`.
- Invariants: default off; toggle never submits or edits text; the indicator is in every
  hint; auto-apply runs inside the `present` scope; on any non-applied outcome the
  review menu appears with the notice; `aa` records a human accept exactly like `a` then
  flips the mode.
- Non-goals: persisted preference, settings screen, portfolio view, refresh action.
- Tests: extend `tests/fixtures/phase4_session.py` scenarios and `tests/product_ux.rs`
  snapshot tests: indicator strings both modes, toggle on each screen, `aa`, plain-mode
  `/auto-apply`.
- Escalate: any need to change the `Input` enum's semantics or the input-drain rules.
- Depends on A0 for types; can be developed against A0's skeleton before A2 lands.

#### A5 — Concurrency and dogfood scenario tests

- Objective: the two-session matrix and race rows of part 9.
- Files: new `tests/auto_apply_concurrency.rs`; fixtures under `tests/fixtures/`.
- Guidance: reuse the FIFO-gated agent from `coherence_recovery.rs`; start two `dispatch run --auto-apply --json` processes on one source; release in controlled order; assert events and source bytes.
- Escalate: any flaky timing that cannot be made deterministic with the gate.
- Depends on A2, A3.

#### A6 — Docs and release notes for 0.3.0

- Files: `docs/coherence.md` (new "Automatic application" section, event table rows,
  policy table), `docs/product-guide.md` (mode, keys, `/auto-apply`, post-hoc review),
  `README.md` (short version and the claims table in `docs/coherence-validation.md`),
  `docs/control-protocol.md` (one paragraph: unchanged; `applied_by` visible in results),
  `docs/release-install.md` (0.3.0 section: no migration), `.github/release-notes.md`.
- Invariant: no claim of tokens or money saved.
- Depends on A2–A5.

#### A7 (optional, later) — Control protocol `auto_apply` and TUI refresh action

- Only after dogfood asks for it. Grant capability `--allow-auto-apply`; submit field;
  actor `machine:<principal>` on the events; review still denied. TUI `refresh` typed
  action that calls `refresh_request` with the same local authorization prompt.

Parallelism: A1 ‖ A0; A3 ‖ A4 after A2; A5 after A3. A4's UI can start on A0's skeleton.

### Phase S: attached work and `serve`

#### S0 — Design freeze (Fable, not delegated)

- Fix `AttachmentRecord`, `RunMode::Attached`, the CLI names (`attach`, `finish`, `serve`),
  the owner-state vocabulary, and the shared watcher persist helper's signature. Decide
  the review-recording rule for attached work (default: review events only). Write the
  migration 21 SQL.

#### S1 — Schema and model

- Objective: migration 21 (rebuild `runs` with `'attached'` in the `CHECK`), `RunMode::Attached`, `RunRecord.attachment`, presenter labels for attached states.
- Files: `src/db.rs` (`MIGRATIONS`), `src/models.rs`, `src/presenter.rs` (`label`, `projection` text only), `src/sync.rs` (assert attached runs are never enqueued).
- Tests: `tests/coherence_upgrade.rs` schema-20 fixture opens, backup file created, old runs unaffected; round-trip of `AttachmentRecord`.
- Escalate: any other table needing change.

#### S2 — Source layer for external worktrees

- Objective: `source::materialize_baseline_from_commit(repo, commit, run_dir) -> SourceSnapshot`
  (via `git archive | tar -x` then `initialize_internal_repository`), `source::repo_identity(path) -> RepoIdentity { common_dir, main_worktree, key }`, `source::merge_base(root, workspace) -> Option<String>`; verify `snapshot_delta` and `collect_diff` against a real linked worktree (`.git` file, ignore rules, build output); measure `validate_candidate_tree` on this repository's own worktree with a `target/` present and report numbers.
- Files: `src/source.rs` only.
- Tests: unit tests with `git worktree add` fixtures; a test that pre-attach edits appear in Δ from the merge-base baseline.
- Escalate: the tree-limit decision (report the measurement; do not change limits).
- Runs concurrently with S1.

#### S3 — `attach` and `finish`

- Objective: both attach forms' validation and record creation; `finish` for foreign Work.
- Files: `src/main.rs`, new `src/orchestrator/attach.rs`, `src/orchestrator.rs` (exports).
- Guidance: build the `RunRecord` per the mapping table in 6.3; copy the root's `dispatch.yml` to `config.snapshot.yml`; commit `run.created` with `attachment` in the payload; `finish` = `collect_diff` + `run_checks_with_config(workspace, …)` + `refresh_outcome` + `run.finished`.
- Invariants: refuse same-checkout, different repository, no merge base, plain-dir foreign; require `--allow-unsafe-local` when checks are configured; never guess the model.
- Tests: CLI tests for every refusal; a foreign attach → `finish` → `check` → `accept` path; `explain` shows provenance.
- Depends on S1, S2.

#### S4 — Wrapped owner loop

- Objective: `dispatch attach -- <cmd>` runtime per part 6.7.
- Files: `src/orchestrator/attach.rs`, `src/orchestrator/phase3.rs` (factor `apply_watch` persist into a shared helper, no behavior change), `src/coherence/watch.rs` (no change expected).
- Invariants: child inherits stdio and process group; wrapper is silent while the child lives; SIGINT/SIGQUIT ignored in the wrapper while the child runs; exit always recorded; watcher verdicts persisted through the shared helper; finish then `auto_apply` only with INTEGRATE.
- Tests: fixture agent script that reads stdin and edits the worktree, run under a PTY (`tests/fixtures/` Python pattern); a landing by a native run during the wrapped session produces `coherence.invalidated` on the attached Work.
- Escalate: any need for `tcsetpgrp` or a pseudo-terminal in the wrapper.
- Depends on S3.

#### S5 — `serve`

- Objective: part 6.7 `serve` behavior and part 6.9 adoption rules.
- Files: `src/main.rs`, new `src/orchestrator/serve.rs`.
- Invariants: one per root (`flock`); poll only; never finish, apply without INTEGRATE, or launch; adoption labels only; view is plain lines.
- Tests: two `serve` on one root; restart with live vs gone wrapper; foreign Work with `--auto-apply` applied after `finish`; `--json` stream shape.
- Depends on S3; concurrent with S4.

#### S6 — Scenario tests

- ATTACHED CLAUDE, ATTACHED CODEX + NATIVE, SERVICE RESTART, SIMULTANEOUS INTEGRATION (serve + native), MANUAL SOURCE CHANGE with attached Work.
- Files: `tests/attach_scenarios.rs`, fixtures.
- Depends on S4, S5.

#### S7 — Docs and release 0.4.0

- `docs/attach.md` (new: model, provenance, capabilities, commands, limits), README
  section, `product-guide.md`, `coherence.md` cross-references, `release-install.md`
  (migration 21, backup), release notes, `coherence-validation.md` claims table.

Parallelism: S1 ‖ S2; S4 ‖ S5. Fable reviews after S2, S3, and before S6.

---

## 11. Dogfood plan

**Begin dogfooding auto-apply as soon as A2 + A3 land** (before the TUI packet):
`dispatch run --auto-apply` in two terminals on this repository with `checks.verify:
[cargo test]`. That already exercises the lock, the gate, L2 and the post-hoc review.
Add the TUI when A4 lands. Keep a log in `docs/dogfood-0.3.md`: for every auto-apply
attempt, the outcome, reason, `analysis`, wall time of L2, and whether the human later
disagreed. After thirty auto-apply attempts with a moved world, review against the
falsifiers in `coherence-validation.md` before starting phase S.

Phase S dogfood: one `dispatch attach -- claude` session in a worktree while a native
`dispatch` session works in the main checkout; then `serve` with a foreign-attached
Codex worktree. Log the same fields plus provenance confidence and adoption events.

---

## 12. Release criteria

Release scope, as decided:

- **v0.3.0, auto-apply and coherence-driven integration**: visible TUI auto-apply
  mode; safe automatic application; final-boundary coherence revalidation;
  concurrent-run behavior; preserved distinction between human review and policy
  application; enough dogfooding to prove it works. Packets A0–A6.
- **v0.4.0, Dispatch service and attach**: repo-scoped local service; shared view of
  the integration world; attached Claude/Codex/other-agent work; Dispatch-native and
  foreign Work participating in the same coherence model; serialized coherent
  integration; restart/recovery semantics. Packets S0–S7.

Progress log:

- 2026-09-22: A0 (`2dc69b4`), A1 (`1d00673`), A2 (`0a1bc6e`, `ae9e984`) merged on
  `worktree-plan-auto-apply-attach`; suite 665 passed. First dogfood of the core
  (three concurrent runs on a Python project, scripted agent behind the Codex
  adapter because the funding guard refused the real one): A applied on an unmoved
  world in 121 ms, B applied after integration checks on the moved world in 753 ms,
  C blocked as REFRESH (`patch_conflict`) with the source untouched; no review
  evidence written.
- 2026-09-22: A3 (`415a81f`), A4 (`1d56631` plus `18f473c`, which moved the TUI
  hook to the single point every Ready result reaches so a goal that answered a
  clarification is also auto-applied), A5 (`ebfcdbe`, six process-level scenarios,
  fence retry observed through the second `coherence-checks` directory) and A6
  (`bc1e481`, docs and version 0.3.0) merged. Release gate on the merged tree: 685
  passed, 0 failed, 2 ignored. Two pre-existing tests are load-sensitive and fail
  only when another suite competes for the machine: the planned-goal deadline
  scenarios in `tests/phase8_planning.rs` (60 s goal deadlines) and
  `claude_control_slow_output` in `tests/phase6_portfolio.rs` (a 10 s pipe-drain
  window); both pass in isolation on every tree. TUI dogfood in the real binary
  through a pty: the hint row shows `[review before apply] Shift+Tab`, Shift+Tab
  flips it to `>> AUTO-APPLY ON - Shift+Tab to pause`, the goal heading carries the
  `>>` marker and the working hint keeps the mode; the goal itself stopped at the
  funding guard ("paid credits are now available"), which the session's allocation
  path revalidates independently of the account probe. The post-finish apply flow
  in the session is covered by the four PTY scenarios in `tests/phase4_review.rs`.
  Real-agent dogfood is blocked until the Codex profiles are revalidated with
  `dispatch setup codex`.

0.3.0 (auto-apply):

- Existing suite green; A2/A3/A4/A5 tests green; PTY tests pass on macOS arm64; Linux CI green.
- Safe defaults verified: session starts with auto-apply off; no config key; `verification_not_configured` never applies.
- Review/evidence semantics: an auto-applied run has `review: pending`, no feedback row, `applied_by: auto_apply`; human accept unchanged (existing tests).
- Final-boundary tests: simultaneous finish, edit during L2, held run lock.
- Docs match: `coherence.md`, `product-guide.md`, README, claims table.
- Install/upgrade: no migration; `release-install.md` updated.

0.4.0 (attach and serve):

- Attach scenarios and restart tests green; two-`serve` refusal; standalone suites unchanged.
- Migration 21 upgrade test with a schema-20 fixture and backup file.
- Multiple-worktree behavior tested (same repo accepted, different repo refused).
- Manual dogfood log exists for both attach forms.
- Docs: `attach.md`, README, product guide, release notes; claims table updated to say
  what attached coherence covers (full confidence for Git merge-base S0, partial for
  snapshot-at-attach).

Not in scope of either release: automatic refresh, pessimistic locking, symbol graph, new
languages, sockets, GitHub/GitLab integration, a portfolio dashboard, killing foreign
processes, cloud anything.

---

## 13. Executive summary

1. **Auto-apply architecture.** One new function, `orchestrator::apply::auto_apply`,
   called by whichever process owns the run right after it finishes Ready, under the
   owner's live policy. It reuses `gate`, `apply_validated` and the existing events, adds
   a bounded-wait lock, a stricter authorization predicate, and one fence retry.
   Nothing about the policy is persisted; only the evidence is.
2. **TUI interaction.** A session-scoped mode, off at every start, toggled by Shift+Tab
   (and `/auto-apply` for plain mode), shown as the first segment of the hint row on every
   screen (`⏸ review before apply` / `⏵⏵ auto-apply on`), plus one review action `aa`
   that accepts this result as a human and turns the mode on for the next ones.
3. **The safety invariant.** A policy applies only a verdict whose evidence is complete for
   the exact world it names, judged while holding the source lock: an unmoved world with
   passed candidate verification, or a moved world whose merged tree passed the project's
   own checks (`analysis: integration`), applied through the digest fence to that same
   world, at most one re-validation if it moves meanwhile. Review is never touched.
4. **Service architecture.** Owner loops, not a daemon: `dispatch attach -- <agent>` owns
   its own Work; `dispatch serve` (one foreground process per integration root, `flock`
   identity) owns foreign attachments and orphaned Work and shows the project view.
   Coordination is SQLite plus `flock`; no socket.
5. **Attachment model.** Attached Work is a `RunRecord` with `mode: attached`, one candidate
   whose workspace is the external worktree, S0 materialized from the Git merge-base into a
   private baseline (full confidence) or a snapshot at attach for plain directories
   (partial, labeled), Δ frozen at agent exit or explicit `finish`, capabilities
   `observe/signal/control/integrate` recorded and enforced; same-checkout attach refused.
6. **Reused code.** `coherence::{evaluate, gate, live_validity, facts, symbols, world,
   integration}`, `source::{apply_validated, safe_apply, snapshot_delta, collect_diff,
   create_snapshot internals, fingerprint_tree}`, `Watcher`/`WatchSpec`,
   `commit_transition` and the event journal, `OperationLock`, `ProcessIdentity` and
   `identity_state`, `refresh_outcome`, `run_result`, the TUI `draw`/`Input` model, the
   PTY test fixtures.
7. **Must not be redesigned.** The review-recording functions and their evidence tables;
   the digest/fingerprint fences; the watcher's single-writer rule; the control protocol's
   human-review exclusion; admission and capacity; the planned driver; the run
   directory and state layout; the rule that facts are recomputed, never stored.
8. **Critical path.** A0 → A2 → A4 → A5 → dogfood 0.3.0; then S0 → S1‖S2 → S3 → S4‖S5 → S6 → 0.4.0.
9. **Effort.** Auto-apply 7–10 focused days; attach and serve 12–17; overall 19–27 with
   Fable specifying and integrating. Medium confidence overall; high on the auto-apply
   core, lower on wrapper process handling and worktree attribution.
10. **Highest-risk unknowns.** Wrapper process and terminal handling; `collect_diff` cost
    and ignore semantics on real worktrees; Shift+Tab across terminals (mitigated by
    `/auto-apply`); lock-wait interaction with a human review; adoption rules after
    `serve` restart.
11. **First packet to delegate.** A1 (bounded blocking lock) immediately, in parallel with
    Fable's A0; then A2 as the first substantive packet once A0 is merged.
12. **Fable keeps vs assigns.** Fable keeps A0 and S0 (interfaces, migration SQL, the
    authorization predicate wording), the review of every packet against parts 5 and 6,
    the tree-limit decision in S2, and the release. Lower-usage models take A1, A2 (with
    the stage tables as the spec), A3, A4, A5, A6, S1, S2, S3, S4, S5, S6, S7.
