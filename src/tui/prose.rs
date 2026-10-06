//! A Markdown file drawn as mrk renders it, block by block.
use super::theme::Theme;
use mrk::document::{self, Block, Rgb, Settings};
use mrk::theme::{Appearance, Palette};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The bar and the space after it, left free for the marks of a changed block.
pub const GUTTER: usize = 2;
/// revu themes with an mrk preset of their own, so code blocks keep the palette's syntax scheme.
const PRESETS: [(&str, &str); 5] = [
    ("dracula", "dracula"),
    ("catppuccin", "catppuccin-mocha"),
    ("catppuccin-latte", "catppuccin-latte"),
    ("nord", "nord"),
    ("tokyonight", "tokyo-night"),
];

/// The rows of the file drawn as prose in `width` columns: its head side, its base side once deleted.
pub fn render(old: &str, new: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let settings = settings(theme, width.saturating_sub(GUTTER));
    let source = if new.is_empty() { old } else { new };
    let text = settings.theme.palette.text;
    let blocks: Vec<Vec<Line<'static>>> = mrk::markdown::render_blocks(source, &settings)
        .iter()
        .map(|block| block.blocks.iter().flat_map(|part| drawn(part, text)).collect())
        .collect();
    blocks.join(&Line::default())
}

/// mrk's settings in revu's colours: the preset of the same theme, or mrk's own for the ground, then every RGB colour revu has.
pub fn settings(theme: Theme, width: usize) -> Settings {
    let preset = PRESETS.iter().find(|(revu, _)| *revu == theme.name).and_then(|(_, mrk)| mrk::theme::find(mrk));
    let base = preset.unwrap_or_else(|| mrk::theme::default_for(appearance(theme.base)));
    let own = |colour: Color, fallback: Rgb| rgb(colour).unwrap_or(fallback);
    let palette = Palette {
        accent: own(theme.accent, base.palette.accent),
        link: own(theme.link, base.palette.link),
        code: own(theme.code, base.palette.code),
        muted: own(theme.muted, base.palette.muted),
        subtle: own(theme.faded, base.palette.subtle),
        surface: own(theme.surface, base.palette.surface),
        success: own(theme.success, base.palette.success),
        warning: own(theme.warn, base.palette.warning),
        caution: own(theme.danger, base.palette.caution),
        ..base.palette
    };
    Settings { width, theme: mrk::theme::Theme { palette, ..base }, cell: None, hyperlinks: false, jumbo_title: None }
}

/// One part of a block after the empty gutter; a picture as its alternative text until pictures are drawn.
fn drawn(part: &Block, text: Rgb) -> Vec<Line<'static>> {
    let row = |spans: Vec<Span<'static>>| Line::from([vec![Span::raw("  ")], spans].concat());
    match part {
        Block::Lines(lines) | Block::DoubleHeight(lines) => lines
            .iter()
            .map(|line| row(line.spans.iter().map(|span| Span::styled(span.text.clone(), style(span.style, text))).collect()))
            .collect(),
        Block::Picture(picture) => vec![row(vec![Span::raw(format!("[picture: {}]", picture.alt))])],
    }
}

/// mrk's style as ratatui's; mrk's text colour becomes the terminal's own, as everywhere in revu.
fn style(style: document::Style, text: Rgb) -> Style {
    let colour = |Rgb(r, g, b): Rgb| Color::Rgb(r, g, b);
    let base = Style { fg: style.fg.filter(|&fg| fg != text).map(colour), bg: style.bg.map(colour), ..Style::default() };
    let modifiers = [
        (style.bold, Modifier::BOLD),
        (style.italic, Modifier::ITALIC),
        (style.underline, Modifier::UNDERLINED),
        (style.strike, Modifier::CROSSED_OUT),
        (style.dim, Modifier::DIM),
    ];
    modifiers.into_iter().filter(|(on, _)| *on).fold(base, |style, (_, modifier)| style.add_modifier(modifier))
}

fn rgb(colour: Color) -> Option<Rgb> {
    match colour {
        Color::Rgb(r, g, b) => Some(Rgb(r, g, b)),
        _ => None,
    }
}

/// Light when the ground is a known bright colour; an ANSI name says nothing, so dark.
fn appearance(ground: Color) -> Appearance {
    match rgb(ground) {
        Some(Rgb(r, g, b)) if u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 > 128_000 => Appearance::Light,
        _ => Appearance::Dark,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const README: &str = "# Widgets\n\nAcme widgets for nina.\n\n- one\n- two\n\nThe end.\n";

    fn theme() -> Theme {
        Theme::named("tokyonight").unwrap()
    }

    /// Each row as its gutter and text, trailing spaces cut.
    fn plain(rows: &[Line]) -> Vec<String> {
        rows.iter().map(|row| row.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_owned()).collect()
    }

    #[test]
    fn the_head_side_reads_as_prose_block_by_block_after_the_gutter() {
        let rows = render("", README, 40, theme());
        assert_eq!(
            plain(&rows),
            ["  Widgets", &format!("  {}", "━".repeat(38)), "", "  Acme widgets for nina.", "", "  • one", "", "  • two", "", "  The end."]
        );
        let span = rows.iter().flat_map(|row| &row.spans).find(|s| s.content == "Acme widgets for nina.").unwrap();
        assert_eq!(span.style.fg, None, "mrk's text colour is the terminal's own");
    }

    #[test]
    fn a_deleted_file_reads_its_base_side() {
        assert_eq!(plain(&render("Gone.\n", "", 40, theme())), ["  Gone."]);
    }

    #[test]
    fn settings_take_the_preset_of_the_same_theme_and_revu_colours_over_it() {
        let tokyo = settings(theme(), 40);
        assert_eq!(tokyo.theme.name, "tokyo-night");
        assert_eq!(Some(tokyo.theme.palette.accent), rgb(theme().accent));
        let plain = settings(Theme::default(), 40);
        assert_eq!(plain.theme.name, "mrk-dark", "the default theme's ANSI colours keep mrk's own");
        assert_eq!(plain.theme.palette, mrk::theme::find("mrk-dark").unwrap().palette);
        assert_eq!(settings(Theme::named("rosepine-dawn").unwrap(), 40).theme.name, "mrk-light");
    }

    #[test]
    fn styles_keep_every_modifier_and_drop_the_text_colour() {
        let text = Rgb(1, 2, 3);
        let mrk = document::Style { fg: Some(Rgb(9, 9, 9)), bg: Some(text), bold: true, strike: true, ..document::Style::default() };
        let expected =
            Style::default().fg(Color::Rgb(9, 9, 9)).bg(Color::Rgb(1, 2, 3)).add_modifier(Modifier::BOLD | Modifier::CROSSED_OUT);
        assert_eq!(style(mrk, text), expected);
        assert_eq!(style(document::Style::fg(text), text), Style::default());
    }
}
