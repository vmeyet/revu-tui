//! How the screen is laid out: zen, the size class the last frame fit in, the line wrap, and the `[tui]` keys that
//! decide them.
use super::Settings;
use crate::config::{QueueLayout, Widths};
use ratatui::layout::Size;
use vtui::motion::Motion;

/// Under this many rows the compact layout is short: the status row and the MR header give their rows to the page.
const SHORT_BELOW: u16 = 20;

#[derive(Clone, Debug)]
pub(crate) struct Layout {
    /// Zen, `zz`: the diff alone in a centred column, no frames, no status line, nothing pulsing.
    pub(crate) zen: bool,
    /// Long diff lines wrap under their text instead of ending in `…`, once `w` chose; until then the layout does.
    wrap: Option<bool>,
    /// What the last frame's size left room for.
    fit: Fit,
    /// `[tui] tap_bar`: the compact layout ends on a row of labels to tap.
    pub(super) tap_bar: bool,
    /// How wide the diff reads in zen, `[tui] zen_width`; `None` grows it with the screen.
    pub(crate) zen_width: Option<u16>,
    /// The terminal widths the layout turns on, `[tui] narrow_width` and `wide_width`.
    pub(crate) widths: Widths,
    /// `[tui] queue`: two-line rows, or one.
    pub(super) queue_layout: QueueLayout,
}

/// What a frame's size leaves room for, each one smaller than the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// The right pane fits beside the diff.
    Full,
    /// Too narrow for the right pane beside the diff.
    Narrow,
    /// The compact layout: one page at a time, full screen.
    Compact,
    /// Compact, and too short for the rows around the page.
    Short,
}

impl Layout {
    /// Out of zen and as wide as the screen can be, until the first frame measures it.
    pub(super) fn new(settings: &Settings) -> Self {
        let Settings { tap_bar, zen_width, widths, queue_layout, .. } = *settings;
        Self { zen: false, wrap: None, fit: Fit::Full, tap_bar, zen_width, widths, queue_layout }
    }

    /// Each frame measures the screen again, so a window that grows or shrinks changes layout without a restart.
    pub(crate) fn fit(&mut self, screen: Size) {
        self.fit = match screen {
            Size { width, height } if width < self.widths.compact && height < SHORT_BELOW => Fit::Short,
            Size { width, .. } if width < self.widths.compact => Fit::Compact,
            Size { width, .. } if width < self.widths.narrow => Fit::Narrow,
            Size { .. } => Fit::Full,
        };
    }

    /// The last frame showed the compact layout.
    pub(crate) fn is_compact(&self) -> bool {
        matches!(self.fit, Fit::Compact | Fit::Short)
    }

    /// The last frame was too narrow for the right pane beside the diff; compact is narrow too.
    pub(crate) fn is_narrow(&self) -> bool {
        self.fit != Fit::Full
    }

    /// The last frame was compact and too short for the rows around the page.
    pub(crate) fn is_short(&self) -> bool {
        self.fit == Fit::Short
    }

    /// The screen moves on its own as little as a phone link likes in the compact layout, freely anywhere else.
    pub(crate) fn motion(&self) -> Motion {
        if self.is_compact() { Motion::Calm } else { Motion::Lively }
    }

    /// Long diff lines wrap: what `w` chose, else on in the compact layout, where a cut line hides most of itself.
    pub(crate) fn wraps(&self) -> bool {
        self.wrap.unwrap_or(self.is_compact())
    }

    /// `w`: long lines the other way from now on, in every layout; says whether they now wrap.
    pub(super) fn toggle_wrap(&mut self) -> bool {
        let wrap = !self.wraps();
        self.wrap = Some(wrap);
        wrap
    }

    /// Threads read under their lines in zen and in a window too narrow for the pane beside the diff.
    pub(super) fn threads_in_diff(&self) -> bool {
        self.zen || self.is_narrow()
    }

    /// Two-line queue rows in the compact layout whatever `[tui] queue` says, one line leaving a title no room there.
    pub(super) fn row_layout(&self) -> QueueLayout {
        if self.is_compact() { QueueLayout::Comfortable } else { self.queue_layout }
    }

    /// The compact layout ends on its tap bar, unless `[tui] tap_bar` hides it.
    pub(super) fn has_tap_bar(&self) -> bool {
        self.is_compact() && self.tap_bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::settings;

    fn fitted(width: u16, height: u16) -> Layout {
        let mut layout = Layout::new(&settings());
        layout.fit(Size { width, height });
        layout
    }

    #[test]
    fn each_size_class_holds_the_ones_after_it() {
        let short = fitted(50, 14);
        assert!(short.is_short() && short.is_compact() && short.is_narrow());
        let compact = fitted(50, 40);
        assert!(!compact.is_short() && compact.is_compact() && compact.is_narrow());
        let narrow = fitted(100, 14);
        assert!(!narrow.is_short() && !narrow.is_compact() && narrow.is_narrow());
        assert!(!fitted(130, 40).is_narrow());
    }

    #[test]
    fn what_w_chose_wraps_in_every_layout() {
        let mut layout = fitted(50, 40);
        assert!(layout.wraps(), "the compact layout wraps until w");
        assert!(!layout.toggle_wrap());
        layout.fit(Size { width: 130, height: 40 });
        assert!(layout.toggle_wrap() && layout.wraps());
    }
}
