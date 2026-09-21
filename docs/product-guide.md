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
