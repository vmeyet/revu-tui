use super::order::{self, Order};
use super::stack::{self, Item};
use super::{Action, App};
use crate::forge::{PipelineStatus, QueueMr, Sections};

/// The one glyph at the right edge of a queue row, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Badge {
    Failed,
    Running,
    Activity,
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
        let groups: [(&'static str, &Vec<QueueMr>); 8] = [
            ("TO REVIEW", &sections.to_review),
            ("MINE", &sections.mine),
            ("READY", &sections.ready),
            ("WATCHING", &sections.watching),
            ("OPEN", &sections.open),
            ("DRAFTS", &sections.drafts),
            ("DONE", &sections.done),
            ("OTHER", &sections.other),
        ];
        let mut rows = Vec::new();
        for (name, mrs) in groups {
            let open = !self.queue_view.closed_sections.contains(name);
            let matching = self.in_order(name, mrs.iter().filter(|mr| self.matches_filter(mr)).collect());
            let only_when_filled = matches!(name, "READY" | "OPEN" | "DRAFTS" | "OTHER") && mrs.is_empty();
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
    pub(super) fn in_order<'m>(&self, section: &str, rows: Vec<&'m QueueMr>) -> Vec<&'m QueueMr> {
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

    pub(super) fn matches_filter(&self, mr: &QueueMr) -> bool {
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

    /// A stack carries the approval mark when every one of its MRs does.
    pub fn stack_approved(&self, mrs: &[&QueueMr]) -> bool {
        !mrs.is_empty() && mrs.iter().all(|mr| self.approved(mr))
    }

    /// The approval mark, beside the badge: on my MR, the forge would let it merge; on anyone
    /// else's, I approved it. An MR no rule guards counts as approved, so mine also needs an approver.
    pub fn approved(&self, mr: &QueueMr) -> bool {
        if mr.author == self.me { mr.approved && !mr.approved_by.is_empty() } else { mr.approved_by.contains(&self.me) }
    }

    /// The row's host, named only when rows from several hosts share the queue.
    pub fn host_tag(&self, mr: &QueueMr) -> Option<String> {
        self.sections.as_ref().filter(|s| s.mixes_hosts()).and_then(|_| self.hosts.tag(&mr.key()))
    }

    pub fn badge(&self, mr: &QueueMr) -> Option<Badge> {
        let failed = mr.conflicts || mr.pipeline == Some(PipelineStatus::Failed);
        let running = mr.pipeline.is_some_and(PipelineStatus::is_running);
        let activity = self.opened.get(&mr.key()).is_some_and(|opened| mr.updated_at > *opened);
        [(failed, Badge::Failed), (running, Badge::Running), (activity, Badge::Activity), (mr.draft, Badge::Draft)]
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
    pub(super) fn fold_section(&mut self, open: Option<bool>) -> Vec<Action> {
        let Some(name) = self.section_here() else { return vec![] };
        let now_open = open.unwrap_or_else(|| self.queue_view.closed_sections.contains(name));
        if now_open {
            self.queue_view.closed_sections.remove(name);
        } else {
            self.queue_view.closed_sections.insert(name.to_owned());
        }
        if let Some(header) = self.queue_rows().iter().position(|r| matches!(r, QueueRow::Section { name: n, .. } if *n == name)) {
            self.queue_selected = header;
        }
        self.queue_move(0);
        self.save_queue_view()
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
            return self.fold_section(None);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn the_queue_lands_in_sections_and_the_cursor_on_the_first_mr() {
        let app = with_queue();
        assert!(!app.queue_loading);
        assert!(app.poll.queue_due.is_some());
        assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
        let rows = app.queue_rows();
        assert!(matches!(rows[0], QueueRow::Section { name: "TO REVIEW", count: 1, open: true }));
        assert!(rows.iter().any(|r| matches!(r, QueueRow::Section { name: "DONE", open: false, .. })));
    }

    #[test]
    fn j_and_k_skip_section_headers_and_stop_at_the_ends() {
        let mut app = with_queue();
        press(&mut app, "j");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(41), "MINE header is skipped");
        press(&mut app, "kkk");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
        press(&mut app, "G");
        assert!(app.selected_mr().is_none(), "the last row is the folded Done header");
        press(&mut app, "k");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
        press(&mut app, "g");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
    }

    #[test]
    fn zo_and_zc_fold_the_section_under_the_cursor() {
        let mut app = with_queue();
        press(&mut app, "G");
        assert!(app.selected_mr().is_none(), "the folded Done header takes the cursor");
        press(&mut app, "zo");
        assert!(!app.queue_view.closed_sections.contains("DONE"));
        press(&mut app, "G");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(40));
        press(&mut app, "zc");
        assert!(app.queue_view.closed_sections.contains("DONE"));
        assert!(matches!(app.queue_rows()[app.queue_selected], QueueRow::Section { name: "DONE", .. }), "the cursor stays on its header");
        press(&mut app, "g");
        press(&mut app, "zc");
        assert!(app.queue_view.closed_sections.contains("TO REVIEW"), "any section folds, not only Done");
        assert!(matches!(app.queue_rows()[app.queue_selected], QueueRow::Section { name: "TO REVIEW", .. }));
        app.handle_key(code(KeyCode::Enter));
        assert!(!app.queue_view.closed_sections.contains("TO REVIEW"), "enter on a folded header opens it");
    }

    #[test]
    fn a_folded_section_is_saved_and_comes_back_folded() {
        let mut app = with_queue();
        let actions = press(&mut app, "gzc");
        let [Action::SaveQueueView { view, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert!(view.closed_sections.contains("TO REVIEW"));
        let mut next = with_queue();
        next.apply(Incoming::QueueView { scope: next.scope(), view: view.clone() });
        assert!(
            next.queue_rows().iter().any(|r| matches!(r, QueueRow::Section { name: "TO REVIEW", open: false, .. })),
            "folded after a restart"
        );
    }

    #[test]
    fn a_view_saved_before_folds_were_kept_folds_done_drafts_and_other() {
        let old: QueueView = serde_json::from_str(r#"{"order":"updated","by_author":true}"#).unwrap();
        assert_eq!(old.closed_sections, QueueView::default().closed_sections);
        assert!(old.closed_sections.contains("DONE") && !old.closed_sections.contains("OPEN"));
    }

    #[test]
    fn the_filter_narrows_live_and_esc_clears_it() {
        let mut app = with_queue();
        press(&mut app, "/runner");
        assert!(app.filtering);
        assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
        app.handle_key(code(KeyCode::Enter));
        assert!(!app.filtering && app.filter == "runner");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.filter, "");
        press(&mut app, "/omar");
        let iids: Vec<u64> = app.queue_rows().iter().filter_map(|r| if let QueueRow::Mr(m) = r { Some(m.number) } else { None }).collect();
        assert_eq!(iids, [42, 35], "author matches too");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.filter.is_empty() && !app.filtering);
    }

    #[test]
    fn badges_follow_the_spec_order() {
        let mut app = with_queue();
        assert_eq!(app.badge(&queued(40)), Some(Badge::Failed));
        assert_eq!(app.badge(&queued(35)), Some(Badge::Running));
        assert_eq!(app.badge(&queued(42)), None);
        app.opened.insert(mr_key(), "2026-09-21T00:00:00Z".parse().unwrap());
        assert_eq!(app.badge(&queued(42)), Some(Badge::Activity), "updated after it was last opened");
        app.opened.insert(mr_key(), today());
        assert_eq!(app.badge(&queued(42)), None);
    }

    fn queued(iid: u64) -> crate::forge::QueueMr {
        let sections = sections();
        [sections.to_review, sections.mine, sections.watching, sections.done].into_iter().flatten().find(|m| m.number == iid).unwrap()
    }

    #[test]
    fn my_mr_carries_the_approval_mark_once_the_forge_would_merge_it() {
        let app = with_queue();
        let mine = queued(41);
        assert_eq!(mine.author, app.me);
        assert!(!app.approved(&mine), "not approved");
        assert!(app.approved(&crate::forge::QueueMr { approved: true, approved_by: vec!["lea".into()], ..mine.clone() }));
        assert!(!app.approved(&crate::forge::QueueMr { approved: true, ..mine }), "no approval rule and nobody approved");
    }

    #[test]
    fn someone_elses_mr_carries_the_approval_mark_when_i_approved_it() {
        let app = with_queue();
        let theirs = queued(42);
        assert!(!app.approved(&theirs));
        assert!(
            !app.approved(&crate::forge::QueueMr { approved: true, approved_by: vec!["lea".into()], ..theirs.clone() }),
            "approved, not by me"
        );
        assert!(app.approved(&crate::forge::QueueMr { approved_by: vec!["nina".into()], ..theirs }));
    }

    #[test]
    fn a_failed_mr_i_approved_shows_both_marks() {
        let mut app = with_queue();
        let failed = queued(40);
        assert_eq!((app.badge(&failed), app.approved(&failed)), (Some(Badge::Failed), true));
        press(&mut app, "Gzo");
        let screen = render(&mut app, 100, 20);
        assert!(screen.contains("✓ ✗ │"), "{screen}");
    }

    #[test]
    fn in_a_checkout_the_queue_starts_on_its_project_and_star_widens_it() {
        let mut app = scoped_app();
        let scope = Some("acme/widgets".to_owned());
        assert_eq!(app.start(), vec![Action::LoadQueue { scope: scope.clone(), from_cache: true }]);
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        assert!(app.queue_rows().iter().any(|r| matches!(r, QueueRow::Section { name: "OPEN", count: 1, .. })));
        assert!(app.queue_rows().iter().any(|r| matches!(r, QueueRow::Section { name: "DRAFTS", count: 1, open: false })));
        assert_eq!(press(&mut app, "*"), vec![Action::LoadQueue { scope: None, from_cache: true }]);
        assert_eq!(app.sections, None, "the project's list is gone before the wider one paints");
        assert_eq!(press(&mut app, "*"), vec![Action::LoadQueue { scope, from_cache: true }]);
    }

    #[test]
    fn an_answer_for_the_other_scope_is_dropped_and_a_cached_one_never_hides_a_fresh_one() {
        let mut app = scoped_app();
        app.apply(queue_answer(None, sections(), false));
        assert_eq!(app.sections, None, "the answer to a scope we left");
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), true));
        assert!(app.sections.is_some() && app.queue_loading, "the cache paints while the fetch runs");
        app.apply(queue_answer(Some("acme/widgets"), Sections::default(), false));
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), true));
        assert_eq!(app.sections, Some(Sections::default()), "a late cache answer never replaces a fresh one");
        assert!(!app.queue_loading);
    }

    #[test]
    fn star_outside_a_checkout_says_why_it_does_nothing() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "*"), vec![]);
        assert!(app.live_toast().unwrap().text.contains("checkout"));
    }

    #[test]
    fn the_queue_names_me_when_no_login_did() {
        let mut app = App::new(Settings { me: String::new(), ..settings() });
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        assert_eq!(app.me, "nina");
        app.apply(Incoming::Queue { scope: None, me: "someone".into(), sections: sections(), opened: HashMap::new(), cached: false });
        assert_eq!(app.me, "nina", "a stored name is never replaced");
    }

    #[test]
    fn a_queue_across_hosts_tags_each_row_and_opens_it_on_its_host() {
        let hosts = crate::forge::Hosts {
            others: vec![("github.com".into(), Kind::GitHub)],
            ..crate::forge::Hosts::one("gitlab.com", Kind::GitLab)
        };
        let mut app = App::new(Settings { hosts, ..settings() });
        let here = sections();
        let queue = fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json"));
        let there = queue.on_host("github.com").sections(&[]);
        let merged = crate::forge::Sections::merge(vec![here, there]);
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: merged, opened: HashMap::new(), cached: false });
        let screen = render(&mut app, 120, 40);
        assert!(screen.contains("#42") && screen.contains("!42"), "the sigil tells the forges apart:\n{screen}");
        app.queue_layout = crate::config::QueueLayout::Compact;
        let screen = render(&mut app, 120, 20);
        assert!(screen.contains("github") && screen.contains("gitlab"), "{screen}");
        let github_row = app.queue_rows().iter().position(|r| matches!(r, QueueRow::Mr(mr) if mr.host.is_some())).unwrap();
        app.queue_selected = github_row;
        let actions = app.handle_key(code(KeyCode::Enter));
        let [Action::Open(key)] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(key.host.as_deref(), Some("github.com"));
    }

    fn two_hosts() -> crate::forge::Hosts {
        crate::forge::Hosts { others: vec![("github.com".into(), Kind::GitHub)], ..crate::forge::Hosts::one("gitlab.com", Kind::GitLab) }
    }

    #[test]
    fn inside_a_checkout_rows_carry_no_host_tag_even_with_two_hosts_logged_in() {
        let mut app = App::new(Settings { project: Some("acme/widgets".into()), hosts: two_hosts(), ..settings() });
        app.today = today();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        let mr = app.sections.as_ref().unwrap().open[0].clone();
        assert_eq!(app.host_tag(&mr), None);
        let screen = render(&mut app, 120, 20);
        assert!(!screen.contains("gitlab "), "{screen}");
    }

    #[test]
    fn the_merged_queue_tags_each_row_with_its_host() {
        let mut app = App::new(Settings { hosts: two_hosts(), ..settings() });
        app.today = today();
        let mut merged = sections();
        let mut there = merged.to_review[0].clone();
        there.host = Some("github.com".into());
        there.number = 7;
        merged.mine.push(there.clone());
        app.apply(queue_answer(None, merged, false));
        assert_eq!(app.host_tag(&there).as_deref(), Some("github"));
        let here = app.sections.as_ref().unwrap().to_review[0].clone();
        assert_eq!(app.host_tag(&here).as_deref(), Some("gitlab"));
    }

    #[test]
    fn others_drafts_wait_folded_in_their_own_section_and_mine_stay_mine() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        let rows = app.queue_rows();
        let drafts = rows.iter().position(|r| matches!(r, QueueRow::Section { name: "DRAFTS", open: false, count: 1 })).unwrap();
        let done = rows.iter().position(|r| matches!(r, QueueRow::Section { name: "DONE", .. })).unwrap();
        let open = rows.iter().position(|r| matches!(r, QueueRow::Section { name: "OPEN", .. })).unwrap();
        assert!(open < drafts && drafts < done);
        assert!(rows.iter().any(|r| matches!(r, QueueRow::Mr(mr) if mr.number == 41 && mr.draft)), "my own draft is in Mine");
    }

    #[test]
    fn go_and_the_jump_reach_a_draft_folded_away_in_drafts() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        assert_eq!(app.completions_for("go "), ["!42", "!41", "!51", "!50", "!40"]);
        press(&mut app, ":");
        type_text(&mut app, "go !50");
        assert_eq!(app.opening, Some(MrKey::new("acme/widgets", 50)));
    }

    #[test]
    fn the_queue_filter_speaks_the_query_language_and_esc_clears_it() {
        let mut app = with_queue();
        press(&mut app, "/");
        type_text(&mut app, "@omar is:failing");
        let shown: Vec<u64> =
            app.queue_rows().iter().filter_map(|r| if let QueueRow::Mr(mr) = r { Some(mr.number) } else { None }).collect();
        assert!(shown.iter().all(|n| [42, 35].contains(n)), "{shown:?}");
        assert_eq!(app.queue_view_label().as_deref(), Some("@omar is:failing"));
        press(&mut app, "/");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.filter.is_empty() && app.queue_view_label().is_none());
    }

    fn with_views() -> App {
        let views = vec![("backlog".to_owned(), "size:small".to_owned()), ("omar".to_owned(), "@omar".to_owned())];
        let mut app = App::new(Settings { views, ..settings() });
        app.today = today();
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        app
    }

    #[test]
    fn quote_then_a_letter_or_a_digit_applies_a_saved_view() {
        let mut app = with_views();
        press(&mut app, "'");
        assert!(app.views_hint().contains("b backlog · o omar"), "{}", app.views_hint());
        press(&mut app, "o");
        assert_eq!((app.filter.as_str(), app.view.as_deref()), ("@omar", Some("omar")));
        assert_eq!(app.queue_view_label().as_deref(), Some("omar"), "the title names the view");
        press(&mut app, "1");
        assert_eq!(app.view.as_deref(), Some("backlog"), "digits take views in name order");
        press(&mut app, "'z");
        assert!(app.live_toast().unwrap().text.contains("no view on `z`"));
        app.handle_key(code(KeyCode::Esc));
        assert!(app.filter.is_empty() && app.view.is_none());
    }

    #[test]
    fn the_status_line_lists_views_while_quote_waits() {
        let mut app = with_views();
        press(&mut app, "'");
        let screen = render(&mut app, 100, 12);
        assert!(screen.lines().last().unwrap().contains("' views: b backlog · o omar"), "{screen}");
    }

    #[test]
    fn what_the_rules_move_out_waits_folded_in_other_and_reviewed_mrs_sort_last() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), ruled_sections(), false));
        let rows = app.queue_rows();
        assert!(rows.iter().any(|r| matches!(r, QueueRow::Section { name: "OTHER", count: 1, open: false })));
        let open: Vec<u64> = rows
            .iter()
            .skip_while(|r| !matches!(r, QueueRow::Section { name: "OPEN", .. }))
            .skip(1)
            .take_while(|r| matches!(r, QueueRow::Mr(_)))
            .map(|r| match r {
                QueueRow::Mr(mr) => mr.number,
                _ => 0,
            })
            .collect();
        assert_eq!(open.last(), Some(&52), "reviewed by others sorts last: {open:?}");
        assert!(open.contains(&53));
    }

    #[test]
    fn the_status_line_says_why_the_selected_mr_sits_there() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), ruled_sections(), false));
        let at = app.queue_rows().iter().position(|r| matches!(r, QueueRow::Mr(mr) if mr.number == 52)).unwrap();
        app.queue_selected = at;
        assert_eq!(app.selected_reason().as_deref(), Some("reviewed by 2"));
        assert!(render(&mut app, 120, 30).lines().last().unwrap().contains("reviewed by 2"));
        app.queue_selected = app.queue_rows().iter().position(|r| matches!(r, QueueRow::Mr(mr) if mr.number == 53)).unwrap();
        assert_eq!(app.selected_reason(), None);
    }

    #[test]
    fn ready_sits_right_after_mine_and_a_failing_command_only_warns() {
        let mut app = scoped_app();
        let sections = scoped_sections();
        let picked = sections.open.iter().find(|mr| !mr.draft).cloned().unwrap();
        let sections = Sections {
            ready: vec![picked.clone()],
            open: sections.open.iter().filter(|mr| mr.number != picked.number).cloned().collect(),
            ..sections
        };
        app.apply(queue_answer(Some("acme/widgets"), sections, false));
        let names: Vec<&str> = app
            .queue_rows()
            .iter()
            .filter_map(|r| match r {
                QueueRow::Section { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        let at = |name| names.iter().position(|n| *n == name).unwrap();
        assert_eq!(at("READY"), at("MINE") + 1);
        app.apply(Incoming::Failed { what: Failure::Ready, message: "ready command `slack` failed: not logged in".into() });
        assert!(app.live_toast().unwrap().text.contains("not logged in"));
        assert!(app.sections.as_ref().is_some_and(|s| s.ready.len() == 1), "Ready keeps its last answer");
    }
}
