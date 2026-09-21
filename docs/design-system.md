# Dispatch terminal design

Direction: a compact inline workspace with a strong goal and a quiet factual path.
The user's supplied screenshot informed hierarchy, cyan focus and progressive
detail; its fake parallel branches, Plan stage on direct work, quota tanks, clock,
model labels and message composer were not implemented.

The approved open-D scheduler is rebuilt in `assets/scheduler.svg`: a near-white
open arc with three round-ended cyan/emerald/amber bars, no glow or raster artifacts.
The four-row terminal treatment uses ordinary line glyphs, with three matching
bars and a legible ASCII fallback. It appears once at session start. There is no
font dependency, animation asset, mascot or raster terminal protocol.

`src/presenter/theme.rs` owns semantic accent/focus/foreground/secondary/inactive,
success/attention/failure and diff tokens. RGB brand values: cyan #67E8F9, focused
cyan #A5F3FC, near-white reference #F8FAFC, slate #94A3B8, emerald #5EE9B5, amber
#FFD236. Failure is a separate readable #F87171 token. Native foreground/background
remain the default; the preview renderer's #070B10 is explicitly a preview surface,
not a forced terminal background. Light mode uses dark teal/slate/ochre/red/green;
256-color and ANSI translations are intentional. Set `DISPATCH_THEME=light` on a
light terminal; background detection is not guessed. `DISPATCH_COLOR` can be
`truecolor`, `256`, `16`, or `none`. `NO_COLOR` removes color. Status words, +/- and
selection markers continue to carry meaning. Set `DISPATCH_REDUCED_MOTION=1` for
a static activity mark; elapsed time still updates truthfully.

Two-column margins, bold goal, one blank row before current state, and a compact
route establish hierarchy. Narrow direct routes become vertical; planned work is
explicitly sequential and uses stable objective rows with checked prerequisites.
Review collapses completed monitoring and puts its actions immediately below the
useful summary/preview. Large changes use the full terminal only when invoked.
Selected review file and running task are separate concepts. Muted pending state
is never success; unknown verification is attention. No predicted quality score.

The existing Ratatui/Crossterm/TextArea stack remains. One editor handles goals,
questions and setup text. Review owns its own command keys; Enter inspects and
never applies. Work input is explicitly paused. Draw calls are coalesced by content,
cursor and terminal size; animation is bounded and acknowledges real elapsed work.
A read-only sample of admission accounting cannot authorize or schedule work.
Reviewer and login handoffs relinquish the terminal, restore it on return and
preserve the displayed target. Inline mode preserves scrollback; only deliberate
inspection uses the alternate screen.

The actual terminal-byte captures in `docs/captures/product-finish` are rendered
with their dimensions, background and binary identity recorded. They are not
image-generated concepts or native GUI screenshots. Ordinary line geometry replaced
an earlier block-glyph cap after real visual inspection exposed a font rendering
problem. Native GUI terminal, tmux/SSH and Linux visual checks remain unclaimed.
