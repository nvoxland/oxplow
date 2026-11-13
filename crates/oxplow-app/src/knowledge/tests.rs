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
