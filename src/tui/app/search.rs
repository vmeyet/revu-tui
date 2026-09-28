//! `/` in the diff: find text anywhere in the MR, in folded files and hunks too, and `n` `N` to
//! walk the matches; a match inside a fold opens it, as vim's search does.
use super::{App, Open};
use crate::review::{Review, Row};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// What `/` looks for; `typing` while the line at the bottom takes the keys, `from` the row the
/// search started on, where `esc` puts the cursor back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub typing: bool,
    from: usize,
}

/// Where a match sits: a file's path, or one line of one of its hunks. Ordered as the diff reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    file: usize,
    hunk: Option<usize>,
    line: Option<usize>,
}

impl App {
    /// `/`: the search line opens, empty, where the cursor is.
    pub(super) fn start_search(&mut self) {
        let from = self.open.as_ref().map_or(0, |o| o.selected);
        self.search = Some(Search { query: String::new(), typing: true, from });
    }

    pub(super) fn handle_search_key(&mut self, key: KeyEvent) {
        let Some(search) = self.search.clone() else { return };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.cancel_search(&search),
            KeyCode::Enter if search.query.is_empty() => self.search = None,
            KeyCode::Enter => {
                self.search = Some(Search { typing: false, ..search });
                if self.matches().is_empty() {
                    self.toast("no match");
                }
            }
            KeyCode::Backspace if search.query.is_empty() => self.cancel_search(&search),
            KeyCode::Backspace => self.retype(&search, search.query[..search.query.len() - last_len(&search.query)].to_owned()),
            KeyCode::Char(c) if !ctrl && !c.is_control() => self.retype(&search, format!("{}{c}", search.query)),
            _ => {}
        }
    }

    /// `n` and `N`: the next or previous match, around the end of the MR back to its start.
    pub(super) fn search_step(&mut self, forward: bool) {
        let Some(open) = &self.open else { return };
        let hits = self.matches();
        let here = hit_at(open);
        let after = |hit: &&Hit| if forward { Some(**hit) > here } else { Some(**hit) < here };
        let next = if forward { hits.iter().find(after) } else { hits.iter().rev().find(after) };
        let wrapped = if forward { hits.first() } else { hits.last() };
        match (next, wrapped) {
            (Some(hit), _) => self.reveal(*hit),
            (None, Some(hit)) => {
                self.reveal(*hit);
                self.toast(if forward { "search went on from the top" } else { "search went on from the bottom" });
            }
            (None, None) => self.toast("no match"),
        }
    }

    /// `esc` with a search kept: its marks go away.
    pub(super) fn clear_search(&mut self) {
        self.search = None;
    }

    /// `3/17` once the cursor sits on a match, `–/17` elsewhere, for the status line.
    pub fn search_count(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        let search = self.search.as_ref().filter(|s| !s.query.is_empty())?;
        let hits = hits(&open.review, &search.query);
        let here = hit_at(open);
        let at = hits.iter().position(|hit| Some(*hit) == here).map_or_else(|| "–".to_owned(), |i| (i + 1).to_string());
        Some(format!("{at}/{}", hits.len()))
    }

    fn matches(&self) -> Vec<Hit> {
        match (&self.open, &self.search) {
            (Some(open), Some(search)) if !search.query.is_empty() => hits(&open.review, &search.query),
            _ => vec![],
        }
    }

    /// Each key typed looks again from where the search started, so the cursor follows the word.
    fn retype(&mut self, search: &Search, query: String) {
        self.search = Some(Search { query, ..search.clone() });
        let Some(open) = &self.open else { return };
        let start = open.rows.get(search.from).and_then(hit_of);
        let hits = self.matches();
        let first = hits.iter().find(|hit| Some(**hit) >= start).or(hits.first()).copied();
        match first {
            Some(hit) => self.reveal(hit),
            None => self.open = Some(open.move_to(search.from)),
        }
    }

    fn cancel_search(&mut self, search: &Search) {
        self.search = None;
        if let Some(open) = &self.open {
            self.open = Some(open.move_to(search.from));
        }
    }

    /// The cursor on the match, its file and hunk opened first when folded.
    fn reveal(&mut self, hit: Hit) {
        let Some(open) = &self.open else { return };
        let Some(file) = open.review.files.get(hit.file) else { return };
        let path = file.new_path.clone();
        let mut fold = open.review.fold.clone();
        if !fold.file_is_open(&path) {
            fold = fold.toggle_file(&path);
        }
        if let Some(hunk) = hit.hunk.filter(|&h| !fold.hunk_is_open(&path, h)) {
            fold = fold.toggle_hunk(&path, hunk);
        }
        let opened = if fold == open.review.fold { open.clone() } else { open.with_fold(fold) };
        let row = opened.rows.iter().position(|row| shows(row, hit)).or_else(|| opened.rows.iter().position(|row| shows_hunk(row, hit)));
        self.open = Some(row.map_or(opened.clone(), |at| opened.move_to(at)));
    }
}

/// Every match in the MR, in reading order: a file's path first, then its lines.
fn hits(review: &Review, query: &str) -> Vec<Hit> {
    let mut found = vec![];
    for (f, file) in review.files.iter().enumerate() {
        if finds(&file.new_path, query) {
            found.push(Hit { file: f, hunk: None, line: None });
        }
        for (h, hunk) in file.hunks.iter().enumerate() {
            let lines = hunk.lines.iter().enumerate().filter(|(_, line)| finds(&line.text, query));
            found.extend(lines.map(|(l, _)| Hit { file: f, hunk: Some(h), line: Some(l) }));
        }
    }
    found
}

/// Where the cursor is, in the order hits are read; `None` above the first file.
fn hit_at(open: &Open) -> Option<Hit> {
    open.row().and_then(hit_of)
}

fn hit_of(row: &Row) -> Option<Hit> {
    match row {
        &Row::File { index, .. } => Some(Hit { file: index, hunk: None, line: None }),
        &Row::Hunk { file, index, .. } => Some(Hit { file, hunk: Some(index), line: None }),
        &Row::Line { file, hunk, index } => Some(Hit { file, hunk: Some(hunk), line: Some(index) }),
        &Row::Pair { file, hunk, removed, .. } => Some(Hit { file, hunk: Some(hunk), line: Some(removed) }),
        &Row::Context { file, hunk, .. } => Some(Hit { file, hunk: Some(hunk), line: None }),
        Row::Header | Row::Gap => None,
    }
}

fn shows(row: &Row, hit: Hit) -> bool {
    match (row, hit.line) {
        (&Row::File { index, .. }, None) => hit.hunk.is_none() && index == hit.file,
        (&Row::Line { file, hunk, index }, Some(line)) => (file, Some(hunk), index) == (hit.file, hit.hunk, line),
        (&Row::Pair { file, hunk, removed, added }, Some(line)) => {
            (file, Some(hunk)) == (hit.file, hit.hunk) && (removed == line || added == line)
        }
        _ => false,
    }
}

fn shows_hunk(row: &Row, hit: Hit) -> bool {
    matches!(row, &Row::Hunk { file, index, .. } if file == hit.file && Some(index) == hit.hunk)
}

/// Smart case, as vim's `smartcase`: all lowercase matches any case, a capital asks for the exact one.
fn finds(text: &str, query: &str) -> bool {
    !query.is_empty() && (if exact_case(query) { text.contains(query) } else { text.to_lowercase().contains(&query.to_lowercase()) })
}

fn exact_case(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

fn last_len(text: &str) -> usize {
    text.chars().next_back().map_or(0, char::len_utf8)
}

/// `line` with every match of `query` past column `from` drawn in `style`, the rest untouched.
pub fn mark(line: &Line<'static>, query: &str, from: usize, style: Style) -> Line<'static> {
    let chars: Vec<(char, Style)> = line.spans.iter().flat_map(|s| s.content.chars().map(move |c| (c, s.style))).collect();
    let wanted: Vec<char> = query.chars().collect();
    let same = |a: char, b: char| if exact_case(query) { a == b } else { a.to_lowercase().eq(b.to_lowercase()) };
    let mut marked = vec![false; chars.len()];
    let mut column = 0;
    for at in 0..chars.len() {
        let fits =
            !wanted.is_empty() && at + wanted.len() <= chars.len() && wanted.iter().enumerate().all(|(k, &q)| same(chars[at + k].0, q));
        if fits && column >= from {
            marked[at..at + wanted.len()].iter_mut().for_each(|m| *m = true);
        }
        column += unicode_width::UnicodeWidthChar::width(chars[at].0).unwrap_or(0);
    }
    let mut spans: Vec<Span<'static>> = vec![];
    for ((c, base), on) in chars.into_iter().zip(marked) {
        let style = if on { base.patch(style) } else { base };
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), style)),
        }
    }
    Line { spans, style: line.style, alignment: line.alignment }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn smart_case_matches_any_case_until_a_capital_is_typed() {
        assert!(finds("let Client = x", "client"));
        assert!(finds("let client = x", "client"));
        assert!(!finds("let client = x", "Client"));
        assert!(!finds("anything", ""));
    }

    #[test]
    fn marks_fall_on_the_match_past_the_gutter_only() {
        let line = Line::from(vec![Span::raw(" 12 "), Span::raw("let x = 12;")]);
        let marked = mark(&line, "12", 4, Style::default().add_modifier(Modifier::REVERSED));
        let shown: Vec<(&str, bool)> =
            marked.spans.iter().map(|s| (s.content.as_ref(), s.style.add_modifier.contains(Modifier::REVERSED))).collect();
        assert_eq!(shown, [(" 12 let x = ", false), ("12", true), (";", false)]);
    }
}
