//! `oxplow_read`: one reader for anything a URI names (`.context/resources.md`,
//! `.context/agent-model.md` "The agent's tools"). The agent's single way to look at
//! a thing it was pointed at or found in a model's `ref` column:
//!
//! - the **row** of the model its resource resolves through (its
//!   declaration's `resolve:`), every column, by its `ref`;
//! - what sits **beside** it, for the resources that have some: a work
//!   item's links and comments, a commit's files, an effort's files;
//! - its **content**, for the resources whose body no model holds: a file
//!   at a revision (`oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-40`;
//!   the working tree when it names none, in the stream asked for else the
//!   caller's), an event's stored body (`#part=input`, `output`, `prompt`,
//!   `result` or `summary`; the caller's own stream's only), a model call's
//!   (`#part=request` or `response`; not one another agent's thread asked
//!   for), a page's definition.
//!
//! Content is cut at [`MAX_CONTENT`] bytes, saying so.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::uri::resources as r;
use oxplow_domain::uri::OxplowUri;
use oxplow_domain::{StreamId, ThreadId};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::Services;

/// The most content one read returns, in bytes; the rest is cut, and
/// `truncated` says so. Ask for lines (`#lines=…`) to read further.
pub const MAX_CONTENT: usize = 64 * 1024;

/// Rows of one related list a read returns at most.
const MAX_RELATED: usize = 200;

/// What a URI names, as `oxplow_read` answers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RefRead {
    /// The URI as read, canonical.
    pub uri: String,
    /// Its resource (`oxplow/effort`).
    pub resource: String,
    /// The resolving model's row, every column; `None` for a resource no
    /// model holds (a file).
    pub row: Option<Map<String, Value>>,
    /// What sits beside it, by name (`links`, `comments`, `files`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub related: BTreeMap<&'static str, Vec<Map<String, Value>>>,
    /// Its body, for a resource whose body no model holds.
    pub content: Option<String>,
    /// The content was cut at [`MAX_CONTENT`].
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The URI doesn't parse, or asks for what its resource can't give.
    #[error("{0}")]
    Invalid(String),
    /// Nothing is there.
    #[error("nothing at `{0}`")]
    NotFound(String),
    #[error(transparent)]
    Storage(#[from] oxplow_domain::DomainError),
}

/// What sits beside a resource's row: `(resource, name, query)`, each
/// query's `?1` the URI (its view left out).
const RELATED: &[(&str, &str, &str)] = &[
    (
        r::WORK_ITEM,
        "links",
        "SELECT * FROM v_work_item_link WHERE from_uri = ?1 ORDER BY created_at",
    ),
    (
        r::WORK_ITEM,
        "comments",
        "SELECT * FROM v_work_item_comment WHERE uri = ?1 ORDER BY created_at",
    ),
    (
        r::COMMIT,
        "files",
        "SELECT * FROM v_commit_file WHERE sha = (SELECT sha FROM v_commit WHERE uri = ?1) ORDER BY path",
    ),
    (
        r::EFFORT,
        "files",
        "SELECT * FROM v_effort_file WHERE effort_id = (SELECT id FROM v_effort WHERE uri = ?1) ORDER BY path",
    ),
];

/// Who is reading: an agent's session's thread and stream, or a person
/// (both `None`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Reader {
    pub thread: Option<ThreadId>,
    pub stream: Option<StreamId>,
}

/// Read what `raw` names for `reader`. A file or page is read in `at`'s
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
    let parsed = OxplowUri::parse(raw.trim()).map_err(|e| ReadError::Invalid(e.to_string()))?;
    let reference = parsed.identity();
    let resource = parsed.resource().to_string();
    let part = parsed.view().get("part").map(String::as_str);
    let mut out = RefRead {
        uri: parsed.to_string(),
        resource: resource.clone(),
        row: None,
        related: BTreeMap::new(),
        content: None,
        truncated: false,
    };
    let param = |name: &str| parsed.param(name).unwrap_or_default().to_string();
    match resource.as_str() {
        r::FILE => {
            let text = file_text(svc, &parsed, stream).await?;
            let text = match parsed.view().get("lines") {
                Some(span) => lines(&text, span)?,
                None => text,
            };
            out.set_content(text);
        }
        r::PAGE => {
            // A page's definition: who declares it, where its source is,
            // which deployment runs it, and its file as written.
            let root = svc
                .worktrees
                .resolve(stream.map(|s| s.to_string()).as_deref())
                .await
                .into_local_path();
            let page = svc
                .extension_catalog
                .page_named(&root, &param("page"))
                .map_err(|_| ReadError::NotFound(out.uri.clone()))?;
            let yaml = std::fs::read_to_string(svc.layout.project_dir.join(&page.origin.running))
                .unwrap_or_default();
            let body = serde_json::json!({
                "page": page.id,
                "title": page.title,
                "description": page.description,
                "origin": page.origin.to_json(svc.worktrees.project_dir()),
                "params": page.params,
                "yaml": yaml,
            });
            out.set_content(
                serde_json::to_string_pretty(&body)
                    .map_err(|e| ReadError::Invalid(e.to_string()))?,
            );
        }
        _ => {
            if parsed.param("rev").is_some() {
                return Err(ReadError::Invalid(format!(
                    "a `{resource}` has no revisions; only a file is read at one"
                )));
            }
            out.row = Some(
                owned_row(svc, &parsed, &reference)
                    .await?
                    .ok_or_else(|| ReadError::NotFound(reference.clone()))?,
            );
            for (_, name, sql) in RELATED.iter().filter(|(k, _, _)| *k == resource) {
                out.related.insert(name, rows(svc, sql, &reference).await?);
            }
            match (resource.as_str(), part) {
                (r::EVENT, Some(part)) => {
                    out.set_content(event_body(svc, &param("id"), part, reader.stream).await?);
                }
                (r::SUBAGENT, Some(part)) => {
                    out.set_content(
                        subagent_result(svc, out.row.as_ref(), part, reader.stream).await?,
                    );
                }
                (r::AI_CALL, Some(part)) => {
                    out.set_content(ai_call_body(svc, &param("id"), part, reader.thread).await?);
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

/// A file's text at the URI's revision (the working tree when it names
/// none), in the caller's stream's worktree.
async fn file_text(
    svc: &Services,
    parsed: &OxplowUri,
    stream: Option<StreamId>,
) -> Result<String, ReadError> {
    let revision: oxplow_domain::vcs::Revision = parsed
        .param("rev")
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
        .read_at(&root, &revision, parsed.param("path").unwrap_or_default())
        .await
        .map_err(|e| match e {
            oxplow_domain::DomainError::Invalid(m) => ReadError::Invalid(m),
            other => ReadError::Storage(other),
        })?
        .ok_or_else(|| ReadError::NotFound(parsed.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The lines a `lines=<from>-<to>` (or `lines=<n>`) view names, 1-based
/// and inclusive.
fn lines(text: &str, span: &str) -> Result<String, ReadError> {
    let bad = || {
        ReadError::Invalid(format!(
            "a file's view names lines, `#lines=10-20` or `#lines=10`; not `#lines={span}`"
        ))
    };
    let (from, to) = span.split_once('-').unwrap_or((span, span));
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

/// The row of the model `uri`'s resource resolves through, whose `ref`
/// is `reference`.
async fn owned_row(
    svc: &Services,
    uri: &OxplowUri,
    reference: &str,
) -> Result<Option<Map<String, Value>>, ReadError> {
    let vocabulary = svc.vocabulary.current();
    let view = vocabulary
        .resources
        .get(uri.resource())
        .and_then(|decl| decl.resolve.clone())
        .ok_or_else(|| {
            ReadError::Invalid(format!(
                "no model holds `{}` URIs; `oxplow_query_sql` over `v_resource` lists the resources and what resolves each",
                uri.resource()
            ))
        })?;
    let sql = format!(
        "SELECT * FROM \"{}\" WHERE uri = ?1 LIMIT 1",
        view.replace('"', "")
    );
    Ok(rows(svc, &sql, reference).await?.into_iter().next())
}

/// `sql` (its `?1` the URI) as JSON objects, up to [`MAX_RELATED`].
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
        "result" => EventBodyKey::Result,
        "summary" => EventBodyKey::Summary,
        other => {
            return Err(ReadError::Invalid(format!(
                "an event's body is `#part=input`, `output`, `prompt`, `result` or `summary`; not `#part={other}`"
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
        let named =
            oxplow_domain::uri::build::of_with_view(r::EVENT, &[("id", id)], &[("part", part)]);
        ReadError::NotFound(format!("{named} (no body, or it expired)"))
    })?;
    serde_json::to_string_pretty(&body).map_err(|e| ReadError::Invalid(e.to_string()))
}

/// A subagent's last message (`#result`): the body of the event that
/// finished it, read like any event's.
async fn subagent_result(
    svc: &Services,
    row: Option<&Map<String, Value>>,
    part: &str,
    stream: Option<StreamId>,
) -> Result<String, ReadError> {
    if part != "result" {
        return Err(ReadError::Invalid(format!(
            "a subagent's body is `#part=result`; not `#part={part}`"
        )));
    }
    let finished = row
        .and_then(|r| r.get("finished_event_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ReadError::NotFound("a subagent that has not finished has no result".into())
        })?;
    event_body(svc, finished, "result", stream).await
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
                "a model call's body is `#part=request` or `#part=response`; not `#part={other}`"
            )))
        }
    };
    let id: i64 = id
        .parse()
        .map_err(|_| ReadError::Invalid(format!("`{id}` isn't a model call id")))?;
    let reader = thread.map(oxplow_domain::uri::build::thread_uri);
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
        let working = read(
            &svc,
            "oxplow:oxplow/file?path=src/a.txt",
            &Reader::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(working.content.as_deref(), Some("one\nTWO\nthree\n"));
        assert_eq!(working.row, None);
        let head = read(
            &svc,
            "oxplow:oxplow/file?path=src/a.txt&rev=git:HEAD",
            &Reader::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(head.content.as_deref(), Some("one\ntwo\nthree\n"));
        let two = read(
            &svc,
            "oxplow:oxplow/file?path=src/a.txt#lines=2-3",
            &Reader::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(two.content.as_deref(), Some("TWO\nthree"));
        assert!(matches!(
            read(&svc, "oxplow:oxplow/file?path=src/a.txt#lines=middle", &Reader::default(), None).await,
            Err(ReadError::Invalid(m)) if m.contains("#lines=10-20")
        ));
        assert!(matches!(
            read(
                &svc,
                "oxplow:oxplow/file?path=src/gone.txt",
                &Reader::default(),
                None
            )
            .await,
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
                    .result["uri"]
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
                json!({ "uri": item, "target": other, "link_type": "blocks" }),
            ),
            (
                "oxplow.work_item.comment",
                json!({ "uri": item, "body": "Seen on Safari only." }),
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
        assert_eq!(read.related["links"][0]["to_uri"], json!(other));
        assert_eq!(read.related["comments"][0]["body"], "Seen on Safari only.");
        assert_eq!(read.content, None);
    }

    /// Any kind a model owns reads as that model's row; one no model owns,
    /// or a ref that isn't there, says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_owned_kind_is_its_models_row() {
        let (_dir, svc) = project().await;
        let stream = read(
            &svc,
            "oxplow:oxplow/stream?id=str1",
            &Reader::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(stream.row.unwrap()["uri"], "oxplow:oxplow/stream?id=str1");
        assert!(matches!(
            read(
                &svc,
                "oxplow:oxplow/stream?id=str99",
                &Reader::default(),
                None
            )
            .await,
            Err(ReadError::NotFound(_))
        ));
        assert!(matches!(
            read(&svc, "oxplow:acme/nonsense?id=x", &Reader::default(), None).await,
            Err(ReadError::Invalid(m)) if m.contains("v_resource")
        ));
        assert!(matches!(
            read(&svc, "oxplow:oxplow/stream?id=str1&rev=git:HEAD", &Reader::default(), None).await,
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
        let collectors = call("oxplow:oxplow/collector?collector=acme/triage").await;
        let mine = call("oxplow:oxplow/thread?id=thr1").await;
        let theirs = call("oxplow:oxplow/thread?id=thr9").await;
        let me = Reader {
            thread: Some(oxplow_domain::ThreadId::new(1)),
            stream: None,
        };
        let row = read(
            &svc,
            &oxplow_domain::uri::build::keyed(
                oxplow_domain::uri::resources::AI_CALL,
                &collectors.to_string(),
            ),
            &me,
            None,
        )
        .await
        .unwrap();
        assert_eq!(row.row.unwrap()["role"], "decide");
        let ai_call = |id: i64, part: &str| {
            format!(
                "{}#part={part}",
                oxplow_domain::uri::build::keyed(
                    oxplow_domain::uri::resources::AI_CALL,
                    &id.to_string()
                )
            )
        };
        let asked = read(&svc, &ai_call(collectors, "request"), &me, None)
            .await
            .unwrap();
        assert!(asked.content.unwrap().contains("Is it risky?"));
        let own = read(&svc, &ai_call(mine, "response"), &me, None)
            .await
            .unwrap();
        assert!(own.content.unwrap().contains("no"));
        assert!(matches!(
            read(&svc, &ai_call(theirs, "response"), &me, None).await,
            Err(ReadError::Invalid(m)) if m.contains("another")
        ));
        assert!(matches!(
            read(&svc, &ai_call(mine, "middle"), &me, None).await,
            Err(ReadError::Invalid(m)) if m.contains("#part=request")
        ));
    }

    /// A file or page can be read in another stream's worktree by naming the
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
        let mine = read(&svc, "oxplow:oxplow/file?path=src/a.txt", &me, None)
            .await
            .unwrap();
        assert_eq!(mine.content.as_deref(), Some("one\nTWO\nthree\n"));
        let theirs = read(
            &svc,
            "oxplow:oxplow/file?path=src/a.txt",
            &me,
            Some(other_id),
        )
        .await
        .unwrap();
        assert_eq!(theirs.content.as_deref(), Some("theirs"));
    }

    /// Content past the cap is cut, and the read says so.
    #[test]
    fn long_content_is_cut_and_says_so() {
        let mut out = RefRead {
            uri: oxplow_domain::uri::build::file_uri("big"),
            resource: oxplow_domain::uri::resources::FILE.into(),
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
