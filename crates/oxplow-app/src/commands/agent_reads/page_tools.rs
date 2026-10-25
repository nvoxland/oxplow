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
                let run = PageRunner::of(&svc)
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
                let result = match format {
                    "json" => json!({
                        "page": run.page.id,
                        "title": run.page.title,
                        "params": run.params,
                        "loaders": loaders_json(&run, p.loader.as_deref()),
                        "tree": run.tree,
                    }),
                    _ => json!({
                        "page": run.page.id,
                        "title": run.page.title,
                        "params": run.params,
                        "text": crate::pages::text::page_text(
                            &svc,
                            &svc.layout.project_dir,
                            &run,
                            &ctx,
                        )
                        .await
                        .map_err(|e| page_error(&name, e))?,
                    }),
                };
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
