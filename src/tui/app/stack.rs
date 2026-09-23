//! Stacked MRs: one author's MRs that build on each other, each targeting the branch of the one
//! below. The queue shows such a chain as one row that unfolds, base first.
use crate::forge::QueueMr;
use std::collections::HashMap;

/// A section's rows once chains are found: lone MRs, and stacks of two or more.
#[derive(Clone, Debug, PartialEq)]
pub enum Item<'a> {
    Single(&'a QueueMr),
    Stack(Stack<'a>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stack<'a> {
    /// The base MR's key, stable while the chain lives: what the queue state remembers unfolded.
    pub id: String,
    /// Base first, each MR after the one it targets.
    pub mrs: Vec<&'a QueueMr>,
}

/// `mrs` with every chain folded into one item, at the place its first member held. A chain only
/// joins MRs of one author in one project; a fork (two MRs on one base) stays one stack.
pub fn group<'a>(mrs: &[&'a QueueMr]) -> Vec<Item<'a>> {
    let parents = parents(mrs);
    let roots: Vec<usize> = (0..mrs.len()).map(|i| root(&parents, i)).collect();
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for root in &roots {
        *sizes.entry(*root).or_default() += 1;
    }
    let mut emitted = vec![false; mrs.len()];
    let mut items = vec![];
    for (i, mr) in mrs.iter().enumerate() {
        let root = roots[i];
        if sizes[&root] < 2 {
            items.push(Item::Single(mr));
        } else if !emitted[root] {
            emitted[root] = true;
            items.push(Item::Stack(stack(mrs, &parents, &roots, root)));
        }
    }
    items
}

/// For each MR, the MR whose branch it targets: same author, same project and host.
fn parents(mrs: &[&QueueMr]) -> Vec<Option<usize>> {
    let place = |mr: &QueueMr, branch: &str| (mr.host.clone(), mr.project.clone(), mr.author.clone(), branch.to_owned());
    let by_source: HashMap<_, usize> = mrs.iter().enumerate().map(|(i, mr)| (place(mr, &mr.source_branch), i)).collect();
    mrs.iter().enumerate().map(|(i, mr)| by_source.get(&place(mr, &mr.target_branch)).copied().filter(|&p| p != i)).collect()
}

/// The chain's base: up the parents until none is left. A cycle has no base, so it settles on
/// its lowest index, the same one from every member.
fn root(parents: &[Option<usize>], start: usize) -> usize {
    let mut seen = vec![start];
    let mut at = start;
    while let Some(parent) = parents[at] {
        if seen.contains(&parent) {
            return seen[seen.iter().position(|&s| s == parent).unwrap_or(0)..].iter().copied().min().unwrap_or(start);
        }
        seen.push(parent);
        at = parent;
    }
    at
}

/// The stack under `root`, base first: by how far each MR sits from the base, then oldest first.
fn stack<'a>(mrs: &[&'a QueueMr], parents: &[Option<usize>], roots: &[usize], root: usize) -> Stack<'a> {
    let depth = |i: usize| {
        let mut depth = 0;
        let mut at = i;
        while at != root && depth < mrs.len() {
            let Some(parent) = parents[at] else { break };
            at = parent;
            depth += 1;
        }
        depth
    };
    let mut members: Vec<usize> = (0..mrs.len()).filter(|&i| roots[i] == root).collect();
    members.sort_by_key(|&i| (depth(i), mrs[i].created_at));
    Stack { id: id_of(mrs[root]), mrs: members.into_iter().map(|i| mrs[i]).collect() }
}

fn id_of(mr: &QueueMr) -> String {
    match &mr.host {
        Some(host) => format!("{host}/{}!{}", mr.project, mr.number),
        None => format!("{}!{}", mr.project, mr.number),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn mr(number: u64, author: &str, source: &str, target: &str) -> QueueMr {
        let base = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        QueueMr {
            host: None,
            number,
            project: "acme/widgets".into(),
            title: format!("mr {number}"),
            description: String::new(),
            draft: false,
            web_url: String::new(),
            updated_at: base,
            created_at: base + Duration::hours(number.try_into().unwrap()),
            source_branch: source.into(),
            target_branch: target.into(),
            conflicts: false,
            author: author.into(),
            author_name: author.into(),
            approved: false,
            approved_by: vec![],
            reviewers: vec![],
            pipeline: None,
            additions: 0,
            deletions: 0,
            files: 0,
            unresolved: 0,
            labels: vec![],
            notes: 0,
        }
    }

    fn shape(items: &[Item]) -> Vec<Vec<u64>> {
        items
            .iter()
            .map(|item| match item {
                Item::Single(mr) => vec![mr.number],
                Item::Stack(stack) => stack.mrs.iter().map(|mr| mr.number).collect(),
            })
            .collect()
    }

    #[test]
    fn a_chain_folds_into_one_stack_base_first_where_its_first_member_stood() {
        let (a, b, c) = (mr(1, "nina", "pdf-1", "main"), mr(2, "nina", "pdf-2", "pdf-1"), mr(3, "nina", "pdf-3", "pdf-2"));
        let lone = mr(9, "omar", "fix", "main");
        let items = group(&[&c, &lone, &a, &b]);
        assert_eq!(shape(&items), vec![vec![1, 2, 3], vec![9]], "the stack stands where its newest member stood");
        let Item::Stack(stack) = &items[0] else { panic!() };
        assert_eq!(stack.id, "acme/widgets!1");
    }

    #[test]
    fn a_fork_on_one_base_stays_one_stack() {
        let (a, b, c) = (mr(1, "nina", "base", "main"), mr(2, "nina", "left", "base"), mr(3, "nina", "right", "base"));
        assert_eq!(shape(&group(&[&a, &b, &c])), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn another_authors_mr_on_the_branch_is_not_stacked() {
        let (a, b) = (mr(1, "nina", "pdf-1", "main"), mr(2, "omar", "pdf-2", "pdf-1"));
        assert_eq!(shape(&group(&[&a, &b])), vec![vec![1], vec![2]]);
    }

    #[test]
    fn another_projects_branch_of_the_same_name_is_not_stacked() {
        let a = mr(1, "nina", "pdf-1", "main");
        let b = QueueMr { project: "acme/gadgets".into(), ..mr(2, "nina", "pdf-2", "pdf-1") };
        assert_eq!(shape(&group(&[&a, &b])), vec![vec![1], vec![2]]);
    }

    #[test]
    fn a_cycle_ends_the_walk_and_still_stacks_once() {
        let (a, b) = (mr(1, "nina", "x", "y"), mr(2, "nina", "y", "x"));
        let items = group(&[&a, &b]);
        assert_eq!(items.len(), 1);
        let Item::Stack(stack) = &items[0] else { panic!() };
        assert_eq!(stack.mrs.len(), 2);
    }

    #[test]
    fn lone_mrs_keep_their_order() {
        let (a, b) = (mr(1, "nina", "a", "main"), mr(2, "nina", "b", "main"));
        assert_eq!(shape(&group(&[&b, &a])), vec![vec![2], vec![1]]);
    }

    #[test]
    fn an_mr_targeting_its_own_branch_is_alone() {
        let a = mr(1, "nina", "same", "same");
        assert_eq!(shape(&group(&[&a])), vec![vec![1]]);
    }
}
