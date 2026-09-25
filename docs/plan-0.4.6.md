# Plan: Dispatch 0.4.6, Work that appears on its own

## Context

In 0.4.5, a background owner (`dispatch start`) keeps the verdicts of known Work
current. The missing piece is Work appearing without the developer registering it.
This release delivers a vertical slice toward that. It is grounded in what the code
and the runtimes actually do today (inspected on 2026-09-24), and was agreed after
review.

**Public promise.**
- Claude Code sessions in their own worktree appear in `dispatch watch`
  automatically, with S0 captured before the session's first edit.
- Any agent started with `dispatch attach -- <agent>` from your checkout gets its
  own Dispatch-made worktree.
- A session running directly in your checkout is told plainly that Dispatch can't
  separate its edits from yours; nothing is claimed about it.

## Decisions

1. **The Work boundary is the workspace, not the runtime session.** Sessions come
   and go within one workspace (Claude `/clear`, `--resume`, compaction; Codex
   `SessionEnd` on idle). A session is an actor recorded on the Work. There is at
   most one active Work per workspace.
2. **Shared-checkout sessions are not Work in 0.4.6.** In a shared checkout the
   workspace is the target, so Δ has already landed in the world and there is
   nothing to review or integrate. The session gets a notice (a Claude
   `systemMessage`) pointing to isolation, and no state is kept. Tracking
   concurrent writers is the later conflict-graph milestone.
3. **Managed isolation for Git roots is a Dispatch-made linked worktree built from
   an exact S0 commit, not the copied candidate workspace.** `create_snapshot`
   copies ignored build output (30 GB for this repository's `target/`), and
   `collect_diff` then refuses anything over 4 GiB or 200,000 files. Plain
   directories keep the copied workspace.
4. **No Codex adapter in 0.4.6.**
   - Every Codex hook definition needs the user's trust review.
   - Its `SessionEnd` also fires on idle.
   - Its worktrees are experimental and off.
   - It can't be trialled under the standing rule.

   `dispatch attach -- codex` isolates Codex without hooks. The seam keeps a later
   adapter small.
5. **A session ending never ends Work, and verification is never automatic.**
   `checks.verify` still needs `--allow-unsafe-local` from a human; there is no new
   permission.
6. **Workspace removal is an observation, not a completion.**
   - When a runtime deletes the workspace, Dispatch freezes the exact Δ and records
     that the workspace was removed. The Work's lifecycle does not change: it
     waits for a human `dispatch finish` or `dispatch reject`.
   - This holds even when no checks are configured. Removal is often the user
     discarding the work at Claude's exit prompt.
   - The one automatic case: an empty Δ closes as "no changes".
7. **Invariant: for a `WorktreeRemove` event, the final exact Δ is durably
   persisted before the hook returns.** Deletion destroys the only authoritative
   source.
   - Durable means: the patch is written and fsynced, then atomically renamed into
     the run directory, and the observation is committed to the database before
     the hook exits 0.
   - If Dispatch cannot do this (an error, or its own deadline before the hook's
     timeout), the hook **exits non-zero**. Claude then keeps the worktree, and
     the message says why. Failing closed here is the only way not to lose the
     work.
   - The hook never blocks removal in any other case: an unwatched project, a
     workspace with no Work, or an empty Δ.
8. **Recovery metadata lives on the run, not in a product-level list.**
   - A Dispatch-made workspace is removed only after its Work is applied.
   - It is kept after a reject, a crash or an unknown state. The reject message
     prints its path.
   - `dispatch status <id>` shows it. Retention cleanup is 0.4.7.
9. **`RunMode` stays Native/Attached.** Mode records who executed the work.
   Provenance, isolation and sessions live on the attachment.
10. **Wrapped attach becomes the managed launch.** `dispatch attach -- <cmd>` run
    from the root stops refusing ("attach needs a separate worktree") and creates
    the managed workspace. From an existing worktree it is unchanged. There is no
    new verb and no fourth execution path.

## Verified current state

- **Owner** (`serve.rs`, 0.4.5):
  - follows active unowned attached Work by a hash of its patch, with a kept index;
  - re-checks Ready Work with `live_validity`;
  - adopts orphans and auto-applies;
  - one owner per root, guarded by `flock`.
- **Attach** (`attach.rs`). Neither form creates a workspace.
  - **Foreign:** Git worktrees only. S0 is `merge_base(root, workspace)`, through
    `materialize_baseline_from_commit` (Full confidence).
  - **Wrapped:** runs in the current directory, which must not be the root. A plain
    directory's S0 is a `create_snapshot` of the workspace (Partial).
  - **What wrapped attach already does:** runs the agent on the terminal in the
    wrapper's process group, forwards signals, runs the mid-run watcher, calls
    `finish_locked` on exit, and auto-applies if asked. It is about 80% of a
    managed launch.
- **Native isolation.**
  - `create_snapshot` copies every file except `.git`/`.dispatch`, checks
    fingerprints before and after, and commits the copy to an internal
    repository. The commit honors ignore rules; the copy does not.
  - `create_candidate_workspace` clones that repository and copies the tree.
- **Finish.** `finish_locked` runs `collect_diff`, whose `validate_candidate_tree`
  walks every file, including ignored ones, and refuses symlinks that point
  outside the tree. It then runs `checks.verify`, which needs `unsafe_local`.
- **World.** Nested repositories are already excluded (`list_git_world`), so a
  Claude worktree under `.claude/worktrees/` does not corrupt the root's world.

## Runtime lifecycle, verified 2026-09-24

**Claude Code** (docs: hooks, worktrees):
- **`SessionStart`**
  - `source`: startup, resume, clear, compact or fork. Payload: `session_id`,
    `cwd`, and sometimes `model`.
  - Fires after `WorktreeCreate` and before the first turn. It can't block.
- **`SessionEnd`**
  - `reason`: clear, resume, logout, prompt_input_exit or other.
  - A 1.5 s budget.
- **`WorktreeRemove`**
  - Fires before a worktree is deleted, with a 600 s default budget.
  - A non-zero exit fails the removal while the directory still exists.
- **`--worktree`**
  - Creates `<repo>/.claude/worktrees/<name>/` on branch `worktree-<name>`.
  - Branches from the remote default branch by default (`worktree.baseRef: head`
    changes this) and checks out tracked files only.
  - On exit it keeps the worktree or removes it along with its work.
- **Resume, fork and paths.** Resume re-enters the worktree; `--fork-session`
  starts in the launch directory. The `cwd` field follows `cd`, while
  `CLAUDE_PROJECT_DIR` stays at the original project root.
- **Configuration.** Hooks merge across user, project and local settings, run in
  parallel, and a handler defined in several places runs once.

**Codex** (0.155.1, docs: hooks):
- `SessionStart`: startup, resume, clear or compact.
- `SessionEnd`: on close, archive or delete, and after idle. A 1 s budget (3 s at
  most).
- Hooks are read from `~/.codex` and the project's `.codex/`, and each needs
  explicit trust. An open report says repository-local hooks don't fire
  interactively (openai/codex#17532).
- Worktrees are experimental and off.

## Design

**Runtime seam.** A hidden `dispatch hook <provider>` command.
- **Input.** The provider's JSON on stdin, at most 64 KiB.
- **Adapter.** A thin provider adapter (`src/runtime/claude.rs`) parses it into
  `RuntimeEvent { provider, session_id, kind: Start{source} | End{reason} |
  WorkspaceRemoved, cwd, model }`.
- **Core.** `runtime::ingest` is provider-neutral. It writes through the existing
  locks and `persist_event`, and the Owner picks the Work up on its next tick.
  There are no sockets.
- **Isolation from coherence.** The coherence engine never sees runtime events.
- **Exit code.** The hook exits 0 except under invariant 7. It never blocks the
  agent's start.

**Trustworthy automatic Work.** Dispatch derives every one of these itself:
- The integration root is the main worktree of `cwd`'s Git common directory.
- That root is watched: its serve lock is held.
- The workspace is a separate linked worktree of the same repository.
- S0 is captured at `SessionStart` (startup or fork) before any turn.

A resume into a workspace Dispatch has not seen gets Partial confidence. Nothing in
the payload chooses a root, a path to delete or a process to signal.

**Isolation model.**

| Case | Workspace | S0 | Confidence | Work? |
|---|---|---|---|---|
| Runtime-native (Claude `--worktree`, or your own worktree) | runtime or user | the workspace's world at `SessionStart` | Full, or Partial on a first-seen resume | yes |
| Dispatch-managed (`attach -- cmd` from the root) | a Dispatch worktree at `<state>/workspaces/<id>`; a copy for plain directories | the root's world at launch | Full | yes |
| Shared checkout | the root | — | — | no; notice only |

**S0 primitive: `source::world_commit(path)`.** It works like `git stash create`:
a temporary copy of that checkout's index, `add -A` honoring ignore rules and
skipping nested repositories, then `write-tree` and `commit-tree`. The result is an
exact commit of the world with no copying.
- The baseline is the existing `materialize_baseline_from_commit`.
- The managed workspace is `git worktree add -b dispatch/<id8>
  <state>/workspaces/<id> <commit>`, which equals S0 byte for byte.

**`SessionStart`.**
1. Validate the input:
   - `session_id` matches `[A-Za-z0-9_-]{1,128}`;
   - `cwd` is absolute, an existing directory, and is canonicalized;
   - the payload is within the size limit.
2. Derive the root. If it is not watched, exit 0.
3. If `cwd` is in the root's own checkout, show the shared-checkout notice and
   stop.
4. Take the per-workspace lock (`locks/workspace-<sha>.lock`).
5. If there is an active attachment for the workspace, add the session to it.
   This covers resume, clear, compact, duplicate events, and a fork into the same
   workspace.
6. Otherwise create the Work:
   - S0: `world_commit(workspace)`, provenance `WorkspaceAtStart`;
   - confidence: Full, or Partial on a first-seen resume;
   - `workspace_owner: Runtime`;
   - the session record.

   `world_commit` is capped at 10 s. Past the cap the session is not tracked and
   the notice says so.

**`SessionEnd`.** Record `ended_at` and the reason on the session, nothing else.
The Work stays active and unowned, and the Owner keeps following it.

**`WorktreeRemove`** (invariant 7):
1. Take the run lock, waiting up to a bounded time.
2. `collect_diff` into a temporary file, fsync it, and rename it into the run as
   the frozen Δ.
3. Commit a `workspace.removed { exact: true }` observation.
4. Exit 0 once all of that is done. Exit non-zero if any step failed or the
   deadline passed; the deadline sits below the hook timeout that setup installs.

An empty Δ closes the Work as "no changes". Anything else waits for a human.

**Finish without the workspace.** `dispatch finish` on Work whose workspace was
removed verifies in a workspace rebuilt from the baseline plus the frozen Δ, using
`create_candidate_workspace` and `git apply`.

**Workspace gone with no hook.** The Owner records `workspace.removed { exact:
false }` with its last following snapshot, marked approximate and not reviewable.

**Idempotency.**
- Work identity is the canonical workspace path plus the repository key.
- Duplicate starts land on the same Work, serialized by the workspace lock.
  Duplicate ends are no-ops.
- A resume never re-captures S0. A fork makes new Work only in a different
  workspace.

**Setup.** `dispatch setup` gains Runtime integrations → Claude Code.
- It previews the exact JSON for `~/.claude/settings.json`: `SessionStart`,
  `SessionEnd` and `WorktreeRemove` hooks running `dispatch hook claude`, with a
  generous `WorktreeRemove` timeout.
- Consent is selected, with focus on Cancel.
- It makes a backup, writes atomically, and preserves unrelated keys.
- Install is idempotent, keyed by our command. Uninstall removes exactly our
  entries.
- It is inert for unwatched projects. `dispatch start` never edits runtime
  configuration. Codex gets manual instructions only.

**Model.** Serde defaults only; the schema stays at 24.
- `AttachmentRecord.workspace_owner: User | Runtime | Dispatch`
- `AttachmentRecord.sessions: Vec<RuntimeSession { provider, session_id, source,
  started_at, ended_at, end_reason, model }>`
- `AttachmentRecord.workspace_removed: Option<{ at, exact }>`
- `AttachmentRecord.managed: Option<{ branch, kept_or_removed }>` for
  Dispatch-made workspaces
- `BaselineProvenance::WorkspaceAtStart { commit }`

State is forward-only: 0.4.5 cannot read runs that use the new values.

## Tests

- **Stage 1.**
  - Finishing succeeds with 250,000 ignored files.
  - An ignored `node_modules` symlink that points outside the tree is accepted.
  - A tracked symlink that escapes is still refused.
- **`world_commit`.**
  - Dirty, untracked and deleted files are captured.
  - Ignored files are excluded and nested repositories skipped.
  - The result equals a `create_snapshot` fingerprint.
- **Managed attach from the root.**
  - The workspace is outside the root, equals S0, and is on branch
    `dispatch/<id>`.
  - Δ is collected and applied.
  - Cleanup happens only after apply; the workspace is kept after a reject or a
    crash.
- **Hook ingestion** (fixture JSON):
  - An unwatched project is a no-op.
  - A foreign repository is refused.
  - A root `cwd` gets the notice and no Work.
  - Duplicate start, resume, clear and fork into the same workspace give one Work.
    A fork elsewhere gives new Work.
  - Oversized payloads, bad IDs and relative `cwd` are rejected.
  - `SessionEnd` finishes nothing.
- **`WorktreeRemove`.**
  - Δ is durable before exit 0: the frozen patch and the observation exist before
    the process exits.
  - A persistence failure exits non-zero and leaves the workspace in place.
  - An empty Δ closes the Work. A non-empty Δ waits.
  - `finish` works after removal.
- **Setup.** Install and uninstall are idempotent and preserve unrelated settings.

## Real-agent trials

These use Claude and Cursor only, with `caffeinate -i`, in the trial project.

- **A.** `dispatch start`, then `claude --worktree`.
  - Work appears, and S0 predates the first edit (compare timestamps).
  - The root moves and the verdict updates.
  - Exit with removal: Δ is durable and the Work is kept.
  - Then `finish` and `accept`.
- **B.** `dispatch attach -- cursor-agent …` from the root.
  - Check S0 and the workspace.
  - The world moves.
  - The agent exits; finish and review.
  - Kill the wrapper, and the workspace survives.
- **C.** `claude` in the root. The notice appears, no Work is created, and
  nothing is applied.
- **D.** Recovery:
  - duplicate hook replay;
  - owner SIGKILL, then `start`;
  - `claude --resume` into the worktree lands in the same Work;
  - killing the Claude process leaves the Work unowned and followed.

## Stages (one commit each on `release-0.4.6`)

| # | Stage |
|---|---|
| 0 | This plan; repository links updated to `rundispatch/dispatch`. |
| 1 | Workspace-scale finish: `validate_candidate_tree` covers only paths that can enter Δ. |
| 2 | `world_commit` and the `WorkspaceAtStart` provenance. |
| 3 | Managed isolation in wrapped attach; cleanup only after apply. |
| 4 | Runtime seam, the Claude adapter, and `dispatch hook claude` (start, end, notice). |
| 5 | `WorktreeRemove`: durable exact Δ (invariant 7), the removal observation, and finish without the workspace. |
| 6 | Setup: install and uninstall runtime integrations. |
| 7 | View and status: origin (`claude worktree` / `dispatch worktree`), session state, workspace removed, recovery metadata on the run. |
| 8 | Trials A–D, docs and version 0.4.6. |

## Risks

- **Hook latency.** `world_commit` uses the checkout's index stat cache and a 10 s
  cap.
- **What Claude does when a `WorktreeRemove` hook times out.** Verify in trial A.
  Dispatch's own deadline sits below the installed timeout, so it always decides
  first.
- **Claude hook schema drift.** Parse tolerantly and record the Claude version.
- **Editing user-level settings.** Mitigated by preview, backup and exact
  uninstall.
- **Missing ignored dependencies.** Managed worktrees lack them, as `claude -w`
  worktrees do. This is documented.
- **Forward-only state after upgrade.**

## Definition of done

- In a watched project, a `claude --worktree` session appears in `dispatch watch`
  with no Dispatch command.
  - Its S0 predates its first edit, and its verdict follows the root.
  - Removing the worktree durably preserves the exact Δ for a human decision.
- `dispatch attach -- <any CLI>` from the root isolates the agent, and the Work
  reaches review and apply. The workspace is removed only after apply.
- A session in the root gets a notice and never becomes Work.
- Replayed events, restarts and resumes never create duplicate Work or a new S0.
- Setup installs and uninstalls hooks exactly, with consent.
- Trials A–D are logged. fmt, clippy and the full suite are green.

## Deferred

- **0.4.7, "finish without a terminal":**
  - per-project consent to run `checks.verify` when discovered Work ends;
  - accept and reject from `watch`;
  - retention cleanup of rejected managed workspaces;
  - the Codex adapter, once it can be trialled.
- **0.4.8, "Work that notices Work":** warnings when concurrent Work overlaps; this
  is the real need behind shared checkouts.
- **Later:**
  - tracked shared-checkout Work;
  - Dispatch acting as Claude's `WorktreeCreate` provider;
  - `.worktreeinclude`-style copying;
  - process scanning, conflict graphs, indexed invalidation, start at login.

## Progress log

- 2026-09-24: Stage 0. Plan agreed with the review revisions: workspace removal is
  an observation, not a completion; recovery metadata stays on the run; invariant
  7 (durable exact Δ before `WorktreeRemove` returns). Repository links point at
  `rundispatch/dispatch`.
- Stage 1. Workspace-scale finish.
  - Reproduced: an attached worktree whose ignored `build/` held a file over
    512 MiB was marked **failed** by `dispatch finish`, leaving nothing to
    review. The same happens for a symlink out of the tree, such as a
    `node_modules` link, or for more than 200,000 ignored files.
  - `validate_candidate_tree` now applies its unchanged limits and symlink rules
    only to paths that can enter Δ: the baseline's tracked paths plus the
    workspace's untracked, non-ignored paths, as the trusted baseline repository
    lists them for `git add -A`.
  - A tracked directory replaced by a symlink is validated as that symlink.
  - The obsolete ignored whole-tree timing test is removed.
  - Tests:
    - `ignored_build_output_never_limits_or_refuses_the_delta` (unit): tracked
      paths are still refused for an oversized file, an absolute link, or a
      symlinked parent;
    - `finish_succeeds_after_a_build_left_ignored_output` (end to end).

    Both fail on the old code.
  - Full suite: 463 passed.
- Stage 2. `source::world_commit(checkout)` records a checkout's exact world as
  a commit, the same way `git stash create` does.
  - It uses a copy of the checkout's own index (for the stat cache) and `add -A`,
    which honors ignore rules and skips untracked nested repositories. HEAD is
    the parent.
  - The user's real index and status are untouched.
  - It is materialized by the existing `materialize_baseline_from_commit`.
  - `BaselineProvenance::WorkspaceAtStart { commit }` is added, with display arms.
  - Test: `world_commit_records_exactly_the_world_and_leaves_the_checkout_alone`,
    covering dirty, deleted, untracked, ignored and nested files, and a repository
    with no commit yet.
- Stage 3. Managed isolation in wrapped attach.
  - `dispatch attach -- <cmd>` run from the checkout itself now makes the
    workspace under `<state>/workspaces/<run-id>`.
    - For Git: `world_commit(root)` checked out by `source::create_linked_workspace`
      on branch `dispatch/<run-id>`, with S0 materialized from the same commit.
    - For a plain directory: `create_snapshot` plus `create_candidate_workspace`.
    - Either way the provenance is `WorkspaceAtStart` and confidence is Full.
  - The attachment records `workspace_owner: dispatch` and `managed { branch,
    removed }`.
  - `persist_applied` releases the workspace (`workspace.released`) once the Work
    is applied, and only a path directly under `<state>/workspaces/`. A failed
    release is reported and the workspace kept.
  - A reject keeps the workspace and prints its path. The wrapper prints one line
    before the agent starts, saying where the agent works.
  - The foreign-attach refusal now mentions the wrapped form.
  - Tests:
    - `wrapped_attach_from_the_checkout_works_in_a_workspace_dispatch_makes`:
      S0 includes uncommitted checkout files, the checkout is untouched, accept
      applies and releases;
    - `a_rejected_workspace_dispatch_made_is_kept`;
    - `wrapped_attach_from_a_plain_directory_works_in_a_private_copy`.
  - Full suite (stages 2-3): 467 passed.
