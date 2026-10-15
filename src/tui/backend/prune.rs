//! Once a day, the cache drops what it keeps for finished or long untouched MRs, and the files it
//! set aside as unreadable long ago.
use super::{Homes, warn_stopped};
use crate::blocking;
use crate::cache::{Cache, keys};
use crate::forge::{Forge, ProjectPath};
use crate::log::OrWarn as _;
use chrono::Utc;
use std::collections::BTreeMap;
use std::time::Duration;

/// How often the cache drops what it keeps for finished MRs.
const PRUNE_EVERY: Duration = Duration::from_hours(24);
/// An MR nobody opened for this long is dropped even when its forge could not say it is finished;
/// a file set aside as unreadable is kept as long.
const KEEP_UNTOUCHED: Duration = Duration::from_hours(30 * 24);

impl Homes {
    /// At most once a day, what the cache keeps for merged, closed or long untouched MRs goes:
    /// their diffs, threads and Claude's answers. Safe to run twice; a failure leaves the cache as it is.
    pub(in crate::tui) async fn prune_daily(&self) {
        let last = blocking::run(|| Ok(Cache::shared().read_entry::<()>(&keys::pruned()))).await.inspect_err(warn_stopped).ok().flatten();
        if last.is_some_and(|entry| entry.age(Utc::now()) < PRUNE_EVERY) {
            return;
        }
        let homes = std::iter::once((&self.forge, &self.cache)).chain(self.others.iter().map(|home| (&home.forge, &home.cache)));
        for (forge, cache) in homes {
            prune(forge, cache).await;
        }
        let forget_unreadable = || {
            Cache::shared().forget_unreadable(std::time::SystemTime::now(), KEEP_UNTOUCHED);
            Ok(())
        };
        blocking::run(forget_unreadable).await.or_warn("dropping the unreadable cache files");
        blocking::run(|| Cache::shared().write_entry(&keys::pruned(), &())).await.or_warn("noting the prune");
    }
}

/// One host's cache pruned: the forge asked once per project which kept MRs are finished; a
/// project it cannot answer for keeps its MRs until they are old.
async fn prune(forge: &Forge, cache: &Cache) {
    let listed = cache.clone();
    let Ok(kept) = blocking::run(move || Ok(listed.kept_mrs())).await else { return };
    let mut by_project: BTreeMap<ProjectPath, Vec<u64>> = BTreeMap::new();
    for mr in &kept {
        by_project.entry(mr.project.clone()).or_default().push(mr.number);
    }
    let mut finished = std::collections::HashSet::new();
    for (project, numbers) in by_project {
        if let Ok(done) = forge.finished(&project, &numbers).await {
            finished.extend(done.into_iter().map(|number| (project.clone(), number)));
        }
    }
    let gone: Vec<(ProjectPath, u64)> = crate::cache::to_forget(&kept, &finished, std::time::SystemTime::now(), KEEP_UNTOUCHED)
        .into_iter()
        .map(|mr| (mr.project.clone(), mr.number))
        .collect();
    let cache = cache.clone();
    blocking::run(move || {
        for (project, number) in &gone {
            cache.forget_mr(project, *number).or_warn("pruning the cache");
        }
        Ok(())
    })
    .await
    .or_warn("pruning the cache");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::backend::test_support::homes_on;

    #[tokio::test]
    async fn pruning_drops_the_merged_mrs_keeps_the_open_ones_and_runs_twice_without_harm() {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"iid": 40, "state": "merged"}, {"iid": 42, "state": "opened"}
            ])))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let Homes { forge, cache, .. } = homes_on(&server, dir.path());
        cache.write("mr/acme+widgets/40/ai/answer.1.json", &1).unwrap();
        cache.write("mr/acme+widgets/42/mr.json", &2).unwrap();
        prune(&forge, &cache).await;
        prune(&forge, &cache).await;
        let left: Vec<u64> = cache.kept_mrs().into_iter().map(|k| k.number).collect();
        assert_eq!(left, [42]);
    }
}
