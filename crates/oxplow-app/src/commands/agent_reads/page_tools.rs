//! Pages for an agent: `pages.read/run` reads what a page shows (its text,
//! each node under `[node <id>]`, or its loaders' rows), `pages.read/find`
//! finds the pages that answer a question, `pages.read/definition` says
//! what a page is made of and where each node comes from, and
//! `pages.write/run_action` presses a page's action as the page acting for
//! the caller. Foundation declares the commands (`oxplow.page.run`, …) and
//! offers the common ones as tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::{CommandError, DomainError, StreamId, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use super::{check_stream_id, stream_or_actors};
use crate::pages::runner::{PageRun, PageRunner, RunNode, RunRequest, RunTarget};
use crate::pages::schema::{parse_inline_page, NodeKind};
use crate::Services;

/// Rows of one loader an agent reads in JSON.
pub const JSON_ROWS: usize = 200;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunPageInput {
    /// The page: `<extension>/<slug>` (see `oxplow.page.find`, `v_page`).
    #[serde(default)]
    pub page: Option<String>,
    /// Or an inline page of your own, as YAML: `title`, `data:` (sql /
    /// rows / page loaders) and `view:` — it only reads.
    #[serde(default)]
    pub spec: Option<String>,
    /// Param values by name (see the page's `params`, or `find`'s
    /// answer); the rest take the viewer's context or their defaults.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// Just this loader's rows (`format: "json"`).
    #[serde(default)]
    pub loader: Option<String>,
    /// `text` (default): the page as the person sees it, each node under
    /// `[node <id>]`. `json`: each loader's columns and rows (up to 200)
    /// and the tree.
    #[serde(default)]
    pub format: Option<String>,
    /// Stream whose data to read; omit for your own.
    #[serde(default)]
    pub stream_id: Option<String>,
    /// Your thread (`thr…`), bound into a `thread_id` param.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Read your stream's draft of the page's extension (its source in
    /// your worktree, not yet deployed) over live data, instead of what
    /// runs. Its commands are the deployed ones.
    #[serde(default)]
    pub draft: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageDefinitionInput {
    /// The page: `<extension>/<slug>`.
    pub page: String,
    /// Param values by name, as for `oxplow.page.run`: which embeds and
    /// switch children it resolves to depend on them.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub stream_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunPageActionInput {
    /// The page: `<extension>/<slug>`.
    pub page: String,
    /// The action's id: a whole-page action (in the page's `actions`), or
    /// a node's (with `node`).
    pub action: String,
    /// The node whose action it is (`[node <id>]` in the page's text);
    /// omit for a whole-page action.
    #[serde(default)]
    pub node: Option<String>,
    /// Param values by name, as for `oxplow.page.run`.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// For a row action: the row it runs on, column → value, as
    /// `oxplow.page.run` returned it (it must be one of the node's rows).
    #[serde(default)]
    pub row: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindPageInput {
    /// What you want to know, in words ("which tests failed this week").
    #[serde(default)]
    pub question: Option<String>,
    /// A uri the page should be about (`oxplow:oxplow/commit?sha=abc1234`):
    /// only the pages that show that kind of thing, its uri bound.
    #[serde(default)]
    pub about: Option<String>,
    /// How many to return (default 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The words of `text` worth matching on: lowercase, three letters or
/// more, the commonest words left out.
fn words(text: &str) -> Vec<String> {
    const COMMON: &[&str] = &[
        "the", "and", "for", "are", "was", "what", "which", "who", "how", "this", "that", "with",
        "from", "did", "does", "has", "have", "show", "list", "all", "any", "its", "there",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 3 && !COMMON.contains(&w.as_str()))
        .collect()
}

fn overlap(question: &[String], text: &str) -> usize {
    let theirs = words(text);
    question.iter().filter(|w| theirs.contains(w)).count()
}

/// The pages that answer `question` (and show `about`), best first.
fn find_pages(svc: &Services, p: &FindPageInput) -> Result<Value, CommandError> {
    let about = p
        .about
        .as_deref()
        .map(|u| {
            oxplow_domain::uri::OxplowUri::parse(u)
                .map(|parsed| parsed.resource().to_string())
                .map_err(|e| invalid("/about", format!("`{u}` isn't an oxplow uri: {e}")))
        })
        .transpose()?;
    let question = p.question.as_deref().map(words).unwrap_or_default();
    let extensions = svc.extension_catalog.unpinned(
        &svc.layout.project_dir,
        "an agent's find reads one snapshot",
    );
    let mut scored: Vec<(usize, Value)> = Vec::new();
    for ext in extensions.iter().filter(|e| e.enabled) {
        for page in &ext.page_files {
            if let Some(kind) = &about {
                let shows = page.renders.iter().any(|r| r == kind)
                    || page
                        .answers
                        .iter()
                        .any(|a| a.about.as_deref() == Some(kind));
                if !shows {
                    continue;
                }
            }
            let matched: Vec<&str> = page
                .answers
                .iter()
                .filter(|a| !question.is_empty() && overlap(&question, &a.question) > 0)
                .map(|a| a.question.as_str())
                .collect();
            let score = page
                .answers
                .iter()
                .map(|a| overlap(&question, &a.question) * 3)
                .max()
                .unwrap_or(0)
                + overlap(&question, &page.title) * 2
                + overlap(&question, page.keywords.as_deref().unwrap_or_default())
                + overlap(&question, &page.description);
            if !question.is_empty() && score == 0 {
                continue;
            }
            let bind = match (&p.about, page.params.iter().any(|x| x.name == "uri")) {
                (Some(uri), true) => json!({ "uri": uri }),
                _ => Value::Null,
            };
            let params: Vec<Value> = page
                .params
                .iter()
                .filter(|x| !crate::extensions::LensContext::supplies(&x.name))
                .map(|x| {
                    json!({
                        "name": x.name,
                        "type": x.kind,
                        "role": x.role,
                        "label": x.label,
                        "default": x.default,
                        "options": x.options.iter().map(|o| &o.value).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let summary = page
                .description
                .split_inclusive(". ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            scored.push((
                score,
                json!({
                    "page": page.id,
                    "title": page.title,
                    "summary": summary,
                    "params": params,
                    "bind": bind,
                    "questions": matched,
                    "tool": page.tool,
                }),
            ));
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1["title"].as_str().cmp(&b.1["title"].as_str()))
    });
    let limit = p.limit.unwrap_or(10).clamp(1, 50);
    let pages: Vec<Value> = scored.into_iter().take(limit).map(|(_, v)| v).collect();
    Ok(json!({ "pages": pages }))
}

fn find_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "find",
        schema::<FindPageInput>(),
        false,
        Handler::External(Arc::new(move |_invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: FindPageInput = parse(input)?;
                if p.question.is_none() && p.about.is_none() {
                    return Err(invalid(
                        "/question",
                        "give a `question`, an `about` uri, or both".to_string(),
                    ));
                }
                let svc = services.get()?;
                Ok(HandlerOutput {
                    result: find_pages(&svc, &p)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

fn cells(values: Option<BTreeMap<String, Value>>) -> BTreeMap<String, SqlCell> {
    values
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, SqlCell::from(v)))
        .collect()
}

async fn context(
    svc: &Services,
    invocation: &Invocation,
    stream_id: Option<String>,
    thread_id: Option<String>,
) -> Result<crate::extensions::LensContext, CommandError> {
    if let Some(stream) = &stream_id {
        check_stream_id("/stream_id", stream)?;
    }
    let stream = stream_or_actors(svc, invocation.actor(), stream_id).await?;
    let thread = thread_id
        .map(|t| {
            ThreadId::try_from_str(&t).ok_or_else(|| {
                invalid(
                    "/thread_id",
                    format!("expected a thread id (thr…), got `{t}`"),
                )
            })
        })
        .transpose()?;
    Ok(crate::extensions::lens_context(
        svc,
        stream.as_deref().and_then(StreamId::try_from_str),
        thread,
    )
    .await)
}

fn page_error(page: &str, e: DomainError) -> CommandError {
    match e {
        DomainError::NotFound => invalid(
            "/page",
            format!(
                "no page `{page}` (ids are `<extension>/<slug>`; see `oxplow.page.find`, `v_page`)"
            ),
        ),
        DomainError::Invalid(message) => CommandError::Invalid {
            field: None,
            message,
        },
        other => failed(other),
    }
}

/// What to run: a page by id or an inline page's YAML.
fn target(page: Option<String>, spec: Option<String>) -> Result<RunTarget, CommandError> {
    match (page, spec) {
        (Some(p), None) => Ok(RunTarget::Page(p)),
        (None, Some(yaml)) => Ok(RunTarget::Spec(Arc::new(
            parse_inline_page(&yaml).map_err(|m| invalid("/spec", m))?,
        ))),
        _ => Err(invalid(
            "/page",
            "give `page` or `spec`, one of them".to_string(),
        )),
    }
}

/// A run's loaders as JSON: columns, rows (up to [`JSON_ROWS`]), whether
/// more were cut, and any error.
fn loaders_json(run: &PageRun, only: Option<&str>) -> Value {
    let mut out = serde_json::Map::new();
    for (name, l) in &run.loaders {
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let rows: Vec<&Vec<SqlCell>> = l.result.rows.iter().take(JSON_ROWS).collect();
        out.insert(
            name.clone(),
            json!({
                "columns": l.result.columns,
                "rows": rows,
                "rowCount": l.result.rows.len(),
                "truncated": l.result.truncated || l.result.rows.len() > JSON_ROWS,
                "error": l.error,
            }),
        );
    }
    Value::Object(out)
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        run_op(services.clone()),
        find_op(services.clone()),
        definition_op(services.clone()),
        run_action_op(services),
    ]
}

fn run_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "run",
        schema::<RunPageInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: RunPageInput = parse(input)?;
                let format = p.format.as_deref().unwrap_or("text");
                if !matches!(format, "text" | "json") {
                    return Err(invalid(
                        "/format",
                        format!("format `{format}` isn't text or json"),
                    ));
                }
                let svc = services.get()?;
                let ctx =
                    context(&svc, &invocation, p.stream_id.clone(), p.thread_id.clone()).await?;
                let name = p.page.clone().unwrap_or_else(|| "inline".into());
                // A draft: the page's extension as the caller's stream's
                // checkout has it.
                let mut draft_of: Option<(String, std::path::PathBuf)> = None;
                let runner = if p.draft.unwrap_or(false) {
                    let page = p.page.as_deref().ok_or_else(|| {
                        invalid("/draft", "a draft is of a page: give `page`".to_string())
                    })?;
                    let extension = page.split_once('/').map_or(page, |(e, _)| e).to_string();
                    let stream =
                        stream_or_actors(&svc, invocation.actor(), p.stream_id.clone()).await?;
                    let checkout = svc
                        .worktrees
                        .resolve(stream.as_deref())
                        .await
                        .into_local_path();
                    let runner = PageRunner::draft(&svc, &checkout, &extension)
                        .await
                        .map_err(|e| page_error(&name, e))?;
                    draft_of = Some((stream.unwrap_or_default(), checkout));
                    runner
                } else {
                    PageRunner::of(&svc)
                };
                let run = runner
                    .run(RunRequest {
                        target: target(p.page, p.spec)?,
                        params: cells(p.params),
                        ctx: ctx.clone(),
                        viewer: invocation.actor().clone(),
                    })
                    .await
                    .map_err(|e| page_error(&name, e))?;
                if let Some(l) = &p.loader {
                    if !run.loaders.contains_key(l) {
                        return Err(invalid(
                            "/loader",
                            format!("page `{name}` ran no loader `{l}`"),
                        ));
                    }
                }
                let mut result = match format {
                    "json" => json!({
                        "page": run.page.id,
                        "title": run.page.title,
                        "params": run.params,
                        "loaders": loaders_json(&run, p.loader.as_deref()),
                        "tree": run.tree,
                    }),
                    _ => {
                        let mut text = crate::pages::text::page_text(
                            &svc,
                            &svc.layout.project_dir,
                            &run,
                            &ctx,
                        )
                        .await
                        .map_err(|e| page_error(&name, e))?;
                        if let Some((stream, _)) = &draft_of {
                            text = format!(
                                "Draft of {} (stream {stream}, not deployed)\n{text}",
                                run.page.id
                            );
                        }
                        json!({
                            "page": run.page.id,
                            "title": run.page.title,
                            "params": run.params,
                            "text": text,
                        })
                    }
                };
                if let Some((stream, checkout)) = &draft_of {
                    result["draft"] =
                        json!({ "stream": stream, "checkout": checkout.to_string_lossy() });
                }
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// One row of a page's definition: a node, where it sits and where it
/// comes from.
#[derive(Debug, Clone, Serialize)]
struct DefinitionRow {
    depth: usize,
    /// Its id as `#node=` names it (`<embed>/<id>` inside an embed).
    id: String,
    kind: &'static str,
    title: Option<String>,
    /// The page it belongs to (an embed's own page for its nodes).
    page: String,
    extension: String,
    /// The file it is declared in.
    file: String,
    /// The loader it shows, and what that loader is.
    loader: Option<String>,
    loader_kind: Option<&'static str>,
    source: Option<String>,
}

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Stack { .. } => "stack",
        NodeKind::Grid { .. } => "grid",
        NodeKind::Switch { .. } => "switch",
        NodeKind::Viz { .. } => "viz",
        NodeKind::Embed { .. } => "embed",
        NodeKind::Slot { .. } => "slot",
        NodeKind::Markdown { .. } => "markdown",
        NodeKind::Component { .. } => "component",
    }
}

fn definition_rows(
    run: &PageRun,
    at: &RunNode,
    prefix: &str,
    depth: usize,
    out: &mut Vec<DefinitionRow>,
) {
    use crate::pages::schema::Loader;
    let Some(def) = crate::pages::text::node(&run.page, &at.id) else {
        // A page another extension mounted at a slot.
        if let Some(e) = &at.embed {
            let id = format!("{prefix}{}", at.id);
            definition_rows(e, &e.tree, &format!("{id}/"), depth + 1, out);
        }
        return;
    };
    let from = match &def.kind {
        NodeKind::Viz { from, .. } => from.clone(),
        _ => None,
    };
    let decl = from.as_deref().and_then(|f| run.page.loader(f));
    let (loader_kind, source) = match decl.map(|d| &d.loader) {
        Some(Loader::Sql { sql, .. }) => (Some("sql"), Some(sql.clone())),
        Some(Loader::Command { command, .. }) => (Some("command"), Some(command.clone())),
        Some(Loader::Page { page, loader, .. }) => (Some("page"), Some(format!("{page}#{loader}"))),
        Some(Loader::Rows { .. }) => (Some("rows"), None),
        None => (None, None),
    };
    let id = format!("{prefix}{}", at.id);
    out.push(DefinitionRow {
        depth,
        id: id.clone(),
        kind: kind_name(&def.kind),
        title: def.title.clone(),
        page: run.page.id.clone(),
        extension: run.page.extension.clone(),
        file: run
            .page
            .origin
            .source
            .clone()
            .unwrap_or_else(|| run.page.origin.running.clone()),
        loader: from,
        loader_kind,
        source,
    });
    for c in &at.children {
        definition_rows(run, c, prefix, depth + 1, out);
    }
    if let Some(e) = &at.embed {
        definition_rows(e, &e.tree, &format!("{id}/"), depth + 1, out);
    }
}

fn definition_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "definition",
        schema::<PageDefinitionInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: PageDefinitionInput = parse(input)?;
                let svc = services.get()?;
                let ctx = context(&svc, &invocation, p.stream_id.clone(), None).await?;
                let run = PageRunner::of(&svc)
                    .run(RunRequest {
                        target: RunTarget::Page(p.page.clone()),
                        params: cells(p.params),
                        ctx,
                        viewer: invocation.actor().clone(),
                    })
                    .await
                    .map_err(|e| page_error(&p.page, e))?;
                let mut rows = Vec::new();
                definition_rows(&run, &run.tree, "", 0, &mut rows);
                let page = &run.page;
                Ok(HandlerOutput {
                    result: json!({
                        "page": page.id,
                        "title": page.title,
                        "extension": page.extension,
                        "source": page.origin.source,
                        "running": page.origin.running,
                        "deployment": page.origin.deployment,
                        "bundled": page.origin.bundled,
                        "params": page.params,
                        "loaders": page.loaders,
                        "nodes": rows,
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

fn run_action_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.write",
        "run_action",
        schema::<RunPageActionInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: RunPageActionInput = parse(input)?;
                let svc = services.get()?;
                let ctx =
                    context(&svc, &invocation, p.stream_id.clone(), p.thread_id.clone()).await?;
                // A run of its own, held for the press: the row is matched
                // against what it holds, as a person's press is.
                let runs = crate::pages::open_runs::PageRuns::default();
                let (run_id, _) = runs
                    .open(
                        &svc,
                        RunRequest {
                            target: RunTarget::Page(p.page.clone()),
                            params: cells(p.params),
                            ctx,
                            viewer: invocation.actor().clone(),
                        },
                        "agent",
                    )
                    .await
                    .map_err(|e| page_error(&p.page, e))?;
                let outcome = crate::pages::actions::run_page_action(
                    &svc,
                    &runs,
                    crate::pages::actions::PageActionCall {
                        run_id,
                        node: p.node,
                        action: p.action,
                        row: p.row.map(|r| cells(Some(r))),
                        input: None,
                        drop: None,
                        on_behalf_of: invocation.actor().clone(),
                        // An agent never confirms.
                        confirmed: false,
                    },
                )
                .await?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(outcome).map_err(failed)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

#[cfg(test)]
mod tests {
    use oxplow_domain::Actor;
    use serde_json::json;

    use crate::pages::test_support::fixture;

    async fn run(
        svc: &crate::Services,
        command: &str,
        input: serde_json::Value,
    ) -> serde_json::Value {
        svc.commands
            .run(&Actor::Human, command, input, false)
            .await
            .unwrap_or_else(|e| panic!("{command}: {e:?}"))
            .result
    }

    #[tokio::test]
    async fn an_agent_reads_a_page_as_text_or_as_its_loaders_rows() {
        let (_dir, svc) = fixture().await;
        let text = run(&svc, "oxplow.page.run", json!({ "page": "acme/numbers" })).await;
        assert!(
            text["text"].as_str().unwrap().contains("[node big]"),
            "{text}"
        );
        let rows = run(
            &svc,
            "oxplow.page.run",
            json!({ "page": "acme/numbers", "params": { "n": 3 }, "format": "json", "loader": "big" }),
        )
        .await;
        assert_eq!(rows["loaders"]["big"]["rows"], json!([[3]]), "{rows}");
        assert!(rows["loaders"].get("count").is_none(), "{rows}");
        // An inline page of its own.
        let inline = run(
            &svc,
            "oxplow.page.run",
            json!({ "spec": "title: Mine\ndata:\n  a: { sql: SELECT 5 AS five }\nview: { viz: number, from: a }\n", "format": "json" }),
        )
        .await;
        assert_eq!(inline["loaders"]["a"]["rows"], json!([[5]]), "{inline}");
    }

    /// An agent finds pages by what they answer: a question matches the
    /// pages' questions first, then titles and descriptions; `about` keeps
    /// the pages for that resource and binds its uri.
    #[tokio::test]
    async fn an_agent_finds_the_pages_that_answer_a_question() {
        let (_dir, svc) = fixture().await;
        let found = run(
            &svc,
            "oxplow.page.find",
            json!({ "question": "which numbers are big" }),
        )
        .await;
        let pages = found["pages"].as_array().unwrap();
        assert_eq!(pages[0]["page"], json!("acme/numbers"), "{found}");
        assert_eq!(
            pages[0]["questions"],
            json!(["Which numbers are big?"]),
            "{found}"
        );
        assert_eq!(pages[0]["params"][0]["name"], json!("n"), "{found}");
        let uri = "oxplow:oxplow/commit?sha=abc1234";
        let about = run(&svc, "oxplow.page.find", json!({ "about": uri })).await;
        let pages = about["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 1, "{about}");
        assert_eq!(pages[0]["page"], json!("acme/commit-look"));
        assert_eq!(pages[0]["bind"], json!({ "uri": uri }));
        let none = run(&svc, "oxplow.page.find", json!({ "question": "zebras" })).await;
        assert_eq!(none["pages"], json!([]), "{none}");
    }

    #[tokio::test]
    async fn a_pages_definition_names_each_node_and_where_it_comes_from() {
        let (_dir, svc) = fixture().await;
        let def = run(
            &svc,
            "oxplow.page.definition",
            json!({ "page": "acme/host" }),
        )
        .await;
        let nodes = def["nodes"].as_array().unwrap();
        let picked = nodes
            .iter()
            .find(|n| n["id"] == json!("picked/echo"))
            .unwrap_or_else(|| panic!("{def}"));
        assert_eq!(picked["page"], json!("acme/picked"));
        assert_eq!(picked["loader_kind"], json!("sql"));
        assert!(
            picked["file"]
                .as_str()
                .unwrap()
                .ends_with("pages/picked.yaml"),
            "{picked}"
        );
    }

    #[tokio::test]
    async fn an_agent_presses_a_pages_row_action() {
        let (_dir, svc) = fixture().await;
        run(
            &svc,
            "oxplow.page.run_action",
            json!({ "page": "acme/acts", "node": "names", "action": "add-row", "row": { "name": "alpha" } }),
        )
        .await;
        let titles = svc
            .sql
            .run(oxplow_db::SqlQuery::new("SELECT title FROM v_work_item"))
            .await
            .unwrap();
        assert_eq!(
            titles.rows,
            vec![vec![oxplow_db::SqlCell::Text("alpha".into())]]
        );
    }
}

#[cfg(test)]
mod draft_tests {
    use oxplow_domain::stores::StreamStore as _;
    use oxplow_domain::{Actor, StreamId, StreamKind};
    use serde_json::json;

    use crate::pages::test_support::{fixture, write};

    /// An agent in a worktree reads its draft of a page — the copy in its
    /// checkout, over live data — and the live page without `draft`.
    #[tokio::test]
    async fn an_agent_reads_its_worktrees_draft_of_a_page() {
        let (_dir, svc) = fixture().await;
        let wt = tempfile::tempdir().unwrap();
        write(
            wt.path(),
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        write(
            wt.path(),
            ".oxplow/extensions/acme/pages/numbers.yaml",
            "title: Numbers Draft\ndata:\n  big: { sql: \"SELECT 99 AS n\" }\nview: { viz: table, from: big }\n",
        );
        let mut other = svc.stream_store.list().await.unwrap().pop().unwrap();
        other.id = StreamId::placeholder();
        other.title = "other".into();
        other.branch = "draft".into();
        other.kind = StreamKind::Worktree;
        other.worktree_path = wt.path().to_string_lossy().into_owned();
        let stream = svc.stream_store.upsert(&other).await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream, "t").await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(thread.id),
            stream_id: Some(stream),
        };
        let run = |input| {
            let svc = svc.clone();
            let agent = agent.clone();
            async move {
                svc.commands
                    .run(&agent, "oxplow.page.run", input, false)
                    .await
                    .unwrap()
                    .result
            }
        };
        let draft = run(json!({ "page": "acme/numbers", "draft": true, "format": "json" })).await;
        assert_eq!(draft["title"], json!("Numbers Draft"), "{draft}");
        assert_eq!(draft["loaders"]["big"]["rows"], json!([[99]]), "{draft}");
        assert_eq!(
            draft["draft"]["stream"],
            json!(stream.to_string()),
            "{draft}"
        );
        let text = run(json!({ "page": "acme/numbers", "draft": true })).await;
        assert!(
            text["text"]
                .as_str()
                .unwrap()
                .starts_with("Draft of acme/numbers"),
            "{text}"
        );
        let live = run(json!({ "page": "acme/numbers", "format": "json" })).await;
        assert_eq!(live["title"], json!("Numbers"), "{live}");
    }
}
