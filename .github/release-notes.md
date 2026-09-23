## Dispatch 0.4.1 — one path

0.4.1 is a subtractive release. It removes the machinery that decided how a native
run starts, what a run record holds and what the database commit does, and it keeps
the coherence and integration substrate unchanged. Production code is about a third
smaller (29,076 → about 18,900 lines of Rust outside test modules).

**One way to run.** Every native run uses exactly one agent: the first eligible
profile in `resources.yml` (written by `dispatch setup`), narrowed by `--agent`,
`--model` and `--effort`. With no profiles configured, `--agent claude|codex|cursor`
runs that agent without a funding contract. With neither, `dispatch run` refuses and
says to run `dispatch setup` or pass `--agent`; there is no silent fallback. Every
run goes through the same native engine, with the same launch record, mid-run
watcher, clarification questions, crash repair and run lock.

**Removed.**

- Cloud contribution sync, public benchmark priors, `recommend`, `evidence`,
  `datasets`, `data refresh`, `--route` and automatic evidence-based routing.
- Private evidence and controlled trials.
- Planning (`--plan`, `/plan`, `--max-invocations`).
- The machine-control protocol (`control`, `control-grant`). `dispatch events`
  remains the read-only journal follower.
- The automatic stronger-model retry (`--no-retry`). Clarification questions stay.
- Capacity observation, shared admission leases, pools and `--priority`.
- The multi-candidate comparison path: `--harnesses`, `--max-parallel`, blind labels,
  `compare`, `evaluate`, `inspect` and the hidden `apply <id> <label>`.

**Safety invariants kept, by replacement proven first.** Each removed subsystem that
enforced an invariant was replaced by a smaller mechanism before it was deleted, and
the deletion did not modify the replacement's tests.

- Funding identity. The Codex adapter now checks, immediately before launch, that
  authentication is ChatGPT, no paid credits are available, the service tier is
  standard, the plan matches and the account is the one setup recorded. An identity
  it cannot observe is refused. Claude keeps its adapter preflight. A refusal is
  sticky per `authorization_revision` until setup re-authorizes the profile, and a
  profile changed between selection and launch is refused (`tests/funding_safety.rs`).
- Crash repair. Every agent launch is recorded durably (intent, spawned with the
  child's process identity, cleaned or uncertain), and a run is never closed while
  an agent it launched may still be alive (`tests/launch_record.rs`). This replaced
  the admission lease as the evidence crash repair relies on.

**Also changed.**

- One review record: every human review, attached work included, is one review
  revision with its reasons and verbatim explanation. `accept` records the review
  and applies under one hold of the run lock. The `cleaner-change` reason is gone.
- Only a failed execution stops a native attempt; a completed attempt whose checks
  fail is delivered for review with its verification state. Ctrl-C records the work
  as cancelled.
- `--json` names the execution policy `execution` (it was `phase3`) and the run mode
  `native` or `attached`. `execution.max_parallel` and the `capacity:` block of
  `resources.yml` are ignored.

**Defects found and fixed on the way.**

- In 0.4.0, a profile refused once stayed refused after it was re-authorized,
  because capacity kept a stale rejected observation. Reproduced with real agents;
  gone with capacity (`tests/reauthorization.rs`).
- During this release, the attempt types lost fields that no longer mean anything,
  and completed-attempt evidence was compared byte for byte with its new
  serialization. Accepting a run recorded by 0.4.0 then modified the source and failed
  to record the application. Found by upgrading a state directory made by the real
  0.4.0 binary; completed attempts are now compared through the current type and
  their stored rows are never rewritten.

**Upgrading.** Migration 24 drops the removed features' tables and triggers and
rebuilds `runs` to allow only `native` and `attached`; a `dispatch.schema-21-*.db`
backup keeps every row. Human judgments stay in the database: reviews, and the blind
evaluations and routed-run feedback recorded before 0.4.1. Earlier runs keep loading,
and fields 0.4.1 no longer uses stay in their metadata as they were. Finish, accept
or reject work started by 0.4.0 before upgrading: a run still queued or executing is
not closed by 0.4.1, because Dispatch never closes a run whose attempt may still have
a live agent. A Codex profile needs the account evidence `dispatch setup codex`
records, so run setup again for an existing one. See `docs/release-install.md`.

**Evidence.** The funding and launch-record proof suites run the real binary against
fixture provider executables. Before capacity and admission were deleted, real Claude
Code runs (`claude-sonnet-5`) completed through the new launch record, a killed
supervisor left a correct record, and refusals worked against the real binaries. A
state directory made by the real 0.4.0 binary (a two-candidate comparison with a
blind evaluation, and a single-candidate run) upgraded to schema 24, and `accept`
applied and recorded the review. On the finished branch, a real Claude Code run
(`claude-sonnet-5`) on a Python project whose state was at schema 23 migrated it,
passed verification in 32 seconds, and, after an unrelated edit moved the source,
was accepted with a CONTINUE verdict after the integration check on the merged tree.
Coherence claims are unchanged; see
`docs/coherence-validation.md` for what is claimed and what would falsify it.
