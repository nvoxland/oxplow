//! The Directory page is oxplow-foundation's `directory` lens, the renderer
//! of `oxplow/dir`: a folder's rows, folders first, each naming the URI it
//! opens.

use oxplow_db::SqlCell;

use crate::test_fixtures::services_with_effort;

#[tokio::test]
async fn a_folder_lists_its_folders_then_its_files_each_with_its_uri() {
    let fx = services_with_effort().await;
    std::fs::create_dir_all(fx._dir.path().join("src/sub")).unwrap();
    std::fs::write(fx._dir.path().join("src/b.rs"), "b").unwrap();
    std::fs::write(fx._dir.path().join("src/a.rs"), "a").unwrap();
    std::fs::write(fx._dir.path().join("top.txt"), "t").unwrap();

    let svc = &fx.svc;
    let stream = svc.streams.ensure_primary().await.unwrap().id;
    let run = crate::test_fixtures::run_as_page(
        svc,
        "oxplow-foundation/directory",
        &[
            ("path".to_string(), SqlCell::Text("src".into())),
            ("stream_id".to_string(), SqlCell::Int(stream.value())),
        ],
    )
    .await;
    let col = |name: &str| run.columns.iter().position(|c| c == name).unwrap();
    let text = |row: &Vec<SqlCell>, name: &str| match &row[col(name)] {
        SqlCell::Text(t) => t.clone(),
        other => panic!("{name}: {other:?}"),
    };
    let rows: Vec<(String, String, String)> = run
        .rows
        .iter()
        .map(|r| (text(r, "name"), text(r, "icon"), text(r, "target")))
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "sub".to_string(),
                "folder".to_string(),
                "oxplow:oxplow/dir?path=src/sub".to_string()
            ),
            (
                "a.rs".to_string(),
                "file".to_string(),
                "oxplow:oxplow/file?path=src/a.rs".to_string()
            ),
            (
                "b.rs".to_string(),
                "file".to_string(),
                "oxplow:oxplow/file?path=src/b.rs".to_string()
            ),
        ]
    );
}
