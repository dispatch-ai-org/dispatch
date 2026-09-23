# Dispatch 0.4.1: pruning plan

v0.4.1 is a subtractive release. It deletes the product archaeology that still
decides how a native run starts, what `RunRecord` holds and what the database
commit does, and it keeps the coherence and integration substrate unchanged.
It builds no daemon, protocol, Herdr integration or Work Rebase, and it pulls in
none of the 0.4.2 items.

The audit that led to this plan was done against `v0.4.0` (`a717977`). The full
decision document lives with the integrator; this file is the shared spec and
the progress log.

## Decisions

- Sync (cloud contribution) is removed entirely. Local evaluation rows stay.
- The automatic stronger-model retry is removed. Clarification questions stay.
- A native run uses a configured profile or an explicit `--agent`. With
  neither, it refuses with "run `dispatch setup` or pass `--agent`"; there is
  no zero-setup fallback.
- The control protocol (`control`, `control-grant`) is removed. `dispatch
  events` remains the read-only journal follower.
- A safety invariant is never replaced in the same change that deletes the
  subsystem providing it. The replacement lands first, runs beside the old
  mechanism and is proven by its own tests; the deletion comes later and must
  not modify those tests.

## Stages

| Stage | Content |
|---|---|
| S0 | Branch `release-0.4.1` from `v0.4.0`; record the baseline; this file |
| S1 | Remove benchmark/public-evidence and sync: `datasets`, `public_priors`, `recommend`, `evidence local`, `data refresh`, `sync`, `--route`, automatic routing |
| S2a | Remove private evidence (commands, selection hook, TUI attest prompt, config) |
| S2b | Remove planning (`--plan`, `/plan`, `--max-invocations`, `planning.rs`, `planned.rs`, hooks) |
| S3 | Remove the control protocol; keep the question-authorization helpers |
| S4 | Extract `ProcessIdentity` and friends to `process.rs`, `OperationLock` to `lock.rs`; named imports in attach/serve/apply |
| S5a | **Additive.** Codex funding-identity refusal as an adapter preflight, beside the existing capacity/authorization check, with a proof suite |
| S5b | One selection rule (profiles filtered by `--agent/--model/--effort`, first eligible); remove router, classifier, recovery retry |
| S6a | **Additive.** Per-attempt durable child-launch record; `repair_abandoned` decides from it (with the admission check as an extra guard), with a proof suite |
| S6b | **Gated on S5a + S6a.** Delete admission and capacity; the S5a/S6a suites are not modified |
| S7a | Every native run goes through the one native engine; migrate tests from `--harnesses fake-* … apply <id> A` to `--agent fake-* … accept <id>` |
| S7b | Delete the inline Routed/Comparison loop, `compare`, `evaluate`, `inspect`, hidden `apply`; rename `phase3.rs` to `native.rs` |
| S8 | `RunMode` → `{Native, Attached}`; one human-review record; single-lock accept; migration 22 drops the obsolete tables |
| S9 | Shared Work constructor and delivery step; watcher and serve share one observe/settle step |
| S10 | CLI, config and docs tightening; release notes; real-agent dogfood; release |

## Invariants every stage must keep

- **Coherence.** The 36-scenario matrix on Git and plain sources; accept,
  check, refresh, integration (L2), recovery, upgrade and the mid-run watcher,
  including `mid_run: stop` cancelling stale native work.
- **Integration.** Two runs finishing against one source serialize on the source
  lock and the second is re-judged; an edit during integration checks is fenced
  and re-validated once; `apply_checked` fences before and after its dry run.
- **Authority.** `applied_by` distinguishes human from auto-apply; auto-apply
  never records a review; auto-apply requires passed verification, and
  integration-level analysis when the world moved.
- **Attach and serve.** Foreign and wrapped attach, finish, the review rule,
  adoption of a gone owner, serve integration and restart.
- **Source safety.** Ignored build artifacts are not force-tracked into
  baselines; the unsafe-local acknowledgement is required; the state directory
  stays outside the source.
- **Process cleanup.** Process-group termination and Docker cleanup on every
  exit path.
- **Persistence.** `state_revision` refuses concurrent writers; per-run event
  sequence; older runs keep loading; upgrades back up the database first.
- **Human review.** Review requires a delivered result; rejection closes the
  work; an application by policy is never recorded as human acceptance.
- **Funding safety (gate for S6b).** A profile never launches when the account
  or funding identity the adapter observes differs from the one the user
  authorized.
- **Crash repair (gate for S6b).** A run is never closed while an agent child
  it launched may still be alive.

## Progress log

- 2026-09-22 — S0: branch `release-0.4.1` from `v0.4.0`. Baseline `cargo test`:
  728 passed, 10 failed, 3 ignored. Eight `terminal_bench` failures and the
  doctest failure were caused by S1 edits made while the baseline was still
  running; `pty_question_deadline` is the known load-sensitive fixture;
  `claude_scoped_control_verified_review` refused because the locally installed
  Claude CLI version differs from the fixture's subscription evidence
  (environmental; the test belongs to the control protocol removed in S3).
- 2026-09-22 — S1: removed `datasets`, `public_priors`, `evidence` (local),
  `sync`, `recommend`, `data refresh`, `--route`, automatic routing and the
  benchmark ranking half of the router, the bundled prior snapshot and the
  envelope schemas. A native run with no configured profile and no `--agent`
  is now refused. Tables are untouched until migration 22. Production −5.1K
  lines, tests −3.6K. `cargo test`: 635 passed, 2 failed, both passing on rerun
  (`phase4_pty_intent_answer_recovery_review_and_restoration`, a known
  load-sensitive PTY fixture; the converted unsafe-local test, fixed).
- 2026-09-22 — S2a: removed private evidence (`evidence private|propose|
  annotate|policy|activate|rollback`, trial selection, the TUI "use this review
  for local routing" prompt, `private_evidence` config, control-grant policy
  pinning). `AllocationDecision.private_evidence` stays as an opaque JSON value
  because the `private_decision_immutable` trigger refuses any projection
  rewrite that changes it; migration 22 drops the trigger. `cargo test`:
  609 passed, 3 failed in `phase8_planning`: `private_attribution` (its
  behavior was removed here, case deleted) and the two load-sensitive deadline
  fixtures, which pass in isolation.
- 2026-09-22 — S2b: removed planning (`--plan`, `/plan`, `--max-invocations`,
  `control-grant --allow-plan`, `planning.rs`, `planned.rs`, the planning
  hooks in the event commit, apply, admission fence and TUI, and the
  planned-only source helpers). A planned goal from 0.4.0 that is still
  awaiting review is refused at apply (its delivery chain can no longer be
  verified) rather than applied unverified; refresh it instead. `cargo test`:
  543 passed, 1 failed (`claude_control_slow_output`, a known load-sensitive
  control fixture; control is removed in S3).
- 2026-09-22 — S3: removed the control protocol (`control --stdio`,
  `control-grant`, sessions, grants, receipts, `commands/inspection.rs`, the
  receipt and actor hooks in the event commit, control-only checkpoint
  recovery, `examples/control_client.py`, `docs/control-protocol.md`). The
  question-authorization helpers were all control-scoped no-ops for local
  callers; `dispatch answer`/`cancel` keep their OS-owner check in the native
  engine. The fake-provider fixture moved to `tests/fixtures/provider_fixture.py`;
  the control-driven portfolio scenarios (`grants`, `pools`) were deleted
  (same-pool admission exclusion is removed in S6b). `cargo test`: 489 passed,
  0 failed.
- 2026-09-22 — S4 (moves only, no behavior change): process identity and
  liveness (`ProcessIdentity`, `identity_state`, `process_group_exists`,
  `identity_from_row`) moved from `admission.rs` to `src/process.rs`;
  `OperationLock`, `SignalListener` and `shutdown_signal` moved to
  `src/lock.rs`; the event helper `transition` moved next to `persist_event`;
  `attach`, `serve` and `apply` import named items instead of `super::*`.
  `cargo test`: 489 passed, 0 failed.
