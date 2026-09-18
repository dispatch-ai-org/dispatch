# Recorded Phase 8 terminal previews

These six PNGs and `.txt` files reconstruct visible cells from actual macOS PTY
output of the release binary and the disposable, synthetic
[`phase8_planning.py`](../../../tests/fixtures/phase8_planning.py) fixture.
They are **terminal-byte renderings, not native graphical screenshots**.
`capture-index.json` records the final binary hash, input-byte hashes, dimensions,
scenario and recorded milestone timing. `.render.json` records preview choices.

| Preview | Terminal / actual state |
|---|---|
| [Startup](pty_wide-startup.png) | 100×32, selected compact scheduler mark and opt-in composer hint |
| [Wide review](pty_wide-review.png) | 100×32, verified sequential delivery, one final review |
| [Narrow review](pty_narrow-review.png) | 38×32, verification summary before task details, review controls preserved |
| [Plain review](pty_plain-review.png) | 100×32 line-mode session; one ready-for-review summary |
| [Question](pty_question_deadline-question.png) | Actual pending question, two calls used, zero model leases |
| [Expired question](pty_question_deadline-deadline.png) | Original deadline expired, failed root, no continuation or apply candidate |

The fixtures use `--ascii --no-color`; the plain case also uses `--plain`. All four
sessions assert terminal restoration. Review cases open Details and the diff,
return to the same review, leave it pending, and exit normally. The deadline case
leaves the question unanswered and proves automatic expiry without spending again.
The displayed model names are synthetic configurations, not live-provider claims.

Raw ANSI milestone prefixes, full temporary transcripts, termios evidence and
process markers remain outside the repo at `/private/tmp/dispatch-phase8/captures`.
Only these bounded, inspected previews and reconstruction metadata are checked-in
candidates. No credentials, grants or real account observations are included.

Reproduce the sessions with the final release binary:

```sh
DISPATCH_REVIEW_CAPTURES=/tmp/dispatch-phase8-captures \
  python3 tests/fixtures/phase8_planning.py /path/to/release/dispatch \
  pty_plain pty_narrow pty_wide pty_question_deadline
```

Use the existing recorded-byte renderer (requires an existing Python/Pillow):

```sh
python3 docs/captures/phase4-refinement/render-capture.py \
  /tmp/dispatch-phase8-captures/pty_narrow.review.ansi \
  /tmp/dispatch-phase8-narrow.png --width 38 --height 32
```

The renderer uses Menlo at 18 px and a chosen dark `#070B10` preview background,
then crops wholly blank top/bottom rows. It fails on unsupported control sequences.
Dispatch itself preserves the terminal background. These previews do not validate
native terminal contrast, light themes, other fonts/platforms or graphical raylib.
See [the design/release handoff](../../phase8-design-handoff.md).
