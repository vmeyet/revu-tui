//! `ctrl-k`: one fuzzy list over the queue's MRs and the open MR's files.
use super::theme::Theme;
use super::ui::{pane, truncate};
use crate::forge::MrKey;
use crate::fuzzy;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Mr(MrKey),
    /// A file of the open MR, by its index in the review.
    File(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub target: Target,
}

pub const MAX_SHOWN: usize = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Jump {
    pub query: String,
    pub selected: usize,
    /// Files first when an MR is open: jumping inside it is the common case.
    pub candidates: Vec<Candidate>,
}

impl Jump {
    pub fn new(candidates: Vec<Candidate>) -> Self {
        Self { candidates, ..Self::default() }
    }

    /// Best matches for the query; everything in order on an empty one.
    pub fn matches(&self) -> Vec<Candidate> {
        if self.query.is_empty() {
            return self.candidates.iter().take(MAX_SHOWN).cloned().collect();
        }
        fuzzy::rank(&self.query, self.candidates.iter().map(|c| (c.label.clone(), c.clone())))
            .into_iter()
            .map(|(_, c)| c)
            .take(MAX_SHOWN)
            .collect()
    }

    pub fn chosen(&self) -> Option<Candidate> {
        self.matches().into_iter().nth(self.selected)
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.matches().len();
        self.selected = if len == 0 { 0 } else { self.selected.saturating_add_signed(delta).min(len - 1) };
    }

    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }
}

pub fn draw(f: &mut Frame, jump: &Jump, area: Rect, theme: Theme) {
    let width = (area.width * 3 / 5).clamp(30.min(area.width), area.width);
    let height = (MAX_SHOWN as u16 + 3).min(area.height);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 3, width, height };
    let block = pane(theme, "jump", true);
    let inner = block.inner(popup);
    let room = inner.width.saturating_sub(4) as usize;
    let prompt = Line::from(vec![
        Span::styled("› ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::raw(jump.query.clone()),
        Span::styled("▌", Style::default().fg(theme.accent)),
    ]);
    let matches = jump.matches();
    let mut lines = vec![prompt];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled("  no match", Style::default().fg(theme.muted))));
    }
    lines.extend(matches.iter().enumerate().map(|(i, c)| {
        let selected = i == jump.selected;
        let (icon, colour) = match c.target {
            Target::Mr(_) => ("◆", theme.accent),
            Target::File(_) => ("·", theme.muted),
        };
        let text = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
        Line::from(vec![
            Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent)),
            Span::styled(format!("{icon} "), Style::default().fg(colour)),
            Span::styled(truncate(&c.label, room), text),
        ])
    }));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)), popup);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn jump() -> Jump {
        let key = |n| MrKey { project: "acme/widgets".into(), number: n };
        Jump::new(vec![
            Candidate { label: "src/pay/charge.rs".into(), target: Target::File(0) },
            Candidate { label: "Cargo.lock".into(), target: Target::File(1) },
            Candidate { label: "!42 charge cards at checkout".into(), target: Target::Mr(key(42)) },
        ])
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        let labels: Vec<String> = jump().matches().into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["src/pay/charge.rs", "Cargo.lock", "!42 charge cards at checkout"]);
    }

    #[test]
    fn a_query_ranks_and_the_selection_follows() {
        let mut j = jump();
        "chk".chars().for_each(|c| j.type_char(c));
        assert!(matches!(j.chosen().unwrap().target, Target::Mr(_)));
        j.move_by(5);
        assert_eq!(j.selected, j.matches().len() - 1);
        j.backspace();
        assert_eq!(j.selected, 0);
    }
}
