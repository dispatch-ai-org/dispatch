//! Presentation colors and the small session signature. No execution authority.
use ratatui::{
    style::{Color, Style},
    text::Span,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub accent: Style,
    pub foreground: Style,
    pub secondary: Style,
    pub inactive: Style,
    pub focus: Style,
    pub warning: Style,
    pub error: Style,
    pub success: Style,
    pub addition: Style,
    pub deletion: Style,
}

#[derive(Clone, Copy)]
enum Colors {
    Rgb,
    Indexed,
    Ansi,
    None,
}

impl Theme {
    pub fn from_env(no_color: bool) -> Self {
        let hint = |name| std::env::var(name).ok();
        Self::from_hints(
            no_color || std::env::var_os("NO_COLOR").is_some(),
            hint("DISPATCH_COLOR").as_deref(),
            hint("DISPATCH_THEME").as_deref(),
            hint("COLORTERM").as_deref(),
            hint("TERM").as_deref(),
        )
    }

    /// Deterministic capability inputs; no terminal queries or startup delay.
    /// These hints cannot reliably identify a terminal's background or custom
    /// ANSI palette. Light backgrounds use the explicit DISPATCH_THEME override.
    pub fn from_hints(
        no_color: bool,
        color: Option<&str>,
        theme: Option<&str>,
        colorterm: Option<&str>,
        term: Option<&str>,
    ) -> Self {
        let colors = if no_color {
            Colors::None
        } else {
            match color.unwrap_or("auto").to_ascii_lowercase().as_str() {
                "truecolor" => Colors::Rgb,
                "256" => Colors::Indexed,
                "16" => Colors::Ansi,
                "none" => Colors::None,
                "auto" if term == Some("dumb") => Colors::None,
                "auto" if matches!(colorterm, Some("truecolor" | "24bit")) => Colors::Rgb,
                "auto" if term.is_some_and(|t| t.contains("256color")) => Colors::Indexed,
                _ => Colors::Ansi,
            }
        };
        let light = theme.is_some_and(|s| s.eq_ignore_ascii_case("light"));
        let style = |rgb: (u8, u8, u8), indexed, ansi| match colors {
            Colors::Rgb => Style::default().fg(Color::Rgb(rgb.0, rgb.1, rgb.2)),
            Colors::Indexed => Style::default().fg(Color::Indexed(indexed)),
            Colors::Ansi => Style::default().fg(ansi),
            Colors::None => Style::default(),
        };
        let (accent, secondary, inactive, focus, warning, error, success) = if light {
            (
                style((0, 101, 120), 24, Color::Blue),
                style((71, 85, 105), 240, Color::DarkGray),
                style((100, 116, 139), 242, Color::DarkGray),
                style((0, 84, 105), 23, Color::Blue),
                style((138, 90, 0), 94, Color::Yellow),
                style((180, 35, 24), 124, Color::Red),
                style((8, 123, 76), 28, Color::Green),
            )
        } else {
            (
                style((103, 232, 249), 87, Color::Cyan),
                style((148, 163, 184), 248, Color::Gray),
                style((100, 116, 139), 243, Color::DarkGray),
                style((165, 243, 252), 159, Color::LightCyan),
                style((255, 210, 54), 220, Color::Yellow),
                style((248, 113, 113), 203, Color::LightRed),
                style((94, 233, 181), 85, Color::LightGreen),
            )
        };
        Self {
            accent,
            // Preserve the user's native foreground as well as background.
            foreground: Style::default(),
            secondary,
            inactive,
            focus,
            warning,
            error,
            success,
            addition: success,
            deletion: error,
        }
    }
}

/// Four-row open-D scheduler: staggered bars, native font, no image protocol.
pub fn signature(project: &str, width: u16, ascii: bool) -> String {
    let width = usize::from(width).min(40);
    if width == 0 {
        return String::new();
    }
    let project = super::sanitize(project).replace(['\n', '\t'], " ");
    if width < 16 {
        let mut lines = vec![fit("DISPATCH", width, ascii)];
        if !project.is_empty() {
            lines.push(fit(&project, width, ascii));
        }
        return lines.join("\n");
    }
    let rows = if ascii {
        ["  +--+", "==   |", "===  |", "==+--+"]
    } else {
        ["  ┌──╮", "━━   ┃", "━━━  ┃", "━━└──╯"]
    };
    format!(
        "{}  DISPATCH\n{}  {}\n{}  {}\n{}",
        rows[0],
        rows[1],
        fit(&project, width.saturating_sub(8), ascii),
        rows[2],
        fit("intent → work → review", width.saturating_sub(8), ascii)
            .replace('→', if ascii { ">" } else { "→" }),
        rows[3]
    )
}

fn fit(text: &str, width: usize, ascii: bool) -> String {
    if Span::raw(text).width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let cells = Span::raw(grapheme).width();
        if used + cells >= width {
            break;
        }
        result.push_str(grapheme);
        used += cells;
    }
    result.push(if ascii { '~' } else { '…' });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(color: &str, theme: &str) -> Theme {
        Theme::from_hints(false, Some(color), Some(theme), None, None)
    }

    #[test]
    fn exact_dark_brand_tokens_and_native_background() {
        let theme = palette("truecolor", "dark");
        assert_eq!(theme.accent.fg, Some(Color::Rgb(103, 232, 249)));
        assert_eq!(theme.focus.fg, Some(Color::Rgb(165, 243, 252)));
        assert_eq!(theme.secondary.fg, Some(Color::Rgb(148, 163, 184)));
        assert_eq!(theme.success.fg, Some(Color::Rgb(94, 233, 181)));
        assert_eq!(theme.warning.fg, Some(Color::Rgb(255, 210, 54)));
        assert_eq!(theme.foreground, Style::default());
        for style in [
            theme.accent,
            theme.foreground,
            theme.secondary,
            theme.inactive,
            theme.focus,
            theme.warning,
            theme.error,
            theme.success,
            theme.addition,
            theme.deletion,
        ] {
            assert_eq!(style.bg, None);
        }
    }

    #[test]
    fn explicit_light_variant_and_color_fallbacks() {
        let light = palette("truecolor", "light");
        assert_eq!(light.accent.fg, Some(Color::Rgb(0, 101, 120)));
        assert_eq!(light.warning.fg, Some(Color::Rgb(138, 90, 0)));
        assert_eq!(palette("256", "dark").accent.fg, Some(Color::Indexed(87)));
        assert_eq!(palette("16", "dark").accent.fg, Some(Color::Cyan));
        assert_eq!(palette("16", "light").accent.fg, Some(Color::Blue));
        assert_eq!(palette("none", "dark").accent, Style::default());
        assert_eq!(
            Theme::from_hints(false, Some("unknown"), None, Some("truecolor"), None)
                .accent
                .fg,
            Some(Color::Cyan)
        );
        let no_color = Theme::from_hints(
            true,
            Some("truecolor"),
            Some("dark"),
            Some("truecolor"),
            Some("xterm-256color"),
        );
        assert_eq!(no_color.accent, Style::default());
        assert_eq!(no_color.warning, Style::default());
    }

    #[test]
    fn auto_capabilities_have_conservative_fallbacks() {
        let auto = |colorterm, term| Theme::from_hints(false, None, None, colorterm, term);
        assert_eq!(auto(None, None).accent.fg, Some(Color::Cyan));
        assert_eq!(auto(None, Some("screen")).accent.fg, Some(Color::Cyan));
        assert_eq!(
            auto(None, Some("screen-256color")).accent.fg,
            Some(Color::Indexed(87))
        );
        assert_eq!(
            auto(Some("truecolor"), Some("xterm-256color")).accent.fg,
            Some(Color::Rgb(103, 232, 249))
        );
        assert_eq!(
            auto(Some("truecolor"), Some("dumb")).accent,
            Style::default()
        );
    }

    #[test]
    fn compact_unicode_ascii_and_narrow_signatures() {
        assert_eq!(
            signature("raylib", 80, false),
            "  ┌──╮  DISPATCH\n━━   ┃  raylib\n━━━  ┃  intent → work → review\n━━└──╯"
        );
        assert_eq!(
            signature("raylib", 80, true),
            "  +--+  DISPATCH\n==   |  raylib\n===  |  intent > work > review\n==+--+"
        );
        assert_eq!(signature("raylib", 12, false), "DISPATCH\nraylib");
        assert_eq!(signature("", 0, false), "");
        for width in [1, 8, 12, 16, 24, 40, 120] {
            for ascii in [false, true] {
                let text = signature("very long 界 project context with spaces", width, ascii);
                assert!(text.lines().count() <= 4);
                assert!(
                    text.lines()
                        .all(|l| Span::raw(l).width() <= usize::from(width).min(40))
                );
                assert!(!ascii || !text.contains(['●', '╰', '…']));
            }
        }
        assert!(!signature("x\x1b]52;c;secret\x07\ny", 40, false).contains("secret"));
    }
}
