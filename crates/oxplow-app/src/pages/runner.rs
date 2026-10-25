//! Running a page: its params resolved, its loaders run in order — a
//! query (earlier loaders readable as tables), a read command run as the
//! viewer, another page's loader, inline rows — and its tree resolved: a
//! switch's child in view, an embed's page run with params by name.
//! Within one run (embeds included) a loader with the same definition and
//! inputs runs once.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use oxplow_db::{SqlCell, SqlQueryResult, TempTable};
use oxplow_domain::{Actor, DomainError};
use serde::{Deserialize, Serialize};

use super::schema::{ArgSource, Loader, LoaderDecl, Node, NodeKind, Page};
use crate::extensions::{
    bind_params, evaluate_alert, resolve_param_values, stream_head, AlertState, LensContext,
};

/// How deep embeds and page loaders may nest before a run stops: a cycle
/// the load checks missed, or a page that embeds too much.
const MAX_DEPTH: usize = 8;

/// What to run.
#[derive(Debug, Clone)]
pub enum RunTarget {
    /// A deployed page, by `<extension>/<slug>`.
    Page(String),
    /// A page given whole (an inline page).
    Spec(Arc<Page>),
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub target: RunTarget,
    /// The values the caller gives: its URI's `?` and `#` params.
    pub params: BTreeMap<String, SqlCell>,
    /// The stream and thread it is viewed from.
    pub ctx: LensContext,
    /// Who views it: a command loader runs as them.
    pub viewer: Actor,
}

/// A run of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageRun {
    pub page: Page,
    /// The values its params took.
    pub params: BTreeMap<String, SqlCell>,
    /// Each loader that ran, by name.
    pub loaders: BTreeMap<String, LoaderRun>,
    pub tree: RunNode,
    pub rail: Option<RunNode>,
    pub alert: Option<AlertState>,
    /// The badge's value, when the page has one.
    pub badge: Option<SqlCell>,
}

/// A loader's result in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LoaderRun {
    pub result: SqlQueryResult,
    /// Why it has no rows when it failed: the rest of the page still shows.
    pub error: Option<String>,
    /// Its definition and inputs, hashed: two loaders with the same key
    /// are the same rows.
    pub key: String,
}

/// A node of a run's tree: the page's node by id, and what it resolved to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunNode {
    pub id: String,
    /// The children shown: a switch's one in view, a stack's or grid's
    /// all.
    pub children: Vec<RunNode>,
    /// An embed's run.
    pub embed: Option<Box<PageRun>>,
    /// Why it shows nothing (an embed that didn't run).
    pub error: Option<String>,
}

/// Runs pages over one oxplow's data.
pub struct PageRunner<'a> {
    pub sql: &'a crate::sql_gateway::SqlGateway,
    pub catalog: &'a crate::extension_catalog::ExtensionCatalog,
    pub commands: &'a crate::commands::CommandBus,
    /// The checkout whose catalog pages are found in.
    pub root: &'a Path,
}

type Memo = HashMap<String, LoaderRun>;

impl<'a> PageRunner<'a> {
    /// The runner over `svc`'s data, finding pages in its project.
    pub fn of(svc: &'a crate::Services) -> Self {
        PageRunner {
            sql: &svc.sql,
            catalog: &svc.extension_catalog,
            commands: svc.commands.as_ref(),
            root: &svc.layout.project_dir,
        }
    }

    pub async fn run(&self, req: RunRequest) -> Result<PageRun, DomainError> {
        let page = match req.target {
            RunTarget::Page(id) => self.find(&id)?,
            RunTarget::Spec(page) => page.as_ref().clone(),
        };
        let mut memo = Memo::new();
        self.run_page(page, req.params, &req.ctx, &req.viewer, &mut memo, 0)
            .await
    }

    fn find(&self, id: &str) -> Result<Page, DomainError> {
        self.catalog
            .find_page(self.root, id)
            .map_err(|_| DomainError::Invalid(format!("no page `{id}`")))
    }

    fn run_page<'r>(
        &'r self,
        page: Page,
        supplied: BTreeMap<String, SqlCell>,
        ctx: &'r LensContext,
        viewer: &'r Actor,
        memo: &'r mut Memo,
        depth: usize,
    ) -> BoxFuture<'r, Result<PageRun, DomainError>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err(DomainError::Invalid(format!(
                    "page `{}` nests more than {MAX_DEPTH} pages deep",
                    page.id
                )));
            }
            let params = self.params_of(&page, &supplied, ctx).await?;
            let skipped = hidden_loaders(&page, &params);
            let mut loaders: BTreeMap<String, LoaderRun> = BTreeMap::new();
            for decl in &page.loaders {
                if skipped.contains(&decl.name) {
                    continue;
                }
                let run = self
                    .run_loader(&page, decl, &params, &loaders, ctx, viewer, memo, depth)
                    .await;
                loaders.insert(decl.name.clone(), run);
            }
            let tree = self
                .resolve(
                    &page, &page.view, &params, &loaders, ctx, viewer, memo, depth,
                )
                .await;
            let rail = match &page.rail {
                Some(r) => Some(
                    self.resolve(&page, r, &params, &loaders, ctx, viewer, memo, depth)
                        .await,
                ),
                None => None,
            };
            let badge = page.badge.as_ref().and_then(|b| {
                let result = &loaders.get(&b.loader)?.result;
                let index = match &b.column {
                    Some(c) => result.columns.iter().position(|x| x == c)?,
                    None => 0,
                };
                result.rows.first().and_then(|r| r.get(index)).cloned()
            });
            let alert = page.alert.as_ref().and_then(|a| {
                let watched = page
                    .badge
                    .as_ref()
                    .map(|b| b.loader.as_str())
                    .or_else(|| page.loaders.first().map(|l| l.name.as_str()))?;
                Some(evaluate_alert(a, &loaders.get(watched)?.result))
            });
            Ok(PageRun {
                page,
                params,
                loaders,
                tree,
                rail,
                alert,
                badge,
            })
        })
    }

    /// `page`'s params for this run, with `head_sha` (when declared and
    /// not given) the viewed stream's HEAD commit.
    async fn params_of(
        &self,
        page: &Page,
        supplied: &BTreeMap<String, SqlCell>,
        ctx: &LensContext,
    ) -> Result<BTreeMap<String, SqlCell>, DomainError> {
        let mut params =
            resolve_param_values(&format!("page {}", page.id), &page.params, supplied, ctx)?;
        let stream = stream_of(&params, ctx);
        if let (Some(SqlCell::Null(())), Some(stream_id)) = (params.get("head_sha"), stream) {
            if let Some(head) = stream_head(self.sql, stream_id).await? {
                params.insert("head_sha".into(), SqlCell::Text(head));
            }
        }
        Ok(params)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_loader(
        &self,
        page: &Page,
        decl: &LoaderDecl,
        params: &BTreeMap<String, SqlCell>,
        earlier: &BTreeMap<String, LoaderRun>,
        ctx: &LensContext,
        viewer: &Actor,
        memo: &mut Memo,
        depth: usize,
    ) -> LoaderRun {
        let stream = stream_of(params, ctx);
        let inputs: Vec<&str> = decl
            .reads_loaders
            .iter()
            .filter_map(|n| earlier.get(n).map(|r| r.key.as_str()))
            .collect();
        let bound: BTreeMap<&String, &SqlCell> = params
            .iter()
            .filter(|(k, _)| {
                decl.reads_params.contains(k) || matches!(decl.loader, Loader::Page { .. })
            })
            .collect();
        let key = key_of(&decl.loader, &bound, stream, &inputs);
        if let Some(hit) = memo.get(&key) {
            return hit.clone();
        }
        let outcome = match &decl.loader {
            Loader::Sql { sql } => {
                let temp = decl
                    .reads_loaders
                    .iter()
                    .filter_map(|n| {
                        let r = &earlier.get(n)?.result;
                        Some(TempTable {
                            name: n.clone(),
                            columns: r.columns.clone(),
                            rows: r.rows.clone(),
                        })
                    })
                    .collect::<Vec<_>>();
                let named = bind_params(&page.params, params, time::OffsetDateTime::now_utc());
                let mut query = oxplow_db::SqlQuery::new(sql)
                    .named(named)
                    .limit(None)
                    .stream(stream);
                query.temp = temp;
                self.sql.run(query).await.map_err(|e| e.to_string())
            }
            Loader::Command { command, args } => {
                self.run_command(page, decl, command, args, params, earlier, viewer)
                    .await
            }
            Loader::Page {
                page: target,
                loader,
                params: given,
            } => {
                let given = bind_args(given, params, earlier);
                match self.find(target) {
                    Err(e) => Err(e.to_string()),
                    Ok(other) if other.loader(loader).is_none() => {
                        Err(format!("page `{target}` has no loader `{loader}`"))
                    }
                    Ok(other) => {
                        let params = given
                            .into_iter()
                            .map(|(k, v)| (k, SqlCell::from(v)))
                            .collect();
                        match self
                            .run_page(other, params, ctx, viewer, memo, depth + 1)
                            .await
                        {
                            Ok(run) => run
                                .loaders
                                .get(loader)
                                .map(|r| match &r.error {
                                    Some(e) => Err(e.clone()),
                                    None => Ok(r.result.clone()),
                                })
                                .unwrap_or_else(|| {
                                    Err(format!("page `{target}`'s loader `{loader}` didn't run"))
                                }),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
            Loader::Rows { columns, rows } => Ok(table(columns.clone(), rows.clone())),
        };
        let run = match outcome {
            Ok(result) => LoaderRun {
                result,
                error: None,
                key: key.clone(),
            },
            Err(error) => LoaderRun {
                result: table(Vec::new(), Vec::new()),
                error: Some(format!("loader `{}`: {error}", decl.name)),
                key: key.clone(),
            },
        };
        memo.insert(key, run.clone());
        run
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_command(
        &self,
        page: &Page,
        decl: &LoaderDecl,
        command: &str,
        args: &BTreeMap<String, ArgSource>,
        params: &BTreeMap<String, SqlCell>,
        earlier: &BTreeMap<String, LoaderRun>,
        viewer: &Actor,
    ) -> Result<SqlQueryResult, String> {
        let spec = self
            .commands
            .spec(command)
            .ok_or_else(|| format!("no command `{command}`"))?;
        if !spec.access.reads_only() {
            return Err(format!(
                "`{command}` isn't a read: a page's loader only reads"
            ));
        }
        let input =
            serde_json::Value::Object(bind_args(args, params, earlier).into_iter().collect());
        // The page reads for whoever views it.
        let actor = Actor::Lens {
            lens_id: format!("{}#{}", page.id, decl.name),
            on_behalf_of: Box::new(viewer.clone()),
        };
        let outcome = self
            .commands
            .run(&actor, command, input, false)
            .await
            .map_err(|e| e.to_string())?;
        let mut result = rows_of(outcome.result);
        result.reads.models = decl.reads.clone();
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve<'r>(
        &'r self,
        page: &'r Page,
        node: &'r Node,
        params: &'r BTreeMap<String, SqlCell>,
        loaders: &'r BTreeMap<String, LoaderRun>,
        ctx: &'r LensContext,
        viewer: &'r Actor,
        memo: &'r mut Memo,
        depth: usize,
    ) -> BoxFuture<'r, RunNode> {
        Box::pin(async move {
            let mut out = RunNode {
                id: node.id.clone(),
                children: Vec::new(),
                embed: None,
                error: None,
            };
            match &node.kind {
                NodeKind::Stack { children } | NodeKind::Grid { children } => {
                    for c in children {
                        let r = self
                            .resolve(page, c, params, loaders, ctx, viewer, memo, depth)
                            .await;
                        out.children.push(r);
                    }
                }
                NodeKind::Switch { param, children } => {
                    if let Some(c) = in_view(param, children, params) {
                        let r = self
                            .resolve(page, c, params, loaders, ctx, viewer, memo, depth)
                            .await;
                        out.children.push(r);
                    }
                }
                NodeKind::Embed {
                    page: target,
                    params: given,
                } => match self.find(target) {
                    Err(e) => out.error = Some(e.to_string()),
                    Ok(child) => {
                        let flowed = flow_params(&child, given, params, loaders, &page.loaders);
                        match self
                            .run_page(child, flowed, ctx, viewer, memo, depth + 1)
                            .await
                        {
                            Ok(run) => out.embed = Some(Box::new(run)),
                            Err(e) => out.error = Some(e.to_string()),
                        }
                    }
                },
                NodeKind::Viz { .. }
                | NodeKind::Slot { .. }
                | NodeKind::Markdown { .. }
                | NodeKind::Component { .. } => {}
            }
            out
        })
    }
}

/// The stream a run reads metrics over: its `stream_id` param, else the
/// viewer's.
fn stream_of(params: &BTreeMap<String, SqlCell>, ctx: &LensContext) -> Option<i64> {
    match params.get("stream_id") {
        Some(SqlCell::Int(id)) => Some(*id),
        _ => ctx.stream_id,
    }
}

fn table(columns: Vec<String>, rows: Vec<Vec<SqlCell>>) -> SqlQueryResult {
    SqlQueryResult {
        columns,
        rows,
        truncated: false,
        reads: oxplow_db::Reads::default(),
        freshness: Vec::new(),
        generation: None,
        coverage: Vec::new(),
    }
}

/// A loader's definition and inputs as one key.
fn key_of(
    loader: &Loader,
    bound: &BTreeMap<&String, &SqlCell>,
    stream: Option<i64>,
    inputs: &[&str],
) -> String {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(serde_json::to_string(loader).unwrap_or_default().as_bytes());
    h.update(serde_json::to_string(bound).unwrap_or_default().as_bytes());
    h.update(format!("{stream:?}").as_bytes());
    for i in inputs {
        h.update(i.as_bytes());
    }
    format!("{:016x}", h.digest())
}

/// `args` with each source taken: a param's value, an earlier loader's
/// first-row column, or the literal.
fn bind_args(
    args: &BTreeMap<String, ArgSource>,
    params: &BTreeMap<String, SqlCell>,
    earlier: &BTreeMap<String, LoaderRun>,
) -> BTreeMap<String, serde_json::Value> {
    args.iter()
        .map(|(name, source)| {
            let value = match source {
                ArgSource::Literal(v) => v.clone(),
                ArgSource::Param { param } => params
                    .get(param)
                    .map(cell_json)
                    .unwrap_or(serde_json::Value::Null),
                ArgSource::Column { from } => {
                    let (loader, column) = from.split_once('.').unwrap_or((from, ""));
                    earlier
                        .get(loader)
                        .and_then(|r| first_row_cell(&r.result, column))
                        .map(|c| cell_json(&c))
                        .unwrap_or(serde_json::Value::Null)
                }
            };
            (name.clone(), value)
        })
        .collect()
}

fn first_row_cell(result: &SqlQueryResult, column: &str) -> Option<SqlCell> {
    let i = result.columns.iter().position(|c| c == column)?;
    result.rows.first()?.get(i).cloned()
}

fn cell_json(cell: &SqlCell) -> serde_json::Value {
    serde_json::to_value(cell).unwrap_or(serde_json::Value::Null)
}

/// A command's result as rows: a `{ columns, rows }` table as it is, an
/// array of objects a row each (columns in first-seen order), one object
/// one row, anything else one `value` cell.
pub fn rows_of(value: serde_json::Value) -> SqlQueryResult {
    use serde_json::Value;
    if let Value::Object(map) = &value {
        if let (Some(Value::Array(columns)), Some(Value::Array(rows))) =
            (map.get("columns"), map.get("rows"))
        {
            let columns: Vec<String> = columns
                .iter()
                .map(|c| c.as_str().unwrap_or_default().to_string())
                .collect();
            let rows = rows
                .iter()
                .map(|r| {
                    r.as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| serde_json::from_value(c.clone()).unwrap_or(SqlCell::Null(())))
                        .collect()
                })
                .collect();
            return table(columns, rows);
        }
    }
    let objects: Vec<serde_json::Map<String, Value>> = match value {
        Value::Array(items) if items.iter().all(Value::is_object) => items
            .into_iter()
            .filter_map(|i| match i {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect(),
        Value::Object(m) => vec![m],
        Value::Null => Vec::new(),
        other => {
            return table(vec!["value".into()], vec![vec![SqlCell::from(other)]]);
        }
    };
    let mut columns: Vec<String> = Vec::new();
    for o in &objects {
        for k in o.keys() {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
    }
    let rows = objects
        .into_iter()
        .map(|mut o| {
            columns
                .iter()
                .map(|c| o.remove(c).map(SqlCell::from).unwrap_or(SqlCell::Null(())))
                .collect()
        })
        .collect();
    table(columns, rows)
}

/// The child of a switch on `param` in view: the one whose id is its
/// value, else the first.
fn in_view<'n>(
    param: &str,
    children: &'n [Node],
    params: &BTreeMap<String, SqlCell>,
) -> Option<&'n Node> {
    let value = match params.get(param) {
        Some(SqlCell::Text(t)) => Some(t.as_str()),
        _ => None,
    };
    value
        .and_then(|v| children.iter().find(|c| c.id == v))
        .or_else(|| children.first())
}

/// The loaders this run needn't run: the ones only nodes under a switch's
/// hidden children show. A loader no node shows still runs (it feeds an
/// embed by name, the badge or a later loader).
fn hidden_loaders(page: &Page, params: &BTreeMap<String, SqlCell>) -> Vec<String> {
    fn shown(node: &Node, out: &mut Vec<String>) {
        if let NodeKind::Viz { from: Some(f), .. } = &node.kind {
            out.push(f.clone());
        }
        for c in node.children() {
            shown(c, out);
        }
    }
    fn visible(node: &Node, params: &BTreeMap<String, SqlCell>, out: &mut Vec<String>) {
        match &node.kind {
            NodeKind::Viz { from: Some(f), .. } => out.push(f.clone()),
            NodeKind::Switch { param, children } => {
                if let Some(c) = in_view(param, children, params) {
                    visible(c, params, out);
                }
            }
            _ => {
                for c in node.children() {
                    visible(c, params, out);
                }
            }
        }
    }
    let mut all = Vec::new();
    let mut seen = Vec::new();
    for root in std::iter::once(&page.view).chain(page.rail.as_ref()) {
        shown(root, &mut all);
        visible(root, params, &mut seen);
    }
    // What a needed loader reads is needed too.
    let mut needed: Vec<String> = page
        .loaders
        .iter()
        .filter(|l| !all.contains(&l.name) || seen.contains(&l.name))
        .map(|l| l.name.clone())
        .chain(page.badge.iter().map(|b| b.loader.clone()))
        .collect();
    for l in page.loaders.iter().rev() {
        if needed.contains(&l.name) {
            needed.extend(l.reads_loaders.iter().cloned());
        }
    }
    page.loaders
        .iter()
        .map(|l| l.name.clone())
        .filter(|n| !needed.contains(n))
        .collect()
}

/// An embedded page's params, by name: what the embed names explicitly,
/// else the host's param of the same name (when it holds a value), else a
/// column of the same name in the first row of the host's loaders, in
/// declaration order.
fn flow_params(
    child: &Page,
    given: &BTreeMap<String, ArgSource>,
    params: &BTreeMap<String, SqlCell>,
    loaders: &BTreeMap<String, LoaderRun>,
    order: &[LoaderDecl],
) -> BTreeMap<String, SqlCell> {
    let explicit = bind_args(given, params, loaders);
    let mut out = BTreeMap::new();
    for p in &child.params {
        if let Some(v) = explicit.get(&p.name) {
            out.insert(p.name.clone(), SqlCell::from(v.clone()));
            continue;
        }
        if let Some(v) = params
            .get(&p.name)
            .filter(|v| !matches!(v, SqlCell::Null(())))
        {
            out.insert(p.name.clone(), v.clone());
            continue;
        }
        let from_row = order
            .iter()
            .filter_map(|d| loaders.get(&d.name))
            .find_map(|r| first_row_cell(&r.result, &p.name));
        if let Some(v) = from_row {
            out.insert(p.name.clone(), v);
        }
    }
    out
}

/// The loaders of `page` that a change makes run again, in declaration
/// order: those that read a changed model or bind a changed param, and
/// every loader that reads one of those.
pub fn dirty_loaders(
    page: &Page,
    run: &PageRun,
    models: &[String],
    params: &[String],
) -> Vec<String> {
    let mut dirty: Vec<String> = Vec::new();
    for decl in &page.loaders {
        let Some(r) = run.loaders.get(&decl.name) else {
            continue;
        };
        let reads_model = r
            .result
            .reads
            .models
            .iter()
            .chain(decl.reads.iter())
            .any(|m| models.contains(m));
        let binds_param = decl.reads_params.iter().any(|p| params.contains(p));
        let downstream = decl.reads_loaders.iter().any(|l| dirty.contains(l));
        if reads_model || binds_param || downstream {
            dirty.push(decl.name.clone());
        }
    }
    dirty
}
