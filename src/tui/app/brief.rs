//! The MR's cover page, `i`: what it says about itself, its checks, who reviews it, its open
//! threads and its files, from the queue row or the open review. Threads and files are rows the
//! cursor walks; `enter` jumps to one in the diff. It opens only on demand: an MR opens on its diff.
use super::{Action, App, Focus};
use crate::ai::triage::Risk;
use crate::forge::checks::JobState;
use crate::forge::{MrKey, QueueMr, ReviewState};
use crate::review::{Place, Review, Row, Side};
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;

const HALF_PAGE: usize = 10;
/// Words of a thread's first note on its row.
const FIRST_WORDS: usize = 8;

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
    /// Open threads as the queue counts them, shown when `threads` is `None`.
    pub unresolved: usize,
    /// The files, riskiest (or biggest) first; `None` from the queue.
    pub files: Option<Vec<FileRow>>,
    /// Which thread or file row the cursor is on: threads first, then files.
    pub selected: usize,
    /// First line shown; the view clamps it and keeps the cursor on screen while it moves.
    pub scroll: usize,
    /// The cursor moved since the last frame, so the view brings it into sight.
    pub follow: bool,
}

/// The pipeline in one line: its status, and the names of the jobs that failed once fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    pub status: Option<String>,
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
    /// `path:line`, `path (outdated)` or `the MR`.
    pub place: String,
    pub author: String,
    pub first_words: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub index: usize,
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub viewed: bool,
    pub risk: Option<Risk>,
}

/// What `enter` on the cursor's row leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Thread(String),
    File(usize),
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
            checks: Checks { status: mr.pipeline.clone(), failed: vec![] },
            review: ReviewLine {
                approved_by: mr.approved_by.clone(),
                approvals_left: mr.approvals_left,
                reviewers,
                i_approved: mr.approved_by.iter().any(|u| u == me),
                i_review: mr.reviewers.iter().any(|r| r.username == me),
            },
            threads: None,
            unresolved: mr.unresolved as usize,
            files: None,
            selected: 0,
            scroll: 0,
            follow: false,
        }
    }

    /// `checks` are the jobs when the pipeline pane already fetched them; `risks` Jev's reading.
    pub fn of_review(
        key: MrKey,
        review: &Review,
        sigil: char,
        me: &str,
        checks: Option<&crate::forge::checks::Checks>,
        risks: Option<&BTreeMap<String, Risk>>,
    ) -> Self {
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
            checks: Checks { status: mr.pipeline.as_ref().map(|p| p.status.clone()), failed },
            review: ReviewLine {
                approved_by: mr.approvals.approved_by.iter().map(|u| u.username.clone()).collect(),
                approvals_left: Some(mr.approvals.approvals_left),
                reviewers: mr.reviewers.iter().map(|u| (u.username.clone(), None)).collect(),
                i_approved: mr.approvals.user_has_approved,
                i_review: mr.reviewers.iter().any(|u| u.username == me),
            },
            unresolved: threads.len(),
            threads: Some(threads),
            files: Some(files_by_risk(review, risks)),
            selected: 0,
            scroll: 0,
            follow: false,
        }
    }

    /// Threads then files: the rows the cursor walks, in the order they are drawn.
    pub fn targets(&self) -> Vec<Target> {
        let threads = self.threads.iter().flatten().map(|t| Target::Thread(t.id.clone()));
        let files = self.files.iter().flatten().map(|f| Target::File(f.index));
        threads.chain(files).collect()
    }

    fn moved(&self, by: isize) -> Self {
        let last = self.targets().len().saturating_sub(1);
        let selected = self.selected.saturating_add_signed(by).min(last);
        Self { selected, follow: true, ..self.clone() }
    }

    fn scrolled(&self, by: isize) -> Self {
        Self { scroll: self.scroll.saturating_add_signed(by), follow: false, ..self.clone() }
    }
}

/// Unresolved threads: those on lines first in file order, then outdated ones, then the MR's own.
fn open_threads(review: &Review) -> Vec<ThreadRow> {
    let mut threads: Vec<&crate::review::Thread> = review.threads.iter().filter(|t| t.resolvable && !t.resolved).collect();
    threads.sort_by_key(|t| match &t.anchor {
        Some(a) if !t.outdated => (0, a.path.clone(), a.line),
        Some(a) => (1, a.path.clone(), a.line),
        None => (2, String::new(), 0),
    });
    threads
        .into_iter()
        .map(|t| {
            let place = match &t.anchor {
                Some(a) if t.outdated => format!("{} (outdated)", a.path),
                Some(a) => format!("{}:{}", a.path, a.line),
                None => "the MR".to_owned(),
            };
            let note = t.first();
            let first_words = note.body.split_whitespace().take(FIRST_WORDS).collect::<Vec<_>>().join(" ");
            ThreadRow { id: t.id.clone(), place, author: note.author.username.clone(), first_words }
        })
        .collect()
}

/// Files riskiest first when Jev read them, else the biggest change first.
fn files_by_risk(review: &Review, risks: Option<&BTreeMap<String, Risk>>) -> Vec<FileRow> {
    let mut files: Vec<FileRow> = review
        .files
        .iter()
        .enumerate()
        .map(|(index, f)| FileRow {
            index,
            path: f.new_path.clone(),
            additions: f.additions,
            deletions: f.deletions,
            viewed: review.viewed.contains(&f.new_path),
            risk: risks.and_then(|r| r.get(&f.new_path).copied()),
        })
        .collect();
    files.sort_by_key(|f| (std::cmp::Reverse(f.risk), std::cmp::Reverse(f.additions + f.deletions), f.index));
    files
}

impl App {
    /// `i` in the queue: the cover from what the queue row knows.
    pub(super) fn open_brief_from_queue(&mut self) {
        self.brief = self.selected_mr().map(|mr| Brief::of_queue(mr, self.hosts.kind_of(&mr.key()).sigil(), &self.me));
    }

    /// `i` in the review: the cover with its threads, files and, once fetched, failed jobs.
    pub(super) fn open_brief_from_review(&mut self) {
        let Some(open) = &self.open else { return };
        let checks = open.pipeline.as_ref().and_then(|p| match &p.run {
            super::pipeline::Run::Ready(checks) => Some(checks),
            _ => None,
        });
        let risks = self.readings.get(&open.key).map(|(_, reading)| &reading.risks);
        let sigil = self.hosts.kind_of(&open.key).sigil();
        self.brief = Some(Brief::of_review(open.key.clone(), &open.review, sigil, &self.me, checks, risks));
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
            KeyCode::Char('j') | KeyCode::Down if walks => Some(brief.moved(1)),
            KeyCode::Char('k') | KeyCode::Up if walks => Some(brief.moved(-1)),
            KeyCode::Char('j') | KeyCode::Down => Some(brief.scrolled(1)),
            KeyCode::Char('k') | KeyCode::Up => Some(brief.scrolled(-1)),
            KeyCode::Char('g') => Some(Brief { scroll: 0, selected: 0, follow: false, ..brief }),
            KeyCode::Char('G') if walks => Some(brief.moved(isize::MAX / 2)),
            KeyCode::Char('G') => Some(Brief { scroll: usize::MAX, ..brief }),
            _ => Some(brief),
        };
        vec![]
    }

    /// `enter`: the thread or file under the cursor in the diff; from the queue, the MR itself.
    fn brief_enter(&mut self, brief: &Brief) -> Vec<Action> {
        self.brief = None;
        if self.open.as_ref().is_none_or(|o| o.key != brief.key) {
            return self.open_selected();
        }
        self.focus = Focus::Review;
        match brief.targets().get(brief.selected) {
            Some(Target::Thread(id)) => self.show_thread(id),
            Some(Target::File(index)) => self.show_file(*index),
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
        let Some(file) = open.review.files.iter().position(|f| match anchor.side {
            Side::New => f.new_path == anchor.path,
            Side::Old => f.old_path == anchor.path,
        }) else {
            return vec![];
        };
        if thread.outdated {
            self.review_jump_to(|row| matches!(row, Row::File { index, .. } if *index == file));
            self.open_pane(Place::Outdated { file });
            return vec![];
        }
        let path = open.review.files[file].new_path.clone();
        let actions = if open.review.fold.file_is_open(&path) { vec![] } else { self.set_file_fold(&path, crate::diff::fold::Fold::Open) };
        let Some(open) = &self.open else { return actions };
        let on_anchor = |place: &Place| match place {
            Place::Line { file: f, new, old } => {
                *f == file
                    && match anchor.side {
                        Side::New => *new == Some(anchor.line),
                        Side::Old => *old == Some(anchor.line),
                    }
            }
            _ => false,
        };
        let found = open.rows.iter().enumerate().find_map(|(i, row)| open.review.place_of(row).filter(on_anchor).map(|p| (i, p)));
        if let Some((index, place)) = found {
            self.open = Some(open.move_to(index));
            self.open_pane(place);
        } else {
            self.review_jump_to(|row| matches!(row, Row::File { index, .. } if *index == file));
            self.toast("its hunk is folded: zo opens it");
        }
        actions
    }
}
