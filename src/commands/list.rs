use crate::api::{Queue, QueueMr, Sections};
use crate::cache::keys;
use crate::cli::ListArgs;
use crate::ctx::Ctx;
use crate::render::{self, Cell, Style, Theme, cell, right};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

const TITLE_W: usize = 64;

pub async fn run(ctx: &Ctx, args: ListArgs) -> Result<()> {
    let queue = if args.cached { cached(ctx)? } else { fetched(ctx).await? };
    let sections = queue.sections(&ctx.config.queue.watch_labels);
    if ctx.json {
        return ctx.emit(&sections);
    }
    print!("{}", text(&sections, &Theme::detect(), Utc::now()));
    Ok(())
}

fn cached(ctx: &Ctx) -> Result<Queue> {
    ctx.cache.read_entry::<Queue>(&keys::queue()).map(|e| e.value).context("nothing cached yet: run `mr list` without --cached")
}

async fn fetched(ctx: &Ctx) -> Result<Queue> {
    let queue = ctx.gitlab.queue().await?;
    ctx.cache.write_entry(&keys::queue(), &queue)?;
    Ok(queue)
}

pub fn text(sections: &Sections, theme: &Theme, now: DateTime<Utc>) -> String {
    let groups = [("TO REVIEW", &sections.to_review), ("MINE", &sections.mine), ("WATCHING", &sections.watching), ("DONE", &sections.done)];
    let filled: Vec<_> = groups.iter().filter(|(_, mrs)| !mrs.is_empty()).collect();
    if filled.is_empty() {
        return format!("{}\n", theme.paint("nothing open", Style::Dim));
    }
    filled
        .iter()
        .map(|(name, mrs)| {
            let header = format!("{} {}\n", theme.paint(name, Style::Bold), theme.paint(&mrs.len().to_string(), Style::Dim));
            let rows: Vec<Vec<Cell>> = mrs.iter().map(|mr| row(mr, now)).collect();
            format!("{header}{}\n", theme.table(&rows))
        })
        .collect()
}

fn row(mr: &QueueMr, now: DateTime<Utc>) -> Vec<Cell> {
    let title = if mr.draft { format!("Draft: {}", mr.title) } else { mr.title.clone() };
    vec![
        cell(format!("  !{}", mr.iid), Style::Accent),
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
    use super::*;
    use crate::api::Client;
    use crate::auth::Credentials;
    use chrono::TimeZone;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn sections() -> Sections {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../api/fixtures/queue.json")))
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        client.queue().await.unwrap().sections(&[])
    }

    #[tokio::test]
    async fn sections_print_as_headed_aligned_blocks() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        let out = text(&sections().await, &Theme::plain(), now);
        assert!(out.starts_with("TO REVIEW 1\n  !42  "), "{out}");
        assert!(out.contains("\nMINE 1\n") && out.contains("\nWATCHING 1\n") && out.contains("\nDONE 1\n"), "{out}");
        assert!(out.contains("+412") && out.contains("−38") && out.contains("acme/widgets"), "{out}");
    }

    #[test]
    fn an_empty_queue_says_so() {
        assert_eq!(text(&Sections::default(), &Theme::plain(), Utc::now()), "nothing open\n");
    }
}
