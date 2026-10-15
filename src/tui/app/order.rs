//! How the queue lays out its rows inside a section: the order (`s`) and grouping by author (`S`),
//! remembered per scope so a repo keeps its own.
use crate::forge::QueueMr;
use serde::{Deserialize, Serialize};

/// The order of rows inside each section. `Updated` is the forge's own: newest activity first,
/// except To review, which Jev ranks by urgency when it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Updated,
    Oldest,
    Author,
    Size,
    Urgency,
}

impl Order {
    /// The next order `s` moves to; urgency only when Jev can rank.
    pub fn next(self, triaged: bool) -> Self {
        match self {
            Order::Updated => Order::Oldest,
            Order::Oldest => Order::Author,
            Order::Author => Order::Size,
            Order::Size if triaged => Order::Urgency,
            Order::Size | Order::Urgency => Order::Updated,
        }
    }

    /// The words the queue title carries; nothing for the default order.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Order::Updated => None,
            Order::Oldest => Some("oldest first"),
            Order::Author => Some("by author"),
            Order::Size => Some("smallest first"),
            Order::Urgency => Some("most urgent first"),
        }
    }
}

/// What `s` chose for one scope, which stacks are unfolded and which sections folded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueView {
    #[serde(default)]
    pub order: Order,
    /// Stacks shown MR by MR, by their base MR's key; every other stack is one folded row.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub open_stacks: std::collections::BTreeSet<String>,
    /// Sections shown as their header alone, by name.
    #[serde(default = "folded_at_first")]
    pub closed_sections: std::collections::BTreeSet<String>,
}

impl Default for QueueView {
    fn default() -> Self {
        Self { order: Order::default(), open_stacks: std::collections::BTreeSet::new(), closed_sections: folded_at_first() }
    }
}

/// One choice in the queue view, saved over the view the cache holds so a choice made in another revu stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    Order(Order),
    Section { name: String, closed: bool },
    Stack { id: String, open: bool },
}

impl ViewChange {
    /// What the change sets: of two changes to one target, the later one made wins.
    pub fn target(&self) -> String {
        match self {
            ViewChange::Order(_) => "order".into(),
            ViewChange::Section { name, .. } => format!("section {name}"),
            ViewChange::Stack { id, .. } => format!("stack {id}"),
        }
    }

    pub fn apply(self, view: QueueView) -> QueueView {
        match self {
            ViewChange::Order(order) => QueueView { order, ..view },
            ViewChange::Section { name, closed } => QueueView { closed_sections: with_member(view.closed_sections, name, closed), ..view },
            ViewChange::Stack { id, open } => QueueView { open_stacks: with_member(view.open_stacks, id, open), ..view },
        }
    }
}

/// `set` holding `item` when `on`, without it otherwise.
pub fn with_member<T: Ord>(mut set: std::collections::BTreeSet<T>, item: T, on: bool) -> std::collections::BTreeSet<T> {
    if on {
        set.insert(item);
    } else {
        set.remove(&item);
    }
    set
}

/// What the reader rarely needs, until they fold or open a section themselves.
fn folded_at_first() -> std::collections::BTreeSet<String> {
    ["DONE", "DRAFTS", "OTHER"].map(str::to_owned).into()
}

/// `rows` in `order`; `urgency` scores an MR when Jev ranked it. Sorts are stable, so rows that
/// tie keep the forge's order: newest activity first.
pub fn sorted(mut rows: Vec<&QueueMr>, order: Order, urgency: impl Fn(&QueueMr) -> f64) -> Vec<&QueueMr> {
    match order {
        Order::Updated => {}
        Order::Oldest => rows.sort_by_key(|mr| mr.created_at),
        Order::Author => rows.sort_by_key(|mr| mr.author.to_lowercase()),
        Order::Size => rows.sort_by_key(|mr| mr.additions + mr.deletions),
        Order::Urgency => rows.sort_by(|a, b| urgency(b).total_cmp(&urgency(a))),
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;
    use chrono::{TimeZone, Utc};

    fn mr(number: u64, author: &str, created: i64, size: u32) -> QueueMr {
        let at = Utc.timestamp_opt(1_790_000_000 + created, 0).unwrap();
        QueueMr {
            host: None,
            number,
            project: "acme/widgets".into(),
            title: format!("mr {number}"),
            description: String::new(),
            draft: false,
            web_url: String::new(),
            updated_at: at,
            created_at: at,
            source_branch: String::new(),
            target_branch: String::new(),
            conflicts: false,
            author: author.into(),
            author_name: author.into(),
            approved: false,
            approved_by: vec![],
            approvals_left: None,
            reviewers: vec![],
            pipeline: None,
            additions: size,
            deletions: 0,
            files: 1,
            unresolved: 0,
            labels: vec![],
            notes: 0,
            commenters: vec![],
            reason: None,
        }
    }

    fn numbers(rows: &[&QueueMr]) -> Vec<u64> {
        rows.iter().map(|mr| mr.number).collect()
    }

    #[test]
    fn a_change_sets_one_choice_and_leaves_the_others() {
        let view = ViewChange::Order(Order::Author).apply(QueueView::default());
        let view = ViewChange::Section { name: "DONE".into(), closed: false }.apply(view);
        let view = ViewChange::Stack { id: "acme/widgets!61".into(), open: true }.apply(view);
        let view = ViewChange::Section { name: "OPEN".into(), closed: true }.apply(view);
        assert_eq!(view.order, Order::Author);
        assert_eq!(view.closed_sections, ["DRAFTS", "OPEN", "OTHER"].map(str::to_owned).into());
        assert_eq!(view.open_stacks, ["acme/widgets!61".to_owned()].into());
        let closed_again = ViewChange::Stack { id: "acme/widgets!61".into(), open: false }.apply(view);
        assert!(closed_again.open_stacks.is_empty());
        assert_eq!(
            ViewChange::Stack { id: "acme/widgets!61".into(), open: false }.target(),
            ViewChange::Stack { id: "acme/widgets!61".into(), open: true }.target()
        );
    }

    #[test]
    fn s_cycles_through_the_orders_and_offers_urgency_only_with_jev() {
        use Order::{Author, Oldest, Size, Updated, Urgency};
        let walk = |triaged| {
            let mut order = Updated;
            (0..6)
                .map(|_| {
                    let now = order;
                    order = order.next(triaged);
                    now
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(walk(false), [Updated, Oldest, Author, Size, Updated, Oldest]);
        assert_eq!(walk(true), [Updated, Oldest, Author, Size, Urgency, Updated]);
        assert_eq!(Updated.label(), None);
        assert_eq!(Author.label(), Some("by author"));
    }

    #[test]
    fn each_order_sorts_and_ties_keep_the_forge_order() {
        let (a, b, c) = (mr(1, "zoe", 30, 50), mr(2, "Ana", 10, 5), mr(3, "ana", 20, 50));
        let rows = vec![&a, &b, &c];
        let none = |_: &QueueMr| 0.0;
        assert_eq!(numbers(&sorted(rows.clone(), Order::Updated, none)), [1, 2, 3]);
        assert_eq!(numbers(&sorted(rows.clone(), Order::Oldest, none)), [2, 3, 1]);
        assert_eq!(numbers(&sorted(rows.clone(), Order::Author, none)), [2, 3, 1], "case does not split an author");
        assert_eq!(numbers(&sorted(rows.clone(), Order::Size, none)), [2, 1, 3]);
        let urgent = |mr: &QueueMr| if mr.number == 3 { 2.5 } else { 0.5 };
        assert_eq!(numbers(&sorted(rows, Order::Urgency, urgent)), [3, 1, 2]);
    }

    #[test]
    fn s_cycles_the_order_saves_it_for_the_scope_and_titles_the_pane() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        let actions = press(&mut app, "s");
        let [Action::ChangeQueueView { scope, change, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((scope.as_ref().map(ProjectPath::as_str), change), (Some("acme/widgets"), &ViewChange::Order(Order::Oldest)));
        press(&mut app, "ss");
        assert_eq!(app.queue_view.order, Order::Size);
        assert_eq!(app.queue_view_label().as_deref(), Some("smallest first"));
        press(&mut app, "s");
        assert_eq!(app.queue_view.order, Order::Updated, "no urgency without Jev: back to the start");
        assert_eq!(app.queue_view_label(), None);
        let screen = render(&mut app, 120, 20);
        assert!(screen.contains("Queue · acme/widgets "), "{screen}");
    }

    #[test]
    fn a_saved_view_for_this_scope_applies_and_one_for_another_is_dropped() {
        let mut app = scoped_app();
        let grouped = QueueView { order: Order::Author, ..QueueView::default() };
        app.apply(Incoming::QueueView { scope: None, view: grouped.clone() });
        assert_eq!(app.queue_view, QueueView::default());
        app.apply(Incoming::QueueView { scope: Some("acme/widgets".into()), view: grouped.clone() });
        assert_eq!(app.queue_view, grouped);
    }
}
