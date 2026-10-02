//! The MR's cover page, `i`: what it says about itself, its checks, who reviews it and its open
//! threads, from the queue row or the open review. Threads are rows the cursor walks; `enter` jumps
//! to one in the diff. It opens only on demand: an MR opens on its diff.
use super::{Action, App, Focus};
use crate::forge::checks::JobState;
use crate::forge::{MrKey, PipelineStatus, QueueMr, ReviewState};
use crate::review::{Place, Review, Row};
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HALF_PAGE: usize = 10;
/// Words of a thread's first note kept for its row; the view cuts them to the width.
const FIRST_WORDS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brief {
    pub key: MrKey,
    /// `!` or `#`, as the MR's forge names it.
    pub sigil: char,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub labels: Vec<String>,
    pub description: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub checks: Checks,
    pub review: ReviewLine,
    /// The open threads; `None` from the queue, which only counts them.
    pub threads: Option<Vec<ThreadRow>>,
    /// The review app to try; known only from an open MR.
    pub deployments: Vec<crate::forge::Deployment>,
    /// Open threads as the queue counts them, shown when `threads` is `None`.
    pub unresolved: usize,
    /// The thread the cursor is on; `None` while reading above them, after `g` or `k` past the first.
    pub selected: Option<usize>,
    /// First line shown; the view clamps it and keeps the cursor on screen while it moves.
    pub scroll: usize,
    /// The cursor moved since the last frame, so the view brings it into sight.
    pub follow: bool,
}

/// The pipeline in one line: its status, and the names of the jobs that failed once fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    pub status: Option<PipelineStatus>,
    pub failed: Vec<String>,
}

/// Approvals and reviewers, and where I stand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewLine {
    pub approved_by: Vec<String>,
    pub approvals_left: Option<u32>,
    /// Reviewers with their state when the forge gave it (the queue does, one MR's page does not).
    pub reviewers: Vec<(String, Option<ReviewState>)>,
    pub i_approved: bool,
    pub i_review: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRow {
    pub id: String,
    /// `path:line`, `path (outdated)` or `the MR`: the whole place, shown while the row is selected.
    pub place: String,
    /// The same place with the file name only, for the row's quiet second line.
    pub short_place: String,
    pub author: String,
    pub first_words: String,
    pub replies: usize,
}

impl Brief {
    pub fn of_queue(mr: &QueueMr, sigil: char, me: &str) -> Self {
        let reviewers = mr.reviewers.iter().map(|r| (r.username.clone(), Some(r.state))).collect();
        Self {
            key: mr.key(),
            sigil,
            number: mr.number,
            title: mr.title.clone(),
            author: mr.author.clone(),
            source_branch: mr.source_branch.clone(),
            target_branch: mr.target_branch.clone(),
            labels: mr.labels.clone(),
            description: mr.description.clone(),
            web_url: mr.web_url.clone(),
            updated_at: mr.updated_at,
            checks: Checks { status: mr.pipeline, failed: vec![] },
            review: ReviewLine {
                approved_by: mr.approved_by.clone(),
                approvals_left: mr.approvals_left,
                reviewers,
                i_approved: mr.approved_by.iter().any(|u| u == me),
                i_review: mr.reviewers.iter().any(|r| r.username == me),
            },
            threads: None,
            deployments: vec![],
            unresolved: mr.unresolved as usize,
            selected: None,
            scroll: 0,
            follow: false,
        }
    }

    /// `checks` are the jobs when the pipeline pane already fetched them.
    pub fn of_review(key: MrKey, review: &Review, sigil: char, me: &str, checks: Option<&crate::forge::checks::Checks>) -> Self {
        let mr = &review.mr;
        let failed = checks.map_or_else(Vec::new, |c| {
            c.jobs().filter(|j| j.state == JobState::Failed && !j.allowed_to_fail).map(|j| j.name.clone()).collect()
        });
        let threads = open_threads(review);
        Self {
            key,
            sigil,
            number: mr.number,
            title: mr.title.clone(),
            author: mr.author.username.clone(),
            source_branch: mr.source_branch.clone(),
            target_branch: mr.target_branch.clone(),
            labels: mr.labels.clone(),
            description: mr.description.clone(),
            web_url: mr.web_url.clone(),
            updated_at: mr.updated_at,
            checks: Checks { status: mr.pipeline.as_ref().map(|p| p.status), failed },
            review: ReviewLine {
                approved_by: mr.approvals.approved_by.iter().map(|u| u.username.clone()).collect(),
                approvals_left: Some(mr.approvals.approvals_left),
                reviewers: mr.reviewers.iter().map(|u| (u.username.clone(), None)).collect(),
                i_approved: mr.approvals.user_has_approved,
                i_review: mr.reviewers.iter().any(|u| u.username == me),
            },
            selected: (!threads.is_empty()).then_some(0),
            unresolved: threads.len(),
            threads: Some(threads),
            deployments: vec![],
            scroll: 0,
            follow: false,
        }
    }

    /// The threads the cursor walks, in the order they are drawn.
    pub fn targets(&self) -> Vec<String> {
        self.threads.iter().flatten().map(|t| t.id.clone()).collect()
    }

    /// The selected thread, for its full place and for `enter`.
    pub fn selected_thread(&self) -> Option<&ThreadRow> {
        self.threads.as_ref()?.get(self.selected?)
    }

    fn down(self) -> Self {
        let last = self.targets().len().saturating_sub(1);
        let selected = self.selected.map_or(0, |i| (i + 1).min(last));
        Self { selected: Some(selected), follow: true, ..self }
    }

    /// Up from the first thread lets go of it and scrolls on, so the head and description come back.
    fn up(self) -> Self {
        match self.selected {
            Some(i) if i > 0 => Self { selected: Some(i - 1), follow: true, ..self },
            _ => Self { selected: None, ..self.scrolled(-1) },
        }
    }

    fn scrolled(self, by: isize) -> Self {
        Self { scroll: self.scroll.saturating_add_signed(by), follow: false, ..self }
    }
}

/// Unresolved threads: those on lines first in file order, then outdated ones, then the MR's own.
fn open_threads(review: &Review) -> Vec<ThreadRow> {
    let mut threads: Vec<&crate::review::Thread> = review.threads.iter().filter(|t| t.unresolved()).collect();
    threads.sort_by_key(|t| match &t.anchor {
        Some(a) if !t.outdated => (0, a.path.clone(), a.line),
        Some(a) => (1, a.path.clone(), a.line),
        None => (2, String::new(), 0),
    });
    threads
        .into_iter()
        .map(|t| {
            let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
            let (place, short_place) = match &t.anchor {
                Some(a) if t.outdated => (format!("{} (outdated)", a.path), format!("{} (outdated)", name(&a.path))),
                Some(a) => (format!("{}:{}", a.path, a.line), format!("{}:{}", name(&a.path), a.line)),
                None => ("the MR".to_owned(), "on the MR".to_owned()),
            };
            let note = t.first();
            let first_words = note.body.split_whitespace().take(FIRST_WORDS).collect::<Vec<_>>().join(" ");
            let replies = t.notes.len().saturating_sub(1);
            ThreadRow { id: t.id.clone(), place, short_place, author: note.author.username.clone(), first_words, replies }
        })
        .collect()
}

impl App {
    /// `i` in the queue: the cover from what the queue row knows.
    pub(super) fn open_brief_from_queue(&mut self) {
        self.brief = self.selected_mr().map(|mr| Brief::of_queue(mr, self.hosts.kind_of(&mr.key()).sigil(), &self.me));
    }

    /// `i` in the review: the cover with its threads and, once fetched, failed jobs.
    pub(super) fn open_brief_from_review(&mut self) {
        let Some(open) = &self.open else { return };
        let checks = open.pipeline.as_ref().and_then(|p| match &p.run {
            super::pipeline::Run::Ready(checks) => Some(checks),
            _ => None,
        });
        let sigil = self.hosts.kind_of(&open.key).sigil();
        let brief = Brief::of_review(open.key.clone(), &open.review, sigil, &self.me, checks);
        let to_try = open.deployments.as_deref().and_then(crate::forge::Deployment::to_try).cloned();
        self.brief = Some(Brief { deployments: to_try.into_iter().collect(), ..brief });
    }

    pub(super) fn handle_brief_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(brief) = self.brief.clone() else { return vec![] };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let walks = !brief.targets().is_empty();
        self.brief = match key.code {
            KeyCode::Esc | KeyCode::Char('i' | 'q') => None,
            KeyCode::Char('o') => return vec![Action::OpenUrl(brief.web_url)],
            KeyCode::Enter => return self.brief_enter(&brief),
            KeyCode::Char('p') => return self.brief_pipeline(),
            KeyCode::Char('d') if ctrl => Some(brief.scrolled(HALF_PAGE as isize)),
            KeyCode::Char('u') if ctrl => Some(brief.scrolled(-(HALF_PAGE as isize))),
            KeyCode::Char('j') | KeyCode::Down if walks => Some(brief.down()),
            KeyCode::Char('k') | KeyCode::Up if walks => Some(brief.up()),
            KeyCode::Char('j') | KeyCode::Down => Some(brief.scrolled(1)),
            KeyCode::Char('k') | KeyCode::Up => Some(brief.scrolled(-1)),
            KeyCode::Char('g') => Some(Brief { scroll: 0, selected: None, follow: false, ..brief }),
            KeyCode::Char('G') if walks => {
                Some(Brief { selected: Some(brief.targets().len() - 1), follow: true, scroll: usize::MAX, ..brief })
            }
            KeyCode::Char('G') => Some(Brief { scroll: usize::MAX, ..brief }),
            _ => Some(brief),
        };
        vec![]
    }

    /// `enter`: the thread under the cursor in the diff; from the queue, the MR itself.
    fn brief_enter(&mut self, brief: &Brief) -> Vec<Action> {
        self.brief = None;
        if self.open.as_ref().is_none_or(|o| o.key != brief.key) {
            return self.open_selected();
        }
        self.focus = Focus::Review;
        match brief.selected_thread() {
            Some(thread) => self.show_thread(&thread.id.clone()),
            None => vec![],
        }
    }

    /// `p`: the pipeline pane, for an open MR.
    fn brief_pipeline(&mut self) -> Vec<Action> {
        let on_open = self.open.as_ref().is_some_and(|o| self.brief.as_ref().is_some_and(|b| b.key == o.key));
        if !on_open {
            self.toast("open the MR to see its pipeline");
            return vec![];
        }
        self.brief = None;
        if self.pipeline_open() { vec![] } else { self.toggle_pipeline() }
    }

    /// Puts the cursor on the thread's line, its file unfolded, and opens it in the pane.
    fn show_thread(&mut self, id: &str) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(thread) = open.review.thread(id) else { return vec![] };
        let Some(anchor) = thread.anchor.clone() else {
            self.review_jump_to(|row| matches!(row, Row::Header));
            self.open_pane(Place::Mr);
            return vec![];
        };
        let Some(file) = open.review.file_of(&anchor) else { return vec![] };
        if thread.outdated {
            self.review_jump_to(|row| matches!(row, Row::File { index, .. } if *index == file));
            self.open_pane(Place::Outdated { file });
            return vec![];
        }
        let path = open.review.files[file].new_path.clone();
        let actions = if open.review.fold.file_is_open(&path) { vec![] } else { self.set_file_fold(&path, crate::diff::fold::Fold::Open) };
        let Some(open) = &self.open else { return actions };
        let found = open.rows.iter().enumerate().find(|(_, row)| open.review.row_holds(row, &anchor));
        let found = found.and_then(|(i, row)| open.review.place_of(row).map(|p| (i, p)));
        if let Some((index, place)) = found {
            self.update_open(|open| open.move_to(index));
            self.open_pane(place);
        } else {
            self.review_jump_to(|row| matches!(row, Row::File { index, .. } if *index == file));
            self.toast("its hunk is folded: zo opens it");
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn i_opens_the_description_from_the_queue_and_the_review_and_closes_on_esc() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        press(&mut app, "Gk");
        press(&mut app, "zo");
        press(&mut app, "i");
        let brief = app.brief.clone().unwrap();
        assert_eq!((brief.number, brief.description.as_str()), (50, "Adds the refund flow."));
        assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl(brief.web_url)]);
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.brief, None);
        let mut app = with_review();
        press(&mut app, "i");
        assert_eq!(app.brief.as_ref().map(|b| b.number), Some(42));
        press(&mut app, "q");
        assert!(app.brief.is_none() && !app.should_quit, "q closes the modal, it does not quit");
    }

    #[test]
    fn the_cover_scrolls_its_text_and_keeps_the_cursor_row_in_sight() {
        let mut app = with_review();
        let long: String = (1..=80).map(|n| format!("line {n}\n")).collect();
        app.open = app.open.clone().map(|o| {
            let mut review = o.review.clone();
            std::sync::Arc::make_mut(&mut review.mr).description = long.clone();
            o.with_review(review)
        });
        press(&mut app, "i");
        app.handle_key(ctrl('d'));
        assert_eq!(app.brief.as_ref().unwrap().scroll, 10);
        app.handle_key(ctrl('u'));
        assert_eq!(app.brief.as_ref().unwrap().scroll, 0);
        render(&mut app, 100, 30);
        press(&mut app, "G");
        let screen = render(&mut app, 100, 30);
        assert!(app.brief.as_ref().unwrap().scroll > 0, "the last thread pulls the page down");
        assert!(screen.lines().any(|l| l.contains("▎ ") && l.contains("(outdated)")), "the cursor row is on screen:\n{screen}");
    }

    #[test]
    fn the_cover_walks_threads_and_enter_goes_there() {
        let mut app = with_review();
        press(&mut app, "i");
        let brief = app.brief.clone().unwrap();
        assert_eq!(brief.targets().len(), 1, "one open thread, and no file rows any more");
        assert_eq!(brief.selected, Some(0), "the first thread is ready for enter");
        assert!(!render(&mut app, 120, 40).contains("FILES"));
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert_eq!(
            open.pane.as_ref().map(|p| p.place.clone()),
            Some(crate::review::Place::Outdated { file: 0 }),
            "the outdated thread opens in the pane"
        );
        assert_eq!(app.focus, Focus::Side);
    }

    #[test]
    fn up_from_the_first_thread_scrolls_back_to_the_top_and_g_stays_there() {
        let mut app = cover_with_threads(20);
        press(&mut app, "G");
        render(&mut app, 100, 24);
        let bottom = app.brief.as_ref().unwrap().scroll;
        assert!(bottom > 0, "the last thread is below the first page");
        press(&mut app, "g");
        render(&mut app, 100, 24);
        let brief = app.brief.as_ref().unwrap();
        assert_eq!((brief.scroll, brief.selected), (0, None), "g stays at the top: nothing pulls it back down");
        press(&mut app, "j");
        assert_eq!(app.brief.as_ref().unwrap().selected, Some(0));
        for _ in 0..3 {
            press(&mut app, "j");
        }
        render(&mut app, 100, 24);
        let before = app.brief.as_ref().unwrap().scroll;
        for _ in 0..(before + 10) {
            press(&mut app, "k");
        }
        render(&mut app, 100, 24);
        let brief = app.brief.as_ref().unwrap();
        assert_eq!((brief.scroll, brief.selected), (0, None), "k past the first thread lets go and reaches the head");
    }

    #[test]
    fn the_cover_from_the_queue_opens_the_mr_on_enter_and_has_no_pipeline_yet() {
        let mut app = with_queue();
        app.queue_move(0);
        press(&mut app, "i");
        assert!(app.brief.as_ref().unwrap().threads.is_none());
        assert!(render(&mut app, 120, 40).contains("open the MR to see its threads"));
        assert_eq!(press(&mut app, "p"), [] as [Action; 0]);
        assert!(app.live_toast().unwrap().text.contains("open the MR"));
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::Open(mr_key())]);
        assert_eq!(app.brief, None);
    }

    #[test]
    fn p_on_the_cover_opens_the_pipeline_pane() {
        let mut app = with_review();
        press(&mut app, "i");
        let actions = press(&mut app, "p");
        assert_eq!(app.brief, None);
        assert!(app.pipeline_open());
        assert_eq!(actions.len(), 1, "the pipeline loads");
    }
}
