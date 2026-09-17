# Phase 4 / first dogfood release report

Date: 2026-09-17. Status: **NOT READY for the first dogfood release**.
Phase 4 implementation and three small current-account dogfood tasks are complete;
the full real-task and manual terminal matrices remain incomplete. No release tag, publication, Phase 5 transport, daemon,
planner, learned routing, or second-provider execution was added.

## A. Phase 4 implementation

Bare `dispatch` prompts immediately when stdin/stdout are terminals. One foreground
session submits a goal, obtains per-goal host-execution acknowledgement, calls the
existing allocation/execution core, handles a durable clarification, presents a
fixed delivery for review, and returns to intent. No run-ID copying is required.
The session requires existing validated included Codex allocation profiles. Missing
profiles stop the goal with an explanation; they do not invoke legacy provider
selection or invent subscription authorization.

## B. Input / terminal architecture

Ratatui 0.30.2, Crossterm 0.29.0, and ratatui-textarea 0.9.2. One integrated editor
and one synchronous, nonblocking Crossterm reader, polled by Tokio. No alternate
screen. The inline viewport starts at at most 8 rows and resizes with the terminal;
committed summaries and diff previews enter ordinary scrollback. Native background
and primary foreground are preserved. Active status uses cyan, attention amber,
and successful verification green when color is enabled.

A temporary Reedline 0.51.0 → Ratatui/EventStream prototype accepted a multiline
Unicode paste, but failed at the first working-view handoff with “The cursor
position could not be read within a normal duration.” This is bounded PTY evidence,
not a claim that Reedline itself is generally defective. Reedline and its prototype
were removed from the shipped dependency graph. The integrated path passed the
subsequent PTY matrix. Prototype evidence remains at `/tmp/dispatch-handoff.log`
and `/tmp/dispatch-input-handoff.rs` for this local session.

An RAII screen guard and panic hook restore raw mode, bracketed paste, and cursor
visibility. Terminal failures during work request cancellation and await the core
future. Ctrl+C cancels work; EOF/hangup closes after cleanup. The latest projection
channel has one slot; plain input has one queued line; editor text is capped at
16 KiB and undo history at 32 entries. Diff previews are capped at 256 KiB. Work-time
input is discarded before the next prompt. Plain mode uses ordinary line input;
multiline editing/bracketed paste belongs to the integrated editor.

## C. State → presenter mapping

The presenter receives `(committed event, committed run)` only after durable state
publication. A coalescing watch channel can skip repainting intermediate frames;
immutable attempt history still displays earlier attempts and the core recovery
reason. It does not select resources, authorize execution, classify failures,
decide recovery, validate questions, grant admission, or choose final delivery.

| Core state | Human presentation |
|---|---|
| Preparing / executing / verifying | Preparing / Working / Verifying |
| Capacity, admission, reconciliation, authorization waiting | The actual wait reason |
| `recovery.selected`, attempt recovery reason | Recovering once; first attempt and recovery branch |
| Pending durable question | Waiting for you, exact question and choices |
| Ready + pending review | Ready for review, verification remains separately visible |
| Interrupted / cancelled / failed / deferred | Explicit stopped outcome and details |
| Accepted / rejected / applied | Separate review/application result |
| Application blocked by source drift | Application blocked: source changed |

An ordinary-use screenshot exposed repeated transient summaries, raw enum labels,
and no visible liveness cue. The presenter now updates one compact graph in place,
with an animated elapsed indicator (elapsed foreground time, not a progress estimate).
Only attention/final summaries enter scrollback. Recovery retains its attempt history;
exact model identifiers and allocation reasons remain available in Details. Input
and its hint follow the prompt immediately, with wrapped heights measured by Ratatui.
The pinned dependency enables its rendered-line-info feature for that calculation.
Diff previews color additions/removals when enabled and omit only the noisy index
hash header; the retained patch and plain output keep that header. Viewing a diff
no longer repeats the ready-for-review summary. No execution decisions changed.

The opening mark is the plan's filled inlet/two hollow outlets fork, not the
six-node conceptual diamond. ASCII does not change execution semantics.

## D. Clarification flow

The question command captures run ID, question ID, revision, and generation from
the displayed run. The existing Phase 3 answer handler commits the answer and
continues automatically. Empty input does not submit; cancel/EOF invokes the typed
cancellation handler. No lease is retained while waiting. Stale, duplicate, wrong
run/question, wrong generation, and unauthorized answers retain core rejection.
Deadline and invocation-budget failures are stated without offering implicit replay.

## E. Recovery presentation

Only the core initiates recovery. Earlier attempt results and the recovery reason
remain visible. The presenter never calls a retry command. The existing two-model
invocation limit, shared deadline, original baseline, final delivery pointer, and
infrastructure/unknown-verification exclusions remain unchanged. `--no-retry`
is also available on the bare interactive entry point.

## F. Review / apply

`d`, `a`, `r`, `i`, and `n` provide diff, accept/apply, reject, artifact details, and
next intent. Core `ReviewCommand` requires the complete run ID, exact candidate ID,
and displayed state revision, under the existing run operation lock. Reading a
diff and submitting review both validate that target. Acceptance and safe apply
reuse the existing feedback and application implementation under that lock.
Duplicate/stale review and a candidate from another run are rejected.

Source fingerprint, dirty tree, plain-directory, linked-worktree, and source-lock
protections are retained. A source-drift refusal can leave review accepted and
application blocked; it is not described as applied. No human evaluation is
inferred from verification. Automated fixtures exercise review actions in their
own disposable state; they are not real human preference evidence.

## G. Plain / machine compatibility

`--plain`, `--ascii`, `--no-color`, `NO_COLOR`, and `TERM=dumb` are supported. Bare
non-TTY invocation prints help and exits 2 without creating state. Existing one-shot
CLI remains available. JSON/JSONL are independent of the visual renderer.

`dispatch events RUN --after CURSOR --until attention|finished --timeout SECONDS`
uses read-only SQLite access and bounded 64-event pages. It never repairs state,
answers a question, or launches work. Events carry the outcome committed in the
same transaction, allowing a follower to see a brief question even after it has
been answered. Each read page/current state uses one SQLite snapshot. Cursor
values ahead of the committed journal are rejected. Timeout exits 124 with a
resumable cursor. Historical pre-Phase-4 events remain readable, but historical
transient outcomes absent from those records cannot be reconstructed.

No execution-schema migration or TUI state table was added. `payload.outcome` is
an additive event-payload field; UI state is not execution or sync authority.

## H. Test matrix

| Coverage | Result |
|---|---|
| Wide/narrow deterministic render snapshots | Pass |
| Live elapsed indicator, compact input placement, diff styling, no repeated review summary | Pass |
| Unicode editing, multiline bracketed paste, one submission | Pass |
| Resize during typing, working-state rendering, answering | Pass |
| ASCII, no-color, plain mode | Pass |
| Bare non-TTY and clean machine output | Pass |
| CSI/OSC/DCS/C1/bidi control sanitization | Pass |
| Ctrl+C, idle EOF, active EOF, active hangup, termination | Pass in macOS PTYs |
| Panic terminal restoration | Pass in a PTY subprocess |
| Clarification → exactly one continuation, released lease | Pass |
| Recovery → original-baseline review/application | Pass |
| Diff/review exact candidate, stale and cross-run rejection | Pass |
| Two simultaneous foreground sessions, shared pool | Pass |
| Source drift refusal; dirty-tree/plain-directory/worktree safety | Existing suite + interactive drift fixture |
| Wait observes already-answered question, startup/completion race, timeout | Pass |
| Native macOS terminal visual/manual check | Blocked: computer-use tool disallows Terminal.app |
| Linux terminal | Not exercised |
| tmux | Not installed / not exercised |
| SSH or representative PTY | Representative macOS PTY exercised; actual SSH not exercised |
| Light/dark graphical terminal themes | Not visually exercised; renderer leaves background unset |

The PTY driver uses Python 3 from the test environment and standard-library modules.
It checks restored terminal attributes, absence of alternate-screen entry, and
absence of color output under no-color. One ignored Rust test is a subprocess-only
panic fixture; the active parent test explicitly executes it in a PTY.

A full validation run exposed an older Phase 2 test's fixed-sleep race under load.
The test now holds the scarce slot behind an explicit release barrier until the
conflicting evidence is published. Its production admission policy was not changed by that fixture fix. A later
run exposed a separate diagnostic race: rejection at the final launch fence was
safe but Phase 3 replaced the specific funding error with a generic stop reason.
Phase 3 now preserves the candidate error when present. The concurrent fixture
checks that the rejection reason remains visible whichever fence observes it.

## I. Real dogfood results

The earlier Phase 1 evidence remains on disk: Luna and Terra each completed one
small Rust task. Resolved identities were recorded; observed model identity was
unknown, and is still not represented as known.

The first Phase 4 attempt correctly refused to launch because fresh evidence reported
`prolite` while the profiles authorized `chatgpt-plus-included`. The user then
confirmed the subscription upgrade. The dogfood profiles now use
`chatgpt-prolite-included`, authorization revision 2, with the same shared pool,
models, efforts, included-only constraint and no-overage acknowledgement. Fresh
provider checks passed; no paid fallback was enabled.

That upgrade exposed a real registration defect: reusing the pool name with changed
funding facts raised a SQLite uniqueness error before launch. Registration now
updates an idle pool atomically, retaining its identity, historical evidence and
monotonic fencing counter. It refuses the change while the previous overlapping
allowance has queued, admitted or reconciliation work. Old bindings cannot launch
after the mapping changes, and an invalidated authorization revision remains
invalid. Regression tests cover all of these cases; no schema migration was added.

Three real tasks then completed using four model invocations total. Every model
lease was released. Tests and review actions were driven through the interactive
terminal, including the diff view. Review actions below are scripted operator
checks in disposable projects, **not founder preference or organic human-quality
judgments**. Observed model identities remain unknown; resolved identities are known.

| Requested case | Current exercise | Resource / reason | Model invocations | Verification / outcome | Friction |
|---|---|---|---:|---|---|
| A. Localized task | Real: positive/negative tests for `add` in `src/lib.rs` | Luna/low; deterministic localized-tests light tier | 1 | Passed; scripted accept/apply succeeded | Upgrade registration bug found and fixed first |
| B. Ordinary bug fix | Real: correct `subtract(7, 2)` to return 5 | Terra/medium; deterministic localized-bug standard tier | 1 | Passed; scripted reject; original source unchanged | None in completed session |
| C. One bounded recovery | Deterministic fixture | light-model → strong-model; target-check failure | 2 | First failed; recovery passed; fixture rejected | No real failure manufactured merely to spend allowance |
| D. Clarification | Real: ask which salutation greeting tests require | Luna/low; deterministic localized-tests light tier | 2 | One question; zero leases while waiting; `Hello` answer; one continuation; passed | Broader initial task stopped because no strong-tier profile was configured |
| E. Review/apply | Real: task A, in-session diff then apply | Luna/low | Included in A | Passed; candidate safely applied | Scripted product check, not founder acceptance evidence |
| F. Reject | Real: task B, in-session diff then reject | Terra/medium | Included in B | Rejected; source unchanged | Scripted product check |
| G. Drift refusal | Real: task D, edit source before in-session apply | Luna/low | Included in D | Passed; review accepted; application blocked by source drift | Correct refusal; original edit preserved |
| H. Two local sessions | Concurrent PTY fixtures passed; real paired launch attempted | Real bug task selected standard; initial clarification task required unconfigured strong tier | 1 real task launched | Real pair did not reach shared admission; deterministic sessions did | Real contention remains unproven in this pass |

Successful run evidence in `/private/tmp/dispatch-phase1-dogfood.9152ub/state/runs/`:

- A/E: `01M2R5HYRDZ2T0W906CPE6JAR8` (35.5-second terminal session).
- B/F: `01M2R5N2QNQ22R0923FDMCSNH3` (25.2-second terminal session).
- D/G: `01M2R5QN61FTY9TWVH20GHMJ62` (41.3-second terminal session).

The exact real question was “Which salutation should the greeting tests require?”
The terminal submitted `Hello` once. Durable history contains one answered question
and two immutable attempts. Database inspection afterward found zero leases,
one retained rejected revision-1 authorization and four revision-2 authorizations.

Earlier zero-invocation failures are retained: `01M2R44TT80DJXF8VMJ3XQGBSM`
(funding mismatch) and `01M2R55XXSJ7PR4M5ZEKXZW7ZK` (registration uniqueness bug).
Timestamped terminal transcripts and the original/proposed profiles remain under
`/private/tmp/dispatch-phase4-dogfood/`. No resets or credits were consumed and no
fallback model/provider was run.

A subsequent ordinary-use report exposed over-allocation: “Let's adjust the size
of the bouncing ball to make it twice as big.” was classified with unknown task
kind and scope, which v1 incorrectly promoted to strong. Policy
`allocation-trial-v2` uses standard for unknown kind/scope, retaining strong for
recognized feature/refactor work or known broad scope. Unknown features stay
unknown; the change does not infer light eligibility or bypass profile/funding
constraints. The exact prompt now reaches review with only light/standard profiles
in the deterministic PTY regression. No model call or edit of the user's bouncing
ball project was made to test this fix.

The user's subsequent ball-project screenshots show Terra delivering the radius
change from 20 to 40 and an accepted/applied outcome. They also explicitly show no
configured verification checks. This is user-reported ordinary use, not an additional
scripted task. The reported inactive-looking and repetitive UI prompted the presenter
correction above; its validation uses fixtures and consumes no model allowance.

## J. Full validation

Final validation passed:

- `cargo fmt --check`
- `cargo test --locked`: **300 passed, 0 failed**, one subprocess-only helper ignored
  by the normal harness and explicitly exercised by its active PTY parent test.
- `cargo clippy --locked --all-targets -- -D warnings`
- `git diff --check`
- `cargo build --locked --release`; release binary reports `dispatch 0.1.2`.

The full suite includes all Phase 0–4 integration tests. Existing tests that bind
local fixture servers were rerun with the necessary local-socket permissions.
The latest full test log is `/private/tmp/dispatch-polish-full-tests.log`.
The first follow-up run hit a short-lease admission-fixture failure during
concurrent build activity; that fixture and the full suite passed on rerun after
compilation finished, without changing admission behavior. The version
was not advanced or tagged while release gates remain open. At the user's request,
the validated build was installed globally at `~/.cargo/bin/dispatch` on 2026-09-17
using `cargo install --path . --locked --force --offline`. Its checksum matches the
validated release binary. Bare invocation from outside the repository displayed
the intent prompt and restored terminal settings on EOF in a PTY smoke check.

## K. Production LOC delta

Against the dirty Phase 0–3 tree captured at the start of this task, production
physical Rust lines increased from **17,158 to 18,539 (+1,381)**. This counts comments,
blank lines and embedded SQL, excluding trailing unit-test modules, consistently
with the approved plan's method. Existing Phase 0–3 changes are not attributed to
this patch. The count now also excludes the trailing `allocation_tests` module,
which the earlier counter missed in both trees; this corrects both absolute
counts equally. The allocation follow-up adds 14 production lines; the presenter follow-up adds
166. The principal
addition is the presenter; no unrelated production
modules were reformatted or refactored.

## L. Plan deviations

- Chose the planned integrated-editor alternative after the handoff prototype failed.
- No prompt-history file: no current need justified one.
- Added a fixed-identity review wrapper and additive committed outcome payload;
  neither adds an execution state machine or database migration.
- Added an explicit barrier to an observed flaky admission fixture during hardening.
- Fixed the observed idle-pool subscription-upgrade registration defect and preserved
  specific final-launch rejection reasons through Phase 3.
- Corrected unknown-feature allocation to standard after the ordinary-use report,
  as required by the approved conservative-default policy.
- Real-account and manual terminal acceptance are incomplete, not waived.

## M. Remaining product friction

- The global installation now has the validated Pro Lite light/standard profiles
  in `~/.dispatch/resources.yml`. New installations still require explicit
  included-resource profile setup.
- The confirmed funding change is now validated. The configured dogfood profiles
  cover light and standard only; goals requiring strong stop with an explanation.
  A strong-tier profile still needs an explicit validated resource configuration.
- Plain input is line-oriented; multiline composition uses the integrated editor.
- Artifact details expose local paths; there is no full log browser or diff pager.
  Large diffs use a bounded scrollback preview plus the full artifact.
- Historical events cannot retroactively gain missing transient state snapshots.
- Decorative graph animation, persistent prompt history, provider setup, and broader integrations
  were deliberately not built.

## N. First dogfood release readiness

The deterministic intent → allocation → bounded work/recovery/clarification →
verification → review/apply loop is implemented. Release readiness remains blocked
by the uncompleted real-task/manual-terminal checks. Current-account Pro Lite
authorization, Luna/Terra execution, real clarification, and the in-session
apply/reject/drift-refusal flows now have evidence. The approved plan's multi-day
voluntary-use gate (20 eligible tasks over at least five working days) also cannot be established in this session. Do not
represent fixture acceptance as founder preference or claim demonstrated capacity
savings.

NOT READY
