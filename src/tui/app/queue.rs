use super::order::{self, Order};
use super::stack::{self, Item};
use super::{Action, App};
use crate::forge::{QueueMr, Sections};

/// The one glyph at the right edge of a queue row, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Badge {
    Failed,
    Running,
    Activity,
    Approved,
    Draft,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueueRow<'a> {
    Section {
        name: &'static str,
        count: usize,
        open: bool,
    },
    /// One author's MRs follow, when `S` groups Open and Drafts.
    Author {
        name: &'a str,
        count: usize,
    },
    Mr(&'a QueueMr),
    /// A chain of one author's MRs, each built on the one below; folded, it is this one row.
    Stack {
        id: String,
        mrs: Vec<&'a QueueMr>,
        open: bool,
    },
    /// A member of an unfolded stack, drawn under its stack row.
    Stacked(&'a QueueMr),
}

/// The sections `S` splits by author: the ones that pile up other people's MRs.
const GROUPED: [&str; 2] = ["OPEN", "DRAFTS"];

impl App {
    /// Sections and their rows after the filter, in the chosen order; a section with no match
    /// still shows its header, except the ones that only exist when they hold something.
    pub fn queue_rows(&self) -> Vec<QueueRow<'_>> {
        let Some(sections) = &self.sections else { return vec![] };
        let groups: [(&'static str, &Vec<QueueMr>); 7] = [
            ("TO REVIEW", &sections.to_review),
            ("MINE", &sections.mine),
            ("WATCHING", &sections.watching),
            ("OPEN", &sections.open),
            ("DRAFTS", &sections.drafts),
            ("DONE", &sections.done),
            ("OTHER", &sections.other),
        ];
        let mut rows = Vec::new();
        for (name, mrs) in groups {
            let open = !self.closed_sections.contains(name);
            let matching = self.in_order(name, mrs.iter().filter(|mr| self.matches_filter(mr)).collect());
            let only_when_filled = matches!(name, "OPEN" | "DRAFTS" | "OTHER") && mrs.is_empty();
            if only_when_filled || (name == "DONE" && matching.is_empty() && self.filter.is_empty()) {
                continue;
            }
            rows.push(QueueRow::Section { name, count: matching.len(), open });
            if !open {
                continue;
            }
            if self.queue_view.by_author && GROUPED.contains(&name) {
                for (author, group) in order::by_author(matching) {
                    rows.push(QueueRow::Author { name: author, count: group.len() });
                    rows.extend(self.stacked(&group));
                }
            } else {
                rows.extend(self.stacked(&matching));
            }
        }
        rows
    }

    /// `mrs` as rows, each chain as one stack row, followed by its MRs when it is unfolded.
    fn stacked<'m>(&self, mrs: &[&'m QueueMr]) -> Vec<QueueRow<'m>> {
        let mut rows = vec![];
        for item in stack::group(mrs) {
            match item {
                Item::Single(mr) => rows.push(QueueRow::Mr(mr)),
                Item::Stack(stack) => {
                    let open = self.queue_view.open_stacks.contains(&stack.id);
                    let members: Vec<&QueueMr> = if open { stack.mrs.clone() } else { vec![] };
                    rows.push(QueueRow::Stack { id: stack.id, mrs: stack.mrs, open });
                    rows.extend(members.into_iter().map(QueueRow::Stacked));
                }
            }
        }
        rows
    }

    /// A section's rows in the chosen order, what others already review last. With the default
    /// order Jev still ranks To review.
    fn in_order<'m>(&self, section: &str, rows: Vec<&'m QueueMr>) -> Vec<&'m QueueMr> {
        let order = match self.queue_view.order {
            Order::Updated if section == "TO REVIEW" && self.triaged() => Order::Urgency,
            order => order,
        };
        let (fresh, reviewed): (Vec<_>, Vec<_>) = order::sorted(rows, order, |mr| self.urgency(mr))
            .into_iter()
            .partition(|mr| mr.reason.as_ref().is_none_or(crate::forge::rules::Reason::moves_out));
        fresh.into_iter().chain(reviewed).collect()
    }

    /// Why the selected MR sits where it does, when a "needs me" rule placed it: the status line says it.
    pub fn selected_reason(&self) -> Option<String> {
        self.selected_mr()?.reason.as_ref().map(ToString::to_string)
    }

    /// `s`: the next order; `S`: grouping by author on or off. Both are remembered for this scope.
    pub(super) fn sort_queue(&mut self) -> Vec<Action> {
        self.queue_view.order = self.queue_view.order.next(self.triaged());
        self.save_queue_view()
    }

    pub(super) fn group_queue(&mut self) -> Vec<Action> {
        self.queue_view.by_author = !self.queue_view.by_author;
        self.save_queue_view()
    }

    fn save_queue_view(&mut self) -> Vec<Action> {
        self.queue_settle();
        vec![Action::SaveQueueView { scope: self.scope(), view: self.queue_view.clone() }]
    }

    /// What the queue title says after the scope: the filter (or the view that set it), the
    /// order, and grouping.
    pub fn queue_view_label(&self) -> Option<String> {
        let filter = if self.filter.is_empty() { None } else { Some(self.view.clone().unwrap_or_else(|| self.filter.clone())) };
        let order = self.queue_view.order.label().map(str::to_owned);
        let grouped = self.queue_view.by_author.then(|| "grouped by author".to_owned());
        let words: Vec<String> = filter.into_iter().chain(order).chain(grouped).collect();
        (!words.is_empty()).then(|| words.join(", "))
    }

    pub fn queue_is_empty(&self) -> bool {
        self.sections.as_ref().is_some_and(|s| s == &Sections::default())
    }

    fn matches_filter(&self, mr: &QueueMr) -> bool {
        self.filter.is_empty() || crate::query::Query::lenient(&self.filter).matches(mr, &self.me)
    }

    /// `'` then a letter, or a digit: the saved view of that first letter, or that rank, filters
    /// the queue; the title names it. Anything else leaves the queue as it was.
    pub(super) fn apply_view(&mut self, pick: char) {
        let chosen = match pick.to_digit(10) {
            Some(rank @ 1..=9) => self.views.get(rank as usize - 1),
            _ => self.views.iter().find(|(name, _)| name.chars().next().is_some_and(|c| c.eq_ignore_ascii_case(&pick))),
        };
        let Some((name, query)) = chosen.cloned() else {
            self.toast(format!("no view on `{pick}` · [queue.views] in the config"));
            return;
        };
        self.filter = query;
        self.view = Some(name);
        self.queue_settle();
    }

    /// What `'` offers while it waits for its letter: `f front · b backlog`.
    pub fn views_hint(&self) -> String {
        let listed: Vec<String> =
            self.views.iter().filter_map(|(name, _)| name.chars().next().map(|first| format!("{first} {name}"))).collect();
        if listed.is_empty() { "no saved views: [queue.views] in the config".into() } else { format!("views: {}", listed.join(" · ")) }
    }

    pub fn selected_mr(&self) -> Option<&QueueMr> {
        match self.queue_rows().get(self.queue_selected) {
            Some(QueueRow::Mr(mr) | QueueRow::Stacked(mr)) => Some(mr),
            _ => None,
        }
    }

    /// A stack's badge: the most pressing of its MRs' badges.
    pub fn stack_badge(&self, mrs: &[&QueueMr]) -> Option<Badge> {
        mrs.iter().filter_map(|mr| self.badge(mr)).min()
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
            .filter(|(_, r)| {
                matches!(r, QueueRow::Mr(_) | QueueRow::Stack { .. } | QueueRow::Stacked(_) | QueueRow::Section { open: false, .. })
            })
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
            QueueRow::Author { .. } | QueueRow::Mr(_) | QueueRow::Stack { .. } | QueueRow::Stacked(_) => None,
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
        }
        if let Some(header) = self.queue_rows().iter().position(|r| matches!(r, QueueRow::Section { name: n, .. } if *n == name)) {
            self.queue_selected = header;
        }
        self.queue_move(0);
    }

    /// The stack under the cursor: its row, or the row of the stack an unfolded MR belongs to.
    fn stack_here(&self) -> Option<(usize, String)> {
        let rows = self.queue_rows();
        match rows.get(self.queue_selected)? {
            QueueRow::Stack { id, .. } => Some((self.queue_selected, id.clone())),
            QueueRow::Stacked(_) => rows[..self.queue_selected].iter().enumerate().rev().find_map(|(i, row)| match row {
                QueueRow::Stack { id, .. } => Some((i, id.clone())),
                _ => None,
            }),
            _ => None,
        }
    }

    /// `zo`, `zc`, `za` and `enter` on a stack: unfold it MR by MR, or fold it back into its row,
    /// the cursor on that row. `None` when the cursor is on no stack, so the section folds instead.
    pub(super) fn fold_stack(&mut self, open: Option<bool>) -> Option<Vec<Action>> {
        let (row, id) = self.stack_here()?;
        let now_open = open.unwrap_or_else(|| !self.queue_view.open_stacks.contains(&id));
        if now_open {
            self.queue_view.open_stacks.insert(id);
        } else {
            self.queue_view.open_stacks.remove(&id);
            self.queue_selected = row;
        }
        Some(self.save_queue_view())
    }

    pub(super) fn open_selected(&mut self) -> Vec<Action> {
        if matches!(self.queue_rows().get(self.queue_selected), Some(QueueRow::Section { .. })) {
            self.fold_section(None);
            return vec![];
        }
        if matches!(self.queue_rows().get(self.queue_selected), Some(QueueRow::Stack { .. })) {
            return self.fold_stack(None).unwrap_or_default();
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
