use super::on;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Tui {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme: Option<String>,
    #[serde(default)]
    pub(crate) ascii: bool,
    /// Pictures in comments, drawn in the thread pane where the terminal can; `false` shows their `[image: …]` line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) images: Option<bool>,
    #[serde(default, skip_serializing_if = "QueueLayout::is_default")]
    pub(crate) queue: QueueLayout,
    /// How wide the diff reads in zen, `zz`, in columns; unset, it grows with the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) zen_width: Option<u16>,
    /// Whether the compact layout follows the width, always shows or never does.
    #[serde(default, skip_serializing_if = "ScreenLayout::is_default")]
    pub(crate) layout: ScreenLayout,
    /// Below this many columns the window is compact; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) compact_width: Option<u16>,
    /// Below this many columns the window is narrow; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) narrow_width: Option<u16>,
    /// From this many columns the queue, the diff and the right pane sit side by side; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wide_width: Option<u16>,
    /// The compact layout ends on a row of labels to tap; `false` gives the row back to the page.
    #[serde(default = "on")]
    pub(crate) tap_bar: bool,
}

impl Default for Tui {
    fn default() -> Self {
        Self {
            theme: None,
            ascii: false,
            images: None,
            queue: QueueLayout::default(),
            zen_width: None,
            layout: ScreenLayout::default(),
            compact_width: None,
            narrow_width: None,
            wide_width: None,
            tap_bar: true,
        }
    }
}

/// Narrower than this, a diff line and its gutters no longer fit.
const MIN_ZEN_WIDTH: u16 = 60;

/// `[tui] layout`: the compact layout below `compact_width`, at any width, or never.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ScreenLayout {
    #[default]
    Auto,
    Compact,
    Full,
}

impl ScreenLayout {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The terminal widths the layout turns on. Below `compact` one page shows at a time, full screen.
/// Below `narrow` an open MR hides the queue unless it has the keys, the right pane is a page of
/// its own and the diff shows one line number; from `wide` the queue, the diff and the right pane
/// sit side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Widths {
    pub(crate) compact: u16,
    pub(crate) narrow: u16,
    pub(crate) wide: u16,
}

impl Default for Widths {
    fn default() -> Self {
        Self { compact: 70, narrow: 120, wide: 150 }
    }
}

impl Tui {
    /// `layout` moves the compact limit past every window, or under every one; unset, it never passes the narrow one.
    pub(crate) fn widths(&self) -> Widths {
        let default = Widths::default();
        let narrow = self.narrow_width.unwrap_or(default.narrow);
        let compact = match self.layout {
            ScreenLayout::Auto => self.compact_width.unwrap_or(default.compact.min(narrow)),
            ScreenLayout::Compact => u16::MAX,
            ScreenLayout::Full => 0,
        };
        Widths { compact, narrow, wide: self.wide_width.unwrap_or(default.wide) }
    }

    pub(super) fn check(&self) -> Result<()> {
        if let Some(width) = self.zen_width.filter(|&width| width < MIN_ZEN_WIDTH) {
            bail!("`tui.zen_width = {width}` is too narrow: {MIN_ZEN_WIDTH} is the least");
        }
        let Widths { narrow, wide, .. } = self.widths();
        if self.compact_width == Some(0) {
            bail!("`tui.compact_width = 0` is no width: 1 is the least, and `tui.layout = \"full\"` turns the compact layout off");
        }
        if narrow == 0 {
            bail!("`tui.narrow_width = 0` is no width: 1 is the least");
        }
        if let Some(compact) = self.compact_width.filter(|&compact| compact > narrow) {
            bail!(
                "`tui.compact_width = {compact}` is above `tui.narrow_width = {narrow}`: a compact window ends where a narrow one starts, or before"
            );
        }
        if narrow > wide {
            bail!(
                "`tui.narrow_width = {narrow}` is above `tui.wide_width = {wide}`: a narrow window ends where a wide one starts, or before"
            );
        }
        Ok(())
    }
}

/// How a queue row reads: two lines (the title, then number, author, age and size), or one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum QueueLayout {
    #[default]
    Comfortable,
    Compact,
}

impl QueueLayout {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Tui {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn zen_width_is_unset_by_default_and_refuses_a_column_too_narrow() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        assert_eq!(load("").unwrap().tui.zen_width, None, "the column grows with the screen");
        assert_eq!(load("[tui]\nzen_width = 120\n").unwrap().tui.zen_width, Some(120));
        let narrow = load("[tui]\nzen_width = 40\n").unwrap_err();
        assert!(narrow.contains("zen_width") && narrow.contains("config.toml"), "{narrow}");
    }

    fn widths_of(toml: &str) -> Result<Widths, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).unwrap();
        Config::load_from(&path).map(|config| config.tui.widths()).map_err(|e| format!("{e:#}"))
    }

    #[test]
    fn layout_widths_default_to_120_and_150_and_refuse_a_narrow_above_the_wide() {
        assert_eq!(widths_of("").unwrap(), Widths { compact: 70, narrow: 120, wide: 150 });
        assert_eq!(widths_of("[tui]\nnarrow_width = 100\n").unwrap(), Widths { compact: 70, narrow: 100, wide: 150 });
        assert_eq!(widths_of("[tui]\nwide_width = 180\n").unwrap(), Widths { compact: 70, narrow: 120, wide: 180 });
        assert_eq!(widths_of("[tui]\nnarrow_width = 130\nwide_width = 130\n").unwrap(), Widths { compact: 70, narrow: 130, wide: 130 });
        let zero = widths_of("[tui]\nnarrow_width = 0\n").unwrap_err();
        assert!(zero.contains("narrow_width = 0") && zero.contains("config.toml"), "{zero}");
        let no_wide = widths_of("[tui]\nwide_width = 0\n").unwrap_err();
        assert!(no_wide.contains("wide_width = 0") && no_wide.contains("config.toml"), "{no_wide}");
        let above = widths_of("[tui]\nnarrow_width = 160\n").unwrap_err();
        assert!(above.contains("narrow_width = 160") && above.contains("wide_width = 150"), "{above}");
        let below = widths_of("[tui]\nwide_width = 100\n").unwrap_err();
        assert!(below.contains("narrow_width = 120") && below.contains("wide_width = 100"), "{below}");
    }

    #[test]
    fn the_compact_width_defaults_to_70_and_never_passes_the_narrow_one() {
        assert_eq!(widths_of("[tui]\ncompact_width = 60\n").unwrap().compact, 60);
        assert_eq!(widths_of("[tui]\ncompact_width = 100\nnarrow_width = 100\n").unwrap().compact, 100);
        assert_eq!(widths_of("[tui]\nnarrow_width = 50\n").unwrap().compact, 50, "an unset one follows a narrow width set under it");
        let above = widths_of("[tui]\ncompact_width = 130\n").unwrap_err();
        assert!(above.contains("compact_width = 130") && above.contains("narrow_width = 120") && above.contains("config.toml"), "{above}");
        let zero = widths_of("[tui]\ncompact_width = 0\n").unwrap_err();
        assert!(zero.contains("compact_width = 0") && zero.contains("layout = \"full\""), "{zero}");
    }

    #[test]
    fn the_layout_setting_forces_the_compact_layout_or_turns_it_off() {
        assert_eq!(widths_of("[tui]\nlayout = \"auto\"\n").unwrap().compact, 70);
        assert_eq!(widths_of("[tui]\nlayout = \"compact\"\n").unwrap().compact, u16::MAX, "every window is under it");
        assert_eq!(widths_of("[tui]\nlayout = \"full\"\ncompact_width = 60\n").unwrap().compact, 0, "no window is under it");
        assert!(widths_of("[tui]\nlayout = \"phone\"\n").unwrap_err().contains("layout"));
    }

    #[test]
    fn the_tap_bar_is_on_unless_switched_off() {
        assert!(toml::from_str::<Config>("").unwrap().tui.tap_bar);
        assert!(toml::from_str::<Config>("[tui]\ntheme = \"nord\"\n").unwrap().tui.tap_bar, "another key leaves it on");
        assert!(!toml::from_str::<Config>("[tui]\ntap_bar = false\n").unwrap().tui.tap_bar);
        assert!(!toml::to_string_pretty(&Config::default()).unwrap().contains("[tui]"), "the default is not written");
    }
}
