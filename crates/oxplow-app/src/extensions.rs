//! Project extensions and their lenses.
//!
//! An extension is a folder `oxplow/extensions/<name>/` in a stream's
//! worktree with an `extension.yaml` and `lenses/<slug>.yaml` files. A
//! lens is a query over the semantic layer (`v_*` views) plus how to
//! show it. See `.context/extensions.md`.
//!
//! Loading never fails as a whole: a broken extension or lens is
//! reported in that extension's `errors` and everything else still
//! loads.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{SemanticLayer, SqlCell, SqlQueryResult};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

/// Where project extensions live, relative to a worktree root.
pub const EXTENSIONS_DIR: &str = "oxplow/extensions";

/// How a lens renders its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LensViz {
    /// Rows and columns.
    Table,
    /// One line per row: the first column (or the first `columns` entry)
    /// is the headline, the rest are secondary.
    List,
    /// A single value: the first column of the first row.
    Number,
    /// The first column of the first row, rendered as markdown.
    Markdown,
    /// Bars: `chart.x` labels, `chart.y` values.
    Bar,
    /// A line over `chart.x` (a time or number), `chart.y` values, one
    /// line per `chart.series` value when set.
    Line,
    /// Nested rectangles sized by `chart.size`, labelled by `chart.label`,
    /// grouped (and coloured) by `chart.group`.
    Treemap,
    /// Other lenses (`children`), stacked, each given the params it
    /// declares from this lens's params.
    Grid,
}

/// Which result columns a chart viz draws from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensChart {
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

impl LensChart {
    /// Every column this chart names.
    fn columns(&self) -> Vec<&String> {
        [
            &self.x,
            &self.y,
            &self.series,
            &self.label,
            &self.size,
            &self.group,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Launcher (Cmd+K) sections a lens can be listed under. Mirrors the
/// renderer's `PageCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum LauncherCategory {
    Work,
    Code,
    Git,
    Activity,
    Knowledge,
    Data,
    Lenses,
    System,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherFile {
    category: LauncherCategory,
}

/// A page a column value can link to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum LensLinkKind {
    /// `task:<id>`; the value is a task id.
    Task,
    /// `file:<path>`; the value is a repo-relative path.
    File,
    /// `wiki:<slug>`.
    Wiki,
    /// The effort's diff view; the value is an effort id.
    EffortDiff,
    /// A git commit; the value is a sha.
    Commit,
    /// A metric's page; the value is a metric key.
    Metric,
}

/// Makes a column's cells link to a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensLink {
    pub kind: LensLinkKind,
    /// Result column holding the target id. Defaults to the column itself.
    #[serde(default)]
    pub from: Option<String>,
    /// For `file`: result column holding a line number to open at.
    #[serde(default)]
    pub line: Option<String>,
}

/// How one result column is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensColumn {
    /// Result column name.
    pub key: String,
    /// Header text; defaults to `key`.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub link: Option<LensLink>,
}

/// A value the viewer (or an agent) can set when running the lens,
/// bound into the query as `:name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensParam {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Used when the caller doesn't supply the param.
    #[serde(default)]
    pub default: Option<SqlCell>,
}

/// A lens file as written on disk (`lenses/<slug>.yaml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LensFile {
    title: String,
    #[serde(default)]
    description: String,
    query: String,
    #[serde(default = "default_viz")]
    viz: LensViz,
    #[serde(default)]
    params: Vec<LensParam>,
    #[serde(default)]
    columns: Vec<LensColumn>,
    /// Shown instead of an empty result.
    #[serde(default)]
    empty: Option<String>,
    #[serde(default)]
    chart: Option<LensChart>,
    /// For `grid`: lens slugs in this extension, or `<ext>/<slug>` ids.
    #[serde(default)]
    children: Vec<String>,
    #[serde(default)]
    launcher: Option<LauncherFile>,
    /// Keep it out of the launcher (e.g. a lens only a slot shows).
    #[serde(default)]
    hidden: bool,
}

fn default_viz() -> LensViz {
    LensViz::Table
}

/// `extension.yaml` as written on disk.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionFile {
    name: String,
    #[serde(default)]
    description: String,
    /// Parsed separately by [`crate::extension_sources::parse_sources`] so
    /// one bad source doesn't fail the whole manifest.
    #[serde(default)]
    sources: Option<serde_yaml::Value>,
    #[serde(default)]
    slots: Vec<SlotFile>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotFile {
    slot: String,
    lens: String,
}

/// Places in core pages an extension can mount a lens, and the params
/// each binds. A mounted lens must declare every one of them.
pub const SLOTS: &[(&str, &[&str])] = &[
    ("effort-review", &["effort_id"]),
    ("task-detail", &["task_id"]),
    ("thread", &["thread_id"]),
];

/// A lens an extension mounts into a core page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensSlot {
    /// Which page, from [`SLOTS`]: `effort-review` (an effort's diff
    /// view, binds `:effort_id`), `task-detail` (`:task_id`) or `thread`
    /// (`:thread_id`).
    pub slot: String,
    pub lens_id: String,
}

/// A loaded lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Lens {
    /// `<extension>/<slug>`: the stable id used by `lens:` page refs,
    /// `run_lens` and `get_lens`.
    pub id: String,
    pub extension: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub query: String,
    pub viz: LensViz,
    pub params: Vec<LensParam>,
    pub columns: Vec<LensColumn>,
    pub empty: Option<String>,
    /// Columns a chart viz draws from.
    pub chart: Option<LensChart>,
    /// For `grid`: child lens ids.
    pub children: Vec<String>,
    /// Launcher section; `None` = "Lenses".
    pub launcher_category: Option<LauncherCategory>,
    /// Not listed in the launcher.
    pub hidden: bool,
    /// Repo-relative path of the lens file.
    pub path: String,
}

/// A loaded extension and anything wrong with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub name: String,
    pub description: String,
    /// Repo-relative path of the extension folder.
    pub path: String,
    /// Problems found while loading; empty when healthy. A lens that
    /// failed to load is listed here and missing from `lenses`.
    pub errors: Vec<String>,
    pub lenses: Vec<Lens>,
    /// Where it was installed from, for extensions added with
    /// `install_extension`; `None` for ones written in this repo.
    pub source: Option<ExtensionSource>,
    /// Declared data sources (valid ones; invalid ones are in `errors`).
    pub sources: Vec<crate::extension_sources::SourceSpec>,
    /// `project` (in `oxplow/extensions/`) or `bundled` (ships with oxplow,
    /// read-only).
    pub origin: String,
    /// Lenses mounted into core pages.
    pub slots: Vec<LensSlot>,
    /// False when `.oxplow/project.yaml` disables it; a disabled
    /// extension has no lenses, slots or sources.
    pub enabled: bool,
}

/// Provenance of an installed extension, kept in its `source.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionSource {
    /// The git URL it was cloned from.
    pub git: String,
    /// The branch, tag or commit asked for; `None` = the remote's default branch.
    pub git_ref: Option<String>,
    /// The commit actually installed.
    pub sha: String,
}

/// File recording an installed extension's [`ExtensionSource`].
pub const SOURCE_FILE: &str = "source.yaml";

/// The result of running a lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensRun {
    pub lens: Lens,
    /// The parameter values actually used (supplied or default).
    pub params: BTreeMap<String, SqlCell>,
    pub result: SqlQueryResult,
}

/// Load bundled extensions plus every project extension under
/// `root/oxplow/extensions/`, sorted by name. A project extension using a
/// bundled name is listed with an error and never shadows the bundled one.
pub fn load_extensions(root: &Path) -> Vec<Extension> {
    let mut out: Vec<Extension> = crate::bundled_extensions::BUNDLED
        .iter()
        .map(|b| {
            load_one(
                &Embedded(b),
                b.name,
                &format!("bundled:{}", b.name),
                "bundled",
            )
        })
        .collect();
    if let Ok(entries) = std::fs::read_dir(root.join(EXTENSIONS_DIR)) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        for n in names {
            let rel = format!("{EXTENSIONS_DIR}/{n}");
            if crate::bundled_extensions::is_reserved(&n) {
                out.push(Extension {
                    errors: vec![format!(
                        "{rel}: the name `{n}` is reserved for an extension that ships with oxplow; rename the folder"
                    )],
                    ..empty_extension(&n, &rel, "project")
                });
                continue;
            }
            out.push(load_one(&Disk(root.join(&rel)), &n, &rel, "project"));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    let disabled = oxplow_config::disabled_extensions(root);
    out.into_iter()
        .map(|e| apply_disabled(e, &disabled))
        .collect()
}

/// Where an extension's files come from.
trait ExtensionFiles {
    /// Contents of a file, by path inside the extension folder.
    fn read(&self, rel: &str) -> Option<String>;
    /// File names directly inside `dir` (e.g. `lenses`).
    fn list(&self, dir: &str) -> Vec<String>;
}

struct Disk(std::path::PathBuf);

impl ExtensionFiles for Disk {
    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(rel)).ok()
    }
    fn list(&self, dir: &str) -> Vec<String> {
        std::fs::read_dir(self.0.join(dir))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct Embedded(&'static crate::bundled_extensions::BundledExtension);

impl ExtensionFiles for Embedded {
    fn read(&self, rel: &str) -> Option<String> {
        self.0
            .files
            .iter()
            .find(|(p, _)| *p == rel)
            .map(|(_, c)| (*c).to_string())
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{dir}/");
        self.0
            .files
            .iter()
            .filter_map(|(p, _)| p.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_string)
            .collect()
    }
}

fn empty_extension(name: &str, path: &str, origin: &str) -> Extension {
    Extension {
        name: name.to_string(),
        description: String::new(),
        path: path.to_string(),
        errors: Vec::new(),
        lenses: Vec::new(),
        source: None,
        sources: Vec::new(),
        origin: origin.to_string(),
        slots: Vec::new(),
        enabled: true,
    }
}

/// Load one extension. Always returns an `Extension`; problems go in
/// `errors`. `rel` is how its paths are shown (a repo-relative folder,
/// or `bundled:<name>`).
fn load_one(files: &dyn ExtensionFiles, name: &str, rel: &str, origin: &str) -> Extension {
    let mut ext = empty_extension(name, rel, origin);

    let Some(manifest) = files.read("extension.yaml") else {
        ext.errors.push(format!("{rel}: missing extension.yaml"));
        return ext;
    };
    let slot_files = match serde_yaml::from_str::<ExtensionFile>(&manifest) {
        Ok(m) if m.name != name => {
            ext.errors.push(format!(
                "{rel}/extension.yaml: name `{}` must match its folder `{name}`",
                m.name
            ));
            return ext;
        }
        Ok(m) => {
            ext.description = m.description;
            if let Some(v) = m.sources {
                let (sources, errors) = crate::extension_sources::parse_sources(name, &v);
                ext.sources = sources;
                ext.errors.extend(
                    errors
                        .into_iter()
                        .map(|e| format!("{rel}/extension.yaml: {e}")),
                );
            }
            m.slots
        }
        Err(e) => {
            ext.errors.push(format!("{rel}/extension.yaml: {e}"));
            return ext;
        }
    };
    if let Some(text) = files.read(SOURCE_FILE) {
        match serde_yaml::from_str::<ExtensionSource>(&text) {
            Ok(src) => ext.source = Some(src),
            Err(e) => ext.errors.push(format!("{rel}/{SOURCE_FILE}: {e}")),
        }
    }

    let mut lens_files: Vec<String> = files
        .list("lenses")
        .into_iter()
        .filter(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
        .collect();
    lens_files.sort();
    for file in lens_files {
        let slug = file
            .trim_end_matches(".yaml")
            .trim_end_matches(".yml")
            .to_string();
        let lens_rel = format!("{rel}/lenses/{file}");
        let parsed = files
            .read(&format!("lenses/{file}"))
            .ok_or_else(|| "unreadable".to_string())
            .and_then(|t| serde_yaml::from_str::<LensFile>(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(l) => ext.lenses.push(Lens {
                id: format!("{name}/{slug}"),
                extension: name.to_string(),
                slug,
                title: l.title,
                description: l.description,
                query: l.query,
                viz: l.viz,
                params: l.params,
                columns: l.columns,
                empty: l.empty,
                chart: l.chart,
                children: l
                    .children
                    .into_iter()
                    .map(|c| {
                        if c.contains('/') {
                            c
                        } else {
                            format!("{name}/{c}")
                        }
                    })
                    .collect(),
                launcher_category: l.launcher.map(|la| la.category),
                hidden: l.hidden,
                path: lens_rel,
            }),
            Err(e) => ext.errors.push(format!("{lens_rel}: {e}")),
        }
    }

    // Drop lenses whose viz lacks what it needs, so every loaded lens renders.
    let ids: Vec<String> = ext.lenses.iter().map(|l| l.id.clone()).collect();
    let mut bad = Vec::new();
    for l in &ext.lenses {
        if let Some(problem) = shape_problem(l, &ids) {
            ext.errors.push(format!("{}: {problem}", l.path));
            bad.push(l.id.clone());
        }
    }
    ext.lenses.retain(|l| !bad.contains(&l.id));

    for s in slot_files {
        let slot_params = SLOTS.iter().find(|(n, _)| *n == s.slot).map(|(_, p)| *p);
        let lens = ext.lenses.iter().find(|l| l.slug == s.lens);
        let missing: Vec<&str> = match (slot_params, lens) {
            (Some(params), Some(l)) => params
                .iter()
                .filter(|p| !l.params.iter().any(|lp| lp.name == **p))
                .copied()
                .collect(),
            _ => vec![],
        };
        if slot_params.is_none() {
            ext.errors.push(format!(
                "{rel}/extension.yaml: unknown slot `{}` (known: {})",
                s.slot,
                SLOTS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ));
        } else if !missing.is_empty() {
            ext.errors.push(format!(
                "{rel}/extension.yaml: slot `{}` passes {}, which lens `{}` doesn't declare in `params`",
                s.slot,
                missing.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", "),
                s.lens
            ));
        } else if lens.is_none() {
            ext.errors.push(format!(
                "{rel}/extension.yaml: slot `{}` mounts lens `{}`, which isn't in lenses/",
                s.slot, s.lens
            ));
        } else {
            ext.slots.push(LensSlot {
                slot: s.slot,
                lens_id: format!("{name}/{}", s.lens),
            });
        }
    }
    ext
}

/// Why `lens` can't render with its viz, if it can't.
fn shape_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    let chart = lens.chart.clone().unwrap_or_default();
    let need = |fields: &[(&str, &Option<String>)]| -> Option<String> {
        let missing: Vec<&str> = fields
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        (!missing.is_empty()).then(|| {
            format!(
                "viz `{:?}` needs `chart: {{ {} }}`",
                lens.viz,
                missing
                    .iter()
                    .map(|m| format!("{m}: <column>"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .to_lowercase()
        })
    };
    match lens.viz {
        LensViz::Bar | LensViz::Line => need(&[("x", &chart.x), ("y", &chart.y)]),
        LensViz::Treemap => need(&[("label", &chart.label), ("size", &chart.size)]),
        LensViz::Grid if lens.children.is_empty() => {
            Some("viz `grid` needs `children: [lens, ...]`".into())
        }
        LensViz::Grid => lens
            .children
            .iter()
            .find(|c| {
                c.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str())
                    && !ids_in_extension.contains(c)
            })
            .map(|c| format!("child lens `{c}` isn't in this extension's lenses/")),
        _ => None,
    }
}

/// Clear what a disabled extension contributes; it stays listed so
/// Settings can turn it back on.
fn apply_disabled(mut ext: Extension, disabled: &[String]) -> Extension {
    if disabled.iter().any(|d| d == &ext.name) {
        ext.enabled = false;
        ext.lenses.clear();
        ext.slots.clear();
        ext.sources.clear();
    }
    ext
}

fn disabled_error(name: &str) -> DomainError {
    DomainError::Invalid(format!(
        "extension `{name}` is disabled in .oxplow/project.yaml (extensions.disabled); enable it in Settings → Extensions"
    ))
}

/// Load the extension named `name`, if its folder exists.
fn load_named(root: &Path, name: &str) -> Result<Extension, DomainError> {
    if oxplow_config::disabled_extensions(root)
        .iter()
        .any(|d| d == name)
    {
        return Err(disabled_error(name));
    }
    if let Some(b) = crate::bundled_extensions::find(name) {
        return Ok(load_one(
            &Embedded(b),
            b.name,
            &format!("bundled:{}", b.name),
            "bundled",
        ));
    }
    if name.is_empty()
        || name.contains(['/', '\\', '.'])
        || !root.join(EXTENSIONS_DIR).join(name).is_dir()
    {
        return Err(DomainError::NotFound);
    }
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    Ok(load_one(&Disk(root.join(&rel)), name, &rel, "project"))
}

/// Find one lens by `<extension>/<slug>`.
pub fn find_lens(root: &Path, id: &str) -> Result<Lens, DomainError> {
    let (ext, slug) = id.split_once('/').ok_or(DomainError::NotFound)?;
    load_named(root, ext)?
        .lenses
        .into_iter()
        .find(|l| l.slug == slug)
        .ok_or(DomainError::NotFound)
}

/// Run a lens: bind supplied params over defaults and query the
/// semantic layer. Unknown params are rejected, so a typo doesn't
/// silently fall back to a default.
pub async fn run_lens(
    layer: &SemanticLayer,
    root: &Path,
    id: &str,
    params: BTreeMap<String, SqlCell>,
) -> Result<LensRun, DomainError> {
    let lens = find_lens(root, id)?;
    execute(layer, lens, params).await.map_err(|e| match e {
        DomainError::Invalid(m) => DomainError::Invalid(explain_unsynced(root, &m)),
        other => other,
    })
}

/// A lens reading a source entity before its first sync fails with
/// SQLite's bare "no such table: v_x". Say which source to run instead.
fn explain_unsynced(root: &Path, message: &str) -> String {
    let Some(rest) = message.split("no such table: ").nth(1) else {
        return message.to_string();
    };
    let view: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    for ext in load_extensions(root) {
        for source in &ext.sources {
            if source.entities.iter().any(|e| e.view == view) {
                let lens = message.split(':').next().unwrap_or("lens");
                return format!(
                    "{lens}: reads `{view}`, which hasn't synced yet. Run source `{}/{}` \
                     (Settings → Extensions → Approve & Run, or Sync Now).",
                    ext.name, source.id
                );
            }
        }
    }
    message.to_string()
}

async fn execute(
    layer: &SemanticLayer,
    lens: Lens,
    supplied: BTreeMap<String, SqlCell>,
) -> Result<LensRun, DomainError> {
    if let Some(unknown) = supplied
        .keys()
        .find(|k| !lens.params.iter().any(|p| &p.name == *k))
    {
        let known: Vec<&str> = lens.params.iter().map(|p| p.name.as_str()).collect();
        return Err(DomainError::Invalid(format!(
            "lens {}: unknown param `{unknown}` (params: {})",
            lens.id,
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        )));
    }
    let mut params = BTreeMap::new();
    for p in &lens.params {
        let v = supplied
            .get(&p.name)
            .cloned()
            .or_else(|| p.default.clone())
            .unwrap_or(SqlCell::Null(()));
        params.insert(p.name.clone(), v);
    }
    let named: Vec<(String, SqlCell)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let result = layer
        .query_sql_named(&lens.query, named, None)
        .await
        .map_err(|e| match e {
            DomainError::Invalid(m) => DomainError::Invalid(format!("lens {}: {m}", lens.id)),
            other => other,
        })?;
    Ok(LensRun {
        lens,
        params,
        result,
    })
}

/// Load one extension and dry-run every lens with default params,
/// reporting query failures and `columns` keys the query doesn't
/// return as errors.
pub async fn validate_extension(
    layer: &SemanticLayer,
    root: &Path,
    name: &str,
) -> Result<Extension, DomainError> {
    let mut ext = load_named(root, name)?;
    for lens in ext.lenses.clone() {
        let id = lens.id.clone();
        match execute(layer, lens, BTreeMap::new()).await {
            Err(e) => ext.errors.push(explain_unsynced(
                root,
                &e.to_string().replacen("invalid value: ", "", 1),
            )),
            Ok(run) => {
                let cols = &run.result.columns;
                let chart_cols = run
                    .lens
                    .chart
                    .as_ref()
                    .map(|c| c.columns())
                    .unwrap_or_default();
                for k in chart_cols {
                    if !cols.contains(k) {
                        ext.errors.push(format!(
                            "lens {id}: chart column `{k}` isn't in the query result (columns: {})",
                            cols.join(", ")
                        ));
                    }
                }
                for c in &run.lens.columns {
                    let mut keys = vec![&c.key];
                    if let Some(link) = c.link.as_ref() {
                        keys.extend(link.from.iter());
                        keys.extend(link.line.iter());
                    }
                    for k in keys {
                        if !cols.contains(k) {
                            ext.errors.push(format!(
                                "lens {id}: column `{k}` isn't in the query result (columns: {})",
                                cols.join(", ")
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(ext)
}

/// Install an extension from a git repo whose root holds `extension.yaml`:
/// clone it (inside `.oxplow/tmp/`, per workspace isolation), copy it to
/// `oxplow/extensions/<name>/` without `.git`, and record its source.
/// Refuses to overwrite an existing folder; use [`update_extension`].
pub fn install_extension(
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
) -> Result<Extension, DomainError> {
    install_from_git(root, git_url, git_ref, None)
}

/// Re-install an installed extension from its recorded source (same URL
/// and ref), picking up new commits.
pub fn update_extension(root: &Path, name: &str) -> Result<Extension, DomainError> {
    let existing = load_named(root, name)?;
    let source = existing.source.ok_or_else(|| {
        DomainError::Invalid(format!(
            "extension `{name}` wasn't installed from git (no {SOURCE_FILE}); edit it in place instead"
        ))
    })?;
    install_from_git(root, &source.git, source.git_ref.as_deref(), Some(name))
}

/// Lowercase letters, digits and single dashes — safe as a folder name
/// and a lens-id prefix.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Clone, validate, then copy into place. `replacing` names the
/// installed extension an update must match; the old folder is removed
/// only after the new clone validated, so a failed update changes nothing.
fn install_from_git(
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    replacing: Option<&str>,
) -> Result<Extension, DomainError> {
    let invalid = |m: String| DomainError::Invalid(m);
    if git_url.starts_with('-') || git_ref.is_some_and(|r| r.starts_with('-')) {
        return Err(invalid("git URL and ref may not start with `-`".into()));
    }
    let storage = |e: std::io::Error| DomainError::Storage(format!("extension install: {e}"));

    let tmp_parent = root.join(".oxplow").join("tmp");
    std::fs::create_dir_all(&tmp_parent).map_err(storage)?;
    let tmp = tempfile::Builder::new()
        .prefix("ext-install-")
        .tempdir_in(&tmp_parent)
        .map_err(storage)?;
    let clone = tmp.path().join("repo");
    let clone_str = clone.to_string_lossy().to_string();
    run_git(tmp.path(), &["clone", "--quiet", "--", git_url, &clone_str])
        .map_err(|e| invalid(format!("couldn't clone {git_url}: {e}")))?;
    if let Some(r) = git_ref {
        run_git(&clone, &["checkout", "--quiet", r])
            .map_err(|e| invalid(format!("couldn't check out `{r}` in {git_url}: {e}")))?;
    }
    let sha = run_git(&clone, &["rev-parse", "HEAD"])
        .map_err(|e| invalid(format!("couldn't read the cloned commit: {e}")))?;

    let manifest = std::fs::read_to_string(clone.join("extension.yaml")).map_err(|_| {
        invalid(format!(
            "{git_url} has no extension.yaml at its root, so it isn't an oxplow extension"
        ))
    })?;
    let manifest: ExtensionFile = serde_yaml::from_str(&manifest)
        .map_err(|e| invalid(format!("{git_url}: extension.yaml: {e}")))?;
    let name = manifest.name;
    if crate::bundled_extensions::is_reserved(&name) {
        return Err(invalid(format!(
            "`{name}` is the name of an extension that ships with oxplow; it can't be installed over"
        )));
    }
    if !is_valid_name(&name) {
        return Err(invalid(format!(
            "extension name `{name}` must be lowercase letters, digits and single dashes"
        )));
    }
    if let Some(expected) = replacing {
        if name != expected {
            return Err(invalid(format!(
                "{git_url} now names itself `{name}`, not `{expected}`; install it separately"
            )));
        }
    }

    let target = root.join(EXTENSIONS_DIR).join(&name);
    match (target.exists(), replacing) {
        (true, None) => {
            return Err(invalid(format!(
                "extension `{name}` is already installed at {EXTENSIONS_DIR}/{name}; use update_extension"
            )))
        }
        (true, Some(_)) => std::fs::remove_dir_all(&target).map_err(storage)?,
        (false, _) => {}
    }
    copy_tree_without_git(&clone, &target).map_err(storage)?;

    let source = ExtensionSource {
        git: git_url.to_string(),
        git_ref: git_ref.map(str::to_string),
        sha,
    };
    let yaml = serde_yaml::to_string(&source)
        .map_err(|e| DomainError::Storage(format!("extension install: {e}")))?;
    std::fs::write(target.join(SOURCE_FILE), yaml).map_err(storage)?;
    Ok(load_named(root, &name)
        .unwrap_or_else(|_| empty_extension(&name, &format!("{EXTENSIONS_DIR}/{name}"), "project")))
}

/// Copy regular files and directories from `from` to `to`, skipping
/// `.git` and anything that isn't a plain file or directory (symlinks
/// could point outside the extension).
fn copy_tree_without_git(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in walkdir::WalkDir::new(from)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry.map_err(std::io::Error::other)?;
        let rel = entry
            .path()
            .strip_prefix(from)
            .map_err(std::io::Error::other)?;
        let dest = to.join(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// What the Explore Data page saves as a new lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NewLens {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub query: String,
    pub viz: LensViz,
}

/// Write a new lens file `oxplow/extensions/<extension>/lenses/<slug>.yaml`,
/// creating the extension (with a minimal `extension.yaml`) if needed.
/// Refuses to overwrite a lens, and refuses git-installed extensions
/// (their files are replaced on update).
pub fn save_lens(
    root: &Path,
    extension: &str,
    slug: &str,
    lens: NewLens,
) -> Result<Lens, DomainError> {
    let invalid = |m: String| DomainError::Invalid(m);
    let storage = |e: std::io::Error| DomainError::Storage(format!("save lens: {e}"));
    if !is_valid_name(extension) {
        return Err(invalid(format!(
            "extension name `{extension}` must be lowercase letters, digits and single dashes"
        )));
    }
    if !is_valid_name(slug) {
        return Err(invalid(format!(
            "lens slug `{slug}` must be lowercase letters, digits and single dashes"
        )));
    }
    if crate::bundled_extensions::is_reserved(extension) {
        return Err(invalid(format!(
            "`{extension}` is a bundled extension (read-only); save to another extension"
        )));
    }
    let dir = root.join(EXTENSIONS_DIR).join(extension);
    if dir.join(SOURCE_FILE).exists() {
        return Err(invalid(format!(
            "`{extension}` is an installed extension (its files are replaced on update); save to another extension"
        )));
    }
    let file = dir.join("lenses").join(format!("{slug}.yaml"));
    if file.exists() {
        return Err(invalid(format!("lens `{extension}/{slug}` already exists")));
    }
    std::fs::create_dir_all(file.parent().unwrap_or(&dir)).map_err(storage)?;
    let manifest = dir.join("extension.yaml");
    if !manifest.exists() {
        std::fs::write(&manifest, format!("name: {extension}\ndescription: \"\"\n"))
            .map_err(storage)?;
    }
    let body = serde_yaml::to_string(&SavedLensFile {
        title: &lens.title,
        description: &lens.description,
        query: &lens.query,
        viz: lens.viz,
    })
    .map_err(|e| DomainError::Storage(format!("save lens: {e}")))?;
    std::fs::write(&file, body).map_err(storage)?;
    find_lens(root, &format!("{extension}/{slug}"))
}

/// The on-disk shape [`save_lens`] writes (a subset of [`LensFile`]).
#[derive(Serialize)]
struct SavedLensFile<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    description: &'a str,
    query: &'a str,
    viz: LensViz,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use std::fs;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    const EXT: &str = "name: review\ndescription: Review helpers\n";
    const LENS: &str = r#"
title: Tasks by status
description: Every task with a given status.
params:
  - { name: status, label: Status, default: in_progress }
query: |
  SELECT id, title FROM v_task WHERE status = :status ORDER BY id
viz: table
columns:
  - { key: title, label: Task, link: { kind: task, from: id } }
empty: No tasks.
"#;

    async fn layer() -> SemanticLayer {
        SemanticLayer::new(Database::in_memory())
    }

    /// Project extensions only (bundled ones are always present).
    fn project_extensions(root: &Path) -> Vec<Extension> {
        load_extensions(root)
            .into_iter()
            .filter(|e| e.origin == "project")
            .collect()
    }

    #[test]
    fn no_extensions_dir_means_no_project_extensions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_extensions(dir.path()).is_empty());
    }

    #[test]
    fn loads_an_extension_and_its_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );

        let exts = project_extensions(dir.path());
        assert_eq!(exts.len(), 1);
        let e = &exts[0];
        assert_eq!(e.name, "review");
        assert_eq!(e.description, "Review helpers");
        assert_eq!(e.path, "oxplow/extensions/review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.lenses.len(), 1);
        let l = &e.lenses[0];
        assert_eq!(l.id, "review/by-status");
        assert_eq!(l.slug, "by-status");
        assert_eq!(l.title, "Tasks by status");
        assert_eq!(l.viz, LensViz::Table);
        assert_eq!(l.path, "oxplow/extensions/review/lenses/by-status.yaml");
        assert_eq!(
            l.params[0].default,
            Some(SqlCell::Text("in_progress".into()))
        );
        assert_eq!(
            l.columns[0].link,
            Some(LensLink {
                kind: LensLinkKind::Task,
                from: Some("id".into()),
                line: None,
            })
        );
    }

    #[test]
    fn a_broken_extension_reports_errors_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/good.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/bad.yaml",
            "title: x\nqueery: SELECT 1\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/nomanifest/lenses/a.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/misnamed/extension.yaml",
            "name: other\n",
        );

        let exts = project_extensions(dir.path());
        let by = |n: &str| {
            exts.iter()
                .find(|e| e.name == n)
                .unwrap_or_else(|| panic!("{n} missing"))
        };

        let review = by("review");
        assert_eq!(
            review
                .lenses
                .iter()
                .map(|l| l.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["good"]
        );
        assert_eq!(review.errors.len(), 1);
        assert!(
            review.errors[0].contains("lenses/bad.yaml"),
            "{:?}",
            review.errors
        );

        let nm = by("nomanifest");
        assert!(nm.errors[0].contains("extension.yaml"), "{:?}", nm.errors);
        assert!(nm.lenses.is_empty());

        let mis = by("misnamed");
        assert!(
            mis.errors[0].contains("must match its folder"),
            "{:?}",
            mis.errors
        );
    }

    #[test]
    fn find_lens_by_id() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        assert_eq!(
            find_lens(dir.path(), "review/by-status").unwrap().title,
            "Tasks by status"
        );
        assert!(matches!(
            find_lens(dir.path(), "review/nope"),
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            find_lens(dir.path(), "nope"),
            Err(DomainError::NotFound)
        ));
    }

    #[tokio::test]
    async fn run_lens_binds_defaults_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/echo.yaml",
            "title: Echo\nparams:\n  - { name: a, default: 1 }\n  - { name: b, default: two }\nquery: SELECT :a AS a, :b AS b\n",
        );
        let sl = layer().await;

        let run = run_lens(&sl, dir.path(), "review/echo", BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(run.result.columns, vec!["a", "b"]);
        assert_eq!(
            serde_json::to_value(&run.result.rows).unwrap(),
            serde_json::json!([[1, "two"]])
        );
        assert_eq!(run.params.get("b"), Some(&SqlCell::Text("two".into())));

        let mut over = BTreeMap::new();
        over.insert("b".to_string(), SqlCell::Text("three".into()));
        let run = run_lens(&sl, dir.path(), "review/echo", over)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&run.result.rows).unwrap(),
            serde_json::json!([[1, "three"]])
        );
    }

    #[tokio::test]
    async fn run_lens_rejects_unknown_params_and_reports_query_errors() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/broken.yaml",
            "title: Broken\nquery: SELECT nope FROM v_nothing\n",
        );
        let sl = layer().await;

        let mut bad = BTreeMap::new();
        bad.insert("stauts".to_string(), SqlCell::Text("done".into()));
        let err = run_lens(&sl, dir.path(), "review/by-status", bad)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("stauts")),
            "{err:?}"
        );

        let err = run_lens(&sl, dir.path(), "review/broken", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("review/broken")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn validate_dry_runs_every_lens() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/broken.yaml",
            "title: Broken\nquery: SELECT nope FROM v_nothing\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/badcol.yaml",
            "title: Bad column\nquery: SELECT 1 AS n\ncolumns:\n  - { key: missing }\n",
        );
        let sl = layer().await;

        let e = validate_extension(&sl, dir.path(), "review").await.unwrap();
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("review/broken")));
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("review/badcol") && m.contains("missing")));

        assert!(matches!(
            validate_extension(&sl, dir.path(), "nope").await,
            Err(DomainError::NotFound)
        ));
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }

    /// A git repo shaped like a published extension.
    fn published_repo(lens_title: &str) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            "extension.yaml",
            "name: shared\ndescription: Shared lenses\n",
        );
        write(
            repo.path(),
            "lenses/count.yaml",
            &format!("title: {lens_title}\nquery: SELECT 1 AS n\nviz: number\n"),
        );
        git(repo.path(), &["init", "-q", "-b", "main"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        repo
    }

    #[test]
    fn installs_from_git_and_records_the_source() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        let url = repo.path().to_string_lossy().to_string();

        let ext = install_extension(project.path(), &url, None).unwrap();
        assert_eq!(ext.name, "shared");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.lenses[0].title, "Count");
        let src = ext.source.clone().unwrap();
        assert_eq!(src.git, url);
        assert_eq!(src.git_ref, None);
        assert_eq!(src.sha.len(), 40);

        let dir = project.path().join("oxplow/extensions/shared");
        assert!(dir.join("lenses/count.yaml").is_file());
        assert!(
            !dir.join(".git").exists(),
            "the clone's .git must not be copied"
        );
        assert!(dir.join(SOURCE_FILE).is_file());
        // Loading later still reports the source.
        assert_eq!(project_extensions(project.path())[0].source, Some(src));
        // The temporary clone is gone.
        let tmp = project.path().join(".oxplow/tmp");
        assert!(!tmp.exists() || std::fs::read_dir(&tmp).unwrap().next().is_none());
    }

    #[test]
    fn refuses_to_overwrite_and_update_pulls_new_commits() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        let url = repo.path().to_string_lossy().to_string();
        install_extension(project.path(), &url, None).unwrap();

        let err = install_extension(project.path(), &url, None).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("already")),
            "{err:?}"
        );

        write(
            repo.path(),
            "lenses/count.yaml",
            "title: Count v2\nquery: SELECT 2 AS n\nviz: number\n",
        );
        git(repo.path(), &["commit", "-q", "-am", "v2"]);
        let ext = update_extension(project.path(), "shared").unwrap();
        assert_eq!(ext.lenses[0].title, "Count v2");
    }

    #[test]
    fn update_only_applies_to_installed_extensions() {
        let project = tempfile::tempdir().unwrap();
        write(
            project.path(),
            "oxplow/extensions/local/extension.yaml",
            "name: local\n",
        );
        let err = update_extension(project.path(), "local").unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("wasn't installed")),
            "{err:?}"
        );
        assert!(matches!(
            update_extension(project.path(), "nope"),
            Err(DomainError::NotFound)
        ));
    }

    #[test]
    fn rejects_repos_that_are_not_extensions() {
        let project = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), "README.md", "hi");
        git(repo.path(), &["init", "-q", "-b", "main"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let err =
            install_extension(project.path(), &repo.path().to_string_lossy(), None).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("extension.yaml")),
            "{err:?}"
        );
        assert!(!project.path().join("oxplow/extensions").exists());

        let err = install_extension(project.path(), "/definitely/not/a/repo", None).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("clone")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_unsafe_extension_names() {
        let project = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), "extension.yaml", "name: ../escape\n");
        git(repo.path(), &["init", "-q", "-b", "main"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let err =
            install_extension(project.path(), &repo.path().to_string_lossy(), None).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("name")),
            "{err:?}"
        );
    }

    #[test]
    fn save_lens_creates_the_extension_and_refuses_overwrites() {
        let project = tempfile::tempdir().unwrap();
        let new = || NewLens {
            title: "Open Tasks".into(),
            description: "From Explore Data".into(),
            query: "SELECT id, title FROM v_task".into(),
            viz: LensViz::Table,
        };
        let lens = save_lens(project.path(), "mine", "open-tasks", new()).unwrap();
        assert_eq!(lens.id, "mine/open-tasks");
        assert_eq!(lens.title, "Open Tasks");
        assert!(project
            .path()
            .join("oxplow/extensions/mine/extension.yaml")
            .is_file());
        let ext = &project_extensions(project.path())[0];
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);

        let err = save_lens(project.path(), "mine", "open-tasks", new()).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("already exists")),
            "{err:?}"
        );
        let err = save_lens(project.path(), "mine", "Bad Slug", new()).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("slug")),
            "{err:?}"
        );
    }

    #[test]
    fn save_lens_refuses_installed_extensions() {
        let project = tempfile::tempdir().unwrap();
        write(
            project.path(),
            "oxplow/extensions/shared/extension.yaml",
            "name: shared\n",
        );
        write(
            project.path(),
            "oxplow/extensions/shared/source.yaml",
            "git: x\ngitRef: null\nsha: abc\n",
        );
        let err = save_lens(
            project.path(),
            "shared",
            "x",
            NewLens {
                title: "X".into(),
                description: String::new(),
                query: "SELECT 1".into(),
                viz: LensViz::Number,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("installed")),
            "{err:?}"
        );
    }

    #[test]
    fn loads_declared_sources_and_reports_bad_ones_without_dropping_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            "name: review\nsources:\n  - id: gh\n    runtime: exec\n    entry: bin/sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n  - id: bad\n    runtime: python\n    entry: x\n    entities: []\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        let e = &project_extensions(dir.path())[0];
        assert_eq!(e.sources.len(), 1);
        assert_eq!(e.sources[0].entities[0].view, "v_review_pr");
        assert_eq!(e.errors.len(), 1, "{:?}", e.errors);
        assert!(
            e.errors[0].contains("extension.yaml") && e.errors[0].contains("runtime"),
            "{:?}",
            e.errors
        );
        assert_eq!(e.lenses.len(), 1);
    }

    #[tokio::test]
    async fn an_unsynced_source_entity_gets_a_helpful_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/gh/extension.yaml",
            "name: gh\nsources:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/gh/lenses/all.yaml",
            "title: All\nquery: SELECT number FROM v_gh_pr\n",
        );
        let sl = layer().await;
        let err = run_lens(&sl, dir.path(), "gh/all", BTreeMap::new())
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("v_gh_pr") && msg.contains("prs") && msg.contains("hasn't synced"),
            "{msg}"
        );
        let e = validate_extension(&sl, dir.path(), "gh").await.unwrap();
        assert!(e.errors[0].contains("hasn't synced"), "{:?}", e.errors);
    }

    #[tokio::test]
    async fn bundled_extensions_load_validate_and_mount_slots() {
        let dir = tempfile::tempdir().unwrap();
        let exts = load_extensions(dir.path());
        let review = exts
            .iter()
            .find(|e| e.name == "oxplow-review")
            .expect("bundled extension present");
        assert_eq!(review.origin, "bundled");
        assert!(review.errors.is_empty(), "{:?}", review.errors);
        assert!(review
            .lenses
            .iter()
            .any(|l| l.id == "oxplow-review/decisions"));
        assert!(review
            .slots
            .iter()
            .any(|s| s.slot == "effort-review" && s.lens_id == "oxplow-review/decisions"));
        assert!(review
            .slots
            .iter()
            .any(|s| s.slot == "effort-review" && s.lens_id == "oxplow-review/inferred-decisions"));
        // Every bundled lens's SQL runs against a real schema.
        let v = validate_extension(&layer().await, dir.path(), "oxplow-review")
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(
            find_lens(dir.path(), "oxplow-review/decisions")
                .unwrap()
                .title,
            "Decisions Made"
        );
    }

    #[test]
    fn bundled_names_are_reserved() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/oxplow-review/extension.yaml",
            "name: oxplow-review\n",
        );
        let exts = load_extensions(dir.path());
        let named: Vec<&Extension> = exts.iter().filter(|e| e.name == "oxplow-review").collect();
        assert_eq!(named.len(), 2);
        let project = named.iter().find(|e| e.origin == "project").unwrap();
        assert!(
            project.errors[0].contains("reserved"),
            "{:?}",
            project.errors
        );
        // The bundled one still wins lookups.
        assert_eq!(
            find_lens(dir.path(), "oxplow-review/decisions")
                .unwrap()
                .title,
            "Decisions Made"
        );
        let err = save_lens(
            dir.path(),
            "oxplow-review",
            "x",
            NewLens {
                title: "X".into(),
                description: String::new(),
                query: "SELECT 1".into(),
                viz: LensViz::Number,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("bundled")),
            "{err:?}"
        );
    }

    #[test]
    fn slots_must_name_a_known_slot_and_an_existing_lens() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/mine/extension.yaml",
            "name: mine\nslots:\n  - { slot: effort-review, lens: nope }\n  - { slot: sidebar, lens: a }\n  - { slot: effort-review, lens: a }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/mine/lenses/a.yaml",
            "title: A\nparams: [{ name: effort_id }]\nquery: SELECT :effort_id\n",
        );
        let e = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "mine")
            .unwrap();
        assert_eq!(e.slots.len(), 1);
        assert_eq!(e.slots[0].lens_id, "mine/a");
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("nope")));
        assert!(e.errors.iter().any(|m| m.contains("sidebar")));
    }

    /// The documented examples in `examples/extensions/` load cleanly, so
    /// a format change can't silently break what the guide tells people to
    /// copy.
    #[test]
    fn documented_examples_load_without_errors() {
        let examples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
        let names: Vec<String> = std::fs::read_dir(&examples)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        assert!(!names.is_empty());
        for name in names {
            let ext = load_one(&Disk(examples.join(&name)), &name, &name, "project");
            assert!(ext.errors.is_empty(), "{name}: {:?}", ext.errors);
        }
    }

    /// One project extension `x` with the given lens files and
    /// extension.yaml tail; returns its load result.
    fn load_x(files: &[(&str, &str)], manifest_tail: &str) -> (tempfile::TempDir, Extension) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            &format!("name: x\n{manifest_tail}"),
        );
        for (slug, body) in files {
            write(
                dir.path(),
                &format!("oxplow/extensions/x/lenses/{slug}.yaml"),
                body,
            );
        }
        let ext = project_extensions(dir.path()).remove(0);
        (dir, ext)
    }

    #[test]
    fn chart_lenses_parse_and_missing_fields_are_errors() {
        let (_d, ext) = load_x(
            &[
                ("visits", "title: V\nquery: SELECT 'a' AS day, 1 AS n\nviz: bar\nchart: { x: day, y: n }\n"),
                ("trend", "title: T\nquery: SELECT 1 AS at, 2 AS v, 'm' AS s\nviz: line\nchart: { x: at, y: v, series: s }\n"),
                ("map", "title: M\nquery: SELECT 'p' AS path, 3 AS churn, 'core' AS zone\nviz: treemap\nchart: { label: path, size: churn, group: zone }\n"),
                ("all", "title: All\nquery: SELECT 1\nviz: grid\nchildren: [visits, trend]\n"),
                ("nobar", "title: N\nquery: SELECT 1\nviz: bar\n"),
                ("badgrid", "title: G\nquery: SELECT 1\nviz: grid\nchildren: [nope]\n"),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        assert_eq!(lens("visits").unwrap().viz, LensViz::Bar);
        assert_eq!(
            lens("visits").unwrap().chart.as_ref().unwrap().y.as_deref(),
            Some("n")
        );
        assert_eq!(
            lens("trend")
                .unwrap()
                .chart
                .as_ref()
                .unwrap()
                .series
                .as_deref(),
            Some("s")
        );
        assert_eq!(
            lens("map")
                .unwrap()
                .chart
                .as_ref()
                .unwrap()
                .group
                .as_deref(),
            Some("zone")
        );
        assert_eq!(lens("all").unwrap().children, vec!["x/visits", "x/trend"]);
        assert!(lens("nobar").is_none() && lens("badgrid").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("nobar") && errs.contains("chart"), "{errs}");
        assert!(errs.contains("badgrid") && errs.contains("nope"), "{errs}");
    }

    #[tokio::test]
    async fn validate_checks_chart_and_link_columns_exist() {
        let (d, _) = load_x(
            &[
                ("a", "title: A\nquery: SELECT 'x' AS day, 1 AS n\nviz: bar\nchart: { x: day, y: missing }\n"),
                ("b", "title: B\nquery: SELECT 'f.rs' AS path\ncolumns:\n  - { key: path, link: { kind: file, line: ln } }\n"),
            ],
            "",
        );
        let v = validate_extension(&layer().await, d.path(), "x")
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("`missing`") && errs.contains("`ln`"),
            "{errs}"
        );
    }

    #[test]
    fn new_link_kinds_parse() {
        let (_d, ext) = load_x(
            &[(
                "l",
                "title: L\nquery: SELECT 1\ncolumns:\n  - { key: a, link: { kind: commit } }\n  - { key: b, link: { kind: metric } }\n  - { key: d, link: { kind: file, line: n } }\n",
            )],
            "",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let kinds: Vec<LensLinkKind> = ext.lenses[0]
            .columns
            .iter()
            .map(|c| c.link.as_ref().unwrap().kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                LensLinkKind::Commit,
                LensLinkKind::Metric,
                LensLinkKind::File
            ]
        );
        assert_eq!(
            ext.lenses[0].columns[2]
                .link
                .as_ref()
                .unwrap()
                .line
                .as_deref(),
            Some("n")
        );
    }

    #[test]
    fn slots_bind_params_the_lens_must_declare() {
        let task_lens = "title: T\nparams: [{ name: task_id }]\nquery: SELECT :task_id\n";
        let thread_lens = "title: Th\nparams: [{ name: thread_id }]\nquery: SELECT :thread_id\n";
        let (_d, ext) = load_x(
            &[("t", task_lens), ("th", thread_lens), ("plain", "title: P\nquery: SELECT 1\n")],
            "slots:\n  - { slot: task-detail, lens: t }\n  - { slot: thread, lens: th }\n  - { slot: task-detail, lens: plain }\n",
        );
        let mounted: Vec<(&str, &str)> = ext
            .slots
            .iter()
            .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
            .collect();
        assert_eq!(mounted, vec![("task-detail", "x/t"), ("thread", "x/th")]);
        let errs = ext.errors.join("\n");
        assert!(errs.contains("task_id") && errs.contains("plain"), "{errs}");
    }

    #[test]
    fn launcher_category_and_hidden_parse() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 1\nlauncher: { category: Activity }\n",
                ),
                ("b", "title: B\nquery: SELECT 1\nhidden: true\n"),
                (
                    "c",
                    "title: C\nquery: SELECT 1\nlauncher: { category: Nowhere }\n",
                ),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        assert_eq!(
            lens("a").unwrap().launcher_category,
            Some(LauncherCategory::Activity)
        );
        assert!(lens("b").unwrap().hidden);
        assert!(!lens("a").unwrap().hidden);
        assert!(lens("c").is_none());
        assert!(
            ext.errors.join("\n").contains("Nowhere"),
            "{:?}",
            ext.errors
        );
    }

    #[tokio::test]
    async fn disabled_extensions_load_empty_and_their_lenses_refuse_to_run() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [review, oxplow-review]\n",
        );
        let exts = load_extensions(dir.path());
        for name in ["review", "oxplow-review"] {
            let e = exts.iter().find(|e| e.name == name).unwrap();
            assert!(!e.enabled, "{name}");
            assert!(
                e.lenses.is_empty() && e.slots.is_empty() && e.sources.is_empty(),
                "{name}"
            );
        }
        let err = run_lens(
            &layer().await,
            dir.path(),
            "review/by-status",
            BTreeMap::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
    }
}
