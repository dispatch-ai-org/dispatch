# Phase 8 implementation and validation — September 18, 2026

Phase 8 is an opt-in bounded planning experiment in the existing Rust execution
core. Direct execution remains the default; private-policy activation and improved
economics are not claimed. The functional finish line is one plan → sequential
suitable tasks → checked integration → root verification → one explicit review.
See [actual usage](planning.md) and [design/release handoff](phase8-design-handoff.md).

## A. Reproducible baseline and maintenance boundary

The inspected HEAD is `db966a914463a23163f118f85e253aec2afbc3e7`, schema 19.
There were 11 dirty maintenance files, 657 added / 20 removed tracked lines, before
Phase 8. No commit, stash, reset, installation or publication was performed.

The exact tracked/untracked source checkpoint is retained outside the repository:

- `/private/tmp/dispatch-phase8/baseline-source.tar.gz` (109 files), SHA-256
  `8c023ceace8e725ea3b50e4a8587d34d9e9fb6ea9673b5961a7570fe99c7f565`.
- `baseline-manifest.json`, `baseline-status.txt`, `baseline-head.txt` and
  `maintenance.patch` in the same directory identify original contents and status.
- Extracted baseline: `/private/tmp/dispatch-phase8/baseline-source`.
- Separate targets: `/private/tmp/dispatch-phase8-baseline-target` and
  `/private/tmp/dispatch-phase8-current-target`.

The existing maintenance covers C classification, owner task/check mapping,
raylib-derived fixture evidence and duplicate-review handling. It is not Phase 8,
and the C/raylib project is not Dispatch's Rust source. Intended commit boundaries,
only after separate authorization: (1) the exact archived maintenance; (2) the
Phase 8 delta against that archive, including its focused completion-event fix,
tests, documentation and curated captures. Shared files require hunk separation.

Baseline validation at four-test concurrency: **419 passed, 1 existing ignored**
(`baseline-confirm.log`). The first restricted attempt had seven loopback permission
failures and is not counted as passing. A permitted attempt exposed the existing
`phase4_follow_registration_racing_completion_cannot_lose_it` failure; the complete
baseline repeat passed. Investigation found the final projection and its event
could commit separately. Phase 8 reuses the existing combined row/event transaction
for final completion/stopping, preventing observers from seeing a terminal outcome
without its semantic completion event. No timeout was increased to hide that race.

Baseline tested executable:
`/private/tmp/dispatch-phase8-baseline-target/debug/dispatch`, SHA-256
`d8041f585234d215965faf93a92cd81bcae1c7149748209787c013c85c288fb1`.

## B–F. Implementation and authority

The root remains the existing allocation Run and GoalExecution. `planning` is an
optional field, absent for historical/direct runs. Tasks use ordinary attempts,
admission, resource/funding filters, final launch fencing, process cleanup,
check execution, questions and review/apply. There is no child top-level goal,
second spawn path, provider SDK, new backend, daemon or concurrent-task executor.

Migration **20** adds `planned_goals`, `planned_tasks`, `planned_snapshots`,
`planned_artifacts`, `planned_invocations`, and a trigger retaining planned launch
knowledge as an existing lease advances. Policy and plan are immutable; manifests
and lineage rows cannot be rewritten through the authoritative transition path.
No sync schema or upload scope changes.

`--plan` and composer `/plan <goal>` explicitly opt in. Scoped stdio additionally
requires `control-grant --allow-plan`; absent/false mode is direct and old grants
remain direct-only. An explicit maximum of 2–6 in planned mode is further capped
by `1 + task count + 1`; direct execution keeps its maximum two. The one persisted
deadline begins before planned preparation and covers admission, work, verification,
integration and questions. Every call has its own authorization and one-shot fence.
The one shared extra cannot consume room reserved for unstarted required children.
`--no-retry` prevents automatic recovery/assistance, not initial tasks or an
authorized answer/dependency continuation. Prepared, positively unlaunched and
uncertain-spawn facts remain distinguishable; no implicit replay/refund exists.

The planner runs isolated with supported read restrictions. Strict successful
final-result parsing accepts one versioned, bounded plan, never an earlier JSON
fragment from a failed/malformed final. All task scopes, checks, DAG edges and
resource feasibility are validated before a child runs. Owner routine/check
mapping can authorize light for at most two existing files; unknown bounded work
requires standard and interfaces/directories/broad work strong. Planner prose is
not owner attestation. Fixed model/effort/harness constraints remain binding.

Stable runnable-task ordering is sequential. Required artifacts are checked and
fingerprinted; current incidental context does not invent a dependency. Each patch
is relative to its own immutable input and must reproduce its checked output when
integrated. Child repair uses that input; dependency continuation records the newer
input and explicitly reconciles its checkpoint. B0-to-final diff is a separate
candidate with exact contributing attempts. Root checks run after all tasks; one
eligible root repair may spend the shared extra. Original verification authority
is restored in check workspaces and proposed changes to it remain visible.

Questions use existing IDs/revisions/generations and scoped factual authority.
Waiting holds no lease; answer/dependency reports cannot add tasks, reset deadlines,
approve spending or create reviews. Failures preserve evidence, stop unrelated
work and expose no normal final candidate. Hash/manifest mismatch blocks delivery
inspection/apply. Owner loss is interrupted with uncertain cleanup retained;
planned crash recovery is explicitly refused, not advertised as resumable.

The presenter adds a compact sequential task list, dependency requirements,
provenance Details, final review and deadline-aware questions. Control v1 gains
additive capability/mode/planning fields and a scoped final-diff reference. Existing
machine receipts, framing, async waits and no-review authority remain. Planned
chains are excluded from standalone Phase 7 quality cohorts; one root feedback
revision cannot create child reviews. All attempt burden and planned check events
remain observable, with separate provider units and unknown costs left unknown.
No private history was imported, no real resource account was inspected/changed,
and no policy or screening gate was activated.

## G. A–V regression evidence matrix

New scenario names below are executable cases in
[`tests/fixtures/phase8_planning.py`](../tests/fixtures/phase8_planning.py), wrapped by
[`tests/phase8_planning.rs`](../tests/phase8_planning.rs). They create fresh source,
state, fake executable protocols and synthetic account proofs. Actual process
markers, exclusive-active files, durable invocation slots, leases, checks and
committed projections are asserted. They are not assertions based on test names.
Existing suites verify the reused primitive under the stated boundary; those are
identified separately rather than presented as new full planned scenarios.

| Area | Deterministic evidence and assertion |
|---|---|
| A Direct | `direct`, `planning_false`: no planning projection, one actual call, absent/false submit canonicalize to the same receipt. Full existing CLI/TUI/control/allocation suites retain direct limits. |
| B Authority | `grant` refuses an old/narrow grant with zero calls; `no_planner`, `no_checks`, `explicit_override` reject before spending. `pinned` keeps one explicit strong model across all roles. |
| C Plan validation | `success`, `malformed`, `truncated`, `nonfinal`, `failed`, `planner_edit`, `oversized`, `duplicate`, `cycle`, `missing_check`, `traversal`, `symlink`, `conflict`: invalid acquired plans spend only the single planner and start no child. |
| D Decomposition | `success`, `control`: actual strong planner, light A, standard B; B asserts A's changed C source is present; real compiler/test runs; exactly one checked root delivery, pending review before scripted actions. |
| E Allocation | `success` uses explicit owner scope/check mapping for light; the other task stays standard despite a similarly small proposal. `pinned`, `explicit_override`, `mixed`; existing Phase 2/6 inclusion, no-overage, account/capacity and explicit-constraint tests remain binding. |
| F Budget | `four` uses planner + four initials; `six` adds exactly one repair; `tight` rejects infeasible plan after one call; `budget` fits without extra; `tight_repair` preserves required initial room and stops; `second_repair`, `no_retry`, `deadline` assert counts and bounded stop. |
| G Readiness | `success`, `dependency`, `already_ready` require the exact checked/integrated producer artifact and record consumed IDs; `no_retry`/`second_repair` demonstrate failed checks cannot release downstream tasks. |
| H Reports | `dependency`, `assistance`, `already_ready` succeed within the shared slot; `self_report`, `cross_report`, `bad_dependency`, `cycle_report`, `stale_report` reject self, wrong revision, unknown task, cycle and wrong generation. No task creation or second planner. |
| I Failed/changed prerequisites | `no_retry`, `second_repair`, `cancel` stop dependents; `manifest_tamper`, `tamper` refuse mutated lineage/delivery; `fault_integration` cannot publish readiness. Immutable artifact rows and completed-attempt regression tests retain historical revisions. |
| J Integration | `success`, `fidelity`, `dependency`, `root_repair`: cumulative trees, exact inputs/contributors and B0 diff; binary bytes, executable mode and rename/deletion survive apply. `out_of_scope`, `fault_manifest`, `fault_integration`, `manifest_tamper` refuse unsafe or unpublished work. Existing source snapshot/apply suite covers symlink/source kinds; planned symlink scopes are conservatively rejected. |
| K Verification | `root_fail` proves passing children are insufficient; `root_repair` checks the published repair again; `weaken` cannot replace the original assertion. Existing `any_infrastructure_or_unknown_check_blocks_other_target_failures` and verification infrastructure tests exercise the shared classifier, not a different planned classifier. |
| L Shared extra | `repair`, `six`, `second_repair`, `tight_repair`, `dependency`, `assistance`, `question`, `question_no_retry` count the one shared repair/continuation; no per-child budget or recursive expansion. |
| M Questions | `question`, `claude_question`: exact authorized, duplicate-idempotent answer with outstanding await and zero leases while waiting; `question_deadline`, `pty_question_deadline` expire without a continuation; `question_no_retry` retains the original deadline. Existing Phase 3/5 stale/category/funding/machine-review denial and answer-commit fault tests exercise the reused handler. |
| N Providers | `success` (Codex), `claude_success`, `claude_question`, `mixed`: actual fake processes over the two adapter protocols, differing roles/configurations, separate account pools and retained per-attempt identity. Phase 6 suite covers account-wide exclusion, title/auxiliary controls, no-overage proofs, changed funding and observed unknowns. |
| O Cancellation/crash | FIFO barriers hold an actual planner/child in `cancel`, `disconnect`, `crash`, `deadline`; crash reload cannot replay and planned recovery refuses. SQL faults in `fault_plan`, `fault_fence`, `fault_completion`, `fault_integration`, `fault_delivery` cover pre-commit/fence/completion/integration/final publication. `fault_manifest` fails disk publication; `uncertain_cleanup` retains the lease/reservation. Existing Phase 2/3/5 tests cover queued acquisition, handoff/spawn uncertainty and answer-commit boundaries in the shared core. |
| P Control | `control` retries submit in-session and after reconnection with the same root/count; `question` duplicates answers while awaiting; `cancel` resolves an outstanding await; `disconnect` cancels. Existing real-pipe Phase 5 slow-reader, broken-pipe, lost-reply, scope and receipt-fault tests remain green. |
| Q Review | `control` scopes final-diff retrieval; `success` verifies source unchanged before explicit fixture acceptance; `drift`, `tamper`, `manifest_tamper` refuse unsafe apply. `private_attribution` asserts exactly one root feedback, zero legacy routing-feedback exports and zero sync outbox. Scripted approval is not a human review. |
| R Private evidence | `private_attribution`: one policy chain, zero standalone eligible executions/reviews, all three launches and six completed check runs counted, no initial child mislabeled continuation, zero policy transitions. Full Phase 7 suite preserves historical metadata, thresholds, explicit provenance and in-flight policy rules. |
| S Migration/reload | `schema19_projection_and_direct_grant_survive_planning_migration` builds an actual pre-20 DB and compares retained projection bytes, old grant authority and empty planned tables. `control` reloads committed identities; fault cases cannot invent ready artifacts; old terminal/attempt immutability tests still pass. |
| T Presentation | `pty_plain`, `pty_narrow` (38×32), `pty_wide` (100×32), `pty_question_deadline`: ASCII/no-color, sequential state, Details/diff navigation, leave pending, exact terminal restoration, no duplicate plain final-review message. Existing Phase 4 tests retain native pager/editor and other terminal behaviors. Curated byte renderings below are not native GUI screenshots. |
| U End-to-end | Release `control` uses OS pipes; release PTYs use actual terminal control; `raylib_copy` copies the real plain C directory and runs its original build, 13 collision and both-size layout checks. Live tree fingerprint unchanged; root review pending. |
| V Disable | `direct`, `planning_false`, `grant`, `control`, `pinned` and migration tests keep omitted/false mode direct, legacy grants narrow and retained history pinned. Existing policy-change/cancel tests forbid permission enlargement. There is no global switch that rewrites an in-flight goal or grants a fresh budget. |

## H. Final validation, identities and size

Final edited implementation, Rust/Cargo 1.97.1 on Darwin arm64:

| Command | Result |
|---|---|
| `cargo fmt --check` | Pass |
| `cargo test --locked --target-dir /private/tmp/dispatch-phase8-current-target -- --test-threads=4` | **488 passed, 0 failed, 1 existing ignored**, 24 suite results including empty main/doc suites |
| `cargo clippy --locked --all-targets --target-dir /private/tmp/dispatch-phase8-current-target -- -D warnings` | Pass |
| `git diff --check` | Pass |
| `cargo build --release --locked --target-dir /private/tmp/dispatch-phase8-current-target` | Pass, optimized build |

This includes 67 Phase 8 scenarios plus migration and narrow-view regressions; baseline
had 419 passing tests. The ignored case is the existing subprocess-only
`terminal_panic_fixture`, exercised by its parent PTY test. Phase 2/3/4/5/6/7 suites
all ran in this full pass (6/38/6/30/33/16 tests respectively). Total reported suite
runtime was 376.63 seconds; this is bounded four-test concurrency, not unrestricted
stress. Exact logs remain outside the repo in `/private/tmp/dispatch-phase8`:
`final-fmt.log`, `final-full.log`, `final-clippy.log`, `final-release-build.log` and
`release-smokes.log`.

The exact executable used by the final subprocess tests is
`/private/tmp/dispatch-phase8-current-target/debug/dispatch`, SHA-256
`fdcf1b2fed4a8f196480c837c5050b808b75947b7114fc9f92981b459a16de34`.
The final smoke executable is
`/private/tmp/dispatch-phase8-current-target/release/dispatch`, SHA-256
`335375b7dc1073076ac69d043d8418235222bab7e9a914c7890350ea5c8d4995`.
`binary-identities.json` retains both sizes and hashes, including the baseline.
No binary was installed over an existing executable.

Final release command (all nine scenarios must return successfully):

```sh
DISPATCH_REVIEW_CAPTURES=/private/tmp/dispatch-phase8/captures \
DISPATCH_RAYLIB_SOURCE=/Users/jese/bin/raylib_bouncing_ball \
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/phase8_planning.py \
  /private/tmp/dispatch-phase8-current-target/release/dispatch \
  success second_repair control mixed pty_plain pty_narrow pty_wide \
  pty_question_deadline raylib_copy
```

**Pass, exit 0 for the complete nine-scenario command.** The successful fixture,
control and mixed-provider cases each launch three supervised invocations. The
second-repair case stops after four with no complete final candidate. All four
PTY sessions restore terminal state; their review cases leave the goal pending.
The copied-raylib case launches three, passes original checks and leaves one
pending root review. Six final release milestones were rendered and inspected;
the 38-column review visibly retains verification above the task list.

The copied raylib project's original `verify.sh` compiles/links the app with
warnings as errors, passes all **13 headless collision cases**, and passes layout
checks at application and alternate dimensions (all four horizontal gaps). Its
original tree digest before/after is
`5853da7b72dd31f377931d95b8b6cdd0411332d127dc1ef084142b1b2d978ea7`.
The synthetic change is a horizontal radius adjustment plus dependent documentation;
it is a mechanics/build smoke, not a visual-quality or economic result. One final
candidate remains pending; the live source is unchanged.

[Six curated recorded-byte previews](captures/phase8/README.md) include wide,
narrow, plain, question and deadline states. Input hashes, milestone dimensions,
timing and binary hash are retained with the previews; temporary raw transcripts,
private fixture state and grants stay outside the repository.

Intermediate failures are retained in the local evidence folder, not counted as
passes: the schema-19 fixture initially omitted a historically required field;
mixed fake accounts initially shared an invalid pool identity; the attribution
fixture initially targeted legacy rather than root feedback; PTY assertions initially
looked for complete redraw text in differential ANSI output. Corrected fixtures
assert actual supported state/identity and stable prompts. The early control fixture
mistook a semantic wait timeout for completion; it now follows the original
persisted deadline instead of increasing the deadline. The implementation fixes
from these checks include atomic completion events and automatic, visible expiry
of planned questions. Recorded narrow-terminal rendering additionally exposed a
hidden verification summary; it now precedes the task list, with a focused
regression and regenerated release evidence. No failing test was blanket ignored or given a larger work
budget merely to get green output.

Source identity is the unchanged HEAD plus the archived maintenance and current
Phase 8 diff. [`phase8-source-manifest.json`](phase8-source-manifest.json) hashes
Cargo files, repository instructions, all Rust production/test sources and Python
fixtures. It deliberately excludes docs/captures to avoid a self-referential hash.
Its canonical JSON (without trailing newline) SHA-256 is
`a91d51aa3e1ef1e6dbb04c9f742de71bcb4810309e0292220df4ce968c7dbb39`.

Consistent physical production Rust LOC, including comments/blank lines and
excluding `#[cfg(test)]` items plus all external tests: HEAD **25,468**; archived
maintenance **25,510** (**+42**); Phase 8 **27,860** (**+2,350** vs maintenance).
All `src` lines including in-file tests: 32,066 → 32,245 → 34,686. New production
files: only `planning.rs` and `orchestrator/planned.rs`; no dependency added.
Measurement code and per-source details are retained in the local evidence folder.
This is a substantial opt-in feature delta, not a claim to fit 1K LOC. Its two
modules hold the contract/persistence and bounded orchestration; spawn, funding,
admission, checks, source capture, questions and review remain shared. Unused
paths were removed/replaced where extracted; unrelated modules were not refactored.

## I. Limits and empirical boundary

All provider calls in this assignment are synthetic executables. No paid model
calls, quota/account changes, private-history imports, installs, push, tags or
publication occurred. Codex/Claude fixture protocol success is not current live
provider certification. Only macOS local execution and the recorded PTY dimensions
were exercised here; no Linux run, native graphical terminal, graphical raylib
window, custom font/light-theme assessment or unrestricted stress claim is made.
One existing ignored test remains ignored; none was added or blanket-disabled.

Owner-selected verification authority, successful baseline checks, rejection of
symlink scopes, and unsupported planned crash recovery are explicit v1 boundaries.
Same-user code is not sandboxed. A plan's semantic completeness, relevance of its
acceptance prose, and overall code quality still require human review. Integration
publication and the database are not one distributed transaction.

All planning/context/child/extra calls and committed checks are retained. Provider
token/cache categories and unknown costs remain separate; unknown human time is
not estimated. The existing end-to-end metric spans root creation through completion, including
integration and later preparation; initial discovery/source capture precede that
metric even though the immutable deadline starts before them. It is not full
submission latency. Isolated integration/preparation timing is not fabricated. No duplicate live counterfactual runs or
percentage/token arithmetic support a savings claim. Phase 7's twenty-review
screening and live policy remain unchanged.

A separately approvable live ordinary-work proposal is in the
[handoff](phase8-design-handoff.md#separately-approvable-live-experiment). It requires
fresh eligible-resource validation, an explicit goal/budget/check contract, preserved
source, and real human review; this implementation does not authorize that run.

## J. Functional freeze

After the final gates recorded above, freeze new feature scope. The next pass is
cohesive visual/interaction polish and release hardening using the existing selected
scheduler mark and actual captures. No parallel execution, worker daemon, new
provider, learned planner or Phase 9 infrastructure project is proposed. This report
establishes readiness for opt-in planned-work dogfood, not public-release readiness.
