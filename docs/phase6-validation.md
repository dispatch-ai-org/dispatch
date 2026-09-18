# Phase 6: two providers through one allocation core

Date: 2026-09-17. This is an implementation/validation report, not evidence of
subscription savings or a release announcement. Two separately approved live
smokes are recorded below. The first exposed unrequested auxiliary-model use and
failed Dispatch authorization; the second passed after disabling title generation.

## Starting checkpoint

HEAD was `d52f954d550fe33a12656c90f99155bce10f7c9c`; tracked and untracked status
was clean. Migration **18** was the latest migration. The existing compiled
suite produced **348 passes, zero failures, one existing ignored panic fixture**
with required loopback/process-inspection permissions. The initial sandbox run
had seven loopback permission failures. The baseline documentation-test step
raced the creation of the new module; rerunning `cargo test --locked --doc` from
the preserved baseline subsequently passed (zero documentation tests). The
intermediate failed command remains in the log.

`/private/tmp/dispatch-phase6-start/` contains HEAD, initial status, tracked patch,
`source.tar`, logs, and the production-LOC measurement script. Archive SHA256:
`d087f2c3668beda6bcbe1301c624d80a22a7acfdb4ef5313ec52595112f57ac2`.
Only tracked repository files were archived; no account records were collected.
No commit, stash, tag, push, publish, installation, login or account change occurred.

## Implementation and shared authority

- `run_dispatch` / `select_available_resource` intersect explicit constraints,
  project configuration and grant-visible profiles, then resolve eligibility.
  `select_resource_filtered` applies the minimum suitable lane and stable profile
  order (`allocation-portfolio-v3`). It never compares provider percentages as
  equivalent compute or applies Codex benchmark evidence to Claude.
- `HarnessesConfig::bind_profile` supplies the immutable choice to the existing
  `ClaudeAdapter`. Provider-specific code is in `src/harness/claude.rs`.
  `run_harness` still calls the existing Rust `Executor`; read-only preflight
  uses that supervisor too. There is no SDK bridge, new runtime or second engine.
- `admit_attempt`, `AdmissionCoordinator::authorize_launch`, existing capacity
  history resolution and owner/fence/cleanup logic remain authoritative.
  Launch snapshots now include the full selected profile. A queued profile that
  is removed, disabled or changed is refused at launch, not retargeted.
- Phase 3 `drive` and `recovery_route` remain the only recovery policy. One initial
  invocation plus at most one recovery/continuation share the original deadline.
  Only an actionable target failure can choose a configured stronger lane.
  A vendor change is not itself stronger. Explicit harness/model/effort and grant
  restrictions remain binding. Clarification stays on its original resource.
- `commands::grant` captures eligible profiles as exact immutable JSON values.
  New profiles cannot enlarge an existing grant. Durable submit/answer receipts
  replay their original effect without selecting a new provider.
- Human review/apply, original-baseline restart/diff, source drift, immutable
  failed workspaces, mixed provenance, TUI/PTY presentation and legacy sync
  exclusion continue through the existing paths. **No migration was added.**

A local Claude funding label is not an account boundary: the validated account
hash supplies its canonical funding scope. Because this contract has no reliable
preflight bucket mapping, all Claude choices for that account conservatively
share one pool, including across label/pool renames and bucket expansion. OpenAI
and Anthropic remain separate even when their configured bucket labels match.
Unknown quota does not authorize funding. Unknown later readings, predicted reset
and expiration do not clear known restrictions. Post-launch capacity rejection is
retained as a restriction and a failed invocation, not a free pre-launch deferral.

## Provider contract and evidence level

| Item | Contract and actual evidence |
|---|---|
| Installed CLI | Claude Code **2.1.274**, with executable fingerprint retained privately. The first approved live smoke failed on auxiliary Haiku usage. A separately approved second smoke passed with title generation disabled and only `claude-sonnet-5` reported. |
| Fixture version | `claude phase6 fixture`; separate local Codex fixture. Executable bytes are pinned by SHA256, and the reported version must match. Synthetic, not a released CLI version. |
| Invocation | Direct argv: `claude -p --output-format stream-json --verbose`, fixed `--model`, supported `--effort`, controlled settings/permissions, then `--` and the prompt. No shell interpolation, PTY, resume or continue. |
| Model and effort | Explicit owner-validated fixed ID; no moving aliases, hybrid/fallback lists or context suffixes. Contract v1 supports low/medium/high/xhigh; an omitted effort remains omitted. Per-model support must be validated by the owner. Unsupported controls fail, never substitute. |
| Authentication | Supported read-only `auth status` in the cleared child environment. Expected fixture schema: `loggedIn`, `authMethod: claude.ai`, `apiProvider: firstParty`, `email`, `orgId`. Console OAuth, keys, cloud/gateway methods, missing identity and account mismatch fail closed. Follow-up read-only discovery confirmed these fields on 2.1.274 and reported `subscriptionType: pro`; no private identity values are included here. |
| Funding | Profile-scoped, time-bound **owner assertion** of print/model/effort inclusion, disabled usage credits and unmanaged account/host configuration. It is not relabeled provider proof. Maximum evidence lifetime is 24 hours. `no_overage_verified` alone cannot admit Claude. |
| No overage | No confirmed universal per-call no-charge switch. The contract requires verified disabled account credits plus standard service, no alternate credentials/providers, fixed model/context, and disabled fast/fallback controls. Unknown account evidence is ineligible. |
| Environment | Existing cleared environment preserves PATH/HOME/TMPDIR/LANG. The Claude adapter also explicitly supplies USER, required by the installed CLI for macOS Keychain lookup. Claude allocation rejects configured environment forwarding. Inherited keys, OAuth tokens, cloud switches, proxies and control-handle variables are not forwarded. Global shell/account configuration is not changed. |
| Settings | `--setting-sources ''`; controlled settings JSON; strict empty MCP config; commands disabled; no Chrome or native session persistence. Project CLAUDE.md remains. Local/global invocation conflicts and known managed settings/MCP/MDM files are rejected. No helper is run to inspect a secret. |
| Headless tools | `dontAsk` and explicit Bash/Read/Edit/Write/Glob/Grep availability/allow rules. No global bypass. No Agent/Task/advisor/planning tools are offered. Permission denial is failure. Bash/tool policy is not a hostile-code sandbox. |
| Completion | Exactly one terminal `result`, last in the stream, with `subtype: success`, `is_error: false`, string result and no permission denials. Exit zero does not override structured failure. Malformed/truncated/repeated/nonfinal terminals fail. |
| Checkpoint | Only the successful terminal result's string `result` can contain the bounded `dispatch_checkpoint` envelope. Intermediate assistant text cannot supply it. Categories are absent or `factual`; unknown categories fail. The generic core receives an adapter-selected checkpoint. |
| Identity/usage | System-init/assistant model fields and final `modelUsage` identities are retained; multiple models leave the single observed-model field unknown and unauthorized substitutions stop further automatic work. Observed effort stays unknown. Final aggregate input/output/cache-read/cache-creation counters are summed once, with explicit semantics. Missing counters stay unknown. |
| Cost | Final `total_cost_usd` stays in bounded raw provider evidence. Candidate transaction cost is unknown, because the nominal estimate does not establish cash charged or subscription balance. |
| Quota | No supported live preflight endpoint established. Account identity survives unknown quota. Stream quota details remain raw/unknown; contract v1 does not manufacture status-line percentages or map undocumented event windows. Supported structured error results retain capacity/funding rejection. |
| Final launch | Available account/configuration checks repeat before the supervised model launch; the final SQLite fence checks the immutable profile and retained shared evidence. A rejected bound preflight invalidates its funding epoch. This is not an atomic guarantee against external account changes between check and request. |

The exact constructed argument list and normalized evidence are exercised by the
fixtures. Full settings JSON is defined once in `controls()`, including the documented per-run environment setting that disables the background title-model request. A real CLI must be
validated against this contract before entering an eligible profile. Managed
accounts/hosts, Windows/WSL policy sources, dynamic compositions, credit-only
configurations, arbitrary extra arguments, native conversation continuation and
live quota percentages are not advertised as supported by this contract.

## Current official sources and unresolved policy scope

Read on 2026-09-17:

- [Subscription SDK/print policy](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan): the June 15 update pauses the announced billing change. Print/SDK usage continues against subscription limits; the historical monthly-credit proposal is not implemented.
- [Usage credits](https://support.claude.com/en/articles/12429409-manage-usage-credits-for-paid-claude-plans): credits can permit separately billed use after included allowance. A positive allowance or plan label does not prove a no-overage boundary.
- [Credential precedence](https://support.claude.com/en/articles/12304248-manage-api-key-environment-variables-in-claude-code): environment API credentials can override subscription login.
- [Headless execution](https://code.claude.com/docs/en/headless): print mode, structured results, background cleanup and bare-mode authentication differ. Bare skips subscription/keychain reads and is deliberately excluded here.
- [CLI reference](https://code.claude.com/docs/en/cli-reference) and [model configuration](https://code.claude.com/docs/en/model-config): controls and aliases are version-dependent. Documentation is not local capability discovery.
- [Fast mode](https://code.claude.com/docs/en/fast-mode), [settings](https://code.claude.com/docs/en/settings), [environment](https://code.claude.com/docs/en/env-vars) and [managed settings](https://code.claude.com/docs/en/managed-settings): managed policy can outrank command settings; these restrictions are not bypassed.
- [Status-line data](https://code.claude.com/docs/en/statusline) is interactive data, not evidence that the same fields exist in headless results. No status-line script was installed or scraped.
- [Official SDK types](https://raw.githubusercontent.com/anthropics/claude-agent-sdk-python/main/src/claude_agent_sdk/types.py) provide protocol context; no SDK was installed or used for execution.
- [SDK overview](https://code.claude.com/docs/en/agent-sdk) separately restricts third-party offerings of claude.ai login/rate limits without approval. That is not silently reconciled with the billing article's third-party-app wording. Dispatch does not offer login, extract tokens or resell access; product/distribution permission and a particular account's eligibility remain unverified.

## Setup, selection and rollback

Keep the existing Codex profiles first if they are the preferred default. Add only
Claude configurations actually validated for the account. One provider and one
suitable lane are sufficient; Dispatch never fabricates three tiers. Configure
`harnesses.claude.executable` in project settings if discovery needs an explicit
path, and leave its `extra_args` empty.

A deliberately **ineligible** example (local account evidence belongs outside Git):

```yaml
# Append under profiles in the user-level resources.yml.
- provider: anthropic
  funding_source: personal-claude
  harness: claude
  model: REPLACE-WITH-VALIDATED-FIXED-ID
  effort: medium
  service_mode: standard
  runtime: local
  pool: claude-included
  provider_buckets: []       # Unknown mapping: account-wide exclusion.
  tier: standard            # Your local policy role, not a vendor quality score.
  included: false
  no_overage_verified: false
  authorization_revision: 1
```

After supported read-only discovery and explicit validation, the local profile's
`claude_subscription` object needs `contract_version: 1`, exact `cli_version`,
`executable_sha256`, `account_sha256`, RFC3339 `checked_at` and `valid_until`, and
true `print_mode_included`, `usage_credits_disabled`, `unmanaged_account`.
The account fingerprint is SHA256 of compact UTF-8 JSON `[email,orgId]` from
supported auth status; do not extract credentials. These fields assert validation
of the **entire containing profile and contract**, not universal provider support.
Do not copy synthetic fixture evidence into a real profile. Keep evidence private.

Automatic selection considers the lowest available suitable lane, then profile
order; an unavailable/exhausted route can yield to the other permitted provider.
Explicit `--agent claude` or `--agent codex`, `--model` and `--effort` remain hard
constraints. Strict configuration errors fail instead of silently falling back.
If all otherwise suitable choices are capacity-blocked, existing deferral/wait
behavior remains. Profiles whose eligibility is unknown cannot execute.

Disable a profile with `enabled: false`, or remove it. History remains readable.
Drain active work; changed queued bindings fail safely. Existing grants cannot
acquire newly enabled profiles; issue a new human-owned grant to broaden scope.
Set `allocation_enabled: false` to return ordinary calls to legacy harness routing
only after coordinated work is drained. This does not certify legacy runs as
included-only. Rejected funding epochs require affirmative revalidation and an
incremented `authorization_revision`, not a cache refresh or reset prediction.

## Deterministic A–S matrix

Every named test runs without real provider authentication. Python is test tooling
only. CLI/control cases execute the actual Rust binary with OS pipes; FIFO gates,
committed admission state, injected observations and spawn-marker counts identify
boundaries. Existing PTY tests cover the unchanged human presenter. The fixtures
are synthetic Claude-shaped schemas, not claimed captured account transcripts.

| Row | Exact tests / injected boundary / asserted outcome | Actual result |
|---|---|---|
| A | `claude_only_cli_identity_usage_review`, `portfolio_missing_optional_and_explicit_failure`, existing `enabled_trial_allocation_reaches_argv_persists_identity_and_stays_out_of_v1_sync`: single-provider execution, absent/disabled optional CLI, explicit unavailable route; no substitution or unnecessary other-provider probe. | PASS |
| B | `claude_only_cli_identity_usage_review`, `claude_scoped_control_verified_review`: fixed argv, 22 reported fixture tokens, unknown cash/effort/quota, passed checks, pending review, source unapplied and zero leases. | PASS |
| C | `claude_malformed_configuration_and_expired_evidence`, `claude_protocol_failures_do_not_recover`, `claude_mixed_models_are_not_collapsed_and_unknown_stays_unknown`: aliases/context/effort/service conflicts fail before spawn; substitution/mixed identities fail after exactly one invocation; unknown is not requested identity. | PASS |
| D | `claude_funding_hazards_before_launch`, `claude_effective_settings_preserve_context_without_hooks`: key/Console-like/cloud auth, missing identity/account mismatch, helper, forwarded credentials, missing inclusion and managed configuration. Zero model launches for unsafe eligibility. | PASS |
| E | `claude_funding_hazards_before_launch`, `claude_protocol_failures_do_not_recover`, `claude_terminal_totals_preserve_cache_units_and_nominal_cost`: extra usage enabled rejects; terminal capacity rejection fails with no retry; nominal USD never becomes a cash charge. No real account boundary was forced. | PASS |
| F | `claude_only_cli_identity_usage_review`, `claude_funding_hazards_before_launch`: authenticated account plus unavailable quota executes under fresh explicit funding evidence; unknown eligibility does not. | PASS |
| G | `portfolio_retained_capacity_routes_to_other_provider`, `partial_mapping_and_null_windows_preserve_applicable_restrictions`, `expired_short_window_cannot_erase_exhausted_long_window` and `shared_capacity_or_funding_changes_block_recovery_before_spawn`: exhausted window survives Unknown and profile/funding/pool rename with expanded labels; other provider proceeds; independent-window supersession remains tested by the core. | PASS |
| H | `portfolio_retained_capacity_routes_to_other_provider`, `portfolio_independent_pools_and_same_pool_exclusion`, `shared_allowance_cannot_be_split_into_per_model_tanks` (config unit test): identical-looking bucket labels do not combine providers; same-account scope cannot invent independent allowances. | PASS |
| I | `portfolio_preference_and_explicit_constraints`, `portfolio_missing_optional_and_explicit_failure`, `portfolio_retained_capacity_routes_to_other_provider`, `uncertain_scope_does_not_require_strong_or_qualify_for_light`: configured preference, missing preferred Codex to Claude, exhausted preferred Claude to Codex, hard model/provider constraints and unsuitable lower-lane rejection. | PASS |
| J | `portfolio_independent_pools_and_same_pool_exclusion`: two independent grant owners, FIFO-held Claude and durable same-pool waiter, concurrent Codex foreground success, waiter cancellation, confirmed lease release. | PASS |
| K | `portfolio_cross_harness_recovery_both_directions`: each direction succeeds from the original baseline with fresh bindings; explicit provider/no-retry permit one invocation; failed recovery permits two and never three. Existing `any_infrastructure_or_unknown_check_blocks_other_target_failures` preserves the escalation gate. | PASS |
| L | `claude_final_preflight_invalidates_epoch`, `portfolio_grant_cannot_expand_and_receipts_do_not_replay`, `claude_control_changed_funding`: account changes at final auth preflight invalidate the epoch with zero model spawns; restoring identity alone cannot renew it; Codex-only grant excludes added Claude, new broader grant includes it; replay after profile change launches nothing. | PASS |
| M | `claude_control_clarification_idempotency`, `claude_only_successful_final_result_can_supply_checkpoint`, `claude_protocol_failures_do_not_recover`, `claude_control_unclassified_question`: terminal-only envelopes, malformed/repeated/later output, unsuccessful result, prose and wrong category; no invalid pending question and no lease while waiting. | PASS |
| N | `claude_deadline_bounds_preflight_and_attempt`, `claude_owner_eof`, `claude_question_eof`, `claude_control_cancellation`, `claude_control_answer_cancel_race`, `claude_control_broken_pipe`, `claude_control_slow_output`, `claude_control_recovery_limits`: original deadline including preflight (at most one actual invocation; zero is valid if the deadline expires before spawn), pipe loss, cancellation and no-replay. Existing executor/admission fault tests cover unconfirmed cleanup and cancellation at spawn. | PASS |
| O | `claude_protocol_failures_do_not_recover`, `claude_malformed_contradictory_or_nonfinal_results_fail_closed`, `claude_terminal_totals_preserve_cache_units_and_nominal_cost`: exit-zero errors, missing/broken/repeated result, permission denial, rate rejection, unsupported model, model switches and exact aggregate cache-token accounting. | PASS |
| P | `claude_effective_settings_preserve_context_without_hooks`, `claude_funding_hazards_before_launch`, `claude_only_cli_identity_usage_review`: controlled argv/settings, unchanged project settings, preserved CLAUDE.md, absent competing environment credentials/control handles; no helper execution or global edits. Fixtures assert the control contract, not the implementation of an unavailable real CLI. | PASS |
| Q | `portfolio_cross_harness_recovery_both_directions`, `claude_control_recovery_drift`, `claude_control_lost_answer_receipt`, `claude_scoped_control_verified_review`, `recovery_delivery_applies_from_original_baseline_and_pending_question_is_not_reviewable`, `phase4_pty_intent_answer_recovery_review_and_restoration`, and `enabled_trial_allocation_reaches_argv_persists_identity_and_stays_out_of_v1_sync`: immutable failed workspace, original diff lineage, review pending, source drift, artifact scoping and no legacy upload. | PASS |
| R | `claude_profile_lifecycle_preserves_history`, `portfolio_grant_cannot_expand_and_receipts_do_not_replay`, `foreground_configuration_changes_respect_overlap_and_allow_disjoint_work`: disable/change/remove and a disabled old definition before an enabled replacement preserve history and receipts; removed queued profile is refused, independent work still proceeds. | PASS |
| S | Complete test suite, release build, release two-provider fixtures and both adapters' control smoke/clarification/idempotency: no provider account, purchase, real model traffic or host integration. | PASS |

The earlier Phase 2 overlap test was intentionally strengthened: a removed queued
profile must now fail without spawning. It still checks live same-pool exclusion
and disjoint progress. This is a Phase 6 launch-authority correction, not a timeout
increase or ignored failure. The sole ignored test remains the pre-existing
explicit terminal panic fixture exercised through its PTY test.

## Command results, size and live gate

Initial Phase 6 validation used the then-current source and actual Dispatch binaries (installed-CLI follow-up results are recorded below):

| Command / check | Actual result |
|---|---|
| `cargo fmt --check` | PASS |
| `RUST_TEST_THREADS=4 cargo test --locked` | PASS: **385 passed, zero failed, one pre-existing ignored test**; documentation tests also pass (zero tests). |
| `cargo test --locked --test phase3_recovery -- --test-threads=4` | PASS: all 38 recovery/PTY tests. |
| Phase 6 integration suite, included in the full run | PASS: all 33 integration tests, plus four Claude parser unit tests in the library suite. |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS, no warnings. |
| `git diff --check` | PASS |
| `cargo build --release --locked` | PASS |
| Release fixture smoke | PASS: all 11 scenarios below against `target/release/dispatch`. |

Rows A–S passed. Release commands used isolated provider fixtures:

- `DISPATCH_FIXTURE_PROVIDER=claude python3 tests/fixtures/phase6_portfolio.py target/release/dispatch <scenario>`: `direct`, `selection`, `recovery` (both directions), `pools`, `capacity` — five passes.
- `DISPATCH_FIXTURE_PROVIDER=<provider> python3 tests/fixtures/phase5_control.py target/release/dispatch <scenario>`: each of `codex` and `claude`, with `smoke`, `clarify`, `idempotency` — six passes.

All full-suite and release assertions ran without real provider traffic. No new ignored tests or inflated timeouts were added.
The environment permits local loopback and process inspection for these fixtures;
production behavior was not changed to accommodate sandbox restrictions.

Validation failures were investigated and retained in the logs. Running the
preserved baseline's documentation check with the current target directory had
replaced the binary with the older Phase 5 executable; the old policy version and
missing profile snapshot in its result identified that contamination. Cleaning
only the Dispatch package artifacts forced a current-source rebuild. A subsequent
default-concurrency full run passed the strengthened queued-profile test but
failed the existing PTY plain-output wait and wall-time deadline assertion. The
same 38-test recovery suite and the complete final suite passed at four-test
concurrency, without changing those tests' synchronization or time limits. This
establishes the recorded four-thread run, not a claim that unrestricted parallel
stress is flake-free. Earlier failed runs are not counted as passes.

Production LOC uses the same Phase 5 method: physical lines in `src/**/*.rs`,
including comments/blank lines, excluding complete `#[cfg(test)]` items/modules.
Integration fixtures and the unchanged 134-line reference client are excluded.
The preserved baseline is **22,749** lines; the initial Phase 6 source was **23,807**, a net
**+1,058**. Most additions implement the Claude protocol, funding/settings checks
and normalized observations; the remainder binds them to deterministic selection,
launch invalidation, recovery and exact profile scope. Replaced Claude invocation
construction and generic-core Codex checkpoint dispatch were removed. No new
runtime dependency, execution engine, database table or migration was introduced.

Plan deviations/limitations: explicit fresh owner-validated profiles substitute
for unavailable real CLI/account discovery, as permitted by the brief. Unknown
quota remains unknown and account-wide exclusion is conservative. Legacy Claude
commands now use bounded noninteractive tool permissions instead of the old global
permission bypass. The strengthened stale-profile launch check and malformed-proof
rejection are safety corrections. Unrestricted test concurrency remains a recorded
validation limitation; live CLI/account compatibility remains a separate gate.

Local logs, the reproducible release-smoke runner, and the LOC script/results are
in `/private/tmp/dispatch-phase6-start/` and need owner retention beyond the
operating system's temporary-file lifetime. The final full-suite log is
`final-full-tests-four-threads.log`; failed unrestricted runs remain in
`final-full-tests.log`. Clippy/build/smoke results are in `final-clippy.log`,
`final-release-build.log` and `final-release-smoke.log`.

LIVE CLAUDE SUBSCRIPTION: **VERIFIED FOR THE TESTED CONFIGURATION** — read-only discovery confirms Claude Code
2.1.274 with first-party claude.ai Pro authentication. The user confirmed usage
credits disabled; the first approved invocation additionally reported
`overageStatus: rejected`, `overageDisabledReason: org_level_disabled`, and
`isUsingOverage: false`. This is time-bound evidence, not an atomic future billing
guarantee. The first live smoke failed the fixed-model authorization gate. The
separately approved second invocation passed with background title generation
disabled; detailed evidence is below. No automatic retry or human acceptance occurred.

Known optional limitations: no live quota mapping, managed/cloud/Windows policy
support, dynamic aliases/compositions, or native session reuse. No planner, DAG,
parallel subtasks, provider racing, paid fallback, daemon, service, host adapter,
benchmark campaign, learned policy or Phase 7 work was added.

## Installed CLI follow-up

Read-only validation after the user installed and logged into Claude Code 2.1.274
reproduced two integration defects that the original synthetic fixtures missed:

- With only PATH/HOME/TMPDIR/LANG, `claude auth status` reported no login. Adding
  only USER made the same command correctly report first-party claude.ai Pro.
  USER is now supplied by the Claude adapter to both auth and model commands;
  the executor permits that explicit nonsecret metadata without forwarding keys,
  tokens or control handles. Other adapters do not acquire USER implicitly.
- The final variadic `--allowedTools` option consumed `auth status` as tool names.
  Controls now end with the nonvariadic `--no-chrome` flag, and auth adds its
  explicit `--json` flag. The corrected command returned valid account JSON.
  Misparsing the auth-only flag now fails instead of accepting an implicit prompt.

`adapter_username_reaches_child_without_forwarding_credentials` checks a real
child process with USER present and credentials/control handles absent. All Claude
fixtures now require USER for auth and execution, and assert the safe auth argv
boundary. The newly documented moving alias `best` is also rejected, with the
existing malformed-configuration fixture covering it.

A private disposable project was prepared for one `claude-sonnet-5` invocation at
medium effort, with a 120-second goal deadline, recovery disabled, unchanged
behavior and local Python verification. Its task adds a docstring to a small
addition function. Account/executable fingerprints and profile evidence stay
outside the repository. No global profile has been enabled by this preparation.
Follow-up validation: `RUST_TEST_THREADS=4 cargo test --locked` passed **386
tests, zero failures, one pre-existing ignored test**. Formatting, diff checks,
Clippy with warnings denied, and the release build passed. The rebuilt release
binary passed the isolated Claude direct-execution fixture. A temporary diagnostic
called the actual Rust `claude::preflight` against the prepared private profile;
it passed with the installed CLI and was then removed. No model was called.
Logs are `/private/tmp/dispatch-claude-login-{tests,clippy,build,release-smoke}.log`.
These fixes add 15 production lines, bringing the current source to **23,822**
(**+1,073** against the preserved baseline), with no new migration or dependency.

Current official [model guidance](https://code.claude.com/docs/en/model-config)
lists Sonnet 5 as Pro's default, supports medium effort, and says its native
context window does not require usage credits. This supplies the intended Sonnet configuration; see the live result below for the auxiliary-model failure.

## First authorized live smoke and title-model correction

The user approved exactly one supervised invocation on the disposable docstring
task: `claude-sonnet-5`, medium effort, 120-second deadline, no recovery. The
provider exited zero with a successful terminal result and made the docstring
edit. Dispatch correctly rejected the attempt as **authorization failure**:
assistant messages named Sonnet 5, but final `modelUsage` also reported
`claude-haiku-4-5-20251001`. The single observed-model field stays unknown, both
identities remain in raw evidence, the rejected epoch remains durable, and no
retry occurred. Final Dispatch verification is `not_run`, review `not_requested`,
and application `not_applied`. A separate read-only `python3 -B -m unittest -q`
diagnostic passed; it does not rewrite the failed run as verified success.

The admission request was released, with zero remaining leases and exactly one
attempt. Original source code was unchanged. Candidate output included the
requested docstring and a Python bytecode-cache change; a second disposable
source ignores bytecode caches and runs verification with `-B`.
The failed workspace and original observations are preserved. Transaction cost
remains unknown. Raw nominal USD estimates are not cash charges. The aggregate
usage has its original provider semantics; per-model helper usage is not silently
merged into or discarded from that evidence.

[Anthropic's environment reference](https://code.claude.com/docs/en/env-vars),
checked on 2026-09-17, documents `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` as disabling
the background small/fast-model title request in print/SDK mode. Title generation
is a supported explanation for the extra Haiku use, not a purpose proven by the
result itself. The adapter now sets this through its controlled `--settings` env
object, shared by preflight and execution. Global settings and executor environment
permissions are unchanged by this correction. Model validation was not weakened.

The Claude fixture now reproduces auxiliary Haiku usage when that control is
absent. The pre-fix release binary reproduced the same authorization failure in
`claude_only_cli_identity_usage_review`. The second proposal retained the first
run's state/history and advanced the authorization revision to 2 only after explicit
revalidation and the user's separate approval. Its result is recorded below.

Correction validation passed: **386 tests, zero failures, one existing ignored**
with four-test concurrency; formatting, diff checks, warning-free Clippy and the
release build passed. The same direct release fixture that failed before the
control now passes. Logs are `/private/tmp/dispatch-claude-title-tests.log`,
`dispatch-claude-title-clippy.log`, `dispatch-claude-title-build.log`,
`dispatch-claude-title-reproduction.log` and `dispatch-claude-title-release-smoke.log`.
The production LOC count remains 23,822 (+1,073 from the preserved baseline).
Fixture success alone did not establish real CLI behavior; the second smoke
provided the separate live evidence below.

## Second authorized live smoke: passed

After the user explicitly approved the corrected proposal, the rebuilt release
binary ran exactly one additional supervised invocation. This used a fresh
disposable source with the same docstring task, the retained private state,
authorization revision 2, a 120-second deadline, and `--no-retry`.

- **Contract:** Claude Code 2.1.274, first-party claude.ai personal Pro,
  `claude-sonnet-5`, medium effort requested/resolved, standard service, usage
  credits disabled, and `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` in controlled
  per-invocation settings. Account and executable fingerprints remain private.
- **Identity:** requested, resolved and observed model all equal
  `claude-sonnet-5`; final `modelUsage` contains only that model. Observed effort
  remains unknown. The result does not prove every unreported internal action.
- **Execution:** Dispatch exit 0, one completed attempt, no recovery, 13,847 ms
  total. Terminal structured result is successful. Exactly one line was added
  to `calculator.py`: a docstring. No other files changed.
- **Verification/review:** configured `python3 -B -m unittest -q` passed (one
  test covering two additions). Work is ready, verification passed, review is
  pending, and application is not applied. Original source is unchanged.
- **Funding observation:** stream events reported `overageStatus: rejected`,
  `overageDisabledReason: org_level_disabled`, and `isUsingOverage: false`.
  Terminal usage reports standard service/speed and first-party model usage.
  These corroborate the user's current no-overage assertion; they are not a
  guarantee about future account settings or an invoice.
- **Usage:** final aggregate input 14, output 727, cache-read input 97,140,
  cache-creation input 2,419 (normalized sum 100,300 with Claude-specific
  semantics). Per-model and incremental counters are not added again. Raw
  nominal list-cost estimate is USD 0.036402; Dispatch cash cost remains unknown.
- **Cleanup/history:** admission released, zero remaining leases, no surviving
  Claude process or smoke supervisor process group, and zero sync outbox rows.
  The first failed attempt and rejected revision-1 authorization remain intact;
  the second run has exactly one completed attempt. No acceptance or application
  was performed, and no real failure/recovery was forced.

Private evidence is under `/private/tmp/dispatch-claude-live-gfshvrb7/`, including
`second-result.json`, `second-stderr.log`, `second-exit-code`,
`second-live-smoke-summary.json`, and the retained run artifacts/database. The
successful run is `01M2S4CED3ABB4GKN84MD8FGR2`; its attempt is
`01M2S4CEXYX3CDJYS50WAMH1TS`. Evidence should be retained beyond temporary-file
lifetime. The live test used the repository release binary and a private resource
profile; it did not enable Claude in the user's global Dispatch configuration.


CORE: **COMPLETE — READY FOR TWO-SUBSCRIPTION DOGFOOD**

LIVE CLAUDE SUBSCRIPTION: **VERIFIED FOR Claude Code 2.1.274 / personal Pro with usage credits disabled / claude-sonnet-5 / medium effort / standard service / controlled title-disabled print mode.**

This status combines fixture-validated mechanics with one successful bounded live
configuration. It does not establish universal account support, proven savings,
or release readiness. No commit/tag/push/publish or Phase 7 work was performed.
