//! A Markdown file drawn as mrk renders it, block by block, each block marked same, added, removed or changed.
use super::prose_view::Rows;
use super::theme::Theme;
use mrk::diff::BlockChange;
use mrk::document::{self, Block, Rgb, Settings};
use mrk::markdown::SourceBlock;
use mrk::theme::{Appearance, Palette};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::convert::identity;
use std::ops::Range;

/// The bar and the space after it.
pub const GUTTER: usize = 2;
/// A run of unchanged blocks longer than this folds, its first and last block kept.
const FOLD_FROM: usize = 3;
/// Every char of a block, to fade it whole.
const WHOLE: Range<usize> = 0..usize::MAX;
/// revu themes with an mrk preset of their own, so code blocks keep the palette's syntax scheme.
const PRESETS: [(&str, &str); 5] = [
    ("dracula", "dracula"),
    ("catppuccin", "catppuccin-mocha"),
    ("catppuccin-latte", "catppuccin-latte"),
    ("nord", "nord"),
    ("tokyonight", "tokyo-night"),
];

/// The rows of `old` and `new` drawn as prose in `width` columns, unchanged runs folded and open.
pub fn render(old: &str, new: &str, width: usize, theme: Theme) -> Rows {
    let settings = settings(theme, width.saturating_sub(GUTTER));
    let look = Look { theme, palette: settings.theme.palette };
    let changes = mrk::diff::blocks(old, new, &settings);
    Rows { folded: rows(&changes, false, &look), unfolded: rows(&changes, true, &look) }
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

/// What the rows are drawn with: revu's theme for the bars and fills, mrk's palette for the text.
struct Look {
    theme: Theme,
    palette: Palette,
}

impl Look {
    /// A removed word struck through in `danger`, on the removed word fill when the ground is known.
    fn removed_word(&self) -> impl Fn(document::Style) -> document::Style {
        let (fg, fill) = (self.palette.caution, self.theme.removed_word.and_then(rgb));
        move |style| document::Style { fg: Some(fg), bg: fill.or(style.bg), strike: true, ..style }
    }

    /// An added word underlined in `success`, on the added word fill when the ground is known.
    fn added_word(&self) -> impl Fn(document::Style) -> document::Style {
        let (fg, fill) = (self.palette.success, self.theme.added_word.and_then(rgb));
        move |style| document::Style { fg: Some(fg), bg: fill.or(style.bg), underline: true, ..style }
    }
}

/// One block of the view, or a fold standing for several.
enum Shown {
    Block { bar: Option<Color>, lines: Vec<document::Line> },
    Folded(usize),
}

fn rows(changes: &[BlockChange], unfolded: bool, look: &Look) -> Vec<Line<'static>> {
    let changed = changes.iter().any(|change| !is_same(change));
    let shown = changes.chunk_by(|a, b| is_same(a) && is_same(b)).flat_map(|run| match run {
        [BlockChange::Same { new: first, .. }, .., BlockChange::Same { new: last, .. }]
            if changed && !unfolded && run.len() > FOLD_FROM =>
        {
            let faded = Some(look.palette.subtle);
            vec![same(first, faded), Shown::Folded(run.len() - 2), same(last, faded)]
        }
        _ => run.iter().flat_map(|change| shown(change, changed, look)).collect(),
    });
    let drawn: Vec<Vec<Line<'static>>> = shown.map(|shown| drawn(shown, look)).collect();
    drawn.join(&Line::default())
}

fn is_same(change: &BlockChange) -> bool {
    matches!(change, BlockChange::Same { .. })
}

/// A change as the blocks it shows; same blocks fade only when something else changed.
fn shown(change: &BlockChange, changed: bool, look: &Look) -> Vec<Shown> {
    let (added, removed, faded) = (Some(look.theme.success), Some(look.theme.danger), Some(look.palette.subtle));
    match change {
        BlockChange::Same { new, .. } => vec![same(new, faded.filter(|_| changed))],
        BlockChange::Added(block) => vec![Shown::Block { bar: added, lines: marked(block, None, &[], identity) }],
        BlockChange::Removed(block) => vec![Shown::Block { bar: removed, lines: marked(block, faded, &[], identity) }],
        BlockChange::Changed { old, new, old_words, new_words } => {
            let new = Shown::Block { bar: added, lines: marked(new, None, new_words, look.added_word()) };
            if old_words.is_empty() && !new_words.is_empty() {
                return vec![new];
            }
            vec![Shown::Block { bar: removed, lines: marked(old, faded, old_words, look.removed_word()) }, new]
        }
    }
}

fn same(block: &SourceBlock, faded: Option<Rgb>) -> Shown {
    Shown::Block { bar: None, lines: marked(block, faded, &[], identity) }
}

/// The block's lines, all in the `faded` colour when given, then its `words` restyled.
fn marked(
    block: &SourceBlock,
    faded: Option<Rgb>,
    words: &[Range<usize>],
    restyle: impl Fn(document::Style) -> document::Style,
) -> Vec<document::Line> {
    let text: Vec<document::Line> = block.blocks.iter().flat_map(text_lines).cloned().collect();
    let text = match faded {
        Some(colour) => document::highlight(&text, &[WHOLE], |style| document::Style { fg: Some(colour), ..style }),
        None => text,
    };
    let lit = if words.is_empty() { text } else { document::highlight(&text, words, restyle) };
    let mut lit = lit.into_iter();
    block
        .blocks
        .iter()
        .flat_map(|part| match part {
            Block::Lines(lines) | Block::DoubleHeight(lines) => lit.by_ref().take(lines.len()).collect(),
            Block::Picture(picture) => vec![document::Line::new(vec![document::Span::plain(format!("[picture: {}]", picture.alt))])],
        })
        .collect()
}

/// The text of one part of a block, what mrk's word ranges count over; a picture has none.
fn text_lines(part: &Block) -> &[document::Line] {
    match part {
        Block::Lines(lines) | Block::DoubleHeight(lines) => lines,
        Block::Picture(_) => &[],
    }
}

fn drawn(shown: Shown, look: &Look) -> Vec<Line<'static>> {
    let text = look.palette.text;
    match shown {
        Shown::Block { bar, lines } => lines
            .into_iter()
            .map(|line| {
                let bar = Span::styled(if bar.is_some() { "▎ " } else { "  " }, Style::default().fg(bar.unwrap_or_default()));
                let spans = line.spans.into_iter().map(|span| Span::styled(span.text, style(span.style, text)));
                Line::from(std::iter::once(bar).chain(spans).collect::<Vec<_>>())
            })
            .collect(),
        Shown::Folded(count) => {
            vec![Line::styled(
                format!("  ··· {count} unchanged block{}", if count == 1 { "" } else { "s" }),
                Style::default().fg(look.theme.muted),
            )]
        }
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::single_range_in_vec_init)]
    use super::*;

    const README: &str = "# Widgets\n\nAcme widgets for nina.\n\n- one\n- two\n\nThe end.\n";

    fn theme() -> Theme {
        Theme::named("tokyonight").unwrap()
    }

    fn source(text: &str) -> Vec<SourceBlock> {
        mrk::markdown::render_blocks(text, &settings(theme(), 40))
    }

    fn block(text: &str) -> SourceBlock {
        source(text).remove(0)
    }

    fn same(text: &str) -> BlockChange {
        BlockChange::Same { old: block(text), new: block(text) }
    }

    /// Each row as its bar and text, trailing spaces cut.
    fn plain(rows: &[Line]) -> Vec<String> {
        rows.iter().map(|row| row.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_owned()).collect()
    }

    fn look() -> Look {
        Look { theme: theme(), palette: settings(theme(), 40).theme.palette }
    }

    fn span<'a>(rows: &'a [Line], text: &str) -> &'a Span<'a> {
        rows.iter().flat_map(|row| &row.spans).find(|s| s.content == text).expect(text)
    }

    #[test]
    fn an_edited_readme_marks_its_removed_heading_edited_paragraph_and_added_bullet_and_fades_the_rest() {
        let old = "# Widgets\n\n## Install\n\nRun the installer once.\n\n- one\n- two\n\nThe end.\n";
        let new = old.replace("## Install\n\n", "").replace("once", "twice").replace("- two\n", "- two\n- three\n");
        let rows = render(old, &new, 40, theme()).unfolded;
        assert_eq!(
            plain(&rows),
            [
                "  Widgets",
                &format!("  {}", "━".repeat(38)),
                "",
                "▎ ▍ Install",
                "",
                "▎ Run the installer once.",
                "",
                "▎ Run the installer twice.",
                "",
                "  • one",
                "",
                "  • two",
                "",
                "▎ • three",
                "",
                "  The end."
            ]
        );
        let bars: Vec<Option<Color>> = rows.iter().map(|row| row.spans.first().and_then(|bar| bar.style.fg)).collect();
        let (added, removed) = (Some(theme().success), Some(theme().danger));
        assert_eq!([bars[3], bars[5], bars[7], bars[13]], [removed, removed, added, added]);
        assert!(span(&rows, "once").style.add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(span(&rows, "twice").style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(span(&rows, "The end.").style.fg, Some(theme().faded), "unchanged blocks fade");
    }

    #[test]
    fn a_file_with_no_change_reads_as_plain_prose_nothing_faded_or_folded() {
        let rows = render(README, README, 40, theme()).folded;
        assert_eq!(
            plain(&rows),
            ["  Widgets", &format!("  {}", "━".repeat(38)), "", "  Acme widgets for nina.", "", "  • one", "", "  • two", "", "  The end."]
        );
        assert_eq!(span(&rows, "Acme widgets for nina.").style.fg, None, "mrk's text colour is the terminal's own");
    }

    #[test]
    fn added_and_removed_blocks_carry_their_bar_and_a_removed_one_fades() {
        let rows = rows(&[BlockChange::Added(block("New line.")), BlockChange::Removed(block("Old line."))], false, &look());
        assert_eq!(plain(&rows), ["▎ New line.", "", "▎ Old line."]);
        assert_eq!(rows[0].spans[0].style.fg, Some(theme().success));
        assert_eq!(rows[2].spans[0].style.fg, Some(theme().danger));
        assert_eq!(span(&rows, "Old line.").style.fg, Some(theme().faded));
    }

    #[test]
    fn a_changed_block_strikes_its_old_words_and_underlines_its_new_ones() {
        let change =
            BlockChange::Changed { old: block("Pay by card."), new: block("Pay by cash."), old_words: vec![7..11], new_words: vec![7..11] };
        let rows = rows(&[change], false, &look());
        assert_eq!(plain(&rows), ["▎ Pay by card.", "", "▎ Pay by cash."]);
        let (old, new) = (span(&rows, "card"), span(&rows, "cash"));
        assert!(old.style.add_modifier.contains(Modifier::CROSSED_OUT) && old.style.fg == Some(theme().danger));
        assert!(new.style.add_modifier.contains(Modifier::UNDERLINED) && new.style.fg == Some(theme().success));
        assert_eq!(span(&rows[2..], "Pay by ").style.fg, None, "kept words of the new side stay plain");
    }

    #[test]
    fn a_change_that_only_adds_words_shows_its_new_side_alone() {
        let change = BlockChange::Changed { old: block("Pay."), new: block("Pay now."), old_words: vec![], new_words: vec![3..7] };
        assert_eq!(plain(&rows(&[change], false, &look())), ["▎ Pay now."]);
    }

    #[test]
    fn a_long_unchanged_run_folds_to_its_ends_unless_unfolded() {
        let mut changes: Vec<BlockChange> = ["One.", "Two.", "Three.", "Four.", "Five."].into_iter().map(same).collect();
        changes.push(BlockChange::Added(block("Six.")));
        assert_eq!(plain(&rows(&changes, false, &look())), ["  One.", "", "  ··· 3 unchanged blocks", "", "  Five.", "", "▎ Six."]);
        assert_eq!(plain(&rows(&changes, true, &look())).len(), 11);
        assert_eq!(span(&rows(&changes, true, &look()), "Two.").style.fg, Some(theme().faded), "same blocks fade next to a change");
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
