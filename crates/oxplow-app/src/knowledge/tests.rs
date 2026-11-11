use super::*;
use oxplow_domain::Actor;

fn dir(fx: &crate::test_fixtures::ThreadFixture) -> PathBuf {
    fx.svc.layout.project_dir.clone()
}

fn oxwiki(id: &str) -> String {
    knowledge_uri(OXWIKI, id)
}

fn page_path(fx: &crate::test_fixtures::ThreadFixture, slug: &str) -> PathBuf {
    oxplow_oxwiki::page_path(&dir(fx), slug)
}

async fn run(
    fx: &crate::test_fixtures::ThreadFixture,
    name: &str,
    input: serde_json::Value,
) -> Result<oxplow_domain::CommandOutcome, CommandError> {
    fx.svc.commands.run(&Actor::Human, name, input, true).await
}

async fn events_of(
    fx: &crate::test_fixtures::ThreadFixture,
    event_type: &str,
) -> Vec<oxplow_domain::StoredEvent> {
    fx.svc
        .event_log_store
        .read_after(0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.envelope.event_type == event_type)
        .collect()
}

async fn sync(fx: &crate::test_fixtures::ThreadFixture, slug: &str) -> bool {
    oxplow_oxwiki::pages::sync_page(&fx.svc.db, &fx.svc.vocabulary, &dir(fx), slug)
        .await
        .unwrap()
}

async fn recorded(fx: &crate::test_fixtures::ThreadFixture, uri: &str) -> bool {
    fx.svc.knowledge_pages.get(uri).await.unwrap().is_some()
}

/// One run writes the row, the file and the event.
#[tokio::test]
async fn one_run_writes_the_row_the_file_and_the_event() {
    let fx = crate::test_fixtures::services_with_task().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    std::fs::write(dir(&fx).join("src/lib.rs"), "fn x() {}\n").unwrap();
    let body = format!("# VCS notes\n\nSee [[src/lib.rs]] and [[{}]].\n", fx.task);
    let out = run(&fx, WRITE_PAGE, json!({ "id": "vcs-notes", "body": body }))
        .await
        .unwrap();
    assert_eq!(out.result["page"], oxwiki("vcs-notes"));
    assert_eq!(out.result["store"], OXWIKI);
    assert_eq!(out.result["id"], "vcs-notes");
    let outbound: Vec<&str> = out.result["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        outbound.contains(&"oxplow:oxplow/file?path=src/lib.rs"),
        "{outbound:?}"
    );
    let page = fx
        .svc
        .knowledge_pages
        .get(&oxwiki("vcs-notes"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.title, "VCS notes");
    assert_eq!(
        (page.store.as_str(), page.id.as_str()),
        (OXWIKI, "vcs-notes")
    );
    assert_eq!(
        std::fs::read_to_string(page_path(&fx, "vcs-notes")).unwrap(),
        body
    );
    assert!(
        page_path(&fx, "vcs-notes").starts_with(dir(&fx).join(".oxplow/oxwiki")),
        "oxwiki keeps its pages in .oxplow/oxwiki"
    );
    let written = events_of(&fx, "knowledge.page.written").await;
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].envelope.cause, out.event_id);
    assert_eq!(written[0].envelope.subject, vec![oxwiki("vcs-notes")]);
    assert_eq!(written[0].envelope.payload["store"], OXWIKI);

    // The file the command wrote syncs to a no-op, not a second write.
    assert!(!sync(&fx, "vcs-notes").await);
    assert_eq!(events_of(&fx, "knowledge.page.written").await.len(), 1);
}

/// A write with no id has the store derive one from its title.
#[tokio::test]
async fn a_page_with_no_id_takes_one_from_its_title() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let out = run(
        &fx,
        WRITE_PAGE,
        json!({ "title": "Auth flow: callbacks", "body": "Validate `next`.\n" }),
    )
    .await
    .unwrap();
    assert_eq!(out.result["page"], oxwiki("auth-flow-callbacks"));
    assert_eq!(
        std::fs::read_to_string(page_path(&fx, "auth-flow-callbacks")).unwrap(),
        "# Auth flow: callbacks\n\nValidate `next`.\n"
    );
    let err = run(&fx, WRITE_PAGE, json!({ "body": "# No id\n" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("`id`"), "{err}");
    // A title that names a page already there doesn't replace it.
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "title": "Auth flow: callbacks", "body": "Other.\n" }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("id: auth-flow-callbacks"), "{err}");
    assert_eq!(
        std::fs::read_to_string(page_path(&fx, "auth-flow-callbacks")).unwrap(),
        "# Auth flow: callbacks\n\nValidate `next`.\n"
    );
}

/// The file is the page: a file that can't be written fails the run,
/// and nothing — row, edges, event — is recorded.
#[tokio::test]
async fn a_file_that_cant_be_written_fails_the_run_and_records_nothing() {
    let fx = crate::test_fixtures::services_with_thread().await;
    // A directory where the page's file goes.
    std::fs::create_dir_all(page_path(&fx, "blocked")).unwrap();
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "blocked", "body": "# Blocked\n" }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("blocked.md"), "{err}");
    assert!(!recorded(&fx, &oxwiki("blocked")).await);
    assert!(events_of(&fx, "knowledge.page.written").await.is_empty());
}

/// One slug rule: an oxwiki file whose name isn't a slug is not a page.
#[tokio::test]
async fn a_file_whose_name_isnt_a_slug_is_not_a_page() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let folder = oxplow_oxwiki::oxwiki_dir(&dir(&fx));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Bad_Name.md"), "# Bad\n").unwrap();
    oxplow_oxwiki::pages::scan_and_sync_all(&fx.svc.db, &fx.svc.vocabulary, &dir(&fx))
        .await
        .unwrap();
    assert!(!recorded(&fx, &oxwiki("Bad_Name")).await);
    assert!(events_of(&fx, "knowledge.page.written").await.is_empty());
}

/// `oxplow.knowledge.link` adds under `## Related` — even when a section
/// follows it — and the body it writes has no `@version`.
#[tokio::test]
async fn link_adds_under_related_and_strips_versions() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    std::fs::write(dir(&fx).join("src/lib.rs"), "x").unwrap();
    for id in ["b", "c"] {
        run(&fx, WRITE_PAGE, json!({ "id": id, "body": "# P\n" }))
            .await
            .unwrap();
    }
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "a", "body": "# A\n\n## Related\n\n- [[b]]\n\n## Notes\n\nmore\n" }),
    )
    .await
    .unwrap();
    // A hand edit leaves a version literal behind, and is synced.
    let path = page_path(&fx, "a");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("more", "see [[src/lib.rs@abc1234]]");
    std::fs::write(&path, edited).unwrap();
    sync(&fx, "a").await;

    run(&fx, LINK, json!({ "id": "a", "target": "c" }))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# A\n\n## Related\n\n- [[b]]\n- [[c]]\n\n## Notes\n\nsee [[src/lib.rs]]\n"
    );
}

/// A dangling link is refused, naming it, and nothing is written.
#[tokio::test]
async fn a_dangling_link_is_refused_naming_it() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "notes", "body": "# N\n\n[[no-such-page]] and [[src/gone.rs]] and [[tsk9999]]\n" }),
    )
    .await
    .unwrap_err();
    let message = err.to_string();
    for target in ["no-such-page", "src/gone.rs", "tsk9999"] {
        assert!(message.contains(target), "{message}");
    }
    assert!(!recorded(&fx, &oxwiki("notes")).await);
    assert!(!page_path(&fx, "notes").exists());
    assert!(events_of(&fx, "knowledge.page.written").await.is_empty());

    let err = run(&fx, WRITE_PAGE, json!({ "id": "Not A Slug", "body": "x" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("kebab-case"), "{err}");
    // A page may link to itself.
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "self", "body": "# S\n\nsee [[self]]\n" }),
    )
    .await
    .unwrap();
}

/// Only links the page doesn't already have are refused: once a cited file
/// is gone, the page can still be verified, rewritten and linked — a new
/// dangling link is still refused.
#[tokio::test]
async fn a_link_that_went_dangling_doesnt_block_the_page() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    std::fs::write(dir(&fx).join("src/a.rs"), "a").unwrap();
    std::fs::write(dir(&fx).join("src/b.rs"), "b").unwrap();
    let body = "# P\n\nsee [[src/a.rs]] and [[src/b.rs]]\n";
    run(&fx, WRITE_PAGE, json!({ "id": "p", "body": body }))
        .await
        .unwrap();
    run(&fx, WRITE_PAGE, json!({ "id": "q", "body": "# Q\n" }))
        .await
        .unwrap();
    std::fs::remove_file(dir(&fx).join("src/a.rs")).unwrap();

    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "p", "body": body, "verified_files": ["src/b.rs"] }),
    )
    .await
    .unwrap();
    run(&fx, LINK, json!({ "id": "p", "target": "q" }))
        .await
        .unwrap();
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "p", "body": format!("{body}and [[src/c.rs]]\n") }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("src/c.rs"), "{err}");
    assert!(!err.to_string().contains("src/a.rs"), "{err}");
}

/// Verified refs must be cited (or under a cited directory, which makes an
/// edge of their own); removed ones must be gone.
#[tokio::test]
async fn verified_and_removed_files_are_checked_against_the_body() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("crates/cp/src")).unwrap();
    std::fs::write(dir(&fx).join("crates/cp/src/lib.rs"), "").unwrap();
    std::fs::write(dir(&fx).join("crates/foo.rs"), "").unwrap();
    let body = "# I\n\nsee [[dir:crates/cp]] and [[crates/foo.rs]]\n";
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "intro", "body": body, "removed_files": ["crates/foo.rs"] }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("crates/foo.rs"), "{err}");
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "intro", "body": body, "verified_files": ["crates/other/main.rs"] }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("crates/other/main.rs"), "{err}");
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "intro", "body": body, "verified_files": ["crates/cp/src/lib.rs"] }),
    )
    .await
    .unwrap();
    let cites = |fx: &crate::test_fixtures::ThreadFixture| {
        let svc = fx.svc.clone();
        async move {
            svc.page_ref_store
                .list_backlinks(
                    &oxplow_domain::uri::build::file_uri("crates/cp/src/lib.rs"),
                    None,
                )
                .await
                .unwrap()
                .iter()
                .any(|e| e.source == oxwiki("intro"))
        }
    };
    assert!(cites(&fx).await);
    // Rewriting the page (the directory still cited) keeps it.
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "intro", "body": format!("{body}\nMore.\n") }),
    )
    .await
    .unwrap();
    assert!(cites(&fx).await);
}

/// A hand edit converges through the same record, as `system:oxwiki_watch`
/// — dangling links and all.
#[tokio::test]
async fn a_hand_edit_converges_as_the_watcher() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let path = page_path(&fx, "hand");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Hand\n\nsee [[not-yet-written]]\n").unwrap();
    assert!(sync(&fx, "hand").await);
    let page = fx.svc.knowledge_pages.get(&oxwiki("hand")).await.unwrap();
    assert_eq!(page.unwrap().title, "Hand");
    let written = events_of(&fx, "knowledge.page.written").await;
    assert_eq!(written[0].envelope.source, "system:oxwiki_watch");
    std::fs::remove_file(&path).unwrap();
    assert!(sync(&fx, "hand").await);
    assert!(!recorded(&fx, &oxwiki("hand")).await);
    assert_eq!(events_of(&fx, "knowledge.page.deleted").await.len(), 1);
}

async fn body_of(fx: &crate::test_fixtures::ThreadFixture, page: &str) -> Option<String> {
    let out = fx
        .svc
        .sql
        .query_sql(
            "SELECT body FROM v_knowledge_body WHERE uri = ?1",
            vec![oxplow_db::SqlCell::Text(page.into())],
            None,
        )
        .await
        .unwrap();
    out.rows.first().map(|r| match &r[0] {
        oxplow_db::SqlCell::Text(t) => t.clone(),
        other => panic!("{other:?}"),
    })
}

/// A page's body is in its row (`v_knowledge_body`), written with it by
/// the command and by the watcher.
#[tokio::test]
async fn the_body_reads_back_from_v_knowledge_body() {
    let fx = crate::test_fixtures::services_with_thread().await;
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "notes", "body": "# Notes\n\nFirst.\n" }),
    )
    .await
    .unwrap();
    assert_eq!(
        body_of(&fx, &oxwiki("notes")).await.as_deref(),
        Some("# Notes\n\nFirst.\n")
    );
    std::fs::write(page_path(&fx, "notes"), "# Notes\n\nEdited by hand.\n").unwrap();
    assert!(sync(&fx, "notes").await);
    assert_eq!(
        body_of(&fx, &oxwiki("notes")).await.as_deref(),
        Some("# Notes\n\nEdited by hand.\n")
    );
}

async fn stale_ref_count(fx: &crate::test_fixtures::ThreadFixture, page: &str) -> i64 {
    let page = page.to_string();
    fx.svc
        .db
        .read(move |c| {
            c.query_row(
                "SELECT stale_ref_count FROM v_knowledge_page WHERE uri = ?1",
                [page],
                |r| r.get(0),
            )
            .map_err(sql)
        })
        .await
        .unwrap()
}

/// A snapshot of the primary stream in which `path` changed.
async fn snapshot_with(fx: &crate::test_fixtures::ThreadFixture, path: &str) -> i64 {
    let path = path.to_string();
    fx.svc
        .db
        .transaction(move |c| {
            c.execute(
                "INSERT INTO snapshot (stream_id, created_at)
                 SELECT id, '2026-09-30T00:00:00.000000Z' FROM streams WHERE kind = 'primary'",
                [],
            )
            .map_err(sql)?;
            let id = c.last_insert_rowid();
            c.execute(
                "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                 SELECT id, ?1, 'h', 1, '2026-09-30T00:00:00.000000Z', 'oxplow', ?2
                 FROM streams WHERE kind = 'primary'",
                params![path, id],
            )
            .map_err(sql)?;
            Ok(id)
        })
        .await
        .unwrap()
}

/// Staleness is the primary stream's, like the pin: another stream's newer
/// snapshot of the file neither makes a page stale nor keeps a verified one
/// stale, and every reader agrees.
#[tokio::test]
async fn another_streams_snapshot_doesnt_make_a_page_stale() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    std::fs::write(dir(&fx).join("src/lib.rs"), "v1").unwrap();
    snapshot_with(&fx, "src/lib.rs").await;
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "lib", "body": "# Lib\n\nsee [[src/lib.rs]]\n" }),
    )
    .await
    .unwrap();
    fx.svc
        .db
        .transaction(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, created_at, updated_at)
                   VALUES (99, 'worktree', 'w', 'w', 'r', 'r', '/w',
                           '2026-09-30T00:00:00.000000Z', '2026-09-30T00:00:00.000000Z');
                 INSERT INTO snapshot (id, stream_id, created_at)
                   VALUES (9999, 99, '2026-09-30T00:00:00.000000Z');
                 INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes,
                                            captured_at, storage, snapshot_id)
                   VALUES (99, 'src/lib.rs', 'h2', 1, '2026-09-30T00:00:00.000000Z',
                           'oxplow', 9999);",
            )
            .map_err(sql)
        })
        .await
        .unwrap();
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 0);
    let fresh = freshness(&fx.svc.db, &oxwiki("lib")).await.unwrap();
    assert_eq!(fresh.len(), 1);
    assert!(!fresh[0].stale, "{fresh:?}");
    assert_ne!(fresh[0].latest_snapshot, Some(9999));
}

/// `v_knowledge_page.stale_ref_count` rises when a pinned file drifts and a
/// snapshot lands, and falls when it is verified.
#[tokio::test]
async fn stale_ref_count_rises_when_a_pinned_file_drifts() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    std::fs::write(dir(&fx).join("src/lib.rs"), "v1").unwrap();
    snapshot_with(&fx, "src/lib.rs").await;
    let body = "# Lib\n\nsee [[src/lib.rs]]\n";
    run(&fx, WRITE_PAGE, json!({ "id": "lib", "body": body }))
        .await
        .unwrap();
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 0);

    snapshot_with(&fx, "src/lib.rs").await;
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 1);

    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "lib", "body": body, "verified_files": ["src/lib.rs"] }),
    )
    .await
    .unwrap();
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 0);
    let page = oxwiki("lib");
    let (outbound, kind, bucket): (String, String, String) = fx
        .svc
        .db
        .read(move |c| {
            c.query_row(
                "SELECT outbound_uris, kind, bucket FROM v_knowledge_page WHERE uri = ?1",
                [page],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(sql)
        })
        .await
        .unwrap();
    assert_eq!(outbound, r#"["oxplow:oxplow/file?path=src/lib.rs"]"#);
    assert_eq!((kind.as_str(), bucket.as_str()), ("note", "note"));
}

/// `verify_files` re-pins the refs it names — all of the page's when it
/// names none — against the body the page already has.
#[tokio::test]
async fn verify_files_repins_the_named_refs_or_all_of_them() {
    let fx = crate::test_fixtures::services_with_thread().await;
    std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
    for f in ["a.rs", "b.rs"] {
        std::fs::write(dir(&fx).join("src").join(f), "v1").unwrap();
    }
    snapshot_with(&fx, "src/a.rs").await;
    snapshot_with(&fx, "src/b.rs").await;
    run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "lib", "body": "# Lib\n\nsee [[src/a.rs]] and [[src/b.rs]]\n" }),
    )
    .await
    .unwrap();
    snapshot_with(&fx, "src/a.rs").await;
    snapshot_with(&fx, "src/b.rs").await;
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 2);

    run(
        &fx,
        VERIFY_FILES,
        json!({ "id": "lib", "files": ["src/a.rs"] }),
    )
    .await
    .unwrap();
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 1);

    run(&fx, VERIFY_FILES, json!({ "id": "lib" }))
        .await
        .unwrap();
    assert_eq!(stale_ref_count(&fx, &oxwiki("lib")).await, 0);

    let err = run(
        &fx,
        VERIFY_FILES,
        json!({ "id": "lib", "files": ["src/zzz.rs"] }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("verified_files"), "{err}");
    let err = run(&fx, VERIFY_FILES, json!({ "id": "nope" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no page `nope`"), "{err}");
}

/// A page an agent's command writes is marked touched by its thread.
#[tokio::test]
async fn a_written_page_is_attributed_to_the_writing_thread() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    fx.svc
        .commands
        .run(
            &agent,
            WRITE_PAGE,
            json!({ "id": "arch", "body": "# Arch\n" }),
            false,
        )
        .await
        .unwrap();
    fx.svc.event_pump.run_once().await.unwrap();
    let thread = fx.thread.value();
    let touched: Vec<String> = fx
        .svc
        .db
        .read(move |c| {
            let mut stmt = c
                .prepare("SELECT page FROM v_knowledge_touch WHERE thread_id = ?1")
                .map_err(sql)?;
            let rows = stmt
                .query_map([thread], |r| r.get(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(sql);
            rows
        })
        .await
        .unwrap();
    assert_eq!(touched, vec![oxwiki("arch")]);
}

/// Delete is confirmed and takes the row, the file and the edges; link adds
/// under Related and refuses a dangling target.
#[tokio::test]
async fn delete_and_link() {
    let fx = crate::test_fixtures::services_with_thread().await;
    run(&fx, WRITE_PAGE, json!({ "id": "a", "body": "# A\n" }))
        .await
        .unwrap();
    run(&fx, WRITE_PAGE, json!({ "id": "b", "body": "# B\n" }))
        .await
        .unwrap();

    let err = run(&fx, LINK, json!({ "id": "a", "target": "nope" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
    let out = run(&fx, LINK, json!({ "id": "a", "target": "b" }))
        .await
        .unwrap();
    assert!(out.result["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| *v == oxwiki("b")));
    assert_eq!(
        std::fs::read_to_string(page_path(&fx, "a")).unwrap(),
        "# A\n\n## Related\n\n- [[b]]\n"
    );

    let err = fx
        .svc
        .commands
        .run(&Actor::Human, DELETE_PAGE, json!({ "id": "b" }), false)
        .await
        .unwrap_err();
    assert!(
        matches!(err, CommandError::NeedsConfirmation { .. }),
        "{err:?}"
    );
    run(&fx, DELETE_PAGE, json!({ "id": "b" })).await.unwrap();
    assert!(!recorded(&fx, &oxwiki("b")).await);
    assert!(!page_path(&fx, "b").exists());
    let err = run(&fx, DELETE_PAGE, json!({ "id": "b" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no page"), "{err}");
}

/// A store kept in memory: what a second enabled store does with a write.
struct Shelf {
    id: &'static str,
    edit: bool,
}

#[async_trait::async_trait]
impl KnowledgeProvider for Shelf {
    fn id(&self) -> &str {
        self.id
    }
    fn features(&self) -> oxplow_domain::knowledge::KnowledgeFeatures {
        oxplow_domain::knowledge::KnowledgeFeatures { edit: self.edit }
    }
    async fn write_page(&self, _: &PageCall<'_>, draft: &PageDraft) -> Result<Kept, CommandError> {
        Ok(Kept {
            id: draft.id.clone().unwrap_or_else(|| "derived".into()),
            body: draft.body.clone(),
            title: None,
            kind: None,
            url: Some(format!("shelf://{}", self.id)),
            author: None,
            refs: None,
            events: Vec::new(),
        })
    }
    async fn delete_page(&self, _: &PageCall<'_>, _: &str) -> Result<Vec<Envelope>, CommandError> {
        Ok(Vec::new())
    }
}

/// Several stores are in use at once: two each keep a page with the same id
/// (two pages, two URIs); a write that names no store is refused naming
/// them; a read-only store's pages aren't written.
#[tokio::test]
async fn two_stores_keep_their_own_pages_and_a_write_names_its_store() {
    let fx = crate::test_fixtures::services_with_thread().await;
    fx.svc.knowledge.register(Arc::new(Shelf {
        id: "shelf",
        edit: true,
    }));
    // As a publication would: its row says it's enabled (what the
    // datasets show), and the vocabulary reads `[[shelf:…]]`.
    fx.svc
        .db
        .transaction(|c| {
            c.execute(
                "INSERT INTO capability_provider (capability, provider, active, source, chosen_by)
                 VALUES ('knowledge', 'shelf', 1, 'external', 'declared')",
                [],
            )
            .map_err(sql)
        })
        .await
        .unwrap();
    let mut vocabulary = (*fx.svc.vocabulary.current()).clone();
    vocabulary.resources = vocabulary
        .resources
        .with_knowledge_stores([OXWIKI, "shelf"]);
    fx.svc.vocabulary.swap(vocabulary);
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "id": "architecture", "body": "# A\n" }),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("name the store: oxwiki, shelf"),
        "{err}"
    );
    for store in [OXWIKI, "shelf"] {
        run(
            &fx,
            WRITE_PAGE,
            json!({ "store": store, "id": "architecture", "body": format!("# {store}\n") }),
        )
        .await
        .unwrap();
    }
    let a = fx
        .svc
        .knowledge_pages
        .get(&oxwiki("architecture"))
        .await
        .unwrap()
        .unwrap();
    let b = fx
        .svc
        .knowledge_pages
        .get(&knowledge_uri("shelf", "architecture"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((a.title.as_str(), b.title.as_str()), ("oxwiki", "shelf"));
    assert_eq!(b.url.as_deref(), Some("shelf://shelf"));

    // Another store's page links as `[[<store>:<id>]]`.
    run(
        &fx,
        LINK,
        json!({ "store": OXWIKI, "id": "architecture", "target": "shelf:architecture" }),
    )
    .await
    .unwrap();

    fx.svc.knowledge.register(Arc::new(Shelf {
        id: "rules",
        edit: false,
    }));
    let err = run(
        &fx,
        WRITE_PAGE,
        json!({ "store": "rules", "id": "x", "body": "# X\n" }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
}

/// With no store enabled a write is refused naming the fix.
#[tokio::test]
async fn with_no_store_a_write_is_refused_naming_the_fix() {
    let fx = crate::test_fixtures::services_with_thread().await;
    fx.svc.knowledge.set_declared(Vec::new());
    let err = run(&fx, WRITE_PAGE, json!({ "id": "x", "body": "# X\n" }))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no knowledge store is enabled"),
        "{err}"
    );
}

/// oxwiki is a store that edits and keeps one kind, `note`, as the rows
/// every surface reads say.
#[tokio::test]
async fn oxwiki_is_published_as_an_editing_store_of_notes() {
    let fx = crate::test_fixtures::services_with_thread().await;
    let rows = fx
        .svc
        .sql
        .query_sql(
            "SELECT provider, active, chosen_by, choosable, features, kinds FROM v_capability_provider
              WHERE capability = 'knowledge'",
            Vec::new(),
            None,
        )
        .await
        .unwrap()
        .rows;
    use oxplow_db::SqlCell::{Int, Text};
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0][..4],
        [Text(OXWIKI.into()), Int(1), Text("declared".into()), Int(0)]
    );
    let features: serde_json::Value = match &rows[0][4] {
        Text(t) => serde_json::from_str(t).unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(features["edit"], true);
    let kinds: serde_json::Value = match &rows[0][5] {
        Text(t) => serde_json::from_str(t).unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        kinds,
        json!([{ "id": "note", "title": "Note", "bucket": "note" }])
    );
}
