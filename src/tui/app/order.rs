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

/// What `s` and `S` chose for one scope, which stacks are unfolded and which sections folded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueView {
    #[serde(default)]
    pub order: Order,
    /// Open and Drafts get one sub-header per author.
    #[serde(default)]
    pub by_author: bool,
    /// Stacks shown MR by MR, by their base MR's key; every other stack is one folded row.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub open_stacks: std::collections::BTreeSet<String>,
    /// Sections shown as their header alone, by name.
    #[serde(default = "folded_at_first")]
    pub closed_sections: std::collections::BTreeSet<String>,
}

impl Default for QueueView {
    fn default() -> Self {
        Self {
            order: Order::default(),
            by_author: false,
            open_stacks: std::collections::BTreeSet::new(),
            closed_sections: folded_at_first(),
        }
    }
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

/// `rows` in runs of one author each, authors by name, the order kept inside each run.
pub fn by_author(mut rows: Vec<&QueueMr>) -> Vec<(&str, Vec<&QueueMr>)> {
    rows.sort_by_key(|mr| mr.author.to_lowercase());
    let mut groups: Vec<(&str, Vec<&QueueMr>)> = vec![];
    for mr in rows {
        match groups.last_mut() {
            Some((author, group)) if author.eq_ignore_ascii_case(&mr.author) => group.push(mr),
            _ => groups.push((mr.author.as_str(), vec![mr])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
    fn s_cycles_through_the_orders_and_offers_urgency_only_with_jev() {
        use Order::{Author, Oldest, Size, Updated, Urgency};
        let walk = |triaged| {
            let mut order = Order::Updated;
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
    fn grouping_makes_one_run_per_author_in_name_order_keeping_the_sort_inside() {
        let (a, b, c, d) = (mr(1, "zoe", 0, 0), mr(2, "ana", 0, 0), mr(3, "zoe", 0, 0), mr(4, "Ana", 0, 0));
        let groups = by_author(vec![&a, &b, &c, &d]);
        let shape: Vec<(&str, Vec<u64>)> = groups.iter().map(|(name, rows)| (*name, numbers(rows))).collect();
        assert_eq!(shape, [("ana", vec![2, 4]), ("zoe", vec![1, 3])]);
        assert_eq!(by_author(vec![]), [] as [(&str, Vec<&QueueMr>); 0]);
    }

    #[test]
    fn s_cycles_the_order_saves_it_for_the_scope_and_titles_the_pane() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        let actions = press(&mut app, "s");
        let view = QueueView { order: order::Order::Oldest, ..QueueView::default() };
        assert_eq!(actions, vec![Action::SaveQueueView { scope: Some("acme/widgets".into()), view }]);
        press(&mut app, "ss");
        assert_eq!(app.queue_view.order, order::Order::Size);
        assert_eq!(app.queue_view_label().as_deref(), Some("smallest first"));
        press(&mut app, "s");
        assert_eq!(app.queue_view.order, order::Order::Updated, "no urgency without Jev: back to the start");
        assert_eq!(app.queue_view_label(), None);
        let screen = render(&mut app, 120, 20);
        assert!(screen.contains("Queue · acme/widgets "), "{screen}");
    }

    #[test]
    fn capital_s_groups_open_and_drafts_by_author_only() {
        let mut app = scoped_app();
        app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
        press(&mut app, "Gkzo");
        press(&mut app, "S");
        let authors: Vec<&str> =
            app.queue_rows().iter().filter_map(|r| if let QueueRow::Author { name, .. } = r { Some(*name) } else { None }).collect();
        assert_eq!(authors, ["lea", "omar"], "one header per author in Open, then in Drafts");
        let before_mine = app
            .queue_rows()
            .iter()
            .take_while(|r| !matches!(r, QueueRow::Section { name: "MINE", .. }))
            .any(|r| matches!(r, QueueRow::Author { .. }));
        assert!(!before_mine, "To review is never grouped");
        assert!(app.selected_mr().is_some(), "author headers are never where the cursor rests");
        press(&mut app, "S");
        assert!(!app.queue_rows().iter().any(|r| matches!(r, QueueRow::Author { .. })));
    }

    #[test]
    fn a_saved_view_for_this_scope_applies_and_one_for_another_is_dropped() {
        let mut app = scoped_app();
        let grouped = QueueView { order: order::Order::Author, by_author: true, ..QueueView::default() };
        app.apply(Incoming::QueueView { scope: None, view: grouped.clone() });
        assert_eq!(app.queue_view, QueueView::default());
        app.apply(Incoming::QueueView { scope: Some("acme/widgets".into()), view: grouped.clone() });
        assert_eq!(app.queue_view, grouped);
    }
}
