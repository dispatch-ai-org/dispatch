# Phase 4 presentation and review refinement

This patch refines the existing local Phase 0–4 loop. It does not change resource
selection, invocation limits, admission, verification, recovery, clarification,
or acceptance authority. It introduces no execution schema or Phase 5 transport.
No real-account model invocation, global installation, editor/Git configuration change, tag,
or publication was used to validate this refinement.

## What the screenshot actually established

The founder completed the goal and applied its result, but the interface made
activity unclear and the decision area competed with repeated status and patch
output. Inspection of the starting presenter established that **diff and details
were already explicit actions**: `d`/`diff` opened the patch; `i`/`details` opened
diagnostics. The visible patch is consistent with choosing the diff action. The
screenshot alone does not establish that Details was selected or that diagnostic
identifiers appeared by default.

The change therefore separates three depths: a compact decision summary, an
explicit change inspector, and explicit diagnostics. Returning from either
inspection view restores the same task and decision controls. Ordinary terminal
scrollback is preserved.

## Layout and controls

The session begins with a two-row node/route signature and the project name. The
artwork is at most 40 columns; narrow terminals use the wordmark and context.
There is an intentional ASCII counterpart. It appears once per session, with no
animation or delayed input. Subsequent goals use a compact goal heading.

The live area has a consistent two-column left margin and bounded text width.
Its hierarchy is goal, meaningful state, verification/change summary, primary
action, then secondary actions. Working state retains the compact semantic route
and elapsed indicator. The latter indicates foreground elapsed time, not model
progress or an estimate of completion.

At review:

- **Enter / Review changes** opens inspection. It never accepts work.
- **Open in editor** opens the selected supported reviewer.
- **Accept & apply**, **Reject**, and **Leave pending** remain deliberate actions.
- **Details** contains run/candidate IDs, revisions, allocation detail and full
  artifact paths. Essential failure, drift and verification warnings remain in
  the primary view.

Action names are visible. Letter actions are selected with their letter and
Enter; pasted action text is not executable control input. Composer editing hints
appear only while entering a goal or answering a question.

Successful model exit is not successful verification. With no configured checks,
the interface says **Unverified — no checks configured**, with no completed
verification-stage marker. Green verification text is reserved for configured
checks that passed. Recovery and durable questions still reflect the core's
committed state.

## Change inspection at different sizes

A tiny single-file patch gets a bounded inline preview of changed lines. Added
and deleted lines retain `+`/`-` markers; changed grapheme spans receive additional
emphasis. Hash headers and artifact IDs do not surround a one-line change.

Larger work opens a changed-file index with relative path, change kind and line
counts, plus explicit binary/deleted/renamed indicators. Every file remains in
the index, including generated and lock files. The index preserves the selection
and path filter when returning from a file. It does not launch a window per file.

| Inspector action | Control |
|---|---|
| Select a file | Up/Down or `j`/`k` |
| Open selected file | Enter |
| Filter paths | `/`, type text, Enter |
| Scroll file | Up/Down, Page Up/Page Down |
| Pan a long line | Left/Right |
| Next/previous hunk | `n` / `p` |
| Next/previous file | `]` / `[` |
| Next bounded patch page | Space when indicated |
| Full patch pager | `v` |
| Selected file in reviewer | `e` |
| Return to file list | Esc |
| Return to decision summary | `q` |

The ordinary session remains inline. Opening inspection or diagnostics explicitly
uses a temporary full-screen view; leaving it restores the inline decision area.
This is an inspection tool, not a permanent dashboard.

Indexing streams the retained patch and stores file/hunk offsets. It does not
load the entire patch into render memory. Interactive pages are bounded at
32 KiB; plain pages at 4 KiB. A bounded page is labeled, and its next page/full
patch pager remain accessible. Metadata safety limits are explicit errors, not
silent omission of the rest of the patch. The authoritative patch remains intact.

Rename presentation recognizes identical regular-file moves when the retained
patch describes them as delete/add. It does not invent a general similarity or
semantic diff engine. Plain mode provides a numbered file list and bounded pages
without escapes. Unix plain input uses cooked polling and line reads with no
background reader. External reviewer launch remains an interactive-view feature;
plain review uses the built-in fallback.

## Reviewer configuration

The optional preference file is `reviewer.yml` inside the selected Dispatch state
directory (normally `~/.dispatch/reviewer.yml`). It is user-owned configuration,
not repository configuration. The file must be a regular file owned by the user,
not group/world writable, and at most 16 KiB. No preference file is required for
the built-in inspector.

These configuration shapes are covered by adapter tests. They assume the named
program is already available; this refinement installs no tools.

```yaml
# Native pager: entire retained patch as a safe display copy.
kind: pager
```

```yaml
# Optional delta: full patch, navigation, and a waiting less session.
kind: delta
```

```yaml
# VS Code: the selected file pair, with supported wait behavior.
kind: code
# Optional absolute path if code is not available on PATH:
# executable: /Applications/Visual Studio Code.app/Contents/Resources/app/bin/code
```

```yaml
# Neovim: the selected file pair in read-only diff mode.
kind: nvim
```

```yaml
# Unknown editor: labeled patch-file fallback, not guessed diff flags.
kind: editor
executable: /absolute/path/to/your-editor
args: ["--wait"]
wait: process
```

Use `wait: process` for an unknown editor only when that command actually waits
until review is finished. Its default is `explicit_return`: close the external
review document, then explicitly return to Dispatch. The disposable material
remains alive through that return action. A launcher exiting is not interpreted
as completing human review.

Selection order is an explicit Dispatch preference, then recognized **global**
Git `diff.tool` names, then global `core.editor`, `VISUAL`, `EDITOR`, and the native
pager. Only the known `code`/`vscode` and `nvim`/`nvimdiff` difftool names are reused.
Repository `difftool.*.cmd` and config includes are not executed. Editor hints are
split into literal executable/arguments, not evaluated by a shell. Configured shell
launchers such as `sh`, `env` and `xargs` are refused; there is no custom
shell-command facility.

VS Code uses `--wait --diff BEFORE AFTER`; Neovim uses a read-only, no-swap
file-pair diff. They are **file-pair adapters**, not directory comparisons. The
changed-file index remains the whole-change-set interface. No directory reviewer
adapter is claimed in this patch. Optional delta styles the existing patch; it
never supplies apply input. Missing or failed tools return to usable built-in
inspection and never trigger model recovery.

The adapter contracts were checked against the official documentation for
[VS Code CLI](https://code.visualstudio.com/docs/configure/command-line),
[Neovim startup](https://neovim.io/doc/user/starting/),
[delta options](https://dandavison.github.io/delta/full--help-output.html), and
[less](https://greenwoodsoftware.com/less/).

## Review identity, copies and terminal ownership

Review is bound to the exact run and final delivery candidate. Before/after file
pairs compare the frozen original baseline with the retained delivery patch
reconstructed in private temporary material. They do not compare arbitrary HEAD
against the live working tree. No commits, history changes, Git metadata, hooks
or authentication configuration are created in the user's repository for review.

External programs receive disposable read-only copies. Known authentication and
configuration paths prevent external export and fall back to built-in inspection;
this is a filename/component safeguard, not a claim to detect every possible
secret. Symlinks are represented as link-target text rather than traversed.
Fingerprints detect changed immutable evidence and edits to review copies.
Read-only editor flags are not treated as a security boundary.

**External edits are not part of the candidate and are never imported.** Reopening
a file discards edited review copies and reconstructs them from retained evidence.
Patch viewers receive a complete display copy with control characters escaped;
the original authoritative patch is neither truncated nor changed.

Dispatch stops its reader and renderer before a terminal reviewer takes over.
It restores terminal modes, waits for the reviewer, restores the captured terminal
attributes, clears queued input at the ownership boundary, and resumes the same
review state. No model lease is held during review. GUI tools use supported wait
behavior where available; unknown launchers require explicit return. Terminal
reviewers get an owned foreground process group, including descendant cleanup on
cancellation or failed return. A successful GUI document wait does not close the
editor application or its other windows. Cancelling an unconfirmed external
return leaves the candidate pending; uncertain cleanup cannot reopen an actionable
accept/apply area.

After external review, immutable evidence is checked, the exact candidate is
retained, and the authoritative revision is refreshed. Existing typed review and
apply guards still reject stale/concurrently reviewed delivery or source drift.
Closing an editor, pager or inspector never accepts or applies a result.

## Palette and fallbacks

Semantic styles are centralized in `src/presenter/theme.rs`. The terminal's native
foreground/background are preserved. Dark RGB accents use the recorded site audit
values: cyan `#67E8F9`, focus `#A5F3FC`, secondary `#94A3B8`, success `#5EE9B5`, and
attention `#FFD236`. Deletions/errors use a distinct readable red. Labels remain
meaningful without color.

`DISPATCH_COLOR=auto|truecolor|256|16|none` controls capability choice. Auto uses
conservative `COLORTERM`/`TERM` hints; absent evidence falls back to 16 colors and
`TERM=dumb` uses no color. Explicit 256- and 16-color palettes exist. `NO_COLOR`
and `--no-color` override color requests. ANSI Cyan is a fallback, not a claim to
reproduce the site RGB value.

`DISPATCH_THEME=light` selects darker accent, success, warning and secondary
colors. Default is dark/native foreground; no background escape queries delay
startup and no perfect theme detection is claimed. ANSI palettes are user
customizable, so these fallbacks still need real visual evaluation.

```sh
DISPATCH_COLOR=truecolor DISPATCH_THEME=dark ./target/debug/dispatch
DISPATCH_COLOR=256 DISPATCH_THEME=light ./target/debug/dispatch
NO_COLOR=1 ./target/debug/dispatch --ascii
./target/debug/dispatch --plain
```

## Recorded previews

These are renderings of actual fixture PTY byte captures, **not native macOS
terminal screenshots**. They use Menlo, an explicitly chosen dark preview
background and only crop wholly blank rows. The corresponding `.ansi`, marker
metadata and rendered text are retained beside each PNG. No model allowance was
used. The renderer fails on unknown terminal instructions instead of silently
inventing screen content.

| Milestone | Capture |
|---|---|
| Startup | [PNG](captures/phase4-refinement/startup.png) · [text](captures/phase4-refinement/startup.txt) |
| Working | [PNG](captures/phase4-refinement/working.png) · [text](captures/phase4-refinement/working.txt) |
| Tiny review | [PNG](captures/phase4-refinement/tiny-review.png) · [text](captures/phase4-refinement/tiny-review.txt) |
| 125-entry change index | [PNG](captures/phase4-refinement/large-review.png) · [text](captures/phase4-refinement/large-review.txt) |

Graphical contrast and polish in the founder's actual terminal, light theme,
font and window dimensions still need the founder's eyes. PTYs and buffer
snapshots establish behavior and recorded output, not that visual acceptance.
The available computer-use surface did not permit Terminal.app access, so no
native Terminal.app screenshot is claimed.

## Demonstration path

Build the local checkout with `cargo build --locked`, then run its binary from an
eligible project. Enter a small goal, observe the live state, and reach review.
Press Enter to inspect changes; use `q` to restore the decision area. Open Details
with `i` then Enter and return with `q`. Choose `a` then Enter only when deliberately
accepting/applying; choose `r` or `n` to reject or leave pending. On a multi-file
result, filter paths and open one file before choosing its editor.

For an allowance-free implementation check, use the deterministic review fixture
suite rather than a real model task. Capture regeneration uses the PTY driver in
`tests/fixtures/review_refinement.py` and the adjacent `render-capture.py` helper.
No real task or global installation was performed as part of this refinement.

## Validation and change measurement

The starting working tree was copied to
`/private/tmp/dispatch-phase4-refinement-start-a8958150`, including
`START-MANIFEST.json`, `START-STATUS.txt` and `START-PRODUCTION-LOC.txt`.
This isolates the refinement from the pre-existing uncommitted Phase 0–4 work.
The recorded starting production count is **18,539 lines**. This is the sum of
physical lines in `src/**/*.rs`, including blank lines, comments and embedded SQL,
after removing each file’s trailing `#[cfg(test)] mod …` section. Test fixtures,
documentation and capture bytes are excluded. The identical method is used at
completion; this is a snapshot-to-working-tree measurement, not the full dirty
Git diff. The final production count is **21,014 lines**, a net **+2,475**.

| Production area | Net lines |
|---|---:|
| Retained-patch indexing, private material and reviewer adapters | +1,085 |
| Change inspector and review controls | +767 |
| Existing presenter changes | +292 |
| Semantic palette and signature | +157 |
| Reviewer terminal handoff | +149 |
| Existing core guard visibility/reuse and module export | +25 |

The execution-core changes reuse process cleanup and exact-candidate review
guards. No execution policy or database schema changed. The two newly direct
Unicode dependencies already existed at the same versions in the lockfile; no
package version was upgraded. The existing uncommitted work was preserved.

Reviewer contract tests cover safe arguments, bounded indexing, exact retained
patch reconstruction, copy/evidence mutation, unsupported tools and preferences.
Native less 668 and installed delta 0.19.2 were exercised in actual PTYs.
The default delta adapter was exercised; its optional side-by-side configuration
was not separately run. Code/Neovim
adapter contracts and GUI waiting behavior use deterministic fake executables;
that is not a claim that native VS Code/Neovim sessions were exercised.

| Area | Evidence and limits |
|---|---|
| Startup, working and tiny review | Recorded real macOS PTY bytes; derived PNGs inspected |
| Several files, 125-entry index, large synthetic patch | Deterministic local fixture flows and bounded index/page tests |
| Rename, deletion, binary, unusual/long paths and lines | Index/reconstruction fixtures; no whole-patch render allocation |
| Palette, Unicode/ASCII, no-color, narrow layout | Deterministic style/layout assertions; explicit truecolor PTY captures |
| Unverified, passing/failed checks, recovery, question | New review fixtures plus retained Phase 0–4 semantic tests |
| Repeated diff/details and return, typeahead | PTY actions and exact pending-review assertions |
| Native terminal reviewer | Installed less 668 and delta 0.19.2 actually opened/closed in PTYs |
| Editor exit/crash/cancel, GUI waiting | Deterministic fake executables; native Code/Neovim absent |
| Panic in inline/inspection view | PTY restoration and primary-screen sentinel; no erase after alternate-screen return |
| Candidate identity, concurrent review, drift, copy changes | Typed command/integrity fixtures; immutable bytes checked |
| Plain/non-TTY and machine output | Existing Phase 0–4 compatibility tests plus review fallback |
| Native graphical contrast, light/dark themes | Not visually exercised in a native terminal; founder check remains |
| Linux, tmux, SSH | Not newly exercised by this refinement |

Final validation on the completed source:

| Check | Result |
|---|---|
| `cargo fmt --check` | Passed |
| `RUST_TEST_THREADS=4 cargo test --locked` | 318 passed, 0 failed, 1 ignored subprocess helper |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `git diff --check` | Passed |

The ignored helper is deliberately invoked by its passing PTY parent test, now
covering both inline and inspection panics. The new review integration suite
contains three tests exercising 18 fixture scenarios. Four test threads limit
concurrent test functions; the simultaneous-session tests still launch concurrent
Dispatch processes.

The first sandboxed full run could not bind seven existing loopback fixture
servers; the authorized rerun allowed those local servers. A default-parallel
run then hit the existing five-second deadline fixture before its first process
started. That case passed alone, and the complete final suite passed with four
test threads. No execution deadline, invocation limit or test assertion was
weakened to obtain the pass.

Complete local logs are retained at
`/private/tmp/dispatch-refinement-full-tests.log` and
`/private/tmp/dispatch-refinement-clippy.log`.

## Deliberate scope additions and remaining limits

The original Phase 4 bounded preview is extended by a navigable file/hunk index,
temporary full-screen inspection, native pager and optional file-pair/editor
adapters, trusted reviewer preferences, private review copies and integrity checks.
Those are review surfaces over existing authority, not new execution policy.
No directory-comparison adapter, full editor, semantic diff engine, custom shell
plugin platform or candidate-edit/import workflow was added.

The ordinary session remains inline; plain/machine paths remain available.
Machine JSON/JSONL never renders artwork, prompts or launches reviewers. This
work does not demonstrate allowance savings or establish broad release readiness.
The next meaningful product check is the founder using the polished interface on
an ordinary eligible task.
