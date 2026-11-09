//! Scalar functions every connection registers, for the SQL that datasets,
//! pages and migrations ship (`.context/semantic-layer.md`).
//!
//! - `uri(resource, key, value, …)` is the one way SQL spells an oxplow
//!   URI, canonical and escaped (`.context/resources.md`), and
//!   `uri_view(uri, key, value, …)` the same URI in a view (its `#`); `uri_param(uri,
//!   key)` reads one param back, `uri_resource(uri)` its resource and
//!   `uri_key(uri)` the param that names it (`thr3`; anything that isn't a
//!   URI as it was). A NULL value leaves its param out.
//! - `first_sentence(markdown)` is a description's opening for a card or a
//!   list row: the first line that isn't a heading, its list, quote,
//!   emphasis (`*`, `__`) and code marks dropped (a lone `_` is an
//!   identifier's), up to the first `.`, `!` or `?` that ends a sentence;
//!   NULL when there's no such line.

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
    c.create_scalar_function("first_sentence", 1, flags, |ctx| {
        Ok(text(ctx, 0)?.and_then(|t| first_sentence(&t)))
    })?;
    c.create_scalar_function("read_measure", 1, flags, read_measure)
}

/// The datasets whose reads `read_measure()` scopes to a measure: the facts
/// and the captures they came from.
pub(crate) const MEASURE_SCOPED: &[&str] = &["v_fact", "v_capture"];

thread_local! {
    static MEASURE_READS: std::cell::RefCell<Option<std::collections::BTreeSet<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// The measures `read_measure()` named while one query ran on this thread.
pub(crate) struct MeasureReads;

impl MeasureReads {
    pub(crate) fn begin() -> Self {
        MEASURE_READS.with(|m| *m.borrow_mut() = Some(Default::default()));
        Self
    }

    /// Sorted, distinct; none after the first call.
    pub(crate) fn take(&self) -> Vec<String> {
        MEASURE_READS.with(|m| {
            m.borrow_mut()
                .as_mut()
                .map(|set| std::mem::take(set).into_iter().collect())
                .unwrap_or_default()
        })
    }
}

impl Drop for MeasureReads {
    fn drop(&mut self) {
        MEASURE_READS.with(|m| *m.borrow_mut() = None);
    }
}

/// The keys written as string literals in `read_measure('…')` calls of
/// `sql`.
pub(crate) fn literal_measure_reads(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sql;
    while let Some(at) = rest.find("read_measure(") {
        rest = &rest[at + "read_measure(".len()..];
        let arg = rest.trim_start();
        if let Some(quoted) = arg.strip_prefix('\'') {
            if let Some(end) = quoted.find('\'') {
                out.push(quoted[..end].to_string());
            }
        }
    }
    out
}

/// `read_measure(key)`: `key`, and a note that the query reads the facts of
/// that measure. A page that filters `v_fact` on `measure_key =
/// read_measure(…)` is re-run only when facts land for that measure, not
/// for every measure (`.context/performance.md`). Use it for every read of
/// `v_fact` and `v_capture` in the query.
fn read_measure(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    let key = text(ctx, 0)?;
    if let Some(key) = &key {
        MEASURE_READS.with(|m| {
            if let Some(set) = m.borrow_mut().as_mut() {
                set.insert(key.clone());
            }
        });
    }
    Ok(key)
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

/// The first sentence of a markdown description (the module's doc).
fn first_sentence(body: &str) -> Option<String> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let mut rest = line;
    // List, numbered-list and quote marks, however many lead the line.
    loop {
        let before = rest;
        for mark in ["- ", "* ", "+ "] {
            if let Some(r) = rest.strip_prefix(mark) {
                rest = r.trim_start();
            }
        }
        if let Some(r) = rest.strip_prefix('>') {
            rest = r.trim_start();
        }
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            let after = &rest[digits..];
            if let Some(r) = after
                .strip_prefix(". ")
                .or_else(|| after.strip_prefix(") "))
            {
                rest = r.trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    let text = rest.replace("__", "").replace(['*', '`'], "");
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    let end = chars.iter().enumerate().position(|(i, c)| {
        matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace())
    });
    let sentence: String = match end {
        Some(i) => chars[..=i].iter().collect(),
        None => text.to_string(),
    };
    (!sentence.is_empty()).then_some(sentence)
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

    /// A description's first sentence, for a card or a list row: past
    /// headings, list, quote, emphasis and code marks, to the first `.`,
    /// `!` or `?` that ends a sentence.
    #[test]
    fn first_sentence_reads_a_descriptions_opening() {
        let first = |body: &str| {
            let c = Connection::open_in_memory().unwrap();
            register(&c).unwrap();
            c.query_row("SELECT first_sentence(?1)", [body], |r| {
                r.get::<_, Option<String>>(0)
            })
            .unwrap()
        };
        for (body, says) in [
            (
                "Signing in from /settings bounces back. The `next` param is stale.",
                Some("Signing in from /settings bounces back."),
            ),
            (
                "## Context\n\n- **The panel** times out after 5s\nmore",
                Some("The panel times out after 5s"),
            ),
            (
                "> Quoted first: yes! Then more.",
                Some("Quoted first: yes!"),
            ),
            ("See `v_effort.title`. Done.", Some("See v_effort.title.")),
            (
                "Version 1.2 ships soon. Next.",
                Some("Version 1.2 ships soon."),
            ),
            ("1. Number one. Two.", Some("Number one.")),
            ("", None),
            ("# Only a heading", None),
        ] {
            assert_eq!(first(body).as_deref(), says, "{body:?}");
        }
        assert_eq!(run("SELECT first_sentence(NULL)").unwrap(), None);
    }
}
