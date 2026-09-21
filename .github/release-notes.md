## Dispatch 0.1.3-rc.1 — experimental local release candidate

Describe a goal, use explicitly validated included resources, follow bounded work,
verify configured checks, and review one exact change set. This candidate adds a
compact open-D scheduler identity, clearer sequential-task states, native large
review details, and one shared CLI/TUI resource setup and funding-revalidation flow.
Historical database upgrades preserve a private rollback copy; newer schemas fail
with an actionable message. Direct execution stays the default. Planning is opt-in.

This file is prepared for owner review; no release is published by the finish pass.
The release gate remains blocked by an unresolved deadline-fixture timing failure
in the full four-thread suite, also reproduced on the unmodified baseline.
The tested candidate scope is macOS arm64. Linux CI has not been run for this source.
Packages are unsigned. See `docs/product-rc-validation.md` for exact build identity,
checks, captures, provider chronology and remaining decisions. No general savings,
universal model support or live-planner certification is claimed.

Funding consent remains explicit. Setup never enables paid fallback, changes global
provider settings, activates a private policy, or widens a machine grant. Human
verification/review/application boundaries remain separate. Local execution is not
a security sandbox; planned crash recovery remains unsupported.
