# Dispatch 0.4.4: verdicts you can trust and act on

## Context

On 2026-09-24 a multi-agent trial ran eleven Work items against one project from a
shared starting point, S0. The runtimes were Claude through Dispatch, attached Claude
Code and attached Cursor. The project changed underneath them on purpose.

The core thesis held:
- An attached patch applied cleanly but still called `validate(token)` after the API
  had become `validate(ctx, token)`. Dispatch refused it with REFRESH (`fact_broken`)
  and a precise before-and-after signature.
- An identical redundant patch got STOP.
- A change that broke a test landed by other Work was refused by the integration
  checks.
- Three unrelated changes, one of them in a file a teammate had also edited,
  continued correctly.

The trial also found where the verdicts cannot yet be trusted or acted on:

| # | Finding | Class |
|---|---|---|
| F1 | The mid-run watcher re-evaluates only when the world moves, never when the agent's work changes. A change that lands before the agent touches the same code is never reported mid-run. | correctness |
| F2 | An accept refused by coherence still records `review: accepted`. The human-judgment record says stale work was accepted, and the run leaves the pending list. | correctness, data integrity |
| F3 | After an integration check refuses an accept, `check` and `status` recompute only file and symbol facts and show CONTINUE ("Next: dispatch accept"), while `serve` shows REFRESH. | correctness of what is shown |
| F4 | An integration failure is reported as a command, an exit code and a log path. The failing test is only in the log. | explanation |
| F5 | `refresh` tells the new agent only the live file and symbol reasons. After an integration failure it says "files changed underneath the work", and the refreshed agent repeated the failure. | explanation, refresh loop |
| F6 | `check` and `accept` advise `dispatch refresh` for attached work, which refresh refuses. | UX |
| F7 | A compatible Python signature change (an added optional parameter) marks every caller `fact_broken`. The patch applied and all tests passed, but the only way forward was to reject and redo. | false REFRESH |
| F8 | Native S0 shows Dispatch's internal snapshot commit, not the project commit. `history` says "not checked" for results applied to an unmoved source. STOP does not say which Work already landed the change. | UX, explanation |
| F9 | Two agents built the same capability under different names, and both landed. | research, out of scope |

## Decisions

- **Both answers to F7.**
  - A narrow compatible-signature rule removes the common false REFRESH.
  - A recorded human override covers what the rule cannot see.
- **The override is a judgment, not a bypass.**
  - It applies only to REFRESH reasons that come from analysis: `fact_broken`,
    `fact_missing`, `same_symbol_edited` and `analysis_uncertain`.
  - It never applies to STOP (`already_applied`), `patch_conflict` or
    `integration_check_failed`.
  - Integration checks still run on the merged tree and must pass; the fences are
    unchanged.
  - A written explanation is required. It is recorded with the overridden verdict,
    as a human application.
  - Auto-apply never overrides.
- **A review records a human decision about work that landed or was rejected.**
  An accept that coherence refuses records nothing; the run stays pending.
- **One effective verdict.**
  - `check`, `status`, `explain`, `refresh` and the service view all show the same
    verdict.
  - A stored integration verdict for the exact world being observed outranks a
    fresh file and symbol evaluation of that same world.

## Stages

Each stage is one commit on `release-0.4.4`. Every stage must pass
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`.

Untouched throughout:
- the migrations;
- the funding preflight and the launch record;
- apply fencing (`apply_checked`, the digest fences);
- the proof suites.

### 1. A refused accept records no review (F2)

- **Change:** `orchestrator::review_locked`.
  - Accept applies first, through `apply::apply_locked` with human authority, which
    already sets `review: accepted`.
  - Only when the application succeeds does it record the review revision
    (`save_goal_feedback`) and the `review.accepted` event.
  - A refused application records `application.failed` as today, and the review
    stays `pending`.
  - An auto-applied run still records only the review. Reject is unchanged.
- **Tests:** in `coherence_accept.rs` and `attach_cli.rs`, a refused accept leaves
  `review: pending`, no review revision and no `review.accepted` event, and the run
  is still the latest pending one.
- **Docs:** product guide, "Review".

### 2. One effective verdict (F3, F5)

- **Change:** add `coherence::effective_validity(run)`. It is the live L0+L1
  evaluation, except that a stored validity at `integration` level that is not
  CONTINUE, for the same world digest, is returned instead.
  - `check` uses it; `with_live_validity` uses it for `status` and `explain`; and
    `refresh_request` takes its reasons from it.
  - `check`'s next step then follows the effective verdict.
- **Tests:**
  - After an integration refusal, `check --json` and `status --json` report
    REFRESH with the integration reason.
  - After the world moves again, they report the fresh evaluation.
  - A refresh task names the failing check.
- **Docs:** coherence reference, "check / status".

### 3. Integration failures name what failed (F4)

- **Change:** `coherence::integration::failure_reasons` adds one excerpt from the
  check's output.
  - The excerpt is the first line matching `FAIL`, `Error`, `error:`, `assert` or
    `panicked`, searching stderr then stdout; otherwise it is the last non-empty
    line. It is clipped.
  - It is placed before the log path, within the existing detail cap. No language
    is special-cased.
- **Tests:** unit tests with unittest, cargo-test and plain shell failure logs.
- **Docs:** coherence reference, "Reasons".

### 4. The mid-run watcher also follows the work (F1)

- **Change:** `coherence::watch::check` remembers two signals: the world, as
  today, and the candidate workspace, using the same size-and-mtime walk as
  `world::signal`. It re-evaluates when either moves.
  - `Policy` is unchanged, so no extra messages are sent for an unchanged verdict.
  - This covers native runs and wrapped attach, which share the watcher.
- **Tests:** in `coherence_watch.rs`, with a gated fixture agent: the world change
  lands first, then the agent edits the same function, then the attempt ends. A
  `coherence.invalidated` event is recorded during the attempt, and with
  `mid_run: stop` the run is stopped.
- **Docs:** coherence reference, "The mid-run watcher"; update the
  coherence-validation claim.

### 5. Compatible Python signatures (F7, the rule)

- **Change:** in `coherence::facts`, when a referenced Python function's signature
  changed, it still holds when both of these are true:
  - every original parameter is unchanged, in name, kind, order, default and
    annotation;
  - every addition is a parameter with a default, a keyword-only parameter with a
    default, `*args` or `**kwargs`.
  - Both signatures are parsed with the existing tree-sitter Python grammar.
    Return annotations must be unchanged.
  - Rust signatures are unchanged in behavior, and so are Modified facts.
- **Tests:** in the facts unit tests, cover:
  - an added defaulted parameter holds;
  - an added `*args` or `**kwargs` holds;
  - an added required parameter breaks;
  - a removed, renamed or reordered parameter breaks;
  - a changed default breaks;
  - a changed annotation breaks;
  - Rust is unaffected.
  The trial's E case (`format_user(user)` → `format_user(user, brackets="()")`)
  becomes CONTINUE.
- **Docs:** coherence reference, "What it checks".

### 6. Recorded human override (F7, the override)

- **Change:** `dispatch accept <run> --despite-refresh --explanation <text|file>`.
  - The explanation is required. The accept also runs the normal apply path under
    the same lock.
  - The gate's REFRESH is overridable only when every reason is an analysis reason
    (see the decisions).
  - Integration checks run on the merged tree regardless of the level reached;
    a failing, timed-out or unrunnable check refuses the override.
  - It commits a `coherence.overridden {coherence, explanation}` event, then the
    usual `result.applied` (human) and the review revision.
  - `status`, `history` and the service view show "applied over REFRESH".
  - The review menu gets the same choice only as an explicit second step, never as
    Enter.
  - Auto-apply and `serve` never override.
- **Tests:**
  - STOP, `patch_conflict` and integration failures cannot be overridden.
  - A missing explanation is refused.
  - A failing check refuses the override and changes nothing.
  - A successful override records the event, `applied_by: human` and the
    explanation.
  - Auto-apply ignores the flag.
- **Docs:**
  - README "Accept-time flow";
  - coherence reference;
  - coherence-validation "Bypasses" (overrides are counted there).

### 7. Attached advice, S0 and history wording (F6, F8)

- **Change:**
  - `CoherenceBlocked` and `check` choose their advice by mode. Attached work gets
    "run your agent again on the current source and attach it, or dispatch
    reject".
  - The Work line shows native S0 as `snapshot at <source HEAD>` when the source is
    Git; otherwise `directory snapshot`.
  - A result applied to an unmoved world shows `unmoved`, not `not checked`.
  - STOP names the landed Work when an applied run on the same source has the same
    patch.
- **Tests:** unit tests for `work_line`; `attach_cli.rs` advice; the STOP naming
  case in `coherence_accept.rs`.
- **Docs:** attach reference.

### 8. Release

- Re-run the multi-agent trial's scenarios against the finished branch as a
  regression trial. Use the same project shape and the same kinds of cases, with
  real agents, and log the result in this plan.
- Expected:
  - every case the trial got right stays right;
  - E becomes CONTINUE;
  - D-refresh's conflict is reported mid-run;
  - refused accepts stay pending;
  - D's refusal is consistent across `check`, `status` and `serve`.
- Release notes, install note, version 0.4.4. No migration.
- Official documentation (README, `docs/attach.md`, `docs/coherence-validation.md`
  and the release notes) describes these runs as real-agent trials and never uses
  the word "dogfood". Reword the three existing uses in `docs/attach.md` and
  `docs/coherence-validation.md`.

## Definition of done

- Every finding F1-F8 has a regression test that fails on 0.4.3 and passes on
  0.4.4.
- The regression trial reproduces the expected verdicts with real agents.
- No refused accept records a review.
- A human can land analysis-only REFRESH work with a recorded explanation, and
  only after the merged tree's checks pass.
- `check`, `status`, `explain`, `refresh` and `serve` agree on the verdict for the
  same world.
- fmt, clippy and the full suite are clean. The proof suites are unmodified.

## Risks

- **The override must not become a habit.** It is confined to analysis reasons,
  requires passing checks and an explanation, and is counted in the validation
  metrics. Never add it to the default review path.
- **The compatible-signature rule must stay narrow.** Only additions with
  defaults, `*args` and `**kwargs`; everything else is still `fact_broken`. It
  does not see keyword-argument collisions from existing `**kwargs` forwarding, so
  the override exists for such cases.
- **Watching the workspace adds one directory walk per tick.** Measure it on the
  trial project and on the Dispatch repository; keep the poll interval.
- **Changing the review order changes the event order** (`result.applied` before
  `review.accepted`). Check the event-order assertions in `auto_apply.rs` and
  `review_session.rs`.

## Deferred

- Semantic redundancy (F9) and behavior-level facts: longer-term research.
- The daemon, and the real two-agent conflict trial under a long-lived authority.
- A live verdict for native Work in `serve`.

## Progress log

- 2026-09-24 — Stage 1 (F2). An accept applies first and records the review
  only after the change landed. A refused accept records `application.failed`
  and leaves the review pending, with no revision and no `review.accepted`
  event, and a bare `dispatch accept` still finds the run.
  - Revising a rejection (reject, then accept) still works:
    `validate_terminal_write` counts a human application (`applied`,
    `applied_by: human`) as the human decision that may reopen rejected work. A
    policy application may not.
  - Tests:
    - REFRESH and STOP refusals in `coherence_accept.rs`, and attached work in
      `attach_cli.rs`. Both fail on 0.4.3.
    - The drift test in `native_runs.rs` now expects `pending`.
    - A unit test for the reopen rule.
  - README and product guide updated.
  - `cargo test`: 431 passed, 0 failed.
- 2026-09-24 — Stage 2 (F3, F5). `coherence::shown_validity` is the verdict
  that `check`, `status`, `explain`, the TUI and `refresh` show. It is the fresh
  L0 and L1 evaluation, except that a stored non-CONTINUE verdict with an
  integration reason, for the same world digest, is shown instead.
  - An integration refusal is stored with its L1 analysis level, not
    `integration`, so it is recognized by its reasons
    (`integration::is_integration_reason`). Mid-run verdicts, which judged a
    work-in-progress patch, are never resurrected.
  - The accept gate still evaluates afresh (`evaluate_run`), so a retry runs the
    checks again and a flaky check does not stick.
  - Test: `coherence_integration.rs`. After the refusal, `check` (JSON and
    human, "Next: dispatch refresh") and `status` report the integration REFRESH.
    `refresh` hands the new agent "check `…` failed". After the world moves,
    `check` shows the fresh CONTINUE. The test fails on stage 1 (`continue`).
  - `cargo test`: 431 passed, 0 failed.
- 2026-09-24 — Stage 3 (F4). An integration failure's reason quotes what
  failed: the first two lines that read like a failure (`FAIL`, `Error`,
  `error:`, `assert`, `panicked`), stderr first, else the last non-empty line.
  - This goes between the exit code and the log path. Integration reasons get
    600 characters and the excerpt 240, so the log path is never cut off.
  - A check that prints nothing keeps the 0.4.3 wording.
  - Test: unit tests with the trial's real unittest failure and a real
    `cargo test` failure.
  - `cargo test`: 432 passed, 0 failed.
- 2026-09-24 — Stage 4 (F1). The watcher remembers two signals, the world's and
  the work's, and evaluates again when either changes.
  - The work signal is a hash of the work-in-progress patch, which the watcher
    already builds with the trusted baseline repository and a temporary index.
    It honors the ignore rules and never runs the workspace's own Git
    configuration.
  - Rejected alternatives:
    - A directory walk of the workspace: this repository's 485,000 files
      (24 GB `target/`) took 10.6 s, one full poll interval.
    - `git status` in the workspace: the agent controls `.git/config` there,
      including `core.fsmonitor`.
  - Test: `coherence_watch.rs`. The source moves while the agent has written
    nothing; the agent then edits the same line; `coherence.invalidated` is
    recorded before it finishes. On 0.4.3 it is never recorded.
  - No test binary got measurably slower.
  - `cargo test`: 433 passed, 0 failed.
- 2026-09-24 — Stage 5 (F7, the rule). `symbols::python_call_compatible` parses
  both headers with the existing Python grammar. A referenced Python function
  still holds when only optional parameters were added: defaults, `*args`,
  `**kwargs`, or `*` followed by defaulted keyword-only parameters. Every
  original parameter, the name, `async` and the return annotation must be
  unchanged.
  - Anything unparsable or cut short for display is not compatible. Modified
    facts and Rust are untouched.
  - Tests:
    - 19 header pairs, covering every accepted and refused shape;
    - the trial's `format_user(user)` → `format_user(user, brackets="()")` holds
      at the facts level, while a required parameter still breaks. That test
      fails without the rule.
    - The 36-scenario matrix is unchanged.
  - `cargo test`: 435 passed, 0 failed.
- 2026-09-24 — Stage 6 (F7 the override, and F6). `dispatch accept
  --despite-refresh --explanation <why>`.
  - `coherence::overridable` allows only a REFRESH whose every reason comes
    from the analysis.
  - `coherence::gate` then runs the checks on the merged tree regardless of
    level. It returns `AcceptGate::Overridden` only when they ran and passed;
    a failing check or no runnable check refuses.
  - The apply uses the verified world digest under the usual locks and fences.
  - The record: `CoherenceRecord.overridden` (serialized JSON, no migration),
    a `coherence.overridden {coherence, explanation}` event before the review,
    and `applied_by: human`. The Work line, `history` and `serve --json`
    (`overridden: true`) show it.
  - Auto-apply's `decide` passes no override, and auto-apply and `serve` never
    override.
  - F6 rides along, since it is the same refusal text: attached work is advised
    to run the agent again and attach, in both `accept`'s refusal and `check`'s
    next step. A non-overridable refusal says why.
  - Deviation from the plan: the review menu does not offer the override. It
    needs a typed explanation, and keeping it a deliberate command keeps it off
    the default path.
  - Tests: `coherence_override.rs` (applied with explanation and recorded; a
    failing check refuses; STOP refuses; no checks refuses; `history` shows
    it); the attached advice in `attach_cli.rs`; the refusal texts in unit
    tests.
  - `cargo test`: 439 passed, 0 failed.
- 2026-09-24 — Stage 7 (F8).
  - The Work line names native S0 by the project commit (`snapshot at
    eec41cc7`), or `directory snapshot` for a plain directory, never Dispatch's
    internal baseline commit.
  - A result applied with no stored verdict shows `unmoved`.
  - STOP names the applied run that already landed the same patch, in `check`
    (human, and `landed_by` in JSON) and in `accept`'s refusal. `index` lines are
    ignored; other runs' stored projections are read without loading or
    repairing them.
  - Tests: `work_line` unit tests; `coherence_accept.rs` with two identical runs
    (the second is STOP, landed by the first); the `product_ux` S0 wording.
  - `cargo test`: 440 passed, 0 failed.
- 2026-09-24 — Stage 8. Release and regression trial.
  - Regression trial with real agents on `release-0.4.4`: the same project shape,
    S0 and tasks as the first trial (`/private/tmp/dispatch-dogfood/coherence-044`);
    Claude through Dispatch, attached Claude Code, attached Cursor.
  - W-API, M1, W, C1 and A: CONTINUE and landed, as before.
  - B (a new caller of `validate(token)`): REFRESH `fact_broken`, with attached
    advice ("run your agent again … and attach"); the review stays pending.
  - An override of B was refused because the merged tree's tests failed. The
    refusal quotes `TypeError: validate() missing 1 required positional argument`.
  - C2: STOP "landed by run" C1.
  - D: refused by the integration check, naming `test_whoami_alice_admin`.
    `check`, `status` and `serve` then all showed REFRESH, and the review stayed
    pending.
  - E (`format_user(user)` after M2's optional parameter): CONTINUE, applied. It
    was a false REFRESH in 0.4.3.
  - D-refresh: given the integration failure, the agent asked whether it may also
    update `tests/test_handlers.py`. In 0.4.3 it repeated the failure.
  - The mid-run watcher was not exercised by a real agent: the refreshed agent
    asked before editing, and the continuation below never started. F1 rests on
    the deterministic test in `coherence_watch.rs`.
  - Fixes from this trial, both covered by tests:
    - The excerpt matched a test's source line (`self.assertEqual(...)`). `assert`
      now counts only at the start of a line, and `ERROR` is a marker.
    - A check-refused override said "cannot be overridden". It now says the checks
      refused it.
  - **New finding F10.** Answering D-refresh's question stopped the
    run at once with `source_drift` ("source drift before fresh attempt"). A native
    run refuses any fresh attempt, including the continuation after an answer,
    once the source has changed at all. In a moving project, answering a
    clarification question therefore always ends the goal, although the accept
    gate could judge the continuation's result. This drift stop predates the
    coherence model (it guarded planned tasks and the removed retry).
- Stage 8b (F10): decided to fix in 0.4.4. The drift stop before a fresh attempt
  is removed: every attempt still starts from S0, the watcher reports the move,
  and the accept gate judges the result. Regression test
  `answering_after_the_source_moved_continues_the_goal` fails on the old code with
  `source_drift` and passes now. Full suite: 441 passed.
  - Official docs no longer say "dogfood". Release notes, install note, version
    0.4.4.
- Stage 8b real-agent re-run (trial project `coherence-044`, native Claude,
  claude-sonnet-5):
  - Re-refreshing D, the agent did not ask this time. Its patch also removed M2's
    `brackets` parameter; the verdict was CONTINUE, because M2 was already in its
    S0. Human review has to catch that kind of loss. The result is left pending.
  - An ask-first task was refused at launch once, correctly: the Mac slept
    14:12:47-14:23:42 UTC, and the Claude approval expired at 14:23:10.
  - After revalidation, run `01M3A25PWQ81BX62XS8DE3VJRC` asked whether `shout`
    appends "!". A teammate then landed `textutil.initials` in the same file
    (`a1f510c`), and the answer was given. The continuation ran (2 attempts), the
    checks passed, and `check` said CONTINUE (symbols, 1 world file). Accept
    applied the patch beside `initials`, and the project tests passed. F10 is
    fixed with a real agent.
  - Found while it waited: `history` showed the run as `working`. It now shows
    `question`, asserted in the F10 regression test.
