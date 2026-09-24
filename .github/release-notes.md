## Dispatch 0.4.5 — Dispatch in the background

Turn watching on and get on with your work:

```text
$ dispatch start
✓ Watching ~/src/project
  Dispatch is running in the background. See it: dispatch watch · Stop: dispatch stop
```

While it runs, Dispatch keeps the verdict of every Work item it knows about current
as the code moves:
- results waiting for your review, native or attached;
- attached work still in progress;
- attached work it may apply.

No agent needs to be set up to watch a project.

**New commands.**
- **`dispatch start`** watches the project in the background and returns at once.
  It is one local process per project, in its own session, so closing the
  terminal does not stop it.
- **`dispatch watch`** shows the project's Work live, under a line saying who
  watches. Leaving it (Ctrl+C) does not stop watching.
- **`dispatch stop`** stops exactly this project's watcher.

`dispatch status` now ends with a `Project:` line saying whether the project is
watched.

**Trustworthy lifecycle.**
- "Watched" means the project's lock is held, and the operating system releases
  that lock when the watcher exits, crashes or the machine reboots. So nothing
  claims a project is watched when it is not, and `start` recovers after a crash.
- `stop` signals a process only when its recorded identity (pid, start time and
  boot) still matches.
- Two `start`s at once leave one watcher.
- After a reboot, run `dispatch start` again.

**Fixed.**
- The project owner re-checks results waiting for review when the code moves. A
  Ready result used to keep the verdict it finished with, so the project view
  could say CONTINUE while `check` said REFRESH. They now agree, and a refusal by
  the merged-tree checks still stands until the code moves.
- A verdict that flipped back within a minute was never recorded, and a restarted
  `serve` ignored a stored verdict that no longer held. The owner now compares
  each verdict with the one stored on the run.
- Attached work with no live owner is re-checked when that work changes, not only
  when the project moves (the gap 0.4.4 closed for native runs).
- Following attached work used to re-hash the whole workspace every tick. The
  owner now keeps a Git index per Work item, cutting the cost fourfold (about
  50 ms per item on a 2,000-file project).
- Results waiting for your review stay in the project view until you decide.
  Reasons quoting Git's errors stay on one line.

**Evidence.** A real-agent trial on a small Python service:
- `dispatch start` returned in 0.1 s.
- On its first tick the watcher found the four results left waiting for review
  that morning and marked them correctly: two REFRESH `patch_conflict`, one
  REFRESH `fact_broken` (a caller of `validate(token)` after the API became
  `validate(ctx, token)`), and one STOP (already landed).
- A native Claude Code result and a foreign-attached Cursor session then ran in
  parallel. Two seconds after a teammate's commit touched the same code,
  `dispatch watch` showed the Claude result as REFRESH, with no command run.
- After SIGKILL, `status` and `watch` said "not watched", `stop` signalled
  nothing, and `start` recovered.

**Cost** (measured on a 2,000-file Git project; no telemetry is collected):
- An idle tick costs about 50 ms for the project signal, plus about 50 ms for
  each attached Work item in progress.
- When the code moves, each affected item takes about 0.4-0.5 s to re-evaluate.

**Not built.** Dispatch does not discover agents it did not launch or that you did
not attach, and it does not start at login. There is still no network service,
protocol or automatic refresh.

**Upgrading.** No migration; the schema stays at 24. The state directory gains
`watchers/`. `watch --json` adds a `watcher` object, and `serve --json` is
unchanged.
