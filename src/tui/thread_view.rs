//! The right pane: one thread, its notes in order, the body as light markdown.
use super::app::{App, Focus};
use super::theme::Theme;
use super::ui::{pane, short_age};
use crate::api::Note;
use crate::review::{Side, Thread};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let today = app.today;
    let me = app.me.clone();
    let focused = app.focus == Focus::Side;
    let Some(open) = app.open.as_ref() else { return };
    let Some(thread) = open.thread.as_ref().and_then(|id| open.review.thread(id)) else { return };
    let block = pane(theme, &pane_title(thread), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = thread_lines(thread, theme, today, &me);
    for draft in open.review.drafts.iter().filter(|d| d.reply_to.as_deref() == Some(thread.id.as_str())) {
        lines.extend(draft_lines(draft, theme));
    }
    let scroll = open.thread_scroll.min(lines.len().saturating_sub(1)) as u16;
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), inner);
}

fn pane_title(thread: &Thread) -> String {
    match &thread.anchor {
        Some(anchor) => {
            let side = if anchor.side == Side::Old { "-" } else { "" };
            format!("{}:{side}{}", anchor.path.rsplit('/').next().unwrap_or(&anchor.path), anchor.line)
        }
        None => "Thread".to_owned(),
    }
}

fn thread_lines<'a>(thread: &Thread, theme: Theme, today: DateTime<Utc>, me: &str) -> Vec<Line<'a>> {
    let mut lines = vec![];
    if let Some(anchor) = &thread.anchor {
        let state = if thread.outdated {
            "outdated"
        } else if thread.resolved {
            "resolved ✓"
        } else if thread.resolvable {
            "unresolved"
        } else {
            ""
        };
        let colour = if thread.resolved || thread.outdated { theme.faded } else { theme.warn };
        lines.push(Line::from(vec![
            Span::styled(anchor.path.clone(), Style::default().fg(theme.muted)),
            Span::styled(format!("  {state}"), Style::default().fg(colour)),
        ]));
        lines.push(Line::default());
    }
    for note in &thread.notes {
        lines.extend(note_lines(note, theme, today, me));
        lines.push(Line::default());
    }
    lines
}

fn draft_lines<'a>(draft: &crate::review::Draft, theme: Theme) -> Vec<Line<'a>> {
    let state = if draft.id.is_none() { "unsaved" } else { "draft" };
    let mut lines = vec![Line::from(vec![
        Span::styled("◇ you", Style::default().fg(theme.warn).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {state}"), Style::default().fg(theme.muted)),
    ])];
    lines.extend(body_lines(&draft.body, theme));
    lines.push(Line::default());
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
        let thread =
            Thread::from_discussion(crate::api::types::from_fixture(include_str!("../review/fixtures/old_side_note.json"))).unwrap();
        assert_eq!(pane_title(&thread), "charge.rs:-13");
    }
}
