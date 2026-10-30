//! `extension add <kind> <id>`: one contribution into an extension that
//! exists, with its comments kept, and the extension still checking clean.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use oxplow_sdk::{check, render_findings, scaffold, ExtensionCatalog, Format, Kind};

async fn checked(root: &Path, name: &str) {
    let report = check(root, name, &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(
        report.ok,
        "{name}: {}",
        render_findings(&report, Format::Text)
    );
}

fn manifest(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(format!(".oxplow/extensions/{name}/extension.yaml"))).unwrap()
}

/// Each addable kind goes into a bare extension and the result checks.
#[tokio::test(flavor = "multi_thread")]
async fn each_kind_adds_to_an_extension_that_still_checks() {
    for (kind, id) in [
        ("page", "overview"),
        ("collector", "sales"),
        ("command", "notes.add"),
        ("effect", "on-done"),
        ("component", "board"),
        ("advisory", "heads-up"),
        ("skill", "triage-guide"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold(root, Kind::Extension, "base", None).unwrap();
        let kind_parsed = Kind::parse(kind).unwrap();
        let added = oxplow_sdk::add_contribution(root, kind_parsed, id, None)
            .unwrap_or_else(|e| panic!("add {kind} {id}: {e}"));
        assert_eq!(added.name, "base");
        assert!(!added.files.is_empty(), "{kind} writes something");
        checked(root, "base").await;
    }
}

/// A second extension with the same id is refused, and nothing is written.
#[test]
fn an_id_the_list_holds_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root, Kind::Collector, "base", None).unwrap();
    let before = manifest(root, "base");
    let err = oxplow_sdk::add_contribution(root, Kind::Collector, "items", None).unwrap_err();
    assert!(err.to_string().contains("`items`"), "{err}");
    assert_eq!(manifest(root, "base"), before);
    // The same id twice through `add`.
    oxplow_sdk::add_contribution(root, Kind::Collector, "sales", None).unwrap();
    let err = oxplow_sdk::add_contribution(root, Kind::Collector, "sales", None).unwrap_err();
    assert!(err.to_string().contains("`sales`"), "{err}");
}

/// A file that exists is refused, and the manifest is not touched.
#[test]
fn an_existing_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root, Kind::Extension, "base", None).unwrap();
    let page = root.join(".oxplow/extensions/base/pages/overview.yaml");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "mine\n").unwrap();
    let err = oxplow_sdk::add_contribution(root, Kind::Page, "overview", None).unwrap_err();
    assert!(err.to_string().contains("pages/overview.yaml"), "{err}");
    assert_eq!(std::fs::read_to_string(&page).unwrap(), "mine\n");
}

/// The manifest's comments survive an add, and a permission joins the line
/// that exists.
#[tokio::test(flavor = "multi_thread")]
async fn comments_survive_and_a_permission_joins_the_existing_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root, Kind::Extension, "base", None).unwrap();
    let path = root.join(".oxplow/extensions/base/extension.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        format!("{text}# keep this note\npermissions: [sql.read] # and this one\n"),
    )
    .unwrap();
    oxplow_sdk::add_contribution(root, Kind::Command, "notes.add", None).unwrap();
    let after = manifest(root, "base");
    assert!(after.contains("# keep this note\n"), "{after}");
    assert!(
        after.contains(
            "permissions: [sql.read, commands:oxplow.work_item.comment] # and this one\n"
        ),
        "{after}"
    );
    checked(root, "base").await;
}

/// Without an extension named, the only one is used; with several, one is
/// asked for.
#[test]
fn the_extension_is_the_only_one_or_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root, Kind::Extension, "one", None).unwrap();
    scaffold(root, Kind::Extension, "two", None).unwrap();
    let err = oxplow_sdk::add_contribution(root, Kind::Page, "overview", None).unwrap_err();
    assert!(err.to_string().contains("one, two"), "{err}");
    let added = oxplow_sdk::add_contribution(root, Kind::Page, "overview", Some("two")).unwrap();
    assert_eq!(added.name, "two");
    let err = oxplow_sdk::add_contribution(root, Kind::Page, "x", Some("nope")).unwrap_err();
    assert!(err.to_string().contains("no extension `nope`"), "{err}");
}

/// A kind with no contribution of its own is refused, saying which are.
#[test]
fn a_kind_that_is_a_whole_extension_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scaffold(root, Kind::Extension, "base", None).unwrap();
    let err = oxplow_sdk::add_contribution(root, Kind::Metric, "loc", None).unwrap_err();
    assert!(err.to_string().contains("`collector`"), "{err}");
}
