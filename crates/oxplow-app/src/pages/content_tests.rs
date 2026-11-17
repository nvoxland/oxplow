//! The `content` loader of a resource page: the record the URI it shows
//! names, per kind of resource.

use oxplow_db::{SqlCell, SqlQueryResult};

use super::content::{resolve_content, ContentOpts};

const NO_TEXT: ContentOpts = ContentOpts {
    text: false,
    max_bytes: 1024,
};
const TEXT: ContentOpts = ContentOpts {
    text: true,
    max_bytes: 1024,
};

async fn project() -> (tempfile::TempDir, std::sync::Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("src/blob.bin"), [0u8, 1, 2, 3]).unwrap();
    crate::test_fixtures::commit_all(dir.path(), "first");
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    (dir, svc)
}

fn cell<'r>(r: &'r SqlQueryResult, column: &str) -> &'r SqlCell {
    let i = r
        .columns
        .iter()
        .position(|c| c == column)
        .unwrap_or_else(|| panic!("no column `{column}` in {:?}", r.columns));
    &r.rows[0][i]
}

fn text(c: &SqlCell) -> &str {
    match c {
        SqlCell::Text(t) => t,
        other => panic!("not text: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_its_metadata_and_text_only_on_request() {
    let (_dir, svc) = project().await;
    let uri = "oxplow:oxplow/file?path=src/a.txt";
    let r = resolve_content(&svc, uri, None, NO_TEXT).await.unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(text(cell(&r, "uri")), uri);
    assert_eq!(text(cell(&r, "path")), "src/a.txt");
    assert_eq!(text(cell(&r, "filesystem")), "oxplow:local");
    assert_eq!(text(cell(&r, "kind")), "file");
    assert_eq!(cell(&r, "size"), &SqlCell::Int(8));
    assert_eq!(cell(&r, "is_text"), &SqlCell::Bool(true));
    assert!(!r.columns.iter().any(|c| c == "text"), "{:?}", r.columns);

    let r = resolve_content(&svc, uri, None, TEXT).await.unwrap();
    assert_eq!(text(cell(&r, "text")), "one\ntwo\n");
    assert_eq!(cell(&r, "truncated"), &SqlCell::Bool(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_files_text_is_cut_at_max_bytes_and_says_so() {
    let (_dir, svc) = project().await;
    let opts = ContentOpts {
        text: true,
        max_bytes: 5,
    };
    let r = resolve_content(&svc, "oxplow:oxplow/file?path=src/a.txt", None, opts)
        .await
        .unwrap();
    assert_eq!(text(cell(&r, "text")), "one\nt");
    assert_eq!(cell(&r, "truncated"), &SqlCell::Bool(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_binary_file_has_metadata_and_no_text() {
    let (_dir, svc) = project().await;
    let r = resolve_content(&svc, "oxplow:oxplow/file?path=src/blob.bin", None, TEXT)
        .await
        .unwrap();
    assert_eq!(cell(&r, "is_text"), &SqlCell::Bool(false));
    assert_eq!(cell(&r, "size"), &SqlCell::Int(4));
    assert_eq!(cell(&r, "text"), &SqlCell::Null(()));
    assert_eq!(cell(&r, "truncated"), &SqlCell::Bool(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_file_is_no_row() {
    let (_dir, svc) = project().await;
    let r = resolve_content(&svc, "oxplow:oxplow/file?path=src/gone.txt", None, TEXT)
        .await
        .unwrap();
    assert!(r.rows.is_empty());
    assert!(r.columns.iter().any(|c| c == "path"), "{:?}", r.columns);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_read_on_the_filesystem_and_revision_its_uri_names() {
    let (_dir, svc) = project().await;
    svc.temp.put("view-1.yaml", b"title: One\n").await.unwrap();
    let r = resolve_content(
        &svc,
        "oxplow:oxplow/file?path=view-1.yaml&filesystem=oxplow:temp",
        None,
        TEXT,
    )
    .await
    .unwrap();
    assert_eq!(text(cell(&r, "filesystem")), "oxplow:temp");
    assert_eq!(text(cell(&r, "text")), "title: One\n");
    assert_eq!(text(cell(&r, "language")), "yaml");

    std::fs::write(_dir.path().join("src/a.txt"), "changed\n").unwrap();
    let r = resolve_content(
        &svc,
        "oxplow:oxplow/file?path=src/a.txt&rev=git:HEAD",
        None,
        TEXT,
    )
    .await
    .unwrap();
    assert_eq!(text(cell(&r, "rev")), "git:HEAD");
    assert_eq!(text(cell(&r, "text")), "one\ntwo\n");

    let err = resolve_content(
        &svc,
        "oxplow:oxplow/file?path=a.txt&filesystem=acme:nope",
        None,
        TEXT,
    )
    .await
    .unwrap_err();
    assert!(err.contains("no filesystem `acme:nope`"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dataset_backed_resource_is_its_datasets_row() {
    let (_dir, svc) = project().await;
    crate::commit_indexer::refresh(&svc).await;
    let found = svc
        .sql
        .query_sql(
            "SELECT uri, sha, subject FROM v_commit LIMIT 1",
            vec![],
            None,
        )
        .await
        .unwrap();
    let [uri, sha, subject] = &found.rows[0][..] else {
        panic!("{found:?}")
    };
    assert_eq!(text(subject), "first");
    let r = resolve_content(&svc, text(uri), None, NO_TEXT)
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(cell(&r, "sha"), sha);
    assert_eq!(text(cell(&r, "subject")), "first");
    assert!(
        r.reads.datasets.iter().any(|d| d == "v_commit"),
        "{:?}",
        r.reads
    );
    // No such commit: no row.
    let r = resolve_content(&svc, "oxplow:oxplow/commit?sha=0000", None, NO_TEXT)
        .await
        .unwrap();
    assert!(r.rows.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn any_other_uri_is_a_row_of_itself() {
    let (_dir, svc) = project().await;
    for (given, identity) in [
        ("oxplow:oxplow/dir?path=src", "oxplow:oxplow/dir?path=src"),
        (
            "oxplow:oxplow/dir?path=src#x=1",
            "oxplow:oxplow/dir?path=src",
        ),
        ("https://example.com/a?b=1", "https://example.com/a?b=1"),
    ] {
        let r = resolve_content(&svc, given, None, TEXT).await.unwrap();
        assert_eq!(r.columns, ["uri"], "{given}");
        assert_eq!(r.rows, [[SqlCell::Text(identity.into())]], "{given}");
    }
}
