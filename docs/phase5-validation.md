# Phase 5 implementation and validation

Phase 5 adds independent foreground machine control over the existing Rust core.
The protocol and runnable examples are in [control-protocol.md](control-protocol.md).
This report describes the implementation, not a new implementation plan.

## Checkout preservation

Starting HEAD: `81368cf941be1181a025d157766cf22377933fb8`.

The starting tree was already dirty: Cargo files, README, config/database/executor/
harness/main/models/orchestrator/router/source/state/sync modules and older tests
had changes. Phase 0–4 modules, docs, presenters and regression tests were untracked.
Those files were preserved. No commit, stash, tag, publish, or history rewrite was
performed. The implementation changes existing `README.md`, `src/db.rs`,
`src/lib.rs`, `src/main.rs`, `src/models.rs`, `src/orchestrator.rs`, and
`src/orchestrator/phase3.rs`; the other starting edits remain untouched.

The reproducible local checkpoint is
`/private/tmp/dispatch-phase5-20260917-142216/`: HEAD, initial short status, a binary
tracked patch, a working-tree archive, and SHA-256 manifest. A supplemental archive
preserves the 433 unchanged ignored research inputs. Compiler output and Finder
metadata are excluded. The archives include untracked Phase 0–4 project files.
Restore into a separate empty directory for comparison; do not overwrite the live
working tree. Temporary checkpoints/logs should be retained by the owner if needed
beyond the operating system's temporary-file lifetime.

Baseline `cargo test --locked` inside the sandbox reached 154 unit passes, seven
local-loopback permission failures and one ignored test. Repeating with local
loopback/process-inspection permissions passed **318 tests, zero failures, one
ignored**. No real provider/model calls were made.

## Shared entrypoints and authority

| Operation | Shared entrypoint / core authority |
|---|---|
| initialize | `Session::from_fd`, immutable `Scope`, versioned transport |
| submit | `commands::execute` → `run_dispatch` → existing admission/supervisor |
| status | `commands::inspection::snapshot`, one committed SQLite read snapshot |
| result | `commands::inspection::result` → `run_result`, immutable attempt evidence |
| capacity | `Scope::run`, authorized cached observation with existing knowledge/expiry fields |
| events / subscribe / await | `commands::inspection::observe` → `Database::event_page` |
| answer | `answer_question` → shared `resolve` → Phase 3 `drive` and fresh admission |
| cancel | `commands::cancel` → existing cancellation token / `cancel_question` |
| recover | `phase3::recover_checkpoint` → exclusive lock, reconciliation, existing `drive` |
| artifact | `commands::inspection::artifact`, authorized completed-attempt lineage |

The local owner issues a fixed 24-hour grant, stored as private scope plus a secret
hash. Its key is passed through a private inherited file handle and closed before
harness launch. The transport cannot provision or broaden grants. Source/state
roots, resource/funding profiles, project policy, invocation/deadline ceilings,
and factual delegation are bounded. Current Phase 2 evidence remains decisive at
every invocation. A client can neither accept/apply a result nor approve unsafe
execution or funding. Machine answers preserve machine provenance. Unknown
question categories remain undelegated.

The core guards question ownership, including separate CLI attempts to take a
live controller's checkpoint. Existing human TUI review, acceptance, explicit
application, source-drift checks, scheduler identity and one-shot flags remain.

## Persistence and contract changes

Migration **18**, the next unused version in this checkout, adds only:

- `control_grants`: private immutable scope and credential digest.
- `control_runs`: authenticated principal, foreground session and cancellation fence.
- `command_receipts`: principal/request ID, canonical digest, owned run, committed reply.

Phase 3 in this checkout had durable clarification records, but **no request
receipt table**, despite the older plan's proposal. Existing questions, attempts,
authorizations, evaluations, sync/outbox records and event identity are reused.
Optional `CheckpointReport.category` and `Clarification.actor` fields preserve
historical deserialization. All authoritative file-backed SQLite writer connections
now use WAL and `synchronous=FULL`; a regression asserts the durability pragma.
Bulk logs remain files, and no execution/probe/check/wait spans a write transaction.

Initial run creation, acceptance event, ownership and submit receipt share one
transaction. Answer/cancel/recovery effects, events and receipts also commit
atomically. Reply loss never silently repeats the effect. There is no promise of
exactly-once child execution or external event delivery. Event cursors remain
per-run sequences; existing one-shot envelopes are unchanged. Admission waiting
reason changes now get committed events, closing an observed semantic-wait gap.

Limits and foreground lifetime are explicit: 64-KiB requests, 16 pending requests,
eight observers, 32 queued output messages, 1-MiB output frames, 16-KiB previews,
60-second waits, and a three-second stalled-output deadline. Overflow closes the
connection and requests cleanup; journal replay preserves authoritative transitions.
Owner EOF/broken output/hangup cancels; read-only follower EOF does not cancel the
owner. Uncertain cleanup retains admission. Recovery never replays an interrupted
invocation and keeps the original maximum of two invocations and absolute deadline.

## Deterministic scenario matrix

All Phase 5 scenarios run the actual binary through OS pipes. The fixture harness
is a local Python executable named `codex`, with account probing disabled. Barriers
use FIFOs, committed journal state, SQLite faults and process termination. They do
not purchase capacity, alter accounts, install tools, or contact provider models.
The human coexistence case uses the existing real PTY/TUI fixture.

| Requirement | Evidence |
|---|---|
| A — standalone verified result, review pending | `standalone_verified_delivery_preserves_human_review`; second goal leaves the first review pending; release smoke |
| B — question, delegated answer, one continuation | `duplex_clarification_and_idempotent_answer`; two real fixture invocations and immutable attempt evidence |
| C — bounded recovery | `explicit_checkpoint_recovery_keeps_original_limits`, `recovery_rejects_live_owner_and_expired_budget`, `recovery_checks_original_source_lineage`; original deadline retained |
| D — duplicate/conflicting submit | `durable_submit_retry_and_payload_conflict`, including reconnection with the same principal |
| E — lost answer reply | `lost_answer_reply_recovers_only_explicitly`; a FIFO stops configuration loading after the committed answer, then the owner is killed; receipt replay launches nothing |
| F — scoped resources, runs, questions and artifacts | `scoped_authority_and_stale_mutations`, `resolved_artifact_paths_and_resource_scope_are_enforced`, `receipt_failure_rolls_back_submit_effect_and_event`; arbitrary recovery paths denied before lock creation |
| G — forged authority cannot elevate | `bounded_versioned_frames_and_no_unintended_execution`, `scoped_authority_and_stale_mutations`; human/actor/grant/source fields rejected; no human feedback created |
| H — stale answers and cancellation fence | `scoped_authority_and_stale_mutations`, `cancelled_answer_receipt_never_reopens_work`; no third invocation |
| I — responsive duplex wait | `pending_wait_does_not_block_cancellation`, `duplex_clarification_and_idempotent_answer`, `observer_bound_leaves_cancellation_responsive` |
| J — semantic waits | `semantic_waits_and_ordered_event_replay` plus clarification's transient-attention replay and a wait outstanding before answer; snapshot/journal polling has no registration gap |
| K — framing and negotiation | `bounded_versioned_frames_and_no_unintended_execution`: fragments, multiple frames, malformed JSON, invalid UTF-8, missing/forged fields, unsupported version/operation, oversized input and partial EOF |
| L — backpressure | `slow_output_cancels_without_blocking_child_drainage`: fixture emits 30 MiB while owner output is unread, then bounded shutdown and complete journal replay |
| M — disconnects | `owner_eof_cancels_execution`, `owner_eof_during_admission_never_spawns`, `owner_eof_closes_question`, `broken_output_pipe_cancels_owner`, `broken_pipe_cancels_queued_admission`, `broken_pipe_cancels_clarification`, `hangup_cancels_waiting_question`, `read_only_follower_disconnect_does_not_cancel_owner` |
| N — crash and explicit ownership | `lost_answer_reply_recovers_only_explicitly`, `explicit_checkpoint_recovery_keeps_original_limits`, source/deadline/live-owner rejection cases; existing Phase 2 orphan/fence and Phase 3 interruption regressions |
| O — shared admission and fresh funding | `human_tui_and_control_share_allowance_without_review_transfer`, `control_and_existing_foreground_share_admission`, `overlapping_changed_mappings_still_exclude_control`, `changed_funding_blocks_continuation`; existing Phase 2 overlap/conflicting-observation suite |
| P — existing product | Full Phase 0–4, CLI/product UX, task-file/JSON/JSONL, PTY, diff, review/apply and source-drift suites; no existing flag renamed |
| Q — migration and historical data | `migration_preserves_existing_questions_attempts_and_authorizations`, existing DB migration/FK/legacy sync tests, cross-session receipts and historical result reads |
| R — optional host | Not shipped; deferred and unverified independently of core |

The initial sandbox denied loopback sockets and reliable live-process inspection.
In the shared-admission fixture this correctly produced conservative reconciliation
instead of pretending the owner was absent. Final process/PTY tests run with the
same required local permissions as the successful baseline. No blocked sandbox
run is counted as a pass.

## Final validation

| Command | Actual result |
|---|---|
| `cargo fmt --check` | Exit 0 |
| `cargo test --locked` | Exit 0: **348 passed, 0 failed, 1 ignored** |
| `cargo test --locked --test phase5_control` | Exit 0: **30 passed**, including the pipelined pending-ID conflict regression |
| `cargo clippy --locked --all-targets -- -D warnings` | Exit 0, no warnings |
| `git diff --check` | Exit 0 |
| `cargo build --release --locked` | Exit 0; optimized build completed in 19.49 seconds on the final source |
| `python3 tests/fixtures/phase5_control.py target/release/dispatch smoke` | Exit 0: verified delivery, pending human review, retained first result after second goal, scoped diff |
| `python3 tests/fixtures/phase5_control.py target/release/dispatch clarify` | Exit 0: real duplex question/answer/continuation and pending wait |

The full suite includes the focused Phase 2 admission, Phase 3 recovery,
Phase 4 CLI/PTY/review, migration, source-safety and legacy-sync regressions.
The ignored test is the pre-existing explicitly launched terminal panic fixture;
PTY regressions exercise its intended isolated path. No additional test was skipped.
SQLite FULL durability is asserted; abrupt process/commit faults are tested, but
hardware power-loss testing was not performed.

Local evidence logs:
`/private/tmp/dispatch-phase5-baseline-unrestricted.log`,
`/private/tmp/dispatch-phase5-final-tests.log`,
`/private/tmp/dispatch-phase5-protocol-tests.log`,
`/private/tmp/dispatch-phase5-clippy.log`,
`/private/tmp/dispatch-phase5-release.log`,
`/private/tmp/dispatch-phase5-release-smoke.log`, and
`/private/tmp/dispatch-phase5-release-clarify.log`.


## Size, deviations, and limits

Production Rust physical lines: **20,967 → 22,749, delta +1,782**.
The same method counts `src/**/*.rs` before/after the preserved checkout, removes
complete `#[cfg(test)]` items/modules, includes comments/blank lines, and excludes
integration tests and the standalone Python example. The reproducible measurement
script and JSON result are in the checkpoint directory as
`measure-production-loc.py` and `production-loc-final.json`. The added surface is
scoped authority/receipts, framing/multiplexing, bounded observation/artifact reads,
and narrow integration into the existing core; no second executor was added.

The brief's generic-first direction is implemented. No daemon, socket/HTTP/MCP
server, new provider/backend, planner, task DAG, parallel subtask execution,
learning policy, host framework, UI redesign, or Phase 6 work was added. The
standalone client is 134 lines of Python standard-library example code, separate
from production Rust and tests.

The older plan's provisional migration numbers were replaced with actual version
18. Its machine review/apply examples were narrowed to the brief's explicit human
review boundary. The optional host observer is deferred; no Herdr behavior is
advertised or depended upon. Unix handle support and fixed 24-hour grant lifetime
are explicit first-release limits. Only completed eligible checkpoints can recover;
lost or expired authority and uncertain active invocations fail closed. Fixture
results do not prove subscription savings, multi-day product validation, or release
publication readiness. Nothing was committed or published.
