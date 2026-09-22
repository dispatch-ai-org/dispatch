# Dispatch 0.3.0: install, upgrade, uninstall

Dispatch 0.3.0 is an experimental developer preview. It adds
[auto-apply](product-guide.md#auto-apply): a session mode and a CLI flag that apply
an eligible finished result automatically, on top of
[work coherence](coherence.md), which validates finished work against the source as
it is now instead of refusing on any difference.

Runtime scope: macOS arm64 is the platform exercised interactively for this
release. Linux x86_64 is built and smoke-tested by CI (build, package, `dispatch
version`) but was not exercised interactively. Windows, SSH/tmux and native GUI
terminals are not certified. Packages are unsigned and not notarized.

## Try without replacing an installation

Download `dispatch-macos-arm64.tar.gz` (or `dispatch-linux-x86_64.tar.gz`) and
`SHA256SUMS` from the GitHub release, or build a local archive (see the end of this
page). Extract the archive into a new directory, then run the executable by its
absolute path. `BUILD.json` records source and binary identities. From the output
directory, `shasum -a 256 -c SHA256SUMS` checks the archive and adjacent executable.
The archive contains only `dispatch`, `LICENSE`, `INSTALL.md`, and `BUILD.json`.
No account state, logs, grants, credentials or fonts are bundled.

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
mkdir -p /tmp/dispatch-install
tar -xzf dispatch-macos-arm64.tar.gz -C /tmp/dispatch-install
/tmp/dispatch-install/dispatch version
cd /path/to/your/project
/tmp/dispatch-install/dispatch --state-dir /path/to/private/dispatch-state setup
/tmp/dispatch-install/dispatch --state-dir /path/to/private/dispatch-state setup --checks
/tmp/dispatch-install/dispatch --state-dir /path/to/private/dispatch-state
```

The expected version is `dispatch 0.3.0`. Use `command -v dispatch` and
`type -a dispatch` to locate older PATH installations. An explicit path is the
reliable way to select this build. Do not replace a working binary just to try
it. Ordinary work uses the provider's account; test-only setup must use fixture
homes and fake executables, as the acceptance scripts do.

## Upgrade, restore, uninstall

Stop all Dispatch sessions before upgrading. Back up the complete private state
directory and keep the matching binary. SQLite migrations now preserve a consistent
600-permission `dispatch.schema-N-*.db` backup beside the database before upgrading
historical schemas. These backups preserve database state, not artifact files; keep
a full state-directory copy as well. Schema remains 20. Newer schemas are refused.

To roll back, stop every session and restore a complete matching backup into a
separate directory; point the matching old binary at it with `--state-dir`. Never
point an old binary at current state or restore only a DB over newer artifacts.
Old grant scope does not expand: issue a new grant explicitly if changed resource
configuration invalidates it. Planned crash recovery remains unsupported.

### Upgrading to 0.2.0

Work coherence adds no database migration: the schema version stays at 20, the
version of the unreleased 0.1.3 candidate. A state directory created by the
published 0.1.x releases is older (v0.1.2 is at schema 11), so its first open runs
the normal historical migrations to 20 and leaves a private
`dispatch.schema-N-*.db` backup beside the database (see above). Work coherence
itself adds one optional field to a run's stored record and needs no backfill. A run created by an older version has no stored coherence data;
when you accept it, Dispatch derives everything it needs from the run's baseline,
its patch and the current source, so it gets coherence checking at accept time
automatically. It reads the configuration frozen with that run, which is a file
copied at run creation; if that copy cannot be read, accept keeps the strict
any-drift refusal.

The `coherence:` block in `dispatch.yml` is optional. With no block, accepting a
result onto a source that changed elsewhere now validates the patch instead of
refusing on any difference. To restore the old any-drift refusal, set
`coherence.accept: strict` before starting the run. See the
[coherence reference](coherence.md) for every key.

### Upgrading to 0.3.0

No database migration: the schema version stays 20, unchanged since 0.2.0.
Auto-apply adds one optional field to a run's stored outcome,
`RunOutcome.applied_by` (`human` or `auto_apply`), serialized only when a run was
actually applied; it is omitted entirely (not `null`) on every unapplied run and on
every run written by an older version, so old `run.json`/database records still
deserialize with no backfill. Every TUI session still starts with auto-apply off,
exactly as before this release, because the mode is session memory and was never
persisted. Nothing changes for existing runs: a run created by 0.2.0 or earlier is
reviewed exactly as it always was unless you explicitly opt into `--auto-apply` or
the session toggle for the *next* run.

Uninstall by removing only the executable you installed and its archive/extraction
directory. Keep `~/.dispatch` (or your explicit state directory), project files and
provider accounts. State deletion and provider logout are separate owner actions.

## Build a local archive

```sh
cargo build --release --locked --target-dir /tmp/dispatch-rc/current/target
python3 scripts/package-local.py /tmp/dispatch-rc/current/target/release/dispatch /tmp/dispatch-rc/release
```

The package metadata, ordering and compression are deterministic for the same
binary and declared source manifest. This does not claim bit-identical Rust builds
across different machines or toolchains. No signing keys, tags, pushes or uploads
are involved. Publication is a separate owner decision.
