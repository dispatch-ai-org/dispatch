## Dispatch 0.4.3 — setup you choose, Work you can read

0.4.3 is a polish and hardening release. It adds no new commands and changes nothing
about how coherence is judged or how results are applied.

**Setup is chosen, not typed.** Every setup step is a menu: ↑/↓ or j/k move, a digit
moves to that row, Enter chooses, Esc goes back. Plain mode (`--plain`, `TERM=dumb`)
shows the same menus as numbered lists.

- Codex lists its own models (`model/list`). Setup offers only the efforts both Codex
  and Dispatch accept. Claude Code cannot list models, so setup suggests fixed IDs.
  *Other model ID…* takes an exact ID, validates it, and shows it verbatim before you
  authorize.
- Authorizing is *Authorize and save* / *Cancel*, with focus on Cancel: Enter alone
  never authorizes. The full assertion is shown in scrollback first. Claude's
  print-mode, credits and account statements stay explicit.
- Each profile row shows whether it is ready and when a Claude authorization
  expires. Revalidating is two choices.
- Tier is no longer asked. It is still written, as `standard`, so older versions can
  read the file.

**Checks.** Checks setup also detects `npm test`, `go test ./...`,
`python3 -m pytest` and `python3 -m unittest` projects. *Other command…* saves one
command line you type.

**Launching versus protecting your own work.** A resource is needed only when
Dispatch launches the agent. Submitting a goal with no agent set up now offers
*Protect work I run myself*, which shows the `attach`, `finish`, `serve` and `check`
commands. Setup, `dispatch resources`, the `run` refusal, `serve` help and the README
all say so.

**One way to read a Work item.** `serve`, `history` and `status` show the same line:

```text
01M37J7H · native claude · S0 snapshot 99240318 · unmoved · ready · checks passed · review pending
```

- Origin, and the agent that did the work (native runs no longer show `dispatch`).
- What it began against: a snapshot, or for attached work a merge base or snapshot
  with its confidence.
- The verdict and its first reason. `unmoved` means the source has not changed;
  `not checked` means nothing has been evaluated yet.
- State, verification, and review.

`serve --json` adds `origin`, `s0`, `verification`, `review` and `applied_by`, and
existing fields keep their meaning. The one exception: `agent` now names the harness
for native runs. `history` shows WORK, STATE and VERDICT. `status` and `explain`
speak of the profile; allocation tiers are gone from the output.

**Fixed.**

- A deadline or cancel noticed right at the native engine's handoff returned an error
  instead of the result, so `dispatch run --json` printed nothing. The result and the
  deadline's exit code (124) are now always delivered.
- Reading a run (`status`, `explain`, every `serve` tick) rewrote every run's
  `metadata.json` even when nothing changed. Unchanged projections are now left alone.

**Tests.**

- Test files are named by topic.
- Fixtures drop the configuration left over from 0.4.0.
- Tests that do not test deadlines now have room for a loaded machine. The TUI
  spinner check polls instead of sampling.
- The native deadline test waits for the run's recorded deadline.
- PTY fixtures wait up to `DISPATCH_TEST_WAIT_SECS` (default 60) for each expected
  screen. A passing run is no slower.
- Load-sensitivity was measured over repeated full-suite runs, with results in
  `docs/plan-0.4.3-polish.md`. On the development machine, a real-time antivirus
  scanner made suite times swing more than sixfold, so the plan's twenty
  consecutive clean runs is not claimed.

**Evidence.** On this release, a real Claude Code run (`claude-sonnet-5`) and a
wrapped attach appeared together in `serve --json` with their origin, agent, S0,
state, verification and review. The run was accepted and applied, the project's
tests pass, and the view followed it to `applied` by a human with the review
accepted. The real Claude Code CLI also walked the new setup menus through to the
authorization screen, where Enter cancelled and nothing changed.

**Upgrading.** No migration; the schema stays at 24. The profile schema is
unchanged, so a `resources.yml` written by 0.4.3 setup still loads in 0.4.1 and
0.4.2.
