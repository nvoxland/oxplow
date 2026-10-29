//! The page file (`pages/<id>.yaml` in an extension): what a page is
//! (its metadata: title, params, what it renders, its actions and the
//! questions it answers), where its data comes from (`loaders:`, named
//! loaders), and how it shows it (`view:`, a tree of nodes bound to the
//! loaders by name). Parsed and checked here, one file at a time; what
//! needs the whole extension (an embed's page, a component) is checked
//! where the extension loads.
//!
//! The YAML stays declarative: a node only binds (`from:`, `{{param.x}}`,
//! an arg's source); filtering and logic are SQL or a read command.
//!
//! A viz node's config (its viz, role block, columns, actions, empty
//! text) is a [`VizConfig`] with no query of its own: what a viz shows is its
//! loader's rows. So the shape checks, the action parser, the text
//! renderers and the desktop's viz components are the ones viz nodes use.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use serde::{Deserialize, Serialize};

use crate::extension::extensions::{shape_problem, Renders, TileSize};

use crate::pages::spec::{
    holds_one_row, parse_actions, typed, AlertFile, FormSpec, LauncherCategory, PageAction,
    PageAlert, PageColumn, PageParam, PageProvenance, ParamRole, ViewContext, Viz, VizChart,
    VizCode, VizConfig, VizCustom, VizDiff, VizEditor, VizEmbed, VizGroup, VizHunks, VizMarkdown,
    VizSteps, VizTerminal, VizTimeline, VizTree,
};

/// A loaded page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// `<extension>/<slug>`.
    pub id: String,
    pub extension: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// What its tab says, when not its title: `{{param.<name>}}` and
    /// `{{row.<column>}}` (its first loader's first row), with `|short`,
    /// `|basename`, `|url`.
    pub label: Option<String>,
    pub icon: Option<String>,
    /// Its launcher section; none: opened only from elsewhere.
    pub category: Option<LauncherCategory>,
    pub keywords: Option<String>,
    /// Its params, typed, each with its role (`resource` names what it
    /// shows, in its URI's `?`; `filter` and `selection` ride in the `#`).
    pub params: Vec<PageParam>,
    /// The resources it renders (`oxplow/commit`); empty: a route.
    #[serde(rename = "for")]
    pub renders: Vec<String>,
    /// Per URI param of the resource it renders: `present`, `absent` or an
    /// id pattern its value must match.
    pub accepts: BTreeMap<String, String>,
    /// `accepts: { content: <loader> }`: one of its own loaders, which
    /// must yield a row for the page to be offered for a URI
    /// (`pages/renderers.rs`).
    pub accepts_content: Option<String>,
    pub default: bool,
    pub pinned: bool,
    pub comments: bool,
    pub refresh: bool,
    pub hidden: bool,
    pub needs: Vec<String>,
    /// Its whole-page actions (a row action goes on the viz node that
    /// shows the row).
    pub actions: Vec<PageAction>,
    pub alert: Option<PageAlert>,
    /// The MCP tool it is also offered as, namespaced.
    pub tool: Option<String>,
    pub answers: Vec<PageAnswer>,
    pub badge: Option<Badge>,
    pub loaders: Vec<LoaderDecl>,
    pub view: Node,
    /// A second tree drawn in the page's rail.
    pub rail: Option<Node>,
    pub origin: PageProvenance,
}

impl Page {
    /// Every node of its view and rail, depth first.
    pub fn nodes(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        self.view.walk(&mut out);
        if let Some(rail) = &self.rail {
            rail.walk(&mut out);
        }
        out
    }

    pub fn loader(&self, name: &str) -> Option<&LoaderDecl> {
        self.loaders.iter().find(|l| l.name == name)
    }

    /// Loader `name` and every loader it reads, directly or not, in the
    /// order they are declared (and run): all that answering `name` takes.
    /// Empty when there is no such loader.
    pub fn loader_chain(&self, name: &str) -> Vec<String> {
        let mut wanted = vec![name.to_string()];
        let mut at = 0;
        while let Some(current) = wanted.get(at).cloned() {
            at += 1;
            if let Some(decl) = self.loader(&current) {
                for read in &decl.reads_loaders {
                    if !wanted.contains(read) {
                        wanted.push(read.clone());
                    }
                }
            }
        }
        self.loaders
            .iter()
            .filter(|l| wanted.contains(&l.name))
            .map(|l| l.name.clone())
            .collect()
    }

    /// Whether it is a resource page (`for:`): a display mode of the
    /// resources it names, handed their `content`; else it is standalone.
    pub fn is_resource_page(&self) -> bool {
        !self.renders.is_empty()
    }

    /// The loader whose first row is "the page's row" (`{{row.x}}`, a
    /// whole-page action's `when:`, the badge count, the alert): the first
    /// one its author declared, else the system's `content`.
    pub fn first_loader(&self) -> Option<&LoaderDecl> {
        self.loaders
            .iter()
            .find(|l| !matches!(l.loader, Loader::Content { .. }))
            .or_else(|| self.loaders.first())
    }
}

/// A question a page answers, with the params that answer it: what an
/// agent finds the page by, what the person is offered (on the pages of
/// `about`'s resource), and a test (`oxplow extension test` runs it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageAnswer {
    pub question: String,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    #[specta(type = BTreeMap<String, oxplow_domain::Json>)]
    pub params: BTreeMap<String, serde_json::Value>,
    /// The shipped skill whose text must name what the page reads (oxplow's
    /// own extensions).
    #[serde(default)]
    pub skill: Option<String>,
}

/// What a page's badge counts: a loader's first row's `column` (else
/// its first), or its row count when the column isn't a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Badge {
    pub loader: String,
    #[serde(default)]
    pub column: Option<String>,
}

/// A value flowing into a command's arg or another page's param:
/// declarative, never an expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ArgSource {
    /// One of the page's params.
    Param { param: String },
    /// An earlier loader's column, from its first row (`<loader>.<col>`).
    Column { from: String },
    /// A literal value.
    Literal(#[specta(type = oxplow_domain::Json)] serde_json::Value),
}

/// When a loader that reads outside the published datasets runs again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum LoaderRefresh {
    /// `manual`: only when the person refreshes the page.
    Keyword(RefreshKeyword),
    /// `{ every: 30s }`.
    Every { every: String },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum RefreshKeyword {
    Manual,
}

/// One of a page's named sources of rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LoaderDecl {
    pub name: String,
    pub loader: Loader,
    /// Column metadata its nodes default to, and the row columns it
    /// declares (what an embed or a slot mount may take by name).
    pub columns: Vec<PageColumn>,
    /// The datasets a `command` loader reads, so it runs again when they
    /// change (a `sql` loader's are what its query read).
    pub reads: Vec<String>,
    pub refresh: Option<LoaderRefresh>,
    pub empty: Option<String>,
    /// The earlier loaders it reads: as tables (`sql`) or through an
    /// arg's `from` (`command`, `page`).
    pub reads_loaders: Vec<String>,
    /// The page params it binds.
    pub reads_params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Loader {
    /// A read-only query over the published datasets, binding `:param`s
    /// (the page's, and `params`: named values from earlier loaders or
    /// literals); an earlier loader reads as a table of its name.
    Sql {
        sql: String,
        params: BTreeMap<String, ArgSource>,
    },
    /// A read-only command (`View` / `Read` access), run as the viewer.
    Command {
        command: String,
        args: BTreeMap<String, ArgSource>,
        /// The field of its result whose list is the rows; none: the
        /// result itself (a list of objects, an object, a table).
        items: Option<String>,
    },
    /// Another page's loader, run with `params`.
    Page {
        page: String,
        loader: String,
        params: BTreeMap<String, ArgSource>,
    },
    /// Rows written inline.
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<SqlCell>>,
    },
    /// What the system knows of the resource a `for:` page shows: the
    /// record its `uri` param names (`pages/content.rs`). Never declared by
    /// an author: it is the first loader of every resource page, named
    /// [`CONTENT`], configured by the page's `content:` key.
    Content {
        /// Read a file's text too (columns `text`, `truncated`).
        text: bool,
        /// The most bytes of a file's text read.
        max_bytes: u32,
    },
}

/// The name of a resource page's first loader, which the system declares.
pub const CONTENT: &str = "content";

/// The bytes of a file's text a resource page's `content` loader reads
/// when its page doesn't say.
pub const DEFAULT_CONTENT_BYTES: u32 = 64 * 1024;

/// The most a page's `content: { max_bytes }` may ask for.
pub const MAX_CONTENT_BYTES: u32 = 4 * 1024 * 1024;

/// A node of a page's view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// Unique in its page; `#node=<id>` points at it.
    pub id: String,
    pub title: Option<String>,
    /// Its heading isn't drawn (it stays in the page for Contents, screen
    /// readers and agents).
    pub hide_title: bool,
    /// A contract another extension's active provider may take its place
    /// under (`work_item.board`).
    pub replaceable: Option<String>,
    /// A grid child's size.
    pub size: Option<TileSize>,
    #[serde(flatten)]
    pub kind: NodeKind,
}

impl Node {
    fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for child in self.children() {
            child.walk(out);
        }
    }

    /// Every viz node's provenance below it, `self` included.
    pub fn for_each_origin(&mut self, f: &mut impl FnMut(&mut PageProvenance)) {
        match &mut self.kind {
            NodeKind::Viz { config: cfg, .. } => f(&mut cfg.origin),
            NodeKind::Stack { children }
            | NodeKind::Grid { children }
            | NodeKind::Switch { children, .. } => {
                for c in children {
                    c.for_each_origin(f);
                }
            }
            _ => {}
        }
    }

    pub fn children(&self) -> &[Node] {
        match &self.kind {
            NodeKind::Stack { children }
            | NodeKind::Grid { children }
            | NodeKind::Switch { children, .. } => children,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NodeKind {
    /// Its children one after another (sections).
    Stack { children: Vec<Node> },
    /// Its children as tiles, each at its `size`.
    Grid { children: Vec<Node> },
    /// The one child whose id is `param`'s value.
    Switch { param: String, children: Vec<Node> },
    /// A loader's rows through a viz. `from` is none only for a viz that
    /// reads no rows (a form, approvals).
    Viz {
        from: Option<String>,
        config: Box<VizConfig>,
    },
    /// Another page, inline: its params by name from this page's params
    /// and its loaders' first rows, `params` naming any explicitly.
    Embed {
        page: String,
        params: BTreeMap<String, ArgSource>,
    },
    /// Where other extensions' pages are mounted (`<page>.<name>`).
    Slot { name: String },
    /// Static markdown.
    Markdown { text: String },
    /// One of the extension's custom components, given the page's data.
    Component {
        component: String,
        #[specta(type = Option<oxplow_domain::Json>)]
        props: Option<serde_json::Value>,
    },
}

/// A page file as written on disk.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageFile {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    category: Option<LauncherCategory>,
    #[serde(default)]
    keywords: Option<String>,
    #[serde(default)]
    params: Vec<PageParam>,
    #[serde(default, rename = "for")]
    renders: Option<Renders>,
    /// Param rules by name, and `content: <loader>` (not a param).
    #[serde(default)]
    accepts: BTreeMap<String, String>,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    comments: bool,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    actions: Vec<serde_yaml::Value>,
    #[serde(default)]
    alert: Option<AlertFile>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    answers: Vec<PageAnswer>,
    #[serde(default)]
    badge: Option<Badge>,
    /// How the `content` loader of a resource page reads.
    #[serde(default)]
    content: Option<ContentFile>,
    /// Its loaders, in order (a mapping keeps it).
    #[serde(default)]
    loaders: Option<serde_yaml::Mapping>,
    view: serde_yaml::Value,
    #[serde(default)]
    rail: Option<serde_yaml::Value>,
}

/// A resource page's `content:` key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentFile {
    /// Read a file's text too.
    #[serde(default)]
    text: bool,
    #[serde(default)]
    max_bytes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoaderFile {
    #[serde(default)]
    sql: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: BTreeMap<String, ArgSource>,
    #[serde(default)]
    items: Option<String>,
    #[serde(default)]
    page: Option<String>,
    #[serde(default)]
    loader: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, ArgSource>,
    #[serde(default)]
    rows: Option<Vec<serde_yaml::Mapping>>,
    #[serde(default)]
    columns: Vec<PageColumn>,
    #[serde(default)]
    reads: Vec<String>,
    #[serde(default)]
    refresh: Option<LoaderRefresh>,
    #[serde(default)]
    empty: Option<String>,
}

/// A viz node's own keys, beside `id`, `title`, `replaceable`, `size`
/// and `from`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VizFile {
    viz: Viz,
    #[serde(default)]
    columns: Vec<PageColumn>,
    #[serde(default)]
    empty: Option<String>,
    #[serde(default)]
    chart: Option<VizChart>,
    #[serde(default)]
    tree: Option<VizTree>,
    #[serde(default)]
    timeline: Option<VizTimeline>,
    #[serde(default)]
    steps: Option<VizSteps>,
    #[serde(default)]
    hunks: Option<VizHunks>,
    #[serde(default)]
    code: Option<VizCode>,
    #[serde(default)]
    diff: Option<VizDiff>,
    #[serde(default)]
    editor: Option<VizEditor>,
    #[serde(default)]
    embed: Option<VizEmbed>,
    #[serde(default)]
    terminal: Option<VizTerminal>,
    #[serde(default)]
    markdown: Option<VizMarkdown>,
    #[serde(default)]
    form: Option<FormSpec>,
    #[serde(default)]
    custom: Option<VizCustom>,
    #[serde(default)]
    group: Option<VizGroup>,
    #[serde(default)]
    emphasis: Option<String>,
    #[serde(default)]
    depth: Option<String>,
    #[serde(default)]
    actions: Vec<serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedFile {
    page: String,
    #[serde(default)]
    params: BTreeMap<String, ArgSource>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchFile {
    param: String,
    children: Vec<serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentFile {
    id: String,
    #[serde(default)]
    props: Option<serde_json::Value>,
}

/// The vizes a node shows no rows through.
fn reads_no_rows(viz: Viz) -> bool {
    matches!(viz, Viz::Form | Viz::Approvals)
}

/// Parse and check page file `text`, page `slug` of extension `extension`
/// (whose commands and tools are under `namespace`), read from `rel`.
pub fn parse_page(
    extension: &str,
    namespace: &str,
    slug: &str,
    rel: &str,
    text: &str,
) -> Result<Page, String> {
    let file: PageFile = serde_yaml::from_str(text)
        .map_err(|e| oxplow_domain::manifest_keys::hint_unknown_page(&e.to_string()))?;
    let id = format!("{extension}/{slug}");
    let renders = file
        .renders
        .clone()
        .map(Renders::into_vec)
        .unwrap_or_default();
    let resource = !renders.is_empty();
    if let Some(why) = page_kind_problem(&file, resource) {
        return Err(why);
    }
    let origin = PageProvenance {
        extension: extension.to_string(),
        source: None,
        running: rel.to_string(),
        deployment: None,
        bundled: false,
        running_uri: None,
    };
    let mut file = file;
    let accepts_content = file.accepts.remove(ACCEPTS_CONTENT);
    let content = file.content.as_ref();
    let content_loader = Loader::Content {
        text: content.is_some_and(|c| c.text),
        max_bytes: content
            .and_then(|c| c.max_bytes)
            .unwrap_or(DEFAULT_CONTENT_BYTES),
    };
    let mut params = file.params;
    let switch_params = switch_params(&file.view);
    for p in &mut params {
        if switch_params.contains(&p.name) && p.role.is_none() {
            p.role = Some(ParamRole::Filter);
        }
    }
    let params = typed(params);
    if let Some(why) = crate::pages::spec::param_problem(&params) {
        return Err(why);
    }
    let loaders = parse_loaders(
        file.loaders.unwrap_or_default(),
        &params,
        resource.then_some(content_loader),
    )?;
    let mut ids = Ids::default();
    let mut ctx = NodeCtx {
        extension,
        page_id: &id,
        title: &file.title,
        params: &params,
        origin: &origin,
        ids: &mut ids,
    };
    let view = parse_node(&file.view, &mut ctx)?;
    let rail = file
        .rail
        .as_ref()
        .map(|r| parse_node(r, &mut ctx))
        .transpose()?;
    // A param a column `select:`s is the page's selection (in the `#`).
    let mut nodes = Vec::new();
    view.walk(&mut nodes);
    if let Some(r) = &rail {
        r.walk(&mut nodes);
    }
    let selected: Vec<String> = nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Viz { config: cfg, .. } => {
                Some(cfg.columns.iter().filter_map(|c| c.select.clone()))
            }
            _ => None,
        })
        .flatten()
        .collect();
    let mut params = params;
    for p in &mut params {
        if selected.contains(&p.name) {
            p.role = Some(ParamRole::Selection);
        }
    }
    // A whole-page action may read a row (`{{row.x}}`, `when:`): the first
    // loader's one row, as a document's row once was.
    let actions = parse_actions(file.actions, &params, false, true)?;
    if let Some(a) = actions
        .iter()
        .find(|a| a.row || a.group.is_some() || a.drag)
    {
        return Err(format!(
            "actions: `{}` acts on a row: a row action goes on the viz node that shows the row",
            a.id
        ));
    }
    let alert = file.alert.map(AlertFile::into_alert).transpose()?;
    for need in &file.needs {
        oxplow_domain::capability::check_need(need)?;
    }
    let tool = file
        .tool
        .map(|t| crate::extension::extension_commands::tool_name(namespace, &t))
        .transpose()?;
    if let Some(loader) = &accepts_content {
        if let Some(why) = accepts_content_problem(loader, &loaders, resource) {
            return Err(why);
        }
    }
    let page = Page {
        id,
        extension: extension.to_string(),
        slug: slug.to_string(),
        title: file.title,
        description: file.description,
        label: file.label,
        icon: file.icon,
        category: file.category,
        keywords: file.keywords,
        params,
        renders,
        accepts: file.accepts,
        accepts_content,
        default: file.default,
        pinned: file.pinned,
        comments: file.comments,
        refresh: file.refresh,
        hidden: file.hidden,
        needs: file.needs,
        actions,
        alert,
        tool,
        answers: file.answers,
        badge: file.badge,
        loaders,
        view,
        rail,
        origin,
    };
    check_page(&page)?;
    Ok(page)
}

/// Why `file` can't be the kind of page it says it is. A resource page
/// (`for:`) is a display mode of the resources it names: never listed on
/// its own, so it has no launcher section, keywords or badge and can't be
/// hidden, and it is handed the URI it shows as its `uri` param. A
/// standalone page has no `content`.
fn page_kind_problem(file: &PageFile, resource: bool) -> Option<String> {
    if let Some(p) = file.params.iter().find(|p| p.name == CONTENT) {
        return Some(format!(
            "param `{}`: `{CONTENT}` is the name of a resource page's loader, so a param can't have it",
            p.name
        ));
    }
    if !resource {
        if file.content.is_some() {
            return Some(
                "`content:` configures the `content` loader of a resource page: this page has no `for:`"
                    .into(),
            );
        }
        return None;
    }
    for (key, given) in [
        ("category", file.category.is_some()),
        ("hidden", file.hidden),
        ("keywords", file.keywords.is_some()),
        ("badge", file.badge.is_some()),
    ] {
        if given {
            return Some(format!(
                "a page with `for:` is a display mode of a resource, opened from the resource and never listed on its own: \
                 it takes no `{key}` (a page with no `for:` is the one the launcher and search list)"
            ));
        }
    }
    if !file.params.iter().any(|p| p.name == "uri") {
        return Some(
            "a page with `for:` is given the URI it shows: declare `params: [{ name: uri }]`"
                .into(),
        );
    }
    if let Some(max) = file.content.as_ref().and_then(|c| c.max_bytes) {
        if max == 0 || max > MAX_CONTENT_BYTES {
            return Some(format!(
                "content: `max_bytes: {max}` must be between 1 and {MAX_CONTENT_BYTES} (4 MiB)"
            ));
        }
    }
    None
}

/// The `accepts` key that names a loader, not a URI param.
pub const ACCEPTS_CONTENT: &str = "content";

/// Why `accepts: { content: <loader> }` can't be asked of a URI: the
/// daemon runs only that loader's chain (sql, rows, a read command,
/// `content`) to decide, so it can't need another page.
fn accepts_content_problem(loader: &str, loaders: &[LoaderDecl], resource: bool) -> Option<String> {
    if !resource {
        return Some(
            "`accepts: content` is for a resource page (a page with `for:`): it says what a URI's content must hold for the page to be offered"
                .into(),
        );
    }
    let Some(decl) = loaders.iter().find(|l| l.name == loader) else {
        let names: Vec<&str> = loaders.iter().map(|l| l.name.as_str()).collect();
        return Some(format!(
            "`accepts: content: {loader}` isn't a loader of the page (loaders: {})",
            names.join(", ")
        ));
    };
    let mut wanted = vec![decl.name.clone()];
    let mut at = 0;
    while let Some(current) = wanted.get(at).cloned() {
        at += 1;
        let Some(l) = loaders.iter().find(|l| l.name == current) else {
            continue;
        };
        if matches!(l.loader, Loader::Page { .. }) {
            return Some(format!(
                "`accepts: content: {loader}` reads the `page` loader `{}`: it is decided from `sql`, `rows`, read-only `command` and `content` loaders only",
                l.name
            ));
        }
        for read in &l.reads_loaders {
            if !wanted.contains(read) {
                wanted.push(read.clone());
            }
        }
    }
    None
}

/// The id of an inline page: a markdown block's or an agent's one-off.
pub const INLINE_ID: &str = "inline";

/// Parse an inline page — a ` ```oxplow-page ` block's, an agent's
/// one-off — which only reads: its loaders are queries, rows and other
/// pages' loaders; its nodes show, embed deployed pages, and never act.
pub fn parse_inline_page(text: &str) -> Result<Page, String> {
    let mut page = parse_page(INLINE_ID, INLINE_ID, INLINE_ID, INLINE_ID, text)?;
    page.id = INLINE_ID.to_string();
    if let Some(why) = inline_problem(&page) {
        return Err(why);
    }
    Ok(page)
}

/// The extension a temp page (a page file on `oxplow:temp`) is under, so
/// its id is `temp/<id>`.
pub const TEMP_EXTENSION: &str = "temp";

/// The file `temp/<id>` is, by its name on `oxplow:temp`.
pub fn temp_file_name(id: &str) -> String {
    format!("{id}.yaml")
}

/// Parse a temp page: the page file `<id>.yaml` on `oxplow:temp`, an
/// agent's shown view. It has the inline rules (it only reads), is
/// `temp/<id>`, runs from its temp file (`PageProvenance.running_uri`, so
/// Definition in About works) and is never a resource page.
pub fn parse_temp_page(id: &str, text: &str) -> Result<Page, String> {
    let file = temp_file_name(id);
    let rel = format!("{}/{file}", crate::filesystems::temp::TEMP_DIR);
    let mut page = parse_page(TEMP_EXTENSION, INLINE_ID, id, &rel, text)?;
    if let Some(why) = inline_problem(&page) {
        return Err(why);
    }
    if page.is_resource_page() {
        return Err("a temp page is a view, not a display mode: it can't have `for:`".into());
    }
    page.origin.running_uri = Some(oxplow_domain::uri::build::file_uri_at(
        &file,
        Some(oxplow_domain::filesystem::TEMP),
        None,
        None,
    ));
    Ok(page)
}

/// Why `page` can't be an inline page: something in it acts.
fn inline_problem(page: &Page) -> Option<String> {
    if let Some(l) = page
        .loaders
        .iter()
        .find(|l| matches!(l.loader, Loader::Command { .. }))
    {
        return Some(format!(
            "loader `{}`: an inline page reads only through `sql`, `rows` and `page` loaders, \
             not a command",
            l.name
        ));
    }
    if !page.actions.is_empty() {
        return Some("an inline page takes no actions: it shows, it never acts".into());
    }
    for node in page.nodes() {
        let why = match &node.kind {
            NodeKind::Component { .. } => Some("a component"),
            NodeKind::Slot { .. } => Some("a slot"),
            NodeKind::Viz { config: cfg, .. } if !cfg.actions.is_empty() => Some("actions"),
            NodeKind::Viz { config: cfg, .. } => match cfg.viz {
                Viz::Form => Some("a form"),
                Viz::Approvals => Some("approvals"),
                Viz::Terminal => Some("a terminal"),
                Viz::Editor => Some("an editor"),
                Viz::Custom => Some("a component"),
                Viz::Embed => Some("a web page"),
                _ => None,
            },
            _ => None,
        };
        if let Some(what) = why {
            return Some(format!(
                "node `{}`: an inline page can't hold {what}: it shows, it never acts",
                node.id
            ));
        }
    }
    None
}

/// The params named by a `switch` anywhere in `view`.
fn switch_params(view: &serde_yaml::Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &serde_yaml::Value, out: &mut Vec<String>) {
        let Some(m) = v.as_mapping() else { return };
        if let Some(sw) = m.get("switch").and_then(|s| s.as_mapping()) {
            if let Some(p) = sw.get("param").and_then(|p| p.as_str()) {
                out.push(p.to_string());
            }
            for c in sw
                .get("children")
                .and_then(|c| c.as_sequence())
                .into_iter()
                .flatten()
            {
                walk(c, out);
            }
        }
        for key in ["stack", "grid"] {
            for c in m
                .get(key)
                .and_then(|c| c.as_sequence())
                .into_iter()
                .flatten()
            {
                walk(c, out);
            }
        }
    }
    walk(view, &mut out);
    out
}

/// A loader name: a plain lowercase identifier that can't be mistaken for
/// (or shadow) a published dataset, or be a SQL keyword.
fn loader_name_problem(name: &str) -> Option<String> {
    let plain = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !plain {
        return Some(format!(
            "loader `{name}`: a loader's name is lowercase letters, digits and `_`, starting with a letter"
        ));
    }
    if name == CONTENT {
        return Some(format!(
            "loader `{name}`: `{CONTENT}` is the system's loader of a resource page (a page with `for:`): name it something else"
        ));
    }
    if oxplow_db::sql_tokens::is_reserved_word(name) {
        return Some(format!(
            "loader `{name}`: that name is a SQL keyword, which SQL can't name a table by: \
             name it something else"
        ));
    }
    if name.starts_with("v_") || name.starts_with("m_") || matches!(name, "temp" | "main") {
        return Some(format!(
            "loader `{name}`: that name is a dataset's (or the database's): name it something else"
        ));
    }
    None
}

/// The identifiers `sql` names as tables or columns, not functions or
/// qualified columns: what an earlier loader's name is looked for among.
/// Read with the one SQL tokenizer, so a string, a comment or a quoted
/// identifier's quotes never hide or invent a name.
fn sql_names(sql: &str) -> Result<Vec<String>, String> {
    use oxplow_db::sql_tokens::{significant, tokenize, TokenKind};
    let tokens = tokenize(sql).map_err(|e| e.to_string())?;
    let sig: Vec<_> = significant(&tokens).collect();
    let mut out = Vec::new();
    for (i, t) in sig.iter().enumerate() {
        let name = match t.kind {
            TokenKind::Word => t.text,
            TokenKind::QuotedIdent => &t.text[1..t.text.len() - 1],
            _ => continue,
        };
        let qualified = i > 0 && sig[i - 1].is_punct('.');
        let call = sig.get(i + 1).is_some_and(|n| n.is_punct('('));
        if !qualified && !call {
            out.push(name.to_ascii_lowercase());
        }
    }
    Ok(out)
}

/// The `:param` names `sql` binds, by the one SQL tokenizer.
fn sql_params(sql: &str) -> Result<Vec<String>, String> {
    use oxplow_db::sql_tokens::{tokenize, TokenKind};
    let tokens = tokenize(sql).map_err(|e| e.to_string())?;
    let mut out: Vec<String> = Vec::new();
    for t in tokens.iter().filter(|t| t.kind == TokenKind::Param) {
        if let Some(name) = t.text.strip_prefix(':') {
            if !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

/// A resource page's `content` loader, first of its loaders.
fn content_decl(loader: Loader) -> LoaderDecl {
    LoaderDecl {
        name: CONTENT.to_string(),
        loader,
        columns: Vec::new(),
        reads: Vec::new(),
        refresh: None,
        empty: None,
        reads_loaders: Vec::new(),
        reads_params: vec!["uri".to_string()],
    }
}

/// Whether `sql` reads `content` as a table (`FROM content`, `JOIN content`).
fn reads_content_table(sql: &str) -> bool {
    use oxplow_db::sql_tokens::{significant, tokenize, TokenKind};
    let Ok(tokens) = tokenize(sql) else {
        return false;
    };
    let sig: Vec<_> = significant(&tokens).collect();
    sig.windows(2).any(|w| {
        w[0].kind == TokenKind::Word
            && (w[0].text.eq_ignore_ascii_case("from") || w[0].text.eq_ignore_ascii_case("join"))
            && w[1].kind == TokenKind::Word
            && w[1].text.eq_ignore_ascii_case(CONTENT)
    })
}

const STANDALONE_CONTENT: &str =
    "`content` is the loader a resource page (a page with `for:`) is handed; \
     a page with no `for:` has none: query for what it shows";

/// `loaders` as declared, after the system's `content` loader when `content`
/// is given (a resource page).
fn parse_loaders(
    loaders: serde_yaml::Mapping,
    params: &[PageParam],
    content: Option<Loader>,
) -> Result<Vec<LoaderDecl>, String> {
    let resource = content.is_some();
    let mut names: Vec<String> = Vec::new();
    if resource {
        names.push(CONTENT.to_string());
    }
    names.extend(
        loaders
            .keys()
            .map(|k| k.as_str().unwrap_or_default().to_string()),
    );
    let mut out: Vec<LoaderDecl> = content.map(content_decl).into_iter().collect();
    let offset = usize::from(resource);
    for (index, (key, value)) in loaders.into_iter().enumerate() {
        let index = index + offset;
        let name = key
            .as_str()
            .ok_or_else(|| "loaders: a loader's name is text".to_string())?
            .to_string();
        if let Some(why) = loader_name_problem(&name) {
            return Err(why);
        }
        let file: LoaderFile =
            serde_yaml::from_value(value).map_err(|e| format!("loader `{name}`: {e}"))?;
        let kinds = [
            file.sql.is_some(),
            file.command.is_some(),
            file.page.is_some(),
            file.rows.is_some(),
        ];
        if kinds.iter().filter(|k| **k).count() != 1 {
            return Err(format!(
                "loader `{name}` needs exactly one of `sql`, `command`, `page` or `rows`"
            ));
        }
        if file.loader.is_some() && file.page.is_none() {
            return Err(format!("loader `{name}`: `loader` is for a `page` loader"));
        }
        let earlier = &names[..index];
        let later = |reads: &str| names[index + 1..].iter().any(|n| n == reads);
        let mut reads_loaders = Vec::new();
        let mut reads_params = Vec::new();
        let check_arg = |arg: &ArgSource,
                         reads_loaders: &mut Vec<String>,
                         reads_params: &mut Vec<String>|
         -> Result<(), String> {
            match arg {
                ArgSource::Param { param } => {
                    if !params.iter().any(|p| &p.name == param) && !ViewContext::supplies(param) {
                        return Err(format!(
                            "loader `{name}`: param `{param}` isn't one of the page's"
                        ));
                    }
                    reads_params.push(param.clone());
                }
                ArgSource::Column { from } => {
                    let source = from.split('.').next().unwrap_or_default();
                    if source == CONTENT && !resource {
                        return Err(format!("loader `{name}`: {STANDALONE_CONTENT}"));
                    }
                    if !earlier.iter().any(|n| n == source) || !from.contains('.') {
                        return Err(format!(
                            "loader `{name}`: `from: {from}` names `{source}`, which isn't a loader \
                             declared before it (write `<loader>.<column>`)"
                        ));
                    }
                    if !reads_loaders.iter().any(|n| n == source) {
                        reads_loaders.push(source.to_string());
                    }
                }
                ArgSource::Literal(_) => {}
            }
            Ok(())
        };
        if !file.params.is_empty() && file.command.is_some() {
            return Err(format!(
                "loader `{name}`: a command loader takes `args`, not `params`"
            ));
        }
        let loader = if let Some(sql) = file.sql {
            for arg in file.params.values() {
                check_arg(arg, &mut reads_loaders, &mut reads_params)?;
            }
            if !resource && reads_content_table(&sql) {
                return Err(format!("loader `{name}`: {STANDALONE_CONTENT}"));
            }
            for word in sql_names(&sql).map_err(|e| format!("loader `{name}`: {e}"))? {
                if word == name {
                    continue;
                }
                if later(&word) {
                    return Err(format!(
                        "loader `{name}` reads `{word}`, which is declared after it: a loader \
                         reads only the loaders before it"
                    ));
                }
                if earlier.contains(&word) && !reads_loaders.contains(&word) {
                    reads_loaders.push(word);
                }
            }
            for p in sql_params(&sql).map_err(|e| format!("loader `{name}`: {e}"))? {
                if !reads_params.contains(&p) && !file.params.contains_key(&p) {
                    reads_params.push(p);
                }
            }
            Loader::Sql {
                sql,
                params: file.params,
            }
        } else if let Some(command) = file.command {
            oxplow_domain::CommandSpec::validate_id(&command)
                .map_err(|e| format!("loader `{name}`: {e}"))?;
            for arg in file.args.values() {
                check_arg(arg, &mut reads_loaders, &mut reads_params)?;
            }
            if file.reads.is_empty() && file.refresh.is_none() {
                return Err(format!(
                    "loader `{name}`: a command loader says what makes it run again: `reads` \
                     (the datasets it reads) or `refresh` (`manual`, or `{{ every: 30s }}`)"
                ));
            }
            Loader::Command {
                command,
                args: file.args,
                items: file.items,
            }
        } else if let Some(page) = file.page {
            let loader = file.loader.ok_or_else(|| {
                format!("loader `{name}`: a page loader names the page's `loader` it takes")
            })?;
            for arg in file.params.values() {
                check_arg(arg, &mut reads_loaders, &mut reads_params)?;
            }
            Loader::Page {
                page,
                loader,
                params: file.params,
            }
        } else {
            let rows = file.rows.unwrap_or_default();
            let mut columns: Vec<String> = Vec::new();
            for row in &rows {
                for k in row.keys() {
                    let k = k.as_str().unwrap_or_default().to_string();
                    if !columns.contains(&k) {
                        columns.push(k);
                    }
                }
            }
            let rows = rows
                .iter()
                .map(|row| {
                    columns
                        .iter()
                        .map(|c| {
                            row.get(c.as_str())
                                .map(|v| {
                                    SqlCell::from(
                                        serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
                                    )
                                })
                                .unwrap_or(SqlCell::Null(()))
                        })
                        .collect()
                })
                .collect();
            Loader::Rows { columns, rows }
        };
        out.push(LoaderDecl {
            name,
            loader,
            columns: file.columns,
            reads: file.reads,
            refresh: file.refresh,
            empty: file.empty,
            reads_loaders,
            reads_params,
        });
    }
    Ok(out)
}

/// Node ids handed out so far, so defaults don't collide.
#[derive(Default)]
struct Ids {
    layout: BTreeMap<&'static str, usize>,
}

impl Ids {
    fn layout(&mut self, kind: &'static str) -> String {
        let n = self.layout.entry(kind).or_default();
        *n += 1;
        if *n == 1 {
            kind.to_string()
        } else {
            format!("{kind}-{n}")
        }
    }
}

struct NodeCtx<'a> {
    extension: &'a str,
    page_id: &'a str,
    title: &'a str,
    params: &'a [PageParam],
    origin: &'a PageProvenance,
    ids: &'a mut Ids,
}

const NODE_KINDS: &str =
    "`stack`, `grid`, `switch`, `viz`, `embed`, `slot`, `markdown` or `component`";

fn parse_node(value: &serde_yaml::Value, ctx: &mut NodeCtx<'_>) -> Result<Node, String> {
    let mut map = value
        .as_mapping()
        .cloned()
        .ok_or_else(|| format!("a node is a mapping with one of {NODE_KINDS}"))?;
    let take_str = |map: &mut serde_yaml::Mapping, key: &str| -> Result<Option<String>, String> {
        match map.remove(key) {
            None => Ok(None),
            Some(serde_yaml::Value::String(s)) => Ok(Some(s)),
            Some(other) => Err(format!("`{key}` is text, not {other:?}")),
        }
    };
    let id = take_str(&mut map, "id")?;
    let title = take_str(&mut map, "title")?;
    let hide_title = match map.remove("hide_title") {
        None => false,
        Some(serde_yaml::Value::Bool(b)) => b,
        Some(other) => return Err(format!("`hide_title` is true or false, not {other:?}")),
    };
    let replaceable = take_str(&mut map, "replaceable")?;
    let size = map
        .remove("size")
        .map(serde_yaml::from_value::<TileSize>)
        .transpose()
        .map_err(|e| format!("`size`: {e}"))?;
    let only = |map: &serde_yaml::Mapping, kind: &str| -> Result<(), String> {
        let extra: Vec<String> = map
            .keys()
            .filter_map(|k| k.as_str())
            .filter(|k| *k != kind)
            .map(|k| format!("`{k}`"))
            .collect();
        if extra.is_empty() {
            Ok(())
        } else {
            Err(format!("a `{kind}` node takes no {}", extra.join(", ")))
        }
    };
    let children = |seq: Option<&serde_yaml::Value>,
                    kind: &str,
                    ctx: &mut NodeCtx<'_>|
     -> Result<Vec<Node>, String> {
        seq.and_then(|s| s.as_sequence())
            .ok_or_else(|| format!("`{kind}` is a list of nodes"))?
            .iter()
            .map(|c| parse_node(c, ctx))
            .collect()
    };
    let (default_id, kind) = if map.contains_key("viz") {
        let from = take_str(&mut map, "from")?;
        let file: VizFile =
            serde_yaml::from_value(serde_yaml::Value::Mapping(map)).map_err(|e| e.to_string())?;
        let node_id = id
            .clone()
            .or_else(|| from.clone())
            .unwrap_or_else(|| ctx.ids.layout("view"));
        let cfg = leaf_viz(ctx, &node_id, title.as_deref(), file)?;
        (
            node_id,
            NodeKind::Viz {
                from,
                config: Box::new(cfg),
            },
        )
    } else if map.contains_key("stack") {
        only(&map, "stack")?;
        let kids = children(map.get("stack"), "stack", ctx)?;
        (ctx.ids.layout("stack"), NodeKind::Stack { children: kids })
    } else if map.contains_key("grid") {
        only(&map, "grid")?;
        let kids = children(map.get("grid"), "grid", ctx)?;
        (ctx.ids.layout("grid"), NodeKind::Grid { children: kids })
    } else if map.contains_key("switch") {
        only(&map, "switch")?;
        let sw: SwitchFile = serde_yaml::from_value(map.remove("switch").unwrap_or_default())
            .map_err(|e| format!("`switch`: {e}"))?;
        let kids = sw
            .children
            .iter()
            .map(|c| parse_node(c, ctx))
            .collect::<Result<Vec<_>, _>>()?;
        (
            ctx.ids.layout("switch"),
            NodeKind::Switch {
                param: sw.param,
                children: kids,
            },
        )
    } else if map.contains_key("embed") {
        only(&map, "embed")?;
        let e: EmbedFile = serde_yaml::from_value(map.remove("embed").unwrap_or_default())
            .map_err(|e| format!("`embed`: {e}"))?;
        let slug = e.page.rsplit('/').next().unwrap_or(&e.page).to_string();
        (
            slug,
            NodeKind::Embed {
                page: e.page,
                params: e.params,
            },
        )
    } else if map.contains_key("slot") {
        only(&map, "slot")?;
        let name = take_str(&mut map, "slot")?.unwrap_or_default();
        (name.clone(), NodeKind::Slot { name })
    } else if map.contains_key("markdown") {
        only(&map, "markdown")?;
        let text = take_str(&mut map, "markdown")?.unwrap_or_default();
        (ctx.ids.layout("markdown"), NodeKind::Markdown { text })
    } else if map.contains_key("component") {
        only(&map, "component")?;
        let c: ComponentFile = serde_yaml::from_value(map.remove("component").unwrap_or_default())
            .map_err(|e| format!("`component`: {e}"))?;
        (
            c.id.clone(),
            NodeKind::Component {
                component: c.id,
                props: c.props,
            },
        )
    } else {
        return Err(format!("a node needs one of {NODE_KINDS}"));
    };
    Ok(Node {
        id: id.unwrap_or(default_id),
        title,
        hide_title,
        replaceable,
        size,
        kind,
    })
}

/// A viz node's config as a viz config with no query: its rows are its loader's.
fn leaf_viz(
    ctx: &NodeCtx<'_>,
    node_id: &str,
    title: Option<&str>,
    f: VizFile,
) -> Result<VizConfig, String> {
    let actions = parse_actions(
        f.actions,
        ctx.params,
        f.group.is_some(),
        holds_one_row(f.viz),
    )
    .map_err(|e| format!("node `{node_id}`: {e}"))?;
    let cfg = VizConfig {
        id: format!("{}#{node_id}", ctx.page_id),
        extension: ctx.extension.to_string(),
        slug: node_id.to_string(),
        title: title.unwrap_or(ctx.title).to_string(),
        viz: f.viz,
        params: ctx.params.to_vec(),
        columns: f.columns,
        empty: f.empty,
        chart: f.chart,
        tree: f.tree,
        timeline: f.timeline,
        steps: f.steps,
        hunks: f.hunks,
        code: f.code,
        diff: f.diff,
        editor: f.editor,
        embed: f.embed,
        terminal: f.terminal,
        markdown: f.markdown,
        form: f.form,
        custom: f.custom,
        group: f.group,
        emphasis: f.emphasis,
        depth: f.depth,
        fills: f.viz.fills(),
        actions,
        origin: ctx.origin.clone(),
    };
    // A custom viz's component is the extension's: checked where it loads.
    if cfg.viz != Viz::Custom {
        if let Some(why) = shape_problem(&cfg, &[]) {
            return Err(format!("node `{node_id}`: {why}"));
        }
    }
    Ok(cfg)
}

/// A section — a child of a `stack`, `grid` or `switch` — says what it is
/// with a `title`; whether that heading is drawn is `hide_title`. A slot
/// or an embed takes its title from the page it shows, so it needs none.
fn check_titles(node: &Node) -> Result<(), String> {
    for child in node.children() {
        let titled = child.title.as_deref().is_some_and(|t| !t.trim().is_empty());
        if !titled && !matches!(child.kind, NodeKind::Slot { .. } | NodeKind::Embed { .. }) {
            return Err(format!(
                "node `{}` is a section and needs a `title`: give it one, and add \
                 `hide_title: true` if its heading shouldn't show (Contents and agents \
                 still read it)",
                child.id
            ));
        }
        check_titles(child)?;
    }
    Ok(())
}

/// What needs the whole page: node ids, bindings, selections, switches,
/// the badge and the answers.
fn check_page(page: &Page) -> Result<(), String> {
    // A node another extension's page may stand in for names a target
    // oxplow knows, once per page.
    let mut targets: Vec<&str> = Vec::new();
    for node in page.nodes() {
        let Some(target) = node.replaceable.as_deref() else {
            continue;
        };
        if oxplow_domain::replaceable::replaceable(target).is_none() {
            return Err(format!(
                "node `{}`: `replaceable: {target}` isn't a replaceable component ({})",
                node.id,
                oxplow_domain::replaceable::targets()
            ));
        }
        if targets.contains(&target) {
            return Err(format!("`replaceable: {target}` is placed once on a page"));
        }
        targets.push(target);
    }
    check_titles(&page.view)?;
    if let Some(rail) = &page.rail {
        check_titles(rail)?;
    }
    let mut seen: Vec<&str> = Vec::new();
    for node in page.nodes() {
        if seen.contains(&node.id.as_str()) {
            return Err(format!(
                "node id `{}` is used twice: give one an `id`",
                node.id
            ));
        }
        seen.push(&node.id);
        match &node.kind {
            NodeKind::Viz { from, config: cfg } => match from {
                Some(from) if page.loader(from).is_none() => {
                    return Err(format!(
                        "node `{}` shows `{from}`, but there is no loader `{from}` in `loaders:`",
                        node.id
                    ));
                }
                None if !reads_no_rows(cfg.viz) => {
                    return Err(format!(
                        "node `{}` needs `from`: the loader whose rows it shows",
                        node.id
                    ));
                }
                _ => {}
            },
            NodeKind::Switch { param, .. } => {
                let declared = page.params.iter().find(|p| &p.name == param);
                if declared.is_none_or(|p| p.role != Some(ParamRole::Filter)) {
                    return Err(format!(
                        "node `{}` switches on param `{param}`: declare it among the page's \
                         params as a `filter`",
                        node.id
                    ));
                }
            }
            _ => {}
        }
    }
    if let Some(b) = &page.badge {
        if page.loader(&b.loader).is_none() {
            return Err(format!(
                "badge: there is no loader `{}` in `loaders:`",
                b.loader
            ));
        }
    }
    for a in &page.answers {
        if a.question.trim().is_empty() {
            return Err("answers: a question is needed".into());
        }
        if let Some(name) = a
            .params
            .keys()
            .find(|k| !page.params.iter().any(|p| &p.name == *k))
        {
            return Err(format!(
                "answers: \"{}\" gives param `{name}`, which isn't one of the page's",
                a.question
            ));
        }
    }
    Ok(())
}

/// oxplow's own extensions (`namespace: oxplow`) share the core page names
/// — `oxplow:oxplow/page?page=<slug>` names a page without its extension —
/// so a slug two of them declare is ambiguous. The first in load order
/// keeps it; each later one loses that page and says why.
pub fn refuse_shared_core_pages(extensions: &mut [crate::extension::extensions::Extension]) {
    let mut owner: std::collections::BTreeMap<String, String> = Default::default();
    for ext in extensions.iter_mut() {
        if !ext.enabled || ext.namespace != oxplow_domain::OXPLOW_NAMESPACE {
            continue;
        }
        let mut errors = Vec::new();
        ext.page_files.retain(|page| match owner.get(&page.slug) {
            Some(first) => {
                errors.push(format!(
                    "{}/pages/{}.yaml: page `{}` is also `{first}`'s, and oxplow's own \
                     extensions share page names; rename one",
                    ext.path, page.slug, page.slug
                ));
                false
            }
            None => true,
        });
        for page in &ext.page_files {
            owner.insert(page.slug.clone(), ext.name.clone());
        }
        ext.errors.extend(errors);
    }
}
