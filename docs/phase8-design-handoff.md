# Phase 8 functional freeze and design/release handoff

The implemented workflow is an explicit planned goal, one bounded planner,
sequential checked tasks, deterministic integration, root checks and one reviewed
change set. Direct work remains the default. After the gates in
[Phase 8 validation](phase8-validation.md), freeze feature scope and concentrate on
cohesive visual/interaction polish and release hardening.

## Preserve the actual selected identity

The current selected compact scheduler signature lives in
[`src/presenter/theme.rs`](../src/presenter/theme.rs): two rows with a filled inlet,
two hollow outlets, project context, native background, and explicit ASCII/color
fallbacks. Phase 8 does not modify this file or replace the theme system.

The retained approved terminal references are actual files under
[`docs/captures/phase4-refinement`](captures/phase4-refinement/README.md), including
`startup.png`, `working.png`, `tiny-review.png`, and `large-review.png`. These are
recorded terminal-byte renderings with metadata, not native graphical screenshots.
Do not substitute early fork/diamond mockups or invent a different asset path.

## Current states and observations

The presenter is a projection of committed state. It does not choose runnable tasks,
satisfy dependencies or carry a separate mutable schedule. The small task list is
explicitly labeled **sequential execution**; geometry must never suggest parallel
workers or independently verified incidental context.

| Real state | Current functional presentation | Next polish focus |
|---|---|---|
| Before submission | Composer advertises `/plan`; ordinary text stays direct | Make mode selection/disclosure fit the compact composer without hiding direct work. |
| Planning | Planner resource and one shared maximum-work disclosure | Keep read-only planning and implementation distinct in plain language. |
| Validated/working | Task objective, chosen model, state, required checked task output | Improve truncation and alignment on narrow screens; show dependency meaning rather than emphasizing IDs. |
| Dependency/clarification | Waiting state and durable question; no model lease | Keep required decision visible even with a compact task list; preserve automatic authorized continuation. |
| Integration/root checks | Distinct integrating/verifying phase from real transitions | Clarify progress without treating “child exited” as root success. |
| Stopped/deadline | Root reason, retained attempt/task evidence, Details; no final apply candidate | Give failures a clear visual hierarchy; never retain a misleading active question after expiry. |
| Final review | One root diff, Details, existing editor/pager, accept/reject/leave pending | Keep final goal review dominant; no per-child acceptance or duplicate ready state. |
| Next goal | Existing optional provenance attestation remains separate | Preserve explicit ordinary/human versus scripted provenance and fixed reviewed identity. |

Observed fixture friction: at 38 columns, long objectives and model/state labels
wrap and lose detail; dependency task IDs are functional but less readable than
short task titles. Details now contains suitability provenance and lineage, but can
be verbose for a four-task goal. “Invocation records” distinguishes recorded work
from actual launch accounting in the documentation; presentation can make that
meaning easier to see. The sequential label and the shared-extra disclosure repeat
context intentionally for this first usable pass; polish their density without
removing the limits. ANSI differential redraws must be assessed as reconstructed
terminal cells, not by treating a stripped raw transcript as a whole new frame.

## Recorded Phase 8 evidence

Six inspected, bounded previews and their metadata are in
[the Phase 8 capture index](captures/phase8/README.md): startup, wide review,
narrow review, plain review, pending question and expired question. The raw
transcripts stay outside the repository. Each capture is tied to the exact final
release executable by `capture-index.json`.

The first 38-column rendering exposed a hidden verification summary. The focused
fix puts verification and change counts above the longer planned task list;
`planned_review_keeps_verification_visible_before_long_task_list` protects limited
content rows, and the final release capture verifies the real screen. Full task
provenance remains available in Details. Further graph density/alignment work is
deferred to this dedicated pass.

The release fixtures exercise OS pipes and real macOS PTYs, plain input,
38×32 and 100×32 ASCII/no-color sessions, Details and diff return, pending review,
question expiry and exact terminal restoration. Existing Phase 4 regression tests
retain editor/native-pager behavior and the theme fallback checks. No native GUI
terminal, Linux execution, custom-font, light-terminal-background or graphical
raylib assessment is claimed. The preview renderer chooses a dark surface; Dispatch
itself retains the terminal background.

Next release work should check the same final binary in supported native terminals,
assess light/dark/custom palette contrast, inspect accessibility and wrapping, then
repeat source-drift, review/editor return, deadlines, disconnect and cleanup gates.
No package install or replacement binary was made by this assignment. Keep release
publication as a separate explicit action.

## Separately approvable live experiment

**Proposal only; no live invocation is authorized or executed here.**

- **One genuine goal:** on a disposable copy of the raylib project, make horizontal
  ball size a named scene setting while preserving the current default behavior,
  then document how that setting affects drawing and boundary/collision behavior.
  Require a single consistent setting used at all existing call sites. The real
  owner should confirm that this is useful ordinary work before spending.
- **Resources:** one currently eligible strong planner, followed by resources that
  satisfy each validated contract under the owner's existing inclusion, funding,
  account, capacity and explicit constraints. Do not force light: only an existing
  owner routine/check mapping may allow it. Exact live profile identities and fresh
  funding eligibility must be shown at the approval/preflight boundary; this
  implementation deliberately made no live account probe and cannot attest that a
  particular account is currently eligible. No paid fallback or account changes.
- **Bound:** `--plan --max-invocations 4 --timeout 600`; one planner, at most two
  initial tasks if one extra is to remain available, one shared extra, no deadline
  reset. If the plan cannot fit, stop after its single planner. Do not induce a
  real failure to exercise repair.
- **Checks and source:** copy the current original tree and retain its fingerprint;
  freeze the existing `verify.sh`, collision test and layout test as verification
  authority. The current checks compile/link with warnings as errors, exercise
  13 collision cases and check layout at two dimensions. These do not prove a new
  rendering outcome; inspect the new setting/call sites and graphical behavior
  during actual human review. Do not change the live application until explicit
  acceptance/apply, and keep the original source available.
- **Outcome/evidence:** retain every planner/child/extra attempt, original and final
  snapshots, all checks, actual/unknown launch facts and separate provider token/
  cache categories. Record the real person's accept/reject and hands-on repair time
  only if actually supplied. No scripted human labels. One root attestation only.
- **Ordinary-work comparison:** declare the goal, check contract, eligible resource
  policy and review standard before work. Compare with subsequent comparable
  ordinary direct work only if it arises independently under that protocol; do not
  repeat this live goal simply to invent a counterfactual. Report overhead and
  unknowns. Savings/default changes remain pending separate evidence and approval.

Approval of that concrete proposal would authorize only the stated bounded local
experiment after fresh preflight, not a portfolio/account change, private-policy
activation, benchmark tournament or public release.

## Scoped continuation

The next product pass is visual/interaction polish and release hardening. Keep the
current scheduler mark, theme, compact startup and tested diff/editor workflow.
Do not turn this handoff into parallel execution, persistent workers, free-form
replanning, another provider, a hosted service, an IDE, or a Phase 9 infrastructure
project. Planning remains off by default until separately authorized empirical
results support a change.
