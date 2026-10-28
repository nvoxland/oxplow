//! Scalar functions every connection registers, for the SQL that datasets,
//! pages and migrations ship (`.context/semantic-layer.md`).
//!
//! - `uri(resource, key, value, …)` is the one way SQL spells an oxplow
//!   URI, canonical and escaped (`.context/resources.md`), and
//!   `uri_view(uri, key, value, …)` the same URI in a view (its `#`); `uri_param(uri,
//!   key)` reads one param back, `uri_resource(uri)` its resource and
//!   `uri_key(uri)` the param that names it (`thr3`; anything that isn't a
//!   URI as it was). A NULL value leaves its param out.
//! - `ref_to_uri(text[, extension])`, `ref_kind_of(text)` and
//!   `ref_to_uri_in_json(json, field, …)` are the migration's: an old ref
//!   as its URI (`uri_migration`), anything else as it was.
//! - `uri_keys_in_json(json, field, …)` is too: the named fields' URIs as
//!   their keys (bare ids).

use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Error};

/// Register the functions on a connection.
pub(crate) fn register(c: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    c.create_scalar_function("uri", -1, flags, uri)?;
    c.create_scalar_function("uri_param", 2, flags, uri_param)?;
    c.create_scalar_function("uri_resource", 1, flags, uri_resource)?;
    c.create_scalar_function("uri_key", 1, flags, uri_key)?;
    c.create_scalar_function("uri_view", -1, flags, uri_view)?;
    c.create_scalar_function("ref_to_uri", -1, flags, ref_to_uri)?;
    c.create_scalar_function("ref_kind_of", 1, flags, ref_kind_of)?;
    c.create_scalar_function("ref_to_uri_in_json", -1, flags, ref_to_uri_in_json)?;
    c.create_scalar_function("uri_keys_in_json", -1, flags, uri_keys_in_json)
}

fn fail(message: &str) -> Error {
    Error::UserFunctionError(message.to_string().into())
}

/// `uri(resource, key, value, …)`: the one way SQL spells an oxplow URI
/// (`.context/resources.md`) — canonical and escaped. A NULL value leaves
/// its param out; a NULL resource gives NULL.
fn uri(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    use oxplow_domain::uri::oxplow_uri::{OxplowUri, ResourceName};
    let n = ctx.len();
    if n == 0 || n.is_multiple_of(2) {
        return Err(fail(
            "uri(resource, key, value, …) takes a resource then key/value pairs",
        ));
    }
    let Some(resource) = text(ctx, 0)? else {
        return Ok(None);
    };
    let resource: ResourceName = resource
        .parse()
        .map_err(|e: oxplow_domain::uri::oxplow_uri::UriParseError| fail(&e.to_string()))?;
    let params = pairs(ctx, "uri")?;
    Ok(Some(
        OxplowUri::new(resource, params, Default::default()).to_string(),
    ))
}

/// The key/value pairs after a function's first argument; a NULL value
/// leaves its key out.
fn pairs(
    ctx: &Context<'_>,
    name: &str,
) -> rusqlite::Result<std::collections::BTreeMap<String, String>> {
    let mut out = std::collections::BTreeMap::new();
    for i in (1..ctx.len()).step_by(2) {
        let key = text(ctx, i)?.ok_or_else(|| fail(&format!("{name}() keys can't be NULL")))?;
        let mut k = key.chars();
        let is_key = k.next().is_some_and(|c| c.is_ascii_alphabetic())
            && k.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !is_key {
            return Err(fail(&format!("{name}(): `{key}` isn't a param key")));
        }
        if let Some(value) = text(ctx, i + 1)? {
            out.insert(key, value);
        }
    }
    Ok(out)
}

/// `uri_view(uri, key, value, …)`: `uri` showing that view (its `#`: a
/// filter, a pick, a line), its identity unchanged. A NULL value leaves its
/// key out; a NULL `uri` gives NULL.
fn uri_view(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    use oxplow_domain::uri::oxplow_uri::OxplowUri;
    if ctx.is_empty() || ctx.len().is_multiple_of(2) {
        return Err(fail(
            "uri_view(uri, key, value, …) takes a URI then key/value pairs",
        ));
    }
    let Some(t) = text(ctx, 0)? else {
        return Ok(None);
    };
    let u = OxplowUri::parse(&t).map_err(|e| fail(&format!("uri_view(): {e}")))?;
    Ok(Some(u.with_view(pairs(ctx, "uri_view")?).to_string()))
}

/// `uri_param(uri, key)`: the (decoded) value of one of a URI's params;
/// NULL when it hasn't that param or isn't an oxplow URI.
fn uri_param(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    let (Some(text_), Some(key)) = (text(ctx, 0)?, text(ctx, 1)?) else {
        return Ok(None);
    };
    Ok(oxplow_domain::uri::oxplow_uri::OxplowUri::parse(&text_)
        .ok()
        .and_then(|u| u.param(&key).map(str::to_string)))
}

/// `uri_resource(uri)`: the resource a URI names (`oxplow/work_item`);
/// NULL when it isn't an oxplow URI.
fn uri_resource(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    Ok(text(ctx, 0)?.and_then(|t| oxplow_domain::uri::build::resource_of(&t)))
}

/// `uri_key(uri)`: the value of the param that names a URI's resource (its
/// bare id: `thr3`, `3`, `triage/todos`); anything that isn't an oxplow URI
/// as it was.
fn uri_key(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    let Some(t) = text(ctx, 0)? else {
        return Ok(None);
    };
    let key = oxplow_domain::uri::oxplow_uri::OxplowUri::parse(&t)
        .ok()
        .and_then(|u| {
            let key = oxplow_domain::uri::build::key_param(&u.resource().to_string())?;
            u.param(&key).map(str::to_string)
        });
    Ok(Some(key.unwrap_or(t)))
}

/// `ref_to_uri(text[, extension])`: an old ref as its URI
/// (`uri_migration`), anything else as it was. `extension` declared the
/// ref's kind, when an extension did.
fn ref_to_uri(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    if ctx.is_empty() || ctx.len() > 2 {
        return Err(fail("ref_to_uri(text[, extension])"));
    }
    let Some(t) = text(ctx, 0)? else {
        return Ok(None);
    };
    let extension = if ctx.len() == 2 { text(ctx, 1)? } else { None };
    Ok(Some(
        crate::uri_migration::v1_to_uri(&t, extension.as_deref()).unwrap_or(t),
    ))
}

/// `ref_kind_of(text)`: an old ref's kind, or NULL.
fn ref_kind_of(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    Ok(text(ctx, 0)?.and_then(|t| crate::uri_migration::v1_kind_of(&t)))
}

/// `ref_to_uri_in_json(json, field, …)`: `json` with each named top-level
/// field's old refs rewritten (a string, or each string of an array); with
/// no field named and an array given, each of its strings. Text that isn't
/// JSON is left as it was.
fn ref_to_uri_in_json(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    use serde_json::Value;
    let Some(t) = text(ctx, 0)? else {
        return Ok(None);
    };
    let Ok(mut v) = serde_json::from_str::<Value>(&t) else {
        return Ok(Some(t));
    };
    let rewrite = |v: &mut Value| match v {
        Value::String(s) => {
            if let Some(u) = crate::uri_migration::v1_to_uri(s, None) {
                *s = u;
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Value::String(s) = item {
                    if let Some(u) = crate::uri_migration::v1_to_uri(s, None) {
                        *s = u;
                    }
                }
            }
        }
        _ => {}
    };
    if ctx.len() == 1 {
        if v.is_array() {
            rewrite(&mut v);
        }
    } else if let Value::Object(map) = &mut v {
        for i in 1..ctx.len() {
            let field = text(ctx, i)?
                .ok_or_else(|| fail("ref_to_uri_in_json() field names can't be NULL"))?;
            if let Some(value) = map.get_mut(&field) {
                rewrite(value);
            }
        }
    }
    Ok(Some(v.to_string()))
}

/// `uri_keys_in_json(json, field, …)`: each named top-level field (or
/// each string of it, for an array) as its URI's key (`uri_key`), a
/// number when the field is written `<name>:int`. Absent fields stay
/// absent; anything that isn't JSON as it was.
fn uri_keys_in_json(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    use serde_json::Value;
    let Some(t) = text(ctx, 0)? else {
        return Ok(None);
    };
    let Ok(mut v) = serde_json::from_str::<Value>(&t) else {
        return Ok(Some(t));
    };
    let key_of = |v: &Value, int: bool| -> Value {
        let Value::String(s) = v else {
            return v.clone();
        };
        let key = oxplow_domain::uri::oxplow_uri::OxplowUri::parse(s)
            .ok()
            .and_then(|u| {
                let key = oxplow_domain::uri::build::key_param(&u.resource().to_string())?;
                u.param(&key).map(str::to_string)
            })
            .unwrap_or_else(|| s.clone());
        match key.parse::<i64>() {
            Ok(n) if int => Value::from(n),
            _ => Value::String(key),
        }
    };
    if let Value::Object(map) = &mut v {
        for i in 1..ctx.len() {
            let spec = text(ctx, i)?
                .ok_or_else(|| fail("uri_keys_in_json() field names can't be NULL"))?;
            let (field, int) = match spec.strip_suffix(":int") {
                Some(f) => (f.to_string(), true),
                None => (spec, false),
            };
            if let Some(value) = map.get_mut(&field) {
                *value = match &*value {
                    Value::Array(items) => {
                        Value::Array(items.iter().map(|x| key_of(x, int)).collect())
                    }
                    other => key_of(other, int),
                };
            }
        }
    }
    Ok(Some(v.to_string()))
}

fn text(ctx: &Context<'_>, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(match ctx.get_raw(i) {
        ValueRef::Null => None,
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Blob(_) => {
            return Err(Error::UserFunctionError(
                "a URI function takes text or number arguments".into(),
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sql: &str) -> rusqlite::Result<Option<String>> {
        let c = Connection::open_in_memory().unwrap();
        register(&c).unwrap();
        c.query_row(sql, [], |r| r.get(0))
    }

    #[test]
    fn uri_builds_a_canonical_uri() {
        assert_eq!(
            run("SELECT uri('oxplow/effort', 'id', 'eff' || 12)")
                .unwrap()
                .as_deref(),
            Some("oxplow:oxplow/effort?id=eff12")
        );
        assert_eq!(
            run("SELECT uri('oxplow/work_item', 'provider', 'oxplow', 'id', 'tsk1', 'x', NULL)")
                .unwrap()
                .as_deref(),
            Some("oxplow:oxplow/work_item?id=tsk1&provider=oxplow")
        );
        assert_eq!(
            run("SELECT uri('oxplow/file', 'path', 'a b&c.rs')")
                .unwrap()
                .as_deref(),
            Some("oxplow:oxplow/file?path=a%20b%26c.rs")
        );
        assert_eq!(run("SELECT uri(NULL, 'id', 'x')").unwrap(), None);
        assert!(run("SELECT uri('oxplow/effort', 'id')").is_err());
        assert!(run("SELECT uri('Effort', 'id', 'x')").is_err());
        assert!(run("SELECT uri('oxplow/effort', 'bad key', 'x')").is_err());
    }

    #[test]
    fn uri_view_sets_a_uris_view() {
        assert_eq!(
            run("SELECT uri_view(uri('oxplow/page', 'page', 'x/y', 'session', 'ses2'), 'turn', 'trn7', 'gone', NULL)")
                .unwrap()
                .as_deref(),
            Some("oxplow:oxplow/page?page=x/y&session=ses2#turn=trn7")
        );
        assert_eq!(run("SELECT uri_view(NULL, 'turn', 'trn7')").unwrap(), None);
        assert!(run("SELECT uri_view('not a uri', 'turn', 'trn7')").is_err());
        assert!(run("SELECT uri_view('oxplow:oxplow/page?page=x/y', 'turn')").is_err());
    }

    #[test]
    fn uri_param_reads_one_param() {
        assert_eq!(
            run("SELECT uri_param('oxplow:oxplow/work_item?id=tsk1&provider=oxplow', 'id')")
                .unwrap()
                .as_deref(),
            Some("tsk1")
        );
        assert_eq!(
            run("SELECT uri_param('oxplow:oxplow/effort?id=eff1', 'gone')").unwrap(),
            None
        );
        assert_eq!(run("SELECT uri_param('effort:eff1', 'id')").unwrap(), None);
        assert_eq!(run("SELECT uri_param(NULL, 'id')").unwrap(), None);
    }

    #[test]
    fn uri_resource_reads_the_resource() {
        assert_eq!(
            run("SELECT uri_resource('oxplow:oxplow/work_item?id=tsk1&provider=oxplow')")
                .unwrap()
                .as_deref(),
            Some("oxplow/work_item")
        );
        assert_eq!(run("SELECT uri_resource('effort:eff1')").unwrap(), None);
        assert_eq!(run("SELECT uri_resource(NULL)").unwrap(), None);
    }

    #[test]
    fn uri_key_reads_the_key_and_keeps_anything_else() {
        for (sql, want) in [
            (
                "SELECT uri_key('oxplow:oxplow/thread?id=thr3')",
                Some("thr3"),
            ),
            ("SELECT uri_key('oxplow:oxplow/snapshot?id=3')", Some("3")),
            (
                "SELECT uri_key('oxplow:oxplow/commit?sha=abc1234')",
                Some("abc1234"),
            ),
            (
                "SELECT uri_key('oxplow:oxplow/collector?collector=triage/todos')",
                Some("triage/todos"),
            ),
            ("SELECT uri_key('thr3')", Some("thr3")),
            // A file has no one key: it stays a URI.
            (
                "SELECT uri_key('oxplow:oxplow/file?path=a.rs')",
                Some("oxplow:oxplow/file?path=a.rs"),
            ),
        ] {
            assert_eq!(run(sql).unwrap().as_deref(), want, "{sql}");
        }
        assert_eq!(run("SELECT uri_key(NULL)").unwrap(), None);
    }

    // The migration's: an old ref as a URI, anything else as it was.
    #[test]
    fn ref_to_uri_rewrites_an_old_ref_and_keeps_anything_else() {
        assert_eq!(
            run("SELECT ref_to_uri('work_item:oxplow:tsk42')")
                .unwrap()
                .as_deref(),
            Some("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")
        );
        assert_eq!(
            run("SELECT ref_to_uri('acme_pr:12', 'acme')")
                .unwrap()
                .as_deref(),
            Some("oxplow:acme/pr?id=12")
        );
        assert_eq!(
            run("SELECT ref_to_uri('oxplow:tsk42')").unwrap().as_deref(),
            Some("oxplow:tsk42")
        );
        assert_eq!(
            run("SELECT ref_to_uri('app')").unwrap().as_deref(),
            Some("app")
        );
        assert_eq!(run("SELECT ref_to_uri(NULL)").unwrap(), None);
        assert_eq!(
            run("SELECT ref_kind_of('acme_pr:12')").unwrap().as_deref(),
            Some("acme_pr")
        );
        assert_eq!(run("SELECT ref_kind_of('nope')").unwrap(), None);
    }

    // Only the fields named are rewritten, in place: the rest of the
    // payload keeps its order.
    #[test]
    fn ref_to_uri_in_json_rewrites_named_fields_and_arrays() {
        assert_eq!(
            run(r#"SELECT ref_to_uri_in_json('{"ref":"effort:eff1","n":2,"refs":["thread:thr1","x"],"other":"effort:eff2"}', 'ref', 'refs')"#)
                .unwrap()
                .as_deref(),
            Some(r#"{"ref":"oxplow:oxplow/effort?id=eff1","n":2,"refs":["oxplow:oxplow/thread?id=thr1","x"],"other":"effort:eff2"}"#)
        );
        // An array is rewritten element by element (an event's subjects).
        assert_eq!(
            run(r#"SELECT ref_to_uri_in_json('["thread:thr1","stream:str2"]')"#)
                .unwrap()
                .as_deref(),
            Some(r#"["oxplow:oxplow/thread?id=thr1","oxplow:oxplow/stream?id=str2"]"#)
        );
        assert_eq!(
            run("SELECT ref_to_uri_in_json('not json', 'ref')")
                .unwrap()
                .as_deref(),
            Some("not json")
        );
        assert_eq!(run("SELECT ref_to_uri_in_json(NULL, 'ref')").unwrap(), None);
    }
    #[test]
    fn uri_keys_in_json_rewrites_named_fields_to_their_keys() {
        let json = |sql: &str| -> serde_json::Value {
            serde_json::from_str(&run(sql).unwrap().unwrap()).unwrap()
        };
        assert_eq!(
            json(
                r#"SELECT uri_keys_in_json('{"thread":"oxplow:oxplow/thread?id=thr1","snapshot":"oxplow:oxplow/snapshot?id=3","supersedes":["oxplow:oxplow/proposal?id=9"],"work_item":"oxplow:oxplow/work_item?id=tsk1&provider=oxplow","other":"oxplow:oxplow/effort?id=eff2"}', 'thread', 'snapshot:int', 'supersedes:int', 'work_item', 'absent')"#
            ),
            serde_json::json!({
                "thread": "thr1",
                "snapshot": 3,
                "supersedes": [9],
                "work_item": "oxplow:oxplow/work_item?id=tsk1&provider=oxplow",
                "other": "oxplow:oxplow/effort?id=eff2",
            })
        );
        // A value already bare stays; an `:int` one becomes a number.
        assert_eq!(
            json(
                r#"SELECT uri_keys_in_json('{"thread":"thr1","claim":"7"}', 'thread', 'claim:int')"#
            ),
            serde_json::json!({ "thread": "thr1", "claim": 7 })
        );
        assert_eq!(run("SELECT uri_keys_in_json(NULL, 'ref')").unwrap(), None);
    }
}
