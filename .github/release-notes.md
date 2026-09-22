## Dispatch 0.3.1 — Baseline no longer force-tracks ignored build output

**The defect.** For a Git source, `source::initialize_internal_repository` staged
every run's frozen baseline with `git add -A -f`, which force-tracks files the
source's own `.gitignore` or `info/exclude` excludes when they happen to exist at
snapshot time (build output such as `__pycache__/*.pyc` or `target/`). A verify
check that regenerated such an artifact in the candidate workspace then changed a
*tracked* file, so `collect_diff` put it into Δ; applying Δ wrote bytecode into the
user's source tree, unrelated runs "conflicted" on it even with disjoint real edits,
and an accept-time integration check could fail outright because the merged-tree
scratch copy (built from `git ls-files --exclude-standard`) correctly never had the
ignored path to begin with. The fix makes the baseline commit track exactly the
paths the world observes (`world::observe`, already ignore-rule-aware): a Git
source's baseline now stages with `git add -A -- .` (no `-f`) and also copies the
source's `info/exclude`, so ignored files stay on disk for build caches but are
never tracked, diffed, or applied. A plain-directory source is unchanged, since its
world has no ignore rules.

**Upgrading.** No database migration. Runs created before 0.3.1 keep their old
baselines and may still show an ignored artifact in their patches; only runs
created by 0.3.1 or later get the fix. See `docs/release-install.md`.

**Evidence.** Found by real-agent dogfood on a Python project with `__pycache__`
present.
