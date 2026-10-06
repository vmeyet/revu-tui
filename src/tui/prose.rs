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
/// The blank column between the two sides.
const GAP: usize = 1;
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

/// The rows of `old` and `new` drawn as prose in `width` columns, inline or `beside` each other, unchanged runs folded and open.
pub fn render(old: &str, new: &str, width: usize, beside: bool, theme: Theme) -> Rows {
    let layout = if beside { Layout::Beside { half: width.saturating_sub(GAP) / 2 } } else { Layout::Inline };
    let settings = settings(theme, layout.side_width(width).saturating_sub(GUTTER));
    let look = Look { theme, palette: settings.theme.palette };
    let changes = mrk::diff::blocks(old, new, &settings);
    Rows { folded: rows(&changes, false, layout, &look), unfolded: rows(&changes, true, layout, &look) }
}

/// Every block in one column, or the old side on the left and the new one on the right, each `half` columns wide.
#[derive(Clone, Copy)]
enum Layout {
    Inline,
    Beside { half: usize },
}

impl Layout {
    /// The columns one side is drawn in, its bar included.
    fn side_width(self, width: usize) -> usize {
        match self {
            Self::Inline => width,
            Self::Beside { half } => half,
        }
    }
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

/// One block after its bar; a same block has none.
struct Marked {
    bar: Option<Color>,
    lines: Vec<document::Line>,
}

/// One block of the view, the two sides of a change `half` columns wide each, or a fold standing for several.
enum Shown {
    Block(Marked),
    Beside { old: Option<Marked>, new: Option<Marked>, half: usize },
    Folded(usize),
}

fn rows(changes: &[BlockChange], unfolded: bool, layout: Layout, look: &Look) -> Vec<Line<'static>> {
    let changed = changes.iter().any(|change| !is_same(change));
    let shown = |change: &BlockChange| shown(change, changed, layout, look);
    let all = changes.chunk_by(|a, b| is_same(a) && is_same(b)).flat_map(|run| match run {
        [first, .., last] if is_same(first) && changed && !unfolded && run.len() > FOLD_FROM => {
            [shown(first), vec![Shown::Folded(run.len() - 2)], shown(last)].into_iter().flatten().collect::<Vec<_>>()
        }
        _ => run.iter().flat_map(shown).collect::<Vec<_>>(),
    });
    let drawn: Vec<Vec<Line<'static>>> = all.map(|shown| drawn(shown, look)).collect();
    drawn.join(&Line::default())
}

fn is_same(change: &BlockChange) -> bool {
    matches!(change, BlockChange::Same { .. })
}

/// A change as the blocks it shows, in one column or side by side.
fn shown(change: &BlockChange, changed: bool, layout: Layout, look: &Look) -> Vec<Shown> {
    let (old, new) = sides(change, changed, look);
    match layout {
        Layout::Beside { half } => vec![Shown::Beside { old, new, half }],
        Layout::Inline => [old.filter(|_| shows_old_inline(change)), new].into_iter().flatten().map(Shown::Block).collect(),
    }
}

/// The block of each side, none on the side it is missing from; same blocks fade only when something else changed.
fn sides(change: &BlockChange, changed: bool, look: &Look) -> (Option<Marked>, Option<Marked>) {
    let (added, removed, faded) = (Some(look.theme.success), Some(look.theme.danger), Some(look.palette.subtle));
    match change {
        BlockChange::Same { old, new } => {
            let faded = faded.filter(|_| changed);
            (Some(same(old, faded)), Some(same(new, faded)))
        }
        BlockChange::Added(block) => (None, Some(Marked { bar: added, lines: marked(block, None, &[], identity) })),
        BlockChange::Removed(block) => (Some(Marked { bar: removed, lines: marked(block, faded, &[], identity) }), None),
        BlockChange::Changed { old, new, old_words, new_words } => (
            Some(Marked { bar: removed, lines: marked(old, faded, old_words, look.removed_word()) }),
            Some(Marked { bar: added, lines: marked(new, None, new_words, look.added_word()) }),
        ),
    }
}

/// In one column a same block shows once, and a change that only adds words shows its new side alone.
fn shows_old_inline(change: &BlockChange) -> bool {
    match change {
        BlockChange::Same { .. } => false,
        BlockChange::Changed { old_words, new_words, .. } => !old_words.is_empty() || new_words.is_empty(),
        BlockChange::Added(_) | BlockChange::Removed(_) => true,
    }
}

fn same(block: &SourceBlock, faded: Option<Rgb>) -> Marked {
    Marked { bar: None, lines: marked(block, faded, &[], identity) }
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
    match shown {
        Shown::Block(block) => barred(block, look),
        Shown::Beside { old, new, half } => {
            let side = |block: Option<Marked>| block.map(|block| barred(block, look)).unwrap_or_default();
            beside(&side(old), &side(new), half)
        }
        Shown::Folded(count) => {
            vec![Line::styled(
                format!("  ··· {count} unchanged block{}", if count == 1 { "" } else { "s" }),
                Style::default().fg(look.theme.muted),
            )]
        }
    }
}

fn barred(block: Marked, look: &Look) -> Vec<Line<'static>> {
    let text = look.palette.text;
    let bar = Span::styled(if block.bar.is_some() { "▎ " } else { "  " }, Style::default().fg(block.bar.unwrap_or_default()));
    block
        .lines
        .into_iter()
        .map(|line| {
            let spans = line.spans.into_iter().map(|span| Span::styled(span.text, style(span.style, text)));
            Line::from(std::iter::once(bar.clone()).chain(spans).collect::<Vec<_>>())
        })
        .collect()
}

/// The old rows padded to `half` columns and the gap, the new rows after them; the shorter side ends in blank rows.
fn beside(old: &[Line<'static>], new: &[Line<'static>], half: usize) -> Vec<Line<'static>> {
    let row = |lines: &[Line<'static>], i: usize| lines.get(i).cloned().unwrap_or_default();
    (0..old.len().max(new.len()))
        .map(|i| {
            let left = row(old, i);
            let pad = Span::raw(" ".repeat((half + GAP).saturating_sub(left.width())));
            Line::from([left.spans, vec![pad], row(new, i).spans].concat())
        })
        .collect()
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
        let rows = render(old, &new, 40, false, theme()).unfolded;
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
        let rows = render(README, README, 40, false, theme()).folded;
        assert_eq!(
            plain(&rows),
            ["  Widgets", &format!("  {}", "━".repeat(38)), "", "  Acme widgets for nina.", "", "  • one", "", "  • two", "", "  The end."]
        );
        assert_eq!(span(&rows, "Acme widgets for nina.").style.fg, None, "mrk's text colour is the terminal's own");
    }

    #[test]
    fn added_and_removed_blocks_carry_their_bar_and_a_removed_one_fades() {
        let rows =
            rows(&[BlockChange::Added(block("New line.")), BlockChange::Removed(block("Old line."))], false, Layout::Inline, &look());
        assert_eq!(plain(&rows), ["▎ New line.", "", "▎ Old line."]);
        assert_eq!(rows[0].spans[0].style.fg, Some(theme().success));
        assert_eq!(rows[2].spans[0].style.fg, Some(theme().danger));
        assert_eq!(span(&rows, "Old line.").style.fg, Some(theme().faded));
    }

    #[test]
    fn a_changed_block_strikes_its_old_words_and_underlines_its_new_ones() {
        let change =
            BlockChange::Changed { old: block("Pay by card."), new: block("Pay by cash."), old_words: vec![7..11], new_words: vec![7..11] };
        let rows = rows(&[change], false, Layout::Inline, &look());
        assert_eq!(plain(&rows), ["▎ Pay by card.", "", "▎ Pay by cash."]);
        let (old, new) = (span(&rows, "card"), span(&rows, "cash"));
        assert!(old.style.add_modifier.contains(Modifier::CROSSED_OUT) && old.style.fg == Some(theme().danger));
        assert!(new.style.add_modifier.contains(Modifier::UNDERLINED) && new.style.fg == Some(theme().success));
        assert_eq!(span(&rows[2..], "Pay by ").style.fg, None, "kept words of the new side stay plain");
    }

    #[test]
    fn a_change_that_only_adds_words_shows_its_new_side_alone() {
        let change = BlockChange::Changed { old: block("Pay."), new: block("Pay now."), old_words: vec![], new_words: vec![3..7] };
        assert_eq!(plain(&rows(&[change], false, Layout::Inline, &look())), ["▎ Pay now."]);
    }

    #[test]
    fn a_long_unchanged_run_folds_to_its_ends_unless_unfolded() {
        let mut changes: Vec<BlockChange> = ["One.", "Two.", "Three.", "Four.", "Five."].into_iter().map(same).collect();
        changes.push(BlockChange::Added(block("Six.")));
        assert_eq!(
            plain(&rows(&changes, false, Layout::Inline, &look())),
            ["  One.", "", "  ··· 3 unchanged blocks", "", "  Five.", "", "▎ Six."]
        );
        assert_eq!(plain(&rows(&changes, true, Layout::Inline, &look())).len(), 11);
        assert_eq!(
            span(&rows(&changes, true, Layout::Inline, &look()), "Two.").style.fg,
            Some(theme().faded),
            "same blocks fade next to a change"
        );
    }

    #[test]
    fn side_by_side_puts_old_left_and_new_right_and_pads_the_shorter_side() {
        let long = "Pay by card or by bank transfer, whichever comes first.";
        let changes = [
            BlockChange::Removed(block("Old.")),
            BlockChange::Changed { old: block(long), new: block("Pay by cash."), old_words: vec![7..11], new_words: vec![7..11] },
            BlockChange::Added(block("New.")),
        ];
        let rows = rows(&changes, false, Layout::Beside { half: 42 }, &look());
        let right = |text: &str| format!("{}{text}", " ".repeat(43));
        assert_eq!(
            plain(&rows),
            [
                "▎ Old.".to_owned(),
                String::new(),
                format!("{:43}▎ Pay by cash.", "▎ Pay by card or by bank transfer,"),
                "▎ whichever comes first.".to_owned(),
                String::new(),
                right("▎ New."),
            ]
        );
        assert!(span(&rows, "card").style.add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(span(&rows, "cash").style.add_modifier.contains(Modifier::UNDERLINED));
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
