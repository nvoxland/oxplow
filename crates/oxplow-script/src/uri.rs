//! `uri`, the Starlark face of oxplow's URIs (`.context/resources.md`
//! "Using URIs"): a script builds a URI with `uri.build(resource,
//! **params)` and reads one with `uri.parse` / `uri.param` / `uri.id` /
//! `uri.resource`, never by string concatenation or slicing. The sandbox
//! has no resource registry, so a resource's params are checked where the
//! URI is used (a command's input, an event's subject); the grammar —
//! the names, escaping, canonical order — is checked here.

use oxplow_domain::uri::build;
use oxplow_domain::uri::OxplowUri;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::values::{Heap, Value};

/// Adds the `uri` namespace to a script's globals.
pub(crate) fn uri_builtins(globals: &mut GlobalsBuilder) {
    globals.namespace("uri", uri_members);
}

fn parse_uri(text: &str) -> anyhow::Result<OxplowUri> {
    OxplowUri::parse(text).map_err(|e| anyhow::anyhow!("`{text}`: {e}"))
}

/// A param's value as text: a string or an int; `None` leaves it out.
fn value_text(what: &str, key: &str, v: Value) -> anyhow::Result<Option<String>> {
    if v.is_none() {
        return Ok(None);
    }
    match (v.unpack_str(), v.unpack_i32()) {
        (Some(s), _) => Ok(Some(s.to_string())),
        (None, Some(n)) => Ok(Some(n.to_string())),
        _ => anyhow::bail!(
            "{what}: `{key}` is a string or an int, not {}",
            v.get_type()
        ),
    }
}

fn pairs(
    what: &str,
    map: &SmallMap<String, Value>,
) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in map.iter() {
        if let Some(text) = value_text(what, k, *v)? {
            out.insert(k.clone(), text);
        }
    }
    Ok(out)
}

#[starlark::starlark_module]
fn uri_members(builder: &mut GlobalsBuilder) {
    /// `oxplow:<resource>?<params>[#<view>]`, canonical: `resource` is
    /// `<extension>/<name>` (`oxplow/file`), each param a string or an
    /// int (`None` leaves it out), `view` a dict of the same
    /// (`uri.build("oxplow/file", path = f, view = {"lines": "3-9"})`).
    fn build<'v>(
        #[starlark(require = pos)] resource: &str,
        #[starlark(require = named)] view: Option<SmallMap<String, Value<'v>>>,
        #[starlark(kwargs)] params: SmallMap<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let what = format!("uri.build({resource})");
        let params = pairs(&what, &params)?;
        let view = match view {
            Some(v) => pairs(&what, &v)?,
            None => Default::default(),
        };
        let name: oxplow_domain::uri::ResourceName = resource
            .parse()
            .map_err(|e| anyhow::anyhow!("{what}: {e}"))?;
        Ok(OxplowUri::build(name, params, view)
            .map_err(|e| anyhow::anyhow!("{what}: {e}"))?
            .to_string())
    }

    /// `{ resource, params, view }` (values decoded). Fails on a string
    /// that isn't an oxplow URI.
    fn parse<'v>(
        #[starlark(require = pos)] text: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let u = crate::uri::parse_uri(text)?;
        Ok(heap.alloc(serde_json::json!({
            "resource": u.resource().to_string(), "params": u.params(), "view": u.view(),
        })))
    }

    /// The resource a URI names (`oxplow/effort`).
    fn resource(#[starlark(require = pos)] text: &str) -> anyhow::Result<String> {
        Ok(crate::uri::parse_uri(text)?.resource().to_string())
    }

    /// One param of a URI, `None` when it has none by that name; with
    /// `resource`, fails unless the URI is of that resource.
    fn param(
        #[starlark(require = pos)] text: &str,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] resource: Option<&str>,
    ) -> anyhow::Result<starlark::values::none::NoneOr<String>> {
        let u = crate::uri::parse_uri(text)?;
        if let Some(r) = resource {
            build::parse_resource(text, r).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Ok(starlark::values::none::NoneOr::from_option(
            u.param(name).map(str::to_string),
        ))
    }

    /// The key of a URI that names one thing by one param
    /// (`oxplow:oxplow/effort?id=eff12` → `eff12`); with `resource`,
    /// fails unless it is of that resource.
    fn id(
        #[starlark(require = pos)] text: &str,
        #[starlark(require = named)] resource: Option<&str>,
    ) -> anyhow::Result<String> {
        let resource = match resource {
            Some(r) => r.to_string(),
            None => crate::uri::parse_uri(text)?.resource().to_string(),
        };
        build::id_of(text, &resource).map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// A contribution's URI as `{ kind, extension, id }`: `extension` is
    /// `None` for a kind named globally (a command's bus id, a metric key).
    fn contribution<'v>(
        #[starlark(require = pos)] text: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let r: oxplow_domain::contribution::ContributionRef =
            text.parse().map_err(|e: String| anyhow::anyhow!(e))?;
        Ok(heap.alloc(serde_json::json!({
            "kind": r.kind().segment(), "extension": r.extension(), "id": r.name(),
        })))
    }

    /// Whether `value` is a string that parses as an oxplow URI (never
    /// fails).
    fn is_uri(#[starlark(require = pos)] value: Value) -> anyhow::Result<bool> {
        Ok(value
            .unpack_str()
            .is_some_and(|s| OxplowUri::parse(s).is_ok()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn run(body: &str) -> Result<serde_json::Value, String> {
        let script = format!("def transform(x):\n    return {body}\n");
        crate::runtime::run_starlark(&script, &json!({})).map_err(|e| e.to_string())
    }

    #[test]
    fn a_script_builds_a_uri_through_the_grammar() {
        assert_eq!(
            run(r#"uri.build("oxplow/file", path = "src/a&b.rs")"#).unwrap(),
            json!("oxplow:oxplow/file?path=src/a%26b.rs")
        );
        assert_eq!(
            run(r#"uri.build("oxplow/decision", id = 3)"#).unwrap(),
            json!("oxplow:oxplow/decision?id=3")
        );
        assert_eq!(
            run(r#"uri.build("oxplow/file", rev = "git:HEAD", path = "a.rs", view = {"lines": "1-2"})"#)
                .unwrap(),
            json!("oxplow:oxplow/file?path=a.rs&rev=git:HEAD#lines=1-2")
        );
        // A `None` param is left out.
        assert_eq!(
            run(r#"uri.build("oxplow/file", path = "a.rs", rev = None)"#).unwrap(),
            json!("oxplow:oxplow/file?path=a.rs")
        );
        let e = run(r#"uri.build("Bad", id = "x")"#).unwrap_err();
        assert!(e.contains("bad resource"), "{e}");
        let e = run(r#"uri.build("oxplow/file", path = [1])"#).unwrap_err();
        assert!(e.contains("a string or an int"), "{e}");
    }

    #[test]
    fn a_script_reads_a_uri_through_the_grammar() {
        assert_eq!(
            run(r#"uri.parse("oxplow:oxplow/file?path=a%26b.rs&rev=git:HEAD#lines=3")"#).unwrap(),
            json!({
                "resource": "oxplow/file",
                "params": { "path": "a&b.rs", "rev": "git:HEAD" },
                "view": { "lines": "3" },
            })
        );
        assert_eq!(
            run(r#"uri.resource("oxplow:oxplow/effort?id=eff12")"#).unwrap(),
            json!("oxplow/effort")
        );
        assert_eq!(
            run(r#"uri.id("oxplow:oxplow/effort?id=eff12")"#).unwrap(),
            json!("eff12")
        );
        assert_eq!(
            run(r#"uri.id("oxplow:oxplow/effort?id=eff12", resource = "oxplow/effort")"#).unwrap(),
            json!("eff12")
        );
        let e =
            run(r#"uri.id("oxplow:oxplow/claim?id=3", resource = "oxplow/decision")"#).unwrap_err();
        assert!(e.contains("oxplow/decision"), "{e}");
        assert_eq!(
            run(r#"[uri.param("oxplow:oxplow/work_item?id=tsk1&provider=oxplow", "provider"), uri.param("oxplow:oxplow/file?path=a", "rev")]"#)
                .unwrap(),
            json!(["oxplow", null])
        );
        let e = run(r#"uri.parse("nope")"#).unwrap_err();
        assert!(e.contains("isn't an oxplow URI"), "{e}");
        assert_eq!(
            run(r#"uri.contribution("oxplow:oxplow/collector?collector=work/hot")"#).unwrap(),
            json!({ "kind": "collector", "extension": "work", "id": "hot" })
        );
        assert_eq!(
            run(r#"uri.contribution("oxplow:oxplow/command?command=oxplow.review.accept")"#)
                .unwrap(),
            json!({ "kind": "command", "extension": null, "id": "oxplow.review.accept" })
        );
        assert!(run(r#"uri.contribution("oxplow:oxplow/effort?id=eff1")"#).is_err());
        // A guard for an optional value: `uri.is_uri` never fails.
        assert_eq!(
            run(
                r#"[uri.is_uri("oxplow:oxplow/wiki?slug=x"), uri.is_uri("nope"), uri.is_uri(None)]"#
            )
            .unwrap(),
            json!([true, false, false])
        );
    }
}
