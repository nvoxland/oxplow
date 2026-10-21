//! oxplow's URI grammar against its golden fixture (`fixtures/uri.json`),
//! which the desktop's parser (`apps/desktop/src/uri/oxplowUri.test.ts`)
//! asserts too.

use std::collections::BTreeMap;

use oxplow_domain::uri::oxplow_uri::{AnyUri, OxplowUri, ResourceName};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/uri.json")).expect("the fixture is JSON")
}

fn map(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .expect("an object")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().expect("a string").to_string()))
        .collect()
}

#[test]
fn every_valid_case_parses_to_its_parts_and_its_canonical_form() {
    for case in fixture()["valid"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let uri = OxplowUri::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(
            uri.resource().extension(),
            case["resource"]["extension"],
            "{text}"
        );
        assert_eq!(uri.resource().name(), case["resource"]["name"], "{text}");
        assert_eq!(uri.params(), &map(&case["params"]), "{text}");
        assert_eq!(uri.view(), &map(&case["view"]), "{text}");
        assert_eq!(uri.to_string(), case["canonical"], "{text}");
        assert_eq!(uri.identity(), case["identity"], "{text}");
        // The canonical form is a fixed point.
        let again = OxplowUri::parse(&uri.to_string()).unwrap();
        assert_eq!(again, uri, "{text}");
    }
}

#[test]
fn every_invalid_case_is_refused_for_its_reason() {
    for case in fixture()["invalid"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let e = OxplowUri::parse(text).expect_err(text);
        assert_eq!(e.reason(), case["reason"], "{text}: {e}");
    }
}

#[test]
fn a_built_uri_is_canonical() {
    for case in fixture()["build"].as_array().unwrap() {
        let resource: ResourceName = case["resource"].as_str().unwrap().parse().unwrap();
        let uri = OxplowUri::new(resource, map(&case["params"]), map(&case["view"]));
        assert_eq!(uri.to_string(), case["canonical"]);
    }
}

// A link may name any resource: one of oxplow's, or anything else a URL
// names (the web, mail).
#[test]
fn any_uri_tells_oxplows_from_the_rest() {
    assert!(matches!(
        AnyUri::parse("oxplow:oxplow/effort?id=eff1"),
        Ok(AnyUri::Oxplow(_))
    ));
    assert!(matches!(
        AnyUri::parse("https://example.com/a?b=1"),
        Ok(AnyUri::External(_))
    ));
    assert!(matches!(
        AnyUri::parse("mailto:someone@example.com"),
        Ok(AnyUri::External(_))
    ));
    // An oxplow: string that isn't a valid one is refused, never external.
    assert!(AnyUri::parse("oxplow:effort").is_err());
    assert!(AnyUri::parse("not a uri").is_err());
}
