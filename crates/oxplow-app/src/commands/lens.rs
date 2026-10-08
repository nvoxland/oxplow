//! Answers in a thread (P6.C1, target §11.4's lens lifecycle): an agent
//! shows the person something — an existing lens or its own lens spec —
//! with `oxplow.lens.show`; `oxplow.lens.keep` writes an answer — or a spec, as Explore
//! Data's Save as Lens does — as a private lens;
//! `oxplow.lens.share` moves a private lens into a shared extension once it
//! passes the shared checks (committing it is the person's git commit).
//! The rows live in `thread_answer` (`v_thread_answer`); `run_answer`
//! runs one for the Answers strip and for `show_lens`'s text.

use crate::commands::ops::Op;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::thread_answer_store::{self as answers, AnswerShows};
use oxplow_db::SqlCell;
use oxplow_domain::events::schema::{LensKept, LensKeptV2, LensShown, LensShownV1};
use oxplow_domain::events::Envelope;
use oxplow_domain::refs::build::{answer_ref, lens_ref, thread_ref};
use oxplow_domain::{CommandError, DomainError, ThreadId};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::{self, LensContext, LensOrigin, LensSpec};

pub const SHOW: &str = "oxplow.lens.show";
pub const KEEP: &str = "oxplow.lens.keep";
pub const SHARE: &str = "oxplow.lens.share";

/// Where the lens commands find lenses and write them, and the database
/// the `External` ones (`keep`, `share`) read and write in transactions
/// of their own.
#[derive(Clone)]
pub struct LensTarget {
    pub project_dir: PathBuf,
    pub catalog: Arc<ExtensionCatalog>,
    pub db: oxplow_db::Database,
    /// Checks a kept spec's query as the explorer runs it, metric
    /// functions and all (tsk987).
    pub sql: crate::sql_gateway::SqlGateway,
}

/// `oxplow.lens.show`: show the person an answer in a thread.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowInput {
    /// An existing lens to show (`<extension>/<slug>`). Give this or `spec`.
    pub lens: Option<String>,
    /// A lens of its own: title, query, viz and what its viz needs.
    pub spec: Option<LensSpec>,
    /// Param values by name (the lens's declared params).
    pub params: Option<BTreeMap<String, Value>>,
    /// The thread it's shown in (`thr3`); the caller's when omitted.
    pub thread: Option<String>,
}

/// `oxplow.lens.keep`: write an answer, or a spec, as a private lens.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepInput {
    /// The answer to keep (`answer:12`). Give this or `spec`.
    pub answer: Option<String>,
    /// A lens of its own to keep (title, query, viz and what its viz
    /// needs), checked as `oxplow.lens.show` checks one.
    pub spec: Option<LensSpec>,
    /// The stream whose worktree a `spec` goes in (`str2`): the caller's
    /// thread's when omitted, else the primary's. An answer goes in its
    /// own thread's.
    pub stream: Option<String>,
    /// The extension it goes in; `my-lenses` when omitted.
    pub extension: Option<String>,
    /// Its slug; from its title when omitted.
    pub slug: Option<String>,
}

/// `oxplow.lens.share`: move a private lens into a shared extension.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareInput {
    /// The lens (`<extension>/<slug>`), in a private extension.
    pub lens: String,
    /// The shared extension it moves to (created, shared, when missing).
    pub extension: String,
    /// The stream whose worktree it's in (`str2`); the primary's when
    /// omitted.
    pub stream: Option<String>,
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message: message.into(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn domain(e: DomainError) -> CommandError {
    match e {
        DomainError::Invalid(m) => CommandError::Invalid {
            field: None,
            message: m,
        },
        DomainError::NotFound => CommandError::Invalid {
            field: None,
            message: "not found".into(),
        },
        other => CommandError::from(other),
    }
}

/// The worktree of `thread`'s stream; `Invalid` for an unknown thread.
fn thread_root_tx(
    conn: &rusqlite::Connection,
    project_dir: &Path,
    thread: i64,
) -> Result<PathBuf, DomainError> {
    let path: Option<String> = conn
        .query_row(
            "SELECT s.worktree_path FROM threads t JOIN streams s ON s.id = t.stream_id
             WHERE t.id = ?1",
            [thread],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| DomainError::Storage(e.to_string()))?;
    let path = path.ok_or_else(|| DomainError::Invalid(format!("no thread `thr{thread}`")))?;
    Ok(crate::worktrees::workspace_path(project_dir, &path))
}

fn thread_stream_tx(conn: &rusqlite::Connection, thread: i64) -> Option<i64> {
    conn.query_row(
        "SELECT stream_id FROM threads WHERE id = ?1",
        [thread],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// The worktree of stream `raw` (`str2`); `Invalid` for one that isn't.
fn stream_root_tx(
    conn: &rusqlite::Connection,
    project_dir: &Path,
    raw: &str,
) -> Result<(i64, PathBuf), DomainError> {
    let stream: oxplow_domain::StreamId = raw
        .parse()
        .map_err(|_| DomainError::Invalid(format!("`{raw}` isn't a stream")))?;
    let path: Option<String> = conn
        .query_row(
            "SELECT worktree_path FROM streams WHERE id = ?1",
            [stream.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| DomainError::Storage(e.to_string()))?;
    let path = path.ok_or_else(|| DomainError::Invalid(format!("no stream `{raw}`")))?;
    Ok((
        stream.value(),
        crate::worktrees::workspace_path(project_dir, &path),
    ))
}

/// A lens spec's query, checked as `query_sql` checks it — one read-only
/// statement over the published models, metric functions resolved through
/// the metric engine (`SqlGateway::check`). `oxplow.lens.show` runs it before its
/// transaction (its [`Precheck`](crate::commands::Precheck), tsk1010),
/// `oxplow.lens.keep` in its run.
async fn check_spec_query(
    sql: &crate::sql_gateway::SqlGateway,
    spec: &LensSpec,
) -> Result<(), CommandError> {
    if spec.query.trim().is_empty() {
        return Ok(());
    }
    sql.check(&spec.query)
        .await
        .map(|_| ())
        .map_err(|e| invalid("/spec/query", e.to_string()))
}

/// A spec's shape and its params bound in `lens_ctx` — everything of its
/// check but the query: the bound params.
fn check_spec_shape(
    spec: &LensSpec,
    params: &BTreeMap<String, Value>,
    lens_ctx: &LensContext,
) -> Result<BTreeMap<String, SqlCell>, CommandError> {
    if let Some(problem) = extensions::spec_problem(spec) {
        return Err(invalid("/spec", problem));
    }
    let lens = extensions::Lens::from_spec("answer/new", spec);
    extensions::resolve_params(&lens, &cells(params), lens_ctx).map_err(domain)
}

fn cells(params: &BTreeMap<String, Value>) -> BTreeMap<String, SqlCell> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), SqlCell::from(v.clone())))
        .collect()
}

fn answer_id(raw: &str) -> Result<i64, CommandError> {
    raw.strip_prefix("answer:")
        .unwrap_or(raw)
        .parse()
        .map_err(|_| {
            invalid(
                "/answer",
                format!("`{raw}` isn't an answer (`answer:<id>`)"),
            )
        })
}

fn show(target: LensTarget) -> Op {
    // A spec's query is checked before the transaction: resolving a metric
    // query takes the metric engine, which a transaction can't (tsk1010).
    let sql = target.sql.clone();
    let precheck: Arc<crate::commands::Precheck> = Arc::new(move |input: Value| {
        let sql = sql.clone();
        Box::pin(async move {
            match parse::<ShowInput>(input)?.spec {
                Some(spec) => check_spec_query(&sql, &spec).await,
                None => Ok(()),
            }
        })
    });
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: ShowInput = parse(input)?;
        let thread: i64 = match input.thread.as_deref() {
            Some(raw) => {
                let named = raw
                    .parse::<ThreadId>()
                    .map_err(|e| invalid("/thread", e.to_string()))?;
                // An agent shows answers in its own thread only; a person
                // may name any.
                if ctx.actor.is_agent_driven() && ctx.actor.thread_id() != Some(named) {
                    return Err(invalid(
                        "/thread",
                        format!("an agent shows answers in its own thread, not `{raw}`"),
                    ));
                }
                named.value()
            }
            None => ctx
                .actor
                .thread_id()
                .ok_or_else(|| invalid("/thread", "no thread given and the caller has none"))?
                .value(),
        };
        let stream = thread_stream_tx(ctx.conn, thread)
            .ok_or_else(|| invalid("/thread", format!("no thread `thr{thread}`")))?;
        let params = input.params.unwrap_or_default();
        let lens_ctx = LensContext {
            stream_id: Some(stream),
            thread_id: Some(thread),
            active: None,
        };
        let (title, shows, lens) = match (input.lens, input.spec) {
            (Some(id), None) => {
                let lens = target
                    .catalog
                    .find_lens(&target.project_dir, &id)
                    .map_err(domain)?;
                extensions::resolve_params(&lens, &cells(&params), &lens_ctx).map_err(domain)?;
                (lens.title.clone(), AnswerShows::Lens(id.clone()), Some(id))
            }
            (None, Some(spec)) => {
                // Its query was checked before the transaction.
                check_spec_shape(&spec, &params, &lens_ctx)?;
                let value = serde_json::to_value(&spec).expect("spec serializes");
                (spec.title.clone(), AnswerShows::Spec(value), None)
            }
            _ => {
                return Err(invalid(
                    "",
                    "give `lens` (an existing lens) or `spec`, not both",
                ))
            }
        };
        let id = answers::insert_tx(ctx.conn, thread, &title, &shows, &json!(params))
            .map_err(CommandError::from)?;
        let event = ctx
            .events
            .typed::<LensShown>(&LensShownV1 {
                answer: answer_ref(id),
                thread: thread_ref(ThreadId::new(thread)),
                lens: lens.as_deref().map(lens_ref),
            })
            .with_subject([answer_ref(id)]);
        Ok(HandlerOutput {
            result: json!({ "answer": answer_ref(id), "title": title }),
            inverse: None,
            events: vec![event],
            after_commit: None,
            unchanged: false,
        })
    }));
    Op::new(
        "lenses.show",
        "show",
        serde_json::to_value(schemars::schema_for!(ShowInput)).expect("schema serializes"),
        false,
        handler,
    )
    .with_precheck(precheck)
}

/// What `oxplow.lens.keep` writes for answer `id`: its spec with the shown
/// params as defaults, the worktree it goes in, and its thread. Read in
/// one transaction; refuses an answer that's kept already.
fn kept_lens_tx(
    tx: &rusqlite::Connection,
    project_dir: &Path,
    id: i64,
) -> Result<(LensSpec, PathBuf, i64), DomainError> {
    let answer = answers::get_tx(tx, id)?.ok_or(DomainError::NotFound)?;
    if let Some(kept) = &answer.kept_lens {
        return Err(DomainError::Invalid(format!(
            "it was kept already, as `{kept}`"
        )));
    }
    let mut spec: LensSpec = match &answer.shows {
        AnswerShows::Lens(lens) => {
            return Err(DomainError::Invalid(format!(
                "it shows the lens `{lens}`, which is kept already"
            )))
        }
        AnswerShows::Spec(value) => serde_json::from_value(value.clone())
            .map_err(|e| DomainError::Invalid(format!("its spec doesn't read: {e}")))?,
    };
    // What it was shown with becomes the kept lens's defaults.
    if let Value::Object(given) = &answer.params {
        for p in &mut spec.params {
            if let Some(v) = given.get(&p.name) {
                p.default = Some(SqlCell::from(v.clone()));
            }
        }
    }
    let root = thread_root_tx(tx, project_dir, answer.thread_id)?;
    Ok((spec, root, answer.thread_id))
}

/// What `oxplow.lens.keep` writes, where, and what it keeps.
struct Keeping {
    spec: LensSpec,
    root: PathBuf,
    /// The thread it came from: the answer's, or the caller's.
    thread: Option<i64>,
    answer: Option<i64>,
}

/// What keeping the answer `raw` writes (see [`kept_lens_tx`]).
async fn keeping_answer(target: &LensTarget, raw: &str) -> Result<Keeping, CommandError> {
    let id = answer_id(raw)?;
    let project_dir = target.project_dir.clone();
    let (spec, root, thread) = target
        .db
        .read(move |tx| kept_lens_tx(tx, &project_dir, id))
        .await
        .map_err(|e| match e {
            DomainError::NotFound => invalid("/answer", format!("no answer `answer:{id}`")),
            DomainError::Invalid(m) => invalid("/answer", m),
            other => CommandError::from(other),
        })?;
    Ok(Keeping {
        spec,
        root,
        thread: Some(thread),
        answer: Some(id),
    })
}

/// What keeping `spec` writes: checked as `oxplow.lens.show` checks it, in the
/// worktree of `stream` — else of the caller's thread's stream, else the
/// primary's.
async fn keeping_spec(
    target: &LensTarget,
    spec: LensSpec,
    stream: Option<String>,
    actor: &oxplow_domain::Actor,
) -> Result<Keeping, CommandError> {
    let project_dir = target.project_dir.clone();
    let thread = actor.thread_id().map(|t| t.value());
    let agent = actor.is_agent_driven();
    let keeping = target
        .db
        .read(move |tx| {
            let (stream_id, root) = match stream.as_deref() {
                Some(raw) => match stream_root_tx(tx, &project_dir, raw) {
                    // An agent writes in its own thread's stream only, as
                    // the agent policy keeps its edits there (tsk988).
                    Ok((id, _))
                        if agent && thread.and_then(|t| thread_stream_tx(tx, t)) != Some(id) =>
                    {
                        return Ok(Err(invalid(
                            "/stream",
                            format!(
                                "an agent keeps a lens in its own thread's stream, not `{raw}`"
                            ),
                        )))
                    }
                    Ok((id, root)) => (Some(id), root),
                    Err(DomainError::Invalid(m)) => return Ok(Err(invalid("/stream", m))),
                    Err(e) => return Err(e),
                },
                None => match thread {
                    Some(t) => (
                        thread_stream_tx(tx, t),
                        thread_root_tx(tx, &project_dir, t)?,
                    ),
                    None => (None, project_dir.clone()),
                },
            };
            let lens_ctx = LensContext {
                stream_id,
                thread_id: thread,
                active: None,
            };
            Ok(
                check_spec_shape(&spec, &BTreeMap::new(), &lens_ctx).map(|_| Keeping {
                    spec,
                    root,
                    thread,
                    answer: None,
                }),
            )
        })
        .await
        .map_err(CommandError::from)??;
    // Its query is checked as the explorer runs it (tsk987).
    check_spec_query(&target.sql, &keeping.spec).await?;
    Ok(keeping)
}

/// `oxplow.lens.keep` writes a file, so it's an `External` command: a `Tx`
/// handler may run more than once (the bus retries on a busy database)
/// and a retried file write strands the first. What it keeps is read and
/// checked in one transaction, the lens file written, and a kept answer's
/// row marked kept in another; when that fails the file is removed again,
/// so nothing is left half done. The bus records the run and its
/// `lens.kept@2` after it returns.
fn keep(target: LensTarget) -> Op {
    let handler = Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
        let target = target.clone();
        Box::pin(async move {
            let input: KeepInput = parse(input)?;
            let keeping = match (input.answer, input.spec) {
                (Some(raw), None) => {
                    if input.stream.is_some() {
                        return Err(invalid(
                            "/stream",
                            "an answer is kept in its own thread's worktree; `stream` goes with a `spec`",
                        ));
                    }
                    keeping_answer(&target, &raw).await?
                }
                (None, Some(spec)) => keeping_spec(&target, spec, input.stream, &actor).await?,
                _ => {
                    return Err(invalid(
                        "",
                        "give `answer` (an answer to keep) or `spec` (a lens of its own), not both",
                    ))
                }
            };
            let Keeping {
                spec,
                root,
                thread,
                answer,
            } = keeping;
            let extension = input.extension.unwrap_or_else(|| "my-lenses".into());
            let slug = input
                .slug
                .unwrap_or_else(|| extensions::slug_of(&spec.title));
            let origin = thread.map(|t| thread_ref(ThreadId::new(t)));
            let lens = extensions::save_lens(
                &root,
                &target.project_dir,
                &extension,
                &slug,
                &spec,
                &LensOrigin {
                    purpose: &spec.title,
                    origin: origin.as_deref(),
                },
            )
            .map_err(domain)?;
            if let Some(id) = answer {
                let lens_id = lens.id.clone();
                if let Err(e) = target
                    .db
                    .transaction(move |tx| answers::set_kept_tx(tx, id, &lens_id))
                    .await
                {
                    let _ = std::fs::remove_file(root.join(&lens.path));
                    return Err(CommandError::from(e));
                }
            }
            let subject: Vec<String> = answer
                .map(answer_ref)
                .into_iter()
                .chain([lens_ref(&lens.id)])
                .collect();
            let event = Envelope::typed::<LensKept>(
                actor.source(),
                &LensKeptV2 {
                    answer: answer.map(answer_ref),
                    lens: lens_ref(&lens.id),
                },
            )
            .with_subject(subject);
            Ok(HandlerOutput {
                // `live`: written to the main worktree, so the app shows it
                // now; in another stream's, it shows once that's merged.
                result: json!({
                    "lens": lens.id,
                    "path": lens.path,
                    "live": root == target.project_dir,
                }),
                inverse: None,
                events: vec![event],
                after_commit: None,
                unchanged: false,
            })
        })
    }));
    Op::new(
        "lenses.write",
        "keep",
        serde_json::to_value(schemars::schema_for!(KeepInput)).expect("schema serializes"),
        false,
        handler,
    )
}

/// `oxplow.lens.share` writes, load-checks, then removes the private copy —
/// filesystem work that must happen before the check can run, so it's an
/// `External` command (see `keep`).
fn share(target: LensTarget) -> Op {
    let handler = Handler::External(Arc::new(move |_: Invocation, input| {
        let target = target.clone();
        Box::pin(async move {
            let input: ShareInput = parse(input)?;
            let root = match input.stream {
                Some(raw) => {
                    let project_dir = target.project_dir.clone();
                    target
                        .db
                        .read(move |tx| stream_root_tx(tx, &project_dir, &raw))
                        .await
                        .map_err(|e| match e {
                            DomainError::Invalid(m) => invalid("/stream", m),
                            other => CommandError::from(other),
                        })?
                        .1
                }
                None => target.project_dir.clone(),
            };
            let id = share_lens(&target, &root, &input.lens, &input.extension).await?;
            Ok(HandlerOutput {
                result: json!({ "lens": id }),
                ..HandlerOutput::default()
            })
        })
    }));
    Op::new(
        "lenses.write",
        "share",
        serde_json::to_value(schemars::schema_for!(ShareInput)).expect("schema serializes"),
        false,
        handler,
    )
}

/// Move lens `id` into shared extension `to` under `root`; its new id.
/// Written, then checked — the extension must load clean with the lens
/// in it and the lens's query must pass the `query_sql` authorizer — and
/// only then is the private copy removed; a failed check leaves nothing
/// behind.
async fn share_lens(
    target: &LensTarget,
    root: &Path,
    id: &str,
    to: &str,
) -> Result<String, CommandError> {
    let lens = target.catalog.find_lens(root, id).map_err(domain)?;
    let source = extensions::load_extensions_in(root, &target.project_dir)
        .into_iter()
        .find(|e| e.name == lens.extension)
        .ok_or_else(|| invalid("/lens", format!("no extension `{}`", lens.extension)))?;
    if source.sharing == extensions::Sharing::Shared {
        return Err(invalid("/lens", format!("`{id}` is shared already")));
    }
    let dir = extensions::writable_extension_dir(root, to).map_err(domain)?;
    let manifest = dir.join("extension.yaml");
    let created = !manifest.exists();
    if !created {
        let existing = extensions::load_extensions_in(root, &target.project_dir)
            .into_iter()
            .find(|e| e.name == to)
            .ok_or_else(|| invalid("/extension", format!("`{to}` doesn't load")))?;
        if existing.sharing != extensions::Sharing::Shared {
            return Err(invalid(
                "/extension",
                format!("`{to}` is private; share into a shared extension, or a new one"),
            ));
        }
    }
    let file = dir.join("lenses").join(format!("{}.yaml", lens.slug));
    if file.exists() {
        return Err(invalid(
            "/extension",
            format!("`{to}` has a `{}` lens already", lens.slug),
        ));
    }
    let text = std::fs::read_to_string(root.join(&lens.path)).map_err(|e| {
        CommandError::from(DomainError::Storage(format!("read {}: {e}", lens.path)))
    })?;
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(file.parent().unwrap_or(&dir))?;
        if created {
            std::fs::write(
                &manifest,
                extensions::scaffold_manifest(&extensions::ManifestScaffold {
                    name: to,
                    description: &lens.title,
                    purpose: &lens.title,
                    origin: None,
                    example_name: &lens.slug,
                    example_input: &format!("{{ lens: {} }}", lens.slug),
                    example_expect: "the lens renders",
                    shared: true,
                }),
            )?;
        }
        std::fs::write(&file, &text)
    };
    // Only what this call wrote: the lens file, the manifest it scaffolded,
    // and the directories left empty by removing them (`remove_dir` is not
    // recursive, so anything that was there before stays).
    let undo = || {
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(dir.join("lenses"));
        if created {
            let _ = std::fs::remove_file(&manifest);
            let _ = std::fs::remove_dir(&dir);
        }
    };
    write().map_err(|e| CommandError::from(DomainError::Storage(format!("write {to}: {e}"))))?;
    let loaded = extensions::load_extensions_in(root, &target.project_dir)
        .into_iter()
        .find(|e| e.name == to);
    let problems: Vec<String> = match &loaded {
        None => vec![format!("`{to}` doesn't load")],
        Some(ext) => {
            let mut p = ext.errors.clone();
            if !ext.lenses.iter().any(|l| l.slug == lens.slug) {
                p.push(format!("the lens `{}` doesn't load in `{to}`", lens.slug));
            }
            p
        }
    };
    let query_problem = if lens.query.trim().is_empty() {
        None
    } else {
        target
            .sql
            .check(&lens.query)
            .await
            .err()
            .map(|e| e.to_string())
    };
    if !problems.is_empty() || query_problem.is_some() {
        undo();
        let mut all = problems;
        all.extend(query_problem);
        return Err(invalid(
            "/lens",
            format!("it doesn't pass the shared checks: {}", all.join("; ")),
        ));
    }
    std::fs::remove_file(root.join(&lens.path)).map_err(|e| {
        CommandError::from(DomainError::Storage(format!("remove {}: {e}", lens.path)))
    })?;
    Ok(format!("{to}/{}", lens.slug))
}

pub fn ops(target: LensTarget) -> Vec<Op> {
    vec![show(target.clone()), keep(target.clone()), share(target)]
}

/// Run answer `id` as the person sees it: its lens, or its own spec (as
/// lens `answer/<id>`), with the params it was shown with.
pub async fn run_answer(
    svc: &crate::Services,
    id: i64,
) -> Result<extensions::LensRun, DomainError> {
    Ok(answer_run(svc, id).await?.0)
}

/// Answer `id`'s run and its text rendering — what an agent reads back
/// from `show_lens` — resolved in the answer's own thread's lens context
/// against the main worktree's lenses, the same ones the run used.
pub async fn text_answer(
    svc: &crate::Services,
    id: i64,
) -> Result<(extensions::LensRun, String), DomainError> {
    let (run, root, ctx) = answer_run(svc, id).await?;
    let text = crate::lens_text::text_of(svc, &root, &run, &ctx).await?;
    Ok((run, text))
}

/// The one resolution of an answer: the main worktree (where every lens
/// the app shows lives), its thread's lens context, and the run in them.
async fn answer_run(
    svc: &crate::Services,
    id: i64,
) -> Result<(extensions::LensRun, PathBuf, LensContext), DomainError> {
    let answer = svc
        .thread_answer_store
        .get(id)
        .await?
        .ok_or(DomainError::NotFound)?;
    let thread = ThreadId::new(answer.thread_id);
    let root = svc.worktrees.project_dir().to_path_buf();
    let ctx = extensions::lens_context(svc, None, Some(thread)).await;
    let params = match &answer.params {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| (k.clone(), SqlCell::from(v.clone())))
            .collect(),
        _ => BTreeMap::new(),
    };
    let run = match answer.shows {
        AnswerShows::Lens(lens) => {
            extensions::run_lens(&svc.sql, &svc.extension_catalog, &root, &lens, params, &ctx)
                .await?
        }
        AnswerShows::Spec(value) => {
            let spec: LensSpec = serde_json::from_value(value)
                .map_err(|e| DomainError::Storage(format!("answer {id}'s spec: {e}")))?;
            extensions::run_spec(&svc.sql, &format!("answer/{id}"), &spec, params, &ctx).await?
        }
    };
    Ok((run, root, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    fn agent(fx: &crate::test_fixtures::EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn events_of(fx: &crate::test_fixtures::EffortFixture, t: &str) -> Vec<Value> {
        fx.svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == t)
            .map(|e| e.envelope.payload)
            .collect()
    }

    fn spec() -> Value {
        json!({
            "title": "Busy Tasks",
            "query": "SELECT title, ref AS id FROM v_work_item ORDER BY ref",
            "viz": "bar",
            "chart": { "x": "title", "y": "id" }
        })
    }

    /// P6.C1's red: an agent shows an answer — stored, logged, and it
    /// runs as the person will see it.
    #[tokio::test]
    async fn show_stores_logs_and_runs_an_answer() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let out = fx
            .svc
            .commands
            .run(&agent(&fx), SHOW, json!({ "spec": spec() }), false)
            .await
            .unwrap();
        let answer = out.result["answer"].as_str().unwrap().to_string();
        let id = answer_id(&answer).unwrap();
        let shown = events_of(&fx, "lens.shown").await;
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0]["answer"], answer.as_str());
        assert_eq!(shown[0]["thread"], thread_ref(fx.thread).as_str());
        let row = fx.svc.thread_answer_store.get(id).await.unwrap().unwrap();
        assert_eq!(row.title, "Busy Tasks");
        assert_eq!(row.thread_id, fx.thread.value());
        assert!(row.effort_id.is_some(), "the thread's open effort");
        let run = run_answer(&fx.svc, id).await.unwrap();
        assert_eq!(run.lens.id, format!("answer/{id}"));
        assert_eq!(run.result.columns, vec!["title", "id"]);
        assert!(!run.result.rows.is_empty());
        // Its text rendering comes from the same run, in the answer's own
        // worktree and context — one resolution, not the caller's.
        let (texted, text) = text_answer(&fx.svc, id).await.unwrap();
        assert_eq!(texted.result, run.result);
        assert!(
            text.contains("Busy Tasks") || text.contains("title"),
            "{text}"
        );
    }

    /// A thread in a worktree stream shows and runs the main worktree's
    /// lenses: what the app shows never comes from a stream's copy.
    #[tokio::test]
    async fn a_worktree_thread_shows_the_main_worktrees_lens() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let ext = fx.svc.layout.project_dir.join("oxplow/extensions/demo");
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: demo\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/tasks.yaml"),
            "title: Tasks\nquery: SELECT title FROM v_work_item\n",
        )
        .unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let wt = worktree.path().to_string_lossy().to_string();
        fx.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE streams SET worktree_path = ?1 WHERE id = 1",
                    [wt.as_str()],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let out = fx
            .svc
            .commands
            .run(&agent(&fx), SHOW, json!({ "lens": "demo/tasks" }), false)
            .await
            .unwrap();
        let id = answer_id(out.result["answer"].as_str().unwrap()).unwrap();
        let (run, text) = text_answer(&fx.svc, id).await.unwrap();
        assert_eq!(run.lens.id, "demo/tasks");
        assert!(text.contains("| title |"), "{text}");
    }

    /// An agent shows answers in its own thread only: naming another one
    /// is refused, and nothing is stored there. A person may name any.
    #[tokio::test]
    async fn an_agent_shows_only_in_its_own_thread() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let other = format!("thr{}", fx.thread.value() + 1000);
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                SHOW,
                json!({ "spec": spec(), "thread": other }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/thread" && message.contains("its own thread")),
            "{err:?}"
        );
        // Naming its own thread is fine.
        fx.svc
            .commands
            .run(
                &agent(&fx),
                SHOW,
                json!({ "spec": spec(), "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(events_of(&fx, "lens.shown").await.len(), 1);
    }

    /// Agent SQL gets exactly `query_sql`'s rights: a raw table, a write
    /// or a broken spec is refused, and nothing is stored.
    #[tokio::test]
    async fn show_refuses_what_query_sql_would() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for (bad, field) in [
            (
                json!({ "title": "Raw", "query": "SELECT * FROM task" }),
                "/spec/query",
            ),
            (
                json!({ "title": "Write", "query": "DELETE FROM task" }),
                "/spec/query",
            ),
            (
                json!({ "title": "Bar", "query": "SELECT 1 AS n", "viz": "bar" }),
                "/spec",
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(&agent(&fx), SHOW, json!({ "spec": bad }), false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{err:?}"
            );
        }
        assert!(events_of(&fx, "lens.shown").await.is_empty());
    }

    /// Keeping an answer writes a private lens whose intent says where it
    /// came from; the params it was shown with become its defaults.
    #[tokio::test]
    async fn keep_writes_a_private_lens_from_the_answer() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let mut s = spec();
        s["params"] = json!([{ "name": "limit", "default": 5 }]);
        s["query"] = json!("SELECT title, ref AS id FROM v_work_item ORDER BY ref LIMIT :limit");
        let answer = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                SHOW,
                json!({ "spec": s, "params": { "limit": 2 } }),
                false,
            )
            .await
            .unwrap()
            .result["answer"]
            .as_str()
            .unwrap()
            .to_string();
        let kept = fx
            .svc
            .commands
            .run(&Actor::Human, KEEP, json!({ "answer": answer }), false)
            .await
            .unwrap();
        assert_eq!(kept.result["lens"], "my-lenses/busy-tasks");
        let root = fx.svc.layout.project_dir.clone();
        let manifest =
            std::fs::read_to_string(root.join("oxplow/extensions/my-lenses/extension.yaml"))
                .unwrap();
        assert!(
            manifest.contains(&format!("origin: {}", thread_ref(fx.thread))),
            "{manifest}"
        );
        let lens = fx
            .svc
            .extension_catalog
            .find_lens(&root, "my-lenses/busy-tasks")
            .unwrap();
        assert_eq!(lens.params[0].default, Some(SqlCell::Int(2)));
        assert_eq!(events_of(&fx, "lens.kept").await.len(), 1);
        let again = fx
            .svc
            .commands
            .run(&Actor::Human, KEEP, json!({ "answer": answer }), false)
            .await
            .unwrap_err();
        assert!(again.to_string().contains("kept already"), "{again}");
    }

    /// tsk943: Explore Data's Save as Lens is `oxplow.lens.keep` with a spec — the
    /// lens file it writes is the one keeping an answer that showed the
    /// same spec writes, and its `lens.kept` names no answer.
    #[tokio::test]
    async fn keep_takes_a_spec_and_writes_the_same_lens() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let stream = fx.svc.streams.list_streams().await.unwrap()[0].id;
        let answer = fx
            .svc
            .commands
            .run(&agent(&fx), SHOW, json!({ "spec": spec() }), false)
            .await
            .unwrap()
            .result["answer"]
            .clone();
        fx.svc
            .commands
            .run(
                &Actor::Human,
                KEEP,
                json!({ "answer": answer, "extension": "kept" }),
                false,
            )
            .await
            .unwrap();
        let saved = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                KEEP,
                json!({
                    "spec": spec(),
                    "stream": stream.to_string(),
                    "extension": "saved",
                    "slug": "busy-tasks"
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(saved.result["lens"], "saved/busy-tasks");
        let file = |ext: &str| {
            std::fs::read_to_string(
                root.join(format!("oxplow/extensions/{ext}/lenses/busy-tasks.yaml")),
            )
            .unwrap()
        };
        assert_eq!(file("saved"), file("kept"));
        let kept = events_of(&fx, "lens.kept").await;
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1], json!({ "lens": "lens:saved/busy-tasks" }));
    }

    /// A kept spec gets `oxplow.lens.show`'s check: a query over a physical table
    /// is refused, and nothing is written.
    #[tokio::test]
    async fn keep_refuses_a_spec_over_a_physical_table() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                KEEP,
                json!({
                    "spec": { "title": "Raw", "query": "SELECT kind FROM streams", "viz": "table" },
                    "extension": "saved"
                }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/spec/query" && message.contains("`streams` is a physical table")),
            "{err:?}"
        );
        assert!(!fx
            .svc
            .layout
            .project_dir
            .join("oxplow/extensions/saved")
            .exists());
        assert!(events_of(&fx, "lens.kept").await.is_empty());
    }

    /// tsk987: Explore Data's "Chart a metric" seeds a `metric_grid()`
    /// query; keeping it as a lens checks it the way the explorer runs it
    /// — the metric functions rewritten — so it's kept, not refused as "no
    /// such table".
    #[tokio::test]
    async fn keep_takes_a_metric_spec() {
        let fx = crate::test_fixtures::services_with_effort().await;
        todos_metric(&fx).await;
        let saved = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                KEEP,
                json!({
                    "spec": {
                        "title": "Weekly TODOs",
                        "query": "SELECT bucket, MEASURE('acme.todos') AS todos FROM metric_grid('week')",
                        "viz": "table"
                    },
                    "extension": "saved"
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(saved.result["lens"], "saved/weekly-todos");
    }

    /// A measure (`acme.todo`) and its metric (`acme.todos`).
    async fn todos_metric(fx: &crate::test_fixtures::EffortFixture) {
        fx.svc
            .fact_store
            .upsert_measure(oxplow_db::NewMeasure::new("acme.todo", "TODOs"))
            .await
            .unwrap();
        fx.svc
            .fact_store
            .upsert_spec(oxplow_db::NewMetricSpec::base(
                "acme.todos",
                "TODOs",
                "acme.todo",
                "sum",
            ))
            .await
            .unwrap();
    }

    /// tsk1010: an agent shows a metric chart in its thread — the query is
    /// checked as `query_sql` checks it, metric functions resolved — both
    /// called directly and as a step of a composite; an unknown metric is
    /// refused at the query.
    #[tokio::test]
    async fn show_takes_a_metric_query() {
        let fx = crate::test_fixtures::services_with_effort().await;
        todos_metric(&fx).await;
        let spec = |metric: &str| {
            json!({
                "title": "Weekly TODOs",
                "query": format!("SELECT bucket, MEASURE('{metric}') AS todos FROM metric_grid('week')"),
                "viz": "table"
            })
        };
        fx.svc
            .commands
            .run(
                &agent(&fx),
                SHOW,
                json!({ "spec": spec("acme.todos") }),
                false,
            )
            .await
            .unwrap();
        fx.svc
            .commands
            .run(
                &agent(&fx),
                crate::commands::compose::SEQUENCE,
                json!({ "calls": [{ "name": SHOW, "input": { "spec": spec("acme.todos") } }] }),
                false,
            )
            .await
            .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                SHOW,
                json!({ "spec": spec("acme.nope") }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/spec/query" && message.contains("acme.nope")),
            "{err:?}"
        );
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                crate::commands::compose::SEQUENCE,
                json!({ "calls": [{ "name": SHOW, "input": { "spec": spec("acme.nope") } }] }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/calls/0/input/spec/query"),
            "{err:?}"
        );
        assert_eq!(events_of(&fx, "lens.shown").await.len(), 2);
    }

    /// tsk988: an agent keeps a lens only in its own thread's stream — the
    /// agent policy keeps it out of other streams' worktrees — while a
    /// person may name any stream.
    #[tokio::test]
    async fn an_agent_keeps_a_lens_only_in_its_own_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let fx = crate::test_fixtures::services_with_effort().await;
        let wt = tempfile::tempdir().unwrap();
        let ts = oxplow_domain::Timestamp::from_unix_ms(1_700_000_000_000);
        fx.svc
            .stream_store
            .upsert(&oxplow_domain::Stream {
                id: oxplow_domain::StreamId::new(2),
                kind: oxplow_domain::StreamKind::Worktree,
                title: "other".into(),
                branch: "other".into(),
                branch_ref: "refs/heads/other".into(),
                branch_source: "main".into(),
                worktree_path: wt.path().to_string_lossy().into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: ts,
                updated_at: ts,
                archived_at: None,
            })
            .await
            .unwrap();
        let keep = |actor: Actor, stream: &str, slug: &str| {
            let svc = fx.svc.clone();
            let input =
                json!({ "spec": spec(), "stream": stream, "extension": "kept", "slug": slug });
            async move { svc.commands.run(&actor, KEEP, input, false).await }
        };
        let err = keep(agent(&fx), "str2", "theirs").await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/stream" && message.contains("its own thread's stream")),
            "{err:?}"
        );
        assert!(!wt.path().join("oxplow/extensions/kept").exists());
        let own = fx.svc.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        // Kept in the main worktree, it shows now; in another stream's,
        // once that stream is merged.
        let mine = keep(agent(&fx), &own, "mine").await.unwrap();
        assert_eq!(mine.result["live"], json!(true));
        let theirs = keep(Actor::Human, "str2", "theirs").await.unwrap();
        assert_eq!(theirs.result["live"], json!(false));
        assert!(wt
            .path()
            .join("oxplow/extensions/kept/lenses/theirs.yaml")
            .exists());
    }

    /// tsk988: a kept lens goes to a private extension — moving one into a
    /// shared extension is `oxplow.lens.share`, a person's.
    #[tokio::test]
    async fn a_lens_isnt_kept_into_a_shared_extension() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let team = fx.svc.layout.project_dir.join("oxplow/extensions/team");
        std::fs::create_dir_all(&team).unwrap();
        std::fs::write(
            team.join("extension.yaml"),
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{ name: a }]\n",
        )
        .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                KEEP,
                json!({ "spec": spec(), "extension": "team" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("`team` is shared")
                && err.to_string().contains("oxplow.lens.share"),
            "{err}"
        );
        assert!(!team.join("lenses").exists());
    }

    /// `oxplow.lens.keep` keeps an answer or a spec: neither, or both, is refused;
    /// so is a stream beside an answer, which is kept in its own thread's
    /// worktree.
    #[tokio::test]
    async fn keep_wants_an_answer_or_a_spec() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for input in [
            json!({}),
            json!({ "answer": "answer:1", "spec": spec() }),
            json!({ "answer": "answer:1", "stream": "str1" }),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, KEEP, input.clone(), false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
            assert!(
                err.to_string().contains("`answer`") && err.to_string().contains("`spec`")
                    || err.to_string().contains("`stream`"),
                "{input}: {err}"
            );
        }
    }

    /// Sharing moves a private lens into a shared extension — only when it
    /// passes the shared checks; a failure leaves nothing behind.
    #[tokio::test]
    async fn share_moves_a_passing_lens_and_refuses_a_failing_one() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let mine = root.join("oxplow/extensions/mine");
        std::fs::create_dir_all(mine.join("lenses")).unwrap();
        std::fs::write(
            mine.join("extension.yaml"),
            "manifest: 2\nname: mine\nintent:\n  purpose: mine\n",
        )
        .unwrap();
        std::fs::write(
            mine.join("lenses/good.yaml"),
            "title: Good\nquery: SELECT ref FROM v_work_item\n",
        )
        .unwrap();
        std::fs::write(
            mine.join("lenses/raw.yaml"),
            "title: Raw\nquery: SELECT id FROM task\n",
        )
        .unwrap();
        let share = |lens: &str| {
            let svc = fx.svc.clone();
            let lens = lens.to_string();
            async move {
                svc.commands
                    .run(
                        &Actor::Human,
                        SHARE,
                        json!({ "lens": lens, "extension": "team" }),
                        false,
                    )
                    .await
            }
        };
        let err = share("mine/raw").await.unwrap_err();
        assert!(err.to_string().contains("shared checks"), "{err}");
        assert!(!root.join("oxplow/extensions/team").exists());
        assert!(mine.join("lenses/raw.yaml").exists());

        let out = share("mine/good").await.unwrap();
        assert_eq!(out.result["lens"], "team/good");
        assert!(!mine.join("lenses/good.yaml").exists());
        let team = extensions::load_extensions(&root)
            .into_iter()
            .find(|e| e.name == "team")
            .unwrap();
        assert_eq!(team.sharing, extensions::Sharing::Shared);
        assert!(team.errors.is_empty(), "{:?}", team.errors);
        // An agent can't share.
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                SHARE,
                json!({ "lens": "mine/raw", "extension": "team" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// Both write lens files, which a `Tx` handler may not do: the bus
    /// retries a `Tx` handler on a busy database, and a retried file write
    /// strands the first one. They run as `External` commands, recorded
    /// after they return.
    #[tokio::test]
    async fn keep_and_share_are_external_commands() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for name in [KEEP, SHARE] {
            let spec = fx.svc.commands.spec(name).unwrap();
            assert_eq!(spec.atomicity, oxplow_domain::Atomicity::External, "{name}");
        }
    }

    /// The target is an extension name, checked the way `save_lens` checks
    /// one; and a failed share removes only what it wrote — never a
    /// directory that was there before.
    #[tokio::test]
    async fn share_checks_the_target_name_and_removes_only_what_it_wrote() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let mine = root.join("oxplow/extensions/mine");
        std::fs::create_dir_all(mine.join("lenses")).unwrap();
        std::fs::write(
            mine.join("extension.yaml"),
            "manifest: 2\nname: mine\nintent:\n  purpose: mine\n",
        )
        .unwrap();
        std::fs::write(
            mine.join("lenses/raw.yaml"),
            "title: Raw\nquery: SELECT id FROM task\n",
        )
        .unwrap();
        let share = |to: &str| {
            let svc = fx.svc.clone();
            let to = to.to_string();
            async move {
                svc.commands
                    .run(
                        &Actor::Human,
                        SHARE,
                        json!({ "lens": "mine/raw", "extension": to }),
                        false,
                    )
                    .await
            }
        };
        let err = share("../../src").await.unwrap_err();
        assert!(err.to_string().contains("extension name"), "{err}");
        assert!(!root.join("oxplow/src").exists());
        let err = share("oxplow-bundled").await.unwrap_err();
        assert!(err.to_string().contains("bundled"), "{err}");

        // A directory that isn't an extension yet, holding someone's files.
        let scratch = root.join("oxplow/extensions/scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("notes.txt"), "keep me").unwrap();
        let err = share("scratch").await.unwrap_err();
        assert!(err.to_string().contains("shared checks"), "{err}");
        assert!(
            scratch.join("notes.txt").exists(),
            "a failed share deleted a pre-existing file"
        );
        assert!(!scratch.join("extension.yaml").exists());
        assert!(!scratch.join("lenses").exists());
        assert!(mine.join("lenses/raw.yaml").exists());
    }
}
