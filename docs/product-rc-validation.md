# Dispatch 0.1.3-rc.1 — product finish and local release validation

Date: September 18, 2026. Scope: **unsigned macOS arm64 local experimental RC**.
**PRODUCT RC: BLOCKED** — the required frozen-source, four-thread suite failed
`phase8_planning::question_deadline`: the original five-second budget expired
before a question existed. Release artifacts are available for review, not approved
for publication. This remains an engineering gate, separate from owner publication
or provider authorization decisions.

The source and evidence are committed at the owner’s request; the failed deadline
gate remains unresolved. No provider model calls, real
login, additional model agents, publication, account change or live private-policy
activation occurred during this pass. All new executions used synthetic providers,
disposable source trees, homes and state. The live raylib application was untouched.

## What shipped

One inline design uses the selected open-D scheduler, terminal-native colors,
clear goals, factual direct/sequential progress and progressively deeper review.
Small-review controls sit beside useful content. Large review labels binary/mode
changes and generated-file hints, marks clipped long lines, and exposes complete
patches beyond bounded previews. Questions identify their original deadline and
launch budget; waiting no longer claims a model is running. Setup-interrupted
goals return for explicit submission. Active work input remains paused.

One shared CLI/TUI setup service discovers supported native authentication, builds
profiles mechanically, requires explicit funding assertions, and atomically saves
only a still-current proposal. Numbered revalidation handles daily Claude expiry.
Provider login uses the existing terminal handoff. Approved project-check choices
reuse existing configuration. No new account service, grant semantics or executor
was introduced. The launch core still checks account/funding/admission authority.

A concrete upgrade defect was fixed: historical schema migrations now preserve a
consistent private database backup first, and future schemas are refused before
migration. Schema is still **20**. Release packaging is deterministic for a given
validated executable/source manifest, rejects wrong-version binaries, and is shared
by the existing release workflow and local packaging command. No dependencies were
added or upgraded.

## Source and executable identity

Comparison began from a clean tree at
`08fc0a7633d18c1cae39ab93cd36fc5405798cc1` (Phase 8). Baseline and current builds use
separate target directories. Baseline version 0.1.2 executable SHA-256:
`a228fd603757914a7af82f8a2029257ddcaac3d19be13174512bf33f73b3d50e`.

The final 38-file runtime freeze includes Rust source, Cargo manifest/lock and the
bundled public-prior snapshot. Canonical sorted compact-JSON manifest SHA-256:
`e01db3a29da11ea544ef4d88e97a30ac32283490d3b9ebeab08e1b5d39b4b39f`.
The [runtime manifest](product-rc-runtime-manifest.json) identifies every file.
The package's `BUILD.json` additionally identifies fixtures, schemas, assets,
scripts, workflow/build contract and installation instructions. Reports/captures
are excluded from that digest to avoid circular identities. The manifests identify the candidate built before the owner-requested commit;
the recorded build-time Git HEAD is its baseline, not the finish-pass commit.

Release executable SHA-256: `433da1f8600d2cd1e9765ad1c7e17d9e08802d5426dd199cd201f67bcfe7d144`.
Extended source-manifest SHA-256: `a83247a56245f84973ef8fd9743f060dd94d376a8005fa5dc4551b5cbc80327a`.
Archive SHA-256: `24d3e9964bbdd3cd03f61f13c502e1f95a0b4f4bd866bd60d18264e213922134`.
Full [build manifest](product-rc-build.json); local outputs are
`release-artifacts/0.1.3-rc.1/dispatch`, `dispatch-macos-arm64.tar.gz`, `SHA256SUMS`
and `BUILD.json`. Two packages of the same inputs compared byte-for-byte equal.
The extracted binary passed version, clean non-TTY and read-only isolated-state
checks. Archive names, file modes and packaged bytes were verified explicitly.

Toolchain: Homebrew rustc **1.97.1 (8bab26f4f 2026-07-14)**, Cargo **1.97.1
(c980f4866 2026-06-30)**; Darwin **25.6.0 arm64**. Ratatui 0.30.2, Crossterm 0.29.0,
ratatui-textarea 0.9.2 and Unicode libraries retain their existing locks.

## Acceptance journeys and gates

- `cargo fmt --all -- --check`: passed.
- `cargo test --locked --target-dir /tmp/dispatch-rc/current/target -- --test-threads=4`:
  **436 passed, 1 failed, 1 existing ignored helper** before Cargo stopped at Phase 8.
- The eight remaining suites (`product_ux`, `public_priors`, `recommend`,
  `resource_setup`, `routed_run`, `routing`, `routing_observations`, `terminal_bench`)
  were then run explicitly with the same locked build and four-thread setting:
  **60 passed**. Doc tests: 0 tests, successful command. Total **496 passed / 1 failed**;
  this is not a green full-suite claim. [Counts](product-rc-test-results.json).
- `cargo clippy --all-targets --locked --target-dir /tmp/dispatch-rc/current/target -- -D warnings`:
  passed. `git diff --check`: passed.
- `cargo build --release --locked --target-dir /tmp/dispatch-rc/current/target`: passed.
- Frozen release executable: Phase 5 `smoke`, Claude Phase 6 `direct`, Phase 7
  `smoke`, and Phase 8 `success mixed question deadline fidelity`: passed. The
  success/mixed/question/fidelity scenarios recorded 3/3/4/3 fixture invocations;
  the deadline scenario stopped its held planner at the original limit.
  Phase 7's six synthetic invocations and explicit policy activation/rollback were
  confined to disposable fixture state. Real private policy was unchanged.
- Deterministic packaging, SHA-256 checks, four-file archive whitelist, extracted
  version, read-only fresh-state status, non-TTY behavior and stale-binary refusal:
  passed. Packaging and capture scripts parsed successfully without extra libraries.

[Final full-suite log](product-rc-logs/frozen-tests.log),
[remaining suites](product-rc-logs/remaining-tests.log),
[build](product-rc-logs/final-build.log), [Clippy](product-rc-logs/final-clippy.log),
[release smokes](product-rc-logs/release-phase8.log). Earlier failed attempts and
baseline/candidate timing diagnostics are retained in the same log directory.

| Brief journey | Evidence exercised on the local OS | Scope / result |
|---|---|---|
| A. Fresh user | `resource_setup` PTY: missing resource → discovery → cancel/confirm → preserved goal; explicit check choice | Zero model calls; setup creates no DB/grants; preserved draft requires submission |
| B. Resource contracts | Shared setup unit tests and CLI/TUI fixture; Claude expiry, cancelled renewal, changed account, unsupported auth; existing Phase 2/6 funding tests | Exact model/effort/tier preserved during refresh; explicit consent; stale file/symlink refusal; no global settings change |
| C. Direct loop | `phase4_cli`, `phase4_review`, `phase3_recovery`, `phase7_evidence` private review fixtures; direct/failed/unverified captures | Success, failure, checks, apply/reject/drift, optional explicit feedback and next intent; no duplicate review transition |
| D. Planned work | 66 of 67 `phase8_planning` cases passed; success/mixed/root-check/recovery/dependency/fault/fidelity fixtures | `question_deadline` blocks the full gate. Other cases preserve sequential bounds, integrated review, constrained resources and failed prerequisites; no live-planner claim |
| E. Questions/cancellation | Phase 3/5/8 revision, replay, deadline, EOF and control tests; question capture; new waiting-state regression | Exact target, lease release, no budget reset or hidden input submission |
| F. Large review | All four `phase4_review` tests; permission/generated indexing regression; 53-entry capture with 40,000-character line | Search/navigation, binary/rename/mode entries, bounded/full patch, native pagers/editor handoff/failure, stale candidate and source drift |
| G. Foreground/control | Phase 3 two-session tests and all 30 Phase 5 tests; Phase 6 control reuse | Shared admission, scoped identities, idempotent receipts, slow reader/disconnect, clean JSON/JSONL; no daemon |
| H. Terminal behavior | Presenter/editor/theme tests, Phase 4 PTYs, Phase 8 narrow/plain/question PTYs, setup login return | Unicode/paste/ASCII/no color; light/native/256/16 tokens; resize/minimal cells; sanitization; EOF/panic/signal restoration; full-screen only on deliberate inspection |
| I. Installation/upgrade | All DB tests, historical schema-19 backup/readability/permissions/current-open checks and future-schema refusal; extracted archive smoke | Schema 20 unchanged, checksums and archive whitelist, no replacement of PATH binary, isolated state; no credentials/logs/grants/fonts packaged |

The new waiting-state test supplements the earlier presenter/reviewer/setup/DB
targeted checks. Terminal restoration is asserted by the PTY fixture against
actual saved OS attributes. The pre-existing ignored panic helper is launched by
its supervising restoration test; no test was newly ignored. Fixture verification
is evidence of mechanics, not a human quality label or provider endorsement.

An intermediate full run under simultaneous compilation/capture activity failed
the existing two-second Claude deadline fixture because no JSON result was found.
Its original failure log is retained. Isolated baseline/candidate checks and a
four-worker matrix with 0/0.3/0.8-second auth delay all returned the expected deadline
JSON and preserved the invocation bound. The next clean run passed that test.
The fixture now prints stdout/stderr if that assertion fails; its timeout and
assertions were not relaxed. The initial failure's exact cause remains unconfirmed.
That intermediate run later overlapped final compilation and failed two Phase 8
PTY timing scenarios: narrow review had not finished checks within the fixture's
wait, and a question's original deadline expired before its child reached a
question. Those logs are retained too. The final frozen-source gate runs after
compilation, without concurrent builds/capture work, and is reported separately
above. No blanket retry or timeout inflation was added.

The final gate passed both those PTY cases but failed the non-PTY
`question_deadline` case. A controlled baseline/candidate comparison reproduced
the preparation sensitivity in the unmodified baseline too, including a diagnostic
with compiler checks replaced by fast shell checks. At the unchanged five-second
budget, the baseline stopped after one launch and the candidate before any launch;
both correctly persisted deadline failure. Diagnostic timing data are retained in
the validation logs. No test assertion, deadline or production guard was relaxed,
and the fixture was not replaced by a weaker test. A deterministic regression
design or a stable, supported runtime test environment is still needed to close
this gate; public RC readiness is not claimed.

Earlier sandbox-only failures were local loopback/controlling-terminal `EPERM`;
the relevant fixture suites were run with approved OS access. A setup fixture's
invalid test configuration and actual ASCII/review-draft regressions were fixed
before the final gate. No production authority was weakened to satisfy a test.

## Visual and performance evidence

[Before/after gallery and repeatable demo](captures/product-finish/README.md)
contains actual rendered VT captures and raw timed recordings. Inspected sizes:
120×40, 100×36 setup, 80×24 light/monochrome and 38×24 narrow. Blank outer rows are
cropped with counts in metadata. White-background previews explicitly render the
light theme; the dark preview surface does not force a terminal background.
These are real macOS PTYs rendered with Menlo, not native GUI screenshots.

Visual fixes came from inspection: scheduler glyph geometry, oversized review gap,
narrow verification visibility, truncation/clipping and misleading running text
during questions. Native GUI terminal emulators, Linux, SSH and tmux remain gaps.
Docker's daemon was unavailable; no cross-compile or unexecuted CI job is claimed
as runtime validation. There is no Windows support claim.

Ten sequential final-release PTY samples: staged-intent acknowledgement
**50.6–59.3 ms** (median **54.9 ms**).
Fresh process to captured intent: **141.9–153.6 ms**
(median **152.2 ms**), including capture settling.
Eight separate `version` processes took **4.48–4.88 ms**;
this is not UI startup and the filesystem cache was not purged.
[Paired timing observations](product-rc-timings.json) preserve baseline/final values.

Input timings isolate local acknowledgement after a staged bracketed paste and
Enter. The reader polls at 50 ms, so values are upper bounds, not exact latency or
provider performance. Startup milestones include an 80 ms capture settling window.
No universal sub-100-ms claim, token equivalence or subscription savings claim is
made. Redraws are coalesced; the elapsed display remains factual and reduced-motion
mode uses a static mark.

## Installation, support and remaining owner decisions

Use the explicit extracted/local executable path; do not replace an existing
installation to try this candidate. [Install/upgrade/rollback](release-install.md),
[product guide](product-guide.md), [planning](planning.md),
[control protocol](control-protocol.md), [design system](design-system.md), and
[provider support](provider-support.md) describe the declared contracts.

1. Decide whether to publish the **unsigned macOS arm64 experimental RC**. No tag,
   push, upload or signing operation has occurred. Linux is an optional wider
   publication scope requiring its own runtime gate, not a claimed result here.
2. Decide public Claude support wording against the current provider terms; obtain
   clarification if required. Prior real Claude evidence is limited to Code
   2.1.274, personal first-party Pro, credits disabled, fixed claude-sonnet-5,
   medium/standard controlled print. Historical Codex direct evidence uses
   Luna/low and Terra/medium. This pass adds fixture evidence, not new live calls.
3. Separately authorize a genuine planned-provider exercise before advertising it
   as live verified. The [prepared exercise](phase8-design-handoff.md#separately-approvable-live-experiment)
   uses a disposable raylib copy, one setting/documentation goal, approved original
   checks, at most four invocations and 600 seconds, no paid fallback and one human
   review. Exact freshly eligible profiles must be displayed at authorization time.
   No account discovery or allowance has been spent to fill this gap.

Ordinary local execution is not sandboxed. Planned crash recovery, active-work
drafts/live steering, automatic policy promotion and universal provider/model
support are not claimed. No parallel executor, daemon, additional provider, host
integration, cloud account platform, model judge or orchestration framework was
built. Empirical savings research is not a prerequisite to this scoped RC.

No website source exists in this checkout. Copy handoff for its owner: “Describe a
software goal. Dispatch selects from resources you explicitly authorize, supervises
bounded work, runs your configured checks, and brings one change set to human
review.” Use the open-D asset; label planning experimental and the macOS candidate
unsigned, with supported-provider details rather than a universal savings promise.

## Reviewable change boundaries

Inherited maintenance is commit `80e4b56`; inherited Phase 8 is `08fc0a7`. Neither
was rewritten. The owner-requested finish-pass commit includes:

1. Scheduler/presentation/review changes and focused tests/assets.
2. Shared resource setup, thin Claude auth extraction, terminal handoff and fixtures.
3. Pre-upgrade backup/future-schema refusal and DB regressions.
4. RC version, shared local/release packaging, docs and actual capture evidence.

The owner explicitly requested this commit. Production Rust grew by **1,029 physical lines**
relative to baseline `08fc0a7`, excluding trailing `#[cfg(test)] mod tests` sections. Of that,
635 lines are the explicit shared setup service/controller; the rest is primarily
presentation/accounting and review/upgrade handling. This is a usability adapter
over current configuration and execution, with no new dependency or executor.
The [LOC ledger](product-rc-production-loc.json) records file counts. Binary/archive
outputs live in ignored `release-artifacts/`; raw fixture evidence and validation
logs contain no actual provider credentials or user task/source content.
