//! Pages for an agent: `pages.read/run` reads what a page shows (its text,
//! each node under `[node <id>]`, or its loaders' rows), `pages.read/find`
//! finds the pages that answer a question, `pages.read/describe` says
//! what a page file is made of and where each node comes from, and
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
use crate::pages::schema::{
    parse_inline_page, parse_temp_page, temp_file_name, NodeKind, PageKind, TEMP_EXTENSION,
};
use crate::Services;

/// Rows of one loader an agent reads in JSON.
pub const JSON_ROWS: usize = 200;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunPageInput {
    /// The page: `<extension>/<slug>` (see `oxplow.page.find`, `v_page`).
    #[serde(default)]
    pub page: Option<String>,
    /// Or an inline page of your own, as YAML: `title`, `loaders:` (sql /
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
pub struct DescribePageInput {
    /// The page file, as a file uri (`oxplow:oxplow/file?path=…`): on
    /// `oxplow:extensions` a page that runs (`<extension>/<pages|parts|queries>/<slug>.yaml`,
    /// at its `rev`), on `oxplow:temp` a view's file, on the worktree a
    /// page file under `extensions/<name>/<folder>/` or
    /// `.oxplow/extensions/<name>/<folder>/`.
    pub uri: String,
    /// For a worktree file: its text as you have it (an edit not saved
    /// yet); read when omitted. Other filesystems are always read.
    #[serde(default)]
    pub text: Option<String>,
    /// Param values by name, as for `oxplow.page.run`: which embeds and
    /// switch children it resolves to depend on them. A JSON object, or
    /// that object as a string (as a page's param carries it).
    #[serde(default, deserialize_with = "object_or_json_text")]
    #[schemars(with = "Option<ParamsArg>")]
    pub params: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub stream_id: Option<String>,
}

/// The schema of [`object_or_json_text`]'s input.
#[derive(JsonSchema)]
#[allow(dead_code)]
#[serde(untagged)]
enum ParamsArg {
    Values(BTreeMap<String, Value>),
    /// The object as JSON text.
    Text(String),
}

/// An object, or the same object written as JSON text (empty text: none).
fn object_or_json_text<'de, D>(d: D) -> Result<Option<BTreeMap<String, Value>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| D::Error::custom(format!("`params` text isn't a JSON object: {e}"))),
        Some(other) => serde_json::from_value(other)
            .map(Some)
            .map_err(D::Error::custom),
    }
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderersInput {
    /// The uri to draw (`oxplow:oxplow/file?path=src/a.rs`, an `https://`
    /// url); a `#` view on it is ignored.
    pub uri: String,
    /// Stream whose files and data the check reads; omit for your own.
    #[serde(default)]
    pub stream_id: Option<String>,
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

/// The pages that answer `question` (and show `about`), best first. A
/// resource page shows `about` when `pages::renderers` offers it for that
/// uri; a standalone page when an answer of its says it is about the
/// uri's resource.
async fn find_pages(
    svc: &Services,
    p: &FindPageInput,
    ctx: &crate::pages::spec::ViewContext,
    viewer: &oxplow_domain::Actor,
) -> Result<Value, CommandError> {
    let about = p
        .about
        .as_deref()
        .map(|u| {
            oxplow_domain::uri::OxplowUri::parse(u)
                .map(|parsed| parsed.resource().to_string())
                .map_err(|e| invalid("/about", format!("`{u}` isn't an oxplow uri: {e}")))
        })
        .transpose()?;
    let offered: Vec<String> = match &p.about {
        Some(uri) => crate::pages::renderers::renderers_for(svc, uri, ctx, viewer)
            .await
            .into_iter()
            .map(|r| r.page)
            .collect(),
        None => Vec::new(),
    };
    let question = p.question.as_deref().map(words).unwrap_or_default();
    let extensions = svc.extension_catalog.unpinned(
        &svc.layout.project_dir,
        "an agent's find reads one snapshot",
    );
    let mut scored: Vec<(usize, Value)> = Vec::new();
    for ext in extensions.iter().filter(|e| e.enabled) {
        for page in &ext.page_files {
            // A resource page is a display mode of its resource: found
            // through `about`, never listed on its own.
            if about.is_none() && page.is_resource_page() {
                continue;
            }
            if let Some(kind) = &about {
                let shows = if page.is_resource_page() {
                    offered.contains(&page.id)
                } else {
                    page.answers
                        .iter()
                        .any(|a| a.about.as_deref() == Some(kind))
                };
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
                .filter(|x| !crate::pages::spec::ViewContext::supplies(&x.name))
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

fn renderers_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "renderers",
        schema::<RenderersInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: RenderersInput = parse(input)?;
                let svc = services.get()?;
                let ctx = context(&svc, &invocation, p.stream_id.clone(), None).await?;
                let renderers =
                    crate::pages::renderers::renderers_for(&svc, &p.uri, &ctx, invocation.actor())
                        .await;
                Ok(HandlerOutput {
                    result: json!({ "uri": p.uri, "renderers": renderers }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

fn find_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "find",
        schema::<FindPageInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
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
                let ctx = context(&svc, &invocation, None, None).await?;
                Ok(HandlerOutput {
                    result: find_pages(&svc, &p, &ctx, invocation.actor()).await?,
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
) -> Result<crate::pages::spec::ViewContext, CommandError> {
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
    Ok(crate::pages::spec::view_context(
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowPageInput {
    /// A page to show: `<extension>/<slug>` (see `oxplow.page.find`).
    #[serde(default)]
    pub page: Option<String>,
    /// Its param values by name (as for `oxplow.page.run`).
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// Or a page of your own as YAML (`title`, `loaders:` with sql / rows /
    /// page loaders, `view:`; it only reads): kept as a page file on
    /// `oxplow:temp` (`temp/<id>`) the person can save to the wiki or as a
    /// page.
    #[serde(default)]
    pub spec: Option<String>,
}

/// The id of the view `yaml` is, on `oxplow:temp`: a hash of its text, so
/// showing the same YAML again is the same page (and the same file).
fn temp_view_id(yaml: &str) -> String {
    format!("v{:016x}", xxhash_rust::xxh3::xxh3_64(yaml.as_bytes()))
}

/// A param value as a uri carries it.
fn uri_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn show_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.show",
        "show",
        schema::<ShowPageInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: ShowPageInput = parse(input)?;
                let svc = services.get()?;
                let ctx = context(&svc, &invocation, None, None).await?;
                let (uri, target) = match (&p.page, &p.spec) {
                    (Some(page), None) => {
                        let (ext, slug) = page.split_once('/').ok_or_else(|| {
                            invalid("/page", format!("`{page}` isn't `<extension>/<slug>`"))
                        })?;
                        let core = svc
                            .extension_catalog
                            .named(&svc.layout.project_dir, ext)
                            .is_ok_and(|e| {
                                e.namespace == crate::extension::extensions::CORE_NAMESPACE
                            });
                        let named = if core { slug.to_string() } else { page.clone() };
                        let values: Vec<(String, String)> = p
                            .params
                            .iter()
                            .flatten()
                            .map(|(k, v)| (k.clone(), uri_value(v)))
                            .collect();
                        let pairs: Vec<(&str, &str)> = values
                            .iter()
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        (
                            oxplow_domain::uri::build::page_uri(&named, &pairs),
                            RunTarget::Page(page.clone()),
                        )
                    }
                    (None, Some(yaml)) => {
                        // A view is a page file on `oxplow:temp`, named by
                        // its content: the same YAML is the same page.
                        let id = temp_view_id(yaml);
                        parse_temp_page(&id, yaml).map_err(|m| invalid("/spec", m))?;
                        svc.temp
                            .put(&temp_file_name(&id), yaml.as_bytes())
                            .await
                            .map_err(|e| failed(DomainError::Storage(e.to_string())))?;
                        let page = format!("{TEMP_EXTENSION}/{id}");
                        (
                            oxplow_domain::uri::build::page_uri(&page, &[]),
                            RunTarget::Page(page),
                        )
                    }
                    _ => {
                        return Err(invalid(
                            "/page",
                            "give `page` or `spec`, one of them".to_string(),
                        ));
                    }
                };
                let run = PageRunner::of(&svc)
                    .run(RunRequest {
                        target,
                        params: cells(p.params.clone()),
                        ctx: ctx.clone(),
                        viewer: invocation.actor().clone(),
                    })
                    .await
                    .map_err(|e| page_error(p.page.as_deref().unwrap_or("inline"), e))?;
                let text = crate::pages::text::page_text(&svc, &svc.layout.project_dir, &run)
                    .await
                    .map_err(failed)?;
                // Beside the conversation: a tab of the caller's thread in
                // its window, if one is open.
                let opened = svc
                    .client_host
                    .call(
                        invocation.actor(),
                        "tabs.write",
                        "open",
                        json!({ "uri": uri }),
                    )
                    .await;
                let mut result = json!({ "uri": uri, "text": text, "opened": opened.is_ok() });
                if let Err(e) = opened {
                    result["why"] = json!(e.to_string());
                }
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        show_op(services.clone()),
        run_op(services.clone()),
        find_op(services.clone()),
        renderers_op(services.clone()),
        describe_op(services.clone()),
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
                        let mut text =
                            crate::pages::text::page_text(&svc, &svc.layout.project_dir, &run)
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
    /// The node it sits in (none at the root).
    parent: Option<String>,
    kind: &'static str,
    /// A viz node's viz (`table`, `detail`, …).
    viz: Option<String>,
    /// What another kind is set to: a switch's param, an embed's page, a
    /// slot's name, a component's.
    detail: Option<String>,
    /// A viz's columns, `key` or `key (Label)`.
    columns: Vec<String>,
    /// The rest of a viz's settings as its YAML gives them: its role
    /// block (`tree`, `terminal`, `markdown`, …), `empty`, row styling.
    settings: Option<Value>,
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

/// What a loader is and what it runs: its SQL, its command, the page
/// loader it takes (`<page>#<loader>`), or nothing for inline rows and a
/// resource page's `content`.
fn loader_source(loader: &crate::pages::schema::Loader) -> (&'static str, Option<String>) {
    use crate::pages::schema::Loader;
    match loader {
        Loader::Sql { sql, .. } => ("sql", Some(sql.clone())),
        Loader::Command { command, .. } => ("command", Some(command.clone())),
        Loader::Page { page, loader, .. } => ("page", Some(format!("{page}#{loader}"))),
        Loader::Rows { .. } => ("rows", None),
        Loader::Content { .. } => ("content", None),
    }
}

/// The parts of a page run its definition lists, flat: its params with
/// the values this run used, its loaders with what they read and the
/// rows and columns they gave, and its actions where they sit (`page`, or the viz
/// node's id).
fn definition_parts(run: &PageRun) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let page = &run.page;
    let params = page
        .params
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "label": p.label,
                "role": p.role,
                "type": p.kind,
                "default": p.default,
                "value": run.params.get(&p.name),
            })
        })
        .collect();
    let loaders = page
        .loaders
        .iter()
        .map(|d| {
            let (kind, source) = loader_source(&d.loader);
            let ran = run.loaders.get(&d.name);
            json!({
                "name": d.name,
                "kind": kind,
                "source": source,
                "reads": d.reads,
                "uses": d.reads_loaders,
                "rows": ran.map(|r| r.result.rows.len()),
                "columns": ran.map(|r| &r.result.columns),
                "error": ran.and_then(|r| r.error.clone()),
            })
        })
        .collect();
    let on_nodes = page.nodes().into_iter().filter_map(|n| match &n.kind {
        NodeKind::Viz { config, .. } => Some((n.id.as_str(), &config.actions)),
        _ => None,
    });
    let actions = std::iter::once(("page", &page.actions))
        .chain(on_nodes)
        .flat_map(|(on, actions)| {
            actions.iter().map(move |a| {
                json!({
                    "id": a.id,
                    "label": a.label,
                    "command": a.command,
                    "open": a.open,
                    "on": on,
                    "row": a.row,
                    "when": a.when,
                    "form": a.form,
                    "prominent": a.prominent,
                })
            })
        })
        .collect();
    (params, loaders, actions)
}

/// A viz's settings beyond what its row already says (its viz, columns,
/// actions, where it comes from): what its YAML set, unset ones left out.
fn viz_settings(config: &crate::pages::spec::VizConfig) -> Option<Value> {
    const SAID: [&str; 10] = [
        "id",
        "extension",
        "slug",
        "title",
        "viz",
        "params",
        "columns",
        "actions",
        "origin",
        "fills",
    ];
    let Ok(Value::Object(all)) = serde_json::to_value(config) else {
        return None;
    };
    let set: serde_json::Map<String, Value> = all
        .into_iter()
        .filter(|(k, v)| {
            !SAID.contains(&k.as_str())
                && !matches!(v, Value::Null | Value::Bool(false))
                && v.as_array().is_none_or(|a| !a.is_empty())
                && v.as_object().is_none_or(|o| !o.is_empty())
        })
        .collect();
    (!set.is_empty()).then_some(Value::Object(set))
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
    parent: Option<&str>,
    depth: usize,
    out: &mut Vec<DefinitionRow>,
) {
    let Some(def) = crate::pages::text::node(&run.page, &at.id) else {
        // A page another extension mounted at a slot.
        if let Some(e) = &at.embed {
            let id = format!("{prefix}{}", at.id);
            definition_rows(e, &e.tree, &format!("{id}/"), parent, depth + 1, out);
        }
        return;
    };
    let (from, viz) = match &def.kind {
        NodeKind::Viz { from, config } => (
            from.clone(),
            serde_json::to_value(config.viz)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string)),
        ),
        _ => (None, None),
    };
    let detail = match &def.kind {
        NodeKind::Switch { param, .. } => Some(format!("on `{param}`")),
        NodeKind::Embed { page, .. } => Some(format!("`{page}`")),
        NodeKind::Slot { name } => Some(format!("`{}.{name}`", run.page.id)),
        NodeKind::Component { component, .. } => Some(format!("`{component}`")),
        _ => None,
    };
    let (columns, settings) = match &def.kind {
        NodeKind::Viz { config, .. } => (
            config
                .columns
                .iter()
                .map(|c| match &c.label {
                    Some(label) if label != &c.key => format!("{} ({label})", c.key),
                    _ => c.key.clone(),
                })
                .collect(),
            viz_settings(config),
        ),
        _ => (Vec::new(), None),
    };
    let (loader_kind, source) = match from.as_deref().and_then(|f| run.page.loader(f)) {
        Some(d) => {
            let (kind, source) = loader_source(&d.loader);
            (Some(kind), source)
        }
        None => (None, None),
    };
    let id = format!("{prefix}{}", at.id);
    out.push(DefinitionRow {
        depth,
        id: id.clone(),
        parent: parent.map(str::to_string),
        kind: kind_name(&def.kind),
        viz,
        detail,
        columns,
        settings,
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
        definition_rows(run, c, prefix, Some(&id), depth + 1, out);
    }
    if let Some(e) = &at.embed {
        definition_rows(e, &e.tree, &format!("{id}/"), Some(&id), depth + 1, out);
    }
}

/// The page a file is, to describe.
enum Described {
    /// A page the catalog runs, by id.
    Running(String),
    /// A page parsed from the file: a draft, an older deployment's copy
    /// or a view.
    File(Box<crate::pages::schema::Page>),
}

/// The largest page file read to describe (a page is a few KiB).
const DESCRIBE_MAX_BYTES: usize = 1 << 20;

/// `<slug>` of a page file name (`<slug>.yaml` or `.yml`).
fn page_slug(file: &str) -> Option<&str> {
    file.strip_suffix(".yaml")
        .or_else(|| file.strip_suffix(".yml"))
        .filter(|slug| !slug.is_empty())
}

/// `(extension, kind, slug)` of a page file path on the worktree:
/// `[.oxplow/]extensions/<extension>/<pages|parts|queries>/<slug>.yaml`.
fn worktree_page(path: &str) -> Option<(&str, PageKind, &str)> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [.., "extensions", ext, folder, file] => {
            Some((ext, PageKind::of_folder(folder)?, page_slug(file)?))
        }
        _ => None,
    }
}

/// The text of the file `uri` names: `given` when the caller has it,
/// else read from its filesystem. `None` when the file isn't there,
/// isn't text or is too big to be a page.
async fn file_text(
    svc: &Services,
    given: Option<&str>,
    filesystem: Option<&str>,
    rev: Option<&str>,
    path: &str,
    stream: Option<StreamId>,
) -> Option<String> {
    if let Some(text) = given {
        return (text.len() <= DESCRIBE_MAX_BYTES).then(|| text.to_string());
    }
    let fs = svc.filesystems.resolve(filesystem).ok()?;
    let bytes = fs
        .read(&oxplow_domain::filesystem::FsCx { stream }, rev, path, None)
        .await
        .ok()??;
    if bytes.len() > DESCRIBE_MAX_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The page the file `uri` is, or `None` when it is no page: not a file
/// uri, a file that isn't there, YAML that is no page or doesn't parse.
async fn described_page(
    svc: &Services,
    p: &DescribePageInput,
    stream: Option<StreamId>,
) -> Result<Option<Described>, CommandError> {
    use oxplow_domain::filesystem::{EXTENSIONS, LOCAL, TEMP};
    let uri = oxplow_domain::uri::OxplowUri::parse(p.uri.trim())
        .map_err(|e| invalid("/uri", format!("`{}` isn't an oxplow uri: {e}", p.uri)))?;
    if uri.resource().to_string() != oxplow_domain::uri::resources::FILE {
        return Ok(None);
    }
    let path = uri.param("path").unwrap_or_default();
    let rev = uri.param("rev");
    let filesystem = uri.param("filesystem").unwrap_or(LOCAL);
    let project = &svc.layout.project_dir;
    // The caller's text is of a worktree file; the other filesystems are read.
    let text = |filesystem| {
        let given = p.text.as_deref().filter(|_| filesystem == LOCAL);
        file_text(svc, given, Some(filesystem), rev, path, stream)
    };
    Ok(match filesystem {
        // The running copy, or the copy of an older (or other) deployment.
        EXTENSIONS => {
            let parts: Vec<&str> = path.split('/').collect();
            let [ext, folder, file] = parts.as_slice() else {
                return Ok(None);
            };
            let (Some(kind), Some(slug)) = (PageKind::of_folder(folder), page_slug(file)) else {
                return Ok(None);
            };
            let id = format!("{ext}/{slug}");
            let running = svc.extension_catalog.find_page(project, &id).ok();
            if let Some(page) = &running {
                if rev.is_none() || page.origin.deployment.as_deref() == rev {
                    return Ok(Some(Described::Running(page.id.clone())));
                }
            }
            let Some(text) = text(EXTENSIONS).await else {
                return Ok(None);
            };
            let namespace = namespace_of(svc, ext);
            let hash = rev.unwrap_or_default();
            let rel = crate::extension::extensions::shown(
                project,
                &crate::extension::extension_store::deployment_dir(project, ext, hash)
                    .join(folder)
                    .join(file),
            );
            let Ok(mut page) =
                crate::pages::schema::parse_page(ext, &namespace, slug, &rel, &text, kind)
            else {
                return Ok(None);
            };
            page.origin.deployment = rev.map(str::to_string);
            // An older deployment of a shipped extension is still shipped.
            page.origin.bundled = crate::extension::extensions::origin_of(ext) == "bundled";
            crate::extension::extensions::qualify_page_refs(&mut page);
            Some(Described::File(Box::new(page)))
        }
        // A view's file: a page that only reads.
        TEMP => {
            let Some(slug) = page_slug(path) else {
                return Ok(None);
            };
            let Some(text) = text(TEMP).await else {
                return Ok(None);
            };
            parse_temp_page(slug, &text)
                .ok()
                .map(|page| Described::File(Box::new(page)))
        }
        // A page file in a worktree, as the caller has it. — a draft, never the shipped copy
        // (`bundled` stays false even for a shipped extension's page).
        LOCAL => {
            let Some((ext, kind, slug)) = worktree_page(path) else {
                return Ok(None);
            };
            let Some(text) = text(LOCAL).await else {
                return Ok(None);
            };
            let namespace = namespace_of(svc, ext);
            let Ok(mut page) =
                crate::pages::schema::parse_page(ext, &namespace, slug, path, &text, kind)
            else {
                return Ok(None);
            };
            crate::extension::extensions::qualify_page_refs(&mut page);
            Some(Described::File(Box::new(page)))
        }
        _ => None,
    })
}

/// The namespace extension `name`'s commands are under: the catalog's
/// when it is here, else what a manifest without `namespace:` gets.
fn namespace_of(svc: &Services, name: &str) -> String {
    svc.extension_catalog
        .named(&svc.layout.project_dir, name)
        .map(|e| e.namespace)
        .unwrap_or_else(|_| name.replace('-', "_"))
}

fn describe_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.read",
        "describe",
        schema::<DescribePageInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: DescribePageInput = parse(input)?;
                let svc = services.get()?;
                let ctx = context(&svc, &invocation, p.stream_id.clone(), None).await?;
                // A file that is no page describes nothing: no rows.
                let Some(described) =
                    described_page(&svc, &p, ctx.stream_id.map(StreamId::new)).await?
                else {
                    return Ok(HandlerOutput {
                        result: Value::Null,
                        ..HandlerOutput::default()
                    });
                };
                let (target, label) = match described {
                    Described::Running(id) => (RunTarget::Page(id.clone()), id),
                    Described::File(page) => {
                        let id = page.id.clone();
                        (RunTarget::Spec(Arc::new(*page)), id)
                    }
                };
                let run = PageRunner::of(&svc)
                    .run(RunRequest {
                        target,
                        params: cells(p.params),
                        ctx,
                        viewer: invocation.actor().clone(),
                    })
                    .await
                    .map_err(|e| page_error(&label, e))?;
                let mut rows = Vec::new();
                definition_rows(&run, &run.tree, "", None, 0, &mut rows);
                let (params, loaders, actions) = definition_parts(&run);
                let page = &run.page;
                Ok(HandlerOutput {
                    result: json!({
                        "page": page.id,
                        "kind": page.kind,
                        "title": page.title,
                        "extension": page.extension,
                        "source": page.origin.source,
                        "running": page.origin.running,
                        "deployment": page.origin.deployment,
                        "bundled": page.origin.bundled,
                        "file": page.origin.source.clone().unwrap_or_else(|| page.origin.running.clone()),
                        "description": page.description,
                        "label": page.label,
                        "for": page.renders,
                        "default": page.default,
                        "pinned": page.pinned,
                        "hidden": page.hidden,
                        "category": page.category,
                        "tool": page.tool,
                        "refresh": page.refresh,
                        "comments": page.comments,
                        "needs": page.needs,
                        "params": params,
                        "loaders": loaders,
                        "actions": actions,
                        "answers": page.answers,
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

    /// The running copy of page `ext/slug`, as a file uri without a revision.
    fn running(page: &str) -> String {
        let (ext, slug) = page.split_once('/').unwrap();
        oxplow_domain::uri::build::file_uri_at(
            &format!("{ext}/pages/{slug}.yaml"),
            Some(oxplow_domain::filesystem::EXTENSIONS),
            None,
            None,
        )
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
            json!({ "spec": "title: Mine\nloaders:\n  a: { sql: SELECT 5 AS five }\nview: { viz: number, from: a }\n", "format": "json" }),
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
        let look = pages
            .iter()
            .find(|p| p["page"] == json!("acme/commit-look"))
            .unwrap_or_else(|| panic!("{about}"));
        assert_eq!(look["bind"], json!({ "uri": uri }));
        // Only pages that show a commit.
        assert!(
            pages.iter().all(|p| p["page"] != json!("acme/numbers")),
            "{about}"
        );
        let none = run(&svc, "oxplow.page.find", json!({ "question": "zebras" })).await;
        assert_eq!(none["pages"], json!([]), "{none}");
    }

    /// A project with extension `rank`: the pages the find ranking tests read.
    async fn ranked(
        pages: &[(&str, String)],
    ) -> (tempfile::TempDir, std::sync::Arc<crate::Services>) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        crate::pages::test_support::write(
            dir.path(),
            ".oxplow/extensions/rank/extension.yaml",
            "manifest: 2\nname: rank\nintent:\n  purpose: test\n",
        );
        for (slug, text) in pages {
            crate::pages::test_support::write(
                dir.path(),
                &format!(".oxplow/extensions/rank/pages/{slug}.yaml"),
                text,
            );
        }
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let problems = crate::test_fixtures::activate_with_problems(&svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        (dir, svc)
    }

    /// The fixture's pages found, in order (the bundled pages are left out).
    fn ids(found: &serde_json::Value) -> Vec<String> {
        found["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["page"].as_str().unwrap().to_string())
            .filter(|id| id.starts_with("rank/"))
            .collect()
    }

    const BODY: &str =
        "loaders:\n  a: { sql: \"SELECT 1 AS n\" }\nview: { viz: number, from: a }\n";

    /// Answer x3, title x2, description and keywords x1.
    #[tokio::test]
    async fn find_ranks_a_matching_answer_over_title_over_description() {
        let (_dir, svc) = ranked(&[
            // Title only. Sorts first by title if scores tie.
            ("by-title", format!("title: Failures\n{BODY}")),
            // An answer, under a title that never matches.
            (
                "by-answer",
                format!(
                    "title: Zulu\nanswers:\n  - {{ question: \"What failures happened?\" }}\n{BODY}"
                ),
            ),
            (
                "by-description",
                format!("title: Alpha\ndescription: Lists failures.\n{BODY}"),
            ),
            // Ties with the description; the title order breaks it.
            (
                "by-keywords",
                format!("title: Beta\nkeywords: failures\n{BODY}"),
            ),
        ])
        .await;
        let found = run(&svc, "oxplow.page.find", json!({ "question": "failures" })).await;
        assert_eq!(
            ids(&found),
            [
                "rank/by-answer",
                "rank/by-title",
                "rank/by-description",
                "rank/by-keywords"
            ],
            "{found}"
        );
    }

    #[tokio::test]
    async fn find_about_keeps_the_pages_for_that_resource_and_binds_its_uri() {
        let (_dir, svc) = ranked(&[
            (
                "shows",
                format!("title: Shows\nfor: oxplow/commit\nparams: [{{ name: uri }}]\n{BODY}"),
            ),
            (
                "answers-about",
                format!(
                    "title: Answers\nparams: [{{ name: uri }}]\nanswers:\n  - {{ question: \"Who wrote it?\", about: oxplow/commit }}\n{BODY}"
                ),
            ),
            (
                "no-uri-param",
                format!(
                    "title: Plain\nanswers:\n  - {{ question: \"Which is plain?\", about: oxplow/commit }}\n{BODY}"
                ),
            ),
            (
                "elsewhere",
                format!(
                    "title: Elsewhere\nfor: oxplow/branch\nparams: [{{ name: uri }}]\n{BODY}"
                ),
            ),
            ("unrelated", format!("title: Unrelated\n{BODY}")),
        ])
        .await;
        let uri = "oxplow:oxplow/commit?sha=abc1234";
        let found = run(&svc, "oxplow.page.find", json!({ "about": uri })).await;
        assert_eq!(
            ids(&found),
            ["rank/answers-about", "rank/no-uri-param", "rank/shows"],
            "{found}"
        );
        for p in found["pages"].as_array().unwrap() {
            if !p["page"].as_str().unwrap().starts_with("rank/") {
                continue;
            }
            let want = if p["page"] == json!("rank/no-uri-param") {
                json!(null)
            } else {
                json!({ "uri": uri })
            };
            assert_eq!(p["bind"], want, "{found}");
        }
        // With a question, the filter and the ranking both apply.
        let both = run(
            &svc,
            "oxplow.page.find",
            json!({ "about": uri, "question": "who wrote" }),
        )
        .await;
        assert_eq!(ids(&both), ["rank/answers-about"], "{both}");
        assert_eq!(both["pages"].as_array().unwrap().len(), 1, "{both}");
    }

    /// A resource page is a display mode of its resource: `find` lists it
    /// only for the resource asked about, never for a question alone.
    #[tokio::test]
    async fn find_without_about_never_lists_a_resource_page() {
        let (_dir, svc) = ranked(&[
            (
                "mode",
                format!(
                    "title: Gadget mode\nfor: oxplow/commit\nparams: [{{ name: uri }}]\n{BODY}"
                ),
            ),
            ("standalone", format!("title: Gadget list\n{BODY}")),
        ])
        .await;
        let found = run(&svc, "oxplow.page.find", json!({ "question": "gadget" })).await;
        assert_eq!(ids(&found), ["rank/standalone"], "{found}");
        let about = run(
            &svc,
            "oxplow.page.find",
            json!({ "about": "oxplow:oxplow/commit?sha=abc1234" }),
        )
        .await;
        assert_eq!(ids(&about), ["rank/mode"], "{about}");
    }

    /// `about` asks the one place that decides which pages draw a uri
    /// (`pages::renderers`): a page whose `accepts` don't hold for the
    /// uri's params is not found.
    #[tokio::test]
    async fn find_about_keeps_only_the_resource_pages_that_accept_the_uri() {
        let (_dir, svc) = ranked(&[
            (
                "numeric",
                format!(
                    "title: Numeric\nfor: oxplow/commit\nparams: [{{ name: uri }}]\naccepts: {{ sha: '^[0-9]+$' }}\n{BODY}"
                ),
            ),
            (
                "any",
                format!("title: Any\nfor: oxplow/commit\nparams: [{{ name: uri }}]\n{BODY}"),
            ),
        ])
        .await;
        let numeric = run(
            &svc,
            "oxplow.page.find",
            json!({ "about": "oxplow:oxplow/commit?sha=1234" }),
        )
        .await;
        assert_eq!(ids(&numeric), ["rank/any", "rank/numeric"], "{numeric}");
        let hex = run(
            &svc,
            "oxplow.page.find",
            json!({ "about": "oxplow:oxplow/commit?sha=abc1234" }),
        )
        .await;
        assert_eq!(ids(&hex), ["rank/any"], "{hex}");
    }

    /// The bus command a page reads offers the same list as the window's.
    #[tokio::test]
    async fn renderers_lists_the_pages_that_can_draw_a_uri() {
        let (_dir, svc) = ranked(&[
            (
                "numeric",
                format!(
                    "title: Numeric\nfor: oxplow/commit\nparams: [{{ name: uri }}]\naccepts: {{ sha: '^[0-9]+$' }}\n{BODY}"
                ),
            ),
            (
                "any",
                format!("title: Any\nfor: oxplow/commit\nparams: [{{ name: uri }}]\n{BODY}"),
            ),
        ])
        .await;
        let out = run(
            &svc,
            "oxplow.page.renderers",
            json!({ "uri": "oxplow:oxplow/commit?sha=1234#oxplow.renderer=rank/any" }),
        )
        .await;
        let pages: Vec<&str> = out["renderers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["page"].as_str())
            .filter(|p| p.starts_with("rank/"))
            .collect();
        assert_eq!(pages, ["rank/any", "rank/numeric"], "{out}");
        assert!(out["renderers"][0]["title"].is_string(), "{out}");
    }

    #[tokio::test]
    async fn find_returns_at_most_limit_pages_ten_by_default() {
        let pages: Vec<(String, String)> = (0..12)
            .map(|i| (format!("w{i:02}"), format!("title: Widget {i:02}\n{BODY}")))
            .collect();
        let refs: Vec<(&str, String)> =
            pages.iter().map(|(s, t)| (s.as_str(), t.clone())).collect();
        let (_dir, svc) = ranked(&refs).await;
        let all = run(&svc, "oxplow.page.find", json!({ "question": "widget" })).await;
        assert_eq!(ids(&all).len(), 10, "{all}");
        let three = run(
            &svc,
            "oxplow.page.find",
            json!({ "question": "widget", "limit": 3 }),
        )
        .await;
        assert_eq!(ids(&three), ["rank/w00", "rank/w01", "rank/w02"], "{three}");
    }

    #[tokio::test]
    async fn a_pages_definition_names_each_node_and_where_it_comes_from() {
        let (_dir, svc) = fixture().await;
        let def = run(
            &svc,
            "oxplow.page.describe",
            json!({ "uri": running("acme/host") }),
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
        // Each node names the node it sits in; the root none.
        assert_eq!(nodes[0]["parent"], json!(null));
        assert!(nodes[1..].iter().all(|n| n["parent"].is_string()), "{def}");
        // Params as a page's param carries them: JSON text.
        let def = run(
            &svc,
            "oxplow.page.describe",
            json!({ "uri": running("acme/numbers"), "params": "{\"n\": 3}" }),
        )
        .await;
        assert_eq!(def["page"], json!("acme/numbers"));
    }

    /// Every part of the page, flat, as the definition page lists it: its
    /// params with this run's values, its loaders with what they read
    /// and how many rows they gave, and its actions wherever they sit.
    #[tokio::test]
    async fn a_pages_definition_lists_its_params_loaders_and_actions() {
        let (_dir, svc) = fixture().await;
        let def = run(
            &svc,
            "oxplow.page.describe",
            json!({ "uri": running("acme/acts") }),
        )
        .await;
        let params = def["params"].as_array().unwrap();
        assert_eq!(params[0]["name"], json!("t"), "{def}");
        assert_eq!(params[0]["value"], json!("from-page"), "{def}");
        let loaders = def["loaders"].as_array().unwrap();
        assert_eq!(loaders[0]["name"], json!("names"), "{def}");
        assert_eq!(loaders[0]["kind"], json!("sql"), "{def}");
        assert_eq!(loaders[0]["rows"], json!(2), "{def}");
        // The columns its rows have, which a viz's settings name.
        assert_eq!(loaders[0]["columns"], json!(["name"]), "{def}");
        let actions = def["actions"].as_array().unwrap();
        let on: Vec<_> = actions
            .iter()
            .map(|a| {
                (
                    a["id"].as_str().unwrap(),
                    a["on"].as_str().unwrap(),
                    a["row"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            on,
            [("add", "page", false), ("add-row", "names", true)],
            "{def}"
        );
        assert_eq!(actions[0]["command"], json!("oxplow.work_item.create"));
        // A loader that reads another names it.
        let def = run(
            &svc,
            "oxplow.page.describe",
            json!({ "uri": running("acme/bound") }),
        )
        .await;
        let echo = def["loaders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["name"] == json!("echo"))
            .unwrap_or_else(|| panic!("{def}"));
        assert_eq!(echo["uses"], json!(["row"]), "{def}");
        assert_eq!(echo["rows"], json!(1), "{def}");
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
            "title: Numbers Draft\nloaders:\n  big: { sql: \"SELECT 99 AS n\" }\nview: { viz: table, from: big }\n",
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

#[cfg(test)]
mod show_tests {
    use oxplow_domain::Actor;
    use serde_json::json;

    use crate::pages::test_support::fixture;

    const VIEW: &str =
        "title: Mine\nloaders:\n  a: { sql: SELECT 5 AS five }\nview: { viz: number, from: a }\n";

    /// An agent shows an inline page: it is written as a page file on
    /// `oxplow:temp` and shown as the page `temp/<id>`; the same YAML shown
    /// again is the same page; a page by id is shown at its own uri. With no
    /// window to open it in, it says so and is still there to open.
    #[tokio::test]
    async fn an_agent_shows_an_inline_page_as_a_temp_page() {
        let (_dir, svc) = fixture().await;
        let show = |spec: &'static str| {
            let svc = svc.clone();
            async move {
                svc.commands
                    .run(
                        &Actor::Human,
                        "oxplow.page.show",
                        json!({ "spec": spec }),
                        false,
                    )
                    .await
                    .unwrap()
                    .result
            }
        };
        let out = show(VIEW).await;
        let uri = out["uri"].as_str().unwrap().to_string();
        assert!(uri.starts_with("oxplow:oxplow/page?page=temp/"), "{out}");
        assert_eq!(out["opened"], json!(false), "{out}");
        assert!(out["text"].as_str().unwrap().contains("# Mine"), "{out}");
        let id = uri.rsplit('/').next().unwrap();
        let on_disk = std::fs::read_to_string(
            svc.layout
                .project_dir
                .join(format!(".oxplow/runtime/temp/{id}.yaml")),
        )
        .expect("the view is a file on oxplow:temp");
        assert_eq!(on_disk, VIEW);
        let page = svc
            .extension_catalog
            .find_page(&svc.layout.project_dir, &format!("temp/{id}"))
            .unwrap();
        assert_eq!(page.title, "Mine");
        assert_eq!(show(VIEW).await["uri"], json!(uri), "same YAML, same page");
        assert_ne!(
            show("title: Other\nview: { markdown: hi }\n").await["uri"],
            json!(uri)
        );
        let page = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.page.show",
                json!({ "page": "acme/numbers", "params": { "n": 3 } }),
                false,
            )
            .await
            .unwrap()
            .result;
        assert_eq!(
            page["uri"],
            json!("oxplow:oxplow/page?n=3&page=acme/numbers"),
            "{page}"
        );
        let bad = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.page.show",
                json!({ "spec": "title: Bad\nview: { slot: s }\n" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(bad, oxplow_domain::CommandError::Invalid { .. }),
            "{bad:?}"
        );
    }
}

#[cfg(test)]
mod describe_tests {
    use oxplow_domain::filesystem::{EXTENSIONS, LOCAL, TEMP};
    use oxplow_domain::uri::build::file_uri_at;
    use oxplow_domain::Actor;
    use serde_json::{json, Value};

    use crate::test_fixtures::{services_with_effort, EffortFixture};

    const MANIFEST: &str = "manifest: 2\nname: acme\nintent:\n  purpose: test\n";
    const PAGE: &str = "# authored with its own comment\ntitle: 'Release'\nloaders:\n  numbers: { sql: 'SELECT 7 AS n' }\nview:\n  id: layout\n  stack:\n    - { id: total, title: Total, viz: number, from: numbers }\n    - { id: note, title: Note, markdown: 'A preserved note.' }\n";
    const OLD: &str = "title: 'Old release'\nloaders:\n  numbers: { sql: 'SELECT 1 AS n' }\nview: { id: only, viz: number, from: numbers }\n";

    /// A project whose extension `acme` is deployed with page `pr`.
    async fn deployed() -> (EffortFixture, String) {
        let fx = services_with_effort().await;
        let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(dir.join("extension.yaml"), MANIFEST).unwrap();
        std::fs::write(dir.join("pages/pr.yaml"), PAGE).unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let hash = fx
            .svc
            .extension_catalog
            .find_page(&fx.svc.layout.project_dir, "acme/pr")
            .unwrap()
            .origin
            .deployment
            .unwrap();
        (fx, hash)
    }

    async fn describe(fx: &EffortFixture, input: Value) -> Value {
        fx.svc
            .commands
            .run(&Actor::Human, "oxplow.page.describe", input.clone(), false)
            .await
            .unwrap_or_else(|e| panic!("{input}: {e:?}"))
            .result
    }

    fn uri(filesystem: &str, path: &str, rev: Option<&str>) -> String {
        file_uri_at(path, Some(filesystem), rev, None)
    }

    /// A page file on `oxplow:extensions` is the page that runs: with no
    /// revision, or with the running deployment's.
    #[tokio::test]
    async fn the_running_copy_on_the_extensions_filesystem_is_the_catalogs_page() {
        let (fx, hash) = deployed().await;
        for rev in [None, Some(hash.as_str())] {
            let def = describe(
                &fx,
                json!({ "uri": uri(EXTENSIONS, "acme/pages/pr.yaml", rev) }),
            )
            .await;
            assert_eq!(def["page"], json!("acme/pr"), "{def}");
            assert_eq!(def["title"], json!("Release"), "{def}");
            assert_eq!(def["deployment"], json!(hash), "{def}");
            let ids: Vec<_> = def["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["id"].clone())
                .collect();
            assert_eq!(
                ids,
                [json!("layout"), json!("total"), json!("note")],
                "{def}"
            );
        }
    }

    /// A revision that isn't the running deployment's is that deployment's
    /// file, parsed on its own.
    #[tokio::test]
    async fn another_revision_is_that_deployments_copy() {
        let (fx, hash) = deployed().await;
        let old = crate::extension::extension_store::deployment_dir(
            &fx.svc.layout.project_dir,
            "acme",
            "old1",
        );
        std::fs::create_dir_all(old.join("pages")).unwrap();
        std::fs::write(old.join("pages/pr.yaml"), OLD).unwrap();
        let def = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "acme/pages/pr.yaml", Some("old1")) }),
        )
        .await;
        assert_eq!(def["title"], json!("Old release"), "{def}");
        assert_eq!(def["deployment"], json!("old1"), "{def}");
        assert!(def["running"].as_str().unwrap().contains("old1"), "{def}");
        assert_eq!(def["nodes"][0]["id"], json!("only"), "{def}");
        // The running one is untouched.
        let now = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "acme/pages/pr.yaml", Some(&hash)) }),
        )
        .await;
        assert_eq!(now["title"], json!("Release"), "{now}");
        // A revision nothing was deployed at: no rows.
        let none = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "acme/pages/pr.yaml", Some("nope9")) }),
        )
        .await;
        assert_eq!(none, Value::Null);
    }

    /// Whether a page ships with oxplow follows its extension, whichever
    /// copy of it is read: the running one, an older deployment's, or a
    /// project's own. A temp view or a worktree draft is not the shipped
    /// copy.
    #[tokio::test]
    async fn a_page_of_a_shipped_extension_says_it_ships_with_oxplow() {
        let (fx, _hash) = deployed().await;
        let project = &fx.svc.layout.project_dir;
        let flag = |def: &Value| def["bundled"].as_bool();
        let running = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "oxplow-foundation/pages/commit.yaml", None) }),
        )
        .await;
        assert_eq!(flag(&running), Some(true), "{running}");
        // An older deployment's copy of a shipped page is still one.
        let old =
            crate::extension::extension_store::deployment_dir(project, "oxplow-foundation", "old1");
        std::fs::create_dir_all(old.join("pages")).unwrap();
        std::fs::write(old.join("pages/commit.yaml"), OLD).unwrap();
        let older = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "oxplow-foundation/pages/commit.yaml", Some("old1")) }),
        )
        .await;
        assert_eq!(older["deployment"], json!("old1"), "{older}");
        assert_eq!(flag(&older), Some(true), "{older}");
        // A project's own extension is not.
        let own = describe(
            &fx,
            json!({ "uri": uri(EXTENSIONS, "acme/pages/pr.yaml", None) }),
        )
        .await;
        assert_eq!(flag(&own), Some(false), "{own}");
        // A temp view is not.
        fx.svc.temp.put("v2.yaml", PAGE.as_bytes()).await.unwrap();
        let temp = describe(&fx, json!({ "uri": uri(TEMP, "v2.yaml", None) })).await;
        assert_eq!(flag(&temp), Some(false), "{temp}");
    }

    #[tokio::test]
    async fn a_temp_file_is_a_view_that_only_reads() {
        let fx = services_with_effort().await;
        fx.svc.temp.put("v1.yaml", PAGE.as_bytes()).await.unwrap();
        let def = describe(&fx, json!({ "uri": uri(TEMP, "v1.yaml", None) })).await;
        assert_eq!(def["page"], json!("temp/v1"), "{def}");
        assert_eq!(def["file"], json!(".oxplow/runtime/temp/v1.yaml"), "{def}");
        let nodes = def["nodes"].as_array().unwrap();
        let total = nodes.iter().find(|n| n["id"] == "total").unwrap();
        assert_eq!(total["parent"], json!("layout"), "{total}");
        assert_eq!(total["loader_kind"], json!("sql"), "{total}");
        assert_eq!(total["source"], json!("SELECT 7 AS n"), "{total}");
        // A view that acts is no view.
        let acting = "title: Acting\nactions:\n  - { id: create, label: Create, command: oxplow.work_item.create, input: { title: Hidden } }\nview: { markdown: Preview }\n";
        fx.svc.temp.put("v2.yaml", acting.as_bytes()).await.unwrap();
        assert_eq!(
            describe(&fx, json!({ "uri": uri(TEMP, "v2.yaml", None) })).await,
            Value::Null
        );
        // A display mode is no view.
        let mode = "title: Mode\nfor: oxplow/file\nparams: [{ name: uri }]\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: table, from: q }\n";
        fx.svc.temp.put("v3.yaml", mode.as_bytes()).await.unwrap();
        assert_eq!(
            describe(&fx, json!({ "uri": uri(TEMP, "v3.yaml", None) })).await,
            Value::Null
        );
        // No such file.
        assert_eq!(
            describe(&fx, json!({ "uri": uri(TEMP, "gone.yaml", None) })).await,
            Value::Null
        );
    }

    /// A node says what it is set to, as its YAML does.
    #[tokio::test]
    async fn a_nodes_settings_are_what_its_yaml_set() {
        const SPEC: &str = "title: Set\nparams: [{ name: area, role: filter, default: one }]\nloaders:\n  items: { sql: \"SELECT 'a' AS id, NULL AS parent\" }\nview:\n  switch:\n    param: area\n    children:\n      - { id: one, title: One, viz: tree, from: items, tree: { id: id, parent: parent, label: id }, columns: [{ key: id, label: Item }], empty: None yet. }\n      - { id: two, title: Two, markdown: 'Hi.' }\n";
        let fx = services_with_effort().await;
        fx.svc.temp.put("set.yaml", SPEC.as_bytes()).await.unwrap();
        let out = describe(&fx, json!({ "uri": uri(TEMP, "set.yaml", None) })).await;
        let nodes = out["nodes"].as_array().unwrap();
        assert_eq!(nodes[0]["detail"], json!("on `area`"), "{out}");
        let one = nodes.iter().find(|n| n["id"] == "one").unwrap();
        assert_eq!(one["columns"], json!(["id (Item)"]), "{one}");
        assert_eq!(one["settings"]["tree"]["label"], json!("id"), "{one}");
        assert_eq!(one["settings"]["empty"], json!("None yet."), "{one}");
        assert!(one["settings"].get("columns").is_none(), "{one}");
    }

    /// A page file in a worktree is described from the text the caller
    /// passes, its extension and slug from its path.
    #[tokio::test]
    async fn a_worktree_file_is_described_from_the_text_given() {
        let (fx, _) = deployed().await;
        for path in [
            "extensions/acme/pages/draft.yaml",
            ".oxplow/extensions/acme/pages/draft.yaml",
        ] {
            let def = describe(&fx, json!({ "uri": uri(LOCAL, path, None), "text": OLD })).await;
            assert_eq!(def["page"], json!("acme/draft"), "{def}");
            assert_eq!(def["title"], json!("Old release"), "{def}");
            assert_eq!(def["extension"], json!("acme"), "{def}");
            assert_eq!(def["file"], json!(path), "{def}");
        }
        // A file under `parts/` or `queries/` is that kind, and the
        // description says so.
        let part = describe(
            &fx,
            json!({ "uri": uri(LOCAL, "extensions/acme/parts/draft.yaml", None), "text": OLD }),
        )
        .await;
        assert_eq!(part["kind"], json!("part"), "{part}");
        assert_eq!(
            part["file"],
            json!("extensions/acme/parts/draft.yaml"),
            "{part}"
        );
        let query = describe(
            &fx,
            json!({
                "uri": uri(LOCAL, "extensions/acme/queries/draft.yaml", None),
                "text": "title: Q\nloaders:\n  numbers: { sql: 'SELECT 1 AS n' }\n",
            }),
        )
        .await;
        assert_eq!(query["kind"], json!("query"), "{query}");
        // Without text it is read from the worktree.
        std::fs::write(
            fx.svc
                .layout
                .project_dir
                .join(".oxplow/extensions/acme/pages/pr.yaml"),
            OLD,
        )
        .unwrap();
        let read = describe(
            &fx,
            json!({ "uri": uri(LOCAL, ".oxplow/extensions/acme/pages/pr.yaml", None) }),
        )
        .await;
        assert_eq!(read["title"], json!("Old release"), "{read}");
        // A file that isn't under an extension's pages is no page.
        assert_eq!(
            describe(
                &fx,
                json!({ "uri": uri(LOCAL, "docs/notes.yaml", None), "text": OLD })
            )
            .await,
            Value::Null
        );
    }

    /// Definition's `accepts.content` relies on this: a file that is no
    /// page, or no YAML, is zero rows, never an error.
    #[tokio::test]
    async fn a_file_that_is_no_page_describes_nothing() {
        let fx = services_with_effort().await;
        let local = |text: &str| json!({ "uri": uri(LOCAL, "extensions/acme/pages/x.yaml", None), "text": text });
        for text in [
            "name: ci\non: [push]\njobs: {}\n",
            "title: [unclosed\n  - : :\n",
            "",
            "just words",
        ] {
            assert_eq!(describe(&fx, local(text)).await, Value::Null, "{text:?}");
        }
        // Not a file uri, not a page path, an unknown filesystem.
        for u in [
            "oxplow:oxplow/commit?sha=abc1234".to_string(),
            uri(EXTENSIONS, "acme/readme.yaml", None),
            uri(EXTENSIONS, "acme/pages/missing.yaml", None),
            uri("acme:s3", "acme/pages/x.yaml", None),
        ] {
            assert_eq!(
                describe(&fx, json!({ "uri": u, "text": OLD })).await,
                Value::Null,
                "{u}"
            );
        }
        // A uri that isn't one is refused.
        let refused = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.page.describe",
                json!({ "uri": "not a uri" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(refused, oxplow_domain::CommandError::Invalid { .. }),
            "{refused:?}"
        );
    }

    /// The tool no longer takes the old inputs: `spec` and the old name are gone.
    #[tokio::test]
    async fn the_old_definition_command_and_its_spec_input_are_gone() {
        let fx = services_with_effort().await;
        assert!(fx.svc.commands.spec("oxplow.page.definition").is_none());
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.page.describe",
                json!({ "uri": uri(TEMP, "a.yaml", None), "spec": PAGE }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, oxplow_domain::CommandError::Invalid { .. }),
            "{err:?}"
        );
    }

    /// A page's provenance names the copy that runs, the same URI `v_page` gives.
    #[tokio::test]
    async fn provenance_carries_the_running_uri() {
        let (fx, hash) = deployed().await;
        let page = fx
            .svc
            .extension_catalog
            .find_page(&fx.svc.layout.project_dir, "acme/pr")
            .unwrap();
        assert_eq!(
            page.origin.running_uri,
            Some(uri(EXTENSIONS, "acme/pages/pr.yaml", Some(&hash)))
        );
    }
}
