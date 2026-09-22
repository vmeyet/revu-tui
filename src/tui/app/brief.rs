//! The description modal: what the MR says about itself, from the queue row or the open review.
use super::{Action, App};
use crate::api::QueueMr;
use crate::review::Review;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HALF_PAGE: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brief {
    pub iid: u64,
    pub title: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub labels: Vec<String>,
    pub description: String,
    pub web_url: String,
    /// First line shown; the view clamps it to what the text allows.
    pub scroll: usize,
}

impl Brief {
    pub fn of_queue(mr: &QueueMr) -> Self {
        Self {
            iid: mr.iid,
            title: mr.title.clone(),
            author: mr.author.clone(),
            source_branch: mr.source_branch.clone(),
            target_branch: mr.target_branch.clone(),
            labels: mr.labels.clone(),
            description: mr.description.clone(),
            web_url: mr.web_url.clone(),
            scroll: 0,
        }
    }

    pub fn of_review(review: &Review) -> Self {
        let mr = &review.mr;
        Self {
            iid: mr.iid,
            title: mr.title.clone(),
            author: mr.author.username.clone(),
            source_branch: mr.source_branch.clone(),
            target_branch: mr.target_branch.clone(),
            labels: mr.labels.clone(),
            description: mr.description.clone(),
            web_url: mr.web_url.clone(),
            scroll: 0,
        }
    }

    fn scrolled(&self, by: isize) -> Self {
        Self { scroll: self.scroll.saturating_add_signed(by), ..self.clone() }
    }
}

impl App {
    pub(super) fn handle_brief_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(brief) = self.brief.clone() else { return vec![] };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.brief = match key.code {
            KeyCode::Esc | KeyCode::Char('i' | 'q') => None,
            KeyCode::Char('o') => return vec![Action::OpenUrl(brief.web_url)],
            KeyCode::Char('d') if ctrl => Some(brief.scrolled(HALF_PAGE as isize)),
            KeyCode::Char('u') if ctrl => Some(brief.scrolled(-(HALF_PAGE as isize))),
            KeyCode::Char('j') | KeyCode::Down => Some(brief.scrolled(1)),
            KeyCode::Char('k') | KeyCode::Up => Some(brief.scrolled(-1)),
            KeyCode::Char('g') => Some(Brief { scroll: 0, ..brief }),
            KeyCode::Char('G') => Some(Brief { scroll: usize::MAX, ..brief }),
            _ => Some(brief),
        };
        vec![]
    }
}
