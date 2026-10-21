//! Scalar functions every connection registers, for the SQL that models
//! and lenses ship (`.context/semantic-layer.md`).
//!
//! `lens_ref(id, key1, value1, …)` is the one way SQL spells a link to a
//! lens: it builds the `lens:` ref through `uri::build` and escapes the
//! params, so a lens, model or script never concatenates a `lens:` prefix
//! (`.context/resources.md` "Using refs"). A NULL value leaves its param out;
//! a NULL id gives NULL.

use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Error};

/// Register the functions on a connection.
pub(crate) fn register(c: &Connection) -> rusqlite::Result<()> {
    c.create_scalar_function(
        "lens_ref",
        -1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        lens_ref,
    )
}

fn text(ctx: &Context<'_>, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(match ctx.get_raw(i) {
        ValueRef::Null => None,
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Blob(_) => {
            return Err(Error::UserFunctionError(
                "lens_ref() takes text or number arguments".into(),
            ));
        }
    })
}

fn lens_ref(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    let n = ctx.len();
    if n == 0 || n.is_multiple_of(2) {
        return Err(Error::UserFunctionError(
            "lens_ref(id, key, value, …) takes a lens id then key/value pairs".into(),
        ));
    }
    let Some(id) = text(ctx, 0)? else {
        return Ok(None);
    };
    let mut params = Vec::new();
    for i in (1..n).step_by(2) {
        let key = text(ctx, i)?.ok_or_else(|| {
            Error::UserFunctionError("lens_ref() param names can't be NULL".into())
        })?;
        params.push((key, text(ctx, i + 1)?));
    }
    let borrowed: Vec<(&str, Option<&str>)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_deref()))
        .collect();
    oxplow_domain::uri::build::lens_uri_with(&id, &borrowed)
        .map(Some)
        .map_err(|e| Error::UserFunctionError(e.to_string().into()))
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
    fn lens_ref_builds_a_lens_ref_with_escaped_params() {
        assert_eq!(
            run("SELECT lens_ref('oxplow-bundled/job-run-output', 'job_run', 'job_run:ab&c')")
                .unwrap()
                .as_deref(),
            Some("lens:oxplow-bundled/job-run-output?job_run=job_run:ab%26c")
        );
        assert_eq!(
            run("SELECT lens_ref('a/b', 'thread_id', 7, 'x', NULL)")
                .unwrap()
                .as_deref(),
            Some("lens:a/b?thread_id=7")
        );
        assert_eq!(
            run("SELECT lens_ref('a/b')").unwrap().as_deref(),
            Some("lens:a/b")
        );
        assert_eq!(run("SELECT lens_ref(NULL, 'k', 'v')").unwrap(), None);
    }

    #[test]
    fn lens_ref_refuses_a_dangling_key_or_no_arguments() {
        assert!(run("SELECT lens_ref('a/b', 'k')").is_err());
        assert!(run("SELECT lens_ref()").is_err());
        assert!(run("SELECT lens_ref('a/b', NULL, 'v')").is_err());
        assert!(run("SELECT lens_ref('', 'k', 'v')").is_err());
    }
}
