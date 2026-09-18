//! Foreground presentation only. Committed outcomes remain execution authority.
use crate::{
    executor::CancellationToken,
    orchestrator::{self, Presentation, QuestionCommand, ReviewCommand, RunOutputMode, RunRequest},
    state::State,
    *,
};
use anyhow::{Context, Result};
mod handoff;
mod inspection;
mod theme;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use std::{
    future::Future,
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};
use theme::Theme;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Default)]
pub struct Options {
    pub plain: bool,
    pub ascii: bool,
    pub no_color: bool,
    pub no_retry: bool,
}

pub fn suitable() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Strip terminal control strings as units, including OSC clipboard/title and
/// DCS. Also remove C1 and bidi controls; preserve only useful text whitespace.
pub fn sanitize(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' || matches!(c, '\u{009b}' | '\u{009d}' | '\u{0090}') {
            let kind = if c == '\x1b' {
                chars.next()
            } else {
                Some(match c {
                    '\u{009b}' => '[',
                    '\u{009d}' => ']',
                    _ => 'P',
                })
            };
            match kind {
                Some('[') => {
                    for x in chars.by_ref() {
                        if ('@'..='~').contains(&x) {
                            break;
                        }
                    }
                }
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    while let Some(x) = chars.next() {
                        if x == '\x07' || x == '\u{009c}' {
                            break;
                        }
                        if x == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\n'
            || c == '\t'
            || (!c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        {
            result.push(c);
        }
    }
    result
}

pub fn label(run: &RunRecord, event: Option<&EventRecord>) -> &'static str {
    let o = &run.outcome;
    if o.application == ApplicationState::BlockedBySourceDrift {
        return "Application blocked: source changed";
    }
    if o.application == ApplicationState::Failed {
        return "Application failed";
    }
    if o.application == ApplicationState::Applied {
        return "Accepted and applied";
    }
    if o.review == ReviewState::Rejected {
        return "Rejected";
    }
    if o.review == ReviewState::Accepted {
        return "Accepted; not applied";
    }
    match o.waiting_on {
        WaitingOn::Human => return "Waiting for you",
        WaitingOn::Capacity => return "Waiting for subscription capacity",
        WaitingOn::Admission => return "Waiting for shared admission",
        WaitingOn::Reconciliation => return "Waiting for process reconciliation",
        WaitingOn::Authorization => return "Waiting for authorization",
        _ => {}
    }
    match o.work_result {
        WorkResult::Interrupted => return "Interrupted",
        WorkResult::Cancelled => return "Cancelled",
        WorkResult::Failed => return "Failed",
        WorkResult::Deferred => return "Deferred",
        WorkResult::Ready => return "Ready for review",
        _ => {}
    }
    if event.is_some_and(|e| e.event_type == "recovery.selected") {
        return "Recovering once after failed verification";
    }
    match o.phase {
        RunPhase::Verifying => "Verifying",
        RunPhase::Executing => "Working",
        RunPhase::Preparing => "Preparing",
        RunPhase::Reviewing => "Ready for review",
        RunPhase::Applying => "Applying",
        RunPhase::Finished => "Finished",
    }
}

fn model_name(model: &str) -> &str {
    // Display aliases only; exact identities remain in Details and machine output.
    match model {
        "gpt-5.6-luna" => "Luna",
        "gpt-5.6-terra" => "Terra",
        "gpt-5.6-sol" => "Sol",
        "gpt-6-astra" => "Astra",
        _ => model,
    }
}

fn verification_text(state: VerificationState) -> &'static str {
    match state {
        VerificationState::NotConfigured => "Unverified — no checks configured",
        VerificationState::NotRun => "Verification has not run",
        VerificationState::Passed => "Verification passed",
        VerificationState::Failed => "Verification failed",
        VerificationState::Inconclusive => "Verification inconclusive",
    }
}

pub fn projection(run: &RunRecord, event: Option<&EventRecord>, width: u16, ascii: bool) -> String {
    let (done, pending, failed, skip, active, inactive) = if ascii {
        ("*", "o", "x", "-", " == ", " -- ")
    } else {
        ("●", "○", "×", "–", " ━━ ", " ── ")
    };
    let model = run
        .attempts
        .last()
        .and_then(|a| a.resolved_model.as_deref())
        .or_else(|| {
            run.allocation
                .as_ref()
                .map(|d| d.selected.resolved_model.as_str())
        })
        .map(model_name)
        .unwrap_or("work");
    let work_mark = if run.attempts.is_empty() {
        pending
    } else {
        done
    };
    let verify_mark = match run.outcome.verification {
        VerificationState::Passed => done,
        VerificationState::Failed | VerificationState::Inconclusive => failed,
        VerificationState::NotConfigured => skip,
        VerificationState::NotRun => pending,
    };
    let review_mark = match run.outcome.review {
        ReviewState::Accepted | ReviewState::Pending
            if run.outcome.work_result == WorkResult::Ready =>
        {
            done
        }
        ReviewState::Rejected => failed,
        _ => pending,
    };
    let work_edge = if run.attempts.is_empty() {
        inactive
    } else {
        active
    };
    let verify_edge = if run.outcome.phase == RunPhase::Verifying
        || matches!(
            run.outcome.verification,
            VerificationState::Passed | VerificationState::Failed | VerificationState::Inconclusive
        ) {
        active
    } else {
        inactive
    };
    let review_edge = if run.outcome.work_result == WorkResult::Ready {
        active
    } else {
        inactive
    };
    let graph = if width >= 68 {
        format!(
            "{done} goal{work_edge}{work_mark} {model}{verify_edge}{verify_mark} verify{review_edge}{review_mark} review"
        )
    } else {
        format!(
            "{done} goal{work_edge}{work_mark} {model}\n{verify_mark} verify{review_edge}{review_mark} review"
        )
    };
    let mut lines = vec![
        goal_heading(&run.task, width),
        String::new(),
        label(run, event).to_owned(),
    ];
    if width >= 40 {
        lines.push(graph);
    }
    // One attempt is already represented by the work node. Show branches only
    // when the committed history contains recovery or clarification continuation.
    if run.attempts.len() > 1 {
        for attempt in &run.attempts {
            let prefix = match attempt.detail.reason.as_deref() {
                Some("target_verification_failure") => "Recovery · ",
                Some(_) => "Continuation · ",
                None => "",
            };
            let status = if attempt.detail.failure == Some(FailureKind::TargetVerification) {
                "verification failed"
            } else {
                match attempt.outcome.as_str() {
                    "preparing" => "preparing",
                    "running" => "working",
                    "completed" => "completed",
                    "failed" => "failed",
                    "cancelled" => "cancelled",
                    "interrupted" => "interrupted",
                    "timed_out" => "timed out",
                    "not_launched" => "not started",
                    _ => "state unavailable",
                }
            };
            let branch = if ascii {
                "+-"
            } else if attempt.ordinal == 1 {
                "├─"
            } else {
                "╰━"
            };
            lines.push(format!(
                "{branch} {prefix}Attempt {} · {}: {status}",
                attempt.ordinal,
                attempt
                    .resolved_model
                    .as_deref()
                    .map(model_name)
                    .unwrap_or("model unknown")
            ));
        }
    }
    if run.outcome.phase == RunPhase::Verifying || run.outcome.work_result != WorkResult::Pending {
        lines.push(verification_text(run.outcome.verification).to_owned());
    }
    if run.outcome.work_result == WorkResult::Ready
        && let [candidate] = run.candidates.as_slice()
    {
        let diff = &candidate.diff_stats;
        lines.push(format!(
            "{} file{} changed · +{} / -{} lines",
            diff.files_changed,
            if diff.files_changed == 1 { "" } else { "s" },
            diff.lines_added,
            diff.lines_removed
        ));
    }
    if let Some(q) = pending_question(run) {
        lines.push(q.report.question.clone());
        if !q.report.choices.is_empty() {
            lines.push(q.report.choices.join(" / "));
        }
    }
    if let Some(failure) = run.phase3.as_ref().and_then(|p| p.failure) {
        let reason = match failure {
            FailureKind::InvocationLimit => {
                "The two-invocation limit leaves no continuation budget."
            }
            FailureKind::Deadline => "The shared deadline expired; no continuation time remains.",
            FailureKind::Authorization => {
                "Current subscription authorization does not permit execution."
            }
            FailureKind::TargetVerification => "Configured verification failed.",
            FailureKind::VerificationInfrastructure => {
                "Verification infrastructure failed; automatic recovery is not permitted."
            }
            FailureKind::VerificationUnknown => {
                "Verification failed for an unknown reason; automatic recovery is not permitted."
            }
            FailureKind::Cancelled => "Execution was cancelled.",
            _ => "Work stopped; inspect Details for the recorded failure.",
        };
        lines.push(reason.into());
    }
    if let Some(reason) = event
        .and_then(|e| e.payload.get("reason").or_else(|| e.payload.get("error")))
        .and_then(serde_json::Value::as_str)
    {
        lines.push(reason.to_owned());
    }
    let text = sanitize(&lines.join("\n"));
    if ascii {
        text.replace('→', "->").replace(['—', '·'], "-")
    } else {
        text
    }
}

pub fn pending_question(run: &RunRecord) -> Option<&Clarification> {
    run.phase3
        .as_ref()?
        .questions
        .last()
        .filter(|q| q.state == QuestionState::Pending)
}
fn question_command(run: &RunRecord) -> Option<QuestionCommand> {
    pending_question(run).map(|q| QuestionCommand {
        run_id: run.id.clone(),
        question_id: q.id.clone(),
        revision: q.revision,
        generation: q.generation,
    })
}
pub fn review_command(run: &RunRecord) -> Result<ReviewCommand> {
    let [candidate] = run.candidates.as_slice() else {
        anyhow::bail!("delivery has no single candidate");
    };
    Ok(ReviewCommand {
        run_id: run.id.clone(),
        candidate_id: candidate.id.clone(),
        revision: run.state_revision,
    })
}

fn details(run: &RunRecord) -> String {
    let mut text = format!(
        "Goal: {}\nRun: {}\nState revision: {}",
        run.task, run.id, run.state_revision
    );
    if let Some(decision) = &run.allocation {
        text.push_str(&format!("\nAllocation: {}", decision.reason));
    }
    for attempt in &run.attempts {
        text.push_str(&format!(
            "\nAttempt {}: {} · {}",
            attempt.ordinal,
            attempt.resolved_model.as_deref().unwrap_or("unknown model"),
            attempt.outcome
        ));
    }
    for c in &run.candidates {
        text.push_str(&format!(
            "\nCandidate: {}\nPatch: {}\nOutput: {}\nErrors: {}",
            c.id,
            c.diff_path.display(),
            c.stdout_path.display(),
            c.stderr_path.display()
        ));
    }
    text
}

const INPUT_LIMIT: usize = 16 * 1024;
#[derive(Debug, PartialEq)]
enum Input {
    Submit(String),
    Cancel,
    Eof,
    Changed,
}
struct Editor {
    text: TextArea<'static>,
}
impl Editor {
    fn new() -> Self {
        let mut text = TextArea::default();
        text.set_cursor_line_style(Style::default());
        text.set_max_histories(32);
        Self { text }
    }
    fn event(&mut self, event: Event) -> Input {
        match event {
            Event::Paste(value) => {
                let value = sanitize(&value.replace("\r\n", "\n").replace('\r', "\n"));
                if self.text.lines().iter().map(String::len).sum::<usize>() + value.len()
                    <= INPUT_LIMIT
                {
                    self.text.insert_str(value);
                }
            }
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                    return Input::Cancel;
                }
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && k.code == KeyCode::Char('d')
                    && self.text.lines().iter().all(String::is_empty)
                {
                    return Input::Eof;
                }
                if k.code == KeyCode::Enter
                    && !k
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
                {
                    let value = self.text.lines().join("\n");
                    self.text = Self::new().text;
                    return Input::Submit(value);
                }
                if let KeyCode::Char(c) = k.code
                    && sanitize(&c.to_string()) != c.to_string()
                {
                    return Input::Changed;
                }
                if k.code == KeyCode::Enter {
                    self.text.insert_newline();
                } else if self.text.lines().iter().map(String::len).sum::<usize>() < INPUT_LIMIT
                    || !matches!(k.code, KeyCode::Char(_))
                {
                    self.text.input(k);
                }
            }
            _ => {}
        }
        Input::Changed
    }
}

type NativeTerminal = Terminal<CrosstermBackend<io::Stdout>>;
fn activity_hint(elapsed: Duration, ascii: bool) -> String {
    let frames = if ascii {
        ["|", "/", "-", "\\"]
    } else {
        ["◐", "◓", "◑", "◒"]
    };
    let frame = frames[(elapsed.as_millis() / 200 % 4) as usize];
    format!("{frame} {}s elapsed · Ctrl+C cancel", elapsed.as_secs())
}

fn content_area(area: Rect) -> Rect {
    let margin = if area.width > 4 { 2 } else { 0 };
    Rect::new(
        area.x + margin,
        area.y,
        area.width
            .saturating_sub(if margin > 0 { margin + 1 } else { 0 })
            .min(100),
        area.height,
    )
}

fn clip(text: &str, width: u16) -> String {
    let text = sanitize(text).replace(['\n', '\t'], " ");
    if UnicodeWidthStr::width(text.as_str()) <= usize::from(width) {
        return text;
    }
    let mut result = String::new();
    let mut cells = 0;
    for cluster in text.graphemes(true) {
        let n = UnicodeWidthStr::width(cluster);
        if cells + n + 3 > usize::from(width) {
            break;
        }
        cells += n;
        result.push_str(cluster);
    }
    result.push_str(&"..."[..usize::from(width).min(3)]);
    result
}
fn goal_heading(task: &str, width: u16) -> String {
    format!("Goal  {}", clip(task, width.saturating_sub(6)))
}

fn styled_body<'a>(body: &'a str, palette: &Theme) -> Text<'a> {
    Text::from(
        body.lines()
            .map(|line| {
                if line == "●─┬─○  DISPATCH" || line == "*-+-o  DISPATCH" {
                    let (mark, wordmark) = line.split_once("  ").unwrap();
                    return Line::from(vec![
                        Span::styled(mark, palette.accent),
                        Span::styled(format!("  {wordmark}"), palette.foreground.bold()),
                    ]);
                }
                if let Some(context) = line
                    .strip_prefix("  ╰─○")
                    .or_else(|| line.strip_prefix("  +-o"))
                {
                    return Line::from(vec![
                        Span::styled(&line[..line.len() - context.len()], palette.accent),
                        Span::styled(context, palette.secondary),
                    ]);
                }
                if line.starts_with("● goal") || line.starts_with("* goal") {
                    return Line::from(
                        line.chars()
                            .map(|c| {
                                Span::styled(
                                    c.to_string(),
                                    if matches!(c, '●' | '━' | '*' | '=') {
                                        palette.accent
                                    } else {
                                        palette.inactive
                                    },
                                )
                            })
                            .collect::<Vec<_>>(),
                    );
                }
                let style = if line.starts_with("Goal  ") {
                    palette.foreground.bold()
                } else if line.starts_with("Unverified")
                    || line.starts_with("Waiting")
                    || line.starts_with("Application blocked")
                {
                    palette.warning
                } else if line == "Verification passed" {
                    palette.success
                } else if line.contains("failed") || line == "Failed" || line == "Interrupted" {
                    palette.error
                } else if line == "Ready for review"
                    || line == "Working"
                    || line == "Preparing"
                    || line == "Verifying"
                {
                    palette.accent
                } else if line.contains("○") || line.contains(" == ") || line.contains(" ━━ ")
                {
                    palette.inactive
                } else {
                    palette.foreground
                };
                Line::styled(line, style)
            })
            .collect::<Vec<_>>(),
    )
}

fn draw_view(
    frame: &mut ratatui::Frame,
    body: &str,
    editor: Option<&Editor>,
    hint: &str,
    palette: &Theme,
    ascii: bool,
) {
    let area = content_area(frame.area());
    let input_height = editor.map_or(0, |e| {
        (e.text.lines().len().clamp(1, 3) as u16).min(area.height.saturating_sub(1))
    });
    let paragraph = Paragraph::new(styled_body(body, palette)).wrap(Wrap { trim: false });
    let body_height = paragraph
        .line_count(area.width)
        .min(area.height.saturating_sub(input_height + 1) as usize) as u16;
    frame.render_widget(
        paragraph,
        Rect::new(area.x, area.y, area.width, body_height),
    );
    if let Some(editor) = editor {
        frame.render_widget(
            Paragraph::new(if ascii { ">" } else { "›" }).style(palette.focus),
            Rect::new(
                area.x,
                area.y + body_height,
                area.width.min(2),
                input_height,
            ),
        );
        frame.render_widget(
            &editor.text,
            Rect::new(
                area.x + 2,
                area.y + body_height,
                area.width.saturating_sub(2),
                input_height,
            ),
        );
    }
    frame.render_widget(
        Paragraph::new(hint).style(palette.secondary),
        Rect::new(
            area.x,
            area.y + body_height + input_height,
            area.width,
            area.height
                .saturating_sub(body_height + input_height)
                .min(1),
        ),
    );
}

/// Text emphasis only, not a semantic diff engine. Keep +/- markers even without color.
fn diff_lines<'a>(text: &'a str, palette: &Theme) -> Vec<Line<'a>> {
    let lines: Vec<_> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let style = if line.starts_with('+') {
                palette.addition
            } else if line.starts_with('-') {
                palette.deletion
            } else if line.starts_with("@@") {
                palette.secondary
            } else {
                palette.foreground
            };
            let other = if line.starts_with('-')
                && !line.starts_with("--- ")
                && !i
                    .checked_sub(1)
                    .and_then(|j| lines.get(j))
                    .is_some_and(|l| l.starts_with('-'))
                && !lines.get(i + 2).is_some_and(|l| l.starts_with('+'))
            {
                lines.get(i + 1).filter(|l| l.starts_with('+'))
            } else if line.starts_with('+')
                && !line.starts_with("+++ ")
                && !lines.get(i + 1).is_some_and(|l| l.starts_with('+'))
                && !i
                    .checked_sub(2)
                    .and_then(|j| lines.get(j))
                    .is_some_and(|l| l.starts_with('-'))
            {
                i.checked_sub(1)
                    .and_then(|j| lines.get(j))
                    .filter(|l| l.starts_with('-'))
            } else {
                None
            };
            if let Some(other) = other {
                let a: Vec<_> = line[1..].graphemes(true).collect();
                let b: Vec<_> = other[1..].graphemes(true).collect();
                let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
                let suffix = a[prefix..]
                    .iter()
                    .rev()
                    .zip(b[prefix..].iter().rev())
                    .take_while(|(a, b)| a == b)
                    .count();
                let start = 1 + a[..prefix].iter().map(|g| g.len()).sum::<usize>();
                let end = line.len() - a[a.len() - suffix..].iter().map(|g| g.len()).sum::<usize>();
                Line::from(vec![
                    Span::styled(&line[..start], style),
                    Span::styled(&line[start..end], style.bold().underlined()),
                    Span::styled(&line[end..], style),
                ])
            } else {
                Line::styled(*line, style)
            }
        })
        .collect()
}

struct Screen {
    terminal: NativeTerminal,
    inspection: bool,
}
static ALTERNATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn restore() {
    if ALTERNATE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen);
    }
    let _ = execute!(
        io::stdout(),
        event::DisableBracketedPaste,
        crossterm::cursor::Show,
        crossterm::style::ResetColor
    );
    let _ = terminal::disable_raw_mode();
}
impl Screen {
    fn new() -> Result<Self> {
        Self::open(false)
    }
    fn open(inspection: bool) -> Result<Self> {
        // One hook across repeated reviewer handoffs; never accumulate closures.
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore();
                previous(info);
            }));
        });
        terminal::enable_raw_mode()?;
        let result = (|| {
            execute!(io::stdout(), event::EnableBracketedPaste)?;
            let (_, height) = terminal::size()?;
            if inspection {
                ALTERNATE.store(true, std::sync::atomic::Ordering::SeqCst);
                execute!(io::stdout(), terminal::EnterAlternateScreen)?;
            }
            let terminal = Terminal::with_options(
                CrosstermBackend::new(io::stdout()),
                TerminalOptions {
                    viewport: if inspection {
                        Viewport::Fullscreen
                    } else {
                        Viewport::Inline(height.clamp(1, 14))
                    },
                },
            )?;
            Ok(Self {
                terminal,
                inspection,
            })
        })();
        if result.is_err() {
            restore();
        }
        result
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        // The alternate viewport is discarded on return. In particular, the
        // panic hook may already have returned to the primary screen: clearing
        // this Fullscreen viewport then would erase the user's terminal display.
        if !self.inspection {
            let _ = self.terminal.clear();
        }
        restore();
    }
}

struct Ui {
    screen: Option<Screen>,
    #[cfg(not(unix))]
    plain_input: Option<tokio::sync::mpsc::Receiver<io::Result<String>>>,
    options: Options,
    palette: Theme,
    closed: bool,
    reviewed: Option<(
        RunRecord,
        std::result::Result<crate::private_evidence::AnnotationRequest, String>,
    )>,
    #[cfg(unix)]
    term: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hup: tokio::signal::unix::Signal,
}
impl Ui {
    fn new(options: Options) -> Result<Self> {
        #[cfg(unix)]
        let screen = if options.plain {
            None
        } else {
            Some(Screen::new()?)
        };
        #[cfg(not(unix))]
        let (screen, plain_input) = if options.plain {
            use std::io::{BufRead, Read};
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            std::thread::spawn(move || {
                loop {
                    let mut line = String::new();
                    let result = io::stdin()
                        .lock()
                        .take((INPUT_LIMIT + 1) as u64)
                        .read_line(&mut line);
                    if line.len() > INPUT_LIMIT {
                        let _ = tx.blocking_send(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "input exceeds 16 KiB",
                        )));
                        break;
                    }
                    if matches!(result, Ok(0)) {
                        break;
                    }
                    let value = result.map(|_| line.trim_end_matches(['\r', '\n']).to_owned());
                    if tx.blocking_send(value).is_err() {
                        break;
                    }
                }
            });
            (None, Some(rx))
        } else {
            (Some(Screen::new()?), None)
        };
        Ok(Self {
            screen,
            #[cfg(not(unix))]
            plain_input,
            options,
            palette: Theme::from_env(options.no_color),
            closed: false,
            reviewed: None,
            #[cfg(unix)]
            term: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(unix)]
            hup: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?,
        })
    }
    fn width(&self) -> u16 {
        terminal::size().map(|s| s.0).unwrap_or(80).min(103)
    }
    fn draw(&mut self, body: &str, editor: Option<&Editor>, hint: &str) -> Result<()> {
        let body = self.display_text(body);
        let hint = self.display_text(hint);
        let Some(screen) = &mut self.screen else {
            return Ok(());
        };
        screen.terminal.draw(|frame| {
            draw_view(
                frame,
                &body,
                editor,
                &hint,
                &self.palette,
                self.options.ascii,
            );
        })?;
        Ok(())
    }
    fn display_text(&self, text: &str) -> String {
        let text = sanitize(text);
        if self.options.ascii {
            text.replace('…', "...")
                .replace('·', "-")
                .replace('→', "->")
                .replace('—', "-")
        } else {
            text
        }
    }
    fn commit(&mut self, text: &str) -> Result<()> {
        self.commit_text(text, false)
    }
    fn commit_text(&mut self, text: &str, diff: bool) -> Result<()> {
        let text = self.display_text(text);
        if let Some(screen) = &mut self.screen {
            // Bounded blocks enter ordinary terminal scrollback, never alternate screen.
            let lines = if diff {
                diff_lines(&text, &self.palette)
            } else {
                styled_body(&text, &self.palette).lines
            };
            for chunk in lines.chunks(32) {
                let paragraph = Paragraph::new(Text::from(
                    chunk
                        .iter()
                        .map(|line| {
                            let mut padded = Line::from("  ");
                            padded.spans.extend(line.spans.clone());
                            padded.style = line.style;
                            padded
                        })
                        .collect::<Vec<_>>(),
                ))
                .wrap(Wrap { trim: false });
                let height = paragraph
                    .line_count(screen.terminal.size()?.width.clamp(1, 103))
                    .min(u16::MAX as usize) as u16;
                screen.terminal.insert_before(height, |buffer| {
                    use ratatui::widgets::Widget;
                    paragraph.render(
                        Rect::new(
                            buffer.area.x,
                            buffer.area.y,
                            buffer.area.width.min(103),
                            buffer.area.height,
                        ),
                        buffer,
                    );
                })?;
            }
        } else {
            // Plain mode preserves the patch verbatim (apart from terminal sanitization).
            println!("{text}");
        }
        Ok(())
    }
    async fn next(&mut self) -> Result<Option<Event>> {
        #[cfg(unix)]
        tokio::select! {
            _ = self.term.recv() => return Ok(None),
            _ = self.hup.recv() => return Ok(None),
            _ = tokio::signal::ctrl_c() => return Ok(Some(Event::Key(event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)))),
            _ = tokio::time::sleep(Duration::from_millis(30)) => {}
        }
        #[cfg(not(unix))]
        tokio::time::sleep(Duration::from_millis(30)).await;
        #[cfg(unix)]
        if self.options.plain {
            // Cooked terminal reads are line-ready; no background input reader
            // can prefetch a review action or compete with an external tool.
            let mut fd = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe {
                libc::poll(&mut fd, 1, 0);
            }
            if fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                return Ok(None);
            }
            if fd.revents & libc::POLLIN == 0 {
                return Ok(Some(Event::FocusGained));
            }
            let mut bytes = vec![0u8; INPUT_LIMIT + 1];
            let size = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
            if size == 0 {
                return Ok(None);
            }
            if size < 0 {
                return Err(io::Error::last_os_error().into());
            }
            anyhow::ensure!(size as usize <= INPUT_LIMIT, "input exceeds 16 KiB");
            return Ok(Some(Event::Paste(
                String::from_utf8_lossy(&bytes[..size as usize])
                    .trim_end_matches(['\r', '\n'])
                    .to_owned(),
            )));
        }
        #[cfg(not(unix))]
        if let Some(rx) = &mut self.plain_input {
            return match rx.try_recv() {
                Ok(line) => Ok(Some(Event::Paste(line?))),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => Ok(None),
                Err(_) => Ok(Some(Event::FocusGained)),
            };
        }
        #[cfg(unix)]
        {
            let mut fd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: 0,
                revents: 0,
            };
            // Only inspect hangup; Crossterm remains the single input reader.
            unsafe {
                libc::poll(&mut fd, 1, 0);
            }
            if fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                return Ok(None);
            }
        }
        if event::poll(Duration::ZERO)? {
            Ok(Some(event::read()?))
        } else {
            Ok(Some(Event::FocusGained))
        }
    }
    async fn prompt(&mut self, body: &str) -> Result<Input> {
        self.input_prompt(body, true).await
    }
    async fn command_prompt(&mut self, body: &str) -> Result<Input> {
        self.input_prompt(body, false).await
    }
    async fn input_prompt(&mut self, body: &str, compose: bool) -> Result<Input> {
        if self.closed {
            return Ok(Input::Eof);
        }
        let mut editor = Editor::new();
        if self.screen.is_none() {
            self.commit(body)?;
            print!("> ");
            io::stdout().flush()?;
        }
        loop {
            self.draw(
                body,
                Some(&editor),
                if compose {
                    "Enter send · Alt+Enter newline · Ctrl+C cancel · Ctrl+D exit"
                } else {
                    "Enter choose · Ctrl+C back · Ctrl+D exit"
                },
            )?;
            let Some(event) = self.next().await? else {
                self.closed = true;
                return Ok(Input::Eof);
            };
            if self.screen.is_none()
                && let Event::Paste(value) = event
            {
                return Ok(Input::Submit(sanitize(&value)));
            }
            match editor.event(event) {
                Input::Changed => {}
                Input::Eof => {
                    self.closed = true;
                    return Ok(Input::Eof);
                }
                input => return Ok(input),
            }
        }
    }
    fn discard_work_input(&mut self) -> Result<()> {
        // Bytes entered while working are never a queued answer or next goal.
        // Keep draining bounded; a flooded input stream closes the session.
        if self.options.plain {
            #[cfg(unix)]
            unsafe {
                libc::tcflush(0, libc::TCIFLUSH);
            }
            #[cfg(not(unix))]
            if let Some(rx) = &mut self.plain_input {
                while rx.try_recv().is_ok() {}
            }
            return Ok(());
        }
        for _ in 0..256 {
            if !self.closed && event::poll(Duration::ZERO)? {
                let _ = event::read()?;
            } else {
                return Ok(());
            }
        }
        self.closed = true;
        anyhow::bail!("input overflow while working; session closed")
    }
    async fn work<F: Future<Output = Result<RunRecord>>>(&mut self, work: F) -> Result<RunRecord> {
        let cancellation = CancellationToken::new();
        let (updates, mut rx) = tokio::sync::watch::channel(None);
        let work = orchestrator::present(
            Presentation {
                updates,
                cancellation: cancellation.clone(),
            },
            work,
        );
        tokio::pin!(work);
        let started = Instant::now();
        let mut body = "Preparing…".to_string();
        let mut latest = None;
        if self.screen.is_none() {
            self.commit(&body)?;
        }
        let mut last_label = String::new();
        let mut input_open = true;
        loop {
            if !cancellation.is_cancelled()
                && let Some((event, run)) = &latest
            {
                body = projection(run, Some(event), self.width(), self.options.ascii);
            }
            if let Err(error) = self.draw(
                &body,
                None,
                &activity_hint(started.elapsed(), self.options.ascii),
            ) {
                cancellation.cancel();
                let _ = (&mut work).await;
                return Err(error);
            }
            tokio::select! {
                result = &mut work => {
                    self.discard_work_input()?;
                    let run = result?;
                    let update = rx.borrow_and_update().clone();
                    let event = update.as_ref()
                        .filter(|(_, committed)| committed.id == run.id && committed.state_revision == run.state_revision)
                        .map(|(event, _)| event);
                    if self.options.plain || pending_question(&run).is_some() || run.outcome.work_result != WorkResult::Ready {
                        self.commit(&projection(&run, event, self.width(), self.options.ascii))?;
                    }
                    return Ok(run);
                },
                changed = rx.changed() => {
                    if changed.is_ok() {
                        let update = rx.borrow_and_update().clone();
                        if let Some((event, run)) = update {
                            let current = label(&run, Some(&event));
                            if self.screen.is_none() && current != last_label {
                                if let Err(error) = self.commit(current) {
                                    cancellation.cancel();
                                    let _ = (&mut work).await;
                                    return Err(error);
                                }
                                last_label = current.into();
                            }
                            latest = Some((event, run));
                        }
                    }
                }
                input = self.next(), if input_open => {
                    match input {
                        Ok(Some(Event::Key(k))) if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) => {
                            cancellation.cancel();
                            body = "Cancelling; waiting for process cleanup…".into();
                        }
                        Ok(Some(Event::Key(k))) if k.code == KeyCode::Char('d') && k.modifiers.contains(KeyModifiers::CONTROL) => {
                            cancellation.cancel();
                            input_open = false;
                            self.closed = true;
                            body = "Closing; waiting for process cleanup…".into();
                        }
                        Ok(None) | Err(_) => {
                            cancellation.cancel();
                            input_open = false;
                            self.closed = true;
                            body = "Terminal closed; waiting for process cleanup…".into();
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

pub async fn session(state: &State, mut options: Options) -> Result<()> {
    anyhow::ensure!(
        suitable(),
        "interactive input requires a terminal; use dispatch run \"<task>\""
    );
    options.plain |= std::env::var("TERM").is_ok_and(|v| v == "dumb");
    options.no_color |= std::env::var_os("NO_COLOR").is_some();
    let mut ui = Ui::new(options)?;
    let source = std::env::current_dir()?;
    let project = source.file_name().unwrap_or_default().to_string_lossy();
    if !options.plain {
        ui.commit(&theme::signature(
            &project,
            ui.width().saturating_sub(3),
            options.ascii,
        ))?;
    }
    loop {
        let prompt = if ui.reviewed.is_some() {
            "Next goal: What do you want to accomplish?\n[f] Use this review for local routing   [i] Details"
        } else {
            "What do you want to accomplish?"
        };
        let task = match ui.prompt(prompt).await? {
            Input::Submit(task) if ui.reviewed.is_some() && task.trim() == "f" => {
                let (run, request) = ui.reviewed.as_ref().unwrap().clone();
                if let Err(error) = attest_review(&mut ui, state, &run, request).await {
                    ui.commit(&format!("Local feedback not recorded: {}. Review and application are unchanged. Choose f to inspect this run again and explicitly confirm, or continue to the next goal.", clip(&format!("{error:#}"), 240)))?;
                    // Refresh only this displayed identity, never another session's latest run.
                    if let Ok(current) = state.load_run(&run.id) {
                        let request =
                            crate::private_evidence::prepare_review_annotation(state, &current)
                                .map_err(|error| format!("{error:#}"));
                        ui.reviewed = Some((current, request));
                    }
                }
                continue;
            }
            Input::Submit(task) if ui.reviewed.is_some() && task.trim() == "i" => {
                let (run, _) = ui.reviewed.as_ref().unwrap().clone();
                ui.diagnostics(&run).await?;
                continue;
            }
            Input::Submit(task) if !task.trim().is_empty() => task,
            Input::Eof | Input::Cancel => return Ok(()),
            _ => continue,
        };
        ui.reviewed = None;
        ui.draw(
            &format!(
                "{}\n\nGoal received. Preparing…",
                goal_heading(&task, ui.width().saturating_sub(3))
            ),
            None,
            "",
        )?;
        if options.plain {
            ui.commit(&format!("Goal received: {task}"))?;
        }
        let result = run_goal(&mut ui, state, &source, task, options).await;
        if let Err(error) = result {
            ui.commit(&format!("Stopped: {error:#}"))?;
        }
    }
}

async fn run_goal(
    ui: &mut Ui,
    state: &State,
    source: &std::path::Path,
    task: String,
    options: Options,
) -> Result<()> {
    let resources = crate::config::ResourceConfig::load(&state.root)?;
    anyhow::ensure!(
        resources.allocation_enabled,
        "Included-resource allocation is not configured for this session. Configure the validated subscription profiles in {} before running work; see the README allocation setup.",
        state.root.join("resources.yml").display()
    );
    let (config, _) = Config::discover(source, None)?;
    // This is an explicit per-goal host-execution permission, never trust from YAML.
    let local = config.execution.backend == "local";
    if local {
        match ui.prompt(&format!("{}\n\nLocal execution is not sandboxed.\nAgent and checks use your permissions in a separate workspace.\nAllow this goal? [y/N]",goal_heading(&task,ui.width().saturating_sub(3)))).await? {
            Input::Submit(answer) if matches!(answer.to_lowercase().as_str(), "y" | "yes") => {},
            _ => { ui.commit("Local execution was not authorized.")?; return Ok(()); }
        }
    }
    let request = RunRequest {
        source: source.to_path_buf(),
        task,
        harnesses: vec![],
        route: false,
        agent: None,
        model: None,
        effort: None,
        config_path: None,
        backend: None,
        timeout_secs: None,
        max_parallel: None,
        priority: 0,
        no_retry: options.no_retry,
        allow_unsafe_local: local,
        allow_forwarded_env: false,
        output: RunOutputMode::Silent,
    };
    let mut run = ui.work(orchestrator::run_dispatch(state, request)).await?;
    loop {
        if let Some(command) = question_command(&run) {
            match ui.prompt("Your answer (Ctrl+C cancels this goal)").await? {
                Input::Submit(answer) if !answer.trim().is_empty() => {
                    run = ui
                        .work(orchestrator::answer_question(
                            state,
                            command,
                            answer,
                            RunOutputMode::Silent,
                        ))
                        .await?;
                    continue;
                }
                Input::Submit(_) => continue,
                _ => {
                    run = orchestrator::cancel_question(state, command, RunOutputMode::Silent)?;
                    ui.commit(&projection(&run, None, ui.width(), options.ascii))?;
                    break;
                }
            }
        }
        if run.outcome.work_result != WorkResult::Ready
            || run.outcome.review != ReviewState::Pending
        {
            if let Input::Submit(action) = ui
                .command_prompt("Work stopped. [i] details  [n] next goal")
                .await?
                && matches!(action.trim(), "i" | "details")
            {
                ui.diagnostics(&run).await?;
                continue;
            }
            break;
        }
        return review_goal(ui, state, run, options).await;
    }
    Ok(())
}

async fn review_goal(
    ui: &mut Ui,
    state: &State,
    mut run: RunRecord,
    options: Options,
) -> Result<()> {
    let mut target = review_command(&run)?;
    ui.draw(
        &projection(&run, None, ui.width().saturating_sub(3), options.ascii),
        None,
        "Preparing change index…",
    )?;
    let state_copy = state.clone();
    let target_copy = target.clone();
    let mut bundle = tokio::task::spawn_blocking(move || {
        crate::reviewer::ReviewBundle::prepare(&state_copy, &target_copy)
    })
    .await??;
    let preview = inspection::tiny_preview(&bundle)?;
    let mut view = inspection::ReviewView::default();
    let mut notice = String::new();
    ui.input_boundary().await?;
    loop {
        let Input::Submit(action) = ui.review_action(&run, &preview, &notice).await? else {
            ui.commit("Left pending. No acceptance or application recorded.")?;
            return Ok(());
        };
        let action = action.trim().to_ascii_lowercase();
        match action.as_str() {
            "" | "d" | "diff" | "e" | "editor" => {
                let external = matches!(action.as_str(), "e" | "editor");
                if options.plain && external {
                    notice = "External reviewers require the interactive terminal. Use Review changes here.".into();
                    continue;
                }
                notice = ui.inspect(&mut bundle, &mut view, state, &target, external).await?;
                bundle.verify()?;
                match orchestrator::refresh_review_target(state, &target) {
                    Ok(current) => {
                        run = current;
                        target.revision = run.state_revision;
                    }
                    Err(error) => {
                        ui.commit(&format!("Review changed elsewhere: {error}. No action was applied."))?;
                        return Ok(());
                    }
                }
            }
            "i" | "details" => ui.diagnostics(&run).await?,
            "a" | "accept" | "r" | "reject" => {
                bundle.verify()?;
                let accept = matches!(action.as_str(), "a" | "accept");
                let result = orchestrator::review_delivery(state, &target, accept);
                run = state.load_run(&target.run_id)?;
                ui.commit(&projection(&run, None, ui.width().saturating_sub(3), options.ascii))?;
                if let Err(error) = result {
                    ui.commit(&format!("{error:#}"))?;
                }
                if matches!(run.outcome.review, ReviewState::Accepted | ReviewState::Rejected) {
                    let request = crate::private_evidence::prepare_review_annotation(state, &run)
                        .map_err(|error| format!("{error:#}"));
                    ui.reviewed = Some((run, request));
                }
                return Ok(());
            }
            "n" | "next" => {
                ui.commit("Left pending. You can review this result later.")?;
                return Ok(());
            }
            _ => notice = "Choose Review changes, Open in editor, Accept & apply, Reject, Leave pending, or Details.".into(),
        }
    }
}

async fn attest_review(
    ui: &mut Ui,
    state: &State,
    run: &RunRecord,
    request: std::result::Result<crate::private_evidence::AnnotationRequest, String>,
) -> Result<()> {
    let request = request.map_err(anyhow::Error::msg)?;
    ui.input_boundary().await?;
    let body = format!(
        "{}\nReview: {:?}\n\nThis was ordinary work and reflects my own review. Record it as local routing evidence. Routing will not change automatically.\n\n[c] Confirm this statement   [b] Back without recording",
        projection(run, None, ui.width().saturating_sub(3), ui.options.ascii)
            .lines()
            .filter(|line| !line.is_empty() && !line.contains(" ━━ ") && !line.contains(" == "))
            .collect::<Vec<_>>()
            .join("\n"),
        run.outcome.review
    );
    match ui.command_prompt(&body).await? {
        Input::Submit(action) if action.trim() == "c" => {
            crate::private_evidence::commit_annotation(state, &request)?;
            ui.commit("Local routing evidence recorded. Routing is unchanged.")?;
        }
        _ => ui.commit("No local feedback recorded. Review and application are unchanged.")?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn run() -> RunRecord {
        serde_json::from_value(serde_json::json!({
            "id":"01TEST", "task":"Fix cache", "exact_prompt":"Fix cache",
            "source_path":"/source", "source_kind":"directory", "source_git_head":null,
            "source_fingerprint":"baseline", "baseline_path":"/baseline", "baseline_commit":"abc",
            "status":"running", "created_at":"2026-09-17T00:00:00Z", "completed_at":null,
            "environment":{"dispatch_version":"test","os":"test","architecture":"test","execution_backend":"local","timeout_secs":30,"cpus":1.0,"memory":"1g","max_parallel":1},
            "evaluation":null,"applied_candidate":null
        })).unwrap()
    }
    fn render(text: &str, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
        terminal
            .draw(|f| f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), f.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(width as usize)
            .map(|row| {
                row.iter()
                    .map(|c| c.symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }
    #[test]
    fn state_render_wide_narrow_ascii_snapshots() {
        let mut r = run();
        r.outcome.work_result = WorkResult::Ready;
        r.outcome.verification = VerificationState::Passed;
        r.outcome.phase = RunPhase::Reviewing;
        r.outcome.review = ReviewState::Pending;
        assert_eq!(
            render(&projection(&r, None, 90, false), 90),
            "Goal  Fix cache\n\nReady for review\n● goal ── ○ work ━━ ● verify ━━ ● review\nVerification passed"
        );
        assert_eq!(
            render(&projection(&r, None, 24, true), 24),
            "Goal  Fix cache\n\nReady for review\nVerification passed"
        );
        for (waiting, expected) in [
            (WaitingOn::Human, "Waiting for you"),
            (WaitingOn::Admission, "Waiting for shared admission"),
            (WaitingOn::Capacity, "Waiting for subscription capacity"),
        ] {
            r.outcome.waiting_on = waiting;
            assert_eq!(label(&r, None), expected);
        }
        r.outcome.application = ApplicationState::BlockedBySourceDrift;
        assert_eq!(label(&r, None), "Application blocked: source changed");
    }
    #[test]
    fn compact_input_and_activity_survive_resize() {
        let mut editor = Editor::new();
        editor.event(Event::Paste("α\n界".into()));
        for width in [90, 24, 8, 1, 90] {
            let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
            terminal
                .draw(|f| {
                    draw_view(
                        f,
                        "Your answer",
                        Some(&editor),
                        "Enter send",
                        &Theme::from_hints(true, None, None, None, None),
                        true,
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let area = content_area(Rect::new(0, 0, width, 8));
            let body_height = Paragraph::new("Your answer")
                .wrap(Wrap { trim: false })
                .line_count(area.width)
                .min(5) as u16;
            assert_eq!(buffer[(area.x, body_height)].symbol(), ">");
            assert_eq!(editor.text.lines(), &["α", "界"]);
        }
        assert_eq!(
            activity_hint(Duration::ZERO, true),
            "| 0s elapsed · Ctrl+C cancel"
        );
        assert_eq!(
            activity_hint(Duration::from_millis(1200), true),
            "- 1s elapsed · Ctrl+C cancel"
        );
        assert_ne!(
            activity_hint(Duration::ZERO, false),
            activity_hint(Duration::from_millis(200), false)
        );
    }
    #[test]
    fn tiny_diff_emphasizes_only_changed_graphemes() {
        let palette = Theme::from_hints(true, None, None, None, None);
        let lines = diff_lines("- int radius = 20;\n+ int radius = 40;", &palette);
        assert_eq!(lines[0].spans[1].content, "2");
        assert_eq!(lines[1].spans[1].content, "4");
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
        assert_eq!(lines[0].spans[1].style.fg, None);
        for text in ["-é🙂\n+é界", "-\n+new", "-old\n+", "-same\n+same"] {
            let displayed = diff_lines(text, &palette)
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(displayed, text);
        }
    }
    #[test]
    fn unconfigured_checks_never_look_verified() {
        let mut r = run();
        r.outcome.work_result = WorkResult::Ready;
        r.outcome.verification = VerificationState::NotConfigured;
        let text = projection(&r, None, 90, false);
        assert!(text.contains("Unverified — no checks configured"));
        assert!(!text.contains("● verify"));
        let palette = Theme::from_hints(false, Some("truecolor"), Some("dark"), None, None);
        let rendered = styled_body(&text, &palette);
        assert!(
            !rendered
                .lines
                .iter()
                .any(|line| line.style == palette.success)
        );
    }
    #[test]
    fn malicious_content_cannot_control_terminal() {
        assert_eq!(
            sanitize(
                "a\x1b[31mred\x1b[0m\x1b]52;c;secret\x07b\x1bPpayload\x1b\\c\u{202e}d\u{009b}2J\x00"
            ),
            "aredbcd"
        );
        assert_eq!(sanitize("x\x1b]unfinished"), "x");
    }
    #[test]
    fn paste_unicode_resize_submit_once() {
        let mut editor = Editor::new();
        assert_eq!(editor.event(Event::Paste("α\n界🙂".into())), Input::Changed);
        for (w, h) in [(100, 30), (18, 5), (80, 24)] {
            editor.event(Event::Resize(w, h));
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| f.render_widget(&editor.text, f.area()))
                .unwrap();
        }
        editor.event(Event::Key(event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            editor.event(Event::Key(event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            Input::Submit("α\n界".into())
        );
        assert_eq!(
            editor.event(Event::Key(event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            Input::Submit(String::new())
        );
        assert_eq!(
            editor.event(Event::Key(event::KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL
            ))),
            Input::Eof
        );
        assert_eq!(
            editor.event(Event::Key(event::KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Input::Cancel
        );
    }
    #[test]
    fn resize_work_and_question_preserves_committed_projection() {
        let mut r = run();
        r.outcome.lifecycle = LifecycleState::Working;
        r.outcome.phase = RunPhase::Verifying;
        let before = serde_json::to_value(&r).unwrap();
        for width in [100, 18, 1, 80] {
            let _ = render(&projection(&r, None, width, false), width);
        }
        assert_eq!(serde_json::to_value(&r).unwrap(), before);
        r.outcome.waiting_on = WaitingOn::Human;
        for width in [80, 18] {
            assert!(projection(&r, None, width, true).contains("Waiting for you"));
        }
    }
    #[test]
    #[ignore = "subprocess helper, exercised by terminal_panic_restores_in_pty"]
    fn terminal_panic_fixture() {
        assert!(suitable());
        let inspection = std::env::var_os("DISPATCH_TEST_INSPECTION_PANIC").is_some();
        println!("primary-terminal-sentinel");
        io::stdout().flush().unwrap();
        let result = std::panic::catch_unwind(|| {
            let _screen = Screen::open(inspection).unwrap();
            println!("panic-fixture-ready");
            panic!("intentional terminal restoration test");
        });
        assert!(result.is_err());
        if inspection {
            // A real shell reads after return. On macOS that read also clears
            // the kernel's transient PENDIN flag after raw -> cooked mode.
            println!("panic-restored-read");
            let mut line = String::new();
            io::stdin().read_line(&mut line).unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn terminal_panic_restores_in_pty() {
        let temp = tempfile::tempdir().unwrap();
        for scenario in ["panic", "inspection-panic"] {
            let output = std::process::Command::new("python3")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/phase4_session.py"
                ))
                .arg(std::env::current_exe().unwrap())
                .arg(temp.path())
                .arg(temp.path())
                .arg(scenario)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
