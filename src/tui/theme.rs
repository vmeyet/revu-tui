//! revu's colours on top of vtui's palette: the fills of a changed line and the colours of code.
//! vtui's tokens read through `Deref`, so a view writes `theme.accent` and `theme.added_fill` alike.
use crate::syntax::Token;
use ratatui::style::Color;
use std::ops::Deref;
use vtui::theme::{mix, rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Theme {
    ui: vtui::theme::Theme,
    /// Text of an added or removed line. The terminal's own foreground: the fill, or on an unknown
    /// ground the sign alone, carries the meaning, which leaves the text free for syntax colours.
    pub(crate) added: Color,
    pub(crate) removed: Color,
    /// Fill under a changed line, and the stronger one under its changed words.
    /// None where the ground is unknown: a fill guessed wrong paints text into invisibility.
    /// [`Theme::with_ground`] computes them once the terminal says what its ground is.
    pub(crate) added_fill: Option<Color>,
    pub(crate) removed_fill: Option<Color>,
    pub(crate) added_word: Option<Color>,
    pub(crate) removed_word: Option<Color>,
    /// Code colours, from each palette's own upstream scheme; the diff keeps the fills underneath.
    pub(crate) syntax: Syntax,
}

impl Deref for Theme {
    type Target = vtui::theme::Theme;

    fn deref(&self) -> &Self::Target {
        &self.ui
    }
}

/// One colour per [`Token`]: what a keyword, a string or a comment looks like in this palette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Syntax {
    pub(crate) keyword: Color,
    pub(crate) string: Color,
    pub(crate) comment: Color,
    pub(crate) number: Color,
    pub(crate) kind: Color,
    pub(crate) function: Color,
    pub(crate) constant: Color,
    pub(crate) punctuation: Color,
}

impl Syntax {
    pub(crate) fn colour(&self, token: Token) -> Color {
        match token {
            Token::Keyword => self.keyword,
            Token::String => self.string,
            Token::Comment => self.comment,
            Token::Number => self.number,
            Token::Type => self.kind,
            Token::Function => self.function,
            Token::Constant => self.constant,
            Token::Punctuation => self.punctuation,
        }
    }

    /// The code colours of the palette called `name`; the terminal's ANSI colours for the default one.
    fn of_palette(name: &str) -> Syntax {
        match name {
            "dracula" => code([0xff79c6, 0xf1fa8c, 0x6272a4, 0xbd93f9, 0x8be9fd, 0x50fa7b, 0xbd93f9, 0x9098bd]),
            "catppuccin" => code([0xcba6f7, 0xa6e3a1, 0x9399b2, 0xfab387, 0xf9e2af, 0x89b4fa, 0xfab387, 0x9399b2]),
            "catppuccin-latte" => code([0x8839ef, 0x40a02b, 0x7c7f93, 0xfe640b, 0xdf8e1d, 0x1e66f5, 0xfe640b, 0x7c7f93]),
            "rosepine" => code([0x31748f, 0xf6c177, 0x6e6a86, 0xc4a7e7, 0x9ccfd8, 0xebbcba, 0xc4a7e7, 0x908caa]),
            "rosepine-dawn" => code([0x286983, 0xea9d34, 0x9893a5, 0x907aa9, 0x56949f, 0xd7827e, 0x907aa9, 0x797593]),
            "nord" => code([0x81a1c1, 0xa3be8c, 0x616e88, 0xb48ead, 0x8fbcbb, 0x88c0d0, 0xb48ead, 0x81a1c1]),
            "tokyonight" => code([0xbb9af7, 0x9ece6a, 0x565f89, 0xff9e64, 0x2ac3de, 0x7aa2f7, 0xff9e64, 0x89ddff]),
            "monokai" => code([0xf92672, 0xe6db74, 0x75715e, 0xae81ff, 0x66d9ef, 0xa6e22e, 0xae81ff, 0xa59f85]),
            _ => ANSI_SYNTAX,
        }
    }
}

const ANSI_SYNTAX: Syntax = Syntax {
    keyword: Color::Magenta,
    string: Color::Green,
    comment: Color::Indexed(244),
    number: Color::Yellow,
    kind: Color::Cyan,
    function: Color::Blue,
    constant: Color::Yellow,
    punctuation: Color::Indexed(246),
};

/// A palette's code colours from hex values, in [`Syntax`] field order: keyword, string, comment,
/// number, type, function, constant, then punctuation in the palette's muted tone.
const fn code([keyword, string, comment, number, kind, function, constant, punctuation]: [u32; 8]) -> Syntax {
    Syntax {
        keyword: rgb(keyword),
        string: rgb(string),
        comment: rgb(comment),
        number: rgb(number),
        kind: rgb(kind),
        function: rgb(function),
        constant: rgb(constant),
        punctuation: rgb(punctuation),
    }
}

/// Green and red a fill walks toward when the palette's own are ANSI names with no RGB value.
const DIFF_GREEN: u32 = 0x3fb950;
const DIFF_RED: u32 = 0xf85149;
const FILL_PCT: u32 = 10;
const WORD_PCT: u32 = 25;

impl Default for Theme {
    fn default() -> Self {
        Theme::over(vtui::theme::Theme::default())
    }
}

impl Theme {
    pub(super) const NAMES: [&'static str; 9] = vtui::theme::Theme::NAMES;

    /// Case, spaces and dashes do not matter: `Tokyo Night`, `tokyo-night` and `tokyonight` are one theme.
    pub(crate) fn named(name: &str) -> Option<Theme> {
        vtui::theme::Theme::named(name).map(Theme::over)
    }

    /// A palette that does not know its ground takes the terminal's: changed lines get the same
    /// tinted fills the RGB palettes carry. A palette with an RGB ground keeps its own.
    pub(super) fn with_ground(self, ground: u32) -> Theme {
        Theme::over(self.ui.with_ground(ground))
    }

    /// revu's colours over `ui`: fills tinted from its ground toward its success and danger colours, once it
    /// knows its ground.
    fn over(ui: vtui::theme::Theme) -> Theme {
        let fill = |toward: u32, pct: u32| hex(ui.base).map(|ground| mix(ground, toward, pct));
        let (green, red) = (hex(ui.success).unwrap_or(DIFF_GREEN), hex(ui.danger).unwrap_or(DIFF_RED));
        Theme {
            ui,
            added: Color::Reset,
            removed: Color::Reset,
            added_fill: fill(green, FILL_PCT),
            removed_fill: fill(red, FILL_PCT),
            added_word: fill(green, WORD_PCT),
            removed_word: fill(red, WORD_PCT),
            syntax: Syntax::of_palette(ui.name),
        }
    }
}

/// `0xRRGGBB` of an RGB colour; `None` for an ANSI name, whose value only the terminal knows.
fn hex(colour: Color) -> Option<u32> {
    match colour {
        Color::Rgb(r, g, b) => Some((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_themes_fill_changed_lines_and_the_default_theme_waits_for_the_ground() {
        for name in Theme::NAMES {
            let theme = Theme::named(name).unwrap();
            let knows_its_ground = matches!(theme.base, Color::Rgb(..));
            assert_eq!(theme.added_fill.is_some(), knows_its_ground, "{name}");
            assert_eq!(theme.removed_word.is_some(), knows_its_ground, "{name}");
            assert_eq!((theme.added, theme.removed), (Color::Reset, Color::Reset), "{name}: text keeps the terminal colour");
        }
        let dracula = Theme::named("dracula").unwrap();
        assert_ne!(dracula.added_fill, dracula.removed_fill);
        assert_ne!(dracula.added_fill, Some(dracula.base));
    }

    #[test]
    fn a_known_ground_tints_the_default_theme_and_leaves_rgb_themes_alone() {
        let tinted = Theme::default().with_ground(0x1e1e2e);
        assert_eq!(tinted.base, rgb(0x1e1e2e));
        assert_eq!(tinted.added_fill, Some(mix(0x1e1e2e, DIFF_GREEN, FILL_PCT)));
        assert_eq!(tinted.removed_word, Some(mix(0x1e1e2e, DIFF_RED, WORD_PCT)));
        let light = Theme::default().with_ground(0xffffff);
        assert_ne!(light.removed_fill, tinted.removed_fill, "a light terminal gets a light tint");
        let nord = Theme::named("nord").unwrap();
        assert_eq!(nord.with_ground(0xffffff), nord);
    }

    #[test]
    fn every_palette_but_the_default_has_its_own_code_colours() {
        for name in Theme::NAMES {
            let own = Theme::named(name).unwrap().syntax != ANSI_SYNTAX;
            assert_eq!(own, name != "default", "{name}");
        }
    }

    /// Lines of `text`, above its test module, that paint a color by hand instead of asking the theme.
    fn raw_colors(path: &str, text: &str) -> Vec<String> {
        let raw = ["Color::", ".cyan()", ".yellow()", ".green()", ".magenta()", ".blue()", ".red()", ".black()", ".on_yellow()", ".dim()"];
        let lines: Vec<&str> = text.lines().collect();
        let test_module = lines.windows(2).position(|w| w[0].trim() == "#[cfg(test)]" && w[1].contains("mod ") && w[1].ends_with('{'));
        lines[..test_module.unwrap_or(lines.len())]
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.trim_start().starts_with("//") && raw.iter().any(|r| line.contains(r)))
            .map(|(i, line)| format!("{path}:{}: {}", i + 1, line.trim()))
            .collect()
    }

    /// The whole point of a theme: no pane paints a color the theme did not choose.
    #[test]
    fn no_raw_colors_outside_the_theme() {
        // prose.rs hands on mrk's own colours for a rendered Markdown file.
        let allowed = ["theme.rs", "prose.rs"];
        let test_only = |path: &str| path.ends_with("tests.rs") || path.ends_with("test_support.rs");
        let leaks: Vec<String> = crate::sources::rust_files("tui")
            .iter()
            .filter(|file| !allowed.contains(&file.path.as_str()) && !test_only(&file.path))
            .flat_map(|file| raw_colors(&file.path, &file.text))
            .collect();
        assert!(leaks.is_empty(), "raw colors:\n{}", leaks.join("\n"));
    }

    #[test]
    fn a_raw_color_is_caught_outside_comments_and_tests() {
        let text = "let a = Style::new().fg(Color::Red);\n// Color::Red in a comment\nlet b = theme.added;\n#[cfg(test)]\npub(crate) mod tests {\n    Color::Red\n}";
        assert_eq!(raw_colors("new_view.rs", text), ["new_view.rs:1: let a = Style::new().fg(Color::Red);"]);
    }
}
