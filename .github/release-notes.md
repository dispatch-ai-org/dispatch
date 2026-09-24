## Dispatch 0.4.4 — verdicts you can trust and act on

A multi-agent trial ran eleven Work items, from Claude, Claude Code and Cursor,
against one project while it changed underneath them. Dispatch caught stale work
where plain `git apply` would have let it through. The trial also showed where its
verdicts could not yet be trusted or acted on. 0.4.4 fixes those places and lets a
human overrule a verdict the analysis got wrong.

**Fixed.**

- **A refused accept no longer records an acceptance.** When coherence refuses an
  accept, nothing is applied, no review is recorded, and the result stays pending
  for you to refresh or reject. Accept now applies first and records your review
  only once the change has landed.
- **Every surface shows the same verdict.** After the checks on the merged tree
  refused a result, `check` and `status` showed CONTINUE, and `check` advised accept
  again, while `serve` showed REFRESH. They now all show the refusal until the
  source moves. `refresh` passes the failure on to the new agent.
- **Integration failures say what failed.** The reason quotes the failing check in
  its own words, for example `ERROR: test_admin_token (…) / TypeError: validate()
  missing 1 required positional argument: 'token'`, not only a log path.
- **The mid-run watcher also follows the agent's work.** It used to re-evaluate only
  when the source moved. A change that landed before the agent touched the same
  code was therefore never reported mid-run. It now re-evaluates when either the
  source or the agent's work so far changes. The work is fingerprinted with the
  trusted baseline repository, so ignored build output never counts.
- **Adding an optional Python parameter keeps callers valid.** A referenced Python
  function that gains only defaulted parameters, `*args` or `**kwargs` no longer
  marks every caller `fact_broken`. Any other signature change still does, and
  Rust signatures compare exactly.
- **Clearer words:**
  - attached work is told to run its agent again, not to `dispatch refresh`;
  - native S0 shows the project commit;
  - a result applied to an unmoved source shows `unmoved`;
  - STOP names the run that already landed the same change.

**New: recorded human override.** `dispatch accept <run> --despite-refresh
--explanation "<why>"` applies a REFRESH that comes only from the file and symbol
analysis, when you have checked that the work still holds.
- Your checks must still run and pass on the merged tree.
- The overridden verdict and your explanation are recorded (`coherence.overridden`).
- It never overrides STOP, a patch that no longer applies, or a failing check.
- Auto-apply never uses it.

**Evidence.** The same trial was re-run with real agents on this release:
- The Claude Code patch that still called `validate(token)` after the API changed
  was refused as before. An override attempt was refused too, because the merged
  tree's tests failed with that `TypeError`, which the refusal quotes.
- The redundant timeout change was STOP, "landed by" the run that applied it first.
- The format change that broke a landed test was refused by the checks, and
  `check`, `status` and `serve` agreed afterwards. The refused accepts stayed
  pending.
- The Cursor caller of `format_user(user)` now applies after the optional parameter
  landed; 0.4.3 refused it.
- Given the failing test, the refreshed agent asked whether it could update that
  test, where 0.4.3's refresh repeated the failure.

**Upgrading.** No migration; the schema stays at 24. `CoherenceRecord` gains an
optional `overridden` field. `serve --json` gains `overridden`, and `check --json`
gains `landed_by` for STOP.
