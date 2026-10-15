//! The conversations of a place as notes and drafts, and as the diff draws them under a line.
use super::body::{Fences, Replaced, body_pieces};
use super::{Look, Piece, lay_out};
use crate::forge::{Note, ThreadId};
use crate::review::{Anchor, Conversation, Place, Review, Side, Spot, Thread};
use crate::tui::app::{Entry, EntryKind, Open, ReviewInputs};
use crate::tui::images::Thumbs;
use crate::tui::theme::Theme;
use crate::tui::ui::short_age;
use chrono::{DateTime, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::{BTreeMap, BTreeSet};
use unicode_width::UnicodeWidthStr;

const SEPARATOR: &str = " · ";

/// The conversations of each line as the diff draws them under it, kept while what they are laid out from stays
/// the same: code in a note is highlighted when its rows are laid out, never on each frame.
#[derive(Debug, Default)]
pub(crate) struct InlineLayout {
    inputs: Option<InlineInputs>,
    stops: BTreeMap<LineKey, Vec<StopRows>>,
}

/// A line of a file: the file, its new number, its old one.
type LineKey = (usize, Option<u32>, Option<u32>);
/// The screen rows of one cursor stop.
type StopRows = Vec<Line<'static>>;

#[derive(Debug, PartialEq)]
struct InlineInputs {
    review: ReviewInputs,
    unfolded: BTreeSet<ThreadId>,
    width: usize,
    theme: Theme,
    ages: Vec<String>,
    me: String,
    ascii: bool,
}

impl InlineLayout {
    /// Forgets the rows laid out from anything but what `open` and `look` show now.
    pub(in crate::tui) fn fit(&mut self, open: &Open, look: Look<'_>) {
        let inputs = InlineInputs {
            review: ReviewInputs::from_open(open),
            unfolded: open.review.unfolded.clone(),
            width: look.width,
            theme: look.theme,
            ages: shown_ages(&open.review, look.today),
            me: look.me.to_owned(),
            ascii: look.ascii,
        };
        if self.inputs.as_ref() != Some(&inputs) {
            *self = Self { inputs: Some(inputs), stops: BTreeMap::new() };
        }
    }

    /// The rows of each cursor stop of the conversations on a line.
    pub(in crate::tui) fn stops(&mut self, review: &Review, place: &Place, look: Look<'_>) -> &[StopRows] {
        let Place::Line { file, new, old } = place else { return &[] };
        self.stops.entry((*file, *new, *old)).or_insert_with(|| inline_stops(review, place, look))
    }
}

/// The conversations of `place` as the rows of each cursor stop, `look.width` columns wide: a folded thread is its
/// one summary row, any other reads as in the pane. A picture reads as its `[image: …]` line.
fn inline_stops(review: &Review, place: &Place, look: Look<'_>) -> Vec<StopRows> {
    let conversations = review.conversations(place);
    let entries = review.stops(&conversations);
    let thumbs = Thumbs::disabled();
    let rows_of = |piece: Piece| lay_out(piece, &thumbs, look.width, look.theme, &str::to_owned).into_iter().map(|(line, _)| line);
    let mut stops = vec![vec![]; entries.len()];
    for (index, conversation) in conversations.iter().enumerate() {
        let thread = conversation.thread.as_ref().and_then(|id| review.thread(id));
        let pieces = match thread.filter(|thread| review.reads_folded(thread)) {
            Some(folded) => {
                let first = entries.iter().find(|e| e.conversation == index).copied();
                vec![(first, Piece::Text(summary(folded, conversation.drafts.len(), look)))]
            }
            None => conversation_lines(review, conversation, index, &entries, look),
        };
        for (entry, piece) in pieces {
            if let Some(stop) = entries.iter().position(|e| Some(*e) == entry) {
                stops[stop].extend(rows_of(piece));
            }
        }
    }
    stops
}

/// A folded thread on one row: its state, who started it, the first line they wrote, how many notes follow, and
/// a `◇` while replies of mine wait in it.
fn summary(thread: &Thread, drafts: usize, look: Look<'_>) -> Line<'static> {
    let Look { theme, me, width, .. } = look;
    let faded = Style::default().fg(theme.faded);
    let first = thread.first();
    let author = author_of(first, me);
    let replies = match thread.notes.len().saturating_sub(1) {
        0 => String::new(),
        1 => " · 1 reply".to_owned(),
        n => format!(" · {n} replies"),
    };
    let waiting = if drafts > 0 { " ◇" } else { "" };
    let words = first.body.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let state = status(thread, thread.notes.len(), theme);
    let taken = state.width() + author.width() + replies.width() + waiting.width() + 2 * SEPARATOR.width();
    let mut spans = state.spans;
    spans.extend([
        Span::styled(SEPARATOR, faded),
        Span::styled(author.clone(), Style::default().fg(theme.user(&author)).add_modifier(Modifier::BOLD)),
        Span::styled(SEPARATOR, faded),
        Span::raw(crate::tui::ui::truncate(words, width.saturating_sub(taken))),
        Span::styled(replies, faded),
        Span::styled(waiting, Style::default().fg(theme.accent)),
    ]);
    Line::from(spans)
}

/// A thread, or my new draft, as the pane shows it: status, the notes `entries` stop on, my replies; each line
/// tagged with the cursor stop it belongs to.
pub(super) fn conversation_lines(
    review: &Review,
    conversation: &Conversation,
    index: usize,
    entries: &[Entry],
    look: Look<'_>,
) -> Vec<(Option<Entry>, Piece)> {
    let theme = look.theme;
    let stop = |kind: EntryKind| entries.iter().find(|e| e.conversation == index && e.kind == kind).copied();
    let mut lines = vec![];
    if let Some(thread) = conversation.thread.as_ref().and_then(|id| review.thread(id)) {
        let shown = entries.iter().filter(|e| e.conversation == index && matches!(e.kind, EntryKind::Note(_))).count();
        lines.push((stop(EntryKind::Note(0)), Piece::Text(status(thread, shown, theme))));
        let replaced = Replaced::at(review, thread.first().position.as_ref());
        for (n, note) in thread.notes.iter().take(shown).enumerate() {
            let entry = stop(EntryKind::Note(n));
            lines.extend(note_lines(note, &replaced, look).into_iter().map(|line| (entry, line)));
        }
    }
    for &draft in &conversation.drafts {
        let entry = stop(EntryKind::Draft(draft));
        let draft = &review.drafts[draft];
        if conversation.thread.is_none() {
            lines.push((entry, Piece::Text(Line::from(Span::styled("◇ draft", Style::default().fg(theme.accent))))));
        }
        let position = draft.position.as_ref().or_else(|| {
            let thread = conversation.thread.as_ref().and_then(|id| review.thread(id))?;
            thread.first().position.as_ref()
        });
        let replaced = Replaced::at(review, position);
        lines.extend(draft_lines(draft, &replaced, look).into_iter().map(|line| (entry, line)));
    }
    lines
}

/// Where a conversation hangs, over it in the list of every conversation: `path:line`, then the
/// code of that line, faded, so the reader keeps the context.
pub(super) fn heading(review: &Review, conversation: &Conversation, look: Look<'_>) -> Vec<Line<'static>> {
    let (place, code) = match review.spot(conversation) {
        Spot::Mr => ("on the MR".to_owned(), None),
        Spot::Outdated(anchor) => (format!("{} · outdated", line_name(anchor)), None),
        Spot::Line(anchor) => (line_name(anchor), review.line_text(anchor)),
    };
    let place = Line::from(Span::styled(place, Style::default().fg(look.theme.muted)));
    let code =
        code.map(|text| Line::from(Span::styled(crate::tui::ui::truncate(text.trim(), look.width), Style::default().fg(look.theme.faded))));
    std::iter::once(place).chain(code).collect()
}

/// `src/pay/charge.rs:57`, `src/pay/charge.rs:-13` for the old side.
fn line_name(anchor: &Anchor) -> String {
    match anchor.side {
        Side::New => format!("{}:{}", anchor.path, anchor.line),
        Side::Old => format!("{}:-{}", anchor.path, anchor.line),
    }
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

/// My draft: `you · draft ◇`, `unsaved` in danger until the forge holds it.
fn draft_lines(draft: &crate::review::Draft, replaced: &Replaced, look: Look<'_>) -> Vec<Piece> {
    let theme = look.theme;
    let (state, colour) = if draft.id.is_none() { ("unsaved", theme.danger) } else { ("draft", theme.muted) };
    let mut lines = vec![Piece::Text(Line::from(vec![
        Span::styled("you", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {state} ◇"), Style::default().fg(colour)),
    ]))];
    lines.extend(body_pieces(&draft.body, &Fences::Coloured(replaced), look.width, theme));
    lines
}

/// Who wrote a note, `you` for me.
fn author_of(note: &Note, me: &str) -> String {
    if note.author.username == me { "you".to_owned() } else { note.author.username.clone() }
}

fn note_age(note: &Note, today: DateTime<Utc>) -> String {
    short_age((today - note.created_at).to_std().unwrap_or_default())
}

/// The age of every note as it reads now: the layouts hold until one of them reads otherwise.
pub(in crate::tui) fn shown_ages(review: &Review, today: DateTime<Utc>) -> Vec<String> {
    review.threads.iter().flat_map(|thread| &thread.notes).map(|note| note_age(note, today)).collect()
}

fn note_lines(note: &Note, replaced: &Replaced, look: Look<'_>) -> Vec<Piece> {
    let Look { theme, today, me, ascii, width } = look;
    let author = author_of(note, me);
    let age = note_age(note, today);
    let mut lines = vec![Piece::Text(Line::from(vec![
        Span::styled(author.clone(), Style::default().fg(theme.user(&author)).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {age}"), Style::default().fg(theme.muted)),
    ]))];
    lines.extend(body_pieces(&note.body, &Fences::Coloured(replaced), width, theme));
    if !note.reactions.is_empty() {
        lines.push(Piece::Text(reactions_line(&note.reactions, theme, ascii)));
    }
    lines
}

/// `👍 2  🎉 1` under a note, mine in the accent colour.
fn reactions_line(reactions: &[crate::forge::Reaction], theme: Theme, ascii: bool) -> Line<'static> {
    let spans = reactions.iter().enumerate().flat_map(|(i, r)| {
        let face = if ascii { r.emoji.text() } else { r.emoji.glyph() };
        let style = if r.mine { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.muted) };
        let gap = if i == 0 { "" } else { "  " };
        [Span::raw(gap), Span::styled(format!("{face} {}", r.count), style)]
    });
    Line::from(spans.collect::<Vec<_>>())
}
