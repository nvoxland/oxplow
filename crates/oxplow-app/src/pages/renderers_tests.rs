//! Which pages can draw a URI: `for`, `accepts` over the URI's params,
//! and `accepts: { content: … }` over what the URI names.

use std::time::Duration;

use oxplow_domain::Actor;

use super::renderers::*;
use super::schema::{parse_page, Page, PageKind};
use super::spec::ViewContext;
use super::test_support::write;

const BODY: &str = "view: { viz: table, from: q }\nloaders:\n  q: { rows: [{ a: 1 }] }\n";

/// A resource page of `extension` named `slug`: `head` is its `for`,
/// `accepts`, `default` lines.
fn page(extension: &str, slug: &str, head: &str) -> Page {
    parse_page(
        extension,
        extension,
        slug,
        &format!("{slug}.yaml"),
        &format!("title: {slug}\nparams: [{{ name: uri }}]\n{head}\n{BODY}"),
        PageKind::Page,
    )
    .unwrap_or_else(|e| panic!("{slug}: {e}"))
}

fn ids(pages: Vec<&Page>) -> Vec<&str> {
    pages.into_iter().map(|p| p.id.as_str()).collect()
}

fn catalog() -> Vec<Page> {
    vec![
        page(
            "oxplow-foundation",
            "editor",
            "for: oxplow/file\naccepts: { rev: absent, filesystem: absent }\ndefault: true",
        ),
        page("oxplow-foundation", "file-viewer", "for: oxplow/file"),
        page("acme", "blame", "for: oxplow/file"),
        page("acme", "pr", "for: acme/pr\naccepts: { id: '^[0-9]+$' }"),
        page(
            "oxplow-foundation",
            "external-url",
            "for: [https, http]\ndefault: true",
        ),
        // A route: renders nothing.
        parse_page(
            "oxplow-foundation",
            "oxplow-foundation",
            "tasks",
            "tasks.yaml",
            &format!("title: tasks\n{BODY}"),
            PageKind::Page,
        )
        .unwrap(),
    ]
}

#[test]
fn the_resource_a_uri_is_drawn_by() {
    assert_eq!(
        resource_of("oxplow:oxplow/file?path=a.ts").as_deref(),
        Some("oxplow/file")
    );
    assert_eq!(
        resource_of("https://example.com/a").as_deref(),
        Some("https")
    );
    assert_eq!(resource_of("HTTP://example.com/a").as_deref(), Some("http"));
    assert_eq!(resource_of("ftp://example.com/a"), None);
    assert_eq!(resource_of("not a uri"), None);
}

#[test]
fn the_pages_for_a_uris_resource_whose_accepts_hold_for_its_params() {
    let pages = catalog();
    assert_eq!(
        ids(candidates(&pages, "oxplow:oxplow/file?path=a.ts")),
        [
            "oxplow-foundation/editor",
            "oxplow-foundation/file-viewer",
            "acme/blame"
        ]
    );
    assert_eq!(
        ids(candidates(
            &pages,
            "oxplow:oxplow/file?path=a.ts&rev=git:HEAD"
        )),
        ["oxplow-foundation/file-viewer", "acme/blame"]
    );
    // The editor edits only the local working tree: another filesystem's file is the viewer's.
    assert_eq!(
        ids(candidates(
            &pages,
            "oxplow:oxplow/file?path=a.ts&filesystem=oxplow:temp"
        )),
        ["oxplow-foundation/file-viewer", "acme/blame"]
    );
    assert_eq!(ids(candidates(&pages, "oxplow:acme/pr?id=12")), ["acme/pr"]);
    assert!(candidates(&pages, "oxplow:acme/pr?id=x").is_empty());
    assert_eq!(
        ids(candidates(&pages, "https://example.com")),
        ["oxplow-foundation/external-url"]
    );
    assert!(candidates(&pages, "oxplow:oxplow/commit?sha=abc").is_empty());
    assert!(candidates(&pages, "ftp://example.com").is_empty());
}

/// `present` wants the param there; a pattern wants all of its value.
#[test]
fn present_absent_and_a_pattern_over_the_params() {
    let pages = vec![
        page(
            "acme",
            "has-rev",
            "for: oxplow/file\naccepts: { rev: present }",
        ),
        page(
            "acme",
            "no-rev",
            "for: oxplow/file\naccepts: { rev: absent }",
        ),
        page(
            "acme",
            "tsx",
            "for: oxplow/file\naccepts: { path: '^[a-z/]+\\.tsx?$' }",
        ),
    ];
    assert_eq!(
        ids(candidates(
            &pages,
            "oxplow:oxplow/file?path=a/b.ts&rev=working"
        )),
        ["acme/has-rev", "acme/tsx"]
    );
    assert_eq!(
        ids(candidates(&pages, "oxplow:oxplow/file?path=a/b.tsx")),
        ["acme/no-rev", "acme/tsx"]
    );
    // Not the whole value: the pattern is anchored.
    assert_eq!(
        ids(candidates(&pages, "oxplow:oxplow/file?path=a/b.tsx.map")),
        ["acme/no-rev"]
    );
    // A pattern needs the param.
    assert_eq!(
        ids(candidates(&pages, "oxplow:oxplow/file")),
        ["acme/no-rev"]
    );
}

// ---- content ------------------------------------------------------------

/// Pages that look at the content of the file they are offered for, each
/// confined to its own `path` so one test's candidates are only its own.
const CONTENT_PAGES: &[(&str, &str)] = &[
    (
        "yaml",
        "for: oxplow/file\naccepts: { path: '^yaml\\.[a-z]+$', content: is_yaml }\nloaders:\n  is_yaml: { sql: \"SELECT 1 AS n FROM content WHERE language = 'yaml'\" }\n",
    ),
    (
        "broken",
        "for: oxplow/file\naccepts: { path: '^broken\\.yaml$', content: bad }\nloaders:\n  bad: { sql: \"SELECT * FROM no_such_table\" }\n",
    ),
    (
        "needs-text",
        "for: oxplow/file\naccepts: { path: '^text\\.[a-z]+$', content: has_x }\ncontent: { text: true }\nloaders:\n  has_x: { sql: \"SELECT 1 AS n FROM content WHERE text LIKE '%x%'\" }\n",
    ),
    // Reads `text` but doesn't ask for it: the column isn't there.
    (
        "forgot-text",
        "for: oxplow/file\naccepts: { path: '^forgot\\.[a-z]+$', content: has_x }\nloaders:\n  has_x: { sql: \"SELECT 1 AS n FROM content WHERE text LIKE '%x%'\" }\n",
    ),
    (
        "chained",
        "for: oxplow/file\naccepts: { path: '^chain\\.[a-z]+$', content: last }\nloaders:\n  first: { sql: \"SELECT language AS l FROM content\" }\n  last: { sql: \"SELECT l FROM first WHERE l = 'yaml'\" }\n  elsewhere: { sql: \"SELECT * FROM no_such_table\" }\n",
    ),
    (
        "no-content",
        "for: oxplow/file\naccepts: { path: '^rows\\.[a-z]+$', content: some }\nloaders:\n  some: { rows: [{ n: 1 }] }\n  none: { sql: \"SELECT 1 AS n WHERE 0\" }\n",
    ),
    (
        "plain",
        "for: oxplow/file\naccepts: { path: '^plain\\.[a-z]+$' }\n",
    ),
    // Listed first by name, but it reads the file's text to decide: after
    // the pages that take any such file, a check on the URI's params
    // alone or on what the file is (its stat) among them.
    (
        "a-checked",
        "for: oxplow/file\ncontent: { text: true, max_bytes: 8 }\naccepts: { path: '^ordered\\.[a-z]+$', content: has_text }\nloaders:\n  has_text: { sql: \"SELECT 1 AS n FROM content WHERE text IS NOT NULL\" }\n",
    ),
    (
        "ab-by-stat",
        "for: oxplow/file\naccepts: { path: '^ordered\\.[a-z]+$', content: is_text }\nloaders:\n  is_text: { sql: \"SELECT 1 AS n FROM content WHERE is_text\" }\n",
    ),
    (
        "b-general",
        "for: oxplow/file\naccepts: { path: '^ordered\\.[a-z]+$' }\n",
    ),
    (
        "c-by-params",
        "for: oxplow/file\naccepts: { path: '^ordered\\.[a-z]+$', content: named }\nloaders:\n  named: { sql: \"SELECT 1 AS n WHERE :uri LIKE '%ordered%'\" }\n",
    ),
    // A check that would count to ten million.
    (
        "slow",
        "for: oxplow/file\naccepts: { path: '^slow\\.[a-z]+$', content: counted }\nloaders:\n  counted: { sql: \"WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 10000000) SELECT count(*) AS n FROM c\" }\n",
    ),
];

async fn project() -> (tempfile::TempDir, std::sync::Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    write(
        dir.path(),
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
    );
    for (slug, head) in CONTENT_PAGES {
        write(
            dir.path(),
            &format!(".oxplow/extensions/acme/pages/{slug}.yaml"),
            &format!("title: {slug}\nparams: [{{ name: uri }}]\n{head}{BODY_VIEW}"),
        );
    }
    for (path, text) in [
        ("yaml.yaml", "a: 1\n"),
        ("yaml.rs", "fn main() {}\n"),
        ("broken.yaml", "a: 1\n"),
        ("text.txt", "has an x\n"),
        ("text.md", "nothing here\n"),
        ("forgot.txt", "has an x\n"),
        ("chain.yaml", "a: 1\n"),
        ("chain.rs", "fn main() {}\n"),
        ("rows.txt", "t\n"),
        ("plain.txt", "t\n"),
        ("ordered.txt", "t\n"),
        ("slow.txt", "t\n"),
    ] {
        write(dir.path(), path, text);
    }
    crate::test_fixtures::commit_all(dir.path(), "files");
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    let problems = crate::test_fixtures::activate_with_problems(&svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    (dir, svc)
}

/// Every content page lists the `content` row, so it parses as a page that
/// shows something.
const BODY_VIEW: &str = "view: { viz: table, from: content }\n";

/// What acme's pages offer for the file `path`.
async fn acme_for(svc: &crate::Services, path: &str) -> Vec<String> {
    acme_within(svc, path, ACCEPT_TIMEOUT).await
}

async fn acme_within(svc: &crate::Services, path: &str, timeout: Duration) -> Vec<String> {
    let pages: Vec<Page> = svc
        .extension_catalog
        .unpinned(&svc.layout.project_dir, "test")
        .iter()
        .filter(|e| e.name == "acme")
        .flat_map(|e| e.page_files.iter().cloned())
        .collect();
    renderers_in(
        svc,
        &pages,
        &format!("oxplow:oxplow/file?path={path}"),
        &ViewContext::default(),
        &Actor::Human,
        timeout,
    )
    .await
    .into_iter()
    .map(|r| r.page)
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_is_offered_when_its_loader_has_a_row_and_not_when_it_has_none() {
    let (_dir, svc) = project().await;
    assert_eq!(acme_for(&svc, "yaml.yaml").await, ["acme/yaml"]);
    // The same page, a file its loader finds no row for.
    assert!(acme_for(&svc, "yaml.rs").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_whose_loader_fails_is_not_offered() {
    let (_dir, svc) = project().await;
    assert!(acme_for(&svc, "broken.yaml").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_whose_check_takes_too_long_is_not_offered() {
    let (_dir, svc) = project().await;
    let started = std::time::Instant::now();
    assert!(acme_within(&svc, "slow.txt", Duration::from_millis(50))
        .await
        .is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    // A page with no content check isn't held up by the time it has.
    assert_eq!(
        acme_within(&svc, "plain.txt", Duration::from_millis(50)).await,
        ["acme/plain"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn text_is_read_only_when_a_surviving_page_asks_for_it() {
    let (_dir, svc) = project().await;
    assert_eq!(acme_for(&svc, "text.txt").await, ["acme/needs-text"]);
    assert!(acme_for(&svc, "text.md").await.is_empty());
    // No page asked for text: the content has no `text` column to read.
    assert!(acme_for(&svc, "forgot.txt").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_chain_of_the_accepts_loader_runs() {
    let (_dir, svc) = project().await;
    // `elsewhere` would fail; it isn't what `last` reads.
    assert_eq!(acme_for(&svc, "chain.yaml").await, ["acme/chained"]);
    assert!(acme_for(&svc, "chain.rs").await.is_empty());
    // A rows loader answers with no content read at all.
    assert_eq!(acme_for(&svc, "rows.txt").await, ["acme/no-content"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_with_no_content_check_is_offered_for_its_params_alone() {
    let (_dir, svc) = project().await;
    assert_eq!(acme_for(&svc, "plain.txt").await, ["acme/plain"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_reads_the_files_text_to_decide_comes_after_the_ones_that_take_any_such_file() {
    let (_dir, svc) = project().await;
    // A check on the URI's params alone, or on what the file is (its
    // stat), is still a general view of the file.
    assert_eq!(
        acme_for(&svc, "ordered.txt").await,
        [
            "acme/ab-by-stat",
            "acme/b-general",
            "acme/c-by-params",
            "acme/a-checked"
        ]
    );
}

/// The surface the desktop and agents share: the whole catalog, foundation's
/// pages included, in the extensions' order.
#[tokio::test(flavor = "multi_thread")]
async fn renderers_for_lists_title_and_default_of_each_offered_page() {
    let (_dir, svc) = project().await;
    let all = renderers_for(
        &svc,
        "oxplow:oxplow/file?path=yaml.yaml",
        &ViewContext::default(),
        &Actor::Human,
    )
    .await;
    assert!(all
        .iter()
        .any(|r| r.page == "acme/yaml" && r.title == "yaml" && !r.default));
    assert!(
        all.iter()
            .any(|r| r.page == "oxplow-foundation/editor" && r.default),
        "{all:?}"
    );
    assert!(renderers_for(
        &svc,
        "oxplow:oxplow/nothing?id=1",
        &ViewContext::default(),
        &Actor::Human
    )
    .await
    .is_empty());
}
