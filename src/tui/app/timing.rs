//! How long opening and idling on a large MR take, in a release build: `cargo test --release -- --ignored timing --nocapture`.
#![allow(clippy::unwrap_used)]
use super::test_support::*;
use std::time::Instant;

const FILES: usize = 300;
const LINES_PER_FILE: usize = 16;
const LOCK_LINES: usize = 1500;
const THREADS: usize = 40;
const RUNS: usize = 5;

fn source_file(index: usize) -> DiffFile {
    let removed = (0..LINES_PER_FILE / 2).map(|n| format!("-export const value{n} = compute(\"{index}\", {n}) // before"));
    let added = (0..LINES_PER_FILE / 2).map(|n| format!("+export const value{n} = compute(\"{index}\", {n} * 2) // after"));
    let body: Vec<String> = removed.chain(added).collect();
    let path = format!("src/module{index}/file.ts");
    let half = LINES_PER_FILE / 2;
    DiffFile {
        diff: format!("@@ -1,{half} +1,{half} @@\n{}\n", body.join("\n")),
        old_path: path.clone(),
        new_path: path,
        ..DiffFile::default()
    }
}

fn lock_file() -> DiffFile {
    let body: Vec<String> = (0..LOCK_LINES).map(|n| format!("+    \"package-{n}\": {{ \"version\": \"1.0.{n}\" }},")).collect();
    DiffFile {
        diff: format!("@@ -0,0 +1,{LOCK_LINES} @@\n{}\n", body.join("\n")),
        old_path: "package-lock.json".into(),
        new_path: "package-lock.json".into(),
        ..DiffFile::default()
    }
}

fn large_diffs() -> Vec<DiffFile> {
    (0..FILES).map(source_file).chain(std::iter::once(lock_file())).collect()
}

fn thread_on(index: usize) -> Discussion {
    let mut thread: serde_json::Value = serde_json::from_str(include_str!("../../forge/gitlab/fixtures/diff_note.json")).unwrap();
    let first = thread["notes"][0].clone();
    let fence: Vec<String> = (0..10).map(|n| format!("const retry{n} = await charge(card, {n}) // try again")).collect();
    let note = |n: usize| {
        let mut note = first.clone();
        note["id"] = json!(9000 + index * 10 + n);
        note["body"] = json!(format!("Could this read better?\n\n```ts\n{}\n```\n\nnote {n}", fence.join("\n")));
        note["position"]["new_path"] = json!(format!("src/module{index}/file.ts"));
        note["position"]["old_path"] = json!(format!("src/module{index}/file.ts"));
        note["position"]["new_line"] = json!(2);
        note
    };
    thread["id"] = json!(format!("thread{index}"));
    thread["notes"] = json!((0..3).map(note).collect::<Vec<_>>());
    fixture::discussion(&thread.to_string())
}

fn large_discussions() -> Vec<Discussion> {
    (0..THREADS).map(thread_on).collect()
}

fn large_review() -> Review {
    Review::new(mr(), &large_diffs(), large_discussions(), &["package-lock.json".into()])
}

fn opening() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    press(&mut app, "\r");
    app.handle_key(code(KeyCode::Enter));
    app
}

fn draw(app: &mut App) {
    render(app, 200, 60);
}

/// The median of `RUNS` timings of `step`, each on a fresh `setup`, in milliseconds.
fn median_ms<T>(setup: impl Fn() -> T, step: impl Fn(&mut T)) -> f64 {
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let mut value = setup();
            let start = Instant::now();
            step(&mut value);
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[RUNS / 2]
}

fn opened() -> App {
    let mut app = opening();
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(large_review()), cached: Some(Duration::from_secs(60)) });
    draw(&mut app);
    app
}

fn with_pane() -> App {
    let mut app = opened();
    press(&mut app, "T");
    draw(&mut app);
    app
}

#[test]
#[ignore = "a timing, run on demand in release"]
fn timing() {
    let open = median_ms(opening, |app| {
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(large_review()), cached: Some(Duration::from_secs(60)) });
        draw(app);
    });
    let reopen = median_ms(opened, |app| {
        app.apply(Incoming::Threads { key: mr_key(), mr: Box::new(mr()), discussions: large_discussions(), drafts: vec![] });
        draw(app);
    });
    let idle = median_ms(with_pane, |app| {
        for _ in 0..60 {
            app.today += chrono::Duration::seconds(1);
            draw(app);
        }
    }) / 60.0;
    let poll = median_ms(with_pane, |app| {
        app.apply(Incoming::Discussions { key: mr_key(), discussions: large_discussions() });
        draw(app);
    });
    println!("open to first frame: {open:.1} ms");
    println!("cache-hit reopen, fresh data applied and drawn: {reopen:.1} ms");
    println!("one idle second with the pane open: {idle:.2} ms");
    println!("no-change discussions poll: {poll:.1} ms");
}
