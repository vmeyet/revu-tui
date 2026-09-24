//! The `?` overlay: every key, grouped by task in the order a review goes, in one or two columns.
use super::app::App;
use super::ui::{pane, truncate};
use crate::keymap::Keymap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

/// One task and its keys; a key that only works in one pane says which in its words.
pub struct Group {
    pub title: &'static str,
    pub keys: &'static [(&'static str, &'static str)],
}

pub const GROUPS: [Group; 7] = [
    Group {
        title: "move",
        keys: &[
            ("j k", "move"),
            ("g G", "first, last"),
            ("^d ^u", "half a page"),
            ("h l", "pane to the left, to the right"),
            ("enter", "open, or toggle the fold"),
            ("esc x", "close the pane, back to the queue"),
            ("tab S-tab", "next, previous file"),
            ("]c [c", "next, previous hunk"),
            ("]n [n", "next, previous conversation"),
            ("]f [f", "next, previous file with a thread"),
            ("]r [r", "next, previous MR that needs you"),
        ],
    },
    Group {
        title: "queue",
        keys: &[
            ("/", "filter: words @author !42 ~label"),
            ("' 1-9", "a saved view, by letter or rank"),
            ("*", "this repo, or every project"),
            ("s", "next sort order"),
            ("S", "group by author"),
            ("zo zc", "open, fold the section or stack"),
        ],
    },
    Group {
        title: "view",
        keys: &[
            ("i", "the MR cover page"),
            ("D", "changed words inline, or split"),
            ("t", "file tree"),
            ("p", "pipeline"),
            ("zz", "zen: the diff alone, quiet"),
            ("← →", "in zen: previous, next MR"),
            ("w W", "wrap long lines, hide whitespace changes"),
            ("+", "more lines around the hunk"),
            ("v", "the file in your own program"),
            ("za zc zo", "toggle, close, open the fold"),
            ("zM zR", "fold, unfold every file"),
            ("zh", "fold the MR header"),
            ("zv", "file viewed: it folds away"),
        ],
    },
    Group {
        title: "comment & publish",
        keys: &[
            ("c", "comment on the line"),
            ("C", "comment on the old side"),
            ("V", "select lines"),
            ("s", "suggest a change"),
            ("E", "write in $EDITOR"),
            ("⌥enter ^o", "in the box: new line, $EDITOR"),
            ("esc", "drop the selection, leave the box"),
            ("P", "publish every draft"),
            ("A M", "approve or unapprove, merge mine"),
            ("Y", "share the MR, after a preview"),
        ],
    },
    Group {
        title: "thread pane",
        keys: &[
            ("r", "reply, as a draft"),
            ("R", "resolve, unresolve"),
            ("J K", "next, previous thread on the line"),
            ("e d", "edit, delete my draft"),
            ("S", "apply the suggestion, after a y"),
            ("u", "open the first link"),
        ],
    },
    Group {
        title: "ask claude",
        keys: &[
            ("a e r s", "explain, risks, summary"),
            ("a t c a", "thread, comment, anything"),
            ("c ⏎ R y", "answer: draft, follow up, again, copy"),
        ],
    },
    Group {
        title: "search & app",
        keys: &[
            ("^k", "MRs · / files · > commands"),
            (":", "commands: :go !42, :set theme=nord"),
            ("o y", "open in the browser, copy the link"),
            ("r", "refresh"),
            ("?", "this help"),
            ("q ^c", "quit"),
        ],
    },
];

/// Padding inside the frame: two columns on each side, one row above and below.
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;
/// Space between the two columns.
const GUTTER: usize = 6;
/// Space between a key and what it does.
const KEY_GAP: usize = 2;
/// From this overlay width the groups share two columns.
const TWO_COLUMNS_FROM: u16 = 100;

/// The last row the list can scroll to: the whole list in one column, so scrolling never stops short.
pub fn last_row() -> usize {
    column_height(&GROUPS.iter().map(|g| g.keys.len()).collect::<Vec<_>>()).saturating_sub(1)
}

/// Rows a column of groups takes: a header per group, its keys, one blank row between groups.
fn column_height(sizes: &[usize]) -> usize {
    sizes.iter().map(|n| n + 1).sum::<usize>() + sizes.len().saturating_sub(1)
}

/// How many groups go in the left column so both columns end as close as possible; order is kept.
pub fn split(sizes: &[usize]) -> usize {
    (1..sizes.len()).min_by_key(|&at| column_height(&sizes[..at]).max(column_height(&sizes[at..]))).unwrap_or(sizes.len())
}

/// The groups with their keys as they are in effect: the user's bindings, `(`/`)` on AZERTY.
fn shown(keymap: &Keymap) -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    GROUPS.iter().map(|g| (g.title, g.keys.iter().map(|(keys, what)| (keymap.label(keys), *what)).collect())).collect()
}

/// A column of groups as lines: faded uppercase headers, keys right-aligned in the accent.
fn column_lines<'a>(groups: &[(&str, Vec<(String, &str)>)], width: usize, app: &App) -> Vec<Line<'a>> {
    let theme = app.theme;
    let key_w = groups.iter().flat_map(|(_, keys)| keys.iter().map(|(k, _)| k.width())).max().unwrap_or(0);
    let what_w = width.saturating_sub(key_w + KEY_GAP);
    let header = Style::default().fg(theme.faded).add_modifier(Modifier::BOLD);
    let key = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);
    let mut lines = vec![];
    for (i, (title, keys)) in groups.iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(title.to_uppercase(), header)));
        for (keys, what) in keys {
            lines.push(Line::from(vec![
                Span::styled(format!("{keys:>key_w$}"), key),
                Span::raw(" ".repeat(KEY_GAP)),
                Span::raw(truncate(what, what_w)),
            ]));
        }
    }
    lines
}

/// The widest line a column needs: its widest key, the gap, its widest description.
fn natural_width(groups: &[(&str, Vec<(String, &str)>)]) -> usize {
    let key_w = groups.iter().flat_map(|(_, keys)| keys.iter().map(|(k, _)| k.width())).max().unwrap_or(0);
    let what_w = groups.iter().flat_map(|(_, keys)| keys.iter().map(|(_, w)| w.width())).max().unwrap_or(0);
    let title_w = groups.iter().map(|(t, _)| t.width()).max().unwrap_or(0);
    (key_w + KEY_GAP + what_w).max(title_w)
}

/// The overlay, centred, at most 90 % of the screen each way; it scrolls by `scroll` rows when taller than that.
pub fn draw(f: &mut Frame, app: &App, area: Rect, scroll: usize) {
    let groups = shown(&app.keymap);
    let sizes: Vec<usize> = groups.iter().map(|(_, keys)| keys.len()).collect();
    let max_w = area.width * 9 / 10;
    let two = max_w >= TWO_COLUMNS_FROM;
    let at = if two { split(&sizes) } else { groups.len() };
    let (left, right) = groups.split_at(at);
    let frame_x = usize::from(2 + 2 * PAD_X);
    let natural = if two { natural_width(left) + GUTTER + natural_width(right) } else { natural_width(left) };
    let width = ((natural + frame_x) as u16).min(max_w).max(area.width.min(40));
    let inner_w = usize::from(width).saturating_sub(frame_x);
    let (left_w, right_w) = if two {
        let share = inner_w.saturating_sub(GUTTER);
        let left_w = share * natural_width(left) / (natural_width(left) + natural_width(right)).max(1);
        (left_w, share - left_w)
    } else {
        (inner_w, 0)
    };
    let left_lines = column_lines(left, left_w, app);
    let right_lines = column_lines(right, right_w, app);
    let rows = left_lines.len().max(right_lines.len());
    let frame_y = usize::from(2 + 2 * PAD_Y);
    let height = ((rows + frame_y) as u16).min(area.height * 9 / 10).max(area.height.min(8));
    let visible = usize::from(height).saturating_sub(frame_y);
    let top = scroll.min(rows.saturating_sub(visible));
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    let hint = if rows > visible {
        format!(" {}–{} of {} · j k scroll · esc close ", top + 1, (top + visible).min(rows), rows)
    } else {
        " esc close ".to_owned()
    };
    let block = pane(app.theme, "keys", true)
        .title_bottom(Line::from(Span::styled(hint, Style::default().fg(app.theme.faded))).right_aligned())
        .padding(Padding::new(PAD_X, PAD_X, PAD_Y, PAD_Y));
    let inner = block.inner(popup);
    f.render_widget(Clear, popup);
    f.render_widget(block.style(Style::default().bg(app.theme.surface)), popup);
    let scroll_to = (top as u16, 0);
    let left_area = Rect { width: left_w as u16, ..inner };
    f.render_widget(Paragraph::new(left_lines).scroll(scroll_to), left_area);
    if two {
        let right_area = Rect { x: inner.x + (left_w + GUTTER) as u16, width: right_w as u16, ..inner };
        f.render_widget(Paragraph::new(right_lines).scroll(scroll_to), right_area);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::collections::HashSet;

    /// A help word as the key map writes a default: `S-tab` is `backtab`, `^k` is `ctrl-k`.
    fn as_default(word: &str) -> String {
        match word {
            "S-tab" => "backtab".to_owned(),
            "←" => "left".to_owned(),
            "→" => "right".to_owned(),
            _ => word.strip_prefix('^').map_or_else(|| word.to_owned(), |rest| format!("ctrl-{rest}")),
        }
    }

    #[test]
    fn every_bindable_action_has_its_key_in_a_group() {
        let words: HashSet<String> =
            GROUPS.iter().flat_map(|g| g.keys.iter()).flat_map(|(keys, _)| keys.split(' ')).map(as_default).collect();
        let missing: Vec<&str> = crate::keymap::ACTIONS.iter().filter(|(_, key)| !words.contains(*key)).map(|(name, _)| *name).collect();
        assert!(missing.is_empty(), "actions with no line in the help: {missing:?}");
    }

    #[test]
    fn a_key_says_one_thing_per_group() {
        for group in &GROUPS {
            let mut seen = HashSet::new();
            for (keys, _) in group.keys {
                assert!(seen.insert(*keys), "{keys} twice in {}", group.title);
            }
        }
    }

    #[test]
    fn two_columns_end_as_close_as_possible_without_reordering() {
        assert_eq!(split(&[3, 3]), 1);
        assert_eq!(split(&[20, 1, 1, 1]), 1, "one tall group stays alone on the left");
        assert_eq!(split(&[1, 1, 1, 1, 10]), 4);
        let sizes: Vec<usize> = GROUPS.iter().map(|g| g.keys.len()).collect();
        let at = split(&sizes);
        let (left, right) = (column_height(&sizes[..at]), column_height(&sizes[at..]));
        assert!(left.abs_diff(right) <= 8, "left {left} rows, right {right}");
    }

    #[test]
    fn a_column_counts_headers_and_the_blank_rows_between_groups() {
        assert_eq!(column_height(&[2, 3]), 3 + 4 + 1);
        assert_eq!(column_height(&[]), 0);
        assert_eq!(last_row() + 1, column_height(&GROUPS.iter().map(|g| g.keys.len()).collect::<Vec<_>>()));
    }
}
