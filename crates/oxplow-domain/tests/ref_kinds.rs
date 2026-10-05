//! The kind registry and the `[[…]]` sugar → canonical ref translation
//! (`.context/refs.md`). The registry is the one list of what a ref
//! can name; the translation is what lets `[[tsk42]]`, `[[git:abc1234]]`
//! and `[[src/a.rs@HEAD:42]]` keep working while everything downstream
//! sees `work_item:oxplow:tsk42`, `commit:abc1234`,
//! `file:src/a.rs@git:HEAD#L42`.

use oxplow_domain::refs::grammar::CanonicalRef;
use oxplow_domain::refs::kind::{core_kinds, KindSpec};
use oxplow_domain::refs::{canonical_wikilink, classify_wikilinks, Reference};

fn canon(s: &str) -> CanonicalRef {
    CanonicalRef::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

#[test]
fn the_core_kinds_are_registered_and_validate_ids() {
    let reg = core_kinds();
    for k in [
        "project",
        "stream",
        "thread",
        "effort",
        "turn",
        "snapshot",
        "commit",
        "branch",
        "file",
        "dir",
        "symbol",
        "work_item",
        "wiki",
        "comment",
        "event",
        "lens",
        "page",
        "metric",
        "model",
        "plugin",
        "command",
        "finding",
        "task_note",
        "run",
        "proposal",
    ] {
        assert!(reg.get(k).is_some(), "core kind {k} missing");
    }
    // A pending command waiting for a person (`command_proposal.id`, P6b).
    assert!(reg.validate(&canon("proposal:12")).is_ok());
    assert!(reg.validate(&canon("proposal:x")).is_err());
    // The id regex applies: a commit is hex, a work item is <provider>:<id>.
    assert!(reg.validate(&canon("commit:4c44d495")).is_ok());
    assert!(reg.validate(&canon("commit:not-hex!")).is_err());
    // A SHA-256 repository's commit ids are 64 hex chars.
    assert!(reg
        .validate(&canon(&format!("commit:{}", "a".repeat(64))))
        .is_ok());
    assert!(reg
        .validate(&canon(&format!("commit:{}", "a".repeat(65))))
        .is_err());
    // A command name has two or more dot segments, like `CommandSpec` allows.
    assert!(reg.validate(&canon("command:config.set")).is_ok());
    assert!(reg.validate(&canon("command:dashboard.item.add")).is_ok());
    assert!(reg.validate(&canon("command:nodots")).is_err());
    // An extension's page (P6.G2): `page:ext.<extension>.<page>`.
    assert!(reg.validate(&canon("page:ext.acme.open-prs")).is_ok());
    assert!(reg.validate(&canon("page:ext.acme.Open PRs")).is_err());
    // A dot only in that shape: not a stray one, not a half-named page.
    for bad in [
        "page:ext.acme",
        "page:settings.",
        "page:..",
        "page:ext.a.b.c",
        "page:a.b",
    ] {
        assert!(reg.validate(&canon(bad)).is_err(), "{bad}");
    }
    assert!(reg.validate(&canon("page:settings?tab=ai")).is_ok());
    assert!(reg.validate(&canon("work_item:oxplow:tsk42")).is_ok());
    assert!(
        reg.validate(&canon("work_item:tsk42")).is_err(),
        "no provider"
    );
    // Only revisioned kinds may carry @rev.
    assert!(reg.validate(&canon("file:src/a.rs@git:HEAD")).is_ok());
    assert!(reg.validate(&canon("wiki:arch@git:HEAD")).is_err());
    // work_item is the only provider-scoped core kind in P1.
    assert!(reg.get("work_item").unwrap().provider_scoped);
    assert!(!reg.get("wiki").unwrap().provider_scoped);
    assert!(!reg.get("commit").unwrap().provider_scoped);
}

#[test]
fn registering_a_colliding_kind_is_an_error() {
    let mut reg = core_kinds();
    let dup = KindSpec::new("commit", r"^[0-9a-f]{7,40}$").expect("spec");
    let err = reg.register(dup).unwrap_err();
    assert!(err.to_string().contains("commit"), "{err}");
    // A plugin kind with a fresh name registers.
    let acme = KindSpec::new("acme_widget", r"^\d+$").expect("spec");
    reg.register(acme).unwrap();
    assert!(reg.validate(&canon("acme_widget:7")).is_ok());
}

#[test]
fn a_kind_spec_rejects_a_bad_name_or_regex() {
    assert!(KindSpec::new("Git-Commit", "^x$").is_err());
    assert!(KindSpec::new("ok", "^(").is_err());
}

#[test]
fn wikilink_sugar_translates_to_canonical_refs() {
    let cases = [
        ("tsk42", "work_item:oxplow:tsk42"),
        ("git:abc1234", "commit:abc1234"),
        ("abc1234", "commit:abc1234"),
        ("commit:abc1234", "commit:abc1234"),
        ("dir:src/", "dir:src"),
        ("dir:src", "dir:src"),
        ("finding:f-9", "finding:f-9"),
        ("architecture", "wiki:architecture"),
        ("wiki:architecture", "wiki:architecture"),
        ("src/a.rs", "file:src/a.rs"),
        ("src/a.rs:42", "file:src/a.rs#L42"),
        ("src/a.rs@HEAD", "file:src/a.rs@git:HEAD"),
        ("src/a.rs@HEAD:42", "file:src/a.rs@git:HEAD#L42"),
        ("src/a.rs@disk", "file:src/a.rs"),
        ("src/a.rs@local:7", "file:src/a.rs#L7"),
        // Already-canonical refs pass through, including provider-scoped ones.
        ("work_item:issues:ENG-12", "work_item:issues:ENG-12"),
        (
            "file:src/a%40b.rs@git:HEAD#L1",
            "file:src/a%40b.rs@git:HEAD#L1",
        ),
        ("effort:eff3", "effort:eff3"),
    ];
    for (interior, expected) in cases {
        let got = canonical_wikilink(&oxplow_domain::refs::kind::core_kinds(), interior)
            .unwrap_or_else(|| panic!("[[{interior}]] should translate"));
        assert_eq!(got.to_string(), expected, "[[{interior}]]");
    }
    for bad in ["", "#13", "not a ref!", "tskfoo/x", "Unknown_Kind:1"] {
        assert!(
            canonical_wikilink(&oxplow_domain::refs::kind::core_kinds(), bad).is_none(),
            "[[{bad}]] should not translate"
        );
    }
}

#[test]
fn classify_wikilinks_reports_the_canonical_ref_beside_the_typed_view() {
    let body = "See [[tsk42]], [[git:abc1234]], [[abc1234]], [[dir:src]] and [[missing thing]].";
    let links = classify_wikilinks(&oxplow_domain::refs::kind::core_kinds(), body);
    let refs: Vec<Option<String>> = links
        .iter()
        .map(|l| l.canonical.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(
        refs,
        vec![
            Some("work_item:oxplow:tsk42".into()),
            Some("commit:abc1234".into()),
            Some("commit:abc1234".into()),
            Some("dir:src".into()),
            None
        ]
    );
    // The typed view is derived from the canonical ref, never parsed twice.
    assert_eq!(links[0].reference, Some(Reference::Task(42)));
    assert_eq!(
        Reference::try_from(&canon("commit:abc1234")).unwrap(),
        Reference::Commit("abc1234".into())
    );
    assert!(
        Reference::try_from(&canon("effort:eff3")).is_err(),
        "no typed view yet"
    );
}
