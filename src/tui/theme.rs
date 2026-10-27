//! revu's colours on top of vtui's palette: the fills of a changed line.
//! vtui's tokens and code colours read through `Deref`, so a view writes `theme.accent`, `theme.syntax` and `theme.added_fill` alike.
use ratatui::style::Color;
use std::ops::Deref;
use vtui::theme::mix;

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
}

impl Deref for Theme {
    type Target = vtui::theme::Theme;

    fn deref(&self) -> &Self::Target {
        &self.ui
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
    use vtui::theme::rgb;

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
