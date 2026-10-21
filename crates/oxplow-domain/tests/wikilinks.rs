//! `[[…]]` authoring sugar against oxplow-foundation's declared resources.

use oxplow_domain::uri::resources::ResourceRegistry;
use oxplow_domain::uri::{canonical_wikilink, classify_wikilinks, OxplowUri, Reference};

/// oxplow-foundation's resources with oxplow's tasks as the work list
/// (`tsk<n>`).
fn tasks() -> ResourceRegistry {
    oxplow_domain::uri::resources::foundation()
        .clone()
        .with_work_item_ids("oxplow", r"tsk\d+")
        .expect("the pattern compiles")
}

#[test]
fn wikilink_sugar_translates_to_uris() {
    let cases = [
        ("tsk42", "oxplow:oxplow/work_item?id=tsk42&provider=oxplow"),
        ("git:abc1234", "oxplow:oxplow/commit?sha=abc1234"),
        ("abc1234", "oxplow:oxplow/commit?sha=abc1234"),
        // A resource's own name with its key.
        ("commit:abc1234", "oxplow:oxplow/commit?sha=abc1234"),
        ("wiki:architecture", "oxplow:oxplow/wiki?slug=architecture"),
        ("effort:eff3", "oxplow:oxplow/effort?id=eff3"),
        ("dir:src/", "oxplow:oxplow/dir?path=src"),
        ("dir:src", "oxplow:oxplow/dir?path=src"),
        ("finding:f-9", "oxplow:oxplow/finding?id=f-9"),
        ("architecture", "oxplow:oxplow/wiki?slug=architecture"),
        ("src/a.rs", "oxplow:oxplow/file?path=src/a.rs"),
        ("file:src/a.rs", "oxplow:oxplow/file?path=src/a.rs"),
        ("src/a.rs:42", "oxplow:oxplow/file?path=src/a.rs#lines=42"),
        (
            "src/a.rs@HEAD",
            "oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD",
        ),
        (
            "src/a.rs@HEAD:42",
            "oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=42",
        ),
        ("src/a.rs@disk", "oxplow:oxplow/file?path=src/a.rs"),
        (
            "src/a.rs@local:7",
            "oxplow:oxplow/file?path=src/a.rs#lines=7",
        ),
        // A URI itself, of a declared resource.
        (
            "oxplow:oxplow/work_item?id=ENG-12&provider=issues",
            "oxplow:oxplow/work_item?id=ENG-12&provider=issues",
        ),
        (
            "oxplow:oxplow/file?path=src/a@b.rs&rev=git:HEAD#lines=1",
            "oxplow:oxplow/file?path=src/a@b.rs&rev=git:HEAD#lines=1",
        ),
    ];
    for (interior, expected) in cases {
        let got = canonical_wikilink(&tasks(), interior)
            .unwrap_or_else(|| panic!("[[{interior}]] should translate"));
        assert_eq!(got.to_string(), expected, "[[{interior}]]");
    }
    for bad in [
        "",
        "#13",
        "not a ref!",
        "tskfoo/x",
        "Unknown_Kind:1",
        "oxplow:acme/pr?id=1",
        "effort:3",
    ] {
        assert!(
            canonical_wikilink(&tasks(), bad).is_none(),
            "[[{bad}]] should not translate"
        );
    }
}

#[test]
fn classify_wikilinks_reports_the_uri_beside_the_typed_view() {
    let body = "See [[tsk42]], [[git:abc1234]], [[abc1234]], [[dir:src]] and [[missing thing]].";
    let links = classify_wikilinks(&tasks(), body);
    let uris: Vec<Option<String>> = links
        .iter()
        .map(|l| l.canonical.as_ref().map(ToString::to_string))
        .collect();
    let item = "oxplow:oxplow/work_item?id=tsk42&provider=oxplow";
    assert_eq!(
        uris,
        vec![
            Some(item.into()),
            Some("oxplow:oxplow/commit?sha=abc1234".into()),
            Some("oxplow:oxplow/commit?sha=abc1234".into()),
            Some("oxplow:oxplow/dir?path=src".into()),
            None
        ]
    );
    // The typed view is derived from the URI, never parsed twice.
    assert_eq!(links[0].reference, Some(Reference::WorkItem(item.into())));
    let uri = |s: &str| OxplowUri::parse(s).unwrap();
    assert_eq!(
        Reference::try_from(&uri("oxplow:oxplow/commit?sha=abc1234")).unwrap(),
        Reference::Commit("abc1234".into())
    );
    assert!(
        Reference::try_from(&uri("oxplow:oxplow/effort?id=eff3")).is_err(),
        "no typed view yet"
    );
}
