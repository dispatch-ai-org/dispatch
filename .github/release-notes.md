## Dispatch 0.4.0 — Dispatch service and attach

**Attach.** Work an external coding agent produces — Claude Code, Codex, Cursor, a
script, anything with a terminal — can now be observed, judged and, if you ask,
applied under the same coherence model as work Dispatch launches itself, without
Dispatch ever driving that agent. This release ships:

- A repo-scoped local service, `dispatch serve`: one foreground process per
  integration root, no daemon, coordinating through the SQLite state and `flock`
  files the codebase already trusts.
- A shared view of the integration world: one line per run — attached and native
  together — `<id> · agent · CONTINUE/REFRESH/STOP · working/ready/applied/blocked ·
  reason`, redrawn in place, that renders every tick but prints only what changed.
- Attached Claude/Codex/other-agent work, in two forms: `dispatch attach -- <command>`
  wraps and owns the agent's own terminal session silently, and `dispatch attach
  --workspace <path>` observes an agent already running in its own worktree, finished
  explicitly with `dispatch finish`.
- Dispatch-native and foreign Work participating in the same coherence model: an
  attached run is an ordinary `RunRecord` (`mode: attached`), so `check`, `explain`,
  `accept`, `reject`, `apply`, `auto_apply` and the watcher all treat it exactly like
  work Dispatch launched, with one added rule — an attached run's gate never takes the
  unmoved-fingerprint shortcut, because its S0 is a Git merge-base commit rather than
  the tree its fingerprint was taken from.
- Serialized coherent integration: `serve`, the wrapped attach owner loop, the TUI and
  the CLI all apply through the same `auto_apply`, serialized by the same per-source
  lock; whoever holds it validates against the current world and the rest wait or fail
  closed.
- Restart/recovery semantics: a `serve` restart reads active attached Work back from
  the database and re-checks each owner's process identity — a live wrapper is left
  alone, a gone one is honestly adopted (`attach.adopted`) — without finishing,
  applying or relaunching anything.

**Defects found and fixed on the way**, by the S6 scenario tests and by two rounds of
real-agent dogfood:

- `serve` re-rendered its project view only on its own verdicts or applies, so a run
  attached or finished by another process never appeared until something of `serve`'s
  own changed. Fixed: the view renders every tick regardless.
- `auto_apply` applied an empty Δ (an agent that edited nothing, or touched only
  ignored paths) as "0 files changed" instead of leaving it reviewable. Fixed with a
  new `empty_delta` skip reason.
- `dispatch explain` showed no coherence verdict for an applied attached run: the gate
  took the unmoved-fingerprint shortcut meant for native runs, whose fingerprint is
  taken from the same tree S0 was copied from — an attached run's S0 is a merge-base
  commit instead, so the shortcut silently skipped evaluation and the integration
  checks. Fixed: attached runs always evaluate.
- A projection-write race: `State::save_run` and the event-projection repair every
  unlocked reader (`serve`'s view, `status`) performs both wrote to a fixed temporary
  file name, so two processes finishing or observing at once could consume each
  other's temporary file and fail with a bare `ENOENT` mid-finish. Fixed with a
  uniquely named temporary file and an atomic rename per write.

**Not included in this release.** No socket and no daemon — coordination stays SQLite
plus `flock`, and `serve` discovers new or changed Work on its next poll tick, not
instantly. No process control of foreign agents: no PID is ever signaled or killed,
`--pid` is liveness-only, and `mid_run: stop` does not apply to attached work — a
`STOP`/`REFRESH` verdict on it is recorded, never enforced. No PR or GitHub/GitLab
integration. No automatic refresh: `dispatch refresh` stays refused for attached work
entirely (there is no Dispatch task to relaunch); finish or reject it instead.

**Upgrading.** Migration 21 (`attached_work_mode`) rebuilds `runs` the way migration
13 did, to widen its `run_mode` `CHECK` to admit `'attached'`; a `dispatch.schema-20-
*.db` backup is created first, and an older binary refuses the upgraded schema. No
existing run is rewritten beyond the table rebuild itself. See `docs/release-install.md`.

**Evidence.** Real-agent attach with Claude Code (`claude-sonnet-5`, print mode) in a
linked worktree of a Python scratch project, `serve --json` watching the root: the
agent created a module and its test; `dispatch finish` froze a two-file Δ and ran the
unittest check; the root had moved four files ahead of S0; `--auto-apply` evaluated
the moved world, ran the integration checks on the merged tree, and applied
(`analysis: integration`); `serve` showed the run go `working` → `applied` with
verdict `continue`; the root's tests pass. Everything else — the scenario suites, the
restart and adoption tests, the migration tests — is fixture evidence with scripted
agents, not a claim about real-agent behavior at scale; see
`docs/coherence-validation.md` for what is claimed and what would falsify it.
