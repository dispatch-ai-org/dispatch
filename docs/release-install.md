<!-- release: version -->
# Local release candidate: 0.1.3-rc.1

Release gate: **blocked** by the five-second planned-question deadline fixture in
the final four-thread suite. The same timing failure reproduces on the baseline.
Use these artifacts for local review; see `docs/product-rc-validation.md` in the
source checkout for the complete evidence before deciding on publication.

Validated runtime scope: macOS arm64. This is an unsigned local candidate, not a
published release. Linux has an existing CI recipe but was not executed for this
candidate; Windows, SSH/tmux and native GUI terminals are not certified here.

## Try without replacing an installation

Extract the local archive into a new directory, then run the executable by its
absolute path. `BUILD.json` records source and binary identities. From the output
directory, `shasum -a 256 -c SHA256SUMS` checks the archive and adjacent executable.
The archive contains only `dispatch`, `LICENSE`, `INSTALL.md`, and `BUILD.json`.
No account state, logs, grants, credentials or fonts are bundled.

```sh
mkdir -p /tmp/dispatch-rc-install
# Use the artifact directory returned by the packaging command.
tar -xzf /tmp/dispatch-rc/release/dispatch-macos-arm64.tar.gz -C /tmp/dispatch-rc-install
/tmp/dispatch-rc-install/dispatch version
cd /path/to/your/project
/tmp/dispatch-rc-install/dispatch --state-dir /path/to/private/dispatch-state setup
/tmp/dispatch-rc-install/dispatch --state-dir /path/to/private/dispatch-state setup --checks
/tmp/dispatch-rc-install/dispatch --state-dir /path/to/private/dispatch-state
```

<!-- release: version -->
The expected version is `dispatch 0.1.3-rc.1`. Use `command -v dispatch` and
`type -a dispatch` to locate older PATH installations. An explicit path is the
reliable way to select this candidate. Do not replace a working binary just to try
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

<!-- release: version -->
The database schema remains 20, so upgrading from a 0.1.x state directory runs no
migration. Work coherence adds one optional field to a run's stored record and
needs no backfill. A run created by an older version has no stored coherence data;
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

Uninstall by removing only the executable you installed and its archive/extraction
directory. Keep `~/.dispatch` (or your explicit state directory), project files and
provider accounts. State deletion and provider logout are separate owner actions.

## Reproduce local packaging

```sh
cargo build --release --locked --target-dir /tmp/dispatch-rc/current/target
python3 scripts/package-local.py /tmp/dispatch-rc/current/target/release/dispatch /tmp/dispatch-rc/release
```

The package metadata, ordering and compression are deterministic for the same
binary and declared source manifest. This does not claim bit-identical Rust builds
across different machines or toolchains. No signing keys, tags, pushes or uploads
are involved. Publication is a separate owner decision.
