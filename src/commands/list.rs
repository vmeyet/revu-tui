use crate::cache::keys;
use crate::cli::ListArgs;
use crate::ctx::Ctx;
use crate::forge::{Kind, Queue, QueueMr, Sections};
use crate::render::{self, Cell, Style, Theme, cell, right};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

const TITLE_W: usize = 64;

/// Prints the queue, scoped to the checkout's project unless `--all`.
pub async fn run(ctx: &Ctx, args: ListArgs) -> Result<()> {
    let queue = if args.cached { cached(ctx)? } else { fetched(ctx).await? };
    let sections = queue.sections(&ctx.config.queue.watch_labels);
    if ctx.json {
        return crate::ctx::emit(&sections);
    }
    print!("{}", text(ctx.forge.kind(), &sections, ctx.project.as_deref(), Theme::detect(), Utc::now()));
    Ok(())
}

fn cached(ctx: &Ctx) -> Result<Queue> {
    let key = keys::queue(ctx.project.as_deref());
    ctx.cache.read_entry::<Queue>(&key).map(|e| e.value).context("nothing cached yet: run `revu list` without --cached")
}

async fn fetched(ctx: &Ctx) -> Result<Queue> {
    let queue = ctx.forge.queue(ctx.project.as_deref()).await?;
    ctx.cache.write_entry(&keys::queue(ctx.project.as_deref()), &queue)?;
    Ok(queue)
}

/// `project` is the scope, named on a first line so a short list never reads as "nothing else is open".
pub(crate) fn text(kind: Kind, sections: &Sections, project: Option<&str>, theme: Theme, now: DateTime<Utc>) -> String {
    let scope = project.map(|p| format!("{}\n\n", theme.paint(&format!("{p} · --all for every project"), Style::Dim))).unwrap_or_default();
    let groups = [
        ("TO REVIEW", &sections.to_review),
        ("MINE", &sections.mine),
        ("WATCHING", &sections.watching),
        ("OPEN", &sections.open),
        ("DONE", &sections.done),
    ];
    let filled: Vec<_> = groups.iter().filter(|(_, mrs)| !mrs.is_empty()).collect();
    if filled.is_empty() {
        return format!("{scope}{}\n", theme.paint("nothing open", Style::Dim));
    }
    let blocks: String = filled
        .iter()
        .map(|(name, mrs)| {
            let header = format!("{} {}\n", theme.paint(name, Style::Bold), theme.paint(&mrs.len().to_string(), Style::Dim));
            let rows: Vec<Vec<Cell>> = mrs.iter().map(|mr| row(kind, mr, now)).collect();
            format!("{header}{}\n", theme.table(&rows))
        })
        .collect();
    format!("{scope}{blocks}")
}

fn row(kind: Kind, mr: &QueueMr, now: DateTime<Utc>) -> Vec<Cell> {
    let title = if mr.draft { format!("Draft: {}", mr.title) } else { mr.title.clone() };
    vec![
        cell(format!("  {}{}", kind.sigil(), mr.number), Style::Accent),
        cell(render::truncate(&title, TITLE_W), Style::Plain),
        cell(&mr.author, Style::Plain),
        right(render::age(mr.updated_at, now), Style::Dim),
        render::pipeline(mr.pipeline.as_deref()),
        right(format!("+{}", mr.additions), Style::Ok),
        right(format!("−{}", mr.deletions), Style::Bad),
        cell(badges(mr), Style::Warn),
        cell(&mr.project, Style::Dim),
    ]
}

fn badges(mr: &QueueMr) -> String {
    let mut out = Vec::new();
    if mr.conflicts {
        out.push("conflicts".to_owned());
    }
    if mr.unresolved > 0 {
        out.push(format!("{} open", mr.unresolved));
    }
    if mr.approved {
        out.push("approved".to_owned());
    }
    out.join(" · ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::gitlab::{Client, fixture};
    use chrono::TimeZone;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn sections() -> Sections {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../forge/gitlab/fixtures/queue.json")))
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        client.queue(None).await.unwrap().sections(&[])
    }

    #[tokio::test]
    async fn sections_print_as_headed_aligned_blocks() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        let out = text(Kind::GitLab, &sections().await, None, Theme::plain(), now);
        assert!(out.starts_with("TO REVIEW 1\n  !42  "), "{out}");
        assert!(out.contains("\nMINE 1\n") && out.contains("\nWATCHING 1\n") && out.contains("\nDONE 1\n"), "{out}");
        assert!(out.contains("+412") && out.contains("−38") && out.contains("acme/widgets"), "{out}");
    }

    #[test]
    fn an_empty_queue_says_so() {
        assert_eq!(text(Kind::GitLab, &Sections::default(), None, Theme::plain(), Utc::now()), "nothing open\n");
    }

    #[test]
    fn a_scoped_list_names_its_project_and_shows_the_open_section() {
        let sections = fixture::queue_in(include_str!("../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets").sections(&[]);
        let out = text(Kind::GitLab, &sections, Some("acme/widgets"), Theme::plain(), Utc::now());
        assert!(out.starts_with("acme/widgets · --all for every project\n\nTO REVIEW 1\n"), "{out}");
        let open = out.find("\nOPEN 2\n").expect("an OPEN section");
        assert!(out.find("\nMINE 1\n").unwrap() < open && open < out.find("\nDONE 1\n").unwrap(), "{out}");
    }
}
