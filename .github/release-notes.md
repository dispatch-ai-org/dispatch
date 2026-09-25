## Dispatch 0.4.6 — Work that appears on its own

Run your agent the way you already do, and Dispatch picks up its work:

- **Claude Code sessions become Work by themselves.** With Claude Code's hooks
  installed (`dispatch setup` → Runtime integrations, shown and approved first),
  a session in its own worktree of a project you watch (`claude --worktree`)
  becomes Work with no Dispatch command.
  - S0 is the worktree as the session found it, captured before its first edit.
  - Later sessions in that worktree join the same Work.
  - Its verdict follows the project as it moves.
- **Any agent gets its own workspace.** `dispatch attach -- <agent>` run from your
  checkout now makes the workspace. It sits outside the checkout, on its own
  `dispatch/…` branch, and starts as the checkout's exact world, uncommitted
  files included. The agent needs no worktree support of its own.
  - The workspace is removed once its work is applied, and kept after a reject
    or a crash; `dispatch status <id>` says where it is.
- **Nothing is claimed where it can't be.** A session running directly in your
  checkout is told that Dispatch cannot tell its edits from yours, and nothing is
  tracked.

**A removed worktree keeps its work.** Claude Code can delete a worktree at
session exit or through its `ExitWorktree` tool.
- Before either, Dispatch writes the work's exact final changes durably into the
  run and records the removal. Only then does the hook let the deletion go ahead.
- If that cannot be done, the hook fails and the worktree stays.
- Removal never finishes the work for you: `dispatch finish` verifies it in a
  workspace rebuilt from S0 and the kept changes, or you `reject` it.
- A worktree with no changes simply closes.

**Fixed.**
- **Finishing no longer fails on ignored build output.** `dispatch finish` refused,
  and marked the work failed, when the workspace held over 4 GiB, 200,000 files, a
  file over 512 MiB, or a symlink out of the tree. That included ignored `target/`
  or `node_modules/`. Only paths that can enter the changes count now.
- **Process identity on macOS no longer drifts.** A process's identity included
  `kern.boottime`, which the clock adjusts while the machine runs, so live
  processes gradually looked "reused": `dispatch stop` refused, and a live
  wrapped attach could have been adopted as orphaned.
  - The identity now uses the boot session's UUID.
  - Identities recorded by older versions still match by pid and exact start
    time, so `dispatch stop` recognises an owner started before upgrading.
- **The project view tells you more:**
  - it names where work came from: `native`, `attached`, `discovered` or
    `isolated`;
  - it says `idle`, `removed`, `lost` and `rejected` where it said `working` or
    `ready`;
  - it stops calling a moved world "unmoved".

**Evidence.** Real-agent trials with Claude Code 2.1.280 and Cursor Agent:
- A `claude -p --worktree` session registered itself, and its Work was created 6 s
  before the agent's first edit.
- A teammate's commit moved its verdict with no command run.
- Removing the worktree through `ExitWorktree` kept the exact changes. The trial
  found that this tool does not run `WorktreeRemove`, which is why Dispatch now
  also hooks it.
- The work was then finished in a rebuilt workspace and applied.
- `dispatch attach -- cursor-agent` from the checkout worked in its own workspace
  while a teammate committed. It was applied, and its workspace released.
- A wrapper killed mid-run left its workspace intact, and the owner adopted the
  work.
- Replayed and resumed sessions never made duplicate work.

**Not built.** Codex's hooks exist, but every hook definition needs the user's
trust review and its session end also means "idle", so a Codex adapter waits;
`dispatch attach -- codex` isolates Codex today. There is no process scanning,
automatic verification, start at login, or retention cleanup of rejected
workspaces yet.

**Upgrading.** No migration; the schema stays at 24. State is forward-only: 0.4.5
cannot read runs that use the new attachment fields. Install Claude Code's hooks
through `dispatch setup`; install them again if you move the binary or the state
directory.
