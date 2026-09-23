use super::{Action, App};
use crate::forge::{QueueMr, Sections};

/// The one glyph at the right edge of a queue row, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Failed,
    Running,
    Activity,
    Approved,
    Draft,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueueRow<'a> {
    Section { name: &'static str, count: usize, open: bool },
    Mr(&'a QueueMr),
}

impl App {
    /// Sections and their rows after the filter; a section with no match still shows its header.
    pub fn queue_rows(&self) -> Vec<QueueRow<'_>> {
        let Some(sections) = &self.sections else { return vec![] };
        let groups: [(&'static str, &Vec<QueueMr>); 5] = [
            ("TO REVIEW", &sections.to_review),
            ("MINE", &sections.mine),
            ("WATCHING", &sections.watching),
            ("OPEN", &sections.open),
            ("DONE", &sections.done),
        ];
        let mut rows = Vec::new();
        for (name, mrs) in groups {
            let open = !self.closed_sections.contains(name);
            let mut matching: Vec<&QueueMr> = mrs.iter().filter(|mr| self.matches_filter(mr)).collect();
            if name == "TO REVIEW" && self.triaged() {
                matching.sort_by(|a, b| self.urgency(b).total_cmp(&self.urgency(a)));
            }
            let unscoped = name == "OPEN" && mrs.is_empty();
            if unscoped || (name == "DONE" && matching.is_empty() && self.filter.is_empty()) {
                continue;
            }
            rows.push(QueueRow::Section { name, count: matching.len(), open });
            if open {
                rows.extend(matching.into_iter().map(QueueRow::Mr));
            }
        }
        rows
    }

    pub fn queue_is_empty(&self) -> bool {
        self.sections.as_ref().is_some_and(|s| s == &Sections::default())
    }

    fn matches_filter(&self, mr: &QueueMr) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        mr.title.to_lowercase().contains(&needle) || mr.author.to_lowercase().contains(&needle) || mr.number.to_string().contains(&needle)
    }

    pub fn selected_mr(&self) -> Option<&QueueMr> {
        match self.queue_rows().get(self.queue_selected) {
            Some(QueueRow::Mr(mr)) => Some(mr),
            _ => None,
        }
    }

    /// The row's host, named only when rows from several hosts share the queue.
    pub fn host_tag(&self, mr: &QueueMr) -> Option<String> {
        self.sections.as_ref().filter(|s| s.mixes_hosts()).and_then(|_| self.hosts.tag(&mr.key()))
    }

    pub fn badge(&self, mr: &QueueMr) -> Option<Badge> {
        let pipeline = mr.pipeline.as_deref().map(str::to_ascii_lowercase);
        let failed = mr.conflicts || pipeline.as_deref() == Some("failed");
        let running = matches!(pipeline.as_deref(), Some("running" | "pending" | "created" | "waiting_for_resource" | "preparing"));
        let activity = self.opened.get(&mr.key()).is_some_and(|opened| mr.updated_at > *opened);
        let approved = mr.approved_by.iter().any(|u| u == &self.me);
        [
            (failed, Badge::Failed),
            (running, Badge::Running),
            (activity, Badge::Activity),
            (approved, Badge::Approved),
            (mr.draft, Badge::Draft),
        ]
        .into_iter()
        .find_map(|(on, badge)| on.then_some(badge))
    }

    /// The cursor walks MR rows and the headers of folded sections, the only row such a section has.
    pub(super) fn queue_move(&mut self, delta: isize) {
        let selectable = self.queue_selectable();
        if selectable.is_empty() {
            self.queue_selected = 0;
            return;
        }
        let at = selectable.iter().position(|&i| i >= self.queue_selected).unwrap_or(selectable.len() - 1);
        let next = (at as isize + delta).clamp(0, selectable.len() as isize - 1) as usize;
        self.queue_selected = selectable[next];
    }

    fn queue_selectable(&self) -> Vec<usize> {
        self.queue_rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, QueueRow::Mr(_) | QueueRow::Section { open: false, .. }))
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn queue_first(&mut self) {
        self.queue_selected = 0;
        self.queue_move(0);
    }

    pub(super) fn queue_last(&mut self) {
        self.queue_selected = usize::MAX;
        self.queue_move(0);
    }

    /// After the rows changed under the cursor: land on the nearest row the cursor may take.
    pub(super) fn queue_settle(&mut self) {
        let len = self.queue_rows().len();
        self.queue_selected = self.queue_selected.min(len.saturating_sub(1));
        self.queue_move(0);
    }

    /// The section the cursor sits in: its header, or the header above its MR.
    fn section_here(&self) -> Option<&'static str> {
        self.queue_rows().into_iter().take(self.queue_selected + 1).rev().find_map(|row| match row {
            QueueRow::Section { name, .. } => Some(name),
            QueueRow::Mr(_) => None,
        })
    }

    /// `zo`, `zc`, `za` and `enter` on a header: open, close or flip the section under the cursor,
    /// and keep the cursor on its header when the section closes over it.
    pub(super) fn fold_section(&mut self, open: Option<bool>) {
        let Some(name) = self.section_here() else { return };
        let now_open = open.unwrap_or_else(|| self.closed_sections.contains(name));
        if now_open {
            self.closed_sections.remove(name);
        } else {
            self.closed_sections.insert(name);
            if let Some(header) = self.queue_rows().iter().position(|r| matches!(r, QueueRow::Section { name: n, .. } if *n == name)) {
                self.queue_selected = header;
            }
        }
    }

    pub(super) fn open_selected(&mut self) -> Vec<Action> {
        if matches!(self.queue_rows().get(self.queue_selected), Some(QueueRow::Section { .. })) {
            self.fold_section(None);
            return vec![];
        }
        let Some(mr) = self.selected_mr() else { return vec![] };
        let key = mr.key();
        self.open_key(key)
    }

    /// Shows the MR `key` in the review, fetching it unless it is the one already open.
    pub(super) fn open_key(&mut self, key: crate::forge::MrKey) -> Vec<Action> {
        if self.open.as_ref().is_some_and(|o| o.key == key) {
            self.focus = super::Focus::Review;
            return vec![];
        }
        self.opening = Some(key.clone());
        self.focus = super::Focus::Review;
        vec![Action::Open(key)]
    }
}
