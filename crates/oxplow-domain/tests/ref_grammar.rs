//! Golden tests for the canonical ref grammar (`.context/refs.md`).
//! The fixture is shared with the TS parser (`apps/desktop/src/refs/ref.test.ts`).

use oxplow_domain::refs::grammar::{CanonicalRef, RefParseError};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    valid: Vec<ValidCase>,
    invalid: Vec<InvalidCase>,
    format: Vec<FormatCase>,
}

#[derive(Deserialize)]
struct ValidCase {
    text: String,
    kind: String,
    id: String,
    rev: Option<String>,
    frag: Option<String>,
}

#[derive(Deserialize)]
struct InvalidCase {
    text: String,
    reason: String,
}

#[derive(Deserialize)]
struct FormatCase {
    kind: String,
    id: String,
    rev: Option<String>,
    frag: Option<String>,
    text: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/ref_grammar.json")).expect("fixture parses")
}

#[test]
fn valid_refs_parse_to_their_components_and_round_trip() {
    for c in fixture().valid {
        let r = CanonicalRef::parse(&c.text).unwrap_or_else(|e| panic!("{}: {e}", c.text));
        assert_eq!(r.kind, c.kind, "kind of {}", c.text);
        assert_eq!(r.id, c.id, "id of {}", c.text);
        assert_eq!(r.rev, c.rev, "rev of {}", c.text);
        assert_eq!(r.frag, c.frag, "frag of {}", c.text);
        assert_eq!(r.to_string(), c.text, "round trip of {}", c.text);
    }
}

#[test]
fn invalid_refs_are_rejected_for_the_stated_reason() {
    for c in fixture().invalid {
        let err = CanonicalRef::parse(&c.text)
            .err()
            .unwrap_or_else(|| panic!("{:?} should not parse", c.text));
        assert_eq!(err.reason(), c.reason, "{:?}: {err}", c.text);
    }
}

#[test]
fn formatting_escapes_only_the_reserved_characters() {
    for c in fixture().format {
        let r = CanonicalRef::new(&c.kind, &c.id, c.rev.as_deref(), c.frag.as_deref())
            .unwrap_or_else(|e| panic!("{}: {e}", c.text));
        assert_eq!(r.to_string(), c.text);
        assert_eq!(CanonicalRef::parse(&c.text).unwrap(), r);
    }
}

#[test]
fn a_bad_kind_error_names_the_kind() {
    let err = CanonicalRef::parse("Git-Commit:abc").unwrap_err();
    assert!(
        matches!(err, RefParseError::BadKind(ref k) if k == "Git-Commit"),
        "{err:?}"
    );
    assert!(err.to_string().contains("Git-Commit"), "{err}");
}
