# Bounded planned work (opt-in experiment)

Direct execution remains the default. Phase 8 adds one explicit planned goal,
executed sequentially through the existing Rust admission, harness, supervision,
verification, question, review and safe-apply paths. It does not establish better
quality or lower cost. No private policy is activated.

## Start and review

Configure included-resource allocation and eligible Codex or Claude profiles using
[the shared terminal setup guide](product-guide.md). Planning needs a suitable
strong profile within those resources, executable checks, and the original
verification files identified by the owner. No provider/account change is implied.

```sh
dispatch run /path/to/project --task 'Implement the multipart goal' \
  --plan --max-invocations 4 --timeout 600 --allow-unsafe-local
```

In the human session, enter `/plan ` followed by the goal. The composer advertises
this option and discloses the maximum work before any invocation. Entering ordinary
text uses direct mode. The prefix is removed; the original goal wording is retained
separately from task proposals. Local execution still requires the existing
acknowledgement: agents and checks run with the user's permissions, not in a
hostile-code sandbox.

Follow the normal final review: `d` diff, `i` Details, `a` accept and safely apply,
`r` reject, or `n` leave pending. One-shot `dispatch diff`, `dispatch accept`, and
`dispatch reject` work on the same final candidate. Children have no acceptance
ceremony and never apply to the original project. A failed partial result cannot
be selected as a completed goal. Review after completed work does not consume the
work deadline; source drift and artifact identity are still checked at apply.

## Shared limits

| Rule | Planned goal | Direct goal |
|---|---|---|
| Planning | Exactly one permitted plan-generating call | None |
| Required tasks | 1–4, one initial call each | One initial attempt |
| Additional work | One extra shared by the entire goal | Existing one recovery/continuation allowance |
| Invocation ceiling | `min(owner/grant maximum, task count + 2, 6)` | At most 2 |
| Deadline | One persisted absolute deadline, starting before preparation | Existing original deadline |
| Default | Off; `--plan` or explicit machine request required | On |

CLI planned budgets accept 2–6; tighter budgets may leave no extra. A plan whose
initial tasks cannot fit is rejected before any child runs. An extra cannot consume
a required initial task's reserved room. A rejected planner result is retained and
never grants another planner call. These are Dispatch-supervised invocations;
controlled/unknown harness-internal composition keeps its existing meaning.

Prepared rows are not proof of a launch. Durable admission knowledge distinguishes
positively unlaunched work, uncertain spawn, recorded child and confirmed cleanup.
Uncertainty retains the reservation; there is no automatic refund/replay. Every
planner, child and extra gets a fresh attempt, current authorization, admission and
one-shot final launch fence. The immutable budget is enforced there as well as in
the driver. Within one goal, there is never more than one running harness.

`--no-retry` disables automatic coding repair and reasoning assistance. It permits
initial planned tasks and an authorized human answer or dependency continuation,
which still consumes the same one extra. All waiting, checks and integration share
the original deadline. Answers and reload do not reset it.

## Owner check and risk policy

A project configuration can contain:

```yaml
checks:
  verify:
    - sh ./verify.sh
planning:
  verification_paths:
    - verify.sh
    - tests/existing_regressions.c
  # Optional. Omit to keep uncertain bounded tasks at standard or stronger.
  # routine:
  #   - paths: [src/small_component.c]
  #     checks: [check-<digest of the approved command>]
```

`verification_paths` must identify the existing tests, wrappers and other files
that establish verification authority. Checks run in another workspace with these
paths restored from the original baseline. Select a directory to freeze its entire
contents, or individual original files to retain newly added test files alongside
them. Changes to these protected paths remain in the proposed delivery and are
reported in Details; they cannot weaken the original checks used for readiness.
This is an owner declaration, not automatic discovery of all test authority.
Checks can execute arbitrary approved commands with the configured backend's
permissions. Unknown/unconfigured verification cannot satisfy a dependency gate.

Check IDs are stable: `check-` plus the first 16 hexadecimal SHA-256 characters of
the JSON-serialized command string. For the command above:

```sh
python3 - <<'PY'
import hashlib, json
command = 'sh ./verify.sh'
print('check-' + hashlib.sha256(json.dumps(command, ensure_ascii=False,
      separators=(',', ':')).encode()).hexdigest()[:16])
PY
```

The catalog and owner policy are frozen before spending and included in the
planner prompt and persisted planning projection. Plan-proposed shell commands
are never accepted. v1 requires passing baseline checks; fix a failing baseline or
use direct work before starting this experiment.

Light eligibility requires at most two existing writable files, wholly covered
by one owner `routine` mapping, and all its relevant approved check references.
Planner labels or acceptance prose cannot establish that mapping. Otherwise,
directory scopes, more than four writable paths, or `.h`/`.proto` interface work
require strong; other bounded contracts require standard. These are conservative
deterministic v1 rules, not inferred quality. Current resource inclusion, funding,
capability, capacity, explicit model/effort/harness constraints and existing
preference rules still apply. A fixed strong model remains fixed for every role;
an incompatible fixed light model fails before planning. Direct suitability rules
and Phase 7 screening thresholds are unchanged.

## Plan and dependency contracts

Only the adapter's successful final result may provide this versioned envelope:

```json
{"dispatch_plan":{"version":1,"tasks":[{"id":"component","objective":"Implement the bounded component","read":["src/component.c"],"write":["src/component.c"],"acceptance":["The approved regression checks pass"],"checks":["check-APPROVED_ID"],"prerequisites":[],"inputs":["Original component"],"outputs":["Checked component change"]}]}}
```

The whole plan is limited to 32 KiB and 1–4 unique task IDs. Field/list/text lengths
are bounded; unknown fields, missing checks, self/duplicate/unknown dependencies,
cycles, traversal, reserved paths and symlink scopes fail. New-file parents must
resolve safely inside the source. Overlapping writes must be ordered by declared
dependencies. All contracts and resource feasibility are validated before children
start. Stable task-ID ordering chooses among runnable tasks. Read scope describes
the contract; it is not an operating-system read restriction. Validation cannot
prove natural-language completeness.

A checked dependency requires a successful producing attempt, passing referenced
checks, an immutable artifact manifest, and incorporation into a verified snapshot.
Exit zero or “done” text alone is insufficient. Undeclared earlier context remains
incidental, even though a sequential task sees the current integration snapshot.

A worker may end with the existing clarification envelope, or with:

```json
{"dispatch_dependency":{"version":1,"plan_revision":"ACTUAL_REVISION","task_id":"ACTUAL_TASK","attempt_id":"ACTUAL_ATTEMPT","generation":1,"prerequisite":"existing-task","assistance":null}}
```

The prompt supplies those identities. Exactly one of `prerequisite` and
`assistance` is present with a non-null value; assistance is bounded reasoning text.
The core checks identity, same-goal existence, direction, scope, cycles, terminal
state and remaining budget. It cannot add tasks or buy/select stronger resources
merely because a worker requests them. A valid missing prerequisite runs first;
the consumer then continues from a new recorded input using its checked artifact.
Already-usable prerequisites do not introduce a polling/wait loop. Incompatible
reinterpretation of completed branches is refused.

Factual questions use the existing exact question/revision/generation and authority
checks. Pending questions hold no model lease. Human answers continue automatically
inside the same root; a machine needs delegated factual authority. No question can
approve spending, permissions, review or a new plan. Foreground control and the
human session end pending questions at the original deadline; one-shot reload or
answer also enforces it.

## Snapshots, failures and reload

The original baseline B0 stays immutable. Each checked contribution is a binary
Git patch against its own input, applied with path validation and `git apply
--check` to a fresh integration workspace. The published tree must fingerprint
exactly like the checked output. Snapshot/manifest IDs, parent contributions,
consumed required artifacts and attempt identities persist under the root.

Child repair restarts from that child's immutable input. Direct recovery still
starts from the original root baseline. Dependency continuation uses the newer
integration input and bounded previous-diff diagnostics; it explicitly reconciles
checkpoint work instead of copying a stale workspace. Unsupported tree/patch
representations, out-of-scope writes, manifest/hash mismatches and stale integration
inputs stop safely. Existing source fidelity, binary, mode and apply safeguards
remain in force; this version conservatively rejects symlink write scopes.

Only an eligible target verification failure can automatically use the shared
strong repair. Infrastructure, uncertain cleanup and unknown failures do not earn
coding escalation. Root checks run again after every required task integrates;
one eligible root integration repair can consume the same extra if still available.
An unsuccessful or second extra need stops with retained evidence.

Artifact bytes are staged, synced and published before a committed readiness
reference. State and semantic events share the existing SQLite transition path.
There is no claim of a disk/database distributed transaction. Failed writes may
leave inspectable orphan evidence but cannot invent an integrated task. Final
review/apply verifies the final patch and contributing manifests/snapshots.

Owner loss is reported as interrupted with unresolved cleanup retained. Planned
crash recovery is explicitly unsupported in v1: `recover` refuses it. Inspect the
retained state and reconcile survival; reloading never launches a paid replay.
A separately authorized new run is a new goal, not a reset of the old budget.

To stop using planning, omit `/plan`/`--plan`, and issue future machine grants
without `--allow-plan`. History and already-pinned policy remain intact. Changing
configuration does not enlarge an existing grant or silently rewrite an active
plan. Existing cancellation remains available. Do not downgrade a state database
to a binary that does not support schema 20.

## Machine authorization and fixtures

```sh
dispatch control-grant /path/to/project --allow-plan --max-invocations 6 \
  --timeout 600 --allow-unsafe-local --delegate-factual
```

Grant files remain private inherited handles; see [control protocol](control-protocol.md).
A request must additionally include `"plan":true`. Legacy grants are direct-only.
The grant pins source/check configuration, eligible resources, expiry and policy
revision; client text cannot enlarge it. The grant command still defaults to two
invocations, even with `--allow-plan`, so set an intentional planned ceiling.
Duplicate submit/answer receipts return the existing root/decision.

Repeatable, disposable fixture demonstration (no live model):

```sh
cargo build --release --locked --target-dir /tmp/dispatch-phase8-target
python3 tests/fixtures/phase8_planning.py \
  /tmp/dispatch-phase8-target/release/dispatch success second_repair control pty_plain
```

The first case uses a strong planner, light C task, dependent standard C task,
actual compilation/checks and one integrated final diff. Its scripted accept tests
safe apply only; it is not a human quality label. `second_repair` preserves the
first repair and stops when a later task would require another. `control` and PTY
cases leave human review pending. Optional `raylib_copy` requires
`DISPATCH_RAYLIB_SOURCE=/absolute/path/to/the/raylib/project` and its local build
dependencies; only a disposable copy is changed. See [validation](phase8-validation.md)
for exact coverage, source/binary identity and empirical limits.
