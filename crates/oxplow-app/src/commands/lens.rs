//! Answers in a thread (P6.C1, target §11.4's lens lifecycle): an agent
//! shows the person something — an existing lens or its own lens spec —
//! with `lens.show`; `lens.keep` writes an answer as a private lens;
//! `lens.share` moves a private lens into a shared extension once it
//! passes the shared checks (committing it is the person's git commit).
//! The rows live in `thread_answer` (`v_thread_answer`); `run_answer`
//! runs one for the Answers strip and for `show_lens`'s text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::semantic_layer::check_query_on;
use oxplow_db::thread_answer_store::{self as answers, AnswerShows};
use oxplow_db::{SqlCell, SqlQuery};
use oxplow_domain::events::schema::{LensKept, LensKeptV1, LensShown, LensShownV1};
use oxplow_domain::refs::build::{answer_ref, lens_ref, thread_ref};
use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, DomainError, Invokers, Lifecycle,
    ThreadId,
};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Command, Handler, HandlerOutput, TxCtx};
use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::{self, LensContext, LensOrigin, LensSpec};

pub const SHOW: &str = "lens.show";
pub const KEEP: &str = "lens.keep";
pub const SHARE: &str = "lens.share";

/// Where the lens commands find lenses and write them.
#[derive(Clone)]
pub struct LensTarget {
    pub project_dir: PathBuf,
    pub catalog: Arc<ExtensionCatalog>,
}

/// `lens.show`: show the person an answer in a thread.
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

/// `lens.keep`: write an answer as a private lens.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepInput {
    /// The answer (`answer:12`).
    pub answer: String,
    /// The extension it goes in; `my-lenses` when omitted.
    pub extension: Option<String>,
    /// Its slug; from its title when omitted.
    pub slug: Option<String>,
}

/// `lens.share`: move a private lens into a shared extension.
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

/// The worktree of `thread`'s stream.
fn thread_root_tx(
    conn: &rusqlite::Connection,
    project_dir: &Path,
    thread: i64,
) -> Result<PathBuf, CommandError> {
    let path: Option<String> = conn
        .query_row(
            "SELECT s.worktree_path FROM threads t JOIN streams s ON s.id = t.stream_id
             WHERE t.id = ?1",
            [thread],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| CommandError::from(DomainError::Storage(e.to_string())))?;
    let path = path.ok_or_else(|| invalid("/thread", format!("no thread `thr{thread}`")))?;
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

fn show(target: LensTarget) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: ShowInput = parse(input)?;
        let thread: i64 = match input.thread.as_deref() {
            Some(raw) => raw
                .parse::<ThreadId>()
                .map_err(|e| invalid("/thread", e.to_string()))?
                .value(),
            None => ctx
                .actor
                .thread_id()
                .ok_or_else(|| invalid("/thread", "no thread given and the caller has none"))?
                .value(),
        };
        let root = thread_root_tx(ctx.conn, &target.project_dir, thread)?;
        let params = input.params.unwrap_or_default();
        let lens_ctx = LensContext {
            stream_id: thread_stream_tx(ctx.conn, thread),
            thread_id: Some(thread),
        };
        let (title, shows, lens) = match (input.lens, input.spec) {
            (Some(id), None) => {
                let lens = target.catalog.find_lens(&root, &id).map_err(domain)?;
                extensions::resolve_params(&lens, &cells(&params), &lens_ctx).map_err(domain)?;
                (lens.title.clone(), AnswerShows::Lens(id.clone()), Some(id))
            }
            (None, Some(spec)) => {
                if let Some(problem) = extensions::spec_problem(&spec) {
                    return Err(invalid("/spec", problem));
                }
                let lens = extensions::Lens::from_spec("answer/new", &spec);
                let bound = extensions::resolve_params(&lens, &cells(&params), &lens_ctx)
                    .map_err(domain)?;
                if !spec.query.trim().is_empty() {
                    // The same authorizer as `query_sql`: one read-only
                    // statement over the published models.
                    check_query_on(
                        ctx.conn,
                        &SqlQuery::new(&spec.query).named(bound.into_iter().collect()),
                    )
                    .map_err(|e| invalid("/spec/query", e.to_string()))?;
                }
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
        })
    }));
    Command::new(
        CommandSpec {
            name: SHOW.into(),
            summary: "Show the person an answer in their thread: an existing lens with params, \
                      or a lens of your own (title, query, viz). It renders beside the \
                      conversation, where they can keep it."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(ShowInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Record,
        },
        handler,
    )
    .expect("lens.show registers")
}

fn keep(target: LensTarget) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: KeepInput = parse(input)?;
        let id = answer_id(&input.answer)?;
        let answer = answers::get_tx(ctx.conn, id)
            .map_err(CommandError::from)?
            .ok_or_else(|| invalid("/answer", format!("no answer `answer:{id}`")))?;
        if let Some(kept) = &answer.kept_lens {
            return Err(invalid(
                "/answer",
                format!("it was kept already, as `{kept}`"),
            ));
        }
        let mut spec: LensSpec = match &answer.shows {
            AnswerShows::Lens(lens) => {
                return Err(invalid(
                    "/answer",
                    format!("it shows the lens `{lens}`, which is kept already"),
                ))
            }
            AnswerShows::Spec(value) => serde_json::from_value(value.clone())
                .map_err(|e| invalid("/answer", format!("its spec doesn't read: {e}")))?,
        };
        // What it was shown with becomes the kept lens's defaults.
        if let Value::Object(given) = &answer.params {
            for p in &mut spec.params {
                if let Some(v) = given.get(&p.name) {
                    p.default = Some(SqlCell::from(v.clone()));
                }
            }
        }
        let root = thread_root_tx(ctx.conn, &target.project_dir, answer.thread_id)?;
        let extension = input.extension.unwrap_or_else(|| "my-lenses".into());
        let slug = input
            .slug
            .unwrap_or_else(|| extensions::slug_of(&spec.title));
        let origin = thread_ref(ThreadId::new(answer.thread_id));
        let lens = extensions::save_lens(
            &root,
            &extension,
            &slug,
            &spec,
            &LensOrigin {
                purpose: &spec.title,
                origin: Some(&origin),
            },
        )
        .map_err(domain)?;
        answers::set_kept_tx(ctx.conn, id, &lens.id).map_err(CommandError::from)?;
        let event = ctx
            .events
            .typed::<LensKept>(&LensKeptV1 {
                answer: answer_ref(id),
                lens: lens_ref(&lens.id),
            })
            .with_subject([answer_ref(id), lens_ref(&lens.id)]);
        Ok(HandlerOutput {
            result: json!({ "lens": lens.id, "path": lens.path }),
            inverse: None,
            events: vec![event],
            after_commit: None,
        })
    }));
    Command::new(
        CommandSpec {
            name: KEEP.into(),
            summary: "Keep an answer from a thread as a private lens (a page the person can \
                      reopen, pin and share), recording where it came from."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(KeepInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Write,
        },
        handler,
    )
    .expect("lens.keep registers")
}

fn share(target: LensTarget) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: ShareInput = parse(input)?;
        let root = match input.stream.as_deref() {
            Some(raw) => {
                let stream: oxplow_domain::StreamId = raw
                    .parse()
                    .map_err(|_| invalid("/stream", format!("`{raw}` isn't a stream")))?;
                let path: String = ctx
                    .conn
                    .query_row(
                        "SELECT worktree_path FROM streams WHERE id = ?1",
                        [stream.value()],
                        |r| r.get(0),
                    )
                    .map_err(|_| invalid("/stream", format!("no stream `{raw}`")))?;
                crate::worktrees::workspace_path(&target.project_dir, &path)
            }
            None => target.project_dir.clone(),
        };
        share_lens(ctx.conn, &target, &root, &input.lens, &input.extension).map(|id| {
            HandlerOutput {
                result: json!({ "lens": id }),
                ..HandlerOutput::default()
            }
        })
    }));
    Command::new(
        CommandSpec {
            name: SHARE.into(),
            summary: "Move a private lens into a shared extension (created shared when \
                      missing), refused unless it passes the shared checks; committing it is \
                      how the team gets it."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(ShareInput))
                .expect("schema serializes"),
            invokers: Invokers::HUMAN_ONLY,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Write,
        },
        handler,
    )
    .expect("lens.share registers")
}

/// Move lens `id` into shared extension `to` under `root`; its new id.
/// Written, then checked — the extension must load clean with the lens
/// in it and the lens's query must pass the `query_sql` authorizer — and
/// only then is the private copy removed; a failed check leaves nothing
/// behind.
fn share_lens(
    conn: &rusqlite::Connection,
    target: &LensTarget,
    root: &Path,
    id: &str,
    to: &str,
) -> Result<String, CommandError> {
    let lens = target.catalog.find_lens(root, id).map_err(domain)?;
    let source = extensions::load_extensions(root)
        .into_iter()
        .find(|e| e.name == lens.extension)
        .ok_or_else(|| invalid("/lens", format!("no extension `{}`", lens.extension)))?;
    if source.sharing == extensions::Sharing::Shared {
        return Err(invalid("/lens", format!("`{id}` is shared already")));
    }
    let dir = root.join(extensions::EXTENSIONS_DIR).join(to);
    let manifest = dir.join("extension.yaml");
    let created = !manifest.exists();
    if !created {
        let existing = extensions::load_extensions(root)
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
    let undo = || {
        let _ = std::fs::remove_file(&file);
        if created {
            let _ = std::fs::remove_dir_all(&dir);
        }
    };
    write().map_err(|e| CommandError::from(DomainError::Storage(format!("write {to}: {e}"))))?;
    let loaded = extensions::load_extensions(root)
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
    let query_problem = (!lens.query.trim().is_empty())
        .then(|| check_query_on(conn, &SqlQuery::new(&lens.query)).err())
        .flatten();
    if !problems.is_empty() || query_problem.is_some() {
        undo();
        let mut all = problems;
        all.extend(query_problem.map(|e| e.to_string()));
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

pub fn commands(target: LensTarget) -> Vec<Command> {
    vec![show(target.clone()), keep(target.clone()), share(target)]
}

/// Run answer `id` as the person sees it: its lens, or its own spec (as
/// lens `answer/<id>`), with the params it was shown with.
pub async fn run_answer(
    svc: &crate::Services,
    id: i64,
) -> Result<extensions::LensRun, DomainError> {
    let answer = svc
        .thread_answer_store
        .get(id)
        .await?
        .ok_or(DomainError::NotFound)?;
    let thread = ThreadId::new(answer.thread_id);
    let root = {
        use oxplow_domain::stores::ThreadStore as _;
        let stream = svc
            .thread_store
            .get(&thread)
            .await?
            .map(|t| t.stream_id.to_string());
        svc.worktrees.resolve(stream.as_deref()).await
    };
    let ctx = extensions::lens_context(svc, None, Some(thread)).await;
    let params = match &answer.params {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| (k.clone(), SqlCell::from(v.clone())))
            .collect(),
        _ => BTreeMap::new(),
    };
    match answer.shows {
        AnswerShows::Lens(lens) => {
            extensions::run_lens(&svc.sql, &svc.extension_catalog, &root, &lens, params, &ctx).await
        }
        AnswerShows::Spec(value) => {
            let spec: LensSpec = serde_json::from_value(value)
                .map_err(|e| DomainError::Storage(format!("answer {id}'s spec: {e}")))?;
            extensions::run_spec(&svc.sql, &format!("answer/{id}"), &spec, params, &ctx).await
        }
    }
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
            "query": "SELECT title, id FROM v_task ORDER BY id",
            "viz": "bar",
            "chart": { "x": "title", "y": "id" }
        })
    }

    /// P6.C1's red: an agent shows an answer — stored, logged, and it
    /// runs as the person will see it.
    #[tokio::test]
    async fn show_stores_logs_and_runs_an_answer() {
        let fx = crate::test_fixtures::services_with_effort().await;
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
        s["query"] = json!("SELECT title, id FROM v_task ORDER BY id LIMIT :limit");
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
            "title: Good\nquery: SELECT id FROM v_task\n",
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
}
