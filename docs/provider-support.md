# Provider contracts and publication scope

Checked against primary provider documentation on September 18, 2026. Dispatch
uses the installed, unmodified provider CLI and its supported login flow. It does
not collect tokens, implement OAuth, bundle provider binaries, sell allowance, or
claim all models or accounts work.

| Contract | Evidence | Limit of the claim |
|---|---|---|
| Codex included ChatGPT allocation | Existing live Luna/low and Terra/medium direct tasks, durable clarification and apply/drift checks in [Phase 4](phase4-first-release.md); this candidate revalidates deterministic core/CLI/PTY fixtures | Historical resolved identities; observed model identity remained unknown. No fresh live call or current account validation in this pass. |
| Claude included controlled print | Prior successful live Claude Code **2.1.274**, first-party personal **Pro**, usage credits disabled, **claude-sonnet-5**, **medium**, standard mode with title-generation disabled; [full chronology](phase6-validation.md#second-authorized-live-smoke-passed) | The first authorized smoke failed auxiliary model authorization; the later smoke passed after the adapter control fix. Preserve both observations. New setup and final execution mechanics were fixture-tested here. |
| Planned Codex / Claude / mixed profiles | Deterministic planner/task protocols, different suitable tiers, sequential dependencies, root checks and one review | No live-planner certification. A genuine bounded goal requires separate owner authorization and fresh eligible profiles. |
| Managed accounts / cloud-provider auth / aliases / fast or paid fallback | Not part of the included Claude contract | Refused rather than silently normalized or enabled. |

[Codex authentication](https://developers.openai.com/codex/auth/) and
[CLI reference](https://developers.openai.com/codex/cli/reference/) document
provider-owned login, device auth and the distinction between ChatGPT and API
funding. Guided setup uses supported read-only account discovery and current core
funding fences; a CLI login alone is not evidence of model inclusion or no overage.

[Claude authentication](https://code.claude.com/docs/en/authentication) and
[CLI reference](https://code.claude.com/docs/en/cli-reference) document `auth login`
and auth status. Dispatch requires first-party subscription auth plus specific
owner assertions for its controlled invocation; quota may remain unknown.

**Owner publication decision:** review the current
[Claude legal/compliance terms](https://code.claude.com/docs/en/legal-and-compliance)
for this exact third-party product. The page distinguishes unmodified Claude Code
with the end user's own sign-in from third-party credential intermediation, and
states conditions for running Claude Code in a product. These are relevant limits,
not a blanket distribution approval for Dispatch. Decide the public Claude support
wording and obtain clarification if needed; keep it scoped experimental until then.
No legal conclusion or provider endorsement is asserted by fixture or live success.

A separately approvable live plan is already concrete in the
[Phase 8 handoff](phase8-design-handoff.md#separately-approvable-live-experiment):
a disposable raylib copy, one useful setting/documentation goal, original checks,
maximum four invocations and 600 seconds, no paid fallback, one human review. Exact
current eligible profile identities must be displayed at authorization time. This
pass did not inspect account credentials, run that experiment, or spend allowance.
