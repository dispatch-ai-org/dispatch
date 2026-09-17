# Foreground control protocol, version 1

`dispatch control --stdio` is a foreground, duplex JSONL client interface. It uses
Dispatch's existing allocator, Phase 2 admission, Phase 3 supervisor and bounded
continuation, verification, journal, and human review workflow. It does not run a
service. Herdr is neither required nor implemented by this change.

## Human authorization and launch

A human local owner first reviews the project's `dispatch.yml` and the included,
no-overage-verified profiles in the state directory's `resources.yml`, then runs:

```sh
dispatch --state-dir /private/path/state control-grant /path/to/project \
  --timeout 600 --max-invocations 2 --allow-unsafe-local --delegate-factual
```

This is an explicit **human provisioning command**, not a protocol operation and
not something a client should invoke to approve its own requests. It prints the
path of a newly created, mode-0600 key in `state/control-grants`. The state root
must belong to the local owner and have mode 0700. The grant captures canonical
source and state-root paths, the semantic project configuration digest, current
eligible resource profiles, a per-goal deadline ceiling (the smaller of the
requested and configured timeout), one or two total invocations, the explicit
unsafe-local acknowledgement, and factual-question delegation. Environment
forwarding is not supported for these grants. Allocation and shared admission
must be enabled.

No approval is requested on `/dev/tty`. Omitting the unsafe-local acknowledgement
for a local grant fails. That acknowledgement does **not** turn local execution
into a sandbox. Protocol scopes do not contain hostile programs sharing the
owner's OS account; such programs may access the owner's other files or tools.
The provisioning command is part of the existing trusted local-owner boundary.
A JSON actor, PID, username, host variable, or desired grant is never accepted as
a protocol credential or as evidence of human acceptance.

The trusted launcher opens the key read-only and passes its descriptor. The key
contents must never go in arguments, task text, environment variables, or logs:

```sh
# Replace the path with the one printed by control-grant.
dispatch --state-dir /private/path/state control --stdio --grant-fd 3 \
  3</private/path/state/control-grants/GRANT-ID.key
```

The implementation currently supports Unix inherited regular-file handles. It
checks ownership, private permissions, regular-file type, and the 64-byte size;
looks up the secret's digest in the private database; validates scope; then closes
the handle before any harness can inherit it. The handshake exposes the public
principal ID, never the secret. One writable connection holds an exclusive grant
lock. A second writer fails; `--read-only` permits a separate scoped observer.
Read-only sessions cannot mutate or claim recovery ownership. Their disconnect
only ends their own requests.

Each grant expires after 24 hours. The same still-valid key authenticates the same
principal across process restarts, allowing receipt replay and **explicit** eligible
checkpoint recovery. A new key is a new principal and cannot inspect or recover
another principal's runs. Expired grants fail explicitly, including receipt
lookups; their old IDs never become fresh spending requests. Receipts are retained
without garbage collection. The human CLI remains available for historical
inspection and review. There is no account, grant-renewal, or live-transfer service.

## Framing, bounds, and session lifetime

Each request is one UTF-8 JSON object terminated by LF. The 65,536-byte bound
includes LF and is checked during reading with an 8-KiB read buffer, before growing
an unbounded allocation. Fragmented writes and several frames per write work.
An unterminated final frame never executes. Unknown request fields are rejected,
including `actor`, `human`, `grant`, arbitrary source paths, and execution flags.
Unknown response fields should be tolerated by clients.

Stdout contains protocol objects only. Stderr contains diagnostics. One writer
serializes all output; harness pipes are independently drained into the existing
bounded capture/artifact path. Limits advertised during initialization are:

| Bound | Value |
|---|---:|
| Request frame | 65,536 bytes, including LF |
| Response/event frame | 1 MiB, before LF |
| Buffered inbound frames | 16 |
| Pending correlated requests | 16 |
| Concurrent observers, including waits | 8 |
| Buffered outbound messages | 32 |
| One output write/flush stall | 3 seconds |
| Wait/subscription duration | 0–60,000 ms |
| Artifact preview | 16,384 bytes |
| Request ID | 1–128 UTF-8 bytes |
| Active goal per writable session | 1 |

A full output queue, an oversized output projection, or a write stalled for three
seconds closes the owning connection and requests supervised cancellation.
Transitions remain in the journal; the client resumes reads using its last
**received** cursor, never an assumed delivered cursor. Closing is the explicit
overflow policy: events are not silently dropped while pretending the stream is
healthy. Slow subscribers cannot hold up harness drainage or cancellation.
There is no exactly-once delivery guarantee.

Owner EOF (including EOF during admission or a question), SIGHUP, SIGTERM,
SIGINT, or a broken stdout pipe requests controlled cleanup. Cleanup uncertainty
retains admission and reports reconciliation; it does not release a potentially
live child. A session exit of 0 means normal foreground shutdown, not that all
of its goals succeeded. Transport/bootstrap errors exit 1; argument errors retain
Clap's exit 2. Per-goal outcomes and result exit codes are independent. Existing
one-shot exit codes are unchanged. `run --jsonl` remains a one-shot stream and
`run --task-file -` still reads task text to EOF.

## Requests and replies

Every request carries `protocol_version: 1` and a correlation `request_id`.
Initialize before any other operation. Responses can arrive out of order; an
outstanding `await` or subscription never occupies the command reader.

```json
{"protocol_version":1,"request_id":"hello","op":"initialize"}
```

The successful response has `type: "response"`, `ok: true`, and a `result`
containing supported versions/operations, limits, `session_id`, public `principal`,
immutable `scope`, `read_only`, and `ownership_mode: "foreground"`. Initialization
describes existing authority; it cannot expand it. Complete operation examples
below use symbolic IDs; substitute the exact IDs/revisions returned by Dispatch.

### Submit, status, result, and capacity

```json
{"protocol_version":1,"request_id":"submit-001","op":"submit","task":"Add tests in src/lib.rs"}
{"type":"response","protocol_version":1,"request_id":"submit-001","ok":true,"result":{"run_id":"RUN-ID","state_revision":1,"cursor":1,"accepted":true}}
{"protocol_version":1,"request_id":"status-001","op":"status","run_id":"RUN-ID"}
{"protocol_version":1,"request_id":"result-001","op":"result","run_id":"RUN-ID"}
{"protocol_version":1,"request_id":"capacity-001","op":"capacity","run_id":"RUN-ID"}
```

Submit also accepts optional `model` and `effort` constraints, which can only
narrow eligible grant/current-policy choices. It returns after baseline capture
and the atomic run/acceptance/ownership/receipt commit, before completion. This
is not a promise that a harness started. It calls `commands::execute` →
`orchestrator::run_dispatch`. The core validates the source, policy, included
resources, remaining limits, fresh funding evidence, and attempt-bound admission.
No protocol operation spawns a harness directly.

A distinct submit while preparing, executing, awaiting admission, or awaiting a
question returns `busy`. An identical committed retry returns its original receipt
instead. After execution finishes, another submit is allowed; previous results
and pending human reviews are retained.

Status calls `commands::inspection::snapshot` and returns the committed outcome,
question/policy state, admission, state revision, and journal cursor from one
SQLite read snapshot. Capacity returns only the identified owned run's cached
observation, including existing sampled/expiry/knowledge fields; `null` is unknown,
not unlimited. Neither operation probes or spends capacity.

Result calls `commands::inspection::result` → `orchestrator::run_result`. While
execution is unfinished it returns `not_ready`. A terminal result includes the
immutable final attempt/candidate identity as `result_id`/`delivery_revision`
(or null when no delivery exists), the current state revision, every attempt,
requested/resolved/observed model identities, checks, verification, review,
application state, and scoped artifact references. Check outcomes are evidence,
not universal quality judgments. In particular:

- Accepted command ≠ started invocation.
- Exited harness ≠ verified delivery.
- Verification passed ≠ human accepted.
- Human accepted ≠ application succeeded.
- Ready for review ≠ applied.

An execution-finished outcome with reconciliation still requires cleanup attention;
consult admission and `waiting_on`, not just one boolean.

### Events and semantic waits

```json
{"protocol_version":1,"request_id":"history-001","op":"events","run_id":"RUN-ID","after":0}
{"protocol_version":1,"request_id":"stream-001","op":"subscribe","run_id":"RUN-ID","after":4,"timeout_ms":10000}
{"type":"event","protocol_version":1,"subscription_id":"stream-001","event_id":"RUN-ID:5","event":{"protocol_version":1,"run_id":"RUN-ID","sequence":5,"event_type":"admission.queued","generation":1,"actor":"machine:PRINCIPAL-ID","timestamp":"2026-09-17T20:00:00Z","attempt_id":null,"candidate_label":null,"payload":{"outcome":{"lifecycle":"waiting","waiting_on":"admission"}}}}
{"protocol_version":1,"request_id":"wait-001","op":"await","run_id":"RUN-ID","after":4,"predicate":"attention_required","timeout_ms":10000}
```

The event example abbreviates only the outcome's fields; real events preserve the
existing complete `EventRecord` vocabulary and payload. `event_id` is the stable
`run_id:sequence` identity, outside the unchanged one-shot event envelope.
`subscription_id` identifies the request that emitted it. Sequence increases
within each run subscription; independent subscriptions may replay different
ranges. `events` drains journal pages after the cursor; `subscribe` also follows
for its bounded lifetime. Their final correlated response returns the resumable
cursor. The journal is authoritative, not terminal text or child JSON.

`commands::inspection::observe` uses `Database::event_page` read transactions and
bounded replay before each 25-ms idle poll. There is no notification-registration
window in which an event can be lost. It checks each event's committed outcome,
so a transient human question is observable even after it was answered. Supported
predicates are `attention_required`, `execution_finished`,
`waiting_reason_changed` (relative to the outcome at the supplied cursor; cursor
zero means initial `none`), and `state_changed`. Attention returns on human waiting
or finished execution, as the existing `follow::Until::Attention` contract does.
Automatic capacity/admission waits do not count as human attention. Already
satisfied terminal/human state can return immediately. A future cursor fails with
`invalid_cursor`; timeout returns `reached: false`, `timed_out: true`, and the last
observed cursor without cancelling or otherwise mutating work.

### Answer and cancel

```json
{"protocol_version":1,"request_id":"answer-001","op":"answer","run_id":"RUN-ID","question_id":"QUESTION-ID","revision":1,"generation":1,"answer":"blue"}
{"protocol_version":1,"request_id":"cancel-001","op":"cancel","run_id":"RUN-ID","revision":12}
```

Answer calls the shared `answer_question`/Phase 3 resolution and continuation
handlers. Question identity binds its run and attempt; revision and generation
must match the pending record. Only a grant with `--delegate-factual` can answer a
strict final checkpoint carrying optional `"category":"factual"`. Legacy reports
without category and unknown categories remain human-only. A checkpoint still
must be the final completed agent message in the exact structured envelope; a
category does not approve funding, unsafe execution, or review. The answer
retains `actor: "machine:PRINCIPAL-ID"`, and cannot become human evaluation data.

The continuation gets fresh admission/funding validation, and consumes the same
original two-invocation/deadline budget. The grant's profile snapshot is intersected
with current policy, never treated as proof that provider funding is still safe.
A policy change can therefore prevent continuation after an answer committed.

Cancel requires the current foreground session's owned run and exact state
revision. A stale snapshot is rejected; obtain status and issue a **new** request
ID for a changed revision. `commands::cancel` atomically appends
`cancel.requested`, its receipt, and a cancellation fence without prematurely
claiming child cleanup or invalidating the supervisor's current projection.
Then the existing cancellation token/supervisor handles drainage and admission
release. An idle question is closed through `cancel_question`. Wait/status/events
report the subsequent actual outcome.

A separate local CLI answer/cancel cannot take a live controller's question; the
shared handler checks the same grant ownership lock. Human review of completed
results remains available through Dispatch. Unclassified or approval questions
require human attention; this protocol does not forward a purported human flag
or offer a live approval/authority-transfer channel.

### Explicit recovery

```json
{"protocol_version":1,"request_id":"recover-001","op":"recover","run_id":"RUN-ID","revision":9}
```

`phase3::recover_checkpoint` is eligible only for an owned persisted checkpoint
with exactly one completed, nonfailed attempt, a pending or committed answered
question, no delivered result, no cancellation, unused original invocation
capacity, and an unexpired original absolute deadline. It authorizes the full ID
before resolving lock paths, acquires the run's exclusive operation lock, rejects
a live/uncertain previous process identity or unresolved admission, and checks
source fingerprint and current grant/project/resource policy. A pending question
is re-owned without launching; an already committed answer continues through
Phase 3. It never retries a partially executed or interrupted invocation.

Reopening a controller only initializes a session. Reading a result or replaying
an answer receipt never continues work. No implicit new goal evades a exhausted
budget. Source drift, unknown owners, missing state, expired grants/deadlines,
failed verification infrastructure, and consumed invocation budgets fail closed.

### Artifacts

```json
{"protocol_version":1,"request_id":"patch-001","op":"artifact","run_id":"RUN-ID","attempt_id":"ATTEMPT-ID","kind":"diff","offset":0}
```

Kinds are `diff`, `stdout`, and `stderr`. The server resolves these through the
owned run's completed attempt evidence, canonicalizes the path, and requires it
to remain within that exact attempt directory. The final file cannot be a
symlink. It accepts no arbitrary local path. Responses contain bounded UTF-8-lossy
preview text, byte `offset`, `next_offset`, and `eof`; artifact files are unchanged.
Result projections omit raw path fields and supply these references instead.

## Mutation receipts and errors

Submit, answer, cancel, and recover require request IDs scoped to the authenticated
principal. Canonical payload digests are SHA-256 over normalized JSON with sorted
object keys and explicit typed defaults. Whitespace/key ordering and omitted versus
null optional fields do not create a different command. Changed content with the
same ID yields `request_conflict`. Identical retries return the stored full reply.
Concurrent not-yet-committed identical IDs return `request_pending`; retry that ID.
Validation failures do not reserve IDs. Successful receipts are retained forever;
expired authority is rejected rather than rebinding an ID.

The command effect, receipt, and semantic event commit together in SQLite.
Initial creation includes the run itself in that transaction. All authoritative
writer connections use WAL and `synchronous=FULL`; no transaction spans snapshot
creation, probes, model work, checks, process inspection, output, or waits. Bulk
artifact logs retain their existing file path. Durable command effects are not
exactly-once OS execution: a crash after answer commit requires explicit eligible
recovery, not automatic replay. Access is rechecked before a receipt is returned.
Replaying an answer receipt after cancellation only describes that earlier commit;
it cannot reopen work or launch another continuation.

```json
{"type":"response","protocol_version":1,"request_id":"result-001","ok":false,"error":{"code":"not_ready","message":"not_ready: execution has not finished"}}
```

Recoverable errors include `invalid_json` (also invalid UTF-8), `invalid_request`
(missing/unknown fields or operations), `unsupported_version`,
`initialize_required`, `unauthorized`, `authorization_required`, `busy`,
`request_pending`, `request_conflict`, `stale_revision`, `invalid_cursor`,
`too_many_requests`, `not_ready`, `recovery_required`, `recovery_ineligible`, and
`command_rejected`. Rejected requests do not cause model execution. Do not parse
human diagnostic text as machine state. Oversized or incomplete frames close the
connection after a `framing_error` if output is writable. Broken/stalled output
cannot reliably deliver a final error; use the journal on reconnection.

## Standalone example and no-provider demonstration

The Python standard-library client demultiplexes responses/events on a reader
thread, permits an outstanding wait while answering, and closes stdin for cleanup:

```sh
python3 examples/control_client.py --binary target/release/dispatch \
  --state-dir /private/path/state \
  --grant /private/path/state/control-grants/GRANT-ID.key \
  --task 'Add tests in src/lib.rs' --factual-answer blue
```

That command uses the human-configured resources and may invoke a real harness.
For **fixture-only** demonstrations with no model, provider, or Herdr installation:

```sh
cargo build --release --locked
python3 tests/fixtures/phase5_control.py target/release/dispatch smoke
python3 tests/fixtures/phase5_control.py target/release/dispatch clarify
cargo test --locked --test phase5_control
```

The fixture creates a temporary project, private state, a deterministic executable
named `codex`, and a grant, then uses real OS pipes and the actual binary. It never
connects to a provider. These tests establish mechanics, not subscription savings
or multi-day product performance. See `phase5-validation.md` for the scenario map.
