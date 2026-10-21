//! `uri`, the Starlark face of oxplow's URI grammar (`.context/resources.md`
//! "Using refs"): a script builds a URI with `uri.build(type, id)` and
//! reads one with `uri.parse` / `uri.id` / `uri.type`, never by string
//! concatenation or slicing. The sandbox has no type registry, so a type
//! is checked where the URI is used (a command's input, an event's
//! subject); the grammar — escaping, the revision's shape — is checked
//! here.

use oxplow_domain::uri::grammar::Uri;
use starlark::environment::GlobalsBuilder;
use starlark::values::{Heap, Value};

/// Adds the `uri` namespace to a script's globals.
pub(crate) fn uri_builtins(globals: &mut GlobalsBuilder) {
    globals.namespace("uri", uri_members);
}

fn parse_uri(text: &str) -> anyhow::Result<Uri> {
    Uri::parse(text).map_err(|e| anyhow::anyhow!("`{text}`: {e}"))
}

#[starlark::starlark_module]
fn uri_members(builder: &mut GlobalsBuilder) {
    /// `<kind>:<id>[@rev][#frag]`, the id escaped. `id` may be a string
    /// or an int (`uri.build("decision", 3)`).
    fn build(
        #[starlark(require = pos)] kind: &str,
        #[starlark(require = pos)] id: Value,
        #[starlark(require = named)] rev: Option<&str>,
        #[starlark(require = named)] frag: Option<&str>,
    ) -> anyhow::Result<String> {
        let id = match (id.unpack_str(), id.unpack_i32()) {
            (Some(s), _) => s.to_string(),
            (None, Some(n)) => n.to_string(),
            _ => anyhow::bail!(
                "ref.build: `id` is a string or an int, not {}",
                id.get_type()
            ),
        };
        Ok(Uri::new(kind, &id, rev, frag)
            .map_err(|e| anyhow::anyhow!("uri.build({kind}, {id}): {e}"))?
            .to_string())
    }

    /// `{ type, id, rev, frag }` (id decoded; rev and frag `None` when
    /// absent). Fails on a string that isn't a ref.
    fn parse<'v>(
        #[starlark(require = pos)] text: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let r = crate::uri::parse_uri(text)?;
        Ok(heap.alloc(serde_json::json!({
            "type": r.kind, "id": r.id, "rev": r.rev, "frag": r.frag,
        })))
    }

    /// The type a URI names.
    fn r#type(#[starlark(require = pos)] text: &str) -> anyhow::Result<String> {
        Ok(crate::uri::parse_uri(text)?.kind)
    }

    /// The decoded id of a ref; with `kind`, fails unless the ref is of
    /// that kind (`uri.id(x["input"]["effort"], kind = "effort")`).
    fn id(
        #[starlark(require = pos)] text: &str,
        #[starlark(require = named)] kind: Option<&str>,
    ) -> anyhow::Result<String> {
        match kind {
            Some(k) => {
                oxplow_domain::uri::build::id_of(text, k).map_err(|e| anyhow::anyhow!("{e}"))
            }
            None => Ok(crate::uri::parse_uri(text)?.id),
        }
    }

    /// A contribution's ref as `{ kind, extension, id }`: `extension` is
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

    /// Whether `value` is a string that parses as a ref (never fails).
    fn is_uri(#[starlark(require = pos)] value: Value) -> anyhow::Result<bool> {
        Ok(value.unpack_str().is_some_and(|s| Uri::parse(s).is_ok()))
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
    fn a_script_builds_a_ref_through_the_grammar() {
        assert_eq!(
            run(r#"uri.build("file", "src/a@b.rs")"#).unwrap(),
            json!("file:src/a%40b.rs")
        );
        assert_eq!(
            run(r#"uri.build("decision", 3)"#).unwrap(),
            json!("decision:3")
        );
        assert_eq!(
            run(r#"uri.build("file", "a.rs", rev = "git:HEAD", frag = "L1-2")"#).unwrap(),
            json!("file:a.rs@git:HEAD#L1-2")
        );
        let e = run(r#"uri.build("Bad", "x")"#).unwrap_err();
        assert!(e.contains("bad kind"), "{e}");
        let e = run(r#"uri.build("file", "a", rev = "HEAD")"#).unwrap_err();
        assert!(e.contains("bad revision"), "{e}");
    }

    #[test]
    fn a_script_reads_a_ref_through_the_grammar() {
        assert_eq!(
            run(r#"uri.parse("file:a%40b.rs@git:HEAD#L3")"#).unwrap(),
            json!({ "type": "file", "id": "a@b.rs", "rev": "git:HEAD", "frag": "L3" })
        );
        assert_eq!(run(r#"uri.type("effort:eff12")"#).unwrap(), json!("effort"));
        assert_eq!(run(r#"uri.id("effort:eff12")"#).unwrap(), json!("eff12"));
        assert_eq!(
            run(r#"uri.id("effort:eff12", kind = "effort")"#).unwrap(),
            json!("eff12")
        );
        let e = run(r#"uri.id("claim:3", kind = "decision")"#).unwrap_err();
        assert!(e.contains("isn't a decision ref"), "{e}");
        let e = run(r#"uri.parse("nope")"#).unwrap_err();
        assert!(e.contains("has no `:`"), "{e}");
        assert_eq!(
            run(r#"uri.contribution("collector:work/hot")"#).unwrap(),
            json!({ "kind": "collector", "extension": "work", "id": "hot" })
        );
        assert_eq!(
            run(r#"uri.contribution("command:oxplow.review.accept")"#).unwrap(),
            json!({ "kind": "command", "extension": null, "id": "oxplow.review.accept" })
        );
        assert!(run(r#"uri.contribution("effort:eff1")"#).is_err());
        // A guard for an optional value: `ref.is_ref` never fails.
        assert_eq!(
            run(r#"[uri.is_uri("wiki:x"), uri.is_uri("nope"), uri.is_uri(None)]"#).unwrap(),
            json!([true, false, false])
        );
    }
}
