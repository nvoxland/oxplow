//! `ref`, the Starlark face of oxplow's ref grammar (`.context/refs.md`
//! "Using refs"): a script builds a ref with `ref.build(kind, id)` and
//! reads one with `ref.parse` / `ref.id` / `ref.kind`, never by string
//! concatenation or slicing. The sandbox has no kind registry, so a kind
//! is checked where the ref is used (a command's input, an event's
//! subject); the grammar — escaping, the revision's shape — is checked
//! here.

use oxplow_domain::refs::grammar::CanonicalRef;
use starlark::environment::GlobalsBuilder;
use starlark::values::{Heap, Value};

/// Adds the `ref` namespace to a script's globals.
pub(crate) fn ref_builtins(globals: &mut GlobalsBuilder) {
    globals.namespace("ref", ref_members);
}

fn parse_ref(text: &str) -> anyhow::Result<CanonicalRef> {
    CanonicalRef::parse(text).map_err(|e| anyhow::anyhow!("`{text}`: {e}"))
}

#[starlark::starlark_module]
fn ref_members(builder: &mut GlobalsBuilder) {
    /// `<kind>:<id>[@rev][#frag]`, the id escaped. `id` may be a string
    /// or an int (`ref.build("decision", 3)`).
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
        Ok(CanonicalRef::new(kind, &id, rev, frag)
            .map_err(|e| anyhow::anyhow!("ref.build({kind}, {id}): {e}"))?
            .to_string())
    }

    /// `{ kind, id, rev, frag }` (id decoded; rev and frag `None` when
    /// absent). Fails on a string that isn't a ref.
    fn parse<'v>(
        #[starlark(require = pos)] text: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let r = crate::refs::parse_ref(text)?;
        Ok(heap.alloc(serde_json::json!({
            "kind": r.kind, "id": r.id, "rev": r.rev, "frag": r.frag,
        })))
    }

    /// The kind a ref names.
    fn kind(#[starlark(require = pos)] text: &str) -> anyhow::Result<String> {
        Ok(crate::refs::parse_ref(text)?.kind)
    }

    /// The decoded id of a ref; with `kind`, fails unless the ref is of
    /// that kind (`ref.id(x["input"]["effort"], kind = "effort")`).
    fn id(
        #[starlark(require = pos)] text: &str,
        #[starlark(require = named)] kind: Option<&str>,
    ) -> anyhow::Result<String> {
        match kind {
            Some(k) => {
                oxplow_domain::refs::build::id_of(text, k).map_err(|e| anyhow::anyhow!("{e}"))
            }
            None => Ok(crate::refs::parse_ref(text)?.id),
        }
    }

    /// Whether `value` is a string that parses as a ref (never fails).
    fn is_ref(#[starlark(require = pos)] value: Value) -> anyhow::Result<bool> {
        Ok(value
            .unpack_str()
            .is_some_and(|s| CanonicalRef::parse(s).is_ok()))
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
            run(r#"ref.build("file", "src/a@b.rs")"#).unwrap(),
            json!("file:src/a%40b.rs")
        );
        assert_eq!(
            run(r#"ref.build("decision", 3)"#).unwrap(),
            json!("decision:3")
        );
        assert_eq!(
            run(r#"ref.build("file", "a.rs", rev = "git:HEAD", frag = "L1-2")"#).unwrap(),
            json!("file:a.rs@git:HEAD#L1-2")
        );
        let e = run(r#"ref.build("Bad", "x")"#).unwrap_err();
        assert!(e.contains("bad kind"), "{e}");
        let e = run(r#"ref.build("file", "a", rev = "HEAD")"#).unwrap_err();
        assert!(e.contains("bad revision"), "{e}");
    }

    #[test]
    fn a_script_reads_a_ref_through_the_grammar() {
        assert_eq!(
            run(r#"ref.parse("file:a%40b.rs@git:HEAD#L3")"#).unwrap(),
            json!({ "kind": "file", "id": "a@b.rs", "rev": "git:HEAD", "frag": "L3" })
        );
        assert_eq!(run(r#"ref.kind("effort:eff12")"#).unwrap(), json!("effort"));
        assert_eq!(run(r#"ref.id("effort:eff12")"#).unwrap(), json!("eff12"));
        assert_eq!(
            run(r#"ref.id("effort:eff12", kind = "effort")"#).unwrap(),
            json!("eff12")
        );
        let e = run(r#"ref.id("claim:3", kind = "decision")"#).unwrap_err();
        assert!(e.contains("isn't a decision ref"), "{e}");
        let e = run(r#"ref.parse("nope")"#).unwrap_err();
        assert!(e.contains("has no `:`"), "{e}");
        // A guard for an optional value: `ref.is_ref` never fails.
        assert_eq!(
            run(r#"[ref.is_ref("wiki:x"), ref.is_ref("nope"), ref.is_ref(None)]"#).unwrap(),
            json!([true, false, false])
        );
    }
}
