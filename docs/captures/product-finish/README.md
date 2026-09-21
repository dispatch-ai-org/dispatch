# Product finish: real before / after evidence

These are actual Dispatch PTY byte streams decoded by the repository's VT
renderer, using Menlo for the preview. They are **not native GUI screenshots or
image-generated mockups**. Only blank outer rows are cropped. Each image has a
text companion and render metadata recording terminal size, preview background,
raw-byte SHA-256 and elapsed milestone. The raw recordings and binary identities
are preserved in `raw/`. Every provider in these captures is synthetic.

Baseline: clean `08fc0a7633d18c1cae39ab93cd36fc5405798cc1`, version 0.1.2,
separate debug build. After: the release executable identified in
[the validation report](../../product-rc-validation.md). The comparison demonstrates
presentation changes; debug/release or fixture timings are not provider benchmarks.

| Journey | Before | After | Inspection finding |
|---|---|---|---|
| Startup, 120×40 | [Before](before/direct-startup.png) | [After](after/direct-startup.png) | Open-D identity, project context, direct default and discoverable setup |
| Direct work | [Before](before/direct-working.png) | [After](after/direct-working.png) | Actual work/check/review route, factual activity, launch accounting |
| Sequential plan | [Before](before/planned-working.png) | [After](after/planned-working.png) | Readable objectives, resource/state and real checked prerequisites |
| Question | [Before](before/attention-question.png) | [After](after/attention-question.png) | Decision effect, exact budget/deadline and answer/cancel; no running claim while waiting |
| Failed | [Before](before/failed-failed.png) | [After](after/failed-failed.png) | Failure distinct from verification and review |
| Unverified | [Before](before/unverified-review.png) | [After](after/unverified-review.png) | No configured checks stays amber and explicit |
| Small review | [Before](before/direct-review.png) | [After](after/direct-review.png) | Controls immediately below the useful preview |
| Large index | [Before](before/large-file-index.png) | [After](after/large-file-index.png) | 53 entries, rename relationship, binary/generated hints, full available space |
| Long-line file | [Before](before/large-file.png) | [After](after/large-file.png) | Right-edge clipping mark, column/pan guidance and explicit truncated-preview/full-patch action |
| Narrow plan review, 38×24 | [Before](before/narrow-review.png) | [After](after/narrow-review.png) | Verification remains visible; completed tasks collapse |
| Light, 80×24 | [Before](before/light-review.png) | [After](after/light-review.png) | Dark foreground and readable teal/green/red on white |
| No color / ASCII, 80×24 | [Before](before/mono-review.png) | [After](after/mono-review.png) | Meaning retained without color or Unicode |
| Missing configuration | [Before](before/missing-setup.png) | [After](after/missing-setup.png) | Guided setup replaces a dead-end error and preserves the goal |

Setup is a new surface, so there is no invented baseline counterpart. Inspect the
[explicit Claude confirmation](setup/cli-claude-confirm.png),
[expired evidence](setup/revalidate-expired.png),
[refresh proposal](setup/revalidate-refresh.png),
[changed account refusal](setup/account-change-blocked.png),
[login return](setup/login-return.png), and
[preserved goal](setup/fresh-preserved.png). These are 100×36 PTYs with disposable
homes, fake accounts and zero model invocations. No real login was initiated.

Visual inspection led to concrete fixes: ordinary line geometry replaced a poorly
rendered block cap; review stopped leaving a large gap before its actions; narrow
review retained verification; long lines gained clipping markers; and waiting for
a question stopped claiming an agent was running. The final frames were inspected
again after those fixes. Linux, native GUI terminal emulators, SSH and tmux were
not visually certified.

## Repeat the fixture demo

From the repository root, using the candidate's explicit executable path:

```sh
python3 scripts/capture-product.py /absolute/path/to/dispatch /tmp/dispatch-demo
DISPATCH_SETUP_CAPTURES=/tmp/dispatch-setup-demo python3 tests/fixtures/resource_setup.py /absolute/path/to/dispatch
```

The scripts drive real terminal input and execution in disposable projects. They
write timed `.cast` recordings, VT output, terminal-restoration evidence and
milestone metadata. They require local controlling-terminal permissions. Optional
preview generation requires Pillow, not a provider or an image model:

```sh
python3 scripts/render-product.py /tmp/dispatch-demo /tmp/dispatch-demo-images
```

The checked-in images/text are the convenient inspection format; raw fixture
bytes provide provenance and allow re-rendering.
