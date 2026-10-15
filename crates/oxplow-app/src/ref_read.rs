//! `oxplow_read`: one reader for anything a ref names (`.context/refs.md`,
//! `.context/agent-model.md` "The agent's tools"). The agent's single way to look at
//! a thing it was pointed at or found in a model's `ref` column:
//!
//! - the **row** of the model that owns the ref's kind (core's,
//!   `oxplow_db::ref_models`, or an extension's `ref_kinds:` resolve view),
//!   every column, by its `ref`;
//! - what sits **beside** it, for the kinds that have some: a work item's
//!   links and comments, a commit's files, an effort's files;
//! - its **content**, for the kinds whose body no model holds: a file at a
//!   revision (`file:src/a.rs@git:HEAD#L10-40`; the working tree when it
//!   names none, in the stream asked for else the caller's), an event's
//!   stored body (`event:<id>#input`, `#output` or `#prompt`; the caller's
//!   own stream's only), a model call's (`ai_call:<id>#request` or
//!   `#response`; not one another agent's thread asked for), a lens's
//!   definition.
//!
//! Content is cut at [`MAX_CONTENT`] bytes, saying so.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::refs::grammar::CanonicalRef;
use oxplow_domain::refs::kind;
use oxplow_domain::{StreamId, ThreadId};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::Services;

/// The most content one read returns, in bytes; the rest is cut, and
/// `truncated` says so. Ask for lines (`#L…`) to read further.
pub const MAX_CONTENT: usize = 64 * 1024;

/// Rows of one related list a read returns at most.
const MAX_RELATED: usize = 200;

/// What a ref names, as `oxplow_read` answers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RefRead {
    /// The ref as read, canonical.
    #[serde(rename = "ref")]
    pub reference: String,
    pub kind: String,
    /// The owning model's row, every column; `None` for a kind no model
    /// owns (a file, a lens).
    pub row: Option<Map<String, Value>>,
    /// What sits beside it, by name (`links`, `comments`, `files`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub related: BTreeMap<&'static str, Vec<Map<String, Value>>>,
    /// Its body, for a kind whose body no model holds.
    pub content: Option<String>,
    /// The content was cut at [`MAX_CONTENT`].
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The ref doesn't parse, or asks for what its kind can't give.
    #[error("{0}")]
    Invalid(String),
    /// Nothing is there.
    #[error("nothing at `{0}`")]
    NotFound(String),
    #[error(transparent)]
    Storage(#[from] oxplow_domain::DomainError),
}

/// What sits beside a kind's row: `(kind, name, query)`, each query's `?1`
/// the canonical ref.
const RELATED: &[(&str, &str, &str)] = &[
    (
        kind::WORK_ITEM,
        "links",
        "SELECT * FROM v_work_item_link WHERE from_ref = ?1 ORDER BY created_at",
    ),
    (
        kind::WORK_ITEM,
        "comments",
        "SELECT * FROM v_work_item_comment WHERE ref = ?1 ORDER BY created_at",
    ),
    (
        kind::COMMIT,
        "files",
        "SELECT * FROM v_commit_file WHERE sha = (SELECT sha FROM v_commit WHERE ref = ?1) ORDER BY path",
    ),
    (
        kind::EFFORT,
        "files",
        "SELECT * FROM v_effort_file WHERE effort_id = (SELECT id FROM v_effort WHERE ref = ?1) ORDER BY path",
    ),
];

/// Who is reading: an agent's session's thread and stream, or a person
/// (both `None`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Reader {
    pub thread: Option<ThreadId>,
    pub stream: Option<StreamId>,
}

/// Read what `raw` names for `reader`. A file or lens is read in `at`'s
/// worktree, else the reader's stream's; an event's body only in the
/// reader's own stream; a model call's body unless another agent's thread
/// asked for it.
pub async fn read(
    svc: &Services,
    raw: &str,
    reader: &Reader,
    at: Option<StreamId>,
) -> Result<RefRead, ReadError> {
    let stream = at.or(reader.stream);
    let parsed = CanonicalRef::parse(raw.trim()).map_err(|e| ReadError::Invalid(e.to_string()))?;
    let reference = CanonicalRef {
        frag: None,
        ..parsed.clone()
    }
    .to_string();
    let mut out = RefRead {
        reference: parsed.to_string(),
        kind: parsed.kind.clone(),
        row: None,
        related: BTreeMap::new(),
        content: None,
        truncated: false,
    };
    match parsed.kind.as_str() {
        kind::FILE => {
            let text = file_text(svc, &parsed, stream).await?;
            let text = match parsed.frag.as_deref() {
                Some(frag) => lines(&text, frag)?,
                None => text,
            };
            out.set_content(text);
        }
        kind::LENS => {
            let root = svc
                .worktrees
                .resolve(stream.map(|s| s.to_string()).as_deref())
                .await
                .into_local_path();
            let lens = svc
                .extension_catalog
                .find_lens(&root, &parsed.id)
                .map_err(|_| ReadError::NotFound(out.reference.clone()))?;
            out.set_content(
                serde_json::to_string_pretty(&lens)
                    .map_err(|e| ReadError::Invalid(e.to_string()))?,
            );
        }
        _ => {
            if parsed.rev.is_some() {
                return Err(ReadError::Invalid(format!(
                    "a `{}` ref has no revisions; only a file is read at one",
                    parsed.kind
                )));
            }
            out.row = Some(
                owned_row(svc, &parsed.kind, &reference)
                    .await?
                    .ok_or_else(|| ReadError::NotFound(reference.clone()))?,
            );
            for (_, name, sql) in RELATED.iter().filter(|(k, _, _)| *k == parsed.kind) {
                out.related.insert(name, rows(svc, sql, &reference).await?);
            }
            match (parsed.kind.as_str(), parsed.frag.as_deref()) {
                (kind::EVENT, Some(frag)) => {
                    out.set_content(event_body(svc, &parsed.id, frag, reader.stream).await?);
                }
                (kind::AI_CALL, Some(frag)) => {
                    out.set_content(ai_call_body(svc, &parsed.id, frag, reader.thread).await?);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

impl RefRead {
    fn set_content(&mut self, mut text: String) {
        if text.len() > MAX_CONTENT {
            let mut cut = MAX_CONTENT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            self.truncated = true;
        }
        self.content = Some(text);
    }
}

/// A file's text at the ref's revision (the working tree when it names
/// none), in the caller's stream's worktree.
async fn file_text(
    svc: &Services,
    parsed: &CanonicalRef,
    stream: Option<StreamId>,
) -> Result<String, ReadError> {
    let revision: oxplow_domain::vcs::Revision = parsed
        .rev
        .as_deref()
        .unwrap_or("working")
        .parse()
        .map_err(ReadError::Invalid)?;
    let root = svc
        .worktrees
        .resolve(stream.map(|s| s.to_string()).as_deref())
        .await
        .into_local_path();
    let bytes = svc
        .trees
        .read_at(&root, &revision, &parsed.id)
        .await
        .map_err(|e| match e {
            oxplow_domain::DomainError::Invalid(m) => ReadError::Invalid(m),
            other => ReadError::Storage(other),
        })?
        .ok_or_else(|| ReadError::NotFound(parsed.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The lines a `L<from>-<to>` (or `L<n>`) fragment names, 1-based and
/// inclusive.
fn lines(text: &str, frag: &str) -> Result<String, ReadError> {
    let bad = || {
        ReadError::Invalid(format!(
            "a file's fragment names lines, `#L10-20` or `#L10`; not `#{frag}`"
        ))
    };
    let span = frag.strip_prefix('L').ok_or_else(bad)?;
    let (from, to) = match span.split_once('-') {
        Some((a, b)) => (a, b.strip_prefix('L').unwrap_or(b)),
        None => (span, span),
    };
    let from: usize = from.parse().map_err(|_| bad())?;
    let to: usize = to.parse().map_err(|_| bad())?;
    if from == 0 || to < from {
        return Err(bad());
    }
    Ok(text
        .lines()
        .skip(from - 1)
        .take(to - from + 1)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The row of the model owning `kind` whose `ref` is `reference`.
async fn owned_row(
    svc: &Services,
    kind: &str,
    reference: &str,
) -> Result<Option<Map<String, Value>>, ReadError> {
    let view = match oxplow_db::ref_models::owner_of(kind) {
        Some(owner) => owner.view.to_string(),
        None => {
            let resolve = svc
                .sql
                .query_sql(
                    "SELECT resolve FROM v_ref_kind WHERE kind = ?1 AND resolve IS NOT NULL",
                    vec![SqlCell::Text(kind.to_string())],
                    None,
                )
                .await?
                .rows
                .into_iter()
                .next()
                .and_then(|row| match row.into_iter().next() {
                    Some(SqlCell::Text(view)) => Some(view),
                    _ => None,
                });
            resolve.ok_or_else(|| {
                ReadError::Invalid(format!(
                    "no model holds `{kind}` refs; `oxplow_query_sql` over `v_ref_kind` lists the kinds"
                ))
            })?
        }
    };
    let sql = format!("SELECT * FROM \"{view}\" WHERE ref = ?1 LIMIT 1");
    Ok(rows(svc, &sql, reference).await?.into_iter().next())
}

/// `sql` (its `?1` the ref) as JSON objects, up to [`MAX_RELATED`].
async fn rows(
    svc: &Services,
    sql: &str,
    reference: &str,
) -> Result<Vec<Map<String, Value>>, ReadError> {
    let out = svc
        .sql
        .query_sql(
            sql,
            vec![SqlCell::Text(reference.to_string())],
            Some(MAX_RELATED),
        )
        .await?;
    Ok(out
        .rows
        .into_iter()
        .map(|row| {
            out.columns
                .iter()
                .cloned()
                .zip(row.into_iter().map(cell))
                .collect()
        })
        .collect())
}

fn cell(cell: SqlCell) -> Value {
    match cell {
        SqlCell::Null(()) => Value::Null,
        SqlCell::Bool(b) => Value::Bool(b),
        SqlCell::Int(i) => Value::from(i),
        SqlCell::Real(r) => Value::from(r),
        SqlCell::Text(t) => Value::String(t),
    }
}

/// An event's stored body (`input`, `output` or `prompt`), the caller's
/// stream's events only.
async fn event_body(
    svc: &Services,
    id: &str,
    part: &str,
    stream: Option<StreamId>,
) -> Result<String, ReadError> {
    use crate::event_bodies::{read, EventBodyError, EventBodyKey};
    let Some(stream) = stream else {
        return Err(ReadError::Invalid(
            "an event's body is read by an agent of the event's stream".into(),
        ));
    };
    let key = match part {
        "input" => EventBodyKey::Input,
        "output" => EventBodyKey::Output,
        "prompt" => EventBodyKey::Prompt,
        other => {
            return Err(ReadError::Invalid(format!(
                "an event's body is `#input`, `#output` or `#prompt`; not `#{other}`"
            )))
        }
    };
    let body = read(svc, id, key, Some(stream))
        .await
        .map_err(|e| match e {
            EventBodyError::Storage(e) => ReadError::Storage(e),
            other => ReadError::Invalid(other.to_string()),
        })?;
    let body = body.ok_or_else(|| {
        let named = CanonicalRef {
            kind: kind::EVENT.into(),
            id: id.into(),
            rev: None,
            frag: Some(part.into()),
        };
        ReadError::NotFound(format!("{named} (no body, or it expired)"))
    })?;
    serde_json::to_string_pretty(&body).map_err(|e| ReadError::Invalid(e.to_string()))
}

/// A model call's stored body (`request` or `response`). An agent reads
/// any call but one another agent's thread asked for.
async fn ai_call_body(
    svc: &Services,
    id: &str,
    part: &str,
    thread: Option<ThreadId>,
) -> Result<String, ReadError> {
    use crate::ai_calls::{read, AiCallBody, AiCallBodyError};
    let body = match part {
        "request" => AiCallBody::Request,
        "response" => AiCallBody::Response,
        other => {
            return Err(ReadError::Invalid(format!(
                "a model call's body is `#request` or `#response`; not `#{other}`"
            )))
        }
    };
    let id: i64 = id
        .parse()
        .map_err(|_| ReadError::Invalid(format!("`{id}` isn't a model call id")))?;
    let reader = thread.map(oxplow_domain::refs::build::thread_ref);
    let body = read(svc, id, body, reader.as_deref())
        .await
        .map_err(|e| match e {
            AiCallBodyError::Storage(e) => ReadError::Storage(e),
            other => ReadError::Invalid(other.to_string()),
        })?
        .ok_or_else(|| {
            ReadError::NotFound(format!("the call's {part} (none recorded, or it expired)"))
        })?;
    Ok(body.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    /// A project with `src/a.txt` committed as three lines, then edited in
    /// the working tree.
    async fn project() -> (tempfile::TempDir, std::sync::Arc<Services>) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.txt"), "one\ntwo\nthree\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("src/a.txt")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        std::fs::write(dir.path().join("src/a.txt"), "one\nTWO\nthree\n").unwrap();
        let services = Services::in_memory(dir.path()).await.unwrap();
        (dir, services)
    }

    /// A file is read in the working tree, at a revision, and by lines.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_reads_at_its_revision_and_lines() {
        let (_dir, svc) = project().await;
        let working = read(&svc, "file:src/a.txt", &Reader::default(), None)
            .await
            .unwrap();
        assert_eq!(working.content.as_deref(), Some("one\nTWO\nthree\n"));
        assert_eq!(working.row, None);
        let head = read(&svc, "file:src/a.txt@git:HEAD", &Reader::default(), None)
            .await
            .unwrap();
        assert_eq!(head.content.as_deref(), Some("one\ntwo\nthree\n"));
        let two = read(&svc, "file:src/a.txt#L2-3", &Reader::default(), None)
            .await
            .unwrap();
        assert_eq!(two.content.as_deref(), Some("TWO\nthree"));
        assert!(matches!(
            read(&svc, "file:src/a.txt#middle", &Reader::default(), None).await,
            Err(ReadError::Invalid(m)) if m.contains("#L10-20")
        ));
        assert!(matches!(
            read(&svc, "file:src/gone.txt", &Reader::default(), None).await,
            Err(ReadError::NotFound(_))
        ));
    }

    /// A work item is its row with its links and comments beside it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_work_item_is_its_row_links_and_comments() {
        let (_dir, svc) = project().await;
        let created = |title: &str| {
            let svc = svc.clone();
            let title = title.to_string();
            async move {
                svc.commands
                    .run(
                        &Actor::Human,
                        "oxplow.work_item.create",
                        json!({ "title": title }),
                        false,
                    )
                    .await
                    .unwrap()
                    .result["ref"]
                    .as_str()
                    .unwrap()
                    .to_string()
            }
        };
        let item = created("Fix the login loop").await;
        let other = created("Ship it").await;
        for (id, input) in [
            (
                "oxplow.work_item.link",
                json!({ "ref": item, "target": other, "link_type": "blocks" }),
            ),
            (
                "oxplow.work_item.comment",
                json!({ "ref": item, "body": "Seen on Safari only." }),
            ),
        ] {
            svc.commands
                .run(&Actor::Human, id, input, false)
                .await
                .unwrap();
        }
        let read = read(&svc, &item, &Reader::default(), None).await.unwrap();
        let row = read.row.unwrap();
        assert_eq!(row["title"], "Fix the login loop");
        assert_eq!(read.related["links"][0]["to_ref"], json!(other));
        assert_eq!(read.related["comments"][0]["body"], "Seen on Safari only.");
        assert_eq!(read.content, None);
    }

    /// Any kind a model owns reads as that model's row; one no model owns,
    /// or a ref that isn't there, says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_owned_kind_is_its_models_row() {
        let (_dir, svc) = project().await;
        let stream = read(&svc, "stream:str1", &Reader::default(), None)
            .await
            .unwrap();
        assert_eq!(stream.row.unwrap()["ref"], "stream:str1");
        assert!(matches!(
            read(&svc, "stream:str99", &Reader::default(), None).await,
            Err(ReadError::NotFound(_))
        ));
        assert!(matches!(
            read(&svc, "nonsense_kind:x", &Reader::default(), None).await,
            Err(ReadError::Invalid(m)) if m.contains("v_ref_kind")
        ));
        assert!(matches!(
            read(&svc, "stream:str1@git:HEAD", &Reader::default(), None).await,
            Err(ReadError::Invalid(m)) if m.contains("no revisions")
        ));
    }

    /// A recorded model call reads as its row, and `#request` / `#response`
    /// as its bodies: an agent reads a collector's or job's call, and its
    /// own thread's, never another thread's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_model_calls_bodies_read_unless_another_agent_asked() {
        let (_dir, svc) = project().await;
        let call = |caller: &'static str| {
            let svc = svc.clone();
            async move {
                svc.db
                    .transaction(move |tx| {
                        let request = oxplow_db::event_content_store::put_text_tx(tx, "ai", "Is it risky?")?;
                        let response = oxplow_db::event_content_store::put_text_tx(tx, "ai", "no")?;
                        tx.execute(
                            "INSERT INTO ai_call (role, provider, model, caller, ok, at, request_hash, response_hash)
                             VALUES ('decide', 'p', 'm', ?1, 1, '2026-01-01', ?2, ?3)",
                            rusqlite::params![caller, request.hash, response.hash],
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                        Ok(tx.last_insert_rowid())
                    })
                    .await
                    .unwrap()
            }
        };
        let collectors = call("collector:acme/triage").await;
        let mine = call("thread:thr1").await;
        let theirs = call("thread:thr9").await;
        let me = Reader {
            thread: Some(oxplow_domain::ThreadId::new(1)),
            stream: None,
        };
        let row = read(&svc, &format!("ai_call:{collectors}"), &me, None)
            .await
            .unwrap();
        assert_eq!(row.row.unwrap()["role"], "decide");
        let asked = read(&svc, &format!("ai_call:{collectors}#request"), &me, None)
            .await
            .unwrap();
        assert!(asked.content.unwrap().contains("Is it risky?"));
        let own = read(&svc, &format!("ai_call:{mine}#response"), &me, None)
            .await
            .unwrap();
        assert!(own.content.unwrap().contains("no"));
        assert!(matches!(
            read(&svc, &format!("ai_call:{theirs}#response"), &me, None).await,
            Err(ReadError::Invalid(m)) if m.contains("another")
        ));
        assert!(matches!(
            read(&svc, &format!("ai_call:{mine}#middle"), &me, None).await,
            Err(ReadError::Invalid(m)) if m.contains("#request")
        ));
    }

    /// A file or lens can be read in another stream's worktree by naming the
    /// stream; without one, the caller's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_reads_in_the_stream_named() {
        use oxplow_domain::stores::StreamStore as _;
        let (_dir, svc) = project().await;
        let primary = svc.stream_store.list().await.unwrap().pop().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(elsewhere.path().join("src")).unwrap();
        std::fs::write(elsewhere.path().join("src/a.txt"), "theirs").unwrap();
        let mut other = primary.clone();
        other.id = StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = svc.stream_store.upsert(&other).await.unwrap();
        let me = Reader {
            thread: None,
            stream: Some(primary.id),
        };
        let mine = read(&svc, "file:src/a.txt", &me, None).await.unwrap();
        assert_eq!(mine.content.as_deref(), Some("one\nTWO\nthree\n"));
        let theirs = read(&svc, "file:src/a.txt", &me, Some(other_id))
            .await
            .unwrap();
        assert_eq!(theirs.content.as_deref(), Some("theirs"));
    }

    /// Content past the cap is cut, and the read says so.
    #[test]
    fn long_content_is_cut_and_says_so() {
        let mut out = RefRead {
            reference: "file:big".into(),
            kind: "file".into(),
            row: None,
            related: BTreeMap::new(),
            content: None,
            truncated: false,
        };
        out.set_content("é".repeat(MAX_CONTENT));
        assert!(out.truncated);
        assert!(out.content.unwrap().len() <= MAX_CONTENT);
    }
}
