## Dispatch 0.2.0 — experimental developer preview

**Work coherence.** Before this release, any difference anywhere in the source tree
refused `dispatch accept`: an edited README, a `cargo build` writing `target/`, one
of your own commits. Dispatch now treats a finished run like an optimistic
transaction. At accept time it observes what changed underneath the run
(ignoring `.gitignore`d files), checks that the patch still applies, checks that the
declarations the work edited or relied on still hold (symbol-level for Rust and
Python, file-level otherwise), and, when the source moved and you configured
`checks.verify`, runs those checks on the merged result before applying.

- Unrelated edits and ignored build output no longer block acceptance.
- A changed callee signature, shared type, contract file, or same-symbol edit is
  reported as **REFRESH** with the reason (old => new signature); a patch that is
  already present is **STOP**.
- New commands: `dispatch check [run]` (read-only verdict) and
  `dispatch refresh [run]` (start a new run of the same task on the current source,
  with a note of what changed; never automatic, needs the same explicit flags as
  `run`).
- While an agent works, allocation runs record advisory verdicts
  (`coherence.invalidated`); `coherence.mid_run: stop` opts in to stopping the agent.
- `coherence.accept: strict` restores the old any-drift refusal.
- `status`, `explain`, the TUI review view and the control-protocol `result`
  show the verdict; `explain` reports agent time that ran after the work became
  invalid (time, never estimated dollars).

Also: README refresh and new logo, corrected exit-code and command documentation,
new technical reference `docs/coherence.md`.

**Upgrading.** No new database migration; state created by the published 0.1.x
releases still migrates to schema 20 on first open and keeps a private backup. Runs
created by older versions get coherence checking at accept time automatically.

**Limits (see `docs/coherence.md`).** Referenced symbols are bound by unique name,
not full name resolution; other languages use file-level facts; transitive behavior
changes are caught only if your checks cover them; integration checks run in a scratch
copy of the non-ignored files without a build cache and hold the apply locks while
they run; nested repositories are not analysed; planned (`--plan`) runs keep the
strict drift stop between tasks. Local execution is not a sandbox. Packages are
unsigned; interactive testing was on macOS arm64, Linux x86_64 is built and
smoke-tested by CI. No general savings, universal model support or live-planner
certification is claimed.
