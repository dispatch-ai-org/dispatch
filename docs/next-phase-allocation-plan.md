# Dispatch: intelligent allocation of software work

## Governing product direction — September 17, 2026

**Dispatch is the product. Phase 5 is “Agent-native Dispatch and interoperability.”** Build the generic machine interface over the existing core first, and demonstrate its complete workflow with a standalone test client without Herdr installed. Herdr support is an optional, bounded adapter with separate integration tests; it is not a prerequisite for the generic Phase 5 deliverable.

Dispatch users and the software-work allocation thesis drive the domain model, scheduling policy, human UX, visual identity and roadmap. Intent, execution visibility, clarification, review and safe apply remain first-class experiences directly in Dispatch. Design follows Dispatch's scheduler-inspired identity and observed usability needs. Other tools are references, not specifications.

Every proposed addition must identify its category and the concrete need it serves:

| Category | Ownership and admission rule |
|---|---|
| **Core Dispatch need** | Improves standalone software allocation/delivery or its human experience. Dispatch owns its semantics and policy. |
| **Generic interoperability need** | Exposes existing Dispatch capabilities to independent clients through a bounded, versioned interface. Demonstrate it without a particular host installed. |
| **Host-specific accommodation** | Adapts a named host's vocabulary, reporting or connection requirements. Keep it isolated, optional and removable, with separate tests and no new core policy. |

Split proposals that mix these categories so a host accommodation cannot become a core requirement. External lifecycle vocabularies are mapped only at the adapter boundary; a lossy host projection must never redefine or reduce Dispatch's semantic state or become input to its scheduler. Do not expand into pane management or a new orchestration framework.

This clarification updates Phase 5 priority and acceptance criteria. It does not start Phase 5 implementation or reopen completed Phase 0–4 execution semantics. The original dated implementation audit and historical sequencing notes below remain historical context.

## Approved Phase 8 sequencing adjustment — September 18, 2026

The founder authorized implementation of the bounded, opt-in planning experiment
before twenty comparable Phase 7 reviews or demonstrated savings. This changes the
implementation sequence only. Phase 7 screening thresholds and live private policy
remain unchanged. Evidence is still required before making planning the default or
claiming improved economics. The implemented v1 uses one planner, at most four
sequential tasks, and one extra invocation shared across repair and continuation,
with an absolute ceiling of six and one original deadline. See
[the implementation guide](planning.md), [validation](phase8-validation.md), and
[functional freeze/design handoff](phase8-design-handoff.md). Historical gates below
remain relevant to empirical claims and default changes.

## A. Product thesis

Dispatch should become the local allocation layer between a software goal and the coding resources available to accomplish it. Its first promise is concrete: **get more accepted, verified software work from the subscriptions a developer already owns**. The developer states an outcome; Dispatch chooses a suitable harness and model configuration, executes in an isolated candidate workspace, verifies the result, and makes review and acceptance straightforward.

Subscription capacity is a strong initial wedge because it is already paid for, finite, unevenly scarce, and difficult to allocate deliberately across a working day. Choosing the strongest model for everything wastes opportunities when a lighter resource can finish the same bounded task. Choosing a weak resource indiscriminately can waste even more capacity through failed attempts, repeated context, and human repair. Dispatch should allocate conservatively between those extremes and expose a short reason for its choice.

The product objective is accepted outcomes under capacity constraints, with verification, review effort, and latency as explicit guardrails. It is not token minimization, maximum model activity, or an invented quality score. A successful process is not a verified change; a verified change is not a human-accepted outcome. Those remain separate facts throughout the product.

The first portfolio can contain one ChatGPT subscription, one Codex installation, and several selectable model/effort configurations. The same representation can later contain Claude, API-funded resources, local models, and other runtimes. Provider identity, harness identity, model identity, funding source, and execution location must therefore be distinct from the beginning, without building infrastructure for all of them now.

The original thesis survives: “OpenRouter routes prompts. Dispatch routes software work.” Public evidence supplies limited priors; deterministic task characteristics constrain their relevance; private human outcomes eventually improve the policy; current availability constrains execution. Normal operation remains local. Live benchmark retrieval and Cloud are unnecessary.

Dispatch should not become a terminal multiplexer, IDE, chat client for every model, autonomous company, benchmark dashboard, or infrastructure control plane. Optional terminal hosts and editors may own session layout and editing. Dispatch owns allocation, verified delivery and the complete human intent/clarification/review/apply loop for one software outcome at a time; no external host is required.

**Recommendation: BUILD WITH CHANGES.** Establish truthful outcomes, resource identity, and a usable single-task loop first. Gate automatic decomposition and claims of capacity savings on actual dogfood evidence.

## B. Current-state audit

### Audit basis and current journey

The inspected checkout is Dispatch **0.1.2**, commit `81368cf941be1181a025d157766cf22377933fb8` (September 8, 2026). Research reflects primary documentation and live pages inspected September 9–10, 2026. Model catalogs, subscription rules, and terminal crate releases are moving targets; versions below identify the inspected state, not permanent product constants.

The repository was clean before this planning work. The current CLI was built and exercised with a deterministic local Codex fixture in a temporary plain directory, covering no arguments, help, routed execution, status, diff, accept, and failed verification. No paid agent calls were needed. Standard validation passed: `cargo fmt --check`, `cargo test --locked` (**183 tests**), and `cargo clippy --locked --all-targets -- -D warnings`.

Observed behavior:

1. Bare `dispatch` exits **2** because a subcommand is required. Its error exposes even the hidden advanced commands.
2. `dispatch run "…"` selects a single eligible harness, asks for unsafe-local acknowledgement when necessary, creates an internal baseline, runs an independent candidate, and prints review instructions.
3. Ordinary `status`, `diff`, `accept`, `reject`, and `explain` resolve the current source without asking for a run ID. Preserve this improvement.
4. The run still exposes `RUN <id>`, baseline hashes, candidate labels, and benchmark counts before the outcome. This information belongs in details.
5. A fixture with failing configured checks printed `Done`, reported `Verification FAIL`, and exited **0**. Internally verification remains separate, but completion language and exit behavior are unsuitable for a first-class worker.
6. Acceptance safely applied the candidate to the temporary original source. The existing safe-apply path is the correct implementation foundation.

The runtime is already Rust/Tokio, not a disposable prototype. There are 17 Rust source files and about **10,603 production physical lines** before trailing unit-test modules, including comments, blank lines, and embedded SQL; this is not a code-only LOC count. Production orchestration alone is approximately 2,246 physical lines. Minimize conceptual additions and extract presentation incrementally.

### Subsystem disposition

| Subsystem | Decision | Continuity and required change |
|---|---|---|
| Rust CLI/library and orchestration | **KEEP / EXTEND** | Retain ownership of the full local work lifecycle. Extract UI output from orchestration as transitions are instrumented. [Entry point](/Users/jese/bin/dispatch/src/main.rs:313), [run path](/Users/jese/bin/dispatch/src/orchestrator.rs:543). |
| Snapshots, ordinary directories, Git, linked worktrees | **KEEP** | Preserve dirty/untracked content, modes, symlink rules, independent candidate state, and original-source safety. Add lineage for attempts later. [Source](/Users/jese/bin/dispatch/src/source.rs:64). |
| Diff/artifact capture and explicit apply | **KEEP** | Reuse fingerprint checks, full-patch validation, source/run locks, and safe apply. Acceptance and successful application remain independent. [Apply](/Users/jese/bin/dispatch/src/source.rs:377). |
| Generic executor and cancellation | **KEEP / EXTEND** | Preserve process-group cleanup, timeouts, bounded output, environment rules, and Docker behavior. Add incremental output observation without another spawn path. [Executor](/Users/jese/bin/dispatch/src/executor.rs:218). |
| Codex, Claude, Cursor adapters | **KEEP / EXTEND** | Add per-attempt model/effort controls, capability discovery, structured failure normalization, and usage semantics inside the adapters. [Adapter contract](/Users/jese/bin/dispatch/src/harness.rs:136). |
| One model per harness configuration | **EXTEND** | Existing explicit `harnesses.codex.model` becomes a fixed selection constraint. Resource selection must not rewrite project configuration. [Configuration](/Users/jese/bin/dispatch/src/config.rs:50). |
| Recursive generic telemetry interpretation | **REPLACE incrementally** | Keep captured raw structured output. Replace max/recursive guesses with adapter-specific interpretation; preserve historical token semantics. [Parser](/Users/jese/bin/dispatch/src/harness.rs:558). |
| Requested model overwriting observed identity | **REPLACE representation** | Store requested, resolved, and observed model separately. The current precedence can hide substitution. [Result construction](/Users/jese/bin/dispatch/src/harness.rs:200). |
| Deterministic task classification | **KEEP / EXTEND** | Preserve conservative language/kind/scope and unknown states. Add explicit ambiguity, requirement, and verification inputs with evidence, not confident difficulty claims. [Classifier](/Users/jese/bin/dispatch/src/classifier.rs:31). |
| Offline normalized public priors | **KEEP / HIDE** | Preserve provenance, bundled fallback, optional refresh, validation, and compatibility/specificity. Move benchmark details out of the default journey. [Priors](/Users/jese/bin/dispatch/src/public_priors.rs:178). |
| Harness-only router | **EXTEND** | Rank executable resource choices. Retain evidence matching, but match model/configuration before using model-specific outcomes. [Router](/Users/jese/bin/dispatch/src/router.rs:23). |
| Stable fallback and deliberate override | **KEEP / EXTEND** | An override still enters the same execution core. Add explicit model selection and honest policy-based reasons. [Selection](/Users/jese/bin/dispatch/src/orchestrator.rs:335). |
| Baseline checks and verification | **KEEP / EXTEND** | Emit live check transitions; distinguish task-target checks, preexisting failures, missing checks, and verification failure. [Checks](/Users/jese/bin/dispatch/src/executor.rs:935). |
| Human evaluation and local observations | **KEEP / EXTEND** | Preserve single-result acceptance versus blind preference. Add attempt identity and local feedback history without manufacturing subtask labels. [Observations](/Users/jese/bin/dispatch/src/models.rs:132). |
| Local evidence inspection | **KEEP; add separate learning policy later** | Existing counts intentionally cannot change routing and do not group by model. Do not silently redefine this query. [Evidence](/Users/jese/bin/dispatch/src/evidence.rs:14), [regression boundary](/Users/jese/bin/dispatch/tests/local_evidence.rs:498). |
| SQLite, migrations, artifact files | **KEEP / EXTEND** | Add small local tables as needed. Keep large logs/patches in files. Make durable transitions authoritative before publishing them. [Migrations](/Users/jese/bin/dispatch/src/db.rs:20). |
| Coarse persisted events | **EXTEND** | Add version, sequence, typed payloads, attempt identity, and live publication. Existing event files remain readable. [Event record](/Users/jese/bin/dispatch/src/models.rs:369). |
| Printing and prompting inside orchestrator | **REPLACE incrementally** | Render semantic state through human and machine presenters. Do not refactor unrelated code. [Summary](/Users/jese/bin/dispatch/src/orchestrator.rs:1086). |
| Advanced multi-harness comparison | **KEEP / HIDE** | Preserve independent alternatives, random blind labels, Tie/Neither, and evaluations. Comparison candidates are not subtasks or retries. |
| Cloud consent, preview, outbox, feedback sync | **KEEP / HIDE** | Freeze existing wire contracts. Resource pools, plans, capacity, and new allocation observations stay local. [Sync](/Users/jese/bin/dispatch/src/sync.rs:29). |
| Local and experimental Docker backends | **KEEP** | This phase adds no remote execution backend, Go service, or Cloud dependency. |
| Bare-command error and log-led presentation | **REPLACE** | Intent first, then a compact semantic graph and review. |

No working execution subsystem needs wholesale deprecation. Deprecate the **assumption** that a harness is the routing unit, and the overloaded use of “done,” rather than deleting the original investments.

### What routing and persistence actually know

`TaskFeatures` currently contains language, task kind, and scope. Classification uses filenames/extensions and narrow task-text rules. Scope normally remains unknown without explicit known file references. A small resulting diff is an after-execution observation, not a pre-execution feature. Configured checks are not equivalent to demonstrated task coverage.

The router chooses one compatible public row per harness using specificity, sample count, and stable provenance; it does not sum unrelated datasets. Automatic evidence-based selection requires at least two eligible harnesses with compatible nonzero evidence. Otherwise the supported order is Claude → Codex → Cursor. The bundled snapshot contains **one generic Codex row**, associated with `openai/gpt-5.6-sol`, with 123/330 benchmark successes. This does not compare Codex tiers or establish a success probability for the founder’s work. [Snapshot](/Users/jese/bin/dispatch/public-priors/public-priors-v1.json:1), [selection](/Users/jese/bin/dispatch/src/orchestrator.rs:335).

Current migration versions are:

| Version | Existing purpose |
|---|---|
| 1 | Sources, runs, candidates, checks, artifacts, events, evaluations, reasons |
| 2–3 | Environment provenance and token semantics |
| 4–5 | Explicit sync consent/outbox/stable IDs and separate ingestion token |
| 6–8 | Benchmark priors, run routing decision, one routing observation per run |
| 9–10 | Versioned consent, typed outbox, immutable uploaded-parent feedback revisions |
| 11 | Manual/distributed prior origins and installed snapshot identity |

Three constraints shape the next design. `routing_observations` is unique per run and assumes exactly one terminal candidate. Current local feedback can overwrite the latest state; append-only revisions are created for already-synced parents, not universally. Metadata JSON, SQLite state, and event JSONL are separate writes. A retry graph cannot be added by stuffing more candidates into the existing comparison list or broadcasting the current print statements. [Observation persistence](/Users/jese/bin/dispatch/src/db.rs:1567), [feedback](/Users/jese/bin/dispatch/src/db.rs:1658), [event persistence](/Users/jese/bin/dispatch/src/orchestrator.rs:1251).

### Website and existing assets

The live site still presents v0.1 multi-agent comparison and says automatic routing is unavailable; the checkout already ships default single-agent routing. Treat this as a copy/version alignment issue. Carry forward the visual system, not outdated product instructions. No website source or additional design-asset package was found in this checkout; the additional `dispatchv0` workspace path was absent. The live CSS and original SVG were inspected directly in the browser. [Live site](https://rundispatch.sh/).[^site]

Exact useful visual tokens:

| Role | Observed site value | Terminal translation |
|---|---|---|
| Page/surface | `#070B10`; secondary `#080C11`; mark surface `#0A1118` | Leave terminal background untouched by default; use these only in an explicit dark theme |
| Primary action/mark | `#67E8F9`; hover `#A5F3FC` | Cyan active route, current node, and focused action |
| Foreground | White; button text `#F8FAFC`; muted copy `#94A3B8` | Default terminal foreground, bold emphasis, readable secondary text |
| Fine boundaries | White at 7%; controls 13%; mark cyan at 26% | Sparse dim edges, no surrounding dashboard boxes |
| Success/wait accents | CSS includes emerald `#5EE9B5`, amber `#FFD236` | Small status accents plus explicit words; failure needs a new accessible red semantic token |
| Typography | Geist Sans; Geist Mono for commands/metadata; tight large headings, tracked small labels | Use the developer’s terminal font; do not require a font install or emulate huge headings |
| Rhythm | 4px spacing base, 64px faint hero grid, broad 96–160px section spacing, shell capped at 1320px | Blank lines and small fixed gaps, few nodes visible, no grid wallpaper |

The actual SVG is a **filled inlet splitting through two rounded cyan paths to two hollow outlets**. It has three circles, not the six-node diamond in the conceptual prompt. Its accessible description mentions convergence, but the paths do not draw a merge. Use the actual fork geometry for the opening mark; introduce merges only when work actually integrates. The page’s graph language is strongest in this mark and fine route accents, not an existing full execution graph. [Original mark](https://rundispatch.sh/brand/dispatch-mark.svg).[^mark]

## C. Core domain model

### Minimal identities, without an enterprise resource platform

Keep `RunRecord` as the durable root of one requested outcome. In the normal product, that is the **goal**; retain the existing `run` command and IDs for scripts. Do not introduce a second competing Goal persistence system. A comparison run remains an explicit advanced mode.

An **attempt** is one Dispatch-supervised harness invocation. A harness can make multiple model requests, spawn its own subagents, or switch models internally. Attempt/concurrency limits do not bound those internal calls unless supported harness controls enforce that separately. Measured profiles should disable internal orchestration where supported, or explicitly retain unknown/mixed internal composition.

| Concept | Minimal representation | Important boundary |
|---|---|---|
| Goal | Existing run plus mode, policy version, priority, total deadline/attempt limits, result pointer, outcome state | Root task text remains verbatim; one human acceptance decision for the delivered outcome |
| Task | Optional child record: objective, acceptance criteria, dependencies, allowed scope, baseline/input reference | Add only with decomposition; a simple run needs no task graph |
| Plan | Versioned artifact containing bounded tasks, dependencies, expected checks, scope and integration order; provenance points to planning attempt | A model proposes a plan; Dispatch validates its structure; it is not a quality judgment |
| Provider | Stable string, e.g. `openai`, `anthropic` | Does not imply a subscription, authentication method, or harness |
| Funding source | Local ID plus kind `subscription`, `api_credit`, `local_compute`; optional plan label and user limits | Credentials stay in the harness’s supported authentication store |
| Subscription | Funding-source metadata | Avoid separate account/team/billing tables; never infer entitlement from a marketing plan name alone |
| Resource pool | Local ID, funding source, scope, capacity observations and shared constraints | Several models and harnesses can draw from the same pool |
| Resource choice | Provider + funding source + harness + requested/resolved model + effort + service mode + runtime + pool references | This is the routing unit; model and effort are separate controls |
| Tier | Local policy role such as light, standard, strong, associated with validated resource choices | Relative role, not a universal cross-provider quality scale or permanent model name |
| Runtime | Current local backend identity and harness version; existing Docker descriptor where explicitly used | A field supports future location distinctions; no distributed scheduler now |
| Capacity | Timestamped, sourced observation of one or more constraints; known/estimated/unknown values | Token counters, money, rate-limit percent and machine slots retain separate units |
| Decision | Immutable feature snapshot, eligible alternatives/exclusions, evidence references, capacity snapshot, selected choice, fallback path and policy version | Explanation must be reproducible without running a model |
| Attempt | ID, run, optional task, role, ordinal, resource, input baseline, parent attempt, timing, outcome and artifacts | Each actual planner/executor/repair invocation is observable and charged against the same goal policy |
| Retry/escalation | A new attempt linked by a typed reason; escalation also changes capability/configuration | Never overwrite a failed attempt; never imply every retry is stronger |
| Verification | Existing check results extended with input snapshot, command identity, baseline/target relationship and phase | A command result is mechanical evidence |
| Observation | Immutable attempt facts plus eventual contribution/goal outcome references | Requested/observed model mismatch and unknown costs survive |
| Human feedback | Append-only local revision for the reviewed artifact and scope; accepted/rejected/deferred, optional reasons | Unreviewed is not rejected; goal acceptance does not label every subtask or failed attempt accepted |

### Execution continuity

Retain one reviewable `CandidateRecord` for a normal goal. Add **attempt records beneath the run**, rather than manufacturing comparison candidates for retries. Each attempt has its own isolated workspace and retained artifacts; the final candidate points to the selected final workspace/diff. Factor the existing candidate procedure only enough to separate preparation, one harness invocation, verification, and final candidate materialization. Every invocation still goes through the existing adapter and `Executor`.

On the first attempt, input is the frozen goal baseline. A recovery attempt can read the previous failed patch and bounded diagnostics, but writes into a fresh workspace. If it starts from a previous attempt’s content, persist that exact derived baseline and parent link. The final reviewed diff is always relative to the original goal baseline. Do not mutate an earlier attempt’s evidence.

With decomposition, child tasks produce artifacts against explicit input snapshots. Integrate them deterministically into a separate goal candidate, then run root verification and review once. Start with sequential integration. Independent branches may later run concurrently only when their scopes and dependencies permit it. Comparison mode keeps its original independent same-task alternatives and blind evaluations.

The final candidate is a **delivery artifact**, not necessarily one model’s output. Attach contributing attempt references; leave single-author model/harness fields unset or explicitly mixed when provenance is mixed. Sum costs only when they share units and interpretation, and preserve the original measurements. A final stronger attempt does not acquire authorship or usage attribution for all earlier work.

### Outcome state

Avoid a giant mutually exclusive enum trying to encode every fact. Expose these independent fields:

```text
lifecycle:    preparing | working | waiting | finished
work_result:  pending | ready | failed | cancelled | interrupted
verification: not_configured | not_run | passed | failed | inconclusive
review:       not_requested | pending | accepted | rejected | deferred
application:  not_applied | applied | blocked_by_source_drift | failed
waiting_on:   none | human | capacity | dependency | authorization | admission | reconciliation
```

The phase provides detail: analyzing, planning, executing, verifying, or integrating. “Ready for review” means a candidate exists; “verified” is shown only for checks that passed; “accepted and applied” is the normal successful goal conclusion. A human may accept a change whose checks are inconclusive, but it does not count as accepted-and-verified work. Section O refines task readiness, aggregate waiting and identified questions without replacing these outcome fields.

### Persistence strategy

Add local execution-detail records first, then `resource_pools`, `capacity_observations`, and `attempts` as their phases arrive. Store variable provider data in versioned JSON columns; use relational IDs, foreign keys, uniqueness, and state columns for lifecycle invariants. Add `tasks` and plan references only at the decomposition gate. Provider and tier catalogs can remain small structs/configuration snapshots rather than separate normalized tables.

For new transitions, use SQLite as the authoritative state and low-volume event journal: commit the state mutation and sequenced event in one transaction, then publish. Metadata JSON and event JSONL are projections that can be repaired from committed state. This is a small durable transition path, not event-sourcing the entire application. Bulk harness output stays in bounded artifact files with truncation information.

Retain old IDs, old run readers, and the v1 sync schemas. New allocation decisions do not fit the existing benchmark-centric `RoutingDecision` wire shape. Give them a local versioned shape instead of forcing fake dataset fields into it. New allocation/attempt/plan records are **local-only** until a separate sync product/consent decision; legacy eligible observations and their existing outbox keep working.

Enforce that exclusion at the persistence/export boundary: allocation-mode runs must not populate legacy `run.routing` or trigger `sync_routing_observation`, even when they have one final candidate. Otherwise today’s one-candidate condition could export a multi-attempt result as one harness outcome. Keep a distinct local allocation-decision field and explicit run mode.

That change must ship together with allocation-aware `status`, `explain`, `accept` and `reject`. Today acceptance requires a legacy routing observation. New allocation runs need local goal feedback independent of that observation before they can become an executable product path. Preserve the existing source locks and apply mechanics; dispatch review by explicit run mode rather than pretending new allocation decisions are old benchmark decisions. [Current review dependency](/Users/jese/bin/dispatch/src/orchestrator.rs:1641).

## D. Routing and scheduling model

### First portfolio: one included ChatGPT allowance, several Codex choices

An initial portfolio might resolve to:

```text
funding: founder-chatgpt / subscription
  pool: codex-included
    constraints: provider-reported windows (possibly more than two)
    choices:
      Codex + included light model + supported effort
      Codex + included standard model + supported effort
      Codex + strongest included model + supported effort
```

These choices share allowance wherever provider evidence says they do. Never give each model a fictional independent tank. Current official documentation lists Plus at $20/month and describes a lighter model option with higher usage; it does not establish a fixed per-task capacity conversion. The model catalog actually available to the account must determine the resolved choices. “Strong” means strongest eligible included choice, not an assumption that the account has access to every frontier model. [Codex pricing](https://learn.chatgpt.com/docs/pricing).[^pricing]

### Deterministic policy v1

Use an ordered decision procedure, not a weighted quality score:

1. **Apply hard constraints.** Resolve source/backend eligibility, available harness, supported model/effort, funding permission, task tool requirements, deadline, and known exhausted windows. Unknown entitlement is not validated inclusion. Deliberate `--agent`/`--model` constraints win unless unavailable; report failure rather than silently substituting.
2. **Determine verification readiness.** Record configured checks, baseline results, and any task-specific reproducer/acceptance check. Merely detecting a test framework is weak evidence. Baseline failures must be considered before spending another model call.
3. **Select an action class.** Use explicit task scope, requirements and conservative rules: execute a bounded task; ask one essential clarification; or propose a bounded plan. Unknown scope is not automatically “easy” or automatically a frontier call.
4. **Choose a minimum suitable lane.** A low-risk, precisely scoped task with relevant executable checks may use light intelligence. Ordinary implementation with incomplete coverage uses standard. Architectural decisions, cross-cutting changes, consequential data changes, or hard debugging justify strong intelligence or clarification. These are policy judgments recorded as such, not calibrated probabilities.
5. **Apply scarcity constraints.** Use known window state, current reservations, priority, reset horizon, and observed relative consumption. Scarcity may choose another eligible resource, serialize work, or defer it; it must not silently lower the required acceptance criteria or choose an unsuitable model.
6. **Use comparable evidence within eligible choices.** Prefer relevant human outcome evidence when sufficient; otherwise compatible public model evidence, then the configured conservative default. Harness-only public evidence can inform a harness fallback but cannot rank that harness’s tiers. Unknown evidence stays unknown.
7. **Persist the decision before execution.** Include a bounded fallback, its trigger, total attempt/deadline limits, and reason codes. The renderer turns those into one sentence.

Example reasons:

```text
Using light: this is a bounded change with a relevant regression check.
Using standard: the scope is clear, but verification coverage is incomplete.
Using strong: this changes a shared interface across the project.
Waiting: the required resource is unavailable until its allowance resets.
```

For the first controlled dogfood version, “bounded” should require an explicit area/file or a user-confirmed small task, testable acceptance, and no cross-cutting risk flag. The current classifier alone does not establish all of those. Do not add another model call merely to classify every task. A concise clarification is often cheaper and more useful.

Concretely, automatic light eligibility is the conjunction of `scope = explicit_known_paths`, `acceptance_check = user_or_project_selected_check_id`, `risk_class = routine` from an explicit task/project policy, and a baseline result compatible with the intended task. Record the source of each field. If scope, risk or check relevance is unknown, use standard or ask the essential clarification; do not infer routine risk from the absence of a keyword. The founder can approve a small set of routine task/check mappings during dogfood. Natural-language extraction may propose such a mapping, but cannot turn an unconfirmed guess into a strong verification claim.

### Scarcity and reset policy

The first policy needs ordinal states, not a purported number of tasks remaining. Proposed starting defaults, explicitly subject to dogfood revision:

| State | Rule | Effect |
|---|---|---|
| Available | Fresh applicable windows have at least 50% remaining | Normal suitable-lane policy |
| Constrained | Any applicable window has 20–50% remaining | Serialize model work; avoid optional planning and speculative retries |
| Reserve | Any applicable window has less than 20% remaining | Preserve expensive calls for urgent work or a pre-authorized recovery; offer deferral for optional work |
| Exhausted | Provider reports rejection/exhaustion | No new calls to that constrained resource until a fresh availability signal |
| Unknown/stale | Missing, stale, conflicting, or unmapped data | Conservative lane policy, one active Dispatch attempt per pool, bounded attempts; no percentage display |

These thresholds are routing policy, not measurements of a provider’s true remaining compute. A reset within 30 minutes may make deferral useful; it does not justify generating extra work to “use up” capacity. A short-window reset cannot erase a separate weekly constraint. Never assume availability at the predicted reset without refreshing or labeling the inference.

Default one model attempt at a time **across Dispatch processes sharing a local subscription pool**. Use a small SQLite lease/reservation, acquired transactionally with owner process identity and bounded liveness. It is a concurrency reservation, not a fabricated numeric capacity debit. Clean cancellation releases it; crash recovery must distinguish a live child from a stale owner. This local coordination matters whenever several foreground Dispatch sessions share a pool, regardless of terminal or caller. Other clients and other machines can still consume the same allowance; represent that uncertainty.

No global distributed scheduler is required. If an external tool places work on remote hosts, Dispatch on each host still makes local decisions. Remote observations can become another input later, but local leases must never be described as an account-wide hard quota guarantee.

### ChatGPT plus Claude, without a rewrite

Add another funding source, independent pools/windows, and Claude resource choices. The same filter/lane/scarcity/choice procedure applies. A constrained Codex pool can lead to an eligible Claude configuration if the task’s requirements and spending permissions allow it. The fallback receives the same frozen goal context, previous patch and check diagnostics through the shared executor; it does not inherit a provider-native conversation by assumption.

Do not compare 30% remaining in one provider to 30% in another as equal compute. Compare feasible choices and relative scarcity within each pool. Cross-provider tradeoffs use observed accepted outcomes, supported capabilities, latency, and the user’s preferences. There is no global “intelligence credit” conversion.

### Learning after the first useful policy

Preserve the distinction between public benchmark completion, local configured-check passage, and local human acceptance. Group new observations by task morphology, verification regime, model/effort, harness version, source context and policy version. Record which resources were eligible and why one was selected. Keep first attempts, recoveries, and inherited-context attempts separate.

Begin with descriptive counts and a shadow policy that does not affect routing. A candidate gate for using local evidence is at least 20 reviewed comparable goals for a choice and adequate observations of the relevant task class; this is a minimum for inspection, not statistical calibration. Missing reviews remain missing. Large uncertainty or a changed model/version falls back to the conservative policy.

Per-resource acceptance counts initially include only attributable single-attempt goals. Multi-attempt and decomposed goal acceptance evaluates the complete allocation policy; it does not establish the final model’s standalone success rate. Keep mechanical subtask outcomes visible without promoting them to human preference labels.

Initial policy updates should be small and inspectable—for example, stop selecting light for a task class with repeated human rejections, then evaluate that change prospectively. Never promote a tier because an LLM praised its code. Do not claim the observed difference between two selected populations is the causal benefit of upgrading. ML, exploration algorithms, and calibrated success probabilities require a later evidence-based decision.

## E. Planner and executor strategy

Planning must earn its capacity. There are three modes:

| Mode | Trigger | Work performed | Initial bound |
|---|---|---|---|
| No separate planning | One clear outcome, bounded scope, explicit acceptance/checks | Pass task directly to the selected executor; its ordinary code inspection still occurs | Zero separate planner attempts |
| Lightweight planning | A few implementation steps but one coherent change; requirements understandable | The same execution call begins by outlining scope and tests, then implements; deterministic preflight can structure known facts | No mandatory separate frontier call |
| Full decomposition | Several independently verifiable outputs, clear integration boundary, substantial reusable context, and enough capacity | One capable-model planning attempt produces a validated bounded task plan | One planner attempt; initially at most four implementation tasks; no recursive decomposition |

If requirements are missing, ask for the missing decision rather than buying an elaborate plan around an assumption. If a goal is tightly coupled, a single strong execution can be more economical than decomposition. Planner output cannot declare its own success probability or become an outcome label.

The economic decision is conceptually:

```text
planning burden + child work + repeated context + integration + recovery
    versus
one suitable direct execution + its likely recovery burden
```

Initially the terms are mostly ordinal/unknown. Do not calculate a fake expected-value number. Allow full decomposition only for tasks that visibly meet the structural conditions, and record all planner/context/integration overhead. Make it an explicit opt-in experiment before a default policy branch.

A task specification should include objective, relevant source areas, constraints, acceptance criteria, applicable check identifiers, dependencies, and input artifacts. It should not carry the whole parent transcript. Keep a shared concise goal brief plus exact source references and dependency artifacts. Never silently discard essential constraints to reduce context.

After planning, precise independent edits with target checks may use light resources; uncertain integration or shared interfaces use standard/strong. The planner may suggest scope and dependencies; the deterministic allocator makes the executable resource choice. Planning itself uses the same attempt records, supervisor, limits and telemetry as implementation.

First decomposition executes tasks sequentially and integrates after each verified contribution. Parallel branches are a later optimization, gated by measured latency benefit and conflict cost. A failed integration does not start an unbounded sequence of replans. Stop at the goal’s total attempt/deadline limit and retain the partial result.

## F. Verification and escalation policy

### Verification contract

Keep configured commands authoritative. Detecting a manifest can suggest a command, but do not silently execute new untrusted project scripts beyond the existing acknowledgement/configuration contract. Record command text/hash, configuration snapshot, source baseline, status, duration and artifacts.

Verification readiness has four honest levels: none configured; general checks configured; baseline checks working; task-specific acceptance/reproduction check available. These describe evidence strength, not guaranteed coverage. A test generated by the same model can add evidence, but passing it alone does not establish the requested behavior. If an attempt modifies verification definitions or removes relevant tests, show that fact and require human review; rerun the original configured contract where applicable.

Run cheap mechanical checks first, then the relevant regression check, then the configured full suite as required. A targeted pass does not replace required project validation. Record skipped/not-run checks explicitly. Compare baseline and candidate outcomes so an existing environment failure does not automatically trigger model escalation. A repair goal may intentionally start with a failing target test: identify that test and distinguish the expected failure from unrelated failures.

### Bounded recovery

For a simple goal, the initial automatic policy is **one attempt plus at most one recovery**. Both share a total wall-clock deadline and capacity authorization. No same-tier loop; no alternating providers indefinitely; no automatic follow-up after human rejection.

| Outcome | Next action |
|---|---|
| Process and configured verification pass | Stop model work; offer human review |
| No checks configured | Present unverified result; do not invent PASS or auto-accept |
| Clear implementation defect from a working target check | One stronger recovery if pre-authorized, suitable and available |
| Existing infrastructure/baseline failure unrelated to task | Block or present inconclusive verification; no automatic stronger call |
| Harness authentication, missing executable, unsupported model | Mark resource unavailable; do not use intelligence to fix configuration |
| Capacity rejection | Refresh availability; stop or choose an already-authorized eligible alternative; no busy retry |
| Timeout/crash | Preserve partial artifacts; stop by default unless a classified recoverable failure fits the same single recovery allowance |
| Strongest attempt fails, or recovery fails | Finish failed with evidence and a concrete next action |
| Human rejects a verified candidate | Record rejection and optional reason; further work requires a new instruction |

Capability escalation is not always the next adjacent tier. Choose the smallest *suitable* recovery configuration for the observed failure, possibly going directly from light to strong. Do not try every catalog entry.

Recovery receives the original goal, immutable prior attempt reference, exact relevant check failure, and bounded diagnostics. Record whether it inherited the previous patch or restarted from the goal baseline. Never hide the cost of the failed first attempt in the final attempt’s statistics.

The familiar expected-cost test is useful later: a cheap attempt followed by possible recovery makes sense when `C_light + P_recovery × C_recovery < C_direct`, with latency/review constraints satisfied. V1 does not know those terms precisely and should not display a computed saving. It uses strong verification and bounded attempts to make the experiment safe and measurable.

Decomposition must also have a goal-wide cap. Four tasks do not each get an unlimited private retry budget. A starting experiment permits one planner attempt, at most four task attempts, and at most one additional recovery/integration attempt across the whole goal; changing the bound is an explicit policy revision. These bound Dispatch invocations, not unobservable internal model requests.

## G. Capacity model

### What can be measured supportably

| Signal | Codex / ChatGPT | Claude Code / Claude | Decision |
|---|---|---|---|
| Select a model | `codex exec --model`; installed CLI 0.153.4 confirms it | Documented `claude -p --model` | Use adapter-owned explicit controls |
| Select effort | Config override; supported choices in model catalog | Documented effort controls; support varies by model/account | Store separately from model and service tier |
| Discover available models | Documented app-server `model/list` | Agent SDK model-discovery interface | Optional capability discovery; entitlement/funding still need validation |
| Account quota snapshot before work | Documented app-server `account/rateLimits/read` | No complete supported preflight endpoint established | Codex probe optional; Claude unknown is valid |
| Updated quota observations | `account/rateLimits/updated` | SDK `rate_limit_event`; statusLine JSON windows | Normalize only fields actually delivered by the installed path |
| Exact next-task allowance use | Not established | Not established | Never promise it |
| Token usage | Structured execution output | Structured result/model usage | Preserve each harness’s semantics; not a subscription balance |
| Reset horizons | Nullable per-window timestamp/duration | Optional in observed windows/events | Missing reset remains unknown |

Codex’s documented multi-bucket response includes opaque limit IDs and windows with `usedPercent`, nullable duration and reset time. The app-server command remains experimental; it is an optional, version-tested metadata probe, not the foundation of execution. Keep `codex exec` as the supported worker path. The generated local schema was inspected successfully; a metadata-only `model/list` experiment did not produce a usable response, so runtime discovery is **not yet verified**. No live account balance was queried. [App-server protocol](https://learn.chatgpt.com/docs/app-server), [noninteractive execution](https://learn.chatgpt.com/docs/non-interactive-mode).[^appserver][^exec]

Claude’s documented statusLine interface can receive five-hour/seven-day quota data after an API response; absent windows are not zero usage. Its delivery in Dispatch’s headless path was not established. Do not overwrite the developer’s status line or add a PTY to harvest it. SDK rate-limit notifications are a promising structured in-session source, but test their exact delivery and units in `stream-json` before relying on them. Claude was not installed in the inspected environment. [Status line](https://code.claude.com/docs/en/statusline), [official SDK type reference](https://code.claude.com/docs/it/agent-sdk/typescript#sdkratelimitevent).[^claude-status][^claude-sdk]

Billing eligibility is separate from capability. Current Claude documentation contains a paused change to subscription treatment of Agent SDK/print-mode usage, and model/speed choices that can require extra credits. In headless operation some paid choices may proceed without the interactive prompt. Initial subscription profiles must resolve to known included, standard-speed configurations; an API key, model alias, or saved fast-mode preference must not cause an implicit paid fallback. Recheck this contract before shipping Claude support. [Subscription update](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan), [model configuration](https://code.claude.com/docs/en/model-config), [fast mode](https://code.claude.com/docs/en/fast-mode).[^claude-billing][^claude-models][^claude-fast]

An included model and a positive preflight allowance **do not guarantee zero cash charges** if the provider can consume already-authorized overage credits during the attempt. Subscription-only eligibility needs a supported per-invocation no-overage control, or a verified account configuration with no chargeable overage path. That guarantee has not been established for this account. If neither can be established, exclude the profile from a strict included-only policy or require separate bounded paid authorization and label the limitation. Sampling percentages cannot enforce a hard cash cap. Revalidate when account configuration changes.

### Measurement shape

Represent knowledge **per field**, not once for the entire account:

```text
CapacityObservation
  pool_id, provider_bucket_id, applicable_resource_ids or unknown_mapping
  sampled_at, source, source_version, valid_until
  constraints[]
    kind: allowance_window | credit_balance | concurrency | device_limit
    unit: provider_percent | provider_credit | currency | slot | opaque
    remaining: reported(value, precision?) | estimated(range, method, n) | unknown
    reset_at: reported(time) | estimated(range) | unknown
    window_duration: reported(duration) | unknown
    scope: reported_scope | unknown
  raw_observation_ref
```

“Reported” means the provider reported that number at that time. It does not mean exact future usable compute. Preserve raw used percentage even if the display derives `100 − usedPercent`; clamp presentation only, retain unusual values for diagnostics. Unknown data has a reason: unsupported, absent, expired, probe failure, conflicting identity, or unknown pool mapping.

Estimated values need a range or qualitative scarcity state, sample count, method, and expiry. Display “allowance appears constrained” rather than “17%” from a rough model. Keep last-observed values separate from current validity. Initial freshness policy can expire snapshots after five minutes, and always invalidate applicability across a known reset; this is a Dispatch default to test, not a provider guarantee.

All applicable constraints must permit admission. Do not sum window percentages, add ChatGPT and Claude balances, or deduce that a model has its own pool from its name. A provider can add limits or change mapping; unknown constraints must not deserialize as unlimited capacity.

Before/after allowance changes are useful **pool observations**. Attribute a change to an attempt only when observation timing, window identity, external activity, and rounding support that attribution. Otherwise label it mixed activity. Zero observed change may be rounding; a falling used percentage may be reset/adjustment, not negative task cost. Local leases control Dispatch concurrency only; they cannot establish exclusivity against other applications or hosts.

Keep three economic quantities separate: sunk subscription price; observable/estimated allowance consumption; and marginal cash charges. A token-derived API price is not a subscription charge. Do not automatically consume earned resets, buy credits, or switch funding sources when capacity is exhausted.

Unknown capacity still permits useful allocation: task fit, known selectable included profiles, supported qualitative consumption priors, one active attempt, bounded recovery, and human outcome feedback. It does not permit a claim of measured savings. If the account exposes no meaningfully cheaper included route, the wedge is unproven for that account; the product can still deliver verified work, but should not pretend to optimize a nonexistent differential.

## H. Interoperability contract

### One core, several consumers

```text
intent / CLI request
        │
        ▼
allocation → existing executor → verification → review / safe apply
        │               │                │
        └──── committed semantic state and events ────┐
                                                      │
                                ┌─────────────────────┼──────────────┐
                                ▼                     ▼              ▼
                         inline / graph / plain    JSON / JSONL   optional host adapter
```

The terminal must never own scheduling or interpret output text to decide completion. A host observer does not influence allocation. Raw harness output is an artifact, not Dispatch’s machine protocol. Generic clients consume Dispatch's full semantic vocabulary. Any host-specific lifecycle mapping stays inside its optional adapter and cannot replace that vocabulary.

### Invocation and terminal ownership

Recommended interface additions, alongside all existing core commands:

```text
dispatch                                      # interactive session when a TTY
dispatch run "fix the cache bug"               # one outcome, then exit
dispatch run --task-file task.md
dispatch run --task-file - --allow-unsafe-local
dispatch run "fix the cache bug" --model <id>  # explicit selection constraint
dispatch run "fix the cache bug" --json        # one final JSON result
dispatch run "fix the cache bug" --jsonl       # sequenced events and final result
dispatch status [run-id] --json
dispatch events <run-id> --after <sequence>    # replay; --follow for a live run
dispatch resume [run-id]                       # advanced recovery, not normal human flow
dispatch control --stdio                      # Phase 5: scoped machine commands/events
dispatch run "fix the cache bug" --plain
```

Use mutually exclusive `--json` and `--jsonl`; avoid a separate “API mode” execution path. Human presentation controls are `--ui auto|inline|off`, `--color auto|always|never`, and `--glyphs auto|unicode|ascii`. `--plain` is the convenient escape-free, artwork-free human mode. Bare interactive `dispatch` defaults to inline; a TTY `run` defaults to compact graph-streaming; non-TTY defaults to plain. Explicit inline on an unsuitable terminal reports a clear option error; auto mode falls back.

Machine modes never prompt interactively. `--task-file -` consumes stdin as task data to EOF; stdin is not also a control channel. Missing required authorization/input produces a structured blocked result and actionable exit. Never silently read `/dev/tty` behind a pipe. Bare no-argument invocation without a TTY prints a concise intent-oriented usage hint and exits 2; it must not hang waiting for a prompt.

Human output can include the graph, but machine stdout contains only the chosen format. In `--json`, emit exactly one result object; in `--jsonl`, every stdout line is one protocol envelope. Diagnostics go to stderr, without decorative progress in machine modes. No raw child JSON, terminal escapes, credentials, or debug prefixes may corrupt stdout. `dispatch diff` retains its existing human format; a future `--patch` can provide a pure patch if needed, rather than changing it silently.

Dispatch owns the foreground terminal. Harnesses stay on their existing headless pipe-based paths; no nested competing TUI and no required PTY. Restore terminal modes/cursor on normal exit, error, panic and handled signals. Ctrl+C cancels through the existing process supervisor, preserves partial evidence and stops further attempts. Do not detach children on pipe closure or TUI exit. Broken output pipes trigger controlled cancellation for an active one-shot worker; document that a pipe consumer should drain the stream if work must continue.

Bare `dispatch` remains open after review for another goal. This is a foreground intent loop, not a daemon or workspace manager. A terminal host's detach behavior may preserve that process; abrupt host/server loss requires durable recovery, not a promise that the worker kept running. Host persistence is optional and is not part of the generic interface's guarantee.

Accept one goal at a time initially. Input arriving while execution owns the viewport must not silently start another goal or consume paid capacity; pause submission until the ready prompt. A host acknowledging pasted terminal bytes is not Dispatch accepting a goal. An active-session stdin EOF/hangup triggers controlled cancellation and checkpointing; Ctrl+D typed in an idle editor is the separate normal-exit action.

### Protocol envelope and status

Proposed event example, not an existing schema:

```json
{
  "protocol_version": 1,
  "event_id": "<stable-id>",
  "run_id": "<goal-run-id>",
  "sequence": 12,
  "timestamp": "<UTC-time>",
  "type": "attempt.started",
  "task_id": null,
  "attempt_id": "<attempt-id>",
  "payload": {
    "resource_id": "codex-light",
    "role": "implementation",
    "reason": "bounded_task_with_target_check"
  }
}
```

Start with a small typed vocabulary: run accepted/state changed; resource selected/unavailable; attempt started/finished; check started/finished; input required; result ready; review recorded; application finished. Add plan/task transitions only with that capability. Events contain semantic relationships and outcomes, never terminal coordinates. Optional diagnostics can expose excluded choices; the default stream need not emit every scored possibility.

Guarantees: increasing per-run sequence; immutable event IDs; state/event transaction before publication; replay after a sequence; snapshot includes last committed sequence; unknown optional fields tolerated; incompatible changes increment protocol version. Recovery may redeliver events, so consumers deduplicate by event ID. Do not promise exactly-once external delivery. A slow UI must not block child pipe drainage; persist lifecycle events, bound live queues, and recover missed events from the journal. Token-sized progress updates are neither mandatory nor durable lifecycle events.

Final result includes run ID, independent outcome fields, requested/resolved/observed resource identity, attempt chain, check results, change summary, local artifact references, capacity observations with knowledge state, and next action. Public event payloads default to minimal summaries; detailed local status/artifacts may contain paths and task text. This local integration format is separate from Cloud upload schemas and does not authorize transmission.

Recommended one-shot exit contract:

| Exit | Meaning |
|---|---|
| 0 | Requested execution policy completed and result is ready; configured required checks passed if present; JSON still explicitly says whether verification was configured and review is pending |
| 1 | Execution/internal failure; fine reason in result |
| 2 | Invalid arguments/configuration or missing task |
| 3 | Required verification failed |
| 4 | Required human input/authorization; checkpoint or actionable preflight state |
| 5 | Required resource unavailable/capacity deferred |
| 124 | Goal deadline/timeout |
| 130 / 143 | Cancelled by SIGINT / terminated by SIGTERM, where available |

This deliberately changes misleading v0.1.2 exits; release-note and test it as a versioned behavior change. Exit 0 never means accepted or applied. Optional `--require-verification` can make absent/inconclusive checks an unsuccessful policy result, with an explicit reason rather than fake failure counts. Interactive session exit describes session shutdown, while each goal’s status remains durable; it cannot summarize every historical goal with one shell exit code.

A blocked one-shot headless goal checkpoints and exits rather than holding a hidden prompt forever. The live human session instead presents the identified question and continues automatically after an authorized answer; ordinary work never requires a `resume` command. Both paths validate question revision, source lineage and remaining policy before another attempt. This does not promise provider conversation continuity or replay an interrupted paid attempt. Multiple unresolved runs must be disambiguated. Section O specifies the shared command boundary and a dedicated bidirectional stdio transport in Phase 5; `run --jsonl` keeps its one-shot meaning. Sockets and a persistent service remain evidence-gated.

### Optional host adapters — Herdr reference integration

**Classification: host-specific accommodation.** Implement and validate generic control with a standalone client before this optional integration. A narrow observer can receive semantic transitions and map them to a host's reporting vocabulary. If Herdr support is implemented, invoke its documented CLI using direct argument arrays with a short deadline; failures are diagnostic and never fail software work. Its absence or removal must leave the core, human UI and generic machine workflow complete. Generic external consumers use Dispatch's JSON/JSONL contract, including the Phase 5 stdio interface. If user-configured hooks become necessary, accept an explicit executable/argv, send a compact event on stdin, bound runtime/output, and keep them off the correctness-critical path. No plugin framework or shell interpolation is needed.

DHH’s August 26, 2026 Lex Fridman episode describes moving from tmux to Herdr for simultaneous agent work, notifications, and multiple machines. This is a host reference, not a specification for Dispatch's identity, roadmap or allocation economics. Herdr v0.9.0, released September 7, keeps runtime state in servers and terminal rendering in clients. Dispatch may run in those panes without owning pane management. [Official transcript](https://lexfridman.com/dhh-2-transcript/), [Herdr release](https://github.com/herdrdev/herdr/releases/tag/v0.9.0).[^dhh][^herdr-release]

Optional adapter acceptance checklist, separate from generic Phase 5 acceptance. These pinned research notes must be revalidated against the supported installed host before advertising integration:

1. Launch `dispatch` in a normal pane, directly or through `herdr pane run`. Current `agent start --kind dispatch` is not supported.
2. When inherited `HERDR_ENV=1` and the required pane/binary context are present, report `--source custom:dispatch --agent dispatch --state idle`. Report working immediately after goal submission, before probes or snapshots, and preserve that interval through planning, execution, recovery and checks.
3. Use `pane report-agent <pane-id>` through `HERDR_BIN_PATH`. Required answers and interactive review map to blocked; ordinary capacity waiting remains working with waiting metadata unless a human decision is required. The observer must actually deliver the working report promptly within Herdr’s activity-detection window, not merely persist a Dispatch event.
4. Call `pane release-agent` for the same source/agent on exit. Use one ordered observer, with no fire-and-forget reports. Validate `--seq` and restart behavior against the supported Herdr release before shipping; never restart counters blindly or substitute wall-clock timestamps. A timed-out report may already have been accepted, so do not blindly retry it. One-shot agent badges may disappear on release: their authoritative completion is process exit plus Dispatch result. Persistent sessions supply ongoing Herdr attention state.
5. Prevent child harnesses and their hooks from claiming the parent pane. Do not forward Herdr reporting context to children; test hooks installed globally in the developer’s harness as well. If isolation cannot be guaranteed, disable competing child reporting through supported per-run configuration or mark integration unsupported—do not rewrite the developer’s global hooks.
6. Add short display metadata for goal/stage only when useful to the tested adapter. Never create or manage panes or internal-subtask layouts. A host may launch Dispatch using its own facilities; no Dispatch launcher plugin is required by this phase.

Herdr accepts reported `idle`, `working`, `blocked` and `unknown`; **done is derived attention state**, not a report value or proof of successful verification. Its prompt/wait behavior does not correlate individual Dispatch goals. Use Dispatch run IDs and JSONL results for reliable orchestration, not a terminal title or “done” badge. [Agents](https://herdr.dev/docs/agents/), [integration guide](https://herdr.dev/docs/integrations/#integrate-your-own-agent), [CLI](https://herdr.dev/docs/cli-reference/#agents), [automation](https://herdr.dev/docs/agent-automation/).[^herdr-agents][^herdr-integration][^herdr-cli][^herdr-automation]

Herdr also exposes a local JSON socket API and executable plugins, but neither needs to be a Dispatch dependency. Critically, v0.9.0 source restricts native session-reference storage/resume to official agents, despite broader language in the integration guide. Detach preserves live processes; server restart does not preserve arbitrary workers. Ship Dispatch’s own checkpoint semantics before requesting native Herdr restore support. This integration is documentation/source-verified; an installed Herdr runtime has not been exercised here. [Socket API](https://herdr.dev/docs/socket-api/), [plugins](https://herdr.dev/docs/plugins/), [session persistence](https://herdr.dev/docs/session-state/), [pinned resume source](https://github.com/herdrdev/herdr/blob/v0.9.0/src/agent_resume.rs#L53).[^herdr-socket][^herdr-plugins][^herdr-session][^herdr-source]

## I. TUI and CLI experience

### Visual and interaction rules

Use the site’s filled inlet, branching paths, hollow possible destinations, cyan active route, fine quiet edges, and generous empty space. There is no permanent sidebar, resource dashboard, chat bubble stack, or log wall. The graph is a view of actual execution state. Do not draw alternatives that were never available, fake task branches, or an artificial planning stage for a direct run.

In text mockups below, `●` denotes a reached/current node, `○` a queued or unused node, `━` an active/selected edge, and `─` a quiet edge. **Labels carry the state**: working, passed, failed, or waiting. Color and glyphs reinforce it; they never carry meaning alone. Use a word such as `passed` alongside ✓, and `failed` alongside ×. Distinguish an unused route from a queued step by label and placement.

Show one current phase, a short reason/status line, and at most one immediate decision. Hide available alternatives by default; `Tab` reveals “why” and bounded details. Reuse stable node positions during a phase. Redraw on state changes and a modest elapsed-time tick; no traveling particles or fake percentage progress. Default motion can be entirely static.

Input is naturally multiline. `Enter` inserts a newline; `Alt+Enter` submits, with portable `Esc` then `Enter` as the same discoverable fallback. Optionally support Ctrl+Enter when the terminal disambiguates it; do not depend on it. Bracketed paste inserts the entire paste without execution. Display “Alt+Enter send · Esc then Enter also works” during editing, reducing the hint after familiar use if desired. Provide standard cursor/word movement, Home/End, undo/redo, kill/yank, and Ctrl+R local history. Up/Down edit multiline text rather than unpredictably recalling a different goal.

History is local, bounded, and excludes unsent drafts by default; offer `history.persistence=none`. Ctrl+C clears a nonempty idle draft, exits on an empty idle prompt, and cancels an active goal. Ctrl+D exits only when the idle input is empty. During execution, `Tab` opens details, PageUp/PageDown scroll the details view, and Ctrl+C cancels. Prompts use the same editor. Do not require mouse capture, function keys, a special font, or a globally installed clipboard helper. Review actions are shown as words; no undiscoverable modal keymap.

No-argument entry begins with intent. After submission, preserve the existing unsafe-local acknowledgement before real execution. Render it as a specific necessary decision on the goal—not a configuration wizard. Repository files cannot grant permission. A separately explicit session authorization may cover the displayed session scope; it must not become silent permanent trust. Scripts retain the current acknowledgement flags.

### 1. Initial prompt

```text
          ╭──○
      ●━━━┤
          ╰──○
       DISPATCH

What do you want to accomplish?

› Fix the cache invalidation bug after a rename.
  Keep the public API unchanged.
  _

Alt+Enter send · Ctrl+C cancel
```

The mark is compact and appears once at session entry. It does not become a large persistent logo consuming the working viewport. On narrow terminals it reduces to the name and prompt.

### 2. Planning

```text
Fix cache invalidation across storage backends

● goal ━━ ● planning · working
                 │
                 ○ implementation

Finding independent changes and their acceptance checks.
Using strong: the goal crosses shared interfaces.

Tab details · Ctrl+C cancel
```

Show this only for a real planner attempt. Lightweight planning inside the execution call is “understanding the change,” not a separate billed branch.

### 3. Simple execution

```text
Fix cache invalidation after rename

● goal ━━ ● Codex / light · working ── ○ verify ── ○ review

Updating the invalidation path.                         0:42
Bounded change with a relevant regression check.

Tab details · Ctrl+C cancel
```

The actual resolved model is in details and optionally beside “light” at wide widths. “Updating…” must come from meaningful observed activity, not guessed progress. Without that signal, say “Agent is working; last output 12s ago.”

### 4. Decomposed execution

```text
Add cache invalidation to all backends

                       ● plan · ready
                       │
          ╭────────────┼────────────╮
          │            │            │
      ● memory     ● disk       ○ shared API
        passed       working      queued
        light        light        standard
          │            │            │
          ╰────────────┼────────────╯
                       ○ integrate
                       │
                       ○ verify → review

One task running; completed changes remain isolated.
```

This is a future task-plan view, not the first milestone. Edges represent dependencies and artifact flow. Only show parallel activity when it actually occurs. Collapse completed subgraphs into a labeled group; never require horizontal scrolling to discover a blocker.

### 5. Escalation

```text
Fix cache invalidation after rename

             ╭── ● light ── ● verify × failed
● goal ━━ ● route
             ╰━━ ● standard · working ── ○ verify ── ○ review

The rename regression still fails. One recovery attempt.
Previous patch and failure are preserved.
```

The first route remains visible and ends at failure. Do not animate it away or imply that only the successful call consumed allowance.

### 6. Human input required

```text
Fix cache invalidation after rename

● goal ━━ ● scope ━━ ● waiting for you

Should renaming invalidate cached child entries too?

› _

Alt+Enter continue · Ctrl+C cancel
```

Pause paid work while waiting. The same graph state handles unsafe-local acknowledgement, missing acceptance criteria, or source drift, with a precise reason and appropriate action.

### 7. Successful execution and completion

```text
Fix cache invalidation after rename

● goal ━━ ● light ━━ ● verify ✓ passed ━━ ● review

2 files changed · regression and project checks passed
Ready for your review.

[Review diff]    [Accept and apply]    [Reject]
```

After explicit acceptance and successful safe apply:

```text
● goal ━━ ● light ━━ ● verify ✓ ━━ ● accepted and applied

What do you want to accomplish next?
› _
```

Do not print a made-up “saved 80%” badge. A capacity detail may say “remaining allowance unknown” or show a timestamped provider observation, separate from task cost.

### 8. Failure

```text
Fix cache invalidation after rename

● goal ━━ ● light ── × verify
                 ╰━━ ● standard ━━ × verify failed

Stopped after two attempts. The rename check still fails.
Partial work and both check logs are available.

[Review partial diff]    [Show failure]    [Return to prompt]
```

Do not say “done” or silently start another attempt. An authentication failure instead says which resource needs attention; it does not draw a verification failure.

### 9. Narrow terminal

```text
Fix rename invalidation

● goal
┃
● Codex / light
┃ working · 0:42
○ verify
│
○ review

Tab details
Ctrl+C cancel
```

Suggested breakpoints: 80+ columns for horizontal simple flow; 48–79 for wrapped/vertical flow; below 48 for one-node-per-line. Below roughly 30 columns or 10 rows, prefer a simple textual state rather than forcing a graph. Never truncate away “failed,” “waiting,” or the required action.

### 10. Non-TUI graph CLI

```text
$ dispatch run "Fix rename invalidation"
● goal ━━ ● Codex / light
  Bounded change with a relevant regression check.
● execute ━━ ● verify ✓ passed ━━ ○ review
  2 files changed. Run dispatch diff, then dispatch accept.
```

Append a few meaningful transitions to scrollback; do not redraw old lines in this mode. Long-running work can emit occasional factual liveness information. Artwork is never the integration API.

### 11. Plain / no-color CLI

```text
$ dispatch run "Fix rename invalidation" --plain
Task: Fix rename invalidation
Selected: Codex, light model
Reason: Bounded change with a relevant regression check.
Execution: completed
Verification: passed (2 checks)
Review: pending
Next: dispatch diff, then dispatch accept or dispatch reject
```

No color alone need not disable Unicode; `NO_COLOR` removes color while labels/edges remain readable. `--plain` removes artwork and control sequences. ASCII graph mode uses `o`, `*`, `-`, `=`, `|`, `+`, and explicit status words. Unicode task content is preserved in every mode.

## J. Rust implementation recommendation

### Recommended dependencies and alternatives

Inspected versions are current documentation versions, not a request to upgrade this repository now. Resolve compatible versions into `Cargo.lock` when each phase begins.

| Library | Recommendation | Reason / limitation |
|---|---|---|
| **Ratatui 0.30.2** | Adopt for the inline graph phase | Immediate-mode rendering, test backend, inline viewport and modern lifecycle helpers. Inspected release requires Rust 1.88+. Use one supported backend. [Docs](https://docs.rs/ratatui/latest/ratatui/), [installation](https://ratatui.rs/installation/).[^ratatui][^ratatui-install] |
| **Crossterm 0.29.0** | Adopt with Ratatui | Terminal events, raw mode, resize and bracketed paste. `event-stream` can join existing Tokio work. One event reader only. [Docs](https://docs.rs/crossterm/latest/crossterm/), [events](https://docs.rs/crossterm/latest/crossterm/event/index.html).[^crossterm][^crossterm-events] |
| **Tokio 1.53.1**, currently locked | Keep | Existing supervisor, pipes, cancellation and concurrency; manifest permits 1.x from 1.47. Use bounded channels; do not create another runtime or replace process-group cleanup with drop semantics. [Process docs](https://docs.rs/tokio/latest/tokio/process/).[^tokio] |
| **Reedline 0.51.0** | Preferred first intent editor | Multiline/Unicode editing, history, undo and shell-style keys without building an editor. Let it own input only between graph stages. [Docs](https://docs.rs/reedline/latest/reedline/), [paste support](https://docs.rs/reedline/latest/reedline/struct.Reedline.html).[^reedline][^reedline-api] |
| **ratatui-textarea 0.9.2** | Preferred alternative if one integrated viewport/editor is demonstrably necessary | Independently maintained Ratatui fork; multiline editing, wrapping, undo, selection and Emacs-like movement. Prototype Unicode/grapheme behavior before selection. Do not ship two editors by default. [Repository](https://github.com/ratatui/ratatui-textarea), [API](https://docs.rs/ratatui-textarea/latest/ratatui_textarea/).[^textarea][^textarea-api] |
| **rat-text 3.1.0** | Evaluate, do not initially adopt | Explicit grapheme support and powerful editor behavior, but rope/editor/focus/scroll machinery exceeds the simple prompt’s needs. Reconsider if the smaller editor fails required correctness. [Docs](https://docs.rs/rat-text/latest/rat_text/).[^rat-text] |
| **tui-prompts 0.6.7** | Do not use as primary editor | Useful prompt/form widgets and soft wrapping; less directly suited to a shell-quality multiline goal editor. [Docs](https://docs.rs/tui-prompts/latest/tui_prompts/).[^tui-prompts] |
| Original **tui-textarea** | Do not choose from old examples by default | Prefer the actively maintained Ratatui fork and validate its 0.30-compatible dependency graph; no claim that the original repository is archived. |
| **unicode-width 0.2.2**, **unicode-segmentation 1.13.3** | Use where Dispatch lays out/truncates text | Measure display cells and preserve grapheme boundaries; byte count and Rust `char` count are insufficient. Align versions/width assumptions with the renderer. [Width](https://docs.rs/unicode-width/latest/unicode_width/), [segmentation](https://docs.rs/unicode-segmentation/latest/unicode_segmentation/).[^unicode-width][^unicode-segmentation] |
| **supports-color 3.0.2**, **supports-unicode** | Evaluate as small capability helpers | Color helper reports levels and honors `NO_COLOR`; Unicode detection is a heuristic, not a font-glyph guarantee. Standard-library `IsTerminal` plus explicit overrides may suffice initially. [Color](https://docs.rs/supports-color/latest/supports_color/), [Unicode](https://docs.rs/supports-unicode/latest/supports_unicode/).[^supports-color][^supports-unicode] |
| **portable-pty 0.9.0** | Do not add to production V1 | Appropriate if a future supported harness genuinely requires PTY interaction, or for terminal test harnesses. Current batch adapters need pipes. It must not create a second supervision path. [Docs](https://docs.rs/portable-pty/latest/portable_pty/).[^pty] |

The first editor recommendation is **Reedline followed by a Ratatui working viewport**, with explicit terminal ownership handoff. During paid work, no general composer is necessary. When input is required, commit the current graph to scrollback, suspend/restore the live viewport, let Reedline collect the answer, then resume the graph. This preserves shell-quality input without constructing a chat application. Validate this handoff in a small prototype before committing to two terminal-owning libraries; if it visibly flickers or breaks resize/paste, choose ratatui-textarea as the one editor instead. This is a bounded dependency decision gate, not two supported input stacks.

### Inline first

Start with an 8–14-row inline viewport, bounded by terminal height, leaving ordinary scrollback intact. Ratatui’s `init_with_options`/`try_init_with_options` can select `Viewport::Inline` without entering alternate screen. `Terminal::insert_before` supports committed output above it. Default `run`/`init` behavior should not be assumed inline. [Lifecycle API](https://docs.rs/ratatui/latest/ratatui/init/index.html), [Terminal API](https://docs.rs/ratatui/latest/ratatui/struct.Terminal.html).[^ratatui-init][^ratatui-terminal]

Inline costs: less room, care around terminal origin/resize, and strict ownership of stdout. Route all human output through the presenter while it is active. Flush completed milestones to scrollback; keep the live graph bounded. A large DAG can collapse groups and reveal a detail view; it does not justify taking over the whole screen by default. Alternate-screen operation can remain a later explicit preference if real tasks demand it.

Terminal capability policy: inspect stdin/stdout TTY, `TERM`, locale and color hints; honor explicit flags and `NO_COLOR`; never force truecolor under tmux/SSH when unsupported. Default foreground/background should remain terminal-native. Offer dark/light semantic palettes where chosen explicitly, and avoid low-contrast hard-coded slate text on light backgrounds. Do not depend on background-color escape-query responses during startup.

Use read-only render state on the UI side and typed commands for user actions. A simple module boundary is enough:

```text
orchestrator.rs + executor.rs + harness.rs    existing work path
models.rs / local allocation types          semantic facts and policy inputs
events.rs                                   durable transition + subscription
presentation/{plain,graph,json,tui}.rs       projections of the same state
integration/herdr.rs                         optional host observer
```

These are suggested ownership locations, not a mandate for a trait framework. UI code cannot spawn a harness, rank resources, run checks, or apply a patch. Avoid global `println!` outside presenters; diagnostics have a separate sink.

Test resize, panic/cancel restoration, bracketed multiline paste, CJK, combining characters, emoji sequences, long paths, narrow dimensions, no-color, ASCII, `TERM=dumb`, tmux and SSH. Terminal widths differ in practice; explicit ASCII fallback covers graph glyphs, while input correctness still needs grapheme tests. Add ANSI/control sanitization for displayed harness text and chunk-boundary secret redaction before any new streaming output. Existing post-capture redaction alone is insufficient for a live stream.

## K. Migration and delivery plan

### Ordering

Use this order:

```text
truthful outcomes + attempt identity + event protocol
    → resource choices + one-harness model controls
    → optional capacity observations + fair shared local admission
    → bounded recovery + durable questions/continuation
    → intent prompt + automatic continuation + graph/review   FIRST DOGFOOD MILESTONE
    → agent-native Dispatch + generic stdio control + standalone client validation
    → second subscription through the same model
    → private outcome policy, first in shadow mode
    → optional planning/decomposition, only if justified
```

Move the generic event/result contract **before** the TUI; otherwise the renderer will become a second state machine. Move basic prompting/review **before** decomposition; the founder needs a useful daily entry point before an ambitious planner. Bring a second real subscription ahead of a large planner to test the resource abstraction against actual differences. Capacity probing can fail to unknown without blocking the initial product. Optional host adapters follow generic Phase 5 validation and do not gate its delivery or normal terminal operation.

Each phase below is a small release or several focused patches, with no parallel replacement engine. For every production patch, use the repository’s full validation, focused regressions, final diff review, and a stated production LOC delta. Use fake capability/model/capacity fixtures in automated tests; paid real-agent runs belong to explicit dogfood activity, not the test suite. Proposed migration numbers assume this plan starts after version 11; renumber if other migrations land first.

### Phase 0 — Make one attempt truthful and observable

**User-visible capability:** `run --json`, `run --jsonl`, and `status --json` reliably distinguish execution, verification, review and application. Human output says “ready for review” or “verification failed,” with correct one-shot exits. A requested model differing from observed output is visible.

**Architecture:** instrument the existing attempt/check lifecycle; introduce typed low-volume events and a result projection; separate requested/resolved/observed model and effort. Preserve raw output and existing token meanings. Begin extracting only touched printing paths. Choose SQLite state+event commit as the new transition authority; retain/repair artifact projections. Define state revision, actor provenance and attempt generation here; implement only transitions the existing path uses. Resolve the control-state durability requirement in O before promising durable acknowledgements.

**Migration 12:** add versioned run outcome/mode and state-revision fields with legacy defaults; add local `attempts` for invocation identity, role, timestamps, requested/observed configuration and raw telemetry references; add event protocol version/per-run sequence/optional attempt reference and transition actor provenance. Backfill only facts available in old records, marking identity provenance legacy. Do not infer historical costs, planner calls, or actual models.

**CLI:** machine output flags, documented exit contract and corrected completion language. Add event replay when the journal supports it; no bidirectional protocol or TUI dependency.

**Tests:** failed checks produce exit 3; unconfigured checks remain explicit; requested/observed mismatch; semantic harness error with process exit 0; ordered transitions; cancellation/check failure preserved; JSON stdout purity; old metadata/migrations/consent/outbox unchanged. Test a failed projection write and recovery from the committed journal. Run all 183 existing tests plus new regressions.

**Success:** the current failed-verification fixture is impossible to mistake for accepted/verified completion, and a caller can observe work without parsing artwork or raw harness text.

**Rollback risk:** exit changes affect scripts; announce them. Additive schema remains, but older binaries may not understand new outcome values. Take a pre-migration state backup and test compatibility; do not promise binary downgrade without it. No routing change yet.

### Phase 1 — Select Codex model configurations through the existing core

**User-visible capability:** explicitly run an available included model/effort, inspect the actual choice, and opt into a deterministic light/standard/strong routing trial. Existing `--agent` and configured fixed models retain their constraint semantics.

**Architecture:** introduce `ResourceChoice`, local funding/pool references, capability snapshot and versioned allocation decision. Resolve dynamic model IDs through supported catalog data where available, otherwise explicit validated profiles. Keep the optional app-server probe adapter-scoped; preserve exec execution. Make status/explain/review allocation-aware and persist local goal feedback without a legacy routing observation, while preserving safe apply. Verify a no-overage control or account configuration before a strict subscription-only trial. Test ChatGPT+Claude-shaped fixtures now even though only Codex dogfoods initially.

**Migration 13:** add separate local allocation-decision storage and universal local goal-feedback revisions; use the run-mode field introduced in migration 12 and preserve legacy routing JSON/observations untouched. Resource configuration is a small versioned user-level file; resolved resource snapshots live on attempts. Do not backfill absent feedback history. No provider/account enterprise tables.

**CLI:** `--model` and an advanced effort override; concise reason in normal output; full choice/capability provenance in `explain`. Trial enablement is user-level, outside project-controlled permission grants. Remove temporary rollout gating once accepted; do not accumulate permanent alternate routers.

**Tests:** multiple models of one harness are distinct choices; controls reach argv without mutating config; invalid effort/model rejection; fixed override wins; missing observed identity stays unknown; model-specific public evidence cannot leak across tiers; allocation runs never create v1 sync records; allocation-mode status/explain/accept/reject work through ordinary commands. Cover preexisting credits, crossing a quota mid-attempt, inherited fast/subagent settings and internal model rerouting.

**Success:** at least two included Codex choices are selectable on the founder’s real account, and the actual chosen configuration can be verified or explicitly marked unobserved. Light routing fires only on the concrete eligibility conjunction. No extra cash route is introduced.

**Rollback risk:** provider model aliases/defaults can change. Disable allocation policy and retain deliberate controls/legacy observations; do not reinterpret existing new-mode records as old benchmark decisions.

### Phase 2 — Add optional capacity and shared local admission

**User-visible capability:** Dispatch explains when it conserves a constrained allowance, waits, or cannot know remaining capacity. Several local Dispatch panes avoid launching uncontrolled simultaneous work against the same pool.

**Architecture:** optional Codex quota probe; extensible constraint list; field-level knowledge/freshness; fair local admission requests and fenced pool leases; deterministic reserve/defer rules. Each foreground process still supervises its own goal. Release model admission after child cleanup, before local verification/review. Record pool sampling separately from attributable attempt consumption. Unknown telemetry does not itself fail execution. See O for queue fairness and crash reconciliation.

**Migration 14:** `resource_pools`, append-only `capacity_observations`, admission requests and pool leases, all local-only. Persist source/version/window identity, owner session, generation, start intent and expiry. Keep funding secrets outside these records. Expiry alone never authorizes replacing a possibly live worker.

**CLI:** brief capacity reason when it affects the route; detailed observations in `explain`/JSON. An advanced manual scarcity override can be recorded as user-estimated, with expiry. No required balance-entry wizard, global portfolio percentage, or capacity dashboard.

**Tests:** missing/null/multiple windows; stale/reset observations; exhausted short or long window; unknown pool mapping; provider rejection; concurrent admission/fairness; owner death around spawn; live orphan/reused PID; stale lease release; external-consumption contamination; no API/credit/reset fallback; probe timeout leaves normal run usable. Waiting and local verification must not retain a model slot after confirmed cleanup.

**Success:** a disabled/broken probe produces honest unknowns, while fresh applicable windows deterministically constrain admission. Two local processes respect the pool concurrency policy and cannot invent separate per-model allowances.

**Rollback risk:** stale/incorrect mapping can over-constrain work. Optional probes can be disabled while preserving unknown-safe routing and observations. Disabling admission removes the concurrency guarantee: first drain/reconcile owned work and explicitly revert to a single active session. Local coordination is never advertised as enforcement across other applications/machines.

### Phase 3 — Bounded recovery and durable clarification

**User-visible capability:** a routine light attempt can get one suitable stronger recovery after an actionable verification failure. Both attempts, total time and final result are visible. A supported structured clarification checkpoints the work and asks an identified question; no model invocation remains allocated while awaiting the answer.

**Architecture:** split the existing candidate procedure at the attempt boundary; add immutable baseline/parent lineage and a final delivery pointer. Normalize failure kinds and supported checkpoint reports in adapters. Add typed, authorized answer/cancel handlers with duplicate/stale request rejection. Enforce goal-wide invocation/deadline limits across recovery and continuation, and distinguish baseline infrastructure failure from the target regression. Keep one original source and safe-apply contract. Do not assume live harness pause/resume.

**Migration 15:** attempt parent/input-snapshot references, continuation/retry reason and final candidate contribution references; pending question/checkpoint, answer revision and command receipts committed with their transitions. These question records move forward from the original Phase 5 proposal. Use Phase 1 local goal feedback; mixed candidate authorship remains explicit.

**CLI:** show the recovery reason and limit; provide an advanced `--no-retry` escape hatch. Existing unsafe-local authorization still applies; any new funding/resource permission must already be within the selected policy’s scope. No automatic continuation after rejection.

**Tests:** no third invocation including yielded continuations; total timeout; cancellation during handoff; stale/duplicate/wrong-goal answer; no child remains allocated during a question; late completion cannot reopen cancelled work; baseline failure spends no blind escalation; immutable failed workspace; original-baseline final diff; mixed provenance; source drift prevents partial apply; recovery does not become a v1 single-harness upload.

**Success:** one real ordinary regression can be attempted light, recovered once when necessary, verified and reviewed without manual copying between agents. Failure preserves evidence and stops within policy.

**Rollback risk:** retry costs can erase savings. Disable recovery without disabling normal runs; retain all attempts and final candidate references. No implicit replay on restart.

### Phase 4 — Intent entry, compact graph, immediate review

**User-visible capability:** bare `dispatch` asks the outcome, routes one task, shows a small live graph, asks necessary decisions and continues automatically, presents a diff and accepts/applies safely, then asks the next outcome. Plain, graph-streaming and machine modes remain complete. No ordinary wait/resume/retry commands are needed.

**Architecture:** implement one presenter over semantic state. Prototype Reedline/inline handoff, select one input stack, then add Ratatui/Crossterm. Keep event processing/render queues bounded; sanitize streamed content before display. Reuse existing review/apply calls with fixed run/candidate identity from the session.

**Schema:** no new execution schema. Optional bounded local prompt history/settings file. UI preferences and drafts are not sync data.

**CLI:** no arguments becomes interactive only with suitable TTYs; presentation flags; minimal discoverable keys. Show unsafe-local acknowledgement after intent. In-run answers/review target fixed question/run/candidate identities, not “latest.” Add a read-only semantic wait to event following (`--until attention|finished`, with timeout and committed cursor); it is an automation facility, never a human coordination step.

**Tests:** state-to-render snapshots at wide/narrow sizes; resize while typing/checking/answering; multiline paste submits once; Unicode editing; no-color/ASCII; no-TTY behavior; cancel/panic/EOF restoration; JSON remains escape-free; answer automatically continues once; cursor wait cannot miss a transition; diff review/accept from the active session; two panes cannot answer or accept each other’s work accidentally. Manual matrix: modern macOS/Linux terminal, tmux, SSH, light/dark themes.

**Success:** satisfy the first dogfood milestone below. Input-to-preparation feedback is immediate; optional discovery cannot make the intent prompt feel stalled. No full-screen dashboard or graph-layout framework is required.

**Rollback risk:** terminal lifecycle bugs can make a good executor unusable. `--plain`/`--ui off` and the ordinary CLI remain reliable recovery paths. Remove the chosen UI layer cleanly if necessary; the core does not depend on it.

### Phase 5 — Agent-native Dispatch and interoperability

**User-visible capability:** an independent client submits a goal, follows its semantic state, answers authorized clarifications, inspects the final changes and result, and completes an authorized review/apply or rejection through a foreground machine session. The same product retains direct human intent, execution visibility, clarification, review and safe apply. One-shot callers keep stable results and advanced checkpoint recovery. None of these capabilities requires Herdr or another host.

**Architecture:** a bounded bidirectional JSONL stdio transport calls the existing typed handlers, with scoped caller authority, idempotent mutations and cursor-based waits. Reuse Phase 3 question/policy/source-lineage validation and existing review/apply guards. Dispatch owns the domain model and scheduling policy; the transport adds no second execution path. No Dispatch daemon, pane management or new orchestration framework.

**Addition classification:**

| Proposed addition | Category | Boundary |
|---|---|---|
| Any necessary fix to existing execution, admission, clarification, review or human UX | Core Dispatch need | Justify with an observed Dispatch defect; preserve established policy and direct human use |
| Versioned stdio commands/events, scoped authority, idempotent replies, semantic waits and artifact/result inspection | Generic interoperability need | Expose existing core capabilities and authorization; no host vocabulary or model policy in the transport |
| Standalone test client and complete workflow/fault fixtures | Generic interoperability need | Run without Herdr installed or host context; use deterministic harness fixtures |
| Optional Herdr lifecycle observer and host integration tests | Host-specific accommodation | Separate adapter and acceptance checks; isolated, removable and unable to mutate core scheduling policy |

**Schema:** reuse Phase 3 questions/command receipts and Phase 2 owner fencing. Reserve migration 16 for durable revocable grants or attachment metadata only if the concrete transport requires it; do not add empty service tables. Migration numbering is provisional. No automatic restart of an interrupted paid attempt; exclusive supervisor ownership prevents double continuation.

**CLI:** `dispatch control --stdio`; advanced checkpoint recovery through the same handlers; status/event/result/capacity and change inspection, semantic waits, and explicitly authorized review/apply/reject. Submission authority alone does not grant acceptance/apply or human-evaluation authority. Forward authorized owner decisions through the existing guards and retain their actual actor/provenance; do not relabel machine judgments as human evaluation. Existing task-file stdin, one-shot JSONL and the human session remain independent and usable.

**Required generic tests and demonstration:** run a standalone client against deterministic fixtures in an environment without Herdr installed or host context. Exercise submit→committed state/wait→identified question→authorized answer→automatic continuation→verification→exact-result/change inspection→explicitly authorized review/apply. Separate cases cover reject, leave pending and cancel. Cover duplicate mutation/reply loss, stale answers and candidates, unauthorized cross-run actions, frame limits, slow subscribers, EOF cleanup, source drift and double-continuation prevention. No terminal scraping or manual copying of machine identities. Retain direct-human intent/visibility/clarification/review/apply regressions; a machine client must not be needed to use Dispatch.

**Optional host validation:** only after the generic workflow is demonstrated, test the bounded Herdr adapter separately against an installed supported release. Check harmless reporter failure, prompt working reports, preserved activity through recovery, child-hook isolation, restart sequence handling and detach/resize/release behavior. These tests gate advertising that adapter, not delivery of generic Phase 5. An absent or disabled adapter must not change Dispatch's full semantic state or standalone workflows.

**Success:** the standalone client completes the authorized software-work loop without Herdr installed, correlates every decision and result to its submitted run, and preserves all core safety guards. Dispatch remains independently useful to humans. Other tools can use Dispatch because its allocation and verified-delivery capabilities are valuable; host adoption is not proof of product value or a release prerequisite.

**Rollback risk:** a control transport can mishandle scope, replay or foreground ownership. Remove or disable it without replacing the core or human UX; recovery must fail closed when state cannot be validated, preserving artifacts rather than spending again. A host observer can independently be disabled or removed without rolling back the generic interface.

### Phase 6 — Add the second subscription through the same abstractions

**User-visible capability:** on an explicitly configured ChatGPT+Claude portfolio, Dispatch can use a suitable included Claude route when preferable or when Codex is unavailable, while preserving one review/apply experience.

**Architecture:** extend the existing Claude adapter with validated model/effort/funding controls, structured rate-limit/failure observations and known/unknown capability handling. Revalidate the provider’s current print-mode subscription rules and prove a supported no-overage control or account configuration for strict subscription-only eligibility. No routing branch hard-coded as “if Claude subscription then …” outside adapter/resource data.

**Schema:** no core migration should be necessary. Store new provider payload versions in existing capacity/attempt fields. If a rewrite is required, treat it as evidence that the resource abstraction was too narrow and fix that before adding more providers.

**CLI:** second funding source and profiles in optional user setup; ordinary intent flow unchanged; `--agent claude` still works. Never require both providers.

**Tests:** both pools constrained independently; no percentage summing; funding mode/fast-mode/alias hazards; quota crossing mid-attempt with preexisting credits; Claude unknown preflight capacity; headless event fixtures; cross-harness recovery lineage; internal subagent/model switching; no false token/cost equivalence. Run a small explicit real-account smoke test before support claims.

**Success:** add and remove Claude without changing goals, checks, review, events or model selection architecture. A single-provider installation remains fully functional.

**Rollback risk:** provider billing changes. Disable affected profiles immediately and retain Codex/unknown-safe paths. Never silently reroute to paid API usage.

### Phase 7 — Learn privately, then cautiously change policy

**User-visible capability:** `explain` can show relevant private accepted/rejected observations and their limits. A shadow policy can propose a better lane; controlled promotion can change future choices after evidence review.

**Architecture:** derive attributable cohorts from local attempts/feedback; separate first-attempt resource outcomes from multi-attempt policy outcomes. Preserve public priors as a separate evidence source. No ML, embeddings, LLM judge or synthetic quality score.

**Schema:** optional derived-query indexes and versioned policy-decision metadata; raw facts remain authoritative. No Cloud ingestion change.

**CLI:** advanced evidence/shadow comparison only; normal user sees a concise supported reason. Policy changes carry a version and can be reverted.

**Tests:** missing review excluded; latest local revision wins; recovery/combined authorship excluded from standalone success counts; compatible model/version/verification grouping; no mixing benchmarks with acceptance labels; shadow mode leaves actual route unchanged; deterministic rollout/rollback.

**Success:** a prospective dogfood comparison shows useful policy changes without increased rejection/rework. Insufficient data produces “not enough evidence,” not a confident learned score.

**Rollback risk:** selection bias and sparse cohorts. Restore the deterministic policy; preserve observations and decisions for inspection. Do not tune thresholds repeatedly on the same small set and call the result validated.

### Phase 8 — Planning and decomposition, only after the economics gate

**User-visible capability:** a multi-part goal can become a small explicit task plan, run through suitable resources, integrate, verify and present one result. Simple goals continue bypassing planning.

**Architecture:** one planner attempt, validated task list/DAG, fixed maximum size, explicit source/context lineage, sequential integration first, shared goal limits. Add scoped worker dependency/assistance proposals, artifact-gated automatic unblocking and fresh admission for continuations. Reuse attempts, executor, checks and events. Concurrency is an optional later optimization within demonstrated independent scopes; a daemon is not a prerequisite.

**Migration 17 (renumber if 16 is unused):** child `tasks`, versioned dependency edges and satisfaction-artifact references, plan version/artifact reference and task linkage on attempts. No second goal database or reinterpretation of blind comparison candidates.

**CLI:** plan preview/detail inside the graph and optional advanced `--plan`; no mandatory plan approval for every routine execution. A consequential unresolved requirement still asks the human. New planning capacity is disclosed before the opt-in experiment.

**Tests:** malformed/cyclic/oversized plans; unauthorized dependency and stale generation; prerequisite fails/cancels/changes artifact; artifact integration gates resumption; no lost wakeup; missing check references; planner timeout charged once; no recursive expansion; task scope overlap; context/lineage preservation; failed integration; root suite runs; continuations share goal-wide limits; final acceptance does not create per-task human labels; cancellation preserves partial evidence.

**Success:** on real suitable goals, planning produces more accepted verified work or materially less time/rework for the same observed allowance. Include planner, repeated context, verification and integration costs. If that comparison fails, retain the no-planner product and stop this phase.

**Rollback risk:** highest in the plan: context loss, dependency mistakes, large snapshots and multiplied costs. Keep decomposition opt-in until proven. Retain plan/attempt artifacts, return to single-task allocation, and delete unused scheduler machinery if the experiment does not justify it.

## L. First dogfood milestone

The first milestone is **one outcome, a suitable included Codex model, visible verification, bounded recovery/clarification, and immediate safe review/apply from an intent prompt**. It ends at Phase 4, now including automatic continuation after a necessary answer and fair admission across foreground sessions. Exact capacity telemetry, a daemon, bidirectional external control, Herdr-specific integration, learned routing, full decomposition and additional providers are not prerequisites.

The founder should prefer it because a routine bounded task needs less model-choice management and less manual movement between implementation, checks and review, while reserving capable intelligence for work that needs it. It must do something useful beyond opening Codex with a prettier header.

Acceptance criteria:

1. **Intent:** from a project, `dispatch` immediately shows the prompt. Multiline/Unicode paste works. No setup, benchmark explanation or run ID precedes intent. Any necessary authorization or missing check decision appears after the goal is understood.
2. **Real resource selection:** at least two included Codex configurations on the founder’s actual account are validated as selectable; request/observed identity and funding are recorded. Unsupported controls do not silently fall back. If relative consumption is unproven, the UI says so.
3. **Useful allocation:** an agreed set of routine task/check mappings takes the light route; ambiguous/cross-cutting work takes a suitable route or asks a useful question. A deterministic explanation is available for every choice. No hidden frontier classifier call.
4. **Bounded work:** at most two implementation harness attempts for a simple goal, one total deadline, one active attempt per local scarce pool. All attempts survive in history; internal model calls are controlled where supported and otherwise marked unknown. A strict included-only trial passes the no-overage eligibility gate in G. No credit purchase or reset consumption without separate authorization; no hard cash guarantee based only on preflight quota samples.
5. **Verification:** known passing/failing/unconfigured states are unambiguous; required full checks still run. Failed checks cannot produce a success exit under the required-check policy. Human acceptance is never inferred.
6. **Review:** review the diff and accept/apply without leaving the session or copying IDs. Dirty sources, plain directories and linked worktrees keep their current safety guarantees. Concurrent panes cannot change the target of acceptance.
7. **Terminal reliability:** cancellation leaves no owned processes, restores the terminal and saves partial state; narrow/no-color/ASCII/plain and machine modes work. Tmux and SSH smoke tests pass.
8. **Voluntary use:** across at least 20 ordinary eligible tasks over five or more working days, the founder chooses Dispatch for at least four of the last five eligible tasks without being prompted to dogfood it. Record why direct Codex was preferred for any eligible task. This is a product gate, not a statistically conclusive study.
9. **Outcome guardrail:** target at least 18 of those 20 tasks accepted with the configured verification contract satisfied, with no observed increase in serious regressions and no material increase in human repair time versus the founder’s recent direct workflow. Review every miss; task mix and small sample size limit inference. Do not select only trivial tasks after the fact.
10. **Economics gate:** on several comparable work sessions with trustworthy allowance sampling, include all attempts and compare allowance per accepted verified goal with direct suitable-model use. Predeclare a material-improvement target (for example 20%) for the pilot; do not treat that target as a forecast. If telemetry is too contaminated/unknown, claim workflow usefulness and observed model/latency facts only. Broader savings claims and decomposition investment wait for better evidence.
11. **Autonomous clarification:** a supported ambiguity case asks one direct, identified question in the live session, ends the current harness invocation cleanly, and continues automatically after the authorized answer. No wait/resume/retry command, duplicate launch or new capacity authorization is hidden in this interaction. A continuation counts toward the same two-invocation ceiling; insufficient remaining budget is explained.
12. **Shared admission:** two foreground sessions using the same local pool obey deterministic fair admission. A waiting question holds no model slot; confirmed harness completion releases the slot while local checks/review proceed. Unavailable work in one pool does not block eligible work in another.
13. **Control correctness:** duplicate/stale answers and wrong-goal actions are rejected or replayed safely; cancellation fences late reports. Crashes around admission/start/answer cannot silently double-launch; an uncertain surviving process withholds replacement admission until reconciled. Test the typed handlers now, before exposing them through Phase 5 IPC.
14. **Semantic attention:** status/events distinguish active work, automatic waiting, a required human decision and terminal result. A read-only machine wait observes committed state/cursors without terminal scraping or lost transitions. Human attention can be required even while a future sibling task still works.

No synthetic tournament is required. Use normal tasks once, with alternating predeclared policy sessions where practical, and preserve task mix, missing feedback, failures and direct-work observations. The purpose is to decide whether the founder wants the product and whether its economic premise survives real work—not produce a benchmark leaderboard.

This milestone can replace direct Codex for ordinary bounded implementation and supported clarification. It is not yet a universal substitute for extended exploratory conversations, arbitrary interactive tool approvals or multi-task coordination. The product should route those needs honestly and expand only from observed friction. Section O specifies the broader agent-native contract and its later gates.

## M. Risks and open questions

| Risk / unproven assumption | Why it matters | Resolution or stop condition |
|---|---|---|
| Model choice may not save enough included allowance | API prices and provider message ranges are not task economics | Validate selectable included choices and real accepted-work consumption before promising savings |
| Quota telemetry is incomplete or unstable | App-server experimental; Claude preflight unknown; windows nullable | Optional versioned probes, unknown fallback, recorded freshness; no terminal scraping/private OAuth endpoint dependency |
| Published capacity mapping differs from account behavior | Shared pools, extra windows, external apps and machines | Preserve opaque bucket identity; treat unmapped profiles conservatively; never create fictional per-model pools |
| Catalog availability is not included funding | Speed, model, API keys, account policy can change billing | Validate funding per profile, disable paid modes, no silent fallback; recheck provider docs before release |
| Provider overage begins inside an included-model attempt | Previously authorized credits may be consumed after the sampled quota is exhausted | Prove no-overage controls/account configuration or exclude from strict included-only policy; report uncertainty explicitly |
| Requested model is not actual model | Alias changes, provider rerouting, incomplete telemetry | Separate identities and record reroute events; do not teach the router from falsely exact old labels |
| Cheap retries cost more than starting suitable | Repeated context, long failing attempts, verification time | One recovery, shared deadline, total-chain observations; start suitable for weakly verified/hard tasks |
| Planning consumes the savings | Frontier planning plus child context and integration can dominate | Full decomposition opt-in and late; require real amortization evidence |
| Task decomposition loses constraints or changes semantics | Plan quality and interface assumptions are uncertain | Validated bounded plans, concise shared brief, explicit dependencies, root verification and human review |
| Verification is weak or modified by the implementer | Green checks can coexist with wrong behavior | Record check provenance and baseline; expose changed tests; no automatic acceptance or universal quality claim |
| Local learning learns policy bias | Selected tasks and optional reviews are not randomized evidence | Cohorts, policy versions, missing outcomes, shadow mode; attributable resource evidence only |
| Snapshot overhead erases productivity | Current fidelity includes ignored/build/dependency content; extra workspaces multiply it | Measure bytes/time first. Preserve fidelity; a copy/exclusion optimization needs its own observed-defect design |
| Multiple local panes overconsume a pool | Per-process limits do not coordinate | Local transactional admission and stale-owner tests; disclose limits with external clients/hosts |
| Coordination accidentally authorizes duplicate or foreign work | Stale answers, worker claims and owner crashes can cross a spending boundary | Scoped typed commands, durable receipts, generation fencing and survivor reconciliation; no arbitrary cross-job prompt channel |
| Local control is mistaken for a security sandbox | Unsafe-local workers share the owner's OS authority | Explicit capability limits and same-UID limitation; keep execution isolation claims separate |
| Persistent service adds another execution owner | Frontend/service races can launch or recover the same goal twice | Defer until detached ownership is needed; exclusive ownership transition and drain/checkpoint rollback |
| Native harness autonomy bypasses Dispatch’s intended call graph | Harnesses may spawn internal subagents, switch models, or keep their own continuation policy | Inspect supported per-run controls; disable recursive/internal orchestration in measured profiles where supported, or label its cost/model composition unknown. Do not imply Dispatch’s two attempts bound every underlying model call |
| Terminal work expands into an editor project | Full chat input, dashboards and graph layout consume effort | One prompt editor, bounded inline graph, no multiplexer; always preserve plain mode |
| Inline rendering behaves poorly in some terminals | Resize, scrollback origin, Unicode width, signals | Targeted prototype and PTY/manual matrix; choose one editor stack; fail gracefully to plain |
| Herdr reports race or get replaced by child hooks | Attention could be wrong even when work is correct | Ordered observer, supported sequence/restart test, child environment isolation; integration can be disabled |
| Herdr cold restore appears supported but is not | Docs broader than pinned official-agent allowlist | Explicit Dispatch checkpoint/resume first; upstream native support only after validation |
| New records accidentally widen sync | Existing one-candidate derivation can misrepresent multi-attempt work | Explicit allocation mode and local-only boundary; exact existing consent/preview payload tests |
| Outcome changes break existing scripts | v0.1.2 commonly returns 0 despite failed checks | Document exit changes, version schema, test old commands and provide a clear migration note |
| Optimization reduces quality to a score | Cheaper work may increase rejection or human repair | Human acceptance/rework guardrails; keep automated verification separate; no synthetic quality ranking |
| Dispatch overlaps with Herdr/native agent capabilities | Workspace duplication and double planning reduce value | Own allocation and safe delivery only; measure value over native harness subagents/planning before duplicating them |

Outstanding facts requiring real dogfood or an implementation spike:

- Which included Codex configurations are actually usable by this account, with which supported effort/service settings, and whether the installed structured output identifies all substitutions.
- Whether the optional Codex metadata probe works reliably under the founder’s authentication and ordinary local process environment.
- How much of the quota deltas can be observed without simultaneous activity from other clients, and whether a useful relative consumption ordering persists after retries.
- Whether the founder’s routine projects have relevant verification contracts; discovering a Rust manifest alone is not enough.
- Whether inherited harness settings enable internal multi-agent behavior, paid service modes, or model switching that must be constrained for an honest measured profile.
- Which supported Codex output mechanism reliably delivers a bounded clarification/checkpoint report, including malformed output and abrupt exit; structured report support is a first-milestone implementation spike, not an already-proven capability of the existing adapter.
- Whether Reedline-to-inline handoff meets the required paste/resize/scrollback experience; the alternative is one Ratatui editor, not a growing collection of editors.
- Herdr’s real custom-report sequence/restart behavior, global hook interaction, and attention behavior across pane/server operations.
- Claude’s then-current included/credit rules and which structured rate-limit events actually reach its headless Rust adapter.

## N. Recommendation and exact next phase

**BUILD WITH CHANGES.** The wedge fits the existing Rust/local-first product and could be useful with a small portfolio. Supported model controls and structured observations make it plausible. The current code already owns the expensive parts: isolated execution, verification, source safety, review, observations and durable local state.

Change the order and ambition. Build the event/outcome contract early; validate selectable included model tiers; make bounded software work pleasant before building decomposition; prove generic interoperability with a standalone client before optional host adapters; treat measured savings as a gate rather than a slogan. Preserve comparison and public evidence as advanced capabilities, keep the existing sync envelopes frozen, and avoid any Cloud dependency.

**Execute Phase 0 next: truthful single-attempt outcomes, requested-versus-observed resource identity, and a small versioned event/result stream through the existing execution path.** The first regression should reproduce today’s `Verification FAIL` plus exit 0, then establish the new result/exit contract without changing routing. Follow with live attempt/check transitions and model-identity preservation, all tested with fixtures. No planner, retry engine, TUI, remote backend, or second provider should be in that first implementation patch.

## O. Addendum: agent-native control and autonomous coordination

This is a focused refinement of the preceding plan, prompted by the agent-native coordination addendum. It preserves the resource model, existing Rust executor, verification/review boundary and intent-first UX. Sections C, H, K and L have received corresponding narrow updates so the milestone and interface descriptions agree. Everything below is proposed design, not implemented capability.

### O1. Decision: become agent-native through the same core

**Yes.** Humans give goals and decisions; machines exchange scoped requests and semantic state. Dispatch should understand a worker’s request for a dependency or assistance and decide what happens next. It must not require a human to relay completion messages between agents.

Use a small typed command boundary beside the event boundary already proposed in H. The core validates authority, current revision, policy and source lineage, commits the transition, then directs the existing supervisor. The TUI calls those functions in-process; CLI and machine transports call the same functions. There is no second scheduler embedded in a renderer or API adapter.

```text
 human intent / answer          machine request          worker report
           ╲                         │                       ╱
            ╰──────────── scoped core commands ────────────╯
                                     │
                       validate → commit → supervise
                                     │
                      state + sequenced semantic events
                        ╱            │             ╲
                   graph UI       CLI / IPC      host observer
```

Worker reports are **claims and requests**, not authority to assign state. There is no general `set_state`, `emit_event`, arbitrary shell-execution RPC or cross-job prompt injection channel. The scheduler owns retries, model selection, budget enforcement and dependency satisfaction.

### O2. Semantic state: separate activity, waiting and outcome

Keep C’s four lifecycle values: `preparing`, `working`, `waiting`, `finished`. Keep work result, verification, review and application independent. Use them on the root run first; introduce child task records only with decomposition. A task also has its phase, current attempt reference, state revision, prerequisite references and structured wait conditions.

| Situation | Semantic representation | What changes it |
|---|---|---|
| Preparing input/snapshot | `preparing` | Preparation completes or fails |
| Runnable, awaiting a local slot | `waiting`, reason `admission`; readiness true | Fair admission and a fresh policy check |
| Implementing, planning or checking | `working`, explicit phase | Supervisor/check transition |
| Prerequisite unavailable | `waiting`, reason `dependency`, task/artifact gate | Required artifact becomes usable |
| Conserving/exhausted allowance | `waiting`, reason `capacity`, constraint and refresh trigger | New observation or authorized policy change |
| Missing judgment or permission | `waiting`, reason `human`/`authorization`, identified question | Authorized answer at the expected question revision |
| Previous process may still exist | `waiting`, reason `reconciliation` | Supervisor establishes its actual condition |
| Execution ended | `finished`, explicit work result | Review/application remain separate facts |

Readiness is a predicate, not another synonym for success: all prerequisite gates are satisfied, no unresolved input prevents this task, its policy permits an invocation, and it has not finished. Admission is separate from readiness. A task can therefore be runnable while it waits for another goal’s model slot.

Retry and escalation are decisions that create an attributed attempt; they are not long-lived lifecycle states. “Retrying” may be a presentation label for the associated admission/execution phase. A worker is the process/backend handle plus an attempt lease, with starting/running/stopping/ended supervision facts. Do not add a permanent worker fleet table.

In a DAG, a job remains working while any task/check is active. A waiting branch must not make the entire goal appear idle. Expose `action_required` and pending question summaries independently, because one branch may need a human while another continues. When nothing is active, derive the root’s primary waiting reason and counts from its tasks; preserve all underlying reasons in detailed state. “Finished execution” still does not mean human acceptance.

Quiet output is never proof of blocking. A silence timeout is a liveness concern, not a dependency, request for judgment or opportunity to start a duplicate worker. A crashed invocation ends as interrupted after reconciliation; it does not become an ordinary automatic retry without policy validation.

### O3. Automatic dependencies, yielding and continuation

The initial structured worker report has a deliberately small vocabulary: implementation complete; needs a decision; needs an existing prerequisite; needs reasoning assistance. Include bounded diagnostic evidence and artifact references. A report that an expected interface is missing describes evidence for failure classification; it cannot declare the prerequisite complete or grant itself a stronger model.

The current executor supplies null stdin, captures output and supervises process cleanup; harness output is parsed after execution. It does not provide live conversational control. Start with a versioned final/checkpoint report at a supported invocation boundary, validated by the adapter. Do not interpret arbitrary log lines or repository text as commands. A harness without a supported report path remains explicitly batch-only. [Executor](/Users/jese/bin/dispatch/src/executor.rs:248), [adapter completion path](/Users/jese/bin/dispatch/src/harness.rs:160).

Once child tasks exist, an automatic dependency interaction is:

1. Task B proposes that it needs a named artifact from existing task A in the same goal. Its capability is bound to B’s current attempt/generation and the current plan revision.
2. The core checks scope, self-reference, cycles, target existence, terminal-state restrictions and the task contract. The stored edge is **A → B**, meaning A supplies B. Unknown task names do not create new work. Scope expansion requires a bounded plan revision under the existing policy.
3. For a valid unmet prerequisite, preserve B’s checkpoint and stop/end its invocation cooperatively. Capture partial artifacts and confirm owned process cleanup before releasing admission. If A’s artifact is already usable, avoid an unnecessary waiting cycle; still honor the adapter’s actual continuation capability.
4. A’s completion claim triggers finalization. The supervisor must establish completion, preserve its immutable output and run the prerequisite’s configured checks. The dependency gate also requires an artifact that can be incorporated into B’s input lineage. Neither prose nor exit 0 satisfies this gate.
5. Once the gate commits, B becomes runnable automatically. Incorporate the required A artifact through the existing isolated snapshot/integration path, preserving B’s previous state. Check conflicts and lineage before launch; do not simply point B at A’s mutable directory.
6. Reevaluate available resources and remaining limits, enter fair admission, and start a new attributed continuation. The human types no dependency, wait or resume command.

If A fails or is cancelled, B records an unsatisfied prerequisite and is not repeatedly launched. The scheduler may use the goal’s remaining authorized recovery once, otherwise presents the goal’s specific failure. A revised prerequisite artifact invalidates satisfaction against the old version; descendants need explicit revalidation/reintegration. Never retroactively claim their older outputs were verified against the replacement.

Assistance requests use the same limits: the router may select a stronger remaining attempt, ask for missing judgment, or stop with evidence. A worker cannot invent a new planner budget. Every paid continuation counts toward E/F’s invocation and deadline limits, including repeated requests to yield. For the simple two-invocation policy, a first invocation that asks a question leaves one invocation; it does not also retain an extra recovery. An expired deadline is not silently reset by an answer.

Full provider conversation resumption is a later adapter capability. Initially, continuation means a fresh bounded invocation with a checkpoint, the original constraints, relevant artifacts and concise diagnostics. Do not use `SIGSTOP` as the economic pause mechanism.

### O4. What releasing blocked work saves—and what it does not establish

Dispatch can prevent avoidable repeated model turns that merely check whether another task has finished. After confirmed cleanup, it can release local admission so useful runnable work proceeds. Local verification and human review should not retain a subscription-model slot once the harness has ended; local compute limits remain a separate scheduling concern.

An idle terminal is not necessarily consuming provider allowance, and stopping a local process does not prove immediate remote cancellation or refund. Report local execution release and provider consumption as separate facts. Measure the full chain: checkpoint overhead, repeated context, new invocations, integration and accepted outcome. Automatic waiting is valuable even where its demonstrated benefit is latency or reduced human coordination rather than proven allowance savings.

The rule is: **never allocate a model merely to wait for a state transition the core already understands.** No polling prompts, speculative rechecks or automatic frontier calls because a worker has become quiet.

### O5. Several Dispatch sessions: shared admission before a daemon

Phase 2 should coordinate cooperating foreground processes through the existing local SQLite database. Each process owns and supervises its own goal. Share capacity observations and a small admission queue; do not make one pane own all the other panes’ execution.

Maintain one runnable admission request per goal initially. It names the proposed route, applicable pool constraints, explicit user priority, enqueue order, owner session and current generation. In a short transaction, choose an eligible request and reserve all applicable local slots together. Reserve the selected route only, not every considered alternative or every future task. Only the selected live owner launches its request, after rechecking applicable capacity and funding constraints.

Use stable FIFO within priority classes, with bounded aging that eventually promotes an eligible old request to the highest class; break ties by original enqueue order and stable identity. Recovery/escalation reenters admission. Ineligible requests do not block other usable pools. Avoid preempting an active model invocation just to improve queue order: already-spent context and work can be lost. Initially allow one active invocation per shared scarce subscription pool, then change concurrency only from measured results.

The queue responds to committed prerequisite/input changes, completed attempts, refreshed capacity and bounded timers. An estimated reset schedules a refresh; it does not manufacture replenishment. Daemonless owners may check the shared queue/journal on a bounded timer with backoff. This is local state coordination, not model polling or terminal scraping.

Keep reservations honest: an admission lease reserves Dispatch’s local execution permission, not a provider-guaranteed quantity of allowance. Estimated consumption forecasts retain units and uncertainty and are reconciled against observations; do not debit fictional exact percentages. The guarantee covers the same local state root/account mapping, not other apps, independent state directories or remote machines.

Crash behavior is part of admission correctness. Persist start intent, owner identity, generation and known process/backend identity around launch. Use process-start/boot identity where supported, not PID alone. A lease heartbeat expiring does not prove a child stopped. Fencing rejects stale writes but cannot stop an old process from spending allowance. Reconcile and clean up before reassignment; if survival is uncertain, mark the slot awaiting reconciliation instead of authorizing duplicate work. SQLite transactions cannot make external process spawn exactly once.

The repo already enables WAL and a busy timeout. Use short transactions and existing SQLite primitives; WAL has one writer and depends on same-machine shared state, so this design does not extend to a network-mounted database. Never hold a write transaction across a harness call, check or machine wait. [Current database configuration](/Users/jese/bin/dispatch/src/db.rs:451).[^sqlite-wal]

### O6. When a local service earns its cost

**Do not require an always-on service for the first milestone.** Shared admission, durable state and a live foreground owner solve the initial multi-pane case. The presence of several terminals or an event API alone is insufficient justification.

Introduce a per-user local Rust service when real dogfood repeatedly requires one of these capabilities:

- Queued or waiting work must resume after every frontend/parent has disconnected.
- A ready goal must run without any live foreground process owning its supervisor.
- Several independent machine clients need sustained control/reattachment, and foreground ownership is causing demonstrated recovery or coordination friction.

A private per-process socket may solve live attachment without an always-on service; only add it if that is the actual missing capability. A service is the next step when durable **execution ownership** must outlive frontends. Do not ship stdio, per-process sockets and a daemon simultaneously in anticipation of all three uses.

When justified, host the same core command functions and existing supervisors in one Rust process, with a private Unix socket. CLI/TUI become clients for service-owned goals; `dispatch control --stdio` can be a transport bridge. Use an exclusive ownership epoch/lock for the state root and fence every job supervisor. Introduce service ownership while affected jobs are idle or safely checkpointed; do not hand off live PIDs optimistically or allow foreground and service schedulers to launch the same job.

Declare lifetime semantics in the handshake. In foreground mode, owner EOF/hangup cancels active work through cleanup; a read-only follower disconnect merely ends that subscription. In future service-owned mode, a frontend disconnect leaves authorized work running, while an explicit cancel stops it. Without a live owner or service, a persisted job cannot secretly wake itself later.

The later service still owns no panes, terminal persistence, SSH fleet, cloud coordinator or generic plugin framework. Its value is surviving ownership and shared scheduling, not a second execution backend.

### O7. Machine protocol and semantic waiting

V1 first establishes typed core handlers, status/results and replayable events. Phase 4 adds a read-only semantic wait to event following. Phase 5 exposes bidirectional control through **`dispatch control --stdio`**, a foreground process with newline-delimited JSON requests, responses and event envelopes. This is a transport over the same core, not a separate execution mode. One active goal per connection/session is enough initially; another submission while busy gets an explicit response.

**Classification: generic interoperability need.** Implement this interface and its standalone client validation before any optional host adapter. Requests and responses use Dispatch's complete semantic vocabulary. Keep host identifiers, attention badges and lifecycle mappings out of the core protocol and scheduling policy.

Keep `run --jsonl` one-shot. Its stdin may already hold a task file, so it must never silently become an RPC input stream. Stdio control requires explicit invocation and version negotiation; no PTY, raw terminal mode, decorative output or invisible `/dev/tty` reads. Diagnostics remain on stderr. Use bounded UTF-8 framing, for example 64 KiB per control message, and artifact references for large content.

| Operation family | Meaning | First delivery |
|---|---|---|
| Submit goal | Create one owned goal within a granted execution/funding scope | Existing CLI; typed path before Phase 5 transport |
| Inspect status/result/capacity | Read scoped projections, with knowledge/freshness and result identity | V1 |
| Subscribe / await state | Replay from cursor, then wait for a matching committed state | V1 read-only; Phase 5 on the duplex stream |
| Answer / cancel | Validate actor, goal and relevant revision, then transition once | V1 in-process; Phase 5 externally |
| Inspect changes / review / apply / reject | Target the exact delivery and use existing guards; mutations require explicit review/apply authority, never merely submit authority | Existing human handlers; generic Phase 5 transport |
| Report checkpoint/completion/blocked | Validate current attempt identity and report capability | Phase 3 adapter boundary; general worker channel only when supported |
| Propose dependency / reasoning assistance | Request a constrained plan/scheduling decision, never assign it | Phase 8 for dependencies; existing bounded recovery policy for assistance |

Proposed shape, abbreviated for readability:

```json
{"protocol_version":1,"request_id":"r17","op":"answer","run_id":"g1","question_id":"q2","question_revision":1,"answer":"Preserve compatibility."}
{"protocol_version":1,"request_id":"r17","ok":true,"run_id":"g1","state_revision":19,"sequence":28}
{"protocol_version":1,"event_id":"e29","run_id":"g1","sequence":29,"type":"attempt.queued","payload":{"reason":"clarification_answered"}}
```

The response acknowledges a committed command effect, not a running child or a human-accepted result. Caller identity comes from the authenticated session/capability, not a self-asserted JSON actor field. Submit returns a stable run ID; result inspection returns the immutable result revision being reviewed. Goal/task/attempt IDs are machine references and detail-view data, not normal human chores.

Mutations carry a request ID scoped to the principal. Persist its canonical payload digest and committed reply in the same transaction as the effect and event. Identical replay returns the prior response; reuse with different content conflicts. Validate question revision and attempt generation where relevant rather than making every harmless progress event invalidate an answer. An uncertain transport outcome means retry the same request, not submit new work. Command receipts remain through the supported retry/recovery lifetime; expired IDs fail explicitly rather than silently becoming fresh spending requests.

The semantic wait takes a run ID, last-seen sequence, predicate and bounded timeout. Useful predicates are `attention_required`, `finished`, `waiting_reason_changed`, and any committed state change. Return the matching snapshot and cursor; terminal/already-actionable current state can satisfy the wait immediately. A timeout means no matching transition was observed and never cancels the job. Awaiting human attention is different from awaiting an automatic capacity/dependency wait.

Close the read/register race by reading the snapshot with its committed sequence, subscribing and replaying subsequent journal entries before sleeping. A daemonless cross-process follower can use bounded journal reads to implement the same contract. Recovery can redeliver events; consumers deduplicate. A lagging subscriber receives a resync cursor/snapshot or explicit gap, not silent loss, and cannot stall child pipe drainage. The protocol promises replayable transitions and idempotent command effects, not exactly-once process execution or external delivery.

The repository uses `synchronous=NORMAL`, which does not promise retention of the latest commits after power loss. Before acknowledging durable control changes that can authorize spending, use and measure `FULL` on authoritative control-writing connections, with appropriate platform durability behavior. Publish required immutable artifact manifests before committing their readiness. Preserve interrupted-state reconciliation even with stronger database durability: database commits and provider calls remain separate systems. This is a reliability requirement, not an instruction to make bulk logs synchronous.[^sqlite-sync]

### O8. Humans provide goals and decisions

The normal human vocabulary is: describe the outcome; answer a necessary question; review the result; cancel. Dispatch handles automatic waits, admission, continuation, verification and bounded recovery. No normal prompt says “run `dispatch resume`” or asks the user to operate the dependency graph.

Questions must name a real decision and its effect. Bind each to an ID/revision and allowed answer authority. The same prompt editor collects the answer; the core validates it and automatically proceeds if prerequisites, permissions and remaining limits allow. If another task can safely proceed meanwhile, the scheduler may run it without answering on the human’s behalf.

Advanced recovery remains useful after an owner crash, a deliberately exited one-shot invocation or an interrupted session. It is an explicit recovery path with source/policy reconciliation, not a routine interaction step. In a future reattached human session, show the unresolved goal/decision and let the user continue through intent-level language rather than teaching lifecycle commands.

### O9. Optional host adapter: Herdr

**Classification: host-specific accommodation.** H's reporting notes describe one optional adapter, implemented and tested separately after the generic standalone-client workflow. Herdr may launch Dispatch in a normal pane and own that host's persistence, layout, remote placement and visibility. Dispatch remains a complete standalone product, and its children stay headless and never create task panes. Herdr's prompt acknowledgement and derived attention state do not establish Dispatch goal completion.[^herdr-automation][^herdr-socket]

Map external lifecycle terms only at this adapter boundary. A coarse host state can omit detail in its own display, but it must not overwrite Dispatch's distinct execution, verification, waiting, review, application or interruption facts. Removing the adapter must require no change to core state, scheduler policy, the generic client or the human interface.

Use two separate integration surfaces:

- **Human pane:** run bare `dispatch`; the optional observer maps semantic activity/attention to Herdr’s supported reports. Any required human question/review maps to blocked attention, with metadata if other work continues. Otherwise automatic dependency/capacity waiting maps to working plus a concise wait reason; no false demand for human intervention. Idle means ready for another goal. A final badge never substitutes for Dispatch’s verification/review facts.
- **Machine caller:** an agent or runner owns `dispatch control --stdio`, submits an identified goal and awaits Dispatch’s semantic state. It answers only delegated questions and obtains the result from Dispatch. Use a real owned stream/process connection, not pasted JSON into a human pane. A later scoped socket enables independent attachment only if a tested caller needs it.

Herdr’s socket remains Herdr’s interface. Do not send Dispatch commands into it without an explicit host integration that supports them. A host can launch/connect the generic Dispatch client using its own facilities; this phase adds no pane manager or launcher framework. Test the installed host’s reporting sequence, detach/restart and child-hook behavior before advertising the optional integration. Those tests are not a prerequisite for the generic release. Remote panes use Dispatch’s local state on that remote host; cross-machine allowance coordination remains an explicit later problem.

### O10. Ownership, trust and authorization

| Principal | Granted authority | Excluded by default |
|---|---|---|
| Human owner/session | Manage its authorized goal, answer decisions, review/apply within explicit scope | Implicit control of unrelated goals |
| Observer | Read granted status/events/result summaries | Mutations, private artifacts outside the grant |
| Worker | Report its current attempt, propose a same-goal prerequisite/assistance need, read assigned artifacts | Spend expansion, arbitrary model choice, other jobs, answering human questions, acceptance/apply |
| External orchestrator | Submit within repository/resource/budget grants; inspect/cancel its goals; answer delegated factual questions | All-user access, broader funding, human quality labels, cross-job prompting |
| Core scheduler | Validate and execute policy-authorized transitions | Elevating authority because task/worker text requests it |

Bind capabilities to operation set and the relevant owner/session/run/task/attempt/generation. Revoke or supersede them on cancellation, completion and ownership change. Check authorization in the shared command handler, including CLI entrypoints—not only in an IPC wrapper. A denied machine operation must not have an equivalent unguarded fallback command.

For stdio, a trusted launcher establishes the scope; the other end of a pipe is not inherently a human. Launching from a broadly privileged shell cannot itself establish that a caller deserves all-user authority. Scoped worker channels inherit a private handle or adapter-bound identity, never a controller credential in the prompt. If a harness cannot expose a suitable channel safely, retain its limited batch report capability.

For later Unix IPC, use a private user-owned directory, restrictive socket permissions and peer-credential checks, followed by explicit capability validation. Process IDs, terminal titles and `HERDR_*` values identify context, not authority. Tokio already provides Unix listeners/streams and peer-credential support on Unix; a new network framework is unnecessary.[^tokio-unix]

Do not place capabilities in argv, logs, project files, model context or public events. Keep provider credentials separate. Artifact access resolves through granted IDs and lineage, not worker-supplied arbitrary local paths. A dependency conveys the required artifact and contract, not unrestricted sibling logs or another goal’s prompt. Artifact instructions remain untrusted data and cannot modify control grants.

An answer is bound to the question’s revision and actor class. Factual information may be delegated. Funding changes, unsafe-local permission, source application and human acceptance require their corresponding explicit authority. An automated review decision must retain machine provenance and must not become the human acceptance signal used by the router. Record actor and authorization outcome in local control audit facts without broadening Cloud sync.

**Local capabilities are not a hostile-code sandbox.** Other OS users can be excluded with filesystem/IPC permissions, but an unsafe-local worker sharing the owner’s UID may read accessible state/credentials or interfere with processes. Scoped protocol authorization reduces misuse through the interface; it cannot contain arbitrary same-user code. Preserve the existing unsafe-local acknowledgement and assess real isolation separately. Do not promise that a socket token cures that execution boundary.

### O11. The graph becomes a projection of scheduling state

Retain the site’s cyan selected route, fine quiet edges, filled reached nodes, hollow pending nodes and generous spacing. Add no dashboard. Nodes come from real tasks/attempts/checks; edges come from actual route, prerequisite and integration relationships. Use labels to distinguish those relationships and keep the common view small.

Dependency wait, with automatic continuation:

```text
Add the new cache interface

● interface ━━ ● light · working
      │
      │ supplies interface
      ○ adapter · waiting for interface

Dispatch will continue the adapter when checks pass.
```

```text
● interface · checks passed
      │
      └━━━━ ● adapter ━━ ● light · working

Continuing with the verified interface change.
```

Shared-capacity admission, shown only as it affects this goal:

```text
Fix the cache race

● goal ── ○ standard · waiting for a slot
                    │
                    ○ verify

Another local goal is using this subscription slot.
Dispatch will continue automatically when available.
```

Distinguish this admission wait from known exhausted allowance (“Allowance constrained; next refresh …”) and unknown telemetry (“Remaining allowance unknown”). Never imply “reserved model” guarantees provider allowance. Details may show the authorized queue position/constraint and brief reason; normal view does not expose every other goal’s private text.

Human decision:

```text
● implementation
      │
      ● needs you

Preserve compatibility with the legacy schema?

› Yes, existing callers must keep working._

Alt+Enter send · Ctrl+C cancel
```

After submission, the graph moves to admission/working automatically. No resume button or lifecycle command is necessary. While waiting, draw no active model node if its invocation has ended. Do not label a blocked checkpoint as verified or the whole goal as complete.

At larger sizes, show the current path, immediate prerequisite and a count of collapsed branches. `Tab` reveals bounded details; narrow terminals reduce to one vertical route and one reason. Bring required decisions into view even if their branch is collapsed. The scheduler publishes node identities, relationships and semantic state; a deterministic reducer projects it for inline/streaming/plain presentation. Rendering never satisfies a dependency or starts work.

### O12. Changes to delivery order and validation

The detailed phase entries in K are updated. The focused changes are:

| Phase | Refinement | Shipping gate |
|---|---|---|
| 0 | Authoritative transitions, state revision, actor provenance and attempt identity; resolve durability before promises | Correct results/events and rejected stale transitions; no unused RPC framework |
| 2 | Shared fair admission, owner generations, crash reconciliation; release model slots before checks/review | Two foreground owners cannot double-admit; live orphan uncertainty blocks replacement |
| 3 | Move durable questions/answers/checkpoints forward; bounded continuation uses the same attempt chain | Supported question → safe yield → authorized answer → one continuation; no budget reset |
| 4 | Intent UI answers directly and continues; actual wait reasons and read-only semantic wait | Revised first dogfood milestone in L |
| 5 | Agent-native Dispatch and interoperability: duplex stdio, scoped authority, idempotent replies, recovery and standalone client | Complete authorized submit/await/answer/result/review/apply workflow without Herdr installed; optional adapters have separate gates |
| 8 | Real child DAG, constrained dependency proposals, immutable artifact gates and automatic scheduling | A→B unblocks only against usable checked inputs; failure/cancel/version changes never loop |
| Later, evidence-gated | Transfer supervision to a local Rust service only when detached ownership is required | Same core/tests; exclusive owner; frontend loss and service restart behave as declared |

A service phase, if earned, must be scoped like the other releases: **capability** is continued authorized work after frontends disconnect; **architecture** moves supervisor ownership into the one local process; **schema** adds only necessary ownership epochs/recovery metadata and durable grants; **CLI** attaches normally with advanced service diagnostics; **tests** cover competing owners, crash boundaries, disconnected clients, revocation and restart; **success** is the specific previously failing detached workflow; **rollback** requires draining/checkpointing service work before returning ownership to foreground processes. Removing a socket is not sufficient rollback while its service still owns workers.

Add fault-injection cases at the spending boundaries: owner dies before/after spawn, process identity persists late, PID is reused, worker outlives owner, old generation releases a new lease, duplicate answer arrives after cancellation, response is lost after commit, and a wait registers during completion. Verify artifact preservation and no silent duplicate invocation. A success-path fake agent alone is insufficient validation for this control plane.

### O13. V1 boundary and final recommendation

| Required for the first dogfood milestone | Next machine-integration release | Later, after evidence |
|---|---|---|
| One goal/root task, existing supervised execution | Foreground bidirectional stdio control | Small real task DAG and automatic prerequisite scheduling |
| Typed state/command authority and explicit waiting reasons | Scoped submit/answer/cancel and correlated responses | General live worker-control channels where harnesses support them |
| Fair same-host admission and honest unknown capacity | Recovery and full Dispatch semantic lifecycle through a standalone client | Provider conversation resume if demonstrably reliable |
| Supported clarification, clean checkpoint and automatic bounded continuation | Independent attachment only if a real caller requires it | Persistent local ownership/service and cross-machine coordination, separately justified |
| Safe review/apply, read-only status/events/semantic wait, cancellation fencing | Exact-delivery inspection and explicitly authorized review/apply; optional removable host adapters after generic validation | Broad replanning, parallel decomposition and global portfolio optimization |

The first milestone remains deliberately small. It now eliminates ordinary manual continuation and protects concurrent sessions from uncoordinated spending. It does not wait for a daemon, arbitrary DAG construction or a second subscription. Unknown telemetry and limited harness interaction remain explicit product states, not reasons to fake precision or introduce terminal scraping.

**BUILD WITH CHANGES.** Adopt agent-native commands and autonomous waiting as core semantics; stage their transports and scheduling breadth with demonstrated need. Humans still provide goals and decisions. Execute **Phase 0 next**, as specified in N: truthful outcomes, requested-versus-observed identity and authoritative versioned transitions through the existing executor. The addendum strengthens that foundation; it does not authorize implementing the larger phase now.

## Sources and evidence notes

Local references above point to the audited checkout. External sources were read September 9–10, 2026; rolling documentation may change. The proposed thresholds, schemas, phases, UX and acceptance targets are design recommendations, not claims that those capabilities already exist. No production code changed in this planning phase.

[^site]: Dispatch, [live product website](https://rundispatch.sh/). Browser screenshot, rendered text and CSS inspected. Site copy represents comparison-era v0.1; CSS includes the exact values recorded in B.
[^mark]: Dispatch, [original brand SVG](https://rundispatch.sh/brand/dispatch-mark.svg). Three-circle fork geometry and original cyan/surface/stroke values inspected directly.
[^pricing]: OpenAI, [Codex pricing](https://learn.chatgpt.com/docs/pricing). Current included plans, lighter-model usage guidance and shared usage context; ranges are estimates, not task-level capacity coefficients.
[^appserver]: OpenAI, [Codex App Server](https://learn.chatgpt.com/docs/app-server). Model discovery, account windows, nullability and protocol capabilities. Optional experimental integration boundary; local 0.153.4 generated schema also inspected.
[^exec]: OpenAI, [noninteractive mode](https://learn.chatgpt.com/docs/non-interactive-mode) and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference). Exec, JSONL, model/effort/service controls. Installed `codex exec --help` and version inspected.
[^claude-status]: Anthropic, [Claude Code status line](https://code.claude.com/docs/en/statusline). Structured local status data and quota-window limitations; headless availability not proven here.
[^claude-sdk]: Anthropic, [official Agent SDK TypeScript reference, rate-limit event](https://code.claude.com/docs/it/agent-sdk/typescript#sdkratelimitevent). Official translated page used because the English page exceeded the research reader’s size limit; code types support the in-session signal, not a universal preflight balance API.
[^claude-billing]: Anthropic, [use the Agent SDK with a Claude plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) and [Claude Code with Pro/Max](https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan). Current update pauses a previously announced billing change; older announcement text on the page must not be treated as effective policy.
[^claude-models]: Anthropic, [model configuration](https://code.claude.com/docs/en/model-config) and [CLI reference](https://code.claude.com/docs/en/cli-reference). Aliases, effort, plan restrictions, planning behavior and headless paid-model caveats.
[^claude-fast]: Anthropic, [fast mode](https://code.claude.com/docs/en/fast-mode). Credits and included-subscription boundary.
[^dhh]: Lex Fridman, [DHH episode 501 transcript](https://lexfridman.com/dhh-2-transcript/), August 26, 2026. Relevant intervals 01:31:46–01:41:25 and 02:42:34–02:48:02: agent panes, notifications, asynchronous work, multiple machines and review attention.
[^herdr-release]: Herdr, [v0.9.0 release](https://github.com/herdrdev/herdr/releases/tag/v0.9.0), September 7, 2026; [pinned manifest](https://github.com/herdrdev/herdr/blob/v0.9.0/Cargo.toml). Server/client architecture and terminal dependency context.
[^herdr-agents]: Herdr, [agents guide](https://herdr.dev/docs/agents/). Detection and lifecycle authorities, custom reporting and screen manifests.
[^herdr-integration]: Herdr, [custom-agent integration](https://herdr.dev/docs/integrations/#integrate-your-own-agent). Environment context, reporting identity, lifecycle and release.
[^herdr-cli]: Herdr, [CLI reference](https://herdr.dev/docs/cli-reference/#agents). Supported kinds, reports, prompt and wait semantics.
[^herdr-automation]: Herdr, [agent automation](https://herdr.dev/docs/agent-automation/). Pane/agent operations, blocked-state interaction and uncertain timeouts.
[^herdr-socket]: Herdr, [socket API](https://herdr.dev/docs/socket-api/). Request correlation, local transport and live subscription limitations.
[^herdr-plugins]: Herdr, [plugins](https://herdr.dev/docs/plugins/). Executable manifests, normal pane entrypoints and popup limitations.
[^herdr-session]: Herdr, [session state](https://herdr.dev/docs/session-state/). Detach versus server restart; text replay versus live process recovery.
[^herdr-source]: Herdr, [v0.9.0 resume implementation](https://github.com/herdrdev/herdr/blob/v0.9.0/src/agent_resume.rs#L53) and [API handler](https://github.com/herdrdev/herdr/blob/v0.9.0/src/app/api/panes.rs#L1531). Official-agent allowlist limits custom session references and native restore.
[^ratatui]: Ratatui maintainers, [Ratatui API](https://docs.rs/ratatui/latest/ratatui/), inspected version 0.30.2. Rendering and backend model.
[^ratatui-install]: Ratatui maintainers, [installation](https://ratatui.rs/installation/). Inspected 0.30.2 requires Rust 1.88 or newer.
[^ratatui-init]: Ratatui maintainers, [terminal lifecycle](https://docs.rs/ratatui/latest/ratatui/init/index.html). Inline options and alternate-screen distinction.
[^ratatui-terminal]: Ratatui maintainers, [Terminal API](https://docs.rs/ratatui/latest/ratatui/struct.Terminal.html). Inline insertion, redraw/resize and scrollback behavior.
[^crossterm]: Crossterm maintainers, [API](https://docs.rs/crossterm/latest/crossterm/), inspected version 0.29.0. Events, raw mode, resize, paste and feature selection.
[^crossterm-events]: Crossterm maintainers, [event module](https://docs.rs/crossterm/latest/crossterm/event/index.html). Event-reader ownership and asynchronous stream restrictions.
[^tokio]: Tokio maintainers, [process module](https://docs.rs/tokio/latest/tokio/process/). Async pipes and process lifecycle; Dispatch already has a supervisor beyond simple child-drop behavior.
[^reedline]: Nushell/Reedline maintainers, [Reedline API](https://docs.rs/reedline/latest/reedline/), inspected version 0.51.0. Multiline editing, history and configurable editing behavior.
[^reedline-api]: Nushell/Reedline maintainers, [Reedline methods](https://docs.rs/reedline/latest/reedline/struct.Reedline.html). Bracketed paste and keyboard protocol support.
[^textarea]: Ratatui maintainers, [ratatui-textarea repository](https://github.com/ratatui/ratatui-textarea). Independently maintained fork and editing/wrapping support.
[^textarea-api]: Ratatui maintainers, [ratatui-textarea API](https://docs.rs/ratatui-textarea/latest/ratatui_textarea/), inspected version 0.9.2.
[^rat-text]: rat-text maintainers, [API](https://docs.rs/rat-text/latest/rat_text/), inspected version 3.1.0. Grapheme operations, rope-backed editor and scrolling support.
[^tui-prompts]: tui-widgets maintainers, [tui-prompts API](https://docs.rs/tui-prompts/latest/tui_prompts/), inspected version 0.6.7. Prompt widgets and soft wrapping.
[^unicode-width]: unicode-rs, [unicode-width API](https://docs.rs/unicode-width/latest/unicode_width/), inspected version 0.2.2. Display-width semantics and caveats.
[^unicode-segmentation]: unicode-rs, [unicode-segmentation API](https://docs.rs/unicode-segmentation/latest/unicode_segmentation/), inspected version 1.13.3. Grapheme, word and sentence boundaries.
[^supports-color]: supports-color maintainers, [API](https://docs.rs/supports-color/latest/supports_color/), inspected version 3.0.2. Color levels and `NO_COLOR`.
[^supports-unicode]: supports-unicode maintainers, [API](https://docs.rs/supports-unicode/latest/supports_unicode/). Heuristic Unicode terminal support.
[^pty]: WezTerm maintainers, [portable-pty API](https://docs.rs/portable-pty/latest/portable_pty/), inspected version 0.9.0. PTY process interfaces.
[^sqlite-wal]: SQLite, [write-ahead logging](https://www.sqlite.org/wal.html). Concurrent readers, one writer, same-machine shared-memory requirement and network-filesystem limitation.
[^sqlite-sync]: SQLite, [synchronous pragma](https://www.sqlite.org/pragma.html#pragma_synchronous). WAL `NORMAL` versus `FULL` durability, including application-crash versus power-loss behavior.
[^tokio-unix]: Tokio, [UnixListener](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html) and [UnixStream peer credentials](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html#method.peer_cred), inspected version 1.53.1. Unix/local transport capabilities; authorization remains Dispatch policy.
