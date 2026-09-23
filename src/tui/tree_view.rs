//! The file tree pane: folders and files with the same counts and marks as the diff's file rows.
use super::app::{App, Focus};
use super::theme::Theme;
use super::ui::{pane, settle_scroll, truncate};
use crate::review::tree::TreeRow;
use crate::review::{File, Review};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let Some(open) = &app.open else { return };
    let Some(tree) = &open.tree else { return };
    let rows = open.tree_rows();
    let viewed = open.review.files.iter().filter(|f| open.review.viewed.contains(&f.new_path)).count();
    let title = format!("Files · {viewed}/{} viewed", open.review.files.len());
    let block = pane(theme, &title, app.focus == Focus::Side);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let height = inner.height as usize;
    let scroll = settle_scroll(0, tree.selected, height);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
        .map(|(i, row)| row_line(&open.review, row, i == tree.selected, inner.width as usize, theme))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn row_line<'a>(review: &Review, row: &TreeRow, selected: bool, width: usize, theme: Theme) -> Line<'a> {
    let bar = Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent));
    match row {
        TreeRow::Folder { name, depth, open, files, .. } => {
            let mark = if *open { "▾ " } else { "▸ " };
            let count = if *open { String::new() } else { format!("  {files}") };
            Line::from(vec![
                bar,
                Span::raw("  ".repeat(*depth)),
                Span::styled(mark, Style::default().fg(theme.faded)),
                Span::styled(format!("{name}/"), Style::default().fg(theme.muted)),
                Span::styled(count, Style::default().fg(theme.faded)),
            ])
        }
        TreeRow::File { index, name, depth } => file_line(review, &review.files[*index], name, *depth, bar, width, theme),
    }
}

/// A file: its name, then `+adds −dels`, `◆n` threads and `✓` once viewed, right aligned.
fn file_line<'a>(review: &Review, file: &File, name: &str, depth: usize, bar: Span<'a>, width: usize, theme: Theme) -> Line<'a> {
    let viewed = review.viewed.contains(&file.new_path);
    let threads = review.threads.iter().filter(|t| t.anchor.as_ref().is_some_and(|a| a.path == file.new_path)).count();
    let marks = format!("{}{}", if threads > 0 { format!(" ◆{threads}") } else { String::new() }, if viewed { " ✓" } else { "" });
    let counts = format!("+{} −{}", file.additions, file.deletions);
    let indent = "  ".repeat(depth) + "  ";
    let room = width.saturating_sub(1 + indent.width() + counts.width() + marks.width() + 1);
    let name = truncate(name, room);
    let pad = room.saturating_sub(name.width()) + 1;
    let text = if viewed { Style::default().fg(theme.faded) } else { Style::default().add_modifier(Modifier::BOLD) };
    Line::from(vec![
        bar,
        Span::raw(indent),
        Span::styled(name, text),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("+{}", file.additions), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{}", file.deletions), Style::default().fg(theme.danger)),
        Span::styled(marks, Style::default().fg(if viewed { theme.success } else { theme.warn })),
    ])
}
