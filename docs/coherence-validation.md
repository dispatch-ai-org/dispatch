# Coherence validation: claims, metrics and falsifiers

This page records what Dispatch claims about work coherence today, what is
measured while the feature is used on real work, and what evidence would show
the thesis to be wrong. It exists so that the public story does not drift ahead
of the product. The technical reference is [coherence.md](coherence.md).

## What is claimed today

| Claim | Status |
|---|---|
| A finished result is validated against the source as it is now, and a stale result is never applied | Shipped; `tests/coherence_accept.rs`, `tests/coherence_cli.rs` |
| Verdicts carry reasons, including the old and new declaration for a broken symbol fact | Shipped for Rust and Python; file-level facts elsewhere |
| Zero false CONTINUE and zero false REFRESH on the fixture matrix | True for the 36 scenarios in `tests/coherence_matrix.rs`, each on a Git and a plain-directory source |
| Agents need no protocol; nothing is locked; no daemon or index | Shipped |
| Mid-run detection, and cancellation with `mid_run: stop` | Implemented for allocation runs; observe-only by default; no real run has been stopped |
| Tokens, minutes or money saved | Not claimed. Only wall-clock time after the first invalid verdict is recorded, and it is not cost |
| Works across languages | Not claimed. Symbol facts exist for `.rs` and `.py` only |
| Scales to many concurrent tasks | Not observed |
| Eligible results can be applied automatically under a session or invocation policy; the review stays not performed | Shipped; `tests/auto_apply.rs`, `tests/auto_apply_cli.rs` |
| Two runs finishing against one source serialize on the source lock and the second is re-judged against the new world; an edit during integration checks is fenced and re-validated once | Shipped; `tests/auto_apply_concurrency.rs` |

Every public statement must fit this table. When a row changes, change the
README and the website in the same release.

## The question that decides the thesis

L0 is `git apply --check` against the current source. Anyone can run that. The
thesis is that the symbol layer (L1) and the integration checks (L2) add verdicts
that L0 alone cannot give. If, on real work, nearly every REFRESH comes from
`patch_conflict` and nearly every CONTINUE comes from an unchanged world, then
MustHold facts are a model over a trivial check and the wedge is thinner than it
looks. Every measurement below is designed to answer this.

## What to record during real use

All of these are derivable from the `events` table and the run projection; none
needs new machinery. Record them per run where the world moved between the
snapshot and the verdict.

- **Base rate of world movement.** Share of runs whose world moved before accept.
  Near zero means the feature has no pain at single-developer scale and the story
  needs users who run agents in parallel.
- **Verdict source.** For each real verdict, what strict mode, file overlap and L0
  alone would have said. This is the direct measure of whether L1 earns its keep.
- **Reason and analysis level.** Distribution of reason codes and of `symbols`,
  `files_only` and `integration`. The share of REFRESH from `FileFallback` facts
  measures the cost of the language gap.
- **Integration check cost.** Wall-clock per L2 run and how often it flips a
  verdict. L2 builds from scratch with no cache while holding the apply locks.
- **Time to invalid versus attempt length.** From `first_invalid_at` and the
  attempt timestamps. If the world rarely moves during an attempt, mid-run
  detection has no economic value and the product is an accept gate.
- **Human agreement.** For each REFRESH or STOP, record agree or disagree at the
  moment it is acted on. A false CONTINUE is only observable when something breaks
  later; define it as "accepted with CONTINUE, then a failure attributed to the
  moved world" and label it when it happens. Human judgment stays the quality
  signal.
- **Refresh outcomes.** Whether a refreshed run was accepted, rejected or went
  stale again.
- **Bypasses.** Every use of `coherence.accept: strict`, every accept over a
  disagreed verdict, and every run on a path with no watcher (planned or legacy).
- **Auto-apply outcomes and post-hoc disagreement.** Every `auto_apply.skipped` and
  `auto_apply.blocked` reason, every `result.applied`/`application.failed` with
  `applied_by: auto_apply`, and every later `review.rejected` on a run that was
  already `applied_by: auto_apply` (a human disagreeing with a CONTINUE that was
  acted on automatically). The only dogfooding so far used a scripted agent behind
  the Codex adapter, not a real agent; treat any auto-apply numbers as fixture
  evidence until a real-agent run is logged.

## What would falsify the positioning

1. L1 adds no verdict beyond L0 on real repositories over a meaningful sample.
2. Most REFRESH verdicts are overridden, so the gate gets turned off.
3. The world almost never moves during an attempt, so the mid-run and cost story is empty.
4. File fallback on non-Rust, non-Python repositories is noisy enough that users disable it.
5. A harness or merge queue ships a rebase-and-reverify step and users find a bare signal sufficient without reasons.

Decision point: after thirty runs where the world moved, review the record
against these five. If none has triggered, invest in the next language and in
cost accounting. If one or two have, the wedge narrows to an accept gate and the
broader control-plane framing waits.

## Messaging rules

- Lead with the problem and the verdict, publicly: "Keep autonomous software
  work valid while the code moves."
- Agent selection, allocation, execution, verification, planning and machine
  control are how Dispatch carries work. They are not separate product stories.
- The "control plane for autonomous software work" framing belongs in the founder
  narrative, not on the homepage or README, until the falsifiers above have been
  tested.
- Never print a savings number that the run's own timestamps cannot support.
