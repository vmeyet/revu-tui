//! The right pane: every conversation of one place, notes in order, bodies as light markdown.
use super::app::{App, Entry, EntryKind, Focus, Open};
use super::theme::Theme;
use super::ui::{pane, short_age};
use crate::forge::Note;
use crate::review::{Conversation, Place, Review, Thread};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let today = app.today;
    let me = app.me.clone();
    let focused = app.focus == Focus::Side;
    let Some(open) = app.open.as_mut() else { return };
    let Some((conversations, entries, current)) = open.pane_view() else { return };
    let Some(pane) = open.pane.clone() else { return };
    let here = open.row().and_then(|row| open.review.place_of(row)).is_some_and(|place| place == pane.place);
    let block = pane_block(theme, &title(&open.review, &pane.place, conversations.len(), here), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width.saturating_sub(1) as usize;
    let mut lines: Vec<(Option<Entry>, Line<'static>)> = vec![];
    for (index, conversation) in conversations.iter().enumerate() {
        if index > 0 {
            lines.push((None, Line::from(Span::styled("─".repeat(width), Style::default().fg(theme.border)))));
        }
        lines.extend(conversation_lines(open, conversation, index, &entries, theme, today, &me));
    }
    let more = open.review.others_in_file(&pane.place);
    if more > 0 {
        lines.push((None, Line::default()));
        let footer = format!("{more} more thread{} in this file · ]n", if more == 1 { "" } else { "s" });
        lines.push((None, Line::from(Span::styled(footer, Style::default().fg(theme.faded)))));
    }
    let rows: Vec<(bool, Line<'static>)> = lines
        .into_iter()
        .flat_map(|(entry, line)| wrap(line, width).into_iter().map(move |l| (entry.is_some() && entry == current, l)))
        .collect();
    let height = inner.height as usize;
    let first = rows.iter().position(|(on, _)| *on).unwrap_or(0);
    let last = rows.iter().rposition(|(on, _)| *on).unwrap_or(0);
    let scroll = settle(pane.scroll, first, last, height);
    if let Some(pane) = open.pane.as_mut() {
        pane.scroll = scroll;
    }
    let drawn: Vec<Line> = rows
        .into_iter()
        .skip(scroll)
        .take(height)
        .map(|(on, line)| {
            let bar = Span::styled(if on { "▎" } else { " " }, Style::default().fg(theme.accent));
            Line::from(std::iter::once(bar).chain(line.spans).collect::<Vec<_>>())
        })
        .collect();
    f.render_widget(Paragraph::new(drawn), inner);
}

/// The pane's frame; its title fades when the reader looks at another line.
fn pane_block(theme: Theme, title: &str, focused: bool) -> ratatui::widgets::Block<'static> {
    pane(theme, title, focused)
}

/// Keeps the cursor's lines in view: the top of the entry first, then as much of it as fits.
fn settle(scroll: usize, first: usize, last: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if first < scroll {
        return first;
    }
    if last >= scroll + height {
        return (last + 1).saturating_sub(height).min(first);
    }
    scroll
}

/// `charge.rs:57 · 2 threads`, `charge.rs:-13` for the old side, `on the MR`, `charge.rs · outdated`;
/// `↑ line 57` when the cursor moved to a line without conversations.
fn title(review: &Review, place: &Place, count: usize, here: bool) -> String {
    let name = |file: usize| {
        let path = &review.files[file].new_path;
        path.rsplit('/').next().unwrap_or(path).to_owned()
    };
    let threads = format!("{count} thread{}", if count == 1 { "" } else { "s" });
    let (head, line) = match place {
        Place::Line { file, new: Some(n), .. } => (format!("{}:{n}", name(*file)), Some(n.to_string())),
        Place::Line { file, old: Some(o), .. } => (format!("{}:-{o}", name(*file)), Some(format!("-{o}"))),
        Place::Line { file, .. } => (name(*file), None),
        Place::Mr => ("on the MR".to_owned(), None),
        Place::Outdated { file } => (format!("{} · outdated", name(*file)), None),
    };
    match (here, line) {
        (false, Some(line)) => format!("{head} · {threads} · ↑ line {line}"),
        _ => format!("{head} · {threads}"),
    }
}

/// A thread, or my new draft, as the pane shows it: status, notes, my replies; each line tagged
/// with the cursor stop it belongs to.
fn conversation_lines(
    open: &Open,
    conversation: &Conversation,
    index: usize,
    entries: &[Entry],
    theme: Theme,
    today: DateTime<Utc>,
    me: &str,
) -> Vec<(Option<Entry>, Line<'static>)> {
    let stop = |kind: EntryKind| entries.iter().find(|e| e.conversation == index && e.kind == kind).copied();
    let mut lines = vec![];
    if let Some(thread) = conversation.thread.as_deref().and_then(|id| open.review.thread(id)) {
        let shown = entries.iter().filter(|e| e.conversation == index && matches!(e.kind, EntryKind::Note(_))).count();
        lines.push((stop(EntryKind::Note(0)), status(thread, shown, theme)));
        for (n, note) in thread.notes.iter().take(shown).enumerate() {
            let entry = stop(EntryKind::Note(n));
            lines.extend(note_lines(note, theme, today, me).into_iter().map(|line| (entry, line)));
        }
    }
    for &draft in &conversation.drafts {
        let entry = stop(EntryKind::Draft(draft));
        let draft = &open.review.drafts[draft];
        if conversation.thread.is_none() {
            lines.push((entry, Line::from(Span::styled("◇ draft", Style::default().fg(theme.accent)))));
        }
        lines.extend(draft_lines(draft, theme).into_iter().map(|line| (entry, line)));
    }
    lines
}

fn status(thread: &Thread, shown: usize, theme: Theme) -> Line<'static> {
    let (text, colour) = if thread.outdated {
        ("◆ outdated".to_owned(), theme.faded)
    } else if thread.resolved && shown < thread.notes.len() {
        let hidden = thread.notes.len() - shown;
        (format!("✓ resolved · {hidden} more note{} · enter unfolds", if hidden == 1 { "" } else { "s" }), theme.faded)
    } else if thread.resolved {
        ("✓ resolved".to_owned(), theme.faded)
    } else if thread.resolvable {
        ("◆ unresolved".to_owned(), theme.warn)
    } else {
        ("· comment".to_owned(), theme.muted)
    };
    Line::from(Span::styled(text, Style::default().fg(colour)))
}

/// A line cut at word boundaries to `width`, styles kept; a word longer than the width is cut.
fn wrap(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let mut rows: Vec<Vec<Span<'static>>> = vec![vec![]];
    let mut used = 0;
    for span in line.spans {
        for word in span.content.split_inclusive(' ') {
            let w = word.width();
            if used + w > width && used > 0 {
                rows.push(vec![]);
                used = 0;
            }
            let mut rest = word;
            while rest.width() > width {
                let cut = rest.char_indices().scan(0, |acc, (i, c)| {
                    *acc += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    Some((i, *acc))
                });
                let at = cut
                    .take_while(|&(_, acc)| acc <= width)
                    .last()
                    .map_or(rest.len(), |(i, _)| i + rest[i..].chars().next().map_or(0, char::len_utf8));
                if let Some(row) = rows.last_mut() {
                    row.push(Span::styled(rest[..at].to_owned(), span.style));
                }
                rows.push(vec![]);
                rest = &rest[at..];
            }
            if let Some(row) = rows.last_mut() {
                row.push(Span::styled(rest.to_owned(), span.style));
            }
            used += rest.width();
        }
    }
    rows.into_iter().map(Line::from).collect()
}

/// My draft: `you · draft ◇`, `unsaved` in danger until the forge holds it.
fn draft_lines<'a>(draft: &crate::review::Draft, theme: Theme) -> Vec<Line<'a>> {
    let (state, colour) = if draft.id.is_none() { ("unsaved", theme.danger) } else { ("draft", theme.muted) };
    let mut lines = vec![Line::from(vec![
        Span::styled("you", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {state} ◇"), Style::default().fg(colour)),
    ])];
    lines.extend(body_lines(&draft.body, theme));
    lines
}

fn note_lines<'a>(note: &Note, theme: Theme, today: DateTime<Utc>, me: &str) -> Vec<Line<'a>> {
    let author = if note.author.username == me { "you".to_owned() } else { note.author.username.clone() };
    let age = short_age((today - note.created_at).to_std().unwrap_or_default());
    let mut lines = vec![Line::from(vec![
        Span::styled(author.clone(), Style::default().fg(theme.user(&author)).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {age}"), Style::default().fg(theme.muted)),
    ])];
    lines.extend(body_lines(&note.body, theme));
    lines
}

/// Code spans, bullets and quotes; the rest is the text as written, wrapped by the widget.
pub fn body_lines<'a>(body: &str, theme: Theme) -> Vec<Line<'a>> {
    let mut in_fence = false;
    let mut lines = vec![];
    for raw in body.lines() {
        if raw.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            lines.push(Line::from(Span::styled(format!("  {raw}"), Style::default().fg(theme.code))));
            continue;
        }
        let line = match raw.trim_start() {
            rest if rest.starts_with("- ") || rest.starts_with("* ") => Line::from(inline(&format!("• {}", &rest[2..]), theme)),
            rest if rest.starts_with("> ") => Line::from(Span::styled(format!("▏{}", &rest[2..]), Style::default().fg(theme.muted))),
            _ => Line::from(inline(raw, theme)),
        };
        lines.push(line);
    }
    lines
}

fn inline<'a>(text: &str, theme: Theme) -> Vec<Span<'a>> {
    text.split('`')
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(i, part)| {
            if i % 2 == 1 {
                Span::styled(part.to_owned(), Style::default().fg(theme.code).bg(theme.surface))
            } else {
                Span::raw(part.to_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect()
    }

    #[test]
    fn bodies_keep_code_bullets_and_quotes() {
        let body = "Use `Key::from` here.\n- one\n> said\n```\nlet x = 1;\n```";
        let lines = text(&body_lines(body, Theme::default()));
        assert_eq!(lines, ["Use Key::from here.", "• one", "▏said", "  let x = 1;"]);
    }

    #[test]
    fn the_title_names_the_line_and_marks_the_old_side() {
        let review = crate::review::tests::review();
        let old = Place::Line { file: 0, new: None, old: Some(13) };
        assert_eq!(title(&review, &old, 1, true), "charge.rs:-13 · 1 thread");
        assert_eq!(title(&review, &old, 2, false), "charge.rs:-13 · 2 threads · ↑ line -13");
        assert_eq!(title(&review, &Place::Mr, 1, true), "on the MR · 1 thread");
    }

    #[test]
    fn long_lines_wrap_at_words_and_long_words_are_cut() {
        let rows = wrap(Line::from("one two three"), 8);
        assert_eq!(text(&rows), ["one two ", "three"]);
        assert_eq!(text(&wrap(Line::from("abcdefghij"), 4)), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn the_cursor_entry_stays_in_view() {
        assert_eq!(settle(0, 2, 3, 10), 0);
        assert_eq!(settle(0, 12, 14, 10), 5);
        assert_eq!(settle(8, 3, 4, 10), 3);
        assert_eq!(settle(0, 2, 30, 10), 2, "a long entry shows from its top");
    }
}
