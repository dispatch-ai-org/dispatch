## Dispatch 0.3.0 — Auto-apply and coherence-driven integration

**Auto-apply.** A finished result can now be applied to the source automatically,
under an explicit session mode or CLI flag, instead of always waiting for human
review. Application stays mechanical and separate from acceptance: an auto-applied
run never records a human review, so `outcome.review` stays `pending` until you
look at it. This release ships:

- A visible TUI auto-apply mode: off at the start of every session, toggled by
  Shift+Tab on any screen or `/auto-apply on|off`, shown as the first segment of the
  hint row (`⏸ review before apply · Shift+Tab` / `⏵⏵ auto-apply on · Shift+Tab to
  pause`), plus one review action, `[aa] Accept & apply, then auto-apply the next
  results`.
- Safe automatic application: a policy applies only a verdict whose evidence is
  complete for the exact world it names — an unmoved world with passed candidate
  verification, or a moved world whose merged tree passed your own `checks.verify`
  (`analysis: integration`). Verification must be configured; without it, nothing
  is ever applied automatically.
- Final-boundary coherence revalidation: the gate re-checks the world under the
  run's and the source's locks immediately before applying, through the same
  digest and fingerprint fences human accept already uses.
- Concurrent-run behavior: two runs finishing against one source serialize on the
  per-source lock; the second is re-judged against the world the first one
  produced. An edit during integration checks is fenced and re-validated once.
- The distinction between human review and policy application is preserved
  end to end: `RunOutcome.applied_by` (`human` | `auto_apply`) records who applied
  a result; a human can still accept or reject an auto-applied run afterward, which
  records the review only — there is no second apply, and rejection does not revert
  the source.
- `dispatch run --auto-apply` and `dispatch refresh --auto-apply`: after the run
  returns Ready, the same process attempts the application and prints the outcome;
  `--json`/`--jsonl` carry a `result.auto_apply` object and a trailing
  `{"type":"auto_apply"}` line.
- Enough dogfooding to prove the mechanism works: concurrent auto-apply sessions,
  a held run lock, and a source edited during integration checks are covered by
  `tests/auto_apply.rs`, `tests/auto_apply_cli.rs` and
  `tests/auto_apply_concurrency.rs`.

**Not included in this release.** No automatic refresh — `dispatch refresh` stays an
explicit, human-typed command. No `dispatch.yml` configuration key for auto-apply;
the policy lives only in the process that owns the run and is never persisted. No
control-protocol auto-apply: machine clients still cannot review or apply, and a
run applied by policy is visible in `status`/`result` (`outcome.applied_by`) but not
triggerable over the protocol. No Dispatch service or `attach`/`serve`/attached
external work; that is planned for 0.4.0.

**Upgrading.** No new database migration; the schema version stays 20.
`RunOutcome.applied_by` is an additive optional field, omitted (not `null`) on
unapplied runs and on runs written by older versions. Every session still starts
with auto-apply off.

**Evidence.** The only dogfooding done for this release used a scripted fixture
agent behind the Codex adapter, not a real coding agent: three concurrent runs on a
Python project, applied one after another once all three had finished, behaved
exactly as the authorization table predicts (one applied on an unmoved world, one
applied after integration checks on a moved world, one blocked as REFRESH with the
source untouched). Treat this as fixture evidence, not a claim about real-agent behavior;
see `docs/coherence-validation.md` for what is claimed and what would falsify it.
