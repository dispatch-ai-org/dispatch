# Private outcome evidence and controlled trials

Phase 7 is local descriptive evidence and one deterministic trial rule. It adds no
provider, account capability, learning service, judge, planner, or execution path.
Public benchmark priors and `dispatch evidence local <source>` retain their old
inspection-only contract. Private annotations/proposals never enter sync envelopes.

## What is counted

`dispatch evidence private <full-run-id>` reads committed SQLite projections in
one read transaction. It reports the frozen decision-time metadata separately
from current evidence. The default boundary is the same canonical project, the
preceding 90 days, and at most 1,000 goals (newest first). A truncated window cannot
propose a trial. Source/check baselines remain in existing immutable run artifacts;
no transcript or artifact tree is parsed during selection.

Project counts include every retained goal in that boundary, including unknown
origin, experiments, failed work, missing reviews, and multi-attempt work.
Origin-specific counts and economic observations keep these populations separate.
`goals` counts roots, `attempts` counts prepared attempt records, `launched` counts
known actual invocations, `prelaunch` counts affirmative unlaunched attempts or
goals with no attempt, and `launch_unknown` retains uncertainty. These are not
interchangeable denominators. Requests refused before the existing core creates a
durable goal have no goal/attempt fact to count; this is retained-goal accounting,
not a count of every attempted CLI submission. Admission lifecycle knowledge survives lease release;
historical exit/timed-out facts also establish launch. Failure maps retain normalized
attempt failures and final goal failures separately; they must not be added together.
The existing provider/core failure vocabulary is preserved, including its historical
limits: an unclassified process failure is not relabeled a coding-quality failure.

Resource cohorts require an attributable single executor invocation, no inherited
parent or clarification chain, compatible pre-execution facts, known harness
version, resolved profile, and no known substitution or operational failure.
`eligible_executions` includes missing reviews and mechanical failures in that
cohort; `reviewed = accepted + rejected`; `missing_review = eligible_executions -
reviewed` within each cohort. Acceptance with passed verification, acceptance
without it, and acceptance with blocked application remain independent counts.
Known check failure remains failure without human feedback. First-attempt checks,
continuation count/triggers, final verification, eventual goal review, and complete
chain burden remain visible without assigning goal review to individual attempts.
Exclusion counts can overlap; they are reasons, not an additive partition.

Human evidence requires the latest feedback revision plus an explicit local-owner
attestation for its exact delivered artifact. The revision remains in
`goal_feedback_revisions`; an annotation creates no acceptance/rejection label.
A later feedback correction needs a new attestation. Superseded revisions remain
in the original history. Structured correctness, completeness, and rework reasons
can support this rule; arbitrary prose is never classified. Changed requirements
and source drift veto a quality-rejection trigger even alongside a quality reason.

Ordinary work, synthetic fixtures, scripted smokes, and unknown origin are explicit
annotations. TTY use, `accept`, submission by a machine, and model text do not infer
human judgment. Existing history defaults to unknown. Machine submission is recorded
separately through existing control ownership; it does not disqualify later real human
review. No history was imported or privately inspected during implementation.

## Optional owner annotation

Review and safe apply are unchanged. Accept/apply or reject normally; the next-goal
composer remains immediately available. Its optional **[f] Use this review for local
routing** action shows the reviewed task, outcome and delivery summary. Choose
**[c] Confirm this statement** to attest: “This was ordinary work and reflects my
own review. Record it as local routing evidence. Routing will not change
automatically.” Choose **[b] Back without recording** or Ctrl+C to cancel; Ctrl+D
exits. Ignoring the action creates no annotation. No run ID or digest needs typing.

The presenter pins the displayed run, delivery digest and feedback revision when
the review finishes. Confirmation and the advanced command share owner/provenance
validation and a short atomic persistence path. Changed delivery, feedback or
provenance is refused; it is never silently retargeted to the latest run. Repeated
identical confirmations are idempotent. An annotation error preserves successful
review/application and can be retried. After an error, `f` shows refreshed facts
for that same run and requires a new explicit confirmation; it never switches to
another session's latest result.
Accepted-but-apply-blocked remains blocked; rejection does not restart work.

The advanced alternative remains available:

```sh
dispatch evidence annotate RUN_ID --origin ordinary --review human
# Optional reported hands-on repair time; never elapsed time waiting for review:
dispatch evidence annotate RUN_ID --origin ordinary --review human --repair-minutes 8
```

The advanced command shows the exact goal, delivery digest, and feedback revision
and requires an explicit controlling-terminal attestation. This is an owner assertion,
not biometric proof. Scripts must use `scripted` review and `scripted_smoke` origin;
the human interface does not convert scripted approval into human judgment. Recorded
synthetic/smoke provenance cannot be upgraded to ordinary through this interface.
The trust boundary is the existing local OS owner; an arbitrary same-UID database
writer can already alter local state. Phase 5 grants have no annotation, review,
activation, or rollback operation, and cannot supply a `human=true` authority.

For observation-only collection, start with one owner-selected project and one
meaningful existing task/check mapping. Use Dispatch for ordinary work, review
normally, and optionally contribute that explicit review in-session. Use existing
structured quality/rework reasons only when they apply. Inspect evidence or shadow
when useful; insufficient evidence is expected initially. The mapping below is an
example, not an assertion that a generic command verifies every task. No mapping,
shadow setting or trial is enabled automatically. Leave real activation for separate
explicit approval; do not manufacture tasks, reclassify smokes or keep a required
time diary to reach the screening threshold.

## Compatibility and rule v1

Add an explicit routine task/check mapping to the project's `dispatch.yml` to
permit proposal screening; this does not alter Phase 6 light eligibility:

```yaml
private_evidence:
  shadow: true
  routine_mappings:
    - id: routine-rust-tests
      features: {language: rust, task_kind: tests, scope: localized}
      verify: ['cargo test --locked']
```

This is an owner assertion that the listed **existing configured verify commands**
are relevant to this task class. It does not independently establish low risk,
minimum suitability, or light-profile eligibility. The commands must
exactly equal `checks.verify`; a mapping does not execute or install new checks.
Unknown language/kind/scope, absent checks, or conflicting mappings cannot screen.
Explicit localized and multi-file mappings support descriptive cohorts; rule v1
screens only localized tasks. A multi-file mapping returns
`task_scope_outside_trial_rule` even with sufficient reviews. Broad scope remains
excluded. Adding a mapping does not retrospectively change frozen run metadata.
The current classifier conservatively requires enough source files to establish
language; a file extension in task text alone is not enough. C uses the existing
dominance threshold (at least two lowercase `.c` implementation files). Ambiguous
`.h` headers and uppercase `.C` files do not establish C; ignored/generated source
directories do not contribute. C recognition does not establish task kind or scope.
Feature intent such as “add another …” or “add a third …” is recognized separately
from scope. Explicit test-writing objects (including “add a third regression test”)
remain test tasks; later instructions to run tests do not change feature intent.
Scope still requires pre-execution file evidence. A task/check mapping never supplies
missing scope, and neither new mappings nor stronger checks rewrite old decisions
or verification results. Recognizing feature intent retains the existing strong-tier
suitability requirement, even when scope remains unknown.

Compatibility includes canonical project, deterministic language/kind/scope,
explicit mapping and check-contract digest, effective policy version, provider,
harness/version, requested/resolved model, effort, service mode, runtime, controlled
adapter contract, executable bytes, and effective harness/execution settings.
Exact commits and final diff sizes do not define task classes. Funding labels,
pool display labels, and renewal of unchanged authorization proof are not behavior
identities. Immutable full launch-profile/funding pinning is unchanged. Actual
harness versions and observed model/effort knowledge form separate outcome groups.
Codex's unknown observed identity stays unknown; proposals concern the resolved
profile, never a confirmed underlying-model acceptance rate. Executable hashing is
local, streaming, and invokes no program; it cannot certify arbitrary mutable
external dependencies of a wrapper. Multiple observed versions under one behavior
abstain. Missing local executable identity and nonlocal runtime abstain for this rule.
Pre-Phase-7 runs lack the frozen mapping/behavior facts and cannot silently enter
these proposal cohorts.

`private-quality-trial-v1` has fixed screening parameters:

- One project, one mapped localized task/check class, one compatible light-profile cohort.
- Ninety-day window, at most 1,000 project goals, at least **20 reviewed** comparable
  goals and **80% review coverage**.
- At least **5** explicitly classified quality/rework rejections and at least
  **25%** of those reviews. Operational failures cannot satisfy this trigger.
- An existing eligible standard/strong alternative with at least **one compatible
  verified execution**. Missing alternative human reviews permit a **trial
  proposal**, never an acceptance comparison or uplift estimate.

These are screening thresholds, not calibration or automatic approval. Public
priors, unrelated resources, incompatible versions, or duplicate feedback events
cannot fill the threshold. Rule parameters/version are retained in each proposal;
there is no executable rule language or optimizer. Parameters are not tuned on the
same sample. The selected alternative follows the existing alternative order.

## Shadow, activation, and rollback

Shadow reuses the actual selection's frozen task/features and eligible/excluded
alternatives after existing probes and capacity checks. It never reserves admission,
spawns, verifies another output, reads credentials, or changes funding. Only actual
execution gets outcomes. The comparison records cutoff, annotation revision,
cohorts/counts, reasons, actual/base and hypothetical choices, proposal ID, and an
evidence digest. Proposal records reference exact feedback revisions and immutable
outcome facts. Decision metadata is immutable in SQLite; repeated explain cannot
rewrite it. Historical comparison is descriptive, not causal.

```sh
dispatch evidence propose RUN_ID           # May correctly return insufficient evidence
dispatch evidence policy PROPOSAL_HASH     # Exact content, limitations, validity, active revision
dispatch evidence activate PROJECT PROPOSAL_HASH --expected-revision 0
dispatch evidence rollback PROJECT --expected-revision 1
```

Proposal IDs are content hashes. Activation displays the exact proposal and requires
an explicit owner attestation. In a short SQLite immediate transaction it validates
hash/version, project/check/resource compatibility, current evidence eligibility,
unchanged screening counts, status, and the expected active revision. The atomic
transition preserves actor, prior revision, proposal/evidence basis, and the fixed
base-policy rollback target. Rollback restores `allocation-portfolio-v3` for future
goals; it does not reinstate another older trial. Only one trial preference per
project is active. Revoked/superseded IDs cannot reactivate through stale caches.

The preference is applied only to a matching light-profile task class inside the
already suitable/authorized eligible set. Explicit choices remain decisive. Missing,
changed, disabled, exhausted, or unauthorized targets use the existing deterministic
fallback or existing blocked/deferred handling. No grant or funding scope is expanded.
New machine submissions check the grant's bound policy revision both before selection
and within its read snapshot. Already bound attempts, receipts, queued work, and
Phase 3 recovery do not recalculate a newer private policy. Existing configuration
change refusals remain in force.

Correction of referenced feedback, provenance, verification/attempt facts, or expiry
makes the active proposal stale on selection and returns future routing to the base
policy. It is not regenerated or reapproved. Policy inspection reports current
validity even before that lazy status update. Ordinary new evidence does not silently
change the approved rule. Analytics query failure records unavailable evidence (null counts, never fabricated zeroes) and
falls back; failure to read authoritative active-policy/grant state is not waived.

## Economics and privacy

Durations report count/total/min/max and coverage: harness execution, admitted wait,
verification, and goal end-to-end wall time. Optional attested repair minutes are
hands-on reports; missing reports remain absent. Whole-origin project-window burden
includes failed and continuation attempts, not just successful final attempts. The
harness-time-per-accepted metric is undefined with zero acceptance, uncertain launches, or incomplete
known-launch duration coverage; it is descriptive wall-time burden, not money or
savings. Token views retain provider/harness/semantic units; terminal categories and
normalized totals are alternate views and must never be summed. New Claude
observations retain input/output/cache-read/cache-creation categories once. Historical
missing category fields remain unknown; no raw transcript import occurs. Nominal USD
is not cash charged. Cash cost and allowance attribution remain unknown where no
attributable durable measurement exists; no pool percentages or provider token units
are converted to a global balance.

Owner detail is local. Normal explain shows concise project counts and points to
advanced detail. Scoped machine results omit private evidence, including embedded
attempt-decision copies; they receive no other goals' IDs, paths, notes, or aggregates.
Existing run/artifact/event scope remains decisive. Nothing is added to Cloud ingestion.

## Prospective dogfood

Start with ordinary eligible work and shadow only. Preserve task mix and note why
a direct harness was used for an otherwise eligible task. Use normal human review;
optional provenance/repair annotation is sufficient—no mandatory time diary.

Before a separately approved trial, freeze its content hash/version, project and
mapped task class, evidence cutoff, a future evaluation window, and the owner's
acceptance/rework/latency guardrails. Keep proposal-building observations out of the
subsequent evaluation population. Retain operational failures, unreviewed work,
all invocations, and complete-chain burden. Compare allowance only if its units and
attribution support it; otherwise report workflow, resource, and time facts without
savings. Do not rerun alternatives for shadow or invent their outcomes. Do not retune
repeatedly on the same small sample. Voluntary use is a product signal, not causal
proof. Roll back future selection if the owner rejects the trial or its predeclared
guardrails warrant it. No real trial was activated by implementation or validation.

A small suggested evaluation window is the next 20 ordinary eligible goals over at
least five working days after separate approval, compared descriptively with a
frozen preceding window of the same task/check class. Include every miss and bypass.
Before activation, the owner should name the acceptable accepted-and-verified count,
maximum serious rework/regressions, and tolerable end-to-end latency increase. If
review or optional repair coverage cannot support a guardrail, report that guardrail
as unresolved. These choices are prospective trial conditions, not new automatic
promotion logic or an assertion that 20 goals establishes causality.
