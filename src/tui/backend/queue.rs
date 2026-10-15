//! The queue of every host I am logged in to, with the pins and the ready command's answer, merged into sections.
use super::saves::Saves;
use super::{Homes, read_cache, run_blocking, write_cache};
use crate::cache::keys;
use crate::forge::{MrKey, ProjectPath, Queue, QueueMr, Sections};
use crate::ready::Source as ReadySource;
use crate::tui::app::{self, Incoming};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone)]
pub(in crate::tui) struct Queues {
    pub(super) homes: Arc<Homes>,
    pub(super) saves: Saves,
    pub(super) watch_labels: Vec<String>,
    pub(super) rules: crate::forge::rules::Rules,
    /// `[queue.ready] command`: what prints the MRs ready for review.
    pub(super) ready_command: Option<String>,
}

impl Queues {
    /// Every project's queue also asks the other hosts I am logged in to; one that fails to
    /// answer shows the rows it last answered rather than failing the whole queue. The ready command
    /// runs alongside; when it fails the queue still paints, with its last answer, and the reason comes
    /// back apart. What follows the answer: the pins found merged or closed, that reason, and the hosts
    /// that did not answer.
    pub(in crate::tui) async fn load(&self, scope: Option<ProjectPath>) -> Result<(Incoming, Vec<Incoming>)> {
        let others = async {
            if scope.is_some() {
                return (vec![], vec![]);
            }
            self.other_queues().await
        };
        let (queue, (others, silent), said) = tokio::join!(self.homes.forge.queue(scope.as_ref()), others, self.ask_ready());
        let queue = queue?;
        let (wanted, saved) = (scope.clone(), queue.clone());
        write_cache("caching the queue", &self.homes, move |homes| homes.cache.write_entry(&keys::queue(wanted.as_ref()), &saved)).await;
        let (ready, failure) = match said {
            Ok(Some(output)) => (Some(self.fresh_ready(scope.as_ref(), output, &queue, &others).await), None),
            Ok(None) => (None, None),
            Err(e) => (self.cached_ready(scope.as_ref()), Some(format!("{e:#}"))),
        };
        let held = ready.iter().flat_map(|source| &source.outside);
        let listed = std::iter::once(&queue).chain(&others).flat_map(Queue::listed).chain(held.map(QueueMr::key)).collect();
        let (pinned, gone) = self.pins_outside(scope.as_ref(), &listed).await;
        let follow = [
            (!gone.is_empty()).then(|| Incoming::PinsGone { scope: scope.clone(), gone }),
            failure.map(|message| Incoming::Failed { what: app::Failure::Ready, message }),
            (!silent.is_empty()).then(|| Incoming::Failed { what: app::Failure::Host, message: silent_note(&silent) }),
        ];
        let answer = run_blocking(self, move |queues| queues.answer(scope, &queue, &others, ready.as_ref(), pinned, false)).await?;
        Ok((answer, follow.into_iter().flatten().collect()))
    }

    /// Each other host's queue; one that does not answer gives the rows it last answered, and its name.
    async fn other_queues(&self) -> (Vec<Queue>, Vec<String>) {
        let others = &self.homes.others;
        let answers = futures_util::future::join_all(others.iter().map(crate::ctx::Home::queue)).await;
        let mut queues = vec![];
        let mut silent = vec![];
        for (home, answer) in others.iter().zip(answers) {
            match answer {
                Ok(queue) => queues.push(queue),
                Err(e) => {
                    tracing::warn!("the queue of {} failed: {e:#}", home.host);
                    silent.push(home.host.clone());
                }
            }
        }
        let wanted = silent.clone();
        let kept = read_cache(&self.homes, move |homes| {
            homes.others.iter().filter(|home| wanted.contains(&home.host)).filter_map(crate::ctx::Home::cached).collect::<Vec<_>>()
        });
        (queues.into_iter().chain(kept.await).collect(), silent)
    }

    pub(in crate::tui) fn cached(&self, scope: Option<ProjectPath>) -> Option<Incoming> {
        let queue: Queue = self.homes.cache.read_entry(&keys::queue(scope.as_ref()))?.value;
        let others: Vec<Queue> =
            if scope.is_none() { self.homes.others.iter().filter_map(crate::ctx::Home::cached).collect() } else { vec![] };
        let ready = self.cached_ready(scope.as_ref());
        Some(self.answer(scope, &queue, &others, ready.as_ref(), vec![], true))
    }

    /// The pins of `scope` the queue does not hold yet, each asked of its forge: the open ones come
    /// back as rows, the merged or closed ones as gone. A pin the forge did not answer for stays.
    async fn pins_outside(&self, scope: Option<&ProjectPath>, listed: &HashSet<MrKey>) -> (Vec<QueueMr>, Vec<MrKey>) {
        let wanted = scope.cloned();
        let pins = read_cache(&self.saves, move |saves| saves.pins(wanted.as_ref())).await;
        let missing: Vec<MrKey> = pins.into_iter().filter(|key| !listed.contains(key)).collect();
        let fetched = crate::ctx::fetch_each(&self.homes.forge, &self.homes.others, &missing).await;
        let mut rows = vec![];
        let mut gone = vec![];
        for (key, mr) in missing.into_iter().zip(fetched) {
            let Ok(mr) = mr else { continue };
            match QueueMr::from_mr(&mr, key.host.clone()) {
                Some(row) => rows.push(row),
                None => gone.push(key),
            }
        }
        (rows, gone)
    }

    async fn ask_ready(&self) -> Result<Option<String>> {
        let Some(command) = &self.ready_command else { return Ok(None) };
        crate::ready::output(command).await.map(Some)
    }

    fn cached_ready(&self, scope: Option<&ProjectPath>) -> Option<ReadySource> {
        self.ready_command.as_ref()?;
        self.homes.cache.read(&keys::ready(scope))
    }

    async fn fresh_ready(&self, scope: Option<&ProjectPath>, output: String, queue: &Queue, others: &[Queue]) -> ReadySource {
        let queues: Vec<&Queue> = std::iter::once(queue).chain(others).collect();
        let source = crate::ready::resolve(output, &self.homes.forge, &self.homes.others, scope, &queues).await;
        let (wanted, saved) = (scope.cloned(), source.clone());
        write_cache("caching the ready command's answer", &self.homes, move |homes| {
            homes.cache.write(&keys::ready(wanted.as_ref()), &saved)
        })
        .await;
        source
    }

    /// `pinned` are the pinned MRs no list holds.
    fn answer(
        &self,
        scope: Option<ProjectPath>,
        queue: &Queue,
        others: &[Queue],
        ready: Option<&ReadySource>,
        pinned: Vec<QueueMr>,
        cached: bool,
    ) -> Incoming {
        let now = Utc::now();
        let parts = std::iter::once(queue).chain(others).map(|q| q.sections_with(&self.watch_labels, &self.rules, now)).collect();
        let sections = Sections::merge(parts);
        let sections = match ready {
            Some(source) => source.apply(sections, self.homes.forge.host(), &self.homes.others, scope.as_ref(), &queue.me, &self.rules),
            None => sections,
        };
        let sections = Sections { pinned, ..sections };
        let opened = self.saves.opened_at(&sections);
        Incoming::Queue { scope, me: queue.me.clone(), sections, opened, cached }
    }
}

fn silent_note(hosts: &[String]) -> String {
    format!("{} did not answer · showing {} last rows", hosts.join(", "), if hosts.len() == 1 { "its" } else { "their" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::forge::Forge;
    use crate::forge::gitlab::Client;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::{homes_in, homes_on, mount_mr, queue};
    use std::collections::BTreeSet;

    fn queues(homes: Homes) -> Queues {
        let homes = Arc::new(homes);
        Queues {
            saves: Saves::new(Arc::clone(&homes)),
            homes,
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::disabled(),
            ready_command: None,
        }
    }

    #[test]
    fn an_mr_from_another_host_reaches_that_host_and_its_cache() {
        let dir = tempfile::tempdir().unwrap();
        let github =
            crate::forge::github::Client::new(&crate::auth::Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }).unwrap();
        let other =
            crate::ctx::Home { host: "github.com".into(), forge: Forge::GitHub(github), cache: Cache::in_dir(dir.path().join("gh")) };
        let queue = queue();
        other.cache.write_entry(&keys::queue(None), &queue.clone().on_host("github.com")).unwrap();
        let queues = queues(Homes { others: vec![other], ..homes_in(&dir.path().join("gl")) });
        queues.homes.cache.write_entry(&keys::queue(None), &queue).unwrap();
        let there = MrKey { host: Some("github.com".into()), ..key() };
        assert_eq!(
            (queues.homes.forge_of(&there).kind(), queues.homes.forge_of(&key()).kind()),
            (crate::forge::Kind::GitHub, crate::forge::Kind::GitLab)
        );
        let Some(Incoming::Queue { sections, .. }) = queues.cached(None) else { panic!("both caches answer") };
        assert_eq!(
            sections.to_review.iter().filter(|mr| mr.host.as_deref() == Some("github.com")).count(),
            queue.sections(&[]).to_review.len()
        );
        assert!(queues.cached(Some("acme/widgets".into())).is_none(), "a scoped queue keeps to its own host");
    }

    #[test]
    fn the_last_ready_answer_paints_ready_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let queues = Queues {
            ready_command: Some("never-run".into()),
            rules: crate::forge::rules::Rules { stale_days: u32::MAX, ..crate::forge::rules::Rules::default() },
            ..queues(homes_in(dir.path()))
        };
        let queue = crate::forge::gitlab::fixture::queue_in(include_str!("../../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets");
        queues.homes.cache.write_entry(&keys::queue(Some(&"acme/widgets".into())), &queue).unwrap();
        let output = format!("ready: https://{}/acme/widgets/-/merge_requests/51", queues.homes.forge.host());
        queues.homes.cache.write(&keys::ready(Some(&"acme/widgets".into())), &ReadySource { output, outside: vec![] }).unwrap();
        let Some(Incoming::Queue { sections, .. }) = queues.cached(Some("acme/widgets".into())) else { panic!("the cache answers") };
        assert_eq!(sections.ready.iter().map(|mr| mr.number).collect::<Vec<_>>(), [51]);
        assert!(sections.open.iter().all(|mr| mr.number != 51));
        let without = Queues { ready_command: None, ..queues };
        let Some(Incoming::Queue { sections, .. }) = without.cached(Some("acme/widgets".into())) else { panic!("the cache answers") };
        assert!(sections.ready.is_empty(), "no command, no Ready, whatever the cache kept");
    }

    #[tokio::test]
    async fn pins_no_list_holds_are_read_from_the_forge_which_alone_can_drop_them() {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-20T10:00:00Z", [1, 1, 0, 0, 0]).await;
        let merged = serde_json::json!({
            "id": 1043, "iid": 43, "project_id": 7, "title": "fix: old", "description": "", "state": "merged", "draft": false,
            "author": {"id": 5, "username": "omar", "name": "Omar"}, "source_branch": "fix/old", "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/43", "updated_at": "2026-09-20T10:00:00Z", "sha": "cccc",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "cccc", "start_sha": "aaaa"}
        });
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/43";
        for (route, body) in [
            (base.to_owned(), merged),
            (format!("{base}/approvals"), serde_json::json!({"approved": false, "approvals_left": 0, "approved_by": []})),
        ] {
            wiremock::Mock::given(method("GET"))
                .and(path(route))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
        }
        let dir = tempfile::tempdir().unwrap();
        let queues = queues(homes_on(&server, dir.path()));
        let pins: BTreeSet<MrKey> = [42, 43, 44].map(|n| MrKey::new("acme/widgets", n)).into();
        queues.homes.cache.write(&keys::pins(None), &pins).unwrap();
        assert_eq!(queues.saves.pins(None), pins, "the pins round trip through the cache");
        let (rows, gone) = queues.pins_outside(None, &HashSet::new()).await;
        assert_eq!(rows.iter().map(|mr| mr.number).collect::<Vec<_>>(), [42], "an open one shows");
        assert_eq!(gone, [MrKey::new("acme/widgets", 43)], "the merged one goes; 44 failed to fetch and stays");
        let (rows, gone) = queues.pins_outside(None, &queue().listed().collect()).await;
        assert!(rows.is_empty() && gone == [MrKey::new("acme/widgets", 43)], "a listed pin is not fetched");
    }

    #[tokio::test]
    async fn a_host_that_fails_to_answer_keeps_its_last_rows_and_is_named() {
        use wiremock::matchers::{method, path};
        let (main, silent) = (wiremock::MockServer::start().await, wiremock::MockServer::start().await);
        wiremock::Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(include_str!("../../forge/gitlab/fixtures/queue.json")))
            .mount(&main)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let creds = crate::auth::Credentials { host: "git.acme.test".into(), token: "glpat-xxxx".into() };
        let forge = Forge::GitLab(Client::with_base(&creds, &format!("{}/api/v4/", silent.uri())).unwrap());
        let other = crate::ctx::Home { host: "git.acme.test".into(), forge, cache: Cache::in_dir(dir.path().join("other")) };
        other.cache.write_entry(&keys::queue(None), &queue().on_host("git.acme.test")).unwrap();
        let queues = queues(Homes { others: vec![other], ..homes_on(&main, &dir.path().join("main")) });
        let (Incoming::Queue { sections, .. }, follow) = queues.load(None).await.unwrap() else { panic!("a queue") };
        let there = sections.all().filter(|mr| mr.host.as_deref() == Some("git.acme.test")).count();
        assert_eq!(there, sections.all().count() / 2, "the silent host keeps the rows it last had");
        let [Incoming::Failed { what: app::Failure::Host, message }] = follow.as_slice() else { panic!("{follow:?}") };
        assert_eq!(message, "git.acme.test did not answer · showing its last rows");
    }
}
