# Dispatch after v0.4.1: hotfix v0.4.2, then polish release v0.4.3

## Context

v0.4.1 pruned Dispatch to one path: one agent per run, from a profile or `--agent`,
around the coherence gate. You asked for a focused polish and hardening release:
smoother setup, less typing, clearer Work and coherence state, cleanup of what the
prune left behind, and a calmer test suite. It must not start the daemon, protocol,
Herdr or Work Rebase work.

The audit (read-only, against `2a3c38d`, the 0.4.1 tag) found a shipped defect, so
the plan is split in two, as you chose:

- **v0.4.2 is a hotfix, now.** `dispatch setup` cannot save a profile for anyone
  whose state has a database. `Proposal::prepare` (`src/setup.rs:188-201`) reads
  `capacity_authorizations`, which migration 24 dropped. The setup test
  (`tests/fixtures/resource_setup.py`) runs without a database and asserts none is
  created, so it never reaches this path. Confirmed on the real migrated dogfood
  state: schema 24, table absent. Claude profiles expire every 24 hours, so every
  existing Claude user is blocked within a day.
- **v0.4.3 is the polish release** this plan describes. It covers what you asked
  for as "v0.4.2".

## Executive summary

- **Hotfix first.** Setup's revision floor moves to `funding_refusals`. A
  regression test runs setup against a migrated database. Ship as v0.4.2.
- **P0 for v0.4.3: setup becomes selection, not typing.**
  - One small selection primitive in the existing `Ui`, with a numbered fallback
    in plain mode.
  - Provider, profile, model, effort, login and check choices become menus.
  - The typed `confirm` and `login` become an explicit selectable Authorize /
    Cancel, with focus starting on Cancel.
  - The tier prompt disappears. Account, plan and executable are shown, never asked.
- **P1: one presentation of a Work item.**
  - Origin (native or attached, and which agent), S0 provenance, verdict and why,
    verification, review and integration.
  - The same line in `serve`, `history` and `status`, and additive fields in `--json`.
  - Allocation-era wording leaves the output.
- **P1: observe vs launch.** Copy and first-run flow make clear that profiles are
  only needed to launch work. `attach`, `check`, `serve` and review need none.
- **P1 hardening.**
  - Reads stop rewriting unchanged metadata.
  - Measure the load-sensitive tests, then fix them.
  - Test fixtures drop the 0.4.0 configuration shape.
- **No new commands.** Nothing is renamed; `serve` keeps its name, and its help and
  docs describe it as foreground project watching. The daemon's `start/status/watch`
  vocabulary is decided when the daemon is built.

## 1. Verified current state

**What 0.4.1 left.**
- One native engine (`src/orchestrator/native.rs`) plus attach/serve.
- Adapter funding preflight with sticky refusals (`funding_refusals`).
- A durable launch record (`src/launch.rs`).
- One review store (`goal_feedback_revisions`).
- Schema 24.
- About 18,900 production lines.

**User-facing rough edges.**

1. Setup (`src/presenter/setup.rs`) is entirely typed:
   - menu letters (`c`, `a`, `l`, `b`) and profile numbers;
   - provider names for login;
   - a free-text model ID. A typo, `claude-sonnect-5`, reached a saved profile on
     2026-09-22 and is still in the dogfood `resources.yml`, disabled;
   - free-text effort, advertised as low, medium, high or xhigh while the
     validator also accepts `minimal`;
   - free-text tier;
   - the literal words `confirm` and `login`.
2. Setup still asks for or writes allocation-era fields: `tier` is asked for,
   `pool` is generated, and there is an explicit-bucket-mapping refusal. Nothing
   selects on tier any more; it only appears in output.
3. The service view (`src/orchestrator/serve.rs::describe`) has several gaps:
   - Native runs are labelled `dispatch`, not the agent that ran.
   - There is no verification, review or S0 column.
   - `—` means both "never evaluated" and "unmoved".
   - `serve` never evaluates native results, so a native row's verdict is only
     what its own watcher stored.
4. The status summary (`orchestrator.rs::print_single_result_summary`) prints
   `Allocation trial · <tier> tier` for every profile run.
   - `history` prints raw `RunStatus` strings (`ready_for_evaluation`, `evaluated`)
     and a CANDIDATES column that is always 1.
   - The presenter shows `Allocation: <reason>`.
   - The setup status line reads "Included-resource allocation is not configured."
5. Nothing tells a new user that attach, serve, check and review work without any
   profile. Setup and the `run` refusal present configuration as the entry point.

**Technical cleanup left.**

- `RunStatus` duplicates `outcome` (lifecycle, work result, review, application).
  New runs never produce `Evaluated` or `Deferred`.
- `State::load_run` (`src/state.rs:96-141`) rewrites `metadata.json` on every read
  whenever a committed projection exists. `status`, `explain`, `check` and each
  `serve` tick therefore write files.
- Leftovers kept only so old runs still load and display:
  - the flattened `RunRecord.historical` map;
  - the `WaitingOn::{Capacity, Admission, Dependency, Authorization}` and
    `WorkResult::Deferred` variants;
  - the `recovery.selected` branch in `presenter.rs:149`.
- `RoutingHumanOutcome` is the review outcome type under an old name.
- Test files keep phase names (`phase0_outcomes`, `phase1_allocation`,
  `phase3_recovery`, `phase4_*`, `phase6_portfolio`). Many fixture `resources.yml`
  strings still carry `capacity:` blocks, `pool`, `tier` and `provider_buckets`.
- Known load-sensitive tests, deferred since 0.4.0:
  - the PTY session fixtures (`phase3_recovery::phase4_pty_intent_answer_recovery_review_and_restoration`,
    `phase4_review` via `tests/fixtures/phase4_session.py`);
  - `phase3_recovery::one_deadline_bounds_invocations_and_waiting_answers`.
  They pass on rerun and fail under load.

## 2. Priorities

| P | Item |
|---|---|
| P0 (v0.4.2) | Setup hotfix and its regression test |
| P0 | Setup selection UI, selectable consent, removal of obsolete setup inputs, setup tests with a database present |
| P1 | One Work line (origin, agent, S0, verdict and reason, verification, review, integration) in `serve`, `history` and `status`; additive `--json` fields |
| P1 | Observe-vs-launch copy: setup header, the `run` refusal, a first-run choice, README "two ways to use Dispatch" |
| P1 | Allocation-era wording out of human output; state words derived from `outcome`, not `RunStatus` |
| P1 | Reads skip unchanged metadata writes |
| P1 | Measure the load-sensitive tests, then fix them |
| P2 | `serve` shows a display-only live verdict for ready native results when the world moves; nothing is stored |
| P2 | More detected checks (`npm test`, `python3 -m pytest`, `python3 -m unittest`, `go test ./...`) plus an "Other command…" typed by the human |
| P2 | Test files renamed by topic; fixture YAML in the 0.4.1 shape; `RoutingHumanOutcome` renamed `ReviewOutcome` |

## 3. Setup UX redesign

**Principle.** If Dispatch can discover or enumerate a value, the user selects it.
Typing is a fallback, never the path. A funding assertion is always a deliberate
choice. Selection keeps it explicit: focus starts on Cancel, and nothing is ever
confirmed by default.

**Every current input, classified.**

| Current input | Becomes |
|---|---|
| Main menu letters `c` / `a` / `l` / `b` and profile numbers | Menu: *Add Claude Code*, *Add Codex*, one row per profile (`claude · claude-sonnet-5 · medium — expires in 3 h`, or its reason for needing attention), *Provider login…*, *Back*. Uninstalled providers are shown but not selectable, with the reason. |
| Login: typing `codex` or `claude` | Menu of the installed providers |
| Login: typing `login` | Selectable *Open provider login* / *Cancel*. Focus is on Open, since the user just chose login; the warning text stays. |
| Model ID (typed, with a default) | Menu from the adapter: models used by existing profiles for that provider first, then the adapter's known IDs. If the installed Codex app-server answers a model-list request during discovery, use that list; verify this in S2 and never assume it. Last row: *Other model ID…*, typed text checked by `config::validate_model` and shown verbatim on the consent screen. |
| Effort (typed) | Menu of the adapter's effort values (one list per provider). Default: medium. This also fixes the `minimal` mismatch. |
| Tier (typed) | **Removed.** Written as `standard` so the file still loads in 0.4.1; no longer displayed. |
| Pool, service mode, runtime, provider | Automatic, as today. The explicit-bucket-mapping refusal is removed. |
| Account fingerprint, plan / funding source, executable, CLI version | Auto-detected and shown on the discovery and consent screens; never asked |
| Typing `confirm` | Consent screen listing exactly what is asserted, then *Authorize and save* / *Cancel*. Focus starts on Cancel, so Enter alone cancels. Esc cancels. |
| Claude attestations (print mode included, usage credits disabled, unmanaged account) | These are human facts Dispatch cannot detect, so they stay explicit lines on the consent screen that the single Authorize choice covers. They are never pre-affirmed silently. |
| Checks: typing a number / `n` / `b` | Menu: detected commands, *Continue without checks*, *Back*. P2 adds more detected commands and *Other command…* |

**Keys:** ↑/↓ or j/k move, Enter chooses, Esc goes back or cancels, Ctrl+C cancels,
Ctrl+D exits.

**Plain mode** (`--plain`, `TERM=dumb`, no alternate screen): the same menu prints
as a numbered list, and a number plus Enter chooses. Consent in plain mode prints
`1) Authorize and save  2) Cancel` and treats empty input as Cancel. Setup still
refuses to run without a TTY.

**Happy path: new Claude profile.**
```text
$ dispatch setup
Accounts / Resources · resources are only needed when Dispatch launches the agent;
attach, check, serve and review work without them.
> Add Claude Code
  Add Codex
  Provider login…
  Back
[Enter]  Checking Claude Code 2.1.274 · account 3f9a…c21e · no model call
Model
> claude-sonnet-5
  … adapter-known models …
  Other model ID…
[Enter]
Effort
  low
> medium
  high
[Enter]
Authorize this resource?
  claude · claude-sonnet-5 · medium · local, standard service, included only
  Account 3f9a…c21e · Claude Code 2.1.274 · expires in 24 h
  By authorizing you confirm: this model and effort are included in your
  subscription; usage credits are disabled; print mode is included; the
  account is unmanaged.
  Authorize and save
> Cancel
[↑ Enter]  Resource saved. Account and funding are checked again before every launch.
```
That is four choices and no typing. The Codex path is the same, with the plan shown
from discovery, for example `chatgpt-plus`.

**Revalidation.** The main menu lists each profile with its expiry. Selecting one runs
discovery, then consent, then save: two choices.

**First run.** Bare `dispatch` with no profiles currently goes straight to setup. It
will offer *Set up an agent for Dispatch to launch* / *Protect work I run myself* /
*Continue*. The second choice prints the `attach` and `serve` commands and changes
nothing.

**Reuse.** One `Ui::select(title, body, items, initial) -> Result<Option<usize>>` in
`src/presenter.rs`, built on the existing `draw`, `next` and `commit` primitives and
modelled on the file index in `src/presenter/inspection.rs` (around lines 571-590).
It is used only by setup, checks and the first-run choice. No widget framework.

## 4. `serve`, status and the TUI

**Improve now (P1).** One function, for example `presenter::work_line(run, validity)`,
replaces `serve.rs::describe` and `state_word`. `history` and the status summary use
it too. Fields:

- **origin:** `native` or `attached`.
- **agent:** the actual harness (`candidate.harness_id`), never `dispatch`.
- **S0:**
  - attached: `merge-base abc1234 (full)` or `snapshot at attach (partial)`;
  - native: `snapshot abc1234 at 14:02`.
- **verdict:**
  - `CONTINUE`, `REFRESH` or `STOP` with the first reason;
  - `unmoved` when the world has not changed;
  - `not checked` when nothing has been evaluated.
- **verification:** passed, failed, not configured or running.
- **review:** pending, accepted or rejected.
- **integration:** applied (human or policy), blocked, or not applied.

`serve --json` work objects gain the same fields, added alongside the existing ones.
`history` replaces CANDIDATES and the raw status with origin, state and verdict. The
status summary replaces `Allocation trial · tier` with `Profile: claude ·
claude-sonnet-5 · medium` (or `--agent claude (no profile)`) and adds the S0, review
and integration lines.

**Terminology now.** Use *Work* for any item Dispatch tracks and *run* only for a
native launch. `serve` help and docs: "Watch this project in the foreground: shows
each Work item's coherence state, observes attached work whose owner has exited, and
auto-applies attached work that is allowed to integrate. Stops when you stop it."
Docs gain a short "Watching a project today" section:

- Foreground `serve`, plus on-demand `check` and `status`.
- It says plainly that nothing watches in the background.

**P2.** When the world moves, `serve` computes a display-only live verdict for ready,
unapplied native results, using `coherence::with_live_validity` the way `check` does.
Nothing is persisted and no locks are taken, so native review and apply keep their
ownership.

**Wait for the daemon.**
- Background watching.
- `start`/`stop`.
- Making project-level `status` the default meaning of `status`. Today `status` is
  per-run; changing it is a daemon-era decision.
- Automatic process discovery.
- A persisted verdict for native Work written by anything other than its owner.
- Any socket or protocol.

## 5. Post-prune cleanup classification

| Class | Items |
|---|---|
| Must (0.4.3) | Setup's revision floor (done in the v0.4.2 hotfix). Allocation-era wording out of human output. Human-facing state derived from `outcome`; `RunStatus` kept for storage. `load_run` writes `metadata.json` only when the bytes differ (check first that `serve` loads through it each tick). Setup stops asking for tier and stops the bucket-mapping refusal. |
| Nice | `RoutingHumanOutcome` → `ReviewOutcome` (serde unchanged). Remove the `recovery.selected` display branch. Test files renamed by topic. Fixture `resources.yml` in the 0.4.1 shape. `tier`, `pool` and `provider_buckets` get serde defaults so hand-written files may omit them; setup keeps writing them for 0.4.1 rollback. |
| Defer | Removing the `historical` map (needed to load old runs losslessly). Dropping the `RunStatus` column or enum (a migration; belongs with the state-authority work). The dual store of `metadata.json` plus `run_projection_json` (the daemon's authority is the natural single writer). Moving crash repair out of `load_run`. |
| Leave alone | The retained human-judgment tables (`evaluations`, `routing_observations`, `routing_feedback_events`). Migrations and their tests. The obsolete `WaitingOn` and `WorkResult` variants (old runs deserialize through them). `funding_safety.rs` and `launch_record.rs` and their fixtures. Coherence, apply fencing, launch record, adapter preflight. |

## 6. Test-suite hardening

1. **Measure before fixing.**
   - Run `cargo test` ten times at the default thread count, and ten times with
     `--test-threads=32` alongside a CPU hog.
   - Record the failures per test in the plan log. Only tests that fail go on the
     fix list.
2. **Fix patterns.**
   - Replace fixed sleeps and short deadlines with waits on committed state: the
     launch record's `spawned` row, the events journal, or `metadata.json` status.
   - Give PTY `wait()` one generous per-step deadline, overridable by environment
     variable for CI.
   - Keep the deadline test's goal deadline separate from fixture timing.
3. **Protecting deleted archaeology.** Remove assertions that only prove removed
   features stay removed where an absence check already covers them (for example
   the repeated `routing_observations` counts). Keep the upgrade tests.
4. **Gaps from the 0.4.1 simplification.**
   - Setup with an existing, migrated database (the hotfix).
   - Revalidation through setup clears a sticky refusal end to end (today's test
     edits `resources.yml` directly).
   - Service-view rows for native and unbound `--agent` runs.
   - `status` for an unbound `--agent` run.
   - An accept whose apply is blocked records the review and leaves the source
     untouched.
5. **New setup tests** (PTY in `resource_setup.py`, plus unit tests for select):
   - Arrow navigation and Esc back.
   - Enter on the consent screen with focus unmoved cancels and writes nothing.
   - Authorize writes an eligible profile.
   - *Other model ID…* rejects an invalid ID and shows a valid one verbatim.
   - No tier prompt.
   - The numbered fallback in plain mode.
   - The revalidation list shows expiry.
   - Every journey is also run with an existing migrated database.

## 7. Implementation stages

Each stage is one commit on `release-0.4.3`, after the hotfix. Every stage must pass
`cargo fmt --check`, `cargo clippy --all-targets -D warnings` and `cargo test`.
Untouched throughout:
- `src/coherence*`;
- `src/orchestrator/apply.rs`;
- `src/launch.rs`;
- the funding preflight (`src/harness/{codex,claude}.rs`), apart from exposing the
  model and effort lists;
- migrations;
- the funding and launch proof suites.

| # | What changes | Why it is safe | Modules | Tests | Docs |
|---|---|---|---|---|---|
| H (v0.4.2) | Revision floor = max(profile revisions, `MAX(authorization_revision) FROM funding_refusals`). Regression tests: `Proposal::prepare` against a schema-24 database, and a revision that exceeds a refused one. Release notes; tag v0.4.2. | One SQL source change; same semantics as before | `src/setup.rs` | New unit test; PTY setup journey with a database present | Release notes, install doc |
| 1 | `Ui::select` and its plain fallback; convert the checks menu | Presentation only; `save_checks` validation unchanged | `presenter.rs`, `presenter/setup.rs` | Select unit tests; checks PTY journey | none |
| 2 | Setup menus: providers and profiles (with expiry), login, model (adapter list plus Other), effort (adapter list), selectable consent with Cancel focus; tier prompt removed; bucket refusal removed | `Proposal::prepare` and `confirm` semantics unchanged; the file shape still loads in 0.4.1 | `presenter/setup.rs`, `setup.rs`, `harness/{claude,codex}.rs` (lists only) | Rewrite `resource_setup.py` journeys; add consent-default and Other-model tests | Product guide setup section |
| 3 | Observe-vs-launch copy: setup header, `run` refusal text, first-run choice, README "two ways to use Dispatch" | Text and one non-writing choice | `presenter.rs`, `orchestrator.rs` (message), README | `product_ux.rs` asserts | README, product guide |
| 4 | `work_line` shared by `serve`, `history` and the status summary; `--json` additive fields | Read-only presentation; JSON is additive | `serve.rs`, `orchestrator.rs`, `presenter.rs` | `serve.rs` rows for native and attached; history and status output tests | attach.md serve view; coherence.md |
| 5 | Allocation-era wording out; state words from `outcome`; `explain` shows the profile, not a tier table | Display only; storage unchanged | `orchestrator.rs`, `presenter.rs`, `setup.rs` (status) | Update output assertions | README examples |
| 6 | `load_run` writes metadata only when the bytes differ | Same content; fewer writes | `state.rs` | Unit test: repeated load leaves mtime unchanged; the upgrade tests still pass | none |
| 7 | Measure and fix flaky tests; fixture YAML in the 0.4.1 shape; renames (`ReviewOutcome`, test files) | Tests and names only | `tests/*`, `models.rs` | Ten-run stress results logged | Plan log |
| 8 (P2, optional) | Display-only live verdict for native Work in `serve`; more detected checks and *Other command…* | No persistence, no locks; the human types the command | `serve.rs`, `setup.rs` | `serve.rs`; checks journeys | attach.md, product guide |
| 9 | Release: docs pass, release notes, real-provider dogfood (Claude setup revalidation through the new UI; one native and one attached item in the `serve` view), version 0.4.3 | Evidence only | docs | Full suite | Release notes |

## 8. Definition of done (v0.4.3)

- From a state with a database, a new Claude or Codex profile is saved with at most
  four selections and zero typed characters. Revalidation takes two selections.
  Enter on an unmoved consent screen never authorizes.
- Every setup journey passes in the alternate screen and in plain mode, with and
  without an existing database.
- `serve`, `history` and `status` show, for native and attached Work alike: origin,
  real agent, S0, verdict with its first reason, verification, review and
  integration. `--json` gained fields and lost none.
- No human output contains "allocation trial", "tier", "ready_for_evaluation" or
  "evaluated". Old runs still load and display.
- Repeated `status` or `serve` reads leave `metadata.json` unchanged when nothing
  changed.
- Twenty consecutive full-suite runs pass (ten of them under load) with no retries.
  fmt and clippy are clean.
- Real-provider dogfood is logged. Coherence, apply, launch-record and funding proof
  suites are unmodified and green.

## 9. Risks and cautions

- **Consent must not weaken.** Keep focus on Cancel, forbid default-Enter
  authorization, keep the Claude attestations as explicit statements, and keep the
  stale-proposal and lock checks in `atomic_config`. Tests pin each of these.
- **Rollback.** 0.4.1 and v0.4.2 must still read `resources.yml` written by 0.4.3.
  Keep writing `tier` and `pool`, and add no required fields.
- **Model lists go stale.** The adapter list is a suggestion. *Other model ID…*
  always remains, and the launch-time preflight is still the authority.
- **PTY fixtures are the most fragile part of the suite, and stages 1-2 rewrite
  them.** Land the stage 7 wait helpers early if stage 2 turns flaky.
- **Scope.** Do not add commands, a background process, a socket or process
  discovery, and do not let `serve` persist verdicts for native Work. Do not touch
  coherence evaluation or apply fencing.

## Explicitly deferred

Background daemon and `start/stop`. Client/server protocol. Automatic runtime
discovery. Herdr integration. Work Rebase. Project-level `status` as the default.
Removing `RunStatus`, the `historical` map or the dual metadata store. Moving crash
repair out of reads. The real two-agent conflict dogfood (until the daemon, as you
decided). New languages for symbol facts.

## Verification (per stage and at release)

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- The hotfix, by hand: `DISPATCH_HOME=/private/tmp/dispatch-dogfood/state-041
  dispatch setup` revalidates the Claude profile against the migrated schema-24 state.
- The setup UX, by hand: new Claude profile, revalidation, cancel and plain mode,
  against a copy of that state.
- The service view, by hand: `serve` on the dogfood project with one native run and
  one attach, both rows complete; `--json` diffed against 0.4.1 for additive-only
  fields.
- The stress runs from section 6, logged in `docs/plan-0.4.3-polish.md` (the repo
  copy of this plan, created in stage H).

## Progress log

- 2026-09-23 — Stage H, the v0.4.2 hotfix. Setup's revision floor reads
  `funding_refusals` read-only (`Database::max_refused_revision`, 0 when the table
  predates 0.4.1), replacing the dropped `capacity_authorizations`. Tests: a unit
  test for setup against a current-schema database with a recorded refusal, and a
  PTY journey against a state that already has a database. Both fail on 0.4.1 with
  `no such table: capacity_authorizations`. By hand: the real `dispatch setup`
  against the migrated schema-24 dogfood state reached the consent screen for the
  Claude profile; it was cancelled, and `resources.yml` was unchanged.
- 2026-09-23 — Stage 1. `Ui::select` in `src/presenter.rs` implements the menu
  rules from section 3: ↑/↓ or j/k move, a digit moves focus, Enter chooses the
  focused row, Esc or Ctrl+C backs out, focus starts on the caller's initial row,
  and disabled rows cannot be chosen. Plain mode prints numbered rows, and an
  empty line chooses the focused row. The checks menu uses it: detected commands,
  *Continue without checks*, *Back*. Tests: the setup journeys choose checks with
  an arrow and Enter (nothing saved) and in plain mode with an empty line (the
  focused command saved). A PTY capture repaints only changed cells, so tests
  assert outcomes, not the drawn marker. `cargo test`: 428 passed, 0 failed.
- 2026-09-23 — Stage 2. Every setup step is a menu.
  - The main menu has *Add Claude Code* and *Add Codex* (unselectable, with a
    reason, when not on PATH), one row per profile showing readiness and Claude's
    expiry, *Provider login…* and *Back*.
  - Login is chosen, then *Open login*.
  - Models: Codex's own list from `model/list`. This was probed on codex-cli
    0.155.1, which lists `gpt-6-*` models with supported and default efforts;
    hidden models are dropped. Claude uses suggested fixed IDs. Configured models
    come first; *Other model ID…* is validated and shown verbatim before consent.
  - Efforts come from the chosen model: those Codex supports and Dispatch accepts
    (`max` and `ultra` are not offered), or Claude's `EFFORTS`.
  - Consent is the full assertion in scrollback, then *Authorize and save* /
    *Cancel* with focus on Cancel.
  - Tier is no longer asked (written as `standard` for rollback), and the
    explicit-bucket refusal is gone.
  - The Codex app-server handshake is shared by the account probe and
    `list_models`; the funding preflight's checks are unchanged.
  - Tests: `parse_models`, plus setup journeys for Enter-cancels-consent,
    Authorize, revalidation with expiry, account change, login, Other model ID
    (rejected, then verbatim), plain mode, and a state with a database. By hand,
    with the real Claude Code CLI on a copy of the dogfood state: menu, model,
    effort, and Enter cancelled; `resources.yml` unchanged.
  - `cargo test`: 429 passed, 0 failed.
- 2026-09-23 — Stage 3. Observe versus launch.
  - Submitting a goal with no agent set up now offers *Set up an agent for
    Dispatch to launch*, *Protect work I run myself* (prints the attach, finish,
    serve and check commands; changes nothing) or *Back to my goal*. The goal is
    preserved each way.
  - The setup header, `dispatch resources`, the `run` refusal and `serve` help
    say a resource is needed only for Dispatch to launch an agent.
  - The README gains "Two ways to use Dispatch" and "Watching a project today"
    (foreground only; nothing watches in the background).
  - Tests: a setup journey for the observe choice; the `run` refusal wording in
    `product_ux.rs`.
  - `cargo test`: 429 passed, 0 failed.
- 2026-09-23 — Stage 4. One Work line.
  - `orchestrator::work_line(run, validity)` gives origin, the agent that did the
    work (never `dispatch`), S0 (`snapshot <commit>`, `merge-base <commit>
    (full)` or `snapshot at attach (partial)`), the verdict (`CONTINUE`,
    `REFRESH`, `STOP`, `unmoved`, `not checked`) with its first reason, state,
    who applied it, verification and review.
  - `serve` uses it for rows and for `--json`. The new JSON fields are `origin`,
    `s0`, `verification`, `review` and `applied_by`; existing keys keep their
    meaning, except that `agent` names the harness for native runs.
  - `history` shows WORK, STATE and VERDICT from each committed projection,
    read as stored, instead of the raw status and a candidate count.
  - The status summary shows the Work line with its start time. `Profile: …`
    replaces "Allocation trial · tier", and the status line comes from `outcome`.
  - Tests: `work_line` unit tests; `serve` JSON fields for native and attached
    rows; `status` and `history` output in `product_ux.rs`; the allocation
    status assertion uses the profile wording.
  - Flake data point: `phase3_recovery::phase4_pty_intent_answer_recovery_review_and_restoration`
    failed once in the full suite ("activity indicator did not update") and
    passed alone.
  - `cargo test`: 428 passed, 0 failed after those fixes.
- 2026-09-23 — Stage 5. Allocation-era wording is out of human output:
  - The run-start line reads `Profile · <harness> · <model> · <effort>`.
  - `explain` lists the selected profile without tier, pool, composition or
    policy version, and names "Configured profiles".
  - The TUI details say "Profile choice".
  - `show` derives its status from `outcome` through `work_line`.
  - The README profile example has `tier: standard`, commented as unused.
  - Kept: "Capability provenance" (still true: user-validated profile), and the
    apply refusal's status word (`apply.rs` is on the untouched list; it names
    only terminal states).
  - Tests: `explain` has no "tier" and lists "Configured profiles".
  - `cargo test`: 428 passed, 0 failed.
- 2026-09-23 — Stage 6. `State::save_run` skips writing `metadata.json` when the
  bytes are unchanged.
  - Verified first: each `serve` tick loads every run in the state through
    `load_run`, up to four times, and every load rewrote the projection.
  - Tests: a unit test that the file keeps its inode on an identical save and is
    replaced on a change; `product_ux.rs` checks that repeated `status` leaves
    `metadata.json` untouched, and it fails without the fix.
  - The failed-projection-write recovery test still passes: a directory in place
    of the file fails the read, so the write is attempted and errors.
  - Flake data point: `phase0_outcomes::committed_transition_recovers_after_projection_write_failure`
    failed once in the full suite. Its fixture run hit its goal deadline under
    load (exit 124) before any projection code ran; it passed 3 of 3 alone.
  - `cargo test`: 428 passed plus that flake.
- 2026-09-23 — Stage 7a. Names and fixtures.
  - Test files are named by topic: `outcomes`, `profile_selection`,
    `native_runs`, `cli_modes`, `review_session`, `claude_profiles`; fixtures
    `tui_session.py` and `claude_profiles.py`.
  - `RoutingHumanOutcome` is `ReviewOutcome` (serde unchanged).
  - Test `resources.yml` strings drop the ignored `capacity:` block, except in
    the funding and launch-record proof suites, which stay untouched and keep
    proving old keys are ignored.
  - `scripts/capture-product.py` is deleted: nothing references it, and it
    imported the planning fixture removed in 0.4.1. `render-product.py` loses
    its "planned" scenario and still renders setup-journey captures.
  - `cargo test`: 429 passed, 0 failed.
- 2026-09-23 — Stage 7b. Flake measurement and fixes.
  - Measured on `2a96f38`, 10 cores: ten full suites at default threads, 272-321 s
    each. Nine were clean. Run 1 failed
    `claude_deadline_bounds_preflight_and_attempt` and
    `phase4_pty_intent_answer_recovery_review_and_restoration`.
  - The load phase as planned (32 test threads plus a CPU hog on every core) was
    stopped. Its first suite ran over 45 minutes, unit tests took over 60 s, and
    23 tests failed across attach, serve and auto-apply because even 30 s goal
    deadlines expired. That is saturation, not something per-test fixes should
    target. Load is redefined as 32 test threads (3x oversubscription) without
    hogs.
  - Product fix found by the Claude deadline flake: a cancel or deadline noticed
    at the native engine's handoff recorded the stop and then returned an
    error, so `dispatch run --json` printed no result. The same condition one
    check earlier delivered the result. The handoff now records the stop and
    leaves the attempt loop too, so the result and the deadline's exit code
    (124) always reach the caller. The unit test keeps its invariants (nothing
    spawned, no launch recorded) and adds that the stop is recorded and
    delivered.
  - Test fixes:
    - Goal deadlines of 2-5 s in tests that do not test deadlines are now 30-60 s
      (`outcomes.rs`, `unsafe_local.rs`, `e2e.rs`, the handoff unit test).
    - The PTY activity check polls up to 10 s for two spinner frames instead of
      sampling 1.2 s.
    - The native deadline test gives the clarifying run 20 s, checks that its
      question is pending, and waits for the run's own `deadline_at` instead of
      sleeping a fixed 5 s.
  - `cargo test`: 429 passed, 0 failed.
