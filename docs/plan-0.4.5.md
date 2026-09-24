# Plan: Dispatch 0.4.5, Dispatch in the background


## Context

v0.4.4 made verdicts trustworthy. The only thing keeping them current is
`dispatch serve`, which occupies a terminal. v0.4.5 turns project watching into
something you switch on and forget:

- `dispatch start` watches the project in the background and returns the shell at once.
- `dispatch watch` is a live view that owns nothing.
- `dispatch stop` ends watching, for exactly this project.

No agent profile is needed, and there is no process discovery, RPC or socket.

The code argues for a small design:
- The flock the serve lock already holds is a correct liveness signal. The kernel
  releases it on crash, kill and reboot.
- Watching is therefore "the serve lock for this root is held". A small record
  exists only so `stop` can find the process to signal, and is always checked
  against `ProcessIdentity`.
- `watch` needs no IPC: the event journal in SQLite is the doorbell.

Inspection also found four defects in `serve`'s owner loop. Each would make a
background watcher quietly wrong, so they are fixed here (see §1).

## 1. Verified current `serve` architecture (`src/orchestrator/serve.rs`, 495 lines)

**Startup.** `serve(state, root, json)`:
1. `resolve_source`;
2. `OperationLock::acquire(locks/serve-<sha256(root)>.lock)`, which gives one
   owner per root and refuses a second;
3. `Config::discover(root)`, where only `coherence.poll_secs` is used (default
   10 s). No resources or profiles are read.

**Each tick** (`tokio::time::interval`, ended by `lock::shutdown_signal`, which
covers Ctrl+C, SIGTERM and SIGHUP):
1. `world::signal`: git HEAD and `git status`, plus stat metadata; for
   directories, a stat walk.
2. `adopt_orphans`: attached runs whose owner was `Live` and whose process is now
   `Gone` become `Adopted`. This is done under the run lock and records
   `attach.adopted`.
3. `reevaluate` runs when the world moved or something was adopted. It covers
   **active** attached runs without a live owner:
   - `snapshot_delta`, `world::observe`, `coherence::evaluate` and
     `watch::settle`;
   - then the in-memory `Policy::step` decides what is worth sending;
   - `apply::persist_verdict` stores it, under the run lock.
4. `ready_for_auto_apply` then `auto_apply`: attached, finished, Ready, pending,
   unapplied, with `capabilities.integrate`. A successful apply triggers an
   immediate `reevaluate`.
5. `render_view(load_source_runs)`: prints `work_line` rows, redrawn in place on
   a TTY, or JSON objects when a line changed. `--json` also emits world lines.

**Defects found:**
- **D1. A held verdict is lost.** `Policy` rate-limits to one message per 60 s and
  holds the newer verdict. The native watcher flushes it on the next tick
  (`step(now, None)`). `serve` calls `step` only inside `reevaluate`, so a held
  verdict, for example REFRESH going back to CONTINUE, is never stored unless the
  world moves again.
- **D2. A restart forgets the stored verdict.** `policies` starts empty, and
  `Policy` assumes the last verdict sent was CONTINUE. After a restart a stored
  REFRESH that is now CONTINUE is "not worth sending", so the stale REFRESH stays
  forever. This matters once watchers restart routinely.
- **D3. The work is not followed.** Active foreign attached Work is re-evaluated
  only when the world moves, not when the attached work itself changes. This is
  the same gap 0.4.4 closed (F1) for the native watcher.
- **D4. Ready results are never re-checked.** `reevaluate` skips finished runs.
  A Ready result awaiting review, attached or native, keeps the verdict from when
  it finished. `watch` would show CONTINUE for work that `check` calls REFRESH.
  Only `check`, `status` and the accept gate compute a live verdict.

**Cost.** `load_source_runs` reads every run in the state directory, including
other projects, through `state.load_run`. Each read:
- opens the database and checks migrations;
- reads the projection;
- runs `repair_abandoned`;
- reads the events and compares the event file.

It does this up to four times per tick, before filtering by source.

## 2. Lifecycle: `start / watch / stop`

```
$ dispatch start            # resolve root → already watching? → spawn owner → wait ready → return
✓ Watching ~/src/project
  Dispatch is running in the background. See it: dispatch watch · Stop: dispatch stop
$ dispatch watch            # live view; Ctrl+C leaves the view, not the watcher
$ dispatch stop
✓ Dispatch stopped watching ~/src/project.
```

**`start [--root <path>]`:**
1. Resolve the root and initialize the state.
2. If the serve lock is busy, print "Already watching" and exit 0.
3. Otherwise spawn `current_exe --state-dir <state> serve --root <root>
   --background` (hidden flag):
   - `setsid` via `pre_exec`, so it has no controlling terminal and gets no
     SIGHUP or Ctrl+C;
   - stdin from `/dev/null`;
   - stdout and stderr to `watchers/<key>.log`, truncated each start;
   - working directory: the root.
   - `-v` is passed through to the child.
4. Wait up to 10 s (it took 0.1 s in the trial; the margin is for a loaded machine):
   - **Ready:** the lock is busy and the record names the spawned pid with an
     `ExactLive` identity.
   - **Child exited:** print the last log lines (for example an invalid
     `dispatch.yml`) and exit 1.
   - **Another start won the race:** the lock is busy but the record names another
     live pid. Print "Already watching" and exit 0; our child exits after being
     refused the lock.
5. Unix only; elsewhere, a clear refusal. No profile or `resources.yml` is read.

**`serve --background`:** the same owner loop without the view.
- Errors go to the log, and each distinct error text is logged only once (log
  growth stays bounded).
- `serve` in both forms writes the record after taking the lock, and removes it
  on graceful exit while still holding the lock. `stop` can therefore also stop
  a foreground `serve`.

**`stop [--root <path>]`:**
- If the lock is free, print "Not watching"; delete any stale record; exit 0.
- If the lock is busy, read the record. Proceed only if all three hold:
  - `record.root == root`;
  - `identity_state(record.identity) == ExactLive`;
  - the identity has a start time and boot ID.
- Then send SIGTERM to that pid. It is never a process group, and never a pid
  alone.
- Wait up to 10 s for the lock to become free.
- If the identity is `Reused`, `Gone` or `Unknown` while the lock is still busy,
  refuse and name the lock path. Never signal a guess.

**`watch [--root <path>] [--json]`:** a read-only client (§5). It takes no lock,
writes no verdicts and never starts or stops the owner.

## 3. Ownership model

- There is **one owner per integration root**: the holder of
  `locks/serve-<sha256(root)>.lock`, as today. Background and foreground
  `serve` share that lock, so they exclude each other.
- The owner is the only process that runs, under the run lock:
  - adoption;
  - verdict persistence for unowned Work;
  - policy auto-apply.

  Everything it does still goes through `persist_event`/`transition`
  (`state_revision`), the per-run `.operation.lock` and the source lock. None of
  these change.
- Native runs keep their own watcher and their run lock while supervised.
  Wrapped attach keeps its live owner. The project owner touches a run only when
  it can take the run lock, exactly as `adopt_orphans`/`reevaluate` do today.
- **New for D4.** The owner also re-evaluates **Ready, unapplied, review-pending
  results** of both modes when the world moves. It uses
  `coherence::shown_validity`, the function `check` uses, so a stored integration
  refusal for the same world digest is kept, never overwritten.
  - It persists through `apply::persist_verdict` under the run lock.
  - The verdict's meaning is unchanged: it is exactly what `check` would show,
    now stored so that `watch` and `serve` agree with `check`.
  - During implementation, verify that `transition` accepts `coherence.*` events
    on finished runs (0.4.4 already stores integration refusals on them).
- `watch`, `status`, `check` and `history` are readers. `load_run`'s existing
  crash repair stays where it is; moving it out of reads is still deferred.
- Standalone commands never consult the watcher. With no owner, everything works
  as in 0.4.4; views just say "not watched".

## 4. Process identity and crash recovery

**Record.** `watchers/<key>.json`:
```
{version:1, root, identity: ProcessIdentity, started_at, dispatch_version, log}
```
It is written atomically by the owner after it takes the lock. The **lock**
decides whether the project is watched; the record only says **whom to signal**.

| Situation | What happens |
|---|---|
| Owner crashes, or is SIGKILLed | The kernel frees the flock. `watch`/`status` read "not watched" regardless of the leftover record. `start` spawns a new owner, which overwrites the record. `stop` says "not watching" and deletes the record. |
| Reboot | The flock is gone and the record is stale; handled as above. The new boot ID makes the identity `Reused`/`Gone`, so it is never signalled. Nothing starts at login; that is deferred. |
| `start` after an unclean exit | Lock free → normal start. The new owner seeds `Policy` from the stored verdicts (D2), so nothing is lost. |
| `stop` racing a crash | The lock is re-checked after the identity check. A pid that vanished is `Gone` and is not signalled. The window between identity check and kill matches the existing attach/launch checks. |
| Two `start`s at once | Both may spawn; the flock admits exactly one child, and the other child exits "already serving". Each `start` reports from the lock and the record: one "Watching", one "Already watching". Both exit 0, and one process remains. |
| Owner from an older binary after an upgrade | The record carries `dispatch_version`. `watch`, `start` and the status line say "watcher runs 0.4.5; this is 0.4.6: dispatch stop && dispatch start". |

No extra state machine: every question is answered by flock plus
`identity_state`.

## 5. IPC: none needed. How `watch` gets updates

- `watch` polls canonical state. Its doorbell is `SELECT MAX(id) FROM events` on
  `Database::open_read_only` (no migration). It also re-reads the lock and record
  state.
- It polls every 500 ms and reloads the project's runs only when the doorbell
  moved, or every 30 s so the "finished within the hour" window ages.
- Every change it shows is an event the owner or another command committed:
  verdicts, adoption, finish, apply, review, question.
- Rendering reuses `render_view`, plus one header line:
  - `Watching ~/src/project · background since 10:02 (pid 812)`, or
  - `Not watched: verdicts may be stale · dispatch start`.
- It exits on `shutdown_signal`, with code 0.

## 6. `status` (decided: keep, add a watcher line)

- `status [run-id]` keeps meaning one Work item, the latest when no ID is given,
  for human, `--json` and `--jsonl` output.
- Human output gains one final line: `Project: watched in the background since
  10:02` or `Project: not watched · dispatch start`.
- JSON is unchanged. The project snapshot lives in `watch`. A project-level
  `status` waits for the daemon's Work model.

## 7. Minimal refactors in `serve.rs`

Split by responsibility; do not rewrite. Keep the owner and the view in `serve.rs`,
and put process mechanics in a new `src/orchestrator/background.rs`.

- **Owner (product semantics).** `struct Owner { root, kind, policies,
  last_signals }` with `fn tick(&mut self, state) -> TickReport { moved,
  adopted, evaluated, persisted, applied, digest, errors, timings }`.
  - It contains `adopt_orphans`, `reevaluate` (plus D1-D4) and the auto-apply
    loop.
  - It prints nothing.
  - It loads the root's runs **once per tick**, filtering on the projected
    metadata's `source_path` before calling `load_run`.
- **View (presentation).** `view_rows`, `describe`, `render_view(runs, json,
  header)`. They are shared by foreground `serve` and `watch`.
- **Process (`background.rs`).** `start`, `stop`, the watcher record
  (read/write/remove), `watcher_state(root) -> Watched{record}/NotWatched`, the
  spawn, and the status line.
- **`serve` itself** becomes: lock → record → loop { `owner.tick()`; unless
  `--background`, render (and world JSON) } → remove the record.
- **D1:** step every tracked run's `Policy` each tick with `None`, so held
  verdicts are flushed.
- **D2:** `Policy::remembering(stored)` seeds `sent` from the run's stored
  validity when first seen.
- **D3:** for active unowned attached Work, the work signal is the SHA-256 of
  `snapshot_delta`, as the 0.4.4 native watcher does. Re-evaluate when the world
  signal or that run's work signal moved.
- **D4:** as in §3.

Foreground `serve` stays a supported, thin command: owner plus view in one
process, with the same output as 0.4.4.

## 8. Measurement (local only)

- `TickReport.timings` holds the durations of:
  - the world signal;
  - loading runs;
  - each Work evaluation;
  - auto-apply.

  With `-v`, each non-idle tick logs one `tracing::debug!` line to the log.
- Ignored test `serve_tick_cost` (`cargo test --test serve -- --ignored
  --nocapture`):
  - a Git fixture of 2,000 files plus 1, 5 and 20 attached Work items;
  - it prints idle-tick cost, world-signal cost and cost per re-evaluated item.
- Numbers from this machine go in the plan log; there is no pass/fail on time.
- A normal regression test asserts that an idle tick evaluates nothing.

## 9. Tests

**New `tests/background.rs`** (Unix; reuses the helpers in `tests/serve.rs`):
- `start` returns within 10 s, and the owner holds the lock with an `ExactLive`
  record. It works with **no `resources.yml`** and in a non-Git directory.
- `start` twice prints "Already watching", and one owner remains.
- Two concurrent `start`s leave exactly one owner, and both exit 0.
- `stop` stops that owner:
  - the lock is freed and the record removed;
  - a `serve` on another root keeps running;
  - `stop` when not watching prints "Not watching" and exits 0.
- **Crash:** SIGKILL the owner. `watch`/`status` then say "not watched", and
  `start` succeeds with a new record.
- **Reused pid:** a record naming an unrelated live `sleep`, with the lock free,
  is not signalled, and the `sleep` survives. With the lock held by a test holder
  and a mismatched identity, `stop` refuses.
- `start` with a broken `dispatch.yml` exits 1 and shows the log's error.
- `watch`:
  - it takes no lock (a `serve` can start while `watch` runs);
  - SIGINT exits 0 and the background owner survives;
  - it shows native and attached rows, and the header in both states.

**Owner semantics** (extend `tests/serve.rs`):
- D1: a held verdict reaches the metadata without another world move.
- D2: restart after REFRESH, with the world moved back; the stored verdict becomes
  CONTINUE.
- D3: foreign attached work edits into a conflict with no world move; REFRESH is
  stored.
- D4: a Ready native result goes REFRESH after the world changes, and a stored
  integration refusal for the same digest survives an owner pass.
- Idle tick: 0 evaluations.

**Status:** a watcher-line assertion in an existing `status` test, plus a
`Policy::remembering` unit test.

**Unchanged and green:** `serve.rs`, attach, auto-apply concurrency, coherence
(accept, integration, override, watch, matrix), native runs, `launch_record` and
the funding proofs.

## 10. Stages (one commit each on `release-0.4.5`; fmt, clippy and test green)

| # | Change | Main files |
|---|---|---|
| 0 | Plan into `docs/plan-0.4.5.md` | docs |
| 1 | Owner/view split; runs loaded once per tick, filtered by source first. No behavior change. | `serve.rs` |
| 2 | D1 + D2 (flush held verdicts; seed from stored) with tests | `serve.rs`, `coherence/watch.rs` |
| 3 | D3 work signal for unowned active attached Work; test | `serve.rs` |
| 4 | D4: owner re-checks Ready results via `shown_validity`; tests (including integration refusal kept) | `serve.rs` |
| 5 | Watcher record plus `serve --background` (no view, deduped log) | `background.rs`, `serve.rs`, `main.rs` |
| 6 | `dispatch start` / `dispatch stop` plus lifecycle and crash tests | `background.rs`, `main.rs`, `tests/background.rs` |
| 7 | `dispatch watch` (journal doorbell, header) plus the `status` watcher line | `serve.rs`, `background.rs`, `main.rs`, `orchestrator.rs` |
| 8 | Measurement: timings, `-v` log line, ignored cost test; numbers in the plan log | `serve.rs`, `tests/serve.rs` |
| 9 | Release: docs, a real-agent trial, version 0.4.5 | README, `docs/coherence.md`, `docs/attach.md`, `docs/product-guide.md`, `release-install.md`, release notes |

**Docs in stage 9:**
- README "Watching a project" becomes start/watch/stop.
- `docs/coherence.md` and `attach.md` describe the owner.
- The product guide gets the new commands.
- Remove "nothing watches in the background".

**Trial in stage 9:** `start` in the trial project, then:
1. an attached Claude/Cursor item plus a native item;
2. land a conflicting change and see `watch` show REFRESH without any command;
3. `stop`;
4. SIGKILL recovery.

Use `caffeinate -i`.

## 11. Risks

- **Detached process hygiene.** `setsid` plus `/dev/null` stdin and a log file
  keep the terminal free. The log grows only with distinct errors and is
  truncated on each start.
- **Persisting verdicts on finished runs (D4)** is new writing. It is kept safe by:
  - the run lock plus `state_revision`;
  - `shown_validity`, so integration refusals are kept;
  - only Ready, unapplied, review-pending results;
  - a test for each.

  Review: accept still gates on a live evaluation, so a stored verdict never
  authorizes anything.
- **Cost of D3** (a `snapshot_delta` per active foreign item per tick). It is
  measured in stage 8. If too slow, fall back to world-move-only for that item
  and log it; do not add an index.
- **Stale binaries after an upgrade** are surfaced through `dispatch_version`
  (§4).
- **macOS sleep and reboot.** Sleep pauses the owner harmlessly, since ticks are
  delayed, not bunched. After a reboot, watching needs `dispatch start` again, and
  the header says so.
- **Scope creep toward a daemon protocol.** There are no sockets, no RPC and no
  process discovery; `watch` only reads.

## 12. Definition of done

- In a project with no agent profile, `dispatch start` returns in under 10 s with
  "✓ Watching …", and the terminal can be closed without stopping it.
- After a change lands, `dispatch watch` shows a Ready result, native or
  attached, going REFRESH with no other command run. Ctrl+C leaves `watch`, and
  the owner keeps running.
- `dispatch stop` stops exactly that owner. After SIGKILL or a reboot, no command
  claims the project is watched, and `start` recovers.
- Concurrent `start`s leave one owner. `stop` never signals a pid whose identity
  is not `ExactLive`.
- D1–D4 are fixed with tests. `serve` in the foreground behaves as before.
  `status [run-id]` is unchanged, apart from the watcher line.
- Costs are measured and logged. fmt, clippy and the full suite are green. Nothing
  in the 0.4.4 coherence behavior changes meaning.

## 13. Deferred to automatic Work discovery and later

- Discovering Claude/Codex/Cursor processes, and inferring S0 for Work Dispatch
  did not launch or attach.
- Herdr; public RPC or sockets; remote service.
- Start at login (launchd or a systemd user unit), and one daemon for all projects.
- A project-level `status`.
- Moving crash repair out of `load_run`.
- Conflict or dependency graphs, indexed invalidation, semantic redundancy, Work
  Rebase.

## Verification

- Per stage: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`. After any CI failure, rerun with `--no-fail-fast`, since CI stops
  at the first failing binary.
- By hand, in `/private/tmp/dispatch-dogfood/coherence-044/repo` with
  `DISPATCH_HOME=…/state-041`:
  1. `start`, then close the terminal;
  2. `watch` in another terminal, then land a change and see REFRESH appear;
  3. `kill -9` the owner, then `watch` says "not watched" and `start` recovers;
  4. `stop`.
- Cost: `cargo test --test serve serve_tick_cost -- --ignored --nocapture`.

## Progress log

- 2026-09-24: Stage 0. Plan committed.
- Stage 1. The Owner/View split. Runs are loaded once per tick, and only the
  root's (the source path comes from the projected metadata). No behavior change.
- Stage 2 (D1, D2). The in-memory `Policy` is replaced for the owner: each verdict
  is compared with the verdict stored on the run, which is reloaded under the run
  lock. This uses `watch::worth_recording`, the same rule the watcher's messages
  use. Both defects are fixed by construction, and nothing is lost on restart.
  - Tests: `a_verdict_that_changes_back_within_a_minute_is_still_recorded` and
    `a_restarted_owner_records_a_change_the_old_one_never_saw`. Both fail on
    the old code.
  - A full-suite run once failed `profile_selection::quota_failure_mid_attempt…`
    with empty output. It passed 5/5 on rerun, on a path stage 2 does not touch.
- Stage 3 (D3). The owner follows active, unowned attached work by a hash of its
  patch, taken without the run lock. `finish` and `accept` take that lock without
  waiting, so the owner holds it only when the world or the work moved.
  `serve_follows_foreign_work_that_edits_into_a_change_already_made` fails on
  the old code.
- Stage 4 (D4). The owner re-checks Ready, unapplied, review-pending results
  (native and attached) with `coherence::live_validity`, which is what `check`
  shows. It checks when a result becomes Ready and whenever the world signal
  moves.
  - The evaluation takes no lock. The lock is held only to record, and only if
    `state_revision` is unchanged.
  - The first check after the world moved is stored even when it is CONTINUE,
    so the view shows CONTINUE, not "not checked". In stage 9 this became every
    first check, so an unmoved result shows `unmoved`.
  - `transition` accepts `coherence.*` events on finished runs.
  - Tests: `the_owner_keeps_a_refusal_by_the_checks_until_the_world_moves` and
    `the_owner_marks_a_ready_native_result_stale_when_the_world_moves`.
- Stages 5-6. `background.rs` holds the record (`watchers/<key>.json`), the
  watched/not-watched check (a probe of the serve lock), `start` and `stop`.
  `serve --background` renders nothing, and its log names each distinct error
  once.
  - `serve` waits up to 1 s for its lock, because the check probes it.
  - It now creates its shutdown listener once, so a SIGTERM that arrives
    mid-tick is not lost.
  - `start` validates `dispatch.yml` before spawning. It starts the owner with
    `setsid` and `/dev/null` stdin, and waits for a record that names the child
    with an `ExactLive` identity.
  - `stop` signals only an `ExactLive` identity while the lock is held.
  - `tests/background.rs` has 7 lifecycle tests. By hand, `start` returned in
    0.48 s.
- Stage 7. `dispatch watch`:
  - Its doorbell is `MAX(id) FROM events` through a read-only connection, plus
    the watcher line, plus a 30 s redraw.
  - The header names who watches. It exits 0 on Ctrl+C and holds no lock.
  - `status` ends with `Project: …`. `watch_follows_the_project_and_leaving_it_keeps_watching`
    sees REFRESH from the background owner.
- Stage 8. `Tick.cost` records the signal, run loading, work following,
  evaluations, rechecks and auto-apply. `-v` logs each non-idle tick and `-vv`
  logs every tick. `an_idle_tick_evaluates_nothing` asserts 0 evaluated and 0
  rechecked on idle ticks.
- Stage 8 measurements (`serve_tick_cost`: this Mac, Git root of 2,000 files,
  foreign attached items with one edit each; Defender and DLP scanning active):

  | items | signal | load runs | follow (idle) | evaluate (world moved) |
  |---|---|---|---|---|
  | 1 | ~48 ms | 3-7 ms | 200 ms → **50 ms** | 350-480 ms |
  | 5 | ~48 ms | 11-20 ms | 1.0 s → **250 ms** | 1.7-2.3 s |
  | 20 | ~48 ms | 32-46 ms | 4-5 s → **1.0-1.4 s** | 6.9-11.2 s |

  - The follow cost was a fresh temporary index for each snapshot, so `git add
    -A` re-hashed the whole workspace every tick.
  - `source::snapshot_delta_indexed` keeps one index per followed run, so Git's
    stat cache re-hashes only changed files. A unit test compares it with a
    fresh index across edits, a same-size rewrite, a revert and deletions.
  - Evaluation after a world move is roughly linear, at about 0.4-0.5 s per
    item. That is the measured case for later indexed invalidation; nothing is
    built for it here.
- Test-suite load. During stage 7, `native_runs` and `review_session` failed in
  parallel with "Codex account could not be read (probe timeout before
  initialize response)". The committed stage 6 code failed the same way at that
  time; serially, all 40 pass.
  - The cause is environmental: the first exec of a freshly written script took
    about 200 ms each while Defender and a DLP scanner were busy, and those
    tests write a new fake agent per test, with a 5 s probe budget.
- Stage 9. Release.
  - Docs: README "Watching a project", the attach.md "The project owner"
    section, the coherence.md watcher section, the product guide, the 0.4.5
    upgrade note and the release notes. The version is 0.4.5.
  - The real-agent trial (`coherence-044`, `state-041`, log in `logs045/`) found
    three presentation defects, all fixed with tests:
    - results waiting for review dropped out of the view after an hour;
    - reasons quoting Git's errors broke the one-line rows;
    - the owner's log carried terminal colour codes.

    It also changed the owner to store its first check of any Work, so an unmoved
    result shows `unmoved`, not "not checked".
  - Trial results:
    - `start` returned in 0.10 s.
    - The first tick re-checked the 4 results waiting for review and recorded
      correct verdicts: two REFRESH `patch_conflict` (D, D-refresh2), REFRESH
      `fact_broken` (B) and STOP `already_applied` (C2).
    - A native Claude item and a foreign Cursor item then ran in parallel.
      Cursor reported edits but left the workspace unchanged, because the
      requested behavior already existed.
    - Teammate commit `51a18a2` touched `truncate` and added `reverse_words`.
      Within 2 s the owner recorded the Claude result as REFRESH (patch no
      longer applies) and the Cursor work as CONTINUE, and `watch` showed both.
    - After SIGKILL, `status` and `watch` said "not watched" and `stop` signalled
      nothing. `start` recovered, and `stop` ended the owner.
  - Full suite at 4 threads: 457 passed, 1 failed. The failure was
    `funding_safety::claude_refusal_is_sticky_until_reauthorized` (a fake
    Claude probe under load, on a path 0.4.5 does not touch). It passed 5/5 on
    rerun.
- Review fixes before merge.
  - `watchers/` is made 0700 before anything is written into it, and the owner's
    log is created 0600. A log or directory left by an earlier build is tightened
    too. Records and logs name paths and carry detailed errors.
  - "Watched" now means `flock` contention and nothing else.
    `OperationLock::is_held` returns true only when the lock is busy. A refused
    symlink or a filesystem error is an error for `start`, `stop` and the
    `Project:` line, where it used to read as "watched".
  - Tests: `is_held_means_contention_and_nothing_else`,
    `the_watcher_directory_and_log_are_private` and
    `a_lock_that_cannot_be_probed_is_an_error_not_a_watcher`. The last two fail
    on the previous code.
  - The plan's start deadline now says 10 s, as implemented.
