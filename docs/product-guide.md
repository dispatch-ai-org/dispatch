# Work with Dispatch

Start from your project directory using the candidate's **absolute binary path**.
Ordinary goals are direct, with at most two supervised invocations. `/plan <goal>`
is an explicit sequential experiment, not a mandatory stage of every task.

## Setup and resources

`dispatch setup` and the session's `/resources` action share the same handlers.
`dispatch setup codex` or `dispatch setup claude` starts a new resource; choose an
existing profile number to revalidate it without retyping hashes or epochs.
`dispatch resources` is noninteractive, does not probe accounts, and reports
configuration eligibility separately from launch-time validation.

1. Install the provider CLI yourself. Setup resolves the named tool on PATH, not
   a repository-supplied harness override. Existing advanced overrides still work
   through the execution core; guided setup does not silently run them.
2. Choose a provider. Read-only discovery uses Codex account/read and rate-limit
   data, or Claude's controlled `auth status --json`. Unknown auth fails closed;
   unknown quota is not an invented balance. No model prompt is sent.
3. If login is needed, choose Login and explicitly confirm the provider login.
   Codex uses `login --device-auth`; Claude uses `auth login`. These can require a
   browser and can change the provider's saved account. Terminal ownership is
   restored on return. No OAuth token is extracted or stored by Dispatch.
4. Choose the exact model/effort/tier you intend to include. Suggested defaults
   are examples, never proof of availability or plan coverage. The final screen
   identifies the CLI, opaque account fingerprint, model, effort, service and
   funding assertion. Type `confirm` only after checking these with your provider.
5. Claude additionally requires explicit controlled-print inclusion, disabled
   usage credits and an unmanaged account. Its evidence expires within 24 hours.
   Revalidate a numbered profile after expiry; cancelling leaves it expired.

The service writes `resources.yml` atomically, with mode 600, under a short
cooperating-writer lock. A changed file invalidates the displayed proposal.
Discovery/cancellation cannot renew a rejected epoch. A confirmed revalidation
advances its epoch; retained conflicts and actual account/funding evidence are
still checked by core at launch. Disabled profiles, explicit pool mappings,
unsupported service modes and changed Claude account scope require deliberate
advanced configuration or a separately confirmed new resource. Setup does not
change machine grants, global provider settings or live private policy.

`--plain`, `--ascii`, `--no-color` and `TERM=dumb` apply to setup too. Non-TTY setup
refuses to prompt. There is no Dispatch login for ordinary local work.

## Checks and direct work

After intent, approve local execution for that goal. It uses a separate workspace
but is **not a security sandbox**. If no checks are configured, Dispatch offers
known choices from existing `verify.sh`, `Cargo.toml`, or `Makefile`. Choosing one
explicitly approves saving and later executing it. The command may execute project
code and is not proof of task-specific correctness. No script or tool is installed.
Use `/checks` or `dispatch setup --checks` to do this independently. Advanced
`checks.verify` and planning verification-path configuration remain supported.

Work shows the actual selected resource, committed phase, elapsed time and launch
accounting. Recorded launches, uncertain spawn and configured limits are distinct;
counts are sampled from committed admission state. The elapsed indicator covers
this foreground work call, not full submission latency or only provider time.
Tool output remains in private logs; Dispatch does not invent file-reading activity
or turn a provider's tool success into authoritative verification.

Enter submits a goal; Alt+Enter adds a newline; bracketed paste never submits.
During work, input is paused and drained; Ctrl+C cancels with process cleanup.
No live steering or hidden next-goal queue exists. A goal interrupted by setup
returns as an editable draft and requires explicit submission. Plain mode prints
that preserved goal because cooked line input cannot prefill an editor.

## Decisions and review

A durable clarification is answered inside the same goal, targeting its exact
question/revision/generation. Waiting holds no model lease. Ctrl+C cancels;
answers never reset limits or authorize spending. Planned questions end at the
original deadline. Planned crash recovery is unsupported.

Review separates configured verification, human acceptance, and application:
`Unverified — no checks configured` is attention, not success. Enter/d opens native
changes; it never accepts. Choose `a` then Enter to accept and safely apply, `r`
then Enter to reject, `n` to leave pending, or `i` for evidence and full paths.
Source drift can preserve acceptance while blocking application.

While a result waits for review the source may keep changing. A Ready run's
status shows one `Coherence:` line (`CONTINUE`, `REFRESH` or `STOP`), computed
fresh from the run's baseline, its patch and the source as it is now; it is
display only and is never stored. `dispatch explain` adds a Coherence section
when the source moved or the run is a refresh.

- `dispatch check [run] [--json]` prints that verdict, the analysis level, how
  many files changed underneath the work, the reasons, and the next command
  (`dispatch accept`, `dispatch refresh` or `dispatch reject`). It changes
  nothing and exits 0 whenever the evaluation succeeds, whatever the verdict.
- `dispatch refresh [run]` starts a new run of the same task against the
  current source. The task gets a fixed addendum naming the earlier run and up to
  ten reasons it went stale. The old run is untouched. It repeats the original
  launch choices (fixed agent/model/effort, or the same harnesses) and asks again
  for `--allow-unsafe-local` / `--allow-forwarded-env` if the original run needed
  them. It is always a new, human-typed launch; nothing refreshes automatically.

`check`, `refresh` and the `Coherence:` line act only on a finished, ready result
that is not yet applied (`check` and the line also need exactly one candidate,
which any single-agent run has). The verdict rules, limits and events
are in the [coherence reference](coherence.md); the README has the short version.

### Auto-apply

A session mode applies eligible results automatically instead of waiting for your
review. It is off by default in every new session and is never persisted between
sessions. Application is not acceptance: an auto-applied result never records a
human review, so `outcome.review` stays `pending` until you look at it.

Toggle it with **Shift+Tab** on any screen (the goal prompt, while working, or the
review menu), or with `/auto-apply on|off` (bare `/auto-apply` toggles) — the path
for `--plain` mode, where raw keys are not readable. The current mode is always the
first segment of the hint row:

| Mode | Unicode | ASCII |
|---|---|---|
| Off | `⏸ review before apply · Shift+Tab` | `[review before apply] Shift+Tab` |
| On | `⏵⏵ auto-apply on · Shift+Tab to pause` | `>> AUTO-APPLY ON - Shift+Tab to pause` |

Toggling prints one line: "Auto-apply on: eligible results will be applied without
review." or "Auto-apply off: results wait for your review." In `--plain` mode, the
goal prompt also gets `(auto-apply on)` appended while the mode is on.

The review menu carries one extra action, `[aa] Accept & apply, then auto-apply the
next results`: it records your acceptance exactly like `a`, applies the result, and
only then turns the mode on for later results. It never reinterprets a result you
have not looked at.

When a goal reaches a Ready, review-pending result under this mode, Dispatch
validates it against the current source and either applies it or drops into the
ordinary review menu with a notice explaining why:

- **Applied**: the session shows "Auto-applied · review not performed" plus the
  `Coherence:` line for a moved world, and returns to the goal prompt. No accept/reject
  attest offer follows, because no human review happened.
- **Skipped or blocked**: the session enters the review menu with a notice above the
  actions — "Auto-apply skipped: `<reason>`" (for example "no checks configured",
  "verification failed", "checks cannot run on the merged tree") or "Auto-apply
  blocked: `<Coherence: REFRESH/STOP line>`" (or the bare reason when no verdict was
  computed, for example a strict-mode drift).
- **Failed** (an apply that was authorized still failed, for example a Git error):
  "Auto-apply failed: `<error>`", also into the review menu.

Ctrl+C is not honored while an auto-apply attempt is in flight: integration checks
on the merged tree are not cancellable, the same as at accept time. The attempt is
bounded by the checks' own timeout.

**Headless.** `dispatch run --auto-apply "<task>"` and `dispatch refresh --auto-apply
[run]` apply the result the same way immediately after the run returns Ready, and
print one line: "Auto-applied Candidate `<label>` to `<source>` (`<n>` file(s)
changed). Review not performed." or "Not applied automatically: `<reason>`. Review
with dispatch check `<id>` or dispatch accept `<id>`." `--json` adds an `auto_apply`
object to the result (`outcome`: `applied`/`blocked`/`skipped`/`failed`; `reason`;
`coherence`; `files_changed`); `--jsonl` streams the run's own events as usual, then
a trailing `{"type":"auto_apply", "run_id", "auto_apply": {...}}` line. Exit code
`6` means the run finished Ready but was not applied automatically (skipped or
blocked); it is used only when the run's own exit code would otherwise have been
`0` — a run that already failed for its own reason (for example failed
verification) keeps that exit code unchanged.

Eligibility, in plain words: verification must be configured and must have passed;
on a moved source, the project's own checks must pass on the merged tree; a
`REFRESH` or `STOP` verdict is never applied. Nothing is ever refreshed
automatically under this mode. There is no `dispatch.yml` key for it: the policy
lives only in the process that owns the run (the TUI session or the CLI
invocation), never on the run record and never across a restart. See
[coherence.md](coherence.md#automatic-application-auto-apply) for the full
eligibility and authorization rules.

### What `explain` shows

`dispatch explain` prints a **Coherence** section when the run's current verdict
says the source moved or is not `CONTINUE`, or when the run is a refresh of an
earlier one. It lists the decision, the analysis level (`symbols`, `files_only` or
`integration`), whether the world changed and how many files, up to ten reasons
as `code: detail`, `first invalid at`, and `refreshed from` for a refreshed run.
For a Ready, unapplied run the verdict is recomputed for display; otherwise the
last stored verdict is used. Viewing it never writes anything.

A reason such as `fact_broken` carries the old and new signature
(`old => new`). `same_symbol_edited`, `patch_conflict`, `fact_missing`,
`integration_check_failed` (with the check and its log path) and
`analysis_uncertain` (a file that does not parse, or a check that could not run)
also make the verdict `REFRESH`; `already_applied` makes it `STOP`.

The line **Agent time after the work became invalid: 4m12s of 9m40s (43%)**
measures wall-clock time only. It sums the run's attempt durations, from their
recorded start and end times, and counts the part after `first invalid at`. It is
time, not dollars, on purpose: Dispatch does not know a transaction cost for every
harness (the Claude adapter discards the nominal figure), and a dollar amount would
be invented. `first invalid at` is stamped only when an invalid verdict is stored,
by the mid-run watcher or by a blocked accept. `check`, `status` and `explain` never
store one, and a blocked accept happens after the attempts have ended, so the
figure is nonzero only for a run whose watcher saw the change while the agent worked.

### While the agent works

For runs that use included-resource allocation, a watcher runs beside each attempt.
Every `coherence.poll_secs` it checks a cheap signal (Git `HEAD`, `git status`, and
file metadata; for a plain directory, a metadata walk). Only when that signal moves
does it observe the tree, capture the work so far with a temporary Git index, and
evaluate it with the file, patch and symbol layers. Integration checks never run
mid-run, and a file that does not parse is ignored, because a person may be
mid-edit. The watcher never writes state; the run's own loop records what it
reports, as events:

- `coherence.invalidated`: the verdict became `REFRESH` or `STOP`, or is still
  invalid for a different source state. At most one message per minute.
- `coherence.checked`: the verdict is `CONTINUE` again after an invalid one.
- `coherence.stopped`: recorded right before cancellation in `stop` mode.

The payload carries the full validity object under `coherence`. In the default
`observe` mode the agent is never touched. With `mid_run: stop`, a `STOP` verdict
cancels the attempt through the normal cancellation path, and so does `REFRESH`
if `stop_on_refresh: true`. The run then ends interrupted with work result
`cancelled`, failure kind `stale_work` and exit code 1; the message reads "work
stopped: the source changed underneath it (...)". The partial patch is kept, the
result cannot be accepted, and nothing is recorded as a routing observation, goal
feedback or evaluation, so the agent is not counted as having failed. The deadline
still takes precedence if it had already passed.

### `status --json`

`dispatch status --json` (and `run --json`) adds a `coherence` object only when the
source moved or the verdict is not `CONTINUE`: `decision` (`continue`, `refresh`
or `stop`), `analysis`, `changed_files`, and `reasons` (at most five, each with
`code`, `fact_id`, `path` and `detail`). It is absent for a run whose source did
not move, so existing consumers are unaffected. Over the control protocol the
same object is described in [control-protocol.md](control-protocol.md).

Small diffs have an inline preview. Large sets open a file index: arrows/j/k select,
`/` filters, Enter opens, Esc returns to files, and q returns to the same review.
Within a file, arrows/h/l pan, arrows/j/k scroll, n/p navigate hunks, brackets switch
files, and Space loads another bounded page. `>` at the right edge marks clipping;
truncated pages say so. `v` opens a full patch pager and `e` the chosen editor.
Use a wide terminal for the complete shortcut footer; these bindings also work
at 38 columns. Generated-file labels are filename/directory hints (`generated?`),
not content classification, and never exclude data. Binary, rename, deletion and
permission changes remain in the exact candidate and index.

External tools inspect disposable exact-baseline/candidate copies. Closing a tool
is not approval; edits there are not imported. Missing/failed tools return to native
inspection. Use an explicit private `reviewer.json` preference as documented in
[review adapter details](phase4-ux-refinement.md); `$EDITOR` receives only supported
file arguments. Repository-supplied shell reviewer commands are never automatic.

After acceptance/rejection, optional `f` then `c` attests that this was ordinary
work and reflects your own review. This is private, explicit evidence; ordinary
acceptance never fabricates this label or activates a policy.

One-shot JSON/JSONL and [machine control](control-protocol.md) use the existing
foreground core independently of the renderer. Machine clients cannot issue human
acceptance or funding attestations through this setup surface. Disconnect triggers
cleanup; no daemon or detached work is introduced.
