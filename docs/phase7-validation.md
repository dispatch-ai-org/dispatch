# Phase 7 validation — private evidence and controlled trials

Date: 2026-09-18. Implementation and fixture validation only. Empirical effectiveness
requires prospective ordinary work. No real model calls, account changes, real human
feedback, global history imports, or real activation. The original implementation
was uncommitted; the separately authorized finishing pass below prepares one scoped
local commit. No tag, push, or publication.

## Checkpoint and chronology

Starting HEAD: `cfe546cdd6f6138f604aba2f5f44ced07d85674e`. Tracked and untracked
status was clean. Actual migration maximum was **18**; actual default allocation
policy was **allocation-portfolio-v3**. The baseline was archived before edits,
without a commit, stash, discard, or tag:

- `/private/tmp/dispatch-phase7-start/source.tar`
- SHA256 `0e1a7862d850b8ea13dcd2ead46682482f680636edbf68188745a85e716f9f64`
- Same directory: `HEAD`, `status.txt`, `tracked.patch`, extracted `baseline/`,
  command logs, and `measure-production-loc.py`.
- Baseline target: `/private/tmp/dispatch-phase7-baseline-target`.
- Current target: `/private/tmp/dispatch-phase7-current-target`.

The approved plan's Sections C/D/F/G/K/L, Phase 5 validation/control protocol,
repository instructions, and the updated Phase 6 report were read before editing.
The updated report supersedes its earlier fixture-only availability snapshot:
Claude Code 2.1.274 was installed and read-only discovery plus two separately
approved real smokes subsequently occurred. The first reported auxiliary Haiku
usage and failed fixed-model authorization **after one real invocation**; verification
remained `not_run`, review `not_requested`, source unapplied. The later diagnostic
check did not change that committed outcome. The second title-disabled Sonnet
invocation passed configured checks but remained pending review and unapplied.
They provide no human acceptance rate, causal improvement, or general support claim.
The Phase 7 synthetic regression preserves those outcome shapes and first-run
burden; it does not import or inspect the private smoke state. USER forwarding,
safe argv boundaries, title-disabled settings, fixed-model checks, account-wide
Claude exclusion, and time-bound funding assertions remain intact.

## Shared implementation and deliberate boundaries

- `private_evidence::select` is called once for new goals after
  `select_available_resource` builds the existing alternatives. Base/shadow use
  those same observations; no provider probe or invocation is added. A base choice
  retained for capacity deferral is not treated as an available trial alternative.
  The shared adapter/executor/admission/check/review/apply path remains authoritative.
- `summarize` uses a single SQLite read snapshot, indexed canonical project filtering,
  a 90-day window and 1,000-goal materialization cap. It reads projections and linked
  feedback/annotations; there is no new store, cache, artifact parser, or daemon.
- `evidence private`, `propose`, `policy`, `annotate`, `activate`, and `rollback` are
  advanced local-owner CLI surfaces. Normal `explain` is concise and separates
  historical decision metadata from current project evidence. Existing `evidence
  local` and public priors are unchanged.
- Feedback revisions remain the outcome authority. Append-only provenance annotations
  reference an exact existing feedback/delivery; they never manufacture feedback.
  Machine submission is independent of later review provenance. Explicit local owner
  attestation is not inferred from a TTY or the accept command.
- `private-quality-trial-v1` proposes one existing suitable standard/strong preference
  after 20 comparable reviews, 80% review coverage, and at least 5 and 25% classified
  quality rejections. One compatible verified alternative supports a **trial**, not
  a superiority claim. All parameters and limits are versioned, fixed, and inspectable.
- Activation uses a proposal content hash, owner attestation, evidence/configuration
  revalidation, and an immediate transaction with expected-revision fencing. Rollback
  restores the deterministic base for future goals. References becoming invalid
  make active proposals stale and future selection falls back. No automatic approval.
- New grant submissions pin/check the private-policy revision twice, including at
  the selection snapshot. Already bound running/queued goals and recovery retain
  their original semantics. Scoped results omit private evidence recursively.
- Claude terminal token categories are normalized once by the Claude adapter and
  stored in existing attempt detail. No per-model or incremental totals are added;
  nominal USD stays distinct from cash. Missing repair effort/cost/allowance remains
  unknown. Whole-chain time burden includes failures and pending outcomes.

The precise denominator, compatibility, economics, privacy, and owner-operation
contracts are in [private-evidence-policy.md](private-evidence-policy.md). There is
no cross-project pooling, cross-provider unit conversion, probability, quality score,
new integration, provider racing, resource creation, policy service, or Phase 8 work.

## Persistence

Migration **19** adds the facts absent from schema 18:

1. Append-only `private_annotations`, linked to existing goals/feedback revisions.
2. Content-addressed `private_proposals` and versioned `private_policy_transitions`.
3. Durable `admission_requests.launch_knowledge`, copied from existing lease lifecycle
   transitions before lease deletion; historical unknowns stay unknown.
4. Source/window, annotation, and attempt-admission query indexes.
5. An immutable marker for **empty disposable synthetic fixture state**, with no CLI
   promotion of an existing state into fixture mode. Synthetic evidence is ineligible
   in ordinary state. No production history is seeded.

Decision metadata resides in the existing allocation/run projection; a trigger
rejects later mutation of that snapshot. Annotations are append-only. There is no
materialized outcome cache to invalidate; current summaries rebuild from authority.
Default empty evidence configuration is omitted from serialization to preserve old
project-config grant digests. Historical `None` metadata remains readable and
ineligible for proposal screening. Existing sync envelopes/outboxes and raw facts
are retained. SQLite transitions use existing durability settings; no hardware
power-loss test or disk-corruption recovery claim is made.

## Regression matrix

Names below are exact Rust tests in `tests/phase7_evidence.rs`,
`private_evidence::tests`, `db::tests`, or named existing suites. All new observations
are synthetic and disposable; tests do not certify empirical policy effectiveness.

| Row | Tests and principal assertions | Result |
|---|---|---|
| A | `private_feedback_revision_stales_proposal`, `one_sqlite_snapshot_keeps_feedback_revision_coherent`: latest exact revision once, repeated events do not multiply counts, old snapshot retains its facts. Existing Phase 5 durable receipt tests preserve replay semantics. | PASS |
| B | `private_outcomes_and_smoke_attribution`: pending/deferred lack votes; accepted-unverified and accepted/apply-blocked are separate; target verification failure remains visible without review. | PASS |
| C | `private_explicit_provenance_not_interface_inference`, `fixture_domain_cannot_be_enabled_after_history_or_removed`: human-attested ordinary machine-submitted work can count; machine/scripted/synthetic/unknown provenance does not qualify in ordinary state; accept/reject or terminal alone confers no attestation. | PASS |
| D | `private_outcomes_and_smoke_attribution`, `terminal_cache_categories_do_not_add_breakdowns_or_estimate_cash`; existing `claude_only_cli_identity_usage_review` and Claude protocol tests: failed authorization plus verified-unreviewed output, retained first invocation burden, no fabricated Sonnet acceptance or cash charge. | PASS |
| E | `private_outcomes_and_smoke_attribution` plus full `phase3_recovery` and `portfolio_cross_harness_recovery_both_directions`: chains excluded from standalone credit, all attempts retained, existing clarification lineage preserved. | PASS |
| F | `private_outcomes_and_smoke_attribution`, full `phase0_outcomes`, `phase2_capacity_admission`, `phase3_recovery`, and `phase6_portfolio`: known normalized failure kinds retained, mechanical failures independent of review, no quality labels inferred from failure/prose. | PASS |
| G | `private_behavior_and_check_compatibility`, `constraints_unknown_identity_and_behavior_changes_are_conservative`: model/effort/settings/version/check differences separate cohorts; proof/display renewal preserves behavior; observed unknown stays unknown. | PASS |
| H | `one_sqlite_snapshot_keeps_feedback_revision_coherent`, `private_feedback_revision_stales_proposal`, `private_policy_transition_is_atomic_and_revision_fenced`: writer correction during an open reader snapshot is coherent; subsequent feedback and attempted snapshot rewrites cannot change the historical decision. | PASS |
| I | `sparse_missing_conflicting_and_unmapped_cohorts_abstain`, `private_feedback_revision_stales_proposal`; existing `local_inspection_excludes_public_priors_and_explicit_runs_and_cannot_change_router`: reviews/resources/versions/public priors are not pooled dishonestly. | PASS |
| J | `screening_parameters_are_not_calibration_or_activation`, `sparse_missing_conflicting_and_unmapped_cohorts_abstain`, `private_bounded_large_history`: empty/sparse/conflicting/unmapped/truncated cohorts abstain; screening never changes active policy. | PASS |
| K | `private_fixture_shadow_activate_rollback`: exact on/off argv, same selected resource/outcomes, one invocation each, same budget/no-retry/admission-release semantics, source unchanged; hypothetical route receives no feedback. Existing provider/admission suites check underlying traffic and permissions. | PASS |
| L | `private_fixture_shadow_activate_rollback`, `default_config_preserves_historical_grant_digest_and_malformed_proposals_fail`, `private_policy_transition_is_atomic_and_revision_fenced`, `private_promoted_preference_preserves_constraints_and_scope`: synthetic proposal inspectable, malformed/version/parameter/scope/stale authority refused; forged machine promotion unsupported. | PASS |
| M | `private_activation_preserves_inflight_and_queued_bindings`, `private_fixture_shadow_activate_rollback`: running/queued choices and budgets pinned, only intended future class changes, old grant cannot expand, rollback restores base. Existing Phase 5 receipts/recovery remain covered. | PASS |
| N | `private_promoted_preference_preserves_constraints_and_scope`, `constraints_unknown_identity_and_behavior_changes_are_conservative`: explicit selection, disabled/changed profiles, narrow grant, funding refusal and exhausted capacity cannot be bypassed; missing target safely falls back. | PASS |
| O | `private_stale_active_evidence_falls_back_after_restart`, `private_feedback_revision_stales_proposal`, `private_policy_transition_is_atomic_and_revision_fenced`: correction stales evidence, new process falls back, stale/revoked IDs cannot reactivate, equal/stale projections cannot rewrite decision history. | PASS |
| P | `terminal_cache_categories_do_not_add_breakdowns_or_estimate_cash`, `private_outcomes_and_smoke_attribution`; existing Claude terminal-total and capacity tests: exact categories, no duplicate breakdown sums, missing categories/cash unknown, zero-accepted or unknown-launch burden undefined, failures retained, provider units separate. | PASS |
| Q | `private_promoted_preference_preserves_constraints_and_scope`, `private_explicit_provenance_not_interface_inference`, full `phase5_control`, `phase4_cli`, `phase4_review`, `product_ux`: no other-goal private aggregates/IDs through scoped results, no machine human feedback, existing UI/control behavior retained. | PASS |
| R | `phase_seven_migration_preserves_feedback_and_unknown_history`, `private_policy_transition_is_atomic_and_revision_fenced`, `fixture_domain_cannot_be_enabled_after_history_or_removed`, `private_optional_analytics_failure_never_waives_authority`, existing migration/FK/sync suites: optional query failure falls back with null counts while authoritative corruption refuses launch; 18→19 preserves revisions/raw rows, default inactive state, rollback on pre-commit fault, restart durability and sync boundaries. | PASS |
| S | Full locked four-thread suite, release build and Phase 5/6/7 release fixtures; `private_bounded_large_history`: 1,052 synthetic stored goals, 1,000-row cap, truncation abstention, measured inspection time below. | PASS |

## Original implementation commands and actual outcomes

| Command | Actual result |
|---|---|
| Preserved baseline: `RUST_TEST_THREADS=4 cargo test --locked --manifest-path /private/tmp/dispatch-phase7-start/baseline/Cargo.toml` with its separate target | PASS: 386 passed, 0 failed, 1 existing ignored |
| `cargo fmt --check` | PASS, exit 0 |
| `cargo test --locked --target-dir /private/tmp/dispatch-phase7-current-target -- --test-threads=4` | PASS: **405 passed, 0 failed, 1 existing ignored**; zero doc tests |
| `cargo clippy --locked --all-targets --target-dir /private/tmp/dispatch-phase7-current-target -- -D warnings` | PASS, exit 0, no warnings |
| `git diff --check` | PASS, exit 0 |
| `cargo build --release --locked --target-dir /private/tmp/dispatch-phase7-current-target` | PASS, optimized build, 20.56 seconds for the final incremental rebuild |
| Focused `phase2_capacity_admission`, `phase5_control`, `phase6_portfolio`, `phase7_evidence`, `--test-threads=4` | PASS: **80 passed**, 0 failed (6 + 30 + 33 + 11) |
| Release fixtures | PASS: **13 scenarios**, plus terminal-category adapter/core/SQLite propagation assertion |

The full suite comprises the existing 386 tests plus seven private-evidence unit
tests, one dedicated schema-18 migration regression, and eleven Phase 7 integration
tests. The pre-existing ignored terminal-panic fixture remains exercised by its
PTY path. Source was not built with the baseline target directory.

Complete final logs are under `/private/tmp/dispatch-phase7-start/`:
`final-full-tests-complete.log`, `final-clippy-complete.log`,
`final-release-complete.log`, `final-focused-safety-control-portfolio.log`, and
`final-release-smokes.log`. The earlier full-run logs retain their actual outcomes;
the original implementation results remain historical. The finishing-pass results
below validate the subsequently committed source.

Original implementation production physical Rust LOC, using the preserved Phase 6 measurement script:
**23,822 → 25,336, net +1,514**. This includes comments, blank lines and embedded SQL
in `src/**/*.rs`, including the new untracked module, and excludes complete
`#[cfg(test)]` items/modules. Integration/Python fixtures, docs and the unchanged
134-line reference client are excluded. The result/script are retained in the
checkpoint directory. No runtime dependency, second execution path, store, or
policy framework was added. The implementation deliberately uses one local
screening rule and derives current summaries rather than maintaining a cache.

Plan choices/limits: provenance attestation is optional and separate from ordinary
review; old feedback is not auto-certified. The trial alternative needs one
compatible verified execution but no claim of human superiority. Cross-project
pooling is not implemented. Complete-chain burden is reported over the declared
project/origin window; it is not a causal comparison across policy versions.
Rollback returns to the fixed deterministic base, not to another previous trial.
These limits keep the approved private-evidence scope small.

The initial baseline sandbox run had seven loopback denials; the permitted baseline
passed **386 tests**, zero failed, one pre-existing ignored panic fixture, at four-test
concurrency. No new ignored tests or timeout increases were introduced. Intermediate
failures were retained and investigated: fixture language was unknown, activation had
a noncanonical path, terminal access was sandbox-denied, exact-cutoff timestamps had
inconsistent formatting, and a fixture incorrectly equated `--no-retry` with changing
the stored maximum-invocation field. These were not called pre-existing timing flakes.
Intermediate compile failures from added summary fields were also corrected. Final accounting review added explicit resource invocation/quality counters and kept burden undefined for uncertain launches; those dedicated regressions passed before final validation. Only
completed successful commands count as passes. No unrestricted-concurrency stress
claim is made.

## Repeatable release smoke

All scripts use Python's standard library and disposable state. They install only
synthetic local harness executables and never use provider credentials. The Phase 7
fixture marks its empty state synthetic before creating goals; synthetic feedback
can screen only in that disposable domain. Its scripted owner activation is not a
human outcome label. It seeds 20 reviewed light-profile goals, five missing reviews,
and one verified/unreviewed alternative, then proves observe/shadow → explicit
activation → future standard selection → rollback → restored light selection.
Exactly six actual fixture invocations are allowed in this smoke; cloned evidence
rows are explicitly synthetic and not claimed as real work.

The completed release scenarios were Phase 6 Claude `direct`, `selection`,
`recovery`, `pools`, and `capacity`; Phase 5 `smoke`, `clarify`, and `idempotency`
for each of Codex and Claude; and Phase 7 `smoke` and `bounds`. An additional
single synthetic Claude invocation verified exact terminal input/output/cache-read/
cache-creation categories in both the returned result and committed attempt,
with cash cost remaining unknown.

The larger-history release fixture stored **1,052 explicitly synthetic goals**.
Inspection materialized at most 1,000 and took **0.186 seconds**; proposal evaluation
took **0.150 seconds** and abstained because the window was truncated. Both timings
include CLI/SQLite work on this machine. The invocation count remained two template
fixture invocations; neither query invoked a provider. These are bounded-history
measurements, not a universal latency guarantee.

```sh
cargo build --release --locked --target-dir /private/tmp/dispatch-phase7-current-target
python3 tests/fixtures/phase7_evidence.py /private/tmp/dispatch-phase7-current-target/release/dispatch smoke
python3 tests/fixtures/phase7_evidence.py /private/tmp/dispatch-phase7-current-target/release/dispatch bounds
DISPATCH_FIXTURE_PROVIDER=claude python3 tests/fixtures/phase6_portfolio.py /private/tmp/dispatch-phase7-current-target/release/dispatch direct
DISPATCH_FIXTURE_PROVIDER=codex python3 tests/fixtures/phase5_control.py /private/tmp/dispatch-phase7-current-target/release/dispatch smoke
DISPATCH_FIXTURE_PROVIDER=claude python3 tests/fixtures/phase5_control.py /private/tmp/dispatch-phase7-current-target/release/dispatch smoke
```

The scripts' PTY/loopback/process tests need the same local permissions as prior
phases. A denial is not a passing result. Temp logs/checkpoint need owner retention
beyond the OS temporary-directory lifetime.


## Finishing pass — optional review attestation and bounded fixture cleanup

Separately authorized on 2026-09-18. HEAD remained
`cfe546cdd6f6138f604aba2f5f44ced07d85674e`; the worktree contained only the intended
Phase 7 implementation/docs/tests. Before editing, its complete tracked/untracked
source was archived at `/private/tmp/dispatch-phase7-finish/source.tar`, SHA256
`7d616283fb59e45dc29a13bf0bf62d03247319ed7982a413b53a718b6fbe53ad`.
The same directory retains HEAD, status, tracked diff, extracted baseline, and logs.
The preserved finishing baseline actually passed **405 tests**, zero failed,
one existing ignored, at four-test concurrency, with target directory
`/private/tmp/dispatch-phase7-finish-baseline-target`. Current builds use
`/private/tmp/dispatch-phase7-current-target`.

Normal acceptance/application/rejection returns directly to the next-goal composer.
Its contextual `f` action is optional, with no ID copying or preselected consent.
The confirmation shows the displayed task/review/delivery and the explicit ordinary-
work/personal-review statement. `c` confirms; `b` or Ctrl+C cancels; Ctrl+D exits.
The same core preparation and commit validation serve the annotation CLI and presenter.
The exact run, delivery, state revision, feedback revision, and provenance revision
are pinned; commit rechecks owner/machine authority and those facts in a short
immediate SQLite transaction. Identical confirmation is idempotent. Annotation errors
never undo review/apply, invoke a model, or activate a policy. After refusal, the
presenter refreshes only that exact run; another `f` action displays its current
facts and requires a new explicit confirmation.

The `owner()` fixture now delegates to a bounded standard-library PTY helper. Its
child acknowledges that it owns a new session/process group before group signalling.
Timeout or premature terminal closure sends TERM, waits 0.2 seconds, then KILL;
reaping polls with WNOHANG for at most two seconds. The child is not reaped before
the last cleanup signal, preventing PID reuse during cleanup. There is no blocking
waitpid fallback. Normal/nonzero statuses remain distinct from errors; capture is
bounded to 64 KiB with a 4 KiB diagnostic tail. Descriptors close in `finally`, and
unestablished cleanup fails explicitly. Scripted tests are mechanics, not real
human judgments.

| Exact regression | Assertions / scenarios |
|---|---|
| `private_review_opt_in_cancel_duplicate_and_plain` | Eight disposable PTY scenarios: accept/reject without annotation; accepted/rejected opt-in; Ctrl+C with resize; back; duplicate presenter plus CLI confirmation; plain/no-color. Exact delivery and feedback IDs, one eligible vote, no reapply/model call, terminal restoration. |
| `private_review_revision_delivery_provenance_and_write_failure` | Seven scenarios: changed feedback revision followed by freshly confirmed retry; changed delivery; concurrent scripted provenance; write fault then retry; preexisting synthetic and scripted-smoke refusal; pending-work refusal. Existing review/apply and labels preserved. |
| `private_review_fixed_session_and_apply_blocked` | Another foreground result cannot retarget the action; source-drift-blocked acceptance remains blocked. No active-policy transition or lease; fixture invocation bounds asserted. |
| `private_owner_helper_bounds_timeout_eof_and_normal_cleanup` | Four Python regressions: unexpected prompt/TERM-resistant hung child; premature PTY closure with live child; normal attestation completion; nonzero exit. All verify finite completion, reaped child, closed descriptor, and exclusively nonblocking waitpid calls. |

All new ordinary/human semantics are simulated only in newly created disposable
test state. No historical private feedback was read/imported. The finishing pass adds
**no migration or dependency**; the intended full Phase 7 commit retains schema 19.
Production LOC by the same method is **23,822 → 25,468 (+1,646)** for all Phase 7;
the finishing pass alone adds **132** over its 25,336-line checkpoint.

Initial follow-up test failures were fixture waits for an already-consumed prompt,
not timing flakes or production review failures. Those waits were corrected without
timeout inflation. The confirmation summary was shortened to keep controls visible
on a 65-column resize. No new ignored tests or permission weakening was introduced.

Final sequential validation on the frozen source passed:

| Command / check | Actual result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo test --locked --target-dir /private/tmp/dispatch-phase7-current-target -- --test-threads=4` | **409 passed, 0 failed, 1 pre-existing ignored**; includes Phase 4 review/PTY, Phase 5 authority, and all 15 Phase 7 integration tests |
| `cargo clippy --locked --all-targets --target-dir /private/tmp/dispatch-phase7-current-target -- -D warnings` | PASS, no warnings |
| `git diff --check` | PASS |
| `cargo build --release --locked --target-dir /private/tmp/dispatch-phase7-current-target` | PASS; final frozen-source incremental check 0.33 seconds, following the 22.23-second rebuild |
| Release Phase 7 `smoke`, `bounds` | PASS; smoke exactly six synthetic invocations; 1,052-goal bounds inspection 0.217s, proposal 0.155s, no query-triggered invocation |
| Release review scenarios `attest`, `plain`, `blocked`, `revision`, `failure` | PASS, including explicit reconfirmation after stale feedback and retry after a write fault |
| Standalone helper regressions | **4 passed**, 1.859 seconds; finite cleanup and normal/nonzero exit assertions |

The actual release executable was
`/private/tmp/dispatch-phase7-current-target/release/dispatch`, SHA256
`f45c306d3bed66304fef83a20cb6f833c64cc55aae7540f51708dd62a59f5590`.
These runs used only disposable synthetic executables/state. The final source and
fixture hashes were checked against the frozen manifest after validation; no older
installed CLI or baseline executable was substituted.

Logs in `/private/tmp/dispatch-phase7-finish/`: `baseline-tests.log`,
`focused-review-corrected.log`, `focused-review-refresh.log`,
`frozen-full-tests.log`, `frozen-clippy.log`, `frozen-release-build.log`, and
`frozen-release-smokes.log`. The earlier broader run overlapped the final same-run
refresh edit, so the subsequent sequential frozen run is the commit validation
claim. No unrestricted-concurrency stress claim is made.

Additional repeatable release checks:

```sh
python3 tests/fixtures/phase7_review.py /private/tmp/dispatch-phase7-current-target/release/dispatch attest plain blocked revision failure
python3 tests/fixtures/phase7_owner_test.py
```

The local commit scope is the 21 intended Phase 7 source, fixture and documentation
files. Checkpoints, logs, binaries and captures stay outside the repository. Private
state, credentials, account evidence, grants and unrelated files are not included.
The staged diff and a credential-pattern/artifact scan are checked before committing.
No push, tag, publication, version change, or Phase 8 work is authorized or performed.

## Remaining empirical gates and limitations

Real local evidence counts were **not inspected**. The two Phase 6 smoke observations
above are report chronology, not private-query results or human labels. No ordinary
state, active real policy, profiles, grants, or account settings were changed.

Use the prospective protocol in [private-evidence-policy.md](private-evidence-policy.md):
ordinary tasks, observe/shadow first, actual human review, optional repair reports,
frozen separately approved future trial windows, complete missingness/failure/burden
reporting, and owner-controlled rollback. Proposed alternatives have no counterfactual
outcome. More accepted verified work with acceptable rework/latency is still an
empirical gate. No savings or release-readiness claim follows from core completion.

Non-blocking limits: owner attestation relies on the local OS owner; historical
origin/behavior/category gaps cannot be repaired by inference; no cross-project
pooling; 1,000-goal truncation deliberately abstains; descriptive distributions are
count/total/min/max, not forecasts; allowance attribution and cash cost remain unknown
when durable facts do not establish them. Complete provider-normalized historical
failure subtypes cannot be reconstructed from unclassified errors. The implementation
uses one fixed rule and one active preference per project, with base-policy rollback,
not a general policy framework.

CORE: COMPLETE — READY FOR PRIVATE-EVIDENCE/SHADOW DOGFOOD

LIVE POLICY: UNCHANGED — no real proposal activated

EFFECTIVENESS: PENDING — prospective accepted-work/rework evidence required
