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

## S5a specification: funding refusals the replacement must reproduce

Enumerated from `v0.4.0` + S1–S4 (`capacity.rs`, `admission.rs::authorize_launch`,
`orchestrator.rs::{select_available_resource, admit_attempt}`, `setup.rs`,
`harness/claude.rs`). Every refusal below happens before any agent child is
spawned. The capacity and authorization path stays active until S6b.

### Codex (`codex app-server` `account/read` + rate limits, `normalize_codex`)

| # | Refusal today | Input |
|---|---|---|
| C1 | Authentication reported as anything but `chatgpt` (e.g. an API key) | `account.type` |
| C2 | Paid credits reported available on any allowance bucket | `credits.hasCredits` |
| C3 | Service tier reported as anything but `standard`/`default` | `serviceTier` |
| C4 | Plan reported differs from the profile's `chatgpt-<plan>` funding source | `account.planType` |
| C5 | Account identity (sha256 of email or id) reported and different from the authorized identity. Today the authorized identity is the **first observation under the profile's `authorization_revision`**, not the setup probe | `account.email`/`id` |
| C6 | Allowance-window identity changed for an explicitly mapped pool (`provider_buckets`) | rate-limit ids |
| C7 | **Sticky**: any conflict writes a `rejected` authorization; that revision stays refused, even if the account switches back, until setup re-authorizes (increments `authorization_revision`) | `capacity_authorizations` |
| C8 | Newest evidence expired at the launch boundary | `valid_until` |
| C9 | The bound profile changed in `resources.yml` between selection and spawn | `resources.yml` |

An **unknown** field (probe unsupported or failed, `codex_probe: false`, field
absent) is not a refusal today. Quota gating (exhausted/reserve) is not funding
safety; it is removed with admission in S6b.

### Claude (adapter-local already: `claude::preflight`, `discover_account`, `validate_executable`)

| # | Refusal today |
|---|---|
| L1 | Subscription evidence expired or its contract fields not affirmed |
| L2 | Executable hash differs from the evidence |
| L3 | CLI version differs from the evidence |
| L4 | Unsafe effective local settings |
| L5 | Not logged in as a `claude.ai` first-party subscription |
| L6 | Extra usage (paid credits) enabled |
| L7 | Account identity (sha256 of email + org) differs from the evidence |
| L8 | Forwarded environment variables configured |
| L9 | **Sticky** as C7: a failed preflight during capacity observation becomes a `rejected` authorization for that revision |

Claude's preflight runs at selection and, through the capacity observation,
again at admission. After S6b it must run immediately before spawn.

### Replacement (S5a, additive)

- `src/harness/codex.rs`: the `account/read` probe and C1–C4 move into a Codex
  adapter preflight; setup records the Codex account identity on the profile
  (as Claude's evidence already does) and C5 compares against it (stricter
  than today's first-sample baseline).
- C8 is satisfied by running the preflight immediately before spawn; C9 by
  re-checking the bound profile against `resources.yml` at the same point.
- Proof suite: one test per row, each run against the new preflight alone,
  plus a differential test that fixture observations refuse under both paths.
- Decisions (2026-09-22):
  - **Stickiness is preserved (C7/L9).** Any funding refusal records a durable
    refusal for that profile and `authorization_revision`; the profile stays
    refused until setup re-authorizes it, as in 0.4.0.
  - **Unknown Codex identity refuses.** Stricter than 0.4.0: a Codex profile
    marked `no_overage_verified` never launches unless the adapter observes the
    authorized identity (a probe that is unsupported, fails or omits the
    identity refuses).
  - **C6 is dropped with pools** in S6b; C1–C5 still refuse.

### S5a as built

- `src/harness/codex.rs` owns the `app-server` account probe (moved from
  `capacity.rs`, which now calls it) and the refusal rule `codex::refusal`
  (C1–C5, missing evidence, unobservable identity). Only an executable named
  `codex` is ever started with `app-server`; any other configured program
  refuses rather than being run as a probe.
- Setup records `codex_account` (account digest) on Codex profiles, as it
  already recorded Claude's evidence. A Codex profile without it is not
  eligible ("run `dispatch setup codex` to revalidate").
- The Codex adapter preflight runs at selection and, through `run_harness`,
  immediately before spawn. A refused preflight is a typed `PreflightRefused`.
- Migration 22 adds `funding_refusals(funding_key, authorization_revision)`.
  A refusal at selection or at the spawn boundary is recorded; selection
  excludes a refused profile, and the native engine checks the record both
  before admission and again after admission, just before execution.
- After admission the native engine re-reads `resources.yml` and refuses when
  the bound profile changed (C9). The narrower window between the spawn-time
  preflight and the spawn itself is still held by admission's launch fence;
  S6a's launch observer takes it over before S6b.
- Proof suite: `tests/funding_safety.rs` (13 cases: authorized, C1–C5,
  unobservable identity, unreadable account, missing evidence, a change at
  the spawn boundary, C9, Codex and Claude stickiness), plus the pure-rule
  tests in `harness::codex`. Codex cases run with `codex_probe: false`, so the
  capacity path never probes and every refusal comes from the new path.
  Claude's L1–L8 are one function (`claude::preflight`) called by both paths;
  the unchanged phase6 `funding` scenarios prove them at the S6b gate.
- **Pre-existing 0.4.0 defect found:** after a single selection-time
  preflight rejection, the capacity path keeps refusing the profile even after
  re-authorization, because selection judges the new revision against the
  stored rejected observation and setup records no fresh one (Claude always;
  Codex with `codex_probe: false`). The new path clears on re-authorization;
  the defect disappears with the capacity path in S6b, which also adds the
  "launches after re-authorization" tests.
- Test fixtures: every fake Codex answers the account probe and every Codex
  profile carries evidence; `timed_out_optional_probe_keeps_a_normal_run_usable_and_explained`
  was deleted because the unknown-identity decision reverses exactly that
  behavior (its inverse is `codex_unreadable_account_is_refused`); two phase2
  message assertions accept the new path's refusal text.

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
- 2026-09-22 — S5a (additive; the capacity and admission path is untouched and
  still active): Codex adapter preflight with setup-recorded account evidence,
  durable sticky funding refusals (migration 22), refusal checks at selection,
  before admission and at the launch boundary, and the launch-boundary
  re-read of `resources.yml`. Proof suite `tests/funding_safety.rs` (13) plus
  `harness::codex` unit tests (11). Found and recorded a pre-existing 0.4.0
  defect (a rejected profile stays refused after re-authorization in the
  capacity path). `cargo test`: 512 passed, 0 failed.
- 2026-09-23 — S5b: one selection rule (`choose_profile`): the first configured
  profile, in file order, that is enabled, eligible, matches the backend and
  any explicit `--agent`/`--model`/`--effort`, and passes the availability,
  preflight and funding checks. Removed `router.rs`, `classifier.rs`, tier
  policy, the automatic stronger-model retry and `--no-retry`; the native
  engine's attempt loop is now a single attempt (a clarification answer
  re-enters it). Tests: removed the retry cases (phase3 ×8, the portfolio
  `recovery` scenario, the PTY `recovery` scenario, `classification.rs`);
  retry-incidental tests now use a successful single attempt; fixtures no
  longer assert classifier output or tier choice. The capacity checks at
  selection are unchanged until S6b. `cargo test`: 479 passed, 0 failed.
