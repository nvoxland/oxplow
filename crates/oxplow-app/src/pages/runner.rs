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

#[path = "result_budget.rs"]
mod result_budget;
use result_budget::ResultBudget;

/// Combined serialized loader-result budget for one returned run, including
/// embeds and slot mounts. Page definitions, params and tree metadata are
/// outside this result budget.
pub(super) const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;
use crate::extension::extensions::{stream_head, PageSlot, SlotAt, SlotEdge};
use crate::pages::spec::{
    bind_params, evaluate_alert, resolve_param_values, AlertState, ViewContext,
};

/// The most bytes of a loader's rows the daemon reads to decide whether a
/// page accepts a URI (`accepts: { content: … }`).
pub(super) const ACCEPT_RESULT_BYTES: usize = 1024 * 1024;

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
    pub ctx: ViewContext,
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
    /// Set when the page needs a capability that isn't active: it ran no
    /// loaders, and says what it needs instead of showing nothing.
    #[serde(default)]
    pub inactive: Option<PageInactive>,
    /// The page's "See also" links, in order, a uri once.
    #[serde(default)]
    pub see_also: Vec<SeeAlso>,
}

/// A related page the run links to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SeeAlso {
    /// What the link says.
    pub title: String,
    /// What it opens.
    pub uri: String,
    /// The hover, when there is one.
    pub description: Option<String>,
}

/// A page that didn't run for want of a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageInactive {
    /// The needs that aren't met (`work_items`, `knowledge`).
    pub needs: Vec<String>,
    /// What a person reads.
    pub message: String,
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
    /// A `replaceable:` node another extension's page stands in for: that
    /// extension, its run the `embed`.
    pub replaced_by: Option<String>,
}

/// Runs pages over one oxplow's data.
pub struct PageRunner<'a> {
    /// The services a `content` loader reads files and resources through.
    pub svc: &'a crate::Services,
    pub sql: crate::sql_gateway::SqlGateway,
    pub catalog: &'a crate::extension::extension_catalog::ExtensionCatalog,
    pub commands: &'a crate::commands::CommandBus,
    /// The checkout whose catalog pages are found in.
    pub root: &'a Path,
    /// A draft's own pages, found before the catalog's.
    pub draft: Option<Draft>,
    /// What the person chose to stand in for each replaceable target
    /// (`replacements`).
    pub replacements: BTreeMap<String, String>,
    /// Which extension brings a provider, for a target bound to one.
    pub capabilities: Option<std::sync::Arc<crate::capabilities::CapabilityRegistry>>,
}

/// An extension's draft: its source in a checkout other than the one that
/// deploys, read over live data. Its queries read its datasets through an
/// overlay; its commands are whatever is deployed (a draft's code never
/// acts).
#[derive(Debug, Clone)]
pub struct Draft {
    pub extension: String,
    pub checkout: std::path::PathBuf,
    pub pages: Vec<Page>,
}

type Memo = HashMap<String, LoaderRun>;

/// Data shared by recursive page execution: memoized work and admission
/// into the retained run's result budget. Borrowed-page loaders share the
/// memo while supplying their own transient budget.
struct RunData<'a> {
    memo: &'a mut Memo,
    budget: &'a mut ResultBudget,
}

impl<'a> PageRunner<'a> {
    /// The runner over `svc`'s data, finding pages in its project.
    pub fn of(svc: &'a crate::Services) -> Self {
        PageRunner {
            svc,
            sql: svc.sql.clone(),
            catalog: &svc.extension_catalog,
            commands: svc.commands.as_ref(),
            root: &svc.layout.project_dir,
            draft: None,
            replacements: svc
                .config
                .read()
                .map(|c| c.replacements.clone())
                .unwrap_or_default(),
            capabilities: Some(svc.capabilities.clone()),
        }
    }

    /// The runner over `svc`'s data with `extension`'s draft in `checkout`:
    /// its pages and its datasets as that checkout has them.
    pub async fn draft(
        svc: &'a crate::Services,
        checkout: &Path,
        extension: &str,
    ) -> Result<Self, DomainError> {
        let settings = oxplow_config::extension_settings(&svc.layout.project_dir);
        let source = settings.source(checkout, extension);
        let loaded = {
            let (source, name) = (source.clone(), extension.to_string());
            tokio::task::spawn_blocking(move || {
                crate::extension::extensions::load_extension_at(&source, &name)
            })
            .await
            .map_err(|e| DomainError::Storage(e.to_string()))?
        };
        if !source.join("extension.yaml").is_file() {
            return Err(DomainError::Invalid(format!(
                "no draft of `{extension}` in {}",
                checkout.display()
            )));
        }
        let pages = loaded.page_files.clone();
        let overlay = crate::extension::extensions::draft_overlay(
            &svc.sql,
            &svc.extension_catalog,
            checkout,
            loaded,
        )
        .await;
        let mut runner = Self::of(svc);
        runner.sql = svc.sql.with_overlay(overlay);
        runner.draft = Some(Draft {
            extension: extension.to_string(),
            checkout: checkout.to_path_buf(),
            pages,
        });
        Ok(runner)
    }

    pub async fn run(&self, req: RunRequest) -> Result<PageRun, DomainError> {
        self.run_seeded(req, Memo::new()).await
    }

    /// [`PageRunner::run`], reusing `seed`'s loader runs by key: what a
    /// re-run keeps of the loaders a change didn't reach.
    pub async fn run_seeded(
        &self,
        req: RunRequest,
        seed: HashMap<String, LoaderRun>,
    ) -> Result<PageRun, DomainError> {
        let page = match req.target {
            RunTarget::Page(id) => self.find(&id)?,
            RunTarget::Spec(page) => page.as_ref().clone(),
        };
        let mut memo = seed;
        let mut budget = ResultBudget::new(MAX_RESULT_BYTES);
        self.run_page(
            page,
            req.params,
            &req.ctx,
            &req.viewer,
            RunData {
                memo: &mut memo,
                budget: &mut budget,
            },
            0,
        )
        .await
    }

    fn find(&self, id: &str) -> Result<Page, DomainError> {
        if let Some(page) = self
            .draft
            .as_ref()
            .and_then(|d| d.pages.iter().find(|p| p.id == id))
        {
            return Ok(page.clone());
        }
        self.catalog
            .find_page(self.root, id)
            .map_err(|_| DomainError::Invalid(format!("no page `{id}`")))
    }

    fn run_page<'r>(
        &'r self,
        page: Page,
        supplied: BTreeMap<String, SqlCell>,
        ctx: &'r ViewContext,
        viewer: &'r Actor,
        data: RunData<'r>,
        depth: usize,
    ) -> BoxFuture<'r, Result<PageRun, DomainError>> {
        Box::pin(async move {
            let RunData { memo, budget } = data;
            if depth > MAX_DEPTH {
                return Err(DomainError::Invalid(format!(
                    "page `{}` nests more than {MAX_DEPTH} pages deep",
                    page.id
                )));
            }
            let params = self.params_of(&page, &supplied, ctx).await?;
            // It needs a capability that isn't active: it runs nothing.
            let unmet = ctx
                .active
                .as_ref()
                .map(|active| active.unmet(&page.needs))
                .unwrap_or_default();
            if !unmet.is_empty() {
                let tree = RunNode {
                    id: page.view.id.clone(),
                    children: Vec::new(),
                    embed: None,
                    error: None,
                    replaced_by: None,
                };
                return Ok(PageRun {
                    page,
                    params,
                    loaders: BTreeMap::new(),
                    tree,
                    rail: None,
                    alert: None,
                    badge: None,
                    see_also: Vec::new(),
                    inactive: Some(PageInactive {
                        message: crate::capabilities::needs_message(&unmet),
                        needs: unmet,
                    }),
                });
            }
            let skipped = hidden_loaders(&page, &params);
            let mut loaders: BTreeMap<String, LoaderRun> = BTreeMap::new();
            for decl in &page.loaders {
                if skipped.contains(&decl.name) {
                    continue;
                }
                let mut run = self
                    .run_loader(&page, decl, &params, &loaders, ctx, viewer, memo, depth)
                    .await;
                // Downstream SQL temp tables and flowing params only see
                // admitted rows, never a result beyond this run's budget.
                if budget.admit(&mut run.result)? {
                    // A dependent memo key must distinguish this admitted
                    // prefix from the full result (and other prefixes).
                    run.key = format!(
                        "{}:budget:{}:{}",
                        run.key,
                        run.result.columns.len(),
                        run.result.rows.len()
                    );
                }
                loaders.insert(decl.name.clone(), run);
            }
            let tree = self
                .resolve(
                    &page, &page.view, &params, &loaders, ctx, viewer, memo, budget, depth,
                )
                .await;
            let rail = match &page.rail {
                Some(r) => Some(
                    self.resolve(
                        &page, r, &params, &loaders, ctx, viewer, memo, budget, depth,
                    )
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
                    .or_else(|| page.first_loader().map(|l| l.name.as_str()))?;
                Some(evaluate_alert(a, &loaders.get(watched)?.result))
            });
            let see_also = self.see_also(&page, &params, &loaders);
            Ok(PageRun {
                inactive: None,
                page,
                params,
                loaders,
                tree,
                rail,
                alert,
                badge,
                see_also,
            })
        })
    }

    /// Loader `target` of `page` and only what it reads (its chain), for
    /// a URI-acceptance check (`pages/renderers.rs`): the rows of `target`.
    /// `supplied` are the page's param values; `content` is the page's
    /// `content` loader already resolved, shared by every page asked about
    /// the same URI. Any loader's failure is the whole answer's, and each
    /// result is cut to [`ACCEPT_RESULT_BYTES`].
    pub(crate) async fn run_loader_chain(
        &self,
        page: &Page,
        target: &str,
        supplied: BTreeMap<String, SqlCell>,
        ctx: &ViewContext,
        viewer: &Actor,
        content: Option<SqlQueryResult>,
    ) -> Result<SqlQueryResult, String> {
        let params = self
            .params_of(page, &supplied, ctx)
            .await
            .map_err(|e| e.to_string())?;
        let chain = page.loader_chain(target);
        let mut memo = Memo::new();
        let mut loaders: BTreeMap<String, LoaderRun> = BTreeMap::new();
        for decl in page.loaders.iter().filter(|d| chain.contains(&d.name)) {
            let mut run = match (&decl.loader, &content) {
                (Loader::Content { .. }, Some(result)) => LoaderRun {
                    result: result.clone(),
                    error: None,
                    key: "content".to_string(),
                },
                (Loader::Content { .. }, None) => {
                    return Err("the page's content wasn't resolved".into())
                }
                _ => {
                    self.run_loader(page, decl, &params, &loaders, ctx, viewer, &mut memo, 0)
                        .await
                }
            };
            if let Some(error) = run.error {
                return Err(error);
            }
            ResultBudget::new(ACCEPT_RESULT_BYTES)
                .admit(&mut run.result)
                .map_err(|e| e.to_string())?;
            loaders.insert(decl.name.clone(), run);
        }
        loaders
            .remove(target)
            .map(|r| r.result)
            .ok_or_else(|| format!("no loader `{target}`"))
    }

    /// `page`'s params for this run, with `head_sha` (when declared and
    /// not given) the viewed stream's HEAD commit.
    async fn params_of(
        &self,
        page: &Page,
        supplied: &BTreeMap<String, SqlCell>,
        ctx: &ViewContext,
    ) -> Result<BTreeMap<String, SqlCell>, DomainError> {
        let mut params =
            resolve_param_values(&format!("page {}", page.id), &page.params, supplied, ctx)?;
        let stream = stream_of(&params, ctx);
        if let (Some(SqlCell::Null(())), Some(stream_id)) = (params.get("head_sha"), stream) {
            if let Some(head) = stream_head(&self.sql, stream_id).await? {
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
        ctx: &ViewContext,
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
        // A resource's content is keyed by the resource (its identity), not
        // by the view it is shown with.
        let identity = match (&decl.loader, params.get("uri")) {
            (Loader::Content { .. }, Some(SqlCell::Text(uri))) => Some(SqlCell::Text(
                oxplow_domain::uri::OxplowUri::parse(uri.trim())
                    .map(|u| u.identity())
                    .unwrap_or_else(|_| uri.clone()),
            )),
            _ => None,
        };
        let mut bound = bound;
        if let (Some((name, _)), Some(identity)) = (params.get_key_value("uri"), &identity) {
            bound.insert(name, identity);
        }
        let key = key_of(&decl.loader, &bound, stream, &inputs);
        if let Some(hit) = memo.get(&key) {
            return hit.clone();
        }
        let outcome = match &decl.loader {
            Loader::Sql { sql, params: given } => match temp_tables(&decl.reads_loaders, earlier) {
                Err(Unreadable::Failed(why)) => Err(why),
                // SQLite holds no table without columns, and a loader that
                // yielded none has no shape to select from: what reads it
                // has no rows.
                Err(Unreadable::NoColumns) => Ok(table(Vec::new(), Vec::new())),
                Ok(temp) => {
                    let mut named =
                        bind_params(&page.params, params, time::OffsetDateTime::now_utc());
                    for (k, v) in bind_args(given, params, earlier) {
                        named.retain(|(n, _)| n != &k);
                        named.push((k, SqlCell::from(v)));
                    }
                    // `config('/key')` reads its own extension's settings.
                    match self.with_config(page, sql).await {
                        Err(e) => Err(e.to_string()),
                        Ok(sql) => {
                            let mut query = oxplow_db::SqlQuery::new(&sql)
                                .named(named)
                                .limit(None)
                                .stream(stream);
                            query.temp = temp;
                            self.sql.run(query).await.map_err(|e| e.to_string())
                        }
                    }
                }
            },
            Loader::Command {
                command,
                args,
                items,
            } => {
                self.run_command(
                    page,
                    decl,
                    command,
                    args,
                    items.as_deref(),
                    params,
                    earlier,
                    viewer,
                )
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
                        // The source page is transient: only the selected
                        // loader is retained by this page. Bound that work
                        // independently, then charge its returned copy when
                        // this page admits it.
                        let mut transient = ResultBudget::new(MAX_RESULT_BYTES);
                        match self
                            .run_page(
                                other,
                                params,
                                ctx,
                                viewer,
                                RunData {
                                    memo,
                                    budget: &mut transient,
                                },
                                depth + 1,
                            )
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
            Loader::Parse { format, text } => {
                let text = bind_args(
                    &BTreeMap::from([("text".to_string(), text.clone())]),
                    params,
                    earlier,
                )
                .remove("text")
                .unwrap_or(serde_json::Value::Null);
                let text = match text {
                    serde_json::Value::Null => String::new(),
                    serde_json::Value::String(t) => t,
                    other => other.to_string(),
                };
                super::parse::parse_text(*format, &text)
            }
            Loader::Content { text, max_bytes } => match params.get("uri") {
                Some(SqlCell::Text(uri)) => {
                    super::content::resolve_content(
                        self.svc,
                        uri,
                        stream,
                        super::content::ContentOpts {
                            text: *text,
                            max_bytes: *max_bytes,
                        },
                    )
                    .await
                }
                // Nothing to look at: the page was opened with no URI.
                _ => Ok(table(vec!["uri".into()], Vec::new())),
            },
        };
        // Bound each memo entry too, before it is cloned into the cache.
        // The shared run budget separately charges every retained copy.
        let outcome = outcome.and_then(|mut result| {
            ResultBudget::new(MAX_RESULT_BYTES)
                .admit(&mut result)
                .map_err(|e| e.to_string())?;
            Ok(result)
        });
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

    /// `sql` with its `config('/key')` reads filled from `page`'s
    /// extension's settings, as they are stored now.
    async fn with_config(&self, page: &Page, sql: &str) -> Result<String, DomainError> {
        if oxplow_db::datasets::config_reads(sql)?.is_empty() {
            return Ok(sql.to_string());
        }
        match self.catalog.named(self.root, &page.extension) {
            Ok(ext) => self.sql.with_extension_config(&ext, sql).await,
            Err(_) => Ok(sql.to_string()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_command(
        &self,
        page: &Page,
        decl: &LoaderDecl,
        command: &str,
        args: &BTreeMap<String, ArgSource>,
        items: Option<&str>,
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
        let actor = Actor::Page {
            page_id: format!("{}#{}", page.id, decl.name),
            on_behalf_of: Box::new(viewer.clone()),
        };
        let outcome = self
            .commands
            .run(&actor, command, input, false)
            .await
            .map_err(|e| e.to_string())?;
        let value = match items {
            None => outcome.result,
            Some(field) => outcome
                .result
                .get(field)
                .cloned()
                .ok_or_else(|| format!("`{command}` returned no `{field}`"))?,
        };
        let mut result = rows_of(value);
        result.reads.datasets = decl.reads.clone();
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve<'r>(
        &'r self,
        page: &'r Page,
        node: &'r Node,
        params: &'r BTreeMap<String, SqlCell>,
        loaders: &'r BTreeMap<String, LoaderRun>,
        ctx: &'r ViewContext,
        viewer: &'r Actor,
        memo: &'r mut Memo,
        budget: &'r mut ResultBudget,
        depth: usize,
    ) -> BoxFuture<'r, RunNode> {
        Box::pin(async move {
            // Another extension's page standing in for it runs instead.
            if let Some(stand_in) = node
                .replaceable
                .as_deref()
                .and_then(|target| self.replacement(target, params))
            {
                let mut out = self
                    .run_embedded(
                        page,
                        &stand_in.page,
                        params,
                        loaders,
                        ctx,
                        viewer,
                        RunData { memo, budget },
                        depth,
                    )
                    .await;
                out.id = node.id.clone();
                out.replaced_by = Some(stand_in.extension);
                return out;
            }
            let mut out = RunNode {
                id: node.id.clone(),
                children: Vec::new(),
                embed: None,
                error: None,
                replaced_by: None,
            };
            match &node.kind {
                NodeKind::Stack { children } | NodeKind::Grid { children } => {
                    // Pages other extensions mount at a slot here go at the
                    // slot, or where their `at` says among its siblings.
                    let mut start = Vec::new();
                    let mut before: Vec<(String, RunNode)> = Vec::new();
                    let mut after: Vec<(String, RunNode)> = Vec::new();
                    let mut resolved: Vec<RunNode> = Vec::new();
                    for c in children.iter().filter(|c| !left_out(c, loaders)) {
                        let NodeKind::Slot { name } = &c.kind else {
                            let r = self
                                .resolve(page, c, params, loaders, ctx, viewer, memo, budget, depth)
                                .await;
                            resolved.push(r);
                            continue;
                        };
                        let mut slot = RunNode {
                            id: c.id.clone(),
                            children: Vec::new(),
                            embed: None,
                            error: None,
                            replaced_by: None,
                        };
                        for mount in self.mounts(&page.id, name) {
                            let r = self
                                .run_mount(
                                    page, &mount, params, loaders, ctx, viewer, memo, budget, depth,
                                )
                                .await;
                            match &mount.at {
                                Some(SlotAt::Edge(SlotEdge::Start)) => start.push(r),
                                Some(SlotAt::Before { before: id }) => before.push((id.clone(), r)),
                                Some(SlotAt::After { after: id }) => after.push((id.clone(), r)),
                                _ => slot.children.push(r),
                            }
                        }
                        resolved.push(slot);
                    }
                    out.children = start;
                    for r in resolved {
                        let id = r.id.clone();
                        out.children.extend(
                            before
                                .iter()
                                .filter(|(a, _)| *a == id)
                                .map(|(_, n)| n.clone()),
                        );
                        out.children.push(r);
                        out.children.extend(
                            after
                                .iter()
                                .filter(|(a, _)| *a == id)
                                .map(|(_, n)| n.clone()),
                        );
                    }
                }
                NodeKind::Slot { name } => {
                    for mount in self.mounts(&page.id, name) {
                        let r = self
                            .run_mount(
                                page, &mount, params, loaders, ctx, viewer, memo, budget, depth,
                            )
                            .await;
                        out.children.push(r);
                    }
                }
                NodeKind::Switch { param, children } => {
                    if let Some(c) = in_view(param, children, params) {
                        let r = self
                            .resolve(page, c, params, loaders, ctx, viewer, memo, budget, depth)
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
                        let remaining = budget.remaining();
                        match self
                            .run_page(
                                child,
                                flowed,
                                ctx,
                                viewer,
                                RunData { memo, budget },
                                depth + 1,
                            )
                            .await
                        {
                            Ok(run) => out.embed = Some(Box::new(run)),
                            Err(e) => {
                                // A failed embed retains no loader results.
                                budget.restore(remaining);
                                out.error = Some(e.to_string());
                            }
                        }
                    }
                },
                NodeKind::Viz { .. } | NodeKind::Markdown { .. } | NodeKind::Component { .. } => {}
            }
            out
        })
    }
}

impl PageRunner<'_> {
    /// `page`'s "See also" links, resolved in order with a uri once. A page
    /// that is not there, or a loader that failed, gives no link: never an
    /// error.
    fn see_also(
        &self,
        page: &Page,
        params: &BTreeMap<String, SqlCell>,
        loaders: &BTreeMap<String, LoaderRun>,
    ) -> Vec<SeeAlso> {
        use super::schema::SeeAlsoDecl;
        use crate::pages::spec::ParamRole;
        let mut out: Vec<SeeAlso> = Vec::new();
        let mut push = |link: SeeAlso| {
            if !link.title.is_empty() && !out.iter().any(|l| l.uri == link.uri) {
                out.push(link);
            }
        };
        for decl in &page.see_also {
            match decl {
                SeeAlsoDecl::Page {
                    page: target,
                    params: given,
                } => {
                    let Ok(child) = self.find(target) else {
                        continue;
                    };
                    // The page's own URI as `v_page.uri` names it: oxplow's
                    // own extensions' pages by slug alone.
                    let core = self
                        .catalog
                        .unpinned(self.root, "a see-also link names a page by its extension")
                        .iter()
                        .any(|e| {
                            e.name == child.extension
                                && e.namespace == crate::extension::extensions::CORE_NAMESPACE
                        });
                    let named = if core {
                        child.slug.clone()
                    } else {
                        child.id.clone()
                    };
                    let mut query: Vec<(String, String)> = vec![("page".into(), named)];
                    let mut view: Vec<(String, String)> = Vec::new();
                    for (name, value) in bind_args(given, params, loaders) {
                        let text = match value {
                            serde_json::Value::Null => continue,
                            serde_json::Value::String(s) if s.is_empty() => continue,
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        let in_view = child.params.iter().any(|p| {
                            p.name == name
                                && matches!(
                                    p.role,
                                    Some(
                                        ParamRole::Filter | ParamRole::Selection | ParamRole::View
                                    )
                                )
                        });
                        if in_view { &mut view } else { &mut query }.push((name, text));
                    }
                    let pairs = |v: &[(String, String)]| -> Vec<(String, String)> { v.to_vec() };
                    let (query, view) = (pairs(&query), pairs(&view));
                    let q: Vec<(&str, &str)> = query
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    let w: Vec<(&str, &str)> =
                        view.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    push(SeeAlso {
                        title: child.title.clone(),
                        uri: oxplow_domain::uri::build::of_with_view(
                            oxplow_domain::uri::resources::PAGE,
                            &q,
                            &w,
                        ),
                        description: Some(child.description.clone()).filter(|d| !d.is_empty()),
                    });
                }
                SeeAlsoDecl::Rows { from } => {
                    let Some(run) = loaders.get(from).filter(|r| r.error.is_none()) else {
                        continue;
                    };
                    let at = |name: &str| run.result.columns.iter().position(|c| c == name);
                    let (Some(title), Some(uri)) = (at("title"), at("uri")) else {
                        continue;
                    };
                    let description = at("description");
                    let text = |row: &Vec<SqlCell>, i: usize| match row.get(i) {
                        Some(SqlCell::Text(t)) if !t.is_empty() => Some(t.clone()),
                        _ => None,
                    };
                    for row in &run.result.rows {
                        if let (Some(title), Some(uri)) = (text(row, title), text(row, uri)) {
                            push(SeeAlso {
                                title,
                                uri,
                                description: description.and_then(|i| text(row, i)),
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// The pages enabled extensions mount at `host`'s slot `slot`, by
    /// extension name.
    fn mounts(&self, host: &str, slot: &str) -> Vec<PageSlot> {
        let extensions = self
            .catalog
            .unpinned(self.root, "a page's slots read one snapshot");
        let mut out: Vec<(String, PageSlot)> = extensions
            .iter()
            .filter(|e| e.enabled)
            .flat_map(|e| {
                e.ui.page_slots
                    .iter()
                    .filter(|m| m.host == host && m.slot == slot)
                    .map(|m| (e.name.clone(), m.clone()))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.into_iter().map(|(_, m)| m).collect()
    }

    /// The replacement standing in for `target` now (`replacements::
    /// chosen`): the enabled extensions' candidates, the person's choice,
    /// and — for a target bound to a provider — the extension bringing the
    /// provider of the thing shown (its `uri` param's `provider`).
    fn replacement(
        &self,
        target: &str,
        params: &BTreeMap<String, SqlCell>,
    ) -> Option<crate::extension::extensions::replacements::UiReplacement> {
        let extensions = self
            .catalog
            .unpinned(self.root, "a replaceable node reads one snapshot");
        let candidates: Vec<&crate::extension::extensions::replacements::UiReplacement> =
            extensions
                .iter()
                .filter(|e| e.enabled)
                .flat_map(|e| e.ui.replacements.iter())
                .filter(|r| r.target == target)
                .collect();
        let first = candidates.first()?;
        let provider_extension = match (first.bound_to_provider, params.get("uri")) {
            (true, Some(SqlCell::Text(uri))) => {
                let provider = oxplow_domain::uri::OxplowUri::parse(uri)
                    .ok()?
                    .param("provider")?
                    .to_string();
                self.capabilities
                    .as_ref()?
                    .get(&first.capability, &provider)?
                    .extension
            }
            _ => None,
        };
        crate::extension::extensions::replacements::chosen(
            &candidates,
            self.replacements.get(target).map(String::as_str),
            provider_extension.as_deref(),
        )
        .cloned()
    }

    /// A mounted page run as an embed (its params by name from the host),
    /// its node id the page's id with `.` for `/`.
    #[allow(clippy::too_many_arguments)]
    async fn run_mount(
        &self,
        host: &Page,
        mount: &PageSlot,
        params: &BTreeMap<String, SqlCell>,
        loaders: &BTreeMap<String, LoaderRun>,
        ctx: &ViewContext,
        viewer: &Actor,
        memo: &mut Memo,
        budget: &mut ResultBudget,
        depth: usize,
    ) -> RunNode {
        let mut out = self
            .run_embedded(
                host,
                &mount.page,
                params,
                loaders,
                ctx,
                viewer,
                RunData { memo, budget },
                depth,
            )
            .await;
        out.id = mount.page.replace('/', ".");
        out
    }

    /// Page `id` run inside `host` as an embed — a slot's mount, a
    /// replacement — its params by name from the host's params and its
    /// loaders' first rows.
    #[allow(clippy::too_many_arguments)]
    async fn run_embedded(
        &self,
        host: &Page,
        id: &str,
        params: &BTreeMap<String, SqlCell>,
        loaders: &BTreeMap<String, LoaderRun>,
        ctx: &ViewContext,
        viewer: &Actor,
        RunData { memo, budget }: RunData<'_>,
        depth: usize,
    ) -> RunNode {
        let mut out = RunNode {
            id: id.to_string(),
            children: Vec::new(),
            embed: None,
            error: None,
            replaced_by: None,
        };
        match self.find(id) {
            Err(e) => out.error = Some(e.to_string()),
            Ok(child) => {
                let flowed = flow_params(&child, &BTreeMap::new(), params, loaders, &host.loaders);
                let remaining = budget.remaining();
                match self
                    .run_page(
                        child,
                        flowed,
                        ctx,
                        viewer,
                        RunData { memo, budget },
                        depth + 1,
                    )
                    .await
                {
                    Ok(run) => out.embed = Some(Box::new(run)),
                    Err(e) => {
                        budget.restore(remaining);
                        out.error = Some(e.to_string());
                    }
                }
            }
        }
        out
    }
}

/// The stream a run reads metrics over: its `stream_id` param, else the
/// viewer's.
fn stream_of(params: &BTreeMap<String, SqlCell>, ctx: &ViewContext) -> Option<i64> {
    match params.get("stream_id") {
        Some(SqlCell::Int(id)) => Some(*id),
        _ => ctx.stream_id,
    }
}

/// Why the loaders a query reads can't be its temp tables.
pub(super) enum Unreadable {
    /// One failed: its error is the reader's (not a syntax error over the
    /// table it never made).
    Failed(String),
    /// One has no columns (it yielded nothing, with no shape).
    NoColumns,
}

/// The loaders `names` as the temp tables a query reads them from.
pub(super) fn temp_tables(
    names: &[String],
    earlier: &BTreeMap<String, LoaderRun>,
) -> Result<Vec<TempTable>, Unreadable> {
    let mut out = Vec::new();
    for n in names {
        let Some(run) = earlier.get(n) else { continue };
        if let Some(error) = &run.error {
            return Err(Unreadable::Failed(format!(
                "reads `{n}`, which failed: {error}"
            )));
        }
        if run.result.columns.is_empty() {
            return Err(Unreadable::NoColumns);
        }
        out.push(TempTable {
            name: n.clone(),
            columns: run.result.columns.clone(),
            rows: run.result.rows.clone(),
        });
    }
    Ok(out)
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
pub(super) fn bind_args(
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

/// A `hide_empty` viz node whose loader ran clean and has no rows: it is
/// left out of the run (a failed loader still shows, with its error).
fn left_out(node: &Node, loaders: &BTreeMap<String, LoaderRun>) -> bool {
    let NodeKind::Viz {
        from: Some(from),
        config,
    } = &node.kind
    else {
        return false;
    };
    config.hide_empty
        && loaders
            .get(from)
            .is_some_and(|run| run.error.is_none() && run.result.rows.is_empty())
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
    // A loader only a choice's list names (`options_from: '#x'`, a form's
    // `choices` or `from`) runs when that list is asked for, not with the
    // page: the rail's `page_options` and a form's open run it.
    let on_demand = on_demand_loaders(page, &all);
    // What a needed loader reads is needed too.
    let mut needed: Vec<String> = page
        .loaders
        .iter()
        .filter(|l| !all.contains(&l.name) || seen.contains(&l.name))
        .filter(|l| !on_demand.contains(&l.name))
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

/// The loaders of `page` that only an option, choice or sub-field list
/// of the page itself names: no node shows one, no other loader reads it,
/// and it is not the one an alert watches, a see-also's, the badge's or a
/// column an embed takes.
fn on_demand_loaders(page: &Page, shown: &[String]) -> Vec<String> {
    use super::refs::{refs, split, RefSite};
    use super::schema::SeeAlsoDecl;
    fn embed_columns(node: &Node, out: &mut Vec<String>) {
        if let NodeKind::Embed { params, .. } = &node.kind {
            for arg in params.values() {
                if let ArgSource::Column { from } = arg {
                    out.push(from.split('.').next().unwrap_or(from).to_string());
                }
            }
        }
        for c in node.children() {
            embed_columns(c, out);
        }
    }
    let mut named: Vec<String> = Vec::new();
    for (site, target) in refs(page) {
        let list = matches!(
            site,
            RefSite::OptionsFrom { .. } | RefSite::FormChoices { .. } | RefSite::FormFrom { .. }
        );
        if let (true, (id, Some(loader))) = (list, split(&target)) {
            if id == page.id {
                named.push(loader.to_string());
            }
        }
    }
    let mut pinned: Vec<String> = Vec::new();
    // The alert watches the badge's loader, else the first.
    if page.alert.is_some() {
        pinned.extend(page.first_loader().map(|l| l.name.clone()));
    }
    pinned.extend(page.badge.iter().map(|b| b.loader.clone()));
    for decl in &page.see_also {
        if let SeeAlsoDecl::Rows { from } = decl {
            pinned.push(from.clone());
        }
    }
    for root in std::iter::once(&page.view).chain(page.rail.as_ref()) {
        embed_columns(root, &mut pinned);
    }
    for l in &page.loaders {
        pinned.extend(l.reads_loaders.iter().cloned());
    }
    named
        .into_iter()
        .filter(|n| !shown.contains(n) && !pinned.contains(n))
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

/// The datasets a page param the run resolves itself reads: `head_sha`,
/// when the page declares it, is the viewed stream's HEAD (`v_stream`), so
/// a commit or a branch switch re-resolves it and re-runs what binds it.
pub fn context_reads(page: &Page, param: &str) -> &'static [&'static str] {
    if param == "head_sha" && page.params.iter().any(|p| p.name == "head_sha") {
        &["v_stream"]
    } else {
        &[]
    }
}

/// The loader runs of `run` (its embeds' and mounts' included) a change
/// to `datasets` / `measures` doesn't reach, by key: a loader that read none
/// of them and reads no loader that did.
pub fn unreached(
    run: &PageRun,
    datasets: &[String],
    measures: &[String],
) -> HashMap<String, LoaderRun> {
    let mut out = HashMap::new();
    fn walk(
        run: &PageRun,
        datasets: &[String],
        measures: &[String],
        out: &mut HashMap<String, LoaderRun>,
    ) {
        let mut dirty: Vec<&str> = Vec::new();
        for decl in &run.page.loaders {
            let Some(r) = run.loaders.get(&decl.name) else {
                continue;
            };
            let reached =
                decl.reads.iter().any(|m| datasets.contains(m))
                    || decl.reads_params.iter().any(|p| {
                        context_reads(&run.page, p)
                            .iter()
                            .any(|m| datasets.iter().any(|x| x == m))
                    })
                    || r.result.reads.datasets.iter().any(|m| {
                        datasets.contains(m) && !r.result.reads.measure_scoped.contains(m)
                    })
                    || r.result.reads.measures.iter().any(|m| measures.contains(m))
                    || decl
                        .reads_loaders
                        .iter()
                        .any(|l| dirty.contains(&l.as_str()));
            if reached {
                dirty.push(&decl.name);
            } else {
                out.insert(r.key.clone(), r.clone());
            }
        }
        fn nodes<'r>(n: &'r RunNode, out: &mut Vec<&'r PageRun>) {
            if let Some(e) = &n.embed {
                out.push(e);
            }
            for c in &n.children {
                nodes(c, out);
            }
        }
        let mut embedded = Vec::new();
        nodes(&run.tree, &mut embedded);
        if let Some(rail) = &run.rail {
            nodes(rail, &mut embedded);
        }
        for e in embedded {
            walk(e, datasets, measures, out);
        }
    }
    walk(run, datasets, measures, &mut out);
    out
}

/// The loaders of `page` that a change makes run again, in declaration
/// order: those that read a changed dataset or bind a changed param, and
/// every loader that reads one of those.
pub fn dirty_loaders(
    page: &Page,
    run: &PageRun,
    datasets: &[String],
    params: &[String],
) -> Vec<String> {
    let mut dirty: Vec<String> = Vec::new();
    for decl in &page.loaders {
        let Some(r) = run.loaders.get(&decl.name) else {
            continue;
        };
        let reads_dataset = r
            .result
            .reads
            .datasets
            .iter()
            .chain(decl.reads.iter())
            .any(|m| datasets.contains(m));
        let binds_param = decl.reads_params.iter().any(|p| params.contains(p));
        let downstream = decl.reads_loaders.iter().any(|l| dirty.contains(l));
        if reads_dataset || binds_param || downstream {
            dirty.push(decl.name.clone());
        }
    }
    dirty
}
