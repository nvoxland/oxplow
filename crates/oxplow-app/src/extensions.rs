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

pub mod manifest_v2;
pub mod migrate_v1;
use manifest_v2::{at, key_line, line_under, ManifestV2};
pub use manifest_v2::{Intent, IntentExample, Sharing};

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

/// When a lens needs attention: its row count reaches `min_rows`, or the
/// first row's `column` goes `above` / `below` a threshold. Shown as a rail
/// badge when the lens is mounted in the `rail` slot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensAlert {
    pub min_rows: Option<i64>,
    pub column: Option<String>,
    pub above: Option<f64>,
    pub below: Option<f64>,
    /// Badge text; else the row count or the column's name.
    pub label: Option<String>,
}

/// `alert:` as written in a lens file (snake_case keys).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlertFile {
    min_rows: Option<i64>,
    column: Option<String>,
    above: Option<f64>,
    below: Option<f64>,
    label: Option<String>,
}

impl AlertFile {
    fn into_alert(self) -> Result<LensAlert, String> {
        match (&self.min_rows, &self.column) {
            (Some(_), Some(_)) => {
                return Err("`alert` takes `min_rows` or `column`, not both".into())
            }
            (None, None) => return Err("`alert` needs `min_rows` or `column`".into()),
            (Some(n), None) if *n < 1 => return Err("`alert.min_rows` must be at least 1".into()),
            (None, Some(_)) if self.above.is_none() && self.below.is_none() => {
                return Err("`alert.column` needs `above` or `below`".into())
            }
            _ => {}
        }
        Ok(LensAlert {
            min_rows: self.min_rows,
            column: self.column,
            above: self.above,
            below: self.below,
            label: self.label,
        })
    }
}

/// A lens's alert, evaluated on one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AlertState {
    pub firing: bool,
    /// Rows the run returned.
    pub count: i64,
    /// The watched column's value, for a threshold alert.
    pub value: Option<f64>,
    /// What the badge says, e.g. `3 rows` or `Coverage low: 72`.
    pub message: String,
}

fn cell_number(c: &SqlCell) -> Option<f64> {
    match c {
        SqlCell::Int(i) => Some(*i as f64),
        SqlCell::Real(r) => Some(*r),
        SqlCell::Text(t) => t.trim().parse().ok(),
        _ => None,
    }
}

fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Evaluate `alert` against a run's result.
fn evaluate_alert(alert: &LensAlert, result: &SqlQueryResult) -> AlertState {
    let count = result.rows.len() as i64;
    if let Some(min) = alert.min_rows {
        let message = match &alert.label {
            Some(l) => format!("{l}: {count}"),
            None => format!("{count} {}", if count == 1 { "row" } else { "rows" }),
        };
        return AlertState {
            firing: count >= min,
            count,
            value: None,
            message,
        };
    }
    let column = alert.column.clone().unwrap_or_default();
    let value = result
        .columns
        .iter()
        .position(|c| *c == column)
        .and_then(|i| result.rows.first().and_then(|r| r.get(i)))
        .and_then(cell_number);
    let firing = value
        .is_some_and(|v| alert.above.is_some_and(|a| v > a) || alert.below.is_some_and(|b| v < b));
    let name = alert.label.clone().unwrap_or(column);
    let message = match value {
        Some(v) => format!("{name}: {}", fmt_number(v)),
        None => name,
    };
    AlertState {
        firing,
        count,
        value,
        message,
    }
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
    /// Any oxplow page by its tab id (`work_item:oxplow:tsk42`,
    /// `commit:<sha>`, `page:git-dashboard`, …),
    /// e.g. `v_page_visit.page_id`.
    Page,
    /// A file's diff within a change: the value is the path; `line`,
    /// `base` and `head` name the columns holding the line and the
    /// change's `base_label` / `head_label` (join `v_change`).
    DiffAt,
    /// Two line ranges side by side: the value is
    /// `path:start-end|peer:start-end`; `head` names a column with the
    /// version to read (a change's `head_label`; the working tree if absent).
    Compare,
}

/// Makes a column's cells link to a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensLink {
    pub kind: LensLinkKind,
    /// Result column holding the target id. Defaults to the column itself.
    #[serde(default)]
    pub from: Option<String>,
    /// For `file` / `diff-at`: result column holding a line number to open at.
    #[serde(default)]
    pub line: Option<String>,
    /// For `diff-at`: column holding the older side's label.
    #[serde(default)]
    pub base: Option<String>,
    /// For `diff-at` / `compare`: column holding the newer side's label.
    #[serde(default)]
    pub head: Option<String>,
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
    /// Buttons from the fixed action registry: `copy`, `add-to-context`,
    /// or `{action: run-source, source: <ext>/<id>}`. Parsed by
    /// [`parse_actions`].
    #[serde(default)]
    actions: Vec<serde_yaml::Value>,
    #[serde(default)]
    alert: Option<AlertFile>,
}

/// What a lens action does. A fixed registry: an extension can't run code
/// through one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum LensActionKind {
    /// Copy the lens result as text (markdown).
    Copy,
    /// Hand the lens and its params to the agent (UI only).
    AddToContext,
    /// Sync a source (an exec source needs a person's approval first).
    RunSource,
}

/// A button on a lens (tsk329).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensAction {
    /// Unique within the lens; `run_lens_action` names it. Defaults to the
    /// kind.
    pub id: String,
    pub kind: LensActionKind,
    pub label: String,
    /// For `run-source`: `<extension>/<source id>`.
    pub source: Option<String>,
}

/// Parse a lens's `actions:`. Each is a bare kind (`copy`) or a map with
/// `action` and optional `id`, `label`, `source`.
fn parse_actions(raw: Vec<serde_yaml::Value>) -> Result<Vec<LensAction>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Full {
        action: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        source: Option<String>,
    }
    let mut out: Vec<LensAction> = Vec::new();
    for v in raw {
        let f = match v {
            serde_yaml::Value::String(action) => Full {
                action,
                id: None,
                label: None,
                source: None,
            },
            other => serde_yaml::from_value::<Full>(other).map_err(|e| format!("actions: {e}"))?,
        };
        let kind = match f.action.as_str() {
            "copy" => LensActionKind::Copy,
            "add-to-context" => LensActionKind::AddToContext,
            "run-source" => LensActionKind::RunSource,
            other => {
                return Err(format!(
                    "unknown action `{other}` (copy, add-to-context or run-source)"
                ))
            }
        };
        match (kind, &f.source) {
            (LensActionKind::RunSource, Some(src))
                if src
                    .split_once('/')
                    .is_some_and(|(e, i)| !e.is_empty() && !i.is_empty()) => {}
            (LensActionKind::RunSource, _) => {
                return Err("action `run-source` needs `source: <extension>/<source>`".into())
            }
            (_, Some(_)) => return Err("only `run-source` takes a source".into()),
            _ => {}
        }
        let id = f.id.unwrap_or_else(|| f.action.clone());
        if out.iter().any(|a| a.id == id) {
            return Err(format!(
                "action `{id}` is declared twice (give one an `id`)"
            ));
        }
        let label = f.label.unwrap_or_else(|| {
            match kind {
                LensActionKind::Copy => "Copy",
                LensActionKind::AddToContext => "Add to Agent Context",
                LensActionKind::RunSource => "Sync",
            }
            .to_string()
        });
        out.push(LensAction {
            id,
            kind,
            label,
            source: f.source,
        });
    }
    Ok(out)
}

fn default_viz() -> LensViz {
    LensViz::Table
}

/// When core runs an advisory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOn {
    /// After each agent tool call; results go to the agent as that call's
    /// context and are recorded as nudges.
    PostToolUse,
    /// On each prompt the human sends; results join the prompt's context.
    Prompt,
}

/// How often the same advisory may reach the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOncePer {
    /// Once per effort, the first time the query returns rows.
    Effort,
    /// Once per effort per `key` value: each row's `key` column fires once.
    Row,
    /// Every time, whenever the query returns rows.
    Turn,
}

fn default_once_per() -> AdvisoryOncePer {
    AdvisoryOncePer::Effort
}

/// Guidance an extension gives the coding agent: a query over the semantic
/// layer, run by core at `on`, with `:effort_id` bound to the thread's
/// open effort. Each result row's `message` column is a line of guidance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Advisory {
    pub id: String,
    pub on: AdvisoryOn,
    pub query: String,
    pub once_per: AdvisoryOncePer,
    /// Line put above the messages (e.g. `# Metric deltas (this effort)`).
    pub heading: Option<String>,
}

/// An advisory as written in `extension.yaml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdvisoryFile {
    id: String,
    on: AdvisoryOn,
    query: String,
    #[serde(default = "default_once_per")]
    once_per: AdvisoryOncePer,
    #[serde(default)]
    heading: Option<String>,
}

/// Places in core pages an extension can mount a lens, and the params each
/// offers (`change_id` = the page's `v_change` row). A mounted lens gets
/// the ones it declares and must declare at least one.
pub const SLOTS: &[(&str, &[&str])] = &[
    ("effort-review", &["effort_id", "change_id"]),
    ("task-detail", &["task_id"]),
    ("thread", &["thread_id"]),
    ("commit", &["change_id"]),
    ("uncommitted", &["change_id"]),
    // The rail: no params; mounted lenses must declare an `alert` and show
    // as a badge while it fires.
    ("rail", &[]),
    // Settings: a section per extension with the lenses it mounts (its
    // own status or configuration views). No params.
    ("settings", &[]),
];

/// A lens an extension mounts into a core page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensSlot {
    /// Which page, from [`SLOTS`]: `effort-review` (an effort's diff
    /// view: `:effort_id`, `:change_id`), `task-detail` (`:task_id`),
    /// `thread` (`:thread_id`), `commit` or `uncommitted` (`:change_id`).
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
    /// Buttons from the fixed action registry (tsk329).
    pub actions: Vec<LensAction>,
    /// When the lens needs attention (a rail badge when mounted in `rail`).
    pub alert: Option<LensAlert>,
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
    /// Things worth fixing that don't stop it loading: a v1 manifest, an
    /// intent with no examples.
    pub warnings: Vec<String>,
    /// `2` for a current manifest; `1` for one read through the v1
    /// compatibility path (see `warnings`).
    pub manifest_version: u32,
    pub sharing: Sharing,
    /// Why it exists (required at v2; `None` for a v1 manifest).
    pub intent: Option<Intent>,
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
    /// extension has no lenses, slots, sources or advisories.
    pub enabled: bool,
    /// Guidance for the coding agent (valid ones; invalid ones are in `errors`).
    pub advisories: Vec<Advisory>,
    /// Measures, metrics and gauges it contributes to the metric catalog
    /// (the `project.yaml` schema). Metrics are `key:` definitions and are
    /// on while the extension is enabled; gauges are `starlark`/`jaq` only,
    /// with their `entryFile` inside the extension.
    pub measures: Vec<oxplow_config::MeasureEntry>,
    pub metrics: Vec<oxplow_config::MetricEntry>,
    pub gauges: Vec<oxplow_config::GaugeEntry>,
    /// Dimensions it contributes (fact or entity), never promoted: an
    /// extension toggling would rebuild the metric cube each time.
    pub dimensions: Vec<oxplow_config::DimensionEntry>,
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
    /// The lens's alert on this result, if it declares one.
    pub alert: Option<AlertState>,
}

/// A lens result as text, for the `copy` action: a markdown lens's text, a
/// number lens's value, anything else as a markdown table of what the
/// lens shows.
pub fn lens_text(run: &LensRun) -> String {
    let cell = |c: &SqlCell| match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.clone(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    };
    let first = run.result.rows.first().and_then(|r| r.first());
    match run.lens.viz {
        LensViz::Markdown | LensViz::Number => first.map(cell).unwrap_or_default(),
        _ => {
            // What the table shows: the declared columns in their order, or
            // every result column when none are declared (the UI's
            // `displayColumns`).
            let shown: Vec<(usize, String)> = if run.lens.columns.is_empty() {
                run.result.columns.iter().cloned().enumerate().collect()
            } else {
                run.lens
                    .columns
                    .iter()
                    .filter_map(|c| {
                        let i = run.result.columns.iter().position(|k| k == &c.key)?;
                        Some((i, c.label.clone().unwrap_or_else(|| c.key.clone())))
                    })
                    .collect()
            };
            let esc = |t: String| t.replace('|', "\\|").replace('\n', " ");
            let mut out = format!(
                "| {} |\n|{}\n",
                shown
                    .iter()
                    .map(|(_, l)| esc(l.clone()))
                    .collect::<Vec<_>>()
                    .join(" | "),
                " --- |".repeat(shown.len())
            );
            for r in &run.result.rows {
                out.push_str(&format!(
                    "| {} |\n",
                    shown
                        .iter()
                        .map(|(i, _)| esc(r.get(*i).map(cell).unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
            out
        }
    }
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
    // A grid's child in ANOTHER extension resolves here, once every
    // extension is loaded; a missing one is an error and the grid is
    // dropped so every loaded lens renders.
    let all_ids: Vec<String> = out
        .iter()
        .flat_map(|e| e.lenses.iter().map(|l| l.id.clone()))
        .collect();
    for ext in &mut out {
        let mut bad = Vec::new();
        for l in ext.lenses.iter().filter(|l| l.viz == LensViz::Grid) {
            for c in &l.children {
                if !all_ids.contains(c) {
                    ext.errors.push(format!(
                        "{}: child lens `{c}` isn't in any loaded extension",
                        l.path
                    ));
                    bad.push(l.id.clone());
                }
            }
        }
        ext.lenses.retain(|l| !bad.contains(&l.id));
    }
    let disabled = oxplow_config::disabled_extensions(root);
    out.into_iter()
        .map(|e| apply_disabled(e, &disabled))
        .collect()
}

/// A `measures:` / `metrics:` / `gauges:` block as typed entries.
fn parse_block<T: serde::de::DeserializeOwned>(
    v: Option<serde_yaml::Value>,
) -> Result<Option<Vec<T>>, String> {
    v.map(|v| serde_yaml::from_value(v).map_err(|e| e.to_string()))
        .transpose()
}

/// A file inside extension `name` (bundled or in `oxplow/extensions/`),
/// e.g. a gauge's script. `None` when there's no such extension or file.
pub fn read_extension_file(root: &Path, name: &str, rel: &str) -> Option<String> {
    if rel.split('/').any(|seg| seg == "..") {
        return None;
    }
    if let Some(b) = crate::bundled_extensions::BUNDLED
        .iter()
        .find(|b| b.name == name)
    {
        return Embedded(b).read(rel);
    }
    Disk(root.join(EXTENSIONS_DIR).join(name)).read(rel)
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
        warnings: Vec::new(),
        manifest_version: manifest_v2::CURRENT,
        sharing: Sharing::Private,
        intent: None,
        lenses: Vec::new(),
        source: None,
        sources: Vec::new(),
        origin: origin.to_string(),
        slots: Vec::new(),
        enabled: true,
        advisories: Vec::new(),
        measures: Vec::new(),
        dimensions: Vec::new(),
        metrics: Vec::new(),
        gauges: Vec::new(),
    }
}

fn is_advisory_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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
    let file = format!("{rel}/extension.yaml");
    // `manifest: 2` says which shape to read; a file without it is v1 and
    // is read through the same textual migration `oxplow plugin migrate`
    // writes, so the two can't disagree.
    let is_v2 = key_line(&manifest, "manifest").is_some();
    let manifest = if is_v2 {
        manifest
    } else {
        migrate_v1::migrate_v1_to_v2(&manifest)
    };
    let parsed: Result<ManifestV2, String> =
        serde_yaml::from_str::<ManifestV2>(&manifest).map_err(|e| e.to_string());
    let m = match parsed {
        Ok(m) if m.name != name => {
            ext.errors.push(format!(
                "{file}: name `{}` must match its folder `{name}`",
                m.name
            ));
            return ext;
        }
        Ok(m) => m,
        Err(e) => {
            ext.errors.push(format!("{file}: {e}"));
            return ext;
        }
    };
    ext.manifest_version = if is_v2 { manifest_v2::CURRENT } else { 1 };
    ext.sharing = m.sharing;
    ext.intent = m.intent.clone();
    ext.description = m.description.clone();
    if is_v2 {
        let (errors, warnings) = manifest_v2::check(&m, &file, &manifest, origin == "bundled");
        ext.errors.extend(errors);
        ext.warnings.extend(warnings);
    } else {
        ext.warnings.push(at(
            &file,
            Some(1),
            "manifest v1 (no `manifest:` key); run `oxplow plugin migrate` to rewrite it as v2 with an `intent`",
        ));
    }
    let slot_files = {
        if let Some(v) = &m.collectors {
            let (sources, errors) = crate::extension_sources::parse_sources(name, v);
            ext.sources = sources;
            ext.errors.extend(
                errors
                    .into_iter()
                    .map(|e| at(&file, key_line(&manifest, "collectors"), e)),
            );
        }
        for v in m.advisories.clone() {
            match serde_yaml::from_value::<AdvisoryFile>(v).map(|a| Advisory {
                id: a.id,
                on: a.on,
                query: a.query,
                once_per: a.once_per,
                heading: a.heading,
            }) {
                Ok(a) if !is_advisory_id(&a.id) => ext.errors.push(at(
                    &file,
                    line_under(&manifest, "advisories", &a.id),
                    format!(
                        "advisory id `{}` must be lowercase letters, digits and dashes",
                        a.id
                    ),
                )),
                Ok(a) => ext.advisories.push(a),
                Err(e) => ext.errors.push(at(
                    &file,
                    key_line(&manifest, "advisories"),
                    format!("advisory: {e}"),
                )),
            }
        }
        let err_at = |key: &str, e: String| at(&file, key_line(&manifest, key), e);
        match parse_block(m.measures.clone())
            .and_then(|v| oxplow_config::validate_measures(v).map_err(|e| e.to_string()))
        {
            Ok(v) => ext.measures = v,
            Err(e) => ext.errors.push(err_at("measures", e)),
        }
        match parse_block(m.dimensions.clone())
            .and_then(|v| oxplow_config::validate_dimensions(v).map_err(|e| e.to_string()))
        {
            Ok(v) => {
                for d in v {
                    if d.promote {
                        ext.errors.push(err_at(
                            "dimensions",
                            format!(
                                "dimension `{}`: `promote` belongs in .oxplow/project.yaml (it rebuilds the metric cube)",
                                d.key.clone().unwrap_or_default()
                            ),
                        ));
                    } else {
                        ext.dimensions.push(d);
                    }
                }
            }
            Err(e) => ext.errors.push(err_at("dimensions", e)),
        }
        match parse_block(m.metrics.clone())
            .and_then(|v| oxplow_config::validate_metrics(v).map_err(|e| e.to_string()))
        {
            Ok(v) => {
                for e in v {
                    if e.key.is_some() {
                        ext.metrics.push(e);
                    } else {
                        ext.errors.push(err_at(
                            "metrics",
                            format!(
                                "metrics: `use: {}` belongs in .oxplow/project.yaml; an extension defines metrics with `key:`",
                                e.use_key.unwrap_or_default()
                            ),
                        ));
                    }
                }
            }
            Err(e) => ext.errors.push(err_at("metrics", e)),
        }
        match parse_block(m.gauges.clone())
            .and_then(|v| oxplow_config::validate_gauges(v).map_err(|e| e.to_string()))
        {
            Ok(v) => {
                for g in v {
                    let key = g.key.clone().unwrap_or_default();
                    let compute = g.compute.clone().unwrap_or_default();
                    let entry = compute.entry_file.clone().unwrap_or_default();
                    if !matches!(compute.runtime.as_str(), "starlark" | "jaq") {
                        ext.errors.push(err_at(
                            "gauges",
                            format!(
                                "gauge `{key}`: extension gauges run `starlark` or `jaq` only, not `{}` (a program belongs in a collector, which needs your approval)",
                                compute.runtime
                            ),
                        ));
                    } else if files.read(&entry).is_none() {
                        ext.errors.push(err_at(
                            "gauges",
                            format!("gauge `{key}`: entryFile `{entry}` isn't in the extension"),
                        ));
                    } else {
                        ext.gauges.push(g);
                    }
                }
            }
            Err(e) => ext.errors.push(err_at("gauges", e)),
        }
        // Cross-references inside the catalog: a metric's source measure
        // and a gauge's emitted measures are normally ones this extension
        // declares or oxplow's built-ins. A measure from another scope
        // (the project's or another extension's `measures:`) resolves
        // when the catalog is assembled, so that is a warning, not an
        // error — but a typo would be silent otherwise.
        let measure_known = |key: &str| {
            key.starts_with("oxplow.")
                || ext.measures.iter().any(|mm| mm.key.as_deref() == Some(key))
        };
        for spec in &ext.metrics {
            if let Some(sm) = spec.source_measure.as_deref() {
                if !measure_known(sm) {
                    ext.warnings.push(err_at(
                        "metrics",
                        format!(
                            "metric `{}`: sourceMeasure `{sm}` is not a measure this extension declares (or a built-in `oxplow.*`); it must come from the project's or another extension's `measures:`",
                            spec.key.clone().unwrap_or_default()
                        ),
                    ));
                }
            }
        }
        for g in &ext.gauges {
            for emitted in &g.emits {
                if !measure_known(emitted) {
                    ext.warnings.push(err_at(
                        "gauges",
                        format!(
                            "gauge `{}`: emits `{emitted}`, which is not a measure this extension declares (or a built-in `oxplow.*`); it must come from the project's or another extension's `measures:`",
                            g.key.clone().unwrap_or_default()
                        ),
                    ));
                }
            }
        }
        m.slot_mounts.clone()
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
            .and_then(|t| serde_yaml::from_str::<LensFile>(&t).map_err(|e| e.to_string()))
            .and_then(|mut l| {
                let alert = l.alert.take().map(AlertFile::into_alert).transpose()?;
                let actions = parse_actions(std::mem::take(&mut l.actions))?;
                Ok((l, alert, actions))
            });
        match parsed {
            Ok((l, alert, actions)) => ext.lenses.push(Lens {
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
                actions,
                alert,
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
        // A slot offers params; a mounted lens takes the ones it declares
        // and must declare at least one, or it can't relate to the page.
        let missing: Vec<&str> = match (slot_params, lens) {
            (Some(params), Some(l))
                if !params
                    .iter()
                    .any(|p| l.params.iter().any(|lp| lp.name == *p)) =>
            {
                params.to_vec()
            }
            _ => vec![],
        };
        let mount_line = line_under(&manifest, "slot_mounts", &format!("lens: {}", s.lens));
        if slot_params.is_none() {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "unknown slot `{}` (known: {})",
                    s.slot,
                    SLOTS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ),
            ));
        } else if !missing.is_empty() {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "slot `{}` passes {}; lens `{}` must declare at least one in `params`",
                    s.slot,
                    missing
                        .iter()
                        .map(|p| format!("`{p}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    s.lens
                ),
            ));
        } else if lens.is_none() {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "slot `{}` mounts lens `{}`, which isn't in lenses/",
                    s.slot, s.lens
                ),
            ));
        } else if s.slot == "rail" && lens.is_some_and(|l| l.alert.is_none()) {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "the `rail` slot shows alerts; lens `{}` declares no `alert`",
                    s.lens
                ),
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
        ext.advisories.clear();
        ext.measures.clear();
        ext.dimensions.clear();
        ext.metrics.clear();
        ext.gauges.clear();
    }
    ext
}

pub(crate) fn disabled_error(name: &str) -> DomainError {
    DomainError::Invalid(format!(
        "extension `{name}` is disabled in .oxplow/project.yaml (extensions.disabled); enable it in Settings → Extensions"
    ))
}

/// Find one lens by `<extension>/<slug>` through the catalog cache.
pub fn find_lens(
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    id: &str,
) -> Result<Lens, DomainError> {
    catalog.find_lens(root, id)
}

/// Where a lens is being looked at from: the viewer's current stream and
/// thread (numeric ids, as the `v_*` views use). A lens that declares a
/// `stream_id` or `thread_id` param gets these unless it's given another
/// value (tsk375).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LensContext {
    pub stream_id: Option<i64>,
    pub thread_id: Option<i64>,
}

impl LensContext {
    /// The implicit value for a param named `name`, if it is one.
    fn value(&self, name: &str) -> Option<SqlCell> {
        match name {
            "stream_id" => self.stream_id.map(SqlCell::Int),
            "thread_id" => self.thread_id.map(SqlCell::Int),
            _ => None,
        }
    }
}

/// The context a lens is viewed from: `stream` (else `thread`'s stream,
/// else the primary) and `thread` (else that stream's selected or active
/// thread). Lookups that fail leave the value unset.
pub async fn lens_context(
    svc: &crate::Services,
    stream: Option<oxplow_domain::StreamId>,
    thread: Option<oxplow_domain::ThreadId>,
) -> LensContext {
    use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
    let mut stream = stream;
    if stream.is_none() {
        if let Some(t) = thread {
            stream = svc
                .thread_store
                .get(&t)
                .await
                .ok()
                .flatten()
                .map(|t| t.stream_id);
        }
    }
    if stream.is_none() {
        stream = svc.stream_store.list().await.ok().and_then(|all| {
            all.into_iter()
                .find(|s| matches!(s.kind, oxplow_domain::StreamKind::Primary))
                .map(|s| s.id)
        });
    }
    let thread = match (thread, stream) {
        (Some(t), _) => Some(t),
        (None, Some(s)) => svc.threads.selected_or_active(&s).await.ok().flatten(),
        (None, None) => None,
    };
    LensContext {
        stream_id: stream.map(|s| s.value()),
        thread_id: thread.map(|t| t.value()),
    }
}

/// Run a lens: bind supplied params over the viewer's context over
/// defaults, and query the semantic layer. Unknown params are rejected,
/// so a typo doesn't silently fall back to a default.
pub async fn run_lens(
    layer: &SemanticLayer,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    id: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    let lens = catalog.find_lens(root, id)?;
    execute(layer, lens, params, ctx)
        .await
        .map_err(|e| match e {
            DomainError::Invalid(m) => DomainError::Invalid(explain_unsynced(catalog, root, &m)),
            other => other,
        })
}

/// A lens reading a source entity before its first sync fails with
/// SQLite's bare "no such table: v_x". Say which source to run instead.
fn explain_unsynced(
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    message: &str,
) -> String {
    let Some(rest) = message.split("no such table: ").nth(1) else {
        return message.to_string();
    };
    let view: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    for ext in catalog.get(root).iter() {
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
    ctx: &LensContext,
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
            .or_else(|| ctx.value(&p.name))
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
    let alert = lens.alert.as_ref().map(|a| evaluate_alert(a, &result));
    Ok(LensRun {
        lens,
        params,
        result,
        alert,
    })
}

/// Load one extension and dry-run every lens with default params,
/// reporting query failures and `columns` keys the query doesn't
/// return as errors.
pub async fn validate_extension(
    layer: &SemanticLayer,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
) -> Result<Extension, DomainError> {
    let mut ext = catalog.named(root, name)?;
    check_extension(layer, catalog, root, &mut ext).await;
    Ok(ext)
}

/// Dry-run a loaded extension's advisories and lenses, appending what's
/// wrong to its `errors`. `root` names sources for an unsynced view.
async fn check_extension(
    layer: &SemanticLayer,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
) {
    for a in ext.advisories.clone() {
        let run = layer
            .query_sql_named(
                &a.query,
                vec![("effort_id".into(), SqlCell::Null(()))],
                None,
            )
            .await;
        match run {
            Err(e) => ext.errors.push(format!(
                "advisory {}: {}",
                a.id,
                e.to_string().replacen("invalid value: ", "", 1)
            )),
            Ok(r) => {
                let mut need = vec!["message"];
                if a.once_per == AdvisoryOncePer::Row {
                    need.push("key");
                }
                for col in need {
                    if !r.columns.iter().any(|c| c == col) {
                        ext.errors.push(format!(
                            "advisory {}: the query must return a `{col}` column (columns: {})",
                            a.id,
                            r.columns.join(", ")
                        ));
                    }
                }
            }
        }
    }
    for lens in ext.lenses.clone() {
        let id = lens.id.clone();
        match execute(layer, lens, BTreeMap::new(), &LensContext::default()).await {
            Err(e) => ext.errors.push(explain_unsynced(
                catalog,
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
                        keys.extend(link.base.iter());
                        keys.extend(link.head.iter());
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
}

/// What installing an extension from git would bring in, for a person to
/// look at first (tsk378): the extension as it would load (its lenses,
/// sources with their programs, hosts and credentials, advisories,
/// gauges; `extension.errors` are load errors, which block the install),
/// the commit it's at, and `problems` a dry run of its lenses and
/// advisories found (reported, not blocking: a lens over a source that
/// hasn't synced can't run yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionReview {
    pub extension: Extension,
    pub git: String,
    pub git_ref: Option<String>,
    /// The commit reviewed; pass it back to install exactly this.
    pub sha: String,
    pub problems: Vec<String>,
}

/// Clone an extension and report what it declares, installing nothing.
/// `replacing` names the installed extension an update must match.
pub async fn review_extension(
    layer: &SemanticLayer,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    replacing: Option<&str>,
) -> Result<ExtensionReview, DomainError> {
    let fetched = {
        let (root, url, r, rep) = (
            root.to_path_buf(),
            git_url.to_string(),
            git_ref.map(str::to_string),
            replacing.map(str::to_string),
        );
        tokio::task::spawn_blocking(move || fetch(&root, &url, r.as_deref(), rep.as_deref()))
            .await
            .map_err(|e| DomainError::Storage(format!("extension review: {e}")))??
    };
    let mut extension = fetched.load();
    let load_errors = extension.errors.len();
    check_extension(layer, catalog, root, &mut extension).await;
    let problems = extension.errors.split_off(load_errors);
    Ok(ExtensionReview {
        extension,
        git: git_url.to_string(),
        git_ref: git_ref.map(str::to_string),
        sha: fetched.sha,
        problems,
    })
}

/// [`review_extension`] for an update: the installed extension's recorded
/// source.
pub async fn review_update(
    layer: &SemanticLayer,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
) -> Result<ExtensionReview, DomainError> {
    let source = installed_source(root, name)?;
    review_extension(
        layer,
        catalog,
        root,
        &source.git,
        source.git_ref.as_deref(),
        Some(name),
    )
    .await
}

/// Install an extension from a git repo whose root holds `extension.yaml`:
/// clone it (inside `.oxplow/tmp/`, per workspace isolation), copy it to
/// `oxplow/extensions/<name>/` without `.git`, and record its source.
/// Installs only `reviewed_sha`, the commit a person looked at with
/// [`review_extension`], and only when it loads without errors. Refuses
/// to overwrite an existing folder; use [`update_extension`].
pub fn install_extension(
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    reviewed_sha: &str,
) -> Result<Extension, DomainError> {
    install_from_git(root, git_url, git_ref, None, reviewed_sha)
}

/// Re-install an installed extension from its recorded source (same URL
/// and ref), at the new commit a person reviewed with [`review_update`].
pub fn update_extension(
    root: &Path,
    name: &str,
    reviewed_sha: &str,
) -> Result<Extension, DomainError> {
    let source = installed_source(root, name)?;
    install_from_git(
        root,
        &source.git,
        source.git_ref.as_deref(),
        Some(name),
        reviewed_sha,
    )
}

/// One extension by name, read from disk now — for the write paths
/// (install, update, save), which must see their own result whatever a
/// cache holds. Reads go through `extension_catalog::ExtensionCatalog`.
fn load_fresh(root: &Path, name: &str) -> Result<Extension, DomainError> {
    let all = load_extensions(root);
    let ext = all
        .iter()
        .find(|e| e.name == name && e.origin == "bundled")
        .or_else(|| all.iter().find(|e| e.name == name))
        .ok_or(DomainError::NotFound)?;
    if !ext.enabled {
        return Err(disabled_error(name));
    }
    Ok(ext.clone())
}

/// Where an installed extension came from.
fn installed_source(root: &Path, name: &str) -> Result<ExtensionSource, DomainError> {
    load_fresh(root, name)?.source.ok_or_else(|| {
        DomainError::Invalid(format!(
            "extension `{name}` wasn't installed from git (no {SOURCE_FILE}); edit it in place instead"
        ))
    })
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

/// A published extension cloned into `.oxplow/tmp/` (removed on drop).
struct Fetched {
    _tmp: tempfile::TempDir,
    clone: std::path::PathBuf,
    sha: String,
    name: String,
}

impl Fetched {
    /// The extension as it would load once installed.
    fn load(&self) -> Extension {
        load_one(
            &Disk(self.clone.clone()),
            &self.name,
            &format!("{EXTENSIONS_DIR}/{}", self.name),
            "project",
        )
    }
}

/// Clone `git_url` at `git_ref` and check it is an installable extension:
/// `extension.yaml` at its root, a safe name that isn't bundled, and for
/// an update (`replacing`) the same name.
fn fetch(
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    replacing: Option<&str>,
) -> Result<Fetched, DomainError> {
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
    // Only the name matters here; the full load reports the rest.
    #[derive(Deserialize)]
    struct Named {
        name: String,
    }
    let name = serde_yaml::from_str::<Named>(&manifest)
        .map_err(|e| invalid(format!("{git_url}: extension.yaml: {e}")))?
        .name;
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
    Ok(Fetched {
        _tmp: tmp,
        clone,
        sha,
        name,
    })
}

/// Clone, check it's the reviewed commit and loads cleanly, then copy
/// into place. The old folder of an update is removed only after all
/// that, so a failed update changes nothing.
fn install_from_git(
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    replacing: Option<&str>,
    reviewed_sha: &str,
) -> Result<Extension, DomainError> {
    let invalid = |m: String| DomainError::Invalid(m);
    let storage = |e: std::io::Error| DomainError::Storage(format!("extension install: {e}"));
    let fetched = fetch(root, git_url, git_ref, replacing)?;
    let name = fetched.name.clone();
    if fetched.sha != reviewed_sha {
        return Err(invalid(format!(
            "{git_url} changed since you reviewed it (now at {}); review it again",
            &fetched.sha[..fetched.sha.len().min(12)]
        )));
    }
    let loaded = fetched.load();
    if !loaded.errors.is_empty() {
        return Err(invalid(format!(
            "extension `{name}` has errors, so it wasn't installed: {}",
            loaded.errors.join("; ")
        )));
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
    copy_tree_without_git(&fetched.clone, &target).map_err(storage)?;

    let source = ExtensionSource {
        git: git_url.to_string(),
        git_ref: git_ref.map(str::to_string),
        sha: fetched.sha.clone(),
    };
    let yaml = serde_yaml::to_string(&source)
        .map_err(|e| DomainError::Storage(format!("extension install: {e}")))?;
    std::fs::write(target.join(SOURCE_FILE), yaml).map_err(storage)?;
    Ok(load_fresh(root, &name)
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
    load_fresh(root, extension)?
        .lenses
        .into_iter()
        .find(|l| l.slug == slug)
        .ok_or(DomainError::NotFound)
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

    fn cat() -> crate::extension_catalog::ExtensionCatalog {
        crate::extension_catalog::ExtensionCatalog::new()
    }

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
                base: None,
                head: None,
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
            find_lens(&cat(), dir.path(), "review/by-status")
                .unwrap()
                .title,
            "Tasks by status"
        );
        assert!(matches!(
            find_lens(&cat(), dir.path(), "review/nope"),
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            find_lens(&cat(), dir.path(), "nope"),
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

        let run = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/echo",
            BTreeMap::new(),
            &LensContext::default(),
        )
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
        let run = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/echo",
            over,
            &LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&run.result.rows).unwrap(),
            serde_json::json!([[1, "three"]])
        );
    }

    /// A lens declaring `stream_id` / `thread_id` gets the caller's
    /// current ones unless it's given others; a default applies only with
    /// no current value (tsk375).
    #[tokio::test]
    async fn run_lens_binds_the_current_stream_and_thread() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/mine.yaml",
            "title: Mine\nparams:\n  - { name: stream_id }\n  - { name: thread_id, default: 9 }\nquery: SELECT :stream_id AS s, :thread_id AS t\n",
        );
        let sl = layer().await;
        let rows = |run: LensRun| serde_json::to_value(&run.result.rows).unwrap();
        let here = LensContext {
            stream_id: Some(2),
            thread_id: Some(5),
        };
        let run = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/mine",
            BTreeMap::new(),
            &here,
        )
        .await
        .unwrap();
        assert_eq!(rows(run), serde_json::json!([[2, 5]]));

        let mut over = BTreeMap::new();
        over.insert("thread_id".to_string(), SqlCell::Int(7));
        let run = run_lens(&sl, &cat(), dir.path(), "review/mine", over, &here)
            .await
            .unwrap();
        assert_eq!(
            rows(run),
            serde_json::json!([[2, 7]]),
            "an explicit value wins"
        );

        let run = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/mine",
            BTreeMap::new(),
            &LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows(run),
            serde_json::json!([[null, 9]]),
            "no context: defaults"
        );
    }

    /// The viewer's context: the given stream (else the primary) and the
    /// given thread (else the stream's selected one) (tsk375).
    #[tokio::test]
    async fn lens_context_defaults_to_the_primary_stream_and_its_thread() {
        let f = crate::test_fixtures::services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let thread = f.svc.thread_store.get(&f.thread).await.unwrap().unwrap();
        let expect = LensContext {
            stream_id: Some(thread.stream_id.value()),
            thread_id: Some(f.thread.value()),
        };
        assert_eq!(lens_context(&f.svc, None, None).await, expect);
        assert_eq!(lens_context(&f.svc, None, Some(f.thread)).await, expect);
        assert_eq!(
            lens_context(&f.svc, Some(thread.stream_id), None).await,
            expect
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
        let err = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/by-status",
            bad,
            &LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("stauts")),
            "{err:?}"
        );

        let err = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "review/broken",
            BTreeMap::new(),
            &LensContext::default(),
        )
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

        let e = validate_extension(&sl, &cat(), dir.path(), "review")
            .await
            .unwrap();
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("review/broken")));
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("review/badcol") && m.contains("missing")));

        assert!(matches!(
            validate_extension(&sl, &cat(), dir.path(), "nope").await,
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

    fn head(repo: &Path) -> String {
        run_git(repo, &["rev-parse", "HEAD"]).unwrap()
    }

    /// Before anything lands in the repo a person sees what the extension
    /// declares (sources with their programs, hosts and credentials) and
    /// what's wrong with it; install then takes exactly the commit they
    /// reviewed (tsk378).
    #[tokio::test]
    async fn review_shows_what_an_extension_declares_before_install() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        write(
            repo.path(),
            "extension.yaml",
            "name: shared\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    network: [api.github.com]\n    credentials: [TOKEN]\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(repo.path(), "sync.sh", "echo '{}'\n");
        write(
            repo.path(),
            "lenses/broken.yaml",
            "title: Broken\nquery: SELECT 1 AS n\ncolumns:\n  - { key: missing }\n",
        );
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "source"]);
        let url = repo.path().to_string_lossy().to_string();
        let sl = layer().await;

        let review = review_extension(&sl, &cat(), project.path(), &url, None, None)
            .await
            .unwrap();
        assert_eq!(review.extension.name, "shared");
        assert_eq!(review.sha, head(repo.path()));
        let source = &review.extension.sources[0];
        assert_eq!(source.network, vec!["api.github.com".to_string()]);
        assert_eq!(source.credentials, vec!["TOKEN".to_string()]);
        assert!(
            review.extension.errors.is_empty(),
            "{:?}",
            review.extension.errors
        );
        assert!(
            review.problems.iter().any(|p| p.contains("missing")),
            "{:?}",
            review.problems
        );
        assert!(
            !project.path().join("oxplow/extensions").exists(),
            "a review installs nothing"
        );
        let tmp = project.path().join(".oxplow/tmp");
        assert!(!tmp.exists() || std::fs::read_dir(&tmp).unwrap().next().is_none());

        // A commit that isn't the reviewed one is refused.
        write(repo.path(), "sync.sh", "curl evil.example\n");
        git(repo.path(), &["commit", "-q", "-am", "swap"]);
        let err = install_extension(project.path(), &url, None, &review.sha).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("changed since")),
            "{err:?}"
        );
        assert!(!project.path().join("oxplow/extensions").exists());
        let ext = install_extension(project.path(), &url, None, &head(repo.path())).unwrap();
        assert_eq!(ext.name, "shared");
    }

    /// An extension that doesn't load cleanly isn't installed (tsk378).
    #[test]
    fn install_refuses_an_extension_with_load_errors() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        write(repo.path(), "extension.yaml", "name: shared\nbogus: 1\n");
        git(repo.path(), &["commit", "-q", "-am", "bad"]);
        let url = repo.path().to_string_lossy().to_string();
        let err = install_extension(project.path(), &url, None, &head(repo.path())).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("bogus")),
            "{err:?}"
        );
        assert!(!project.path().join("oxplow/extensions").exists());
    }

    #[test]
    fn installs_from_git_and_records_the_source() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        let url = repo.path().to_string_lossy().to_string();

        let ext = install_extension(project.path(), &url, None, &head(repo.path())).unwrap();
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
        install_extension(project.path(), &url, None, &head(repo.path())).unwrap();

        let err = install_extension(project.path(), &url, None, &head(repo.path())).unwrap_err();
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
        let ext = update_extension(project.path(), "shared", &head(repo.path())).unwrap();
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
        let err = update_extension(project.path(), "local", "x").unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("wasn't installed")),
            "{err:?}"
        );
        assert!(matches!(
            update_extension(project.path(), "nope", "x"),
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
        let err = install_extension(project.path(), &repo.path().to_string_lossy(), None, "x")
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("extension.yaml")),
            "{err:?}"
        );
        assert!(!project.path().join("oxplow/extensions").exists());

        let err =
            install_extension(project.path(), "/definitely/not/a/repo", None, "x").unwrap_err();
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
        let err = install_extension(project.path(), &repo.path().to_string_lossy(), None, "x")
            .unwrap_err();
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
        let err = run_lens(
            &sl,
            &cat(),
            dir.path(),
            "gh/all",
            BTreeMap::new(),
            &LensContext::default(),
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("v_gh_pr") && msg.contains("prs") && msg.contains("hasn't synced"),
            "{msg}"
        );
        let e = validate_extension(&sl, &cat(), dir.path(), "gh")
            .await
            .unwrap();
        assert!(e.errors[0].contains("hasn't synced"), "{:?}", e.errors);
    }

    const EXT_V2: &str = "manifest: 2\nname: review\ndescription: Review helpers\nintent:\n  purpose: Review agent work\n  examples:\n    - { name: one, input: {}, expect: {} }\n";

    fn only(root: &Path, name: &str) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == name && e.origin == "project")
            .unwrap()
    }

    #[test]
    fn a_v2_manifest_needs_an_intent_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            "manifest: 2\nname: review\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        let e = only(dir.path(), "review");
        assert_eq!(e.manifest_version, 2);
        assert!(
            e.errors.iter().any(
                |m| m.starts_with("oxplow/extensions/review/extension.yaml:1:")
                    && m.contains("`intent` is required")
            ),
            "{:?}",
            e.errors
        );
        // The lenses still load: an intent problem is the manifest's.
        assert_eq!(e.lenses.len(), 1);
        // A clean v2 manifest has no errors and no warnings.
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            EXT_V2,
        );
        let e = only(dir.path(), "review");
        assert!(
            e.errors.is_empty() && e.warnings.is_empty(),
            "{:?} {:?}",
            e.errors,
            e.warnings
        );
        assert_eq!(e.sharing, Sharing::Private);
        assert_eq!(e.intent.as_ref().unwrap().purpose, "Review agent work");
    }

    #[test]
    fn a_shared_manifest_may_not_use_an_experimental_kind() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = "manifest: 2\nname: review\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nref_kinds:\n  - kind: ticket\n";
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            manifest,
        );
        let e = only(dir.path(), "review");
        let err = e
            .errors
            .iter()
            .find(|m| m.contains("`ref_kinds` is experimental"))
            .unwrap_or_else(|| panic!("{:?}", e.errors));
        assert!(
            err.starts_with("oxplow/extensions/review/extension.yaml:8:"),
            "{err}"
        );
        assert_eq!(e.sharing, Sharing::Shared);
    }

    #[test]
    fn a_slot_mount_to_a_missing_lens_names_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!("{EXT_V2}slot_mounts:\n  - {{ slot: rail, lens: tasks }}\n  - {{ slot: task-detail, lens: nope }}\n");
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        let e = only(dir.path(), "review");
        let err = e
            .errors
            .iter()
            .find(|m| m.contains("mounts lens `nope`"))
            .unwrap_or_else(|| panic!("{:?}", e.errors));
        assert!(
            err.starts_with("oxplow/extensions/review/extension.yaml:10:"),
            "{err}"
        );
    }

    #[test]
    fn a_metric_naming_a_measure_from_elsewhere_is_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{EXT_V2}measures:\n  - {{ key: review.todo, title: TODOs, unit: count, subjectKind: file }}\nmetrics:\n  - {{ key: review.todo_total, title: TODO total, sourceMeasure: review.todo, aggregation: sum }}\n  - {{ key: review.bad, title: Bad, sourceMeasure: review.missing, aggregation: sum }}\n"
        );
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        let e = only(dir.path(), "review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        let warn = e
            .warnings
            .iter()
            .find(|m| m.contains("sourceMeasure `review.missing`"))
            .unwrap_or_else(|| panic!("{:?}", e.warnings));
        assert!(warn.contains("extension.yaml:"), "{warn}");
        assert_eq!(e.metrics.len(), 2, "both specs still load");
    }

    #[test]
    fn a_v1_manifest_still_loads_with_a_migration_warning() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            "name: review\ndescription: Review helpers\nslots:\n  - { slot: task-detail, lens: tasks }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            "title: Task\nquery: SELECT id FROM v_task WHERE id = :task_id\nparams: [{ name: task_id }]\n",
        );
        let e = only(dir.path(), "review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.manifest_version, 1);
        // The migrator's skeleton intent: purpose from the description,
        // examples left for the agent to fill in.
        let intent = e.intent.as_ref().expect("skeleton intent");
        assert_eq!(intent.purpose, "Review helpers");
        assert!(intent.examples.is_empty());
        assert!(
            e.warnings
                .iter()
                .any(|w| w.contains("manifest v1") && w.contains("plugin migrate")),
            "{:?}",
            e.warnings
        );
        assert_eq!(e.slots.len(), 1, "v1 `slots` become slot mounts");
    }

    /// Rewriting a manifest from v1 to v2 must not ask the person to
    /// approve the extension's programs again: neither hash reads the
    /// manifest's shape.
    #[test]
    fn migrating_a_manifest_to_v2_keeps_the_consent_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let ext_rel = "oxplow/extensions/review";
        let source = "  - id: github\n    runtime: exec\n    entry: bin/sync.sh\n    env: [GITHUB_TOKEN]\n    network: [api.github.com]\n    entities:\n      - name: pr\n        key: number\n        columns:\n          number: int\n";
        let advisory =
            "  - id: coverage-target\n    on: post-tool-use\n    query: SELECT 'x' AS message\n";
        write(
            dir.path(),
            &format!("{ext_rel}/bin/sync.sh"),
            "#!/bin/sh\necho '{}'\n",
        );
        write(
            dir.path(),
            &format!("{ext_rel}/extension.yaml"),
            &format!("name: review\ndescription: d\nsources:\n{source}advisories:\n{advisory}"),
        );
        let v1 = only(dir.path(), "review");
        assert!(v1.errors.is_empty(), "{:?}", v1.errors);
        assert_eq!(v1.manifest_version, 1);
        let ext_dir = dir.path().join(ext_rel);
        let source_hash_v1 = crate::source_runner::approval_hash(&ext_dir, &v1.sources[0]).unwrap();
        let advisory_hash_v1 = crate::exec_consent::advisory_program(&v1)
            .hash(Path::new(""))
            .unwrap();

        write(
            dir.path(),
            &format!("{ext_rel}/extension.yaml"),
            &format!(
                "manifest: 2\nname: review\ndescription: d\nintent:\n  purpose: Pull requests and coverage nudges\n  examples: [{{ name: a }}]\ncollectors:\n{source}advisories:\n{advisory}"
            ),
        );
        let v2 = only(dir.path(), "review");
        assert!(v2.errors.is_empty(), "{:?}", v2.errors);
        assert!(v2.warnings.is_empty(), "{:?}", v2.warnings);
        assert_eq!(v2.manifest_version, 2);
        assert_eq!(v2.sources, v1.sources);
        assert_eq!(v2.advisories, v1.advisories);
        assert_eq!(
            crate::source_runner::approval_hash(&ext_dir, &v2.sources[0]).unwrap(),
            source_hash_v1
        );
        assert_eq!(
            crate::exec_consent::advisory_program(&v2)
                .hash(Path::new(""))
                .unwrap(),
            advisory_hash_v1
        );
    }

    #[test]
    fn a_grid_child_in_another_extension_resolves_once_everything_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            EXT_V2,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/board/extension.yaml",
            "manifest: 2\nname: board\nintent:\n  purpose: x\n  examples: [{ name: a }]\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/board/lenses/all.yaml",
            "title: All\nquery: SELECT 1\nviz: grid\nchildren: [review/tasks, review/missing]\n",
        );
        let board = only(dir.path(), "board");
        assert!(
            board
                .errors
                .iter()
                .any(|m| m.contains("child lens `review/missing` isn't in any loaded extension")),
            "{:?}",
            board.errors
        );
        assert!(board.lenses.is_empty(), "the broken grid is dropped");
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
        // Bundled manifests are v2, shared, with an intent, and clean.
        for e in exts.iter().filter(|e| e.origin == "bundled") {
            assert!(e.warnings.is_empty(), "{}: {:?}", e.name, e.warnings);
            assert_eq!(e.manifest_version, 2, "{}", e.name);
            assert_eq!(e.sharing, Sharing::Shared, "{}", e.name);
            assert!(
                !e.intent.as_ref().unwrap().examples.is_empty(),
                "{}",
                e.name
            );
        }
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
        // The analytics extension's advisory and lens SQL runs too.
        let a = validate_extension(&layer().await, &cat(), dir.path(), "oxplow-analytics")
            .await
            .unwrap();
        assert!(a.errors.is_empty(), "{:?}", a.errors);
        // Every bundled lens's SQL runs against a real schema.
        let v = validate_extension(&layer().await, &cat(), dir.path(), "oxplow-review")
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(
            find_lens(&cat(), dir.path(), "oxplow-review/decisions")
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
            find_lens(&cat(), dir.path(), "oxplow-review/decisions")
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

    #[test]
    fn the_settings_slot_takes_parameterless_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/mine/extension.yaml",
            "name: mine\nslots:\n  - { slot: settings, lens: status }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/mine/lenses/status.yaml",
            "title: Status\nquery: SELECT 1\n",
        );
        let e = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "mine")
            .unwrap();
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.slots[0].slot, "settings");
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
        let v = validate_extension(&layer().await, &cat(), d.path(), "x")
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
                "title: L\nquery: SELECT 1\ncolumns:\n  - { key: a, link: { kind: commit } }\n  - { key: b, link: { kind: metric } }\n  - { key: p, link: { kind: page } }\n  - { key: d, link: { kind: file, line: n } }\n",
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
                LensLinkKind::Page,
                LensLinkKind::File
            ]
        );
        assert_eq!(
            ext.lenses[0].columns[3]
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
    fn extensions_declare_measures_metrics_and_gauges() {
        let manifest = [
            "measures:",
            "  - { key: acme.todo, title: TODOs }",
            "metrics:",
            "  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum }",
            "  - { use: oxplow.rust.unsafe_blocks }",
            "gauges:",
            "  - key: acme.missing",
            "    emits: [acme.todo]",
            "    compute: { runtime: starlark, entryFile: gauges/nope.star }",
            "  - key: acme.shell",
            "    emits: [acme.todo]",
            "    compute: { runtime: exec, entryFile: gauges/todo.star }",
            "",
        ]
        .join("\n");
        let (_d, ext) = load_x(&[], &manifest);
        assert_eq!(ext.measures.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.metrics.len(), 1, "{:?}", ext.errors);
        // A missing script and an `exec` gauge are refused; so is a `use:`,
        // which only a project can write.
        assert!(ext.gauges.is_empty());
        for (needle, also) in [
            ("acme.missing", "gauges/nope.star"),
            ("acme.shell", "exec"),
            ("use: oxplow.rust.unsafe_blocks", "project.yaml"),
        ] {
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(needle) && e.contains(also)),
                "{needle}: {:?}",
                ext.errors
            );
        }
    }

    #[test]
    fn extensions_declare_dimensions_but_not_promoted_ones() {
        let manifest = [
            "dimensions:",
            "  - { key: acme.team, label: Team }",
            "  - { key: acme.prio, entity: v_task, expr: e.priority }",
            "  - { key: acme.hot, promote: true }",
            "",
        ]
        .join("\n");
        let (_d, ext) = load_x(&[], &manifest);
        let keys: Vec<_> = ext
            .dimensions
            .iter()
            .filter_map(|d| d.key.clone())
            .collect();
        assert_eq!(keys, vec!["acme.team", "acme.prio"], "{:?}", ext.errors);
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("acme.hot") && e.contains("promote")),
            "{:?}",
            ext.errors
        );
    }

    #[test]
    fn an_extension_gauge_runs_its_own_script() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            "name: x\ngauges:\n  - key: acme.todo_scan\n    emits: [acme.todo]\n    compute: { runtime: starlark, entryFile: gauges/todo.star }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/x/gauges/todo.star",
            "def run(ctx):\n    return []\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.gauges.len(), 1);
        assert_eq!(
            read_extension_file(dir.path(), "x", "gauges/todo.star").as_deref(),
            Some("def run(ctx):\n    return []\n")
        );
        assert_eq!(
            read_extension_file(dir.path(), "x", "gauges/none.star"),
            None
        );
    }

    #[test]
    fn alerts_take_a_row_count_or_a_value_threshold() {
        let (_d, ext) = load_x(
            &[
                (
                    "rows",
                    "title: R\nquery: SELECT 1\nalert: { min_rows: 2, label: Too many }\n",
                ),
                (
                    "value",
                    "title: V\nquery: SELECT 1 AS n\nalert: { column: n, above: 5 }\n",
                ),
                (
                    "both",
                    "title: B\nquery: SELECT 1 AS n\nalert: { min_rows: 1, column: n, above: 5 }\n",
                ),
                (
                    "nothing",
                    "title: N\nquery: SELECT 1 AS n\nalert: { column: n }\n",
                ),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        let rows = lens("rows").unwrap().alert.clone().unwrap();
        assert_eq!(
            (rows.min_rows, rows.label.as_deref()),
            (Some(2), Some("Too many"))
        );
        let value = lens("value").unwrap().alert.clone().unwrap();
        assert_eq!(
            (value.column.as_deref(), value.above),
            (Some("n"), Some(5.0))
        );
        assert!(lens("both").is_none() && lens("nothing").is_none());
        assert_eq!(
            ext.errors.iter().filter(|e| e.contains("alert")).count(),
            2,
            "{:?}",
            ext.errors
        );
    }

    #[tokio::test]
    async fn a_run_carries_its_alert_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/rows.yaml",
            "title: Rows\nparams: [{ name: n, default: 0 }]\nquery: SELECT value FROM json_each('[1,2,3]') WHERE value <= :n\nalert: { min_rows: 2 }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/cov.yaml",
            "title: Cov\nparams: [{ name: v, default: 90 }]\nquery: SELECT :v AS pct\nalert: { column: pct, below: 80, label: Coverage low }\n",
        );
        let sl = layer().await;
        let run = |id: &'static str, k: &'static str, v: i64| {
            let sl = &sl;
            let root = dir.path().to_path_buf();
            async move {
                let mut p = BTreeMap::new();
                p.insert(k.to_string(), SqlCell::Int(v));
                run_lens(sl, &cat(), &root, id, p, &LensContext::default())
                    .await
                    .unwrap()
                    .alert
                    .unwrap()
            }
        };
        let a = run("review/rows", "n", 1).await;
        assert!(!a.firing);
        let a = run("review/rows", "n", 3).await;
        assert!(a.firing);
        assert_eq!(a.message, "3 rows");
        let a = run("review/cov", "v", 72).await;
        assert!(a.firing);
        assert_eq!(a.message, "Coverage low: 72");
        assert!(!run("review/cov", "v", 91).await.firing);
    }

    #[test]
    fn rail_lenses_must_declare_an_alert() {
        let (_d, ext) = load_x(
            &[
                ("ok", "title: A\nquery: SELECT 1\nalert: { min_rows: 1 }\n"),
                ("quiet", "title: Q\nquery: SELECT 1\n"),
            ],
            "slots:\n  - { slot: rail, lens: ok }\n  - { slot: rail, lens: quiet }\n",
        );
        assert_eq!(
            ext.slots
                .iter()
                .map(|s| s.lens_id.as_str())
                .collect::<Vec<_>>(),
            vec!["x/ok"]
        );
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("rail") && e.contains("quiet") && e.contains("alert")),
            "{:?}",
            ext.errors
        );
    }

    #[test]
    fn lens_actions_come_from_a_fixed_registry() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 1\nactions:\n  - copy\n  - add-to-context\n  - { action: run-source, source: github/prs, label: Sync PRs }\n",
                ),
                ("b", "title: B\nquery: SELECT 1\nactions: [shell]\n"),
                ("c", "title: C\nquery: SELECT 1\nactions: [{ action: run-source }]\n"),
                ("d", "title: D\nquery: SELECT 1\nactions: [copy, copy]\n"),
                ("e", "title: E\nquery: SELECT 1\nactions: [{ action: copy, source: x/y }]\n"),
            ],
            "",
        );
        let a = ext
            .lenses
            .iter()
            .find(|l| l.slug == "a")
            .expect("valid actions load");
        let got: Vec<_> = a
            .actions
            .iter()
            .map(|x| (x.id.as_str(), x.kind, x.label.as_str(), x.source.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("copy", LensActionKind::Copy, "Copy", None),
                (
                    "add-to-context",
                    LensActionKind::AddToContext,
                    "Add to Agent Context",
                    None
                ),
                (
                    "run-source",
                    LensActionKind::RunSource,
                    "Sync PRs",
                    Some("github/prs")
                ),
            ]
        );
        for (slug, needle) in [
            ("b", "unknown action `shell`"),
            ("c", "needs `source: <extension>/<source>`"),
            ("d", "twice"),
            ("e", "only `run-source` takes a source"),
        ] {
            assert!(
                ext.lenses.iter().all(|l| l.slug != slug),
                "{slug} should fail"
            );
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(needle)),
                "{slug}: {:?}",
                ext.errors
            );
        }
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
            &cat(),
            dir.path(),
            "review/by-status",
            BTreeMap::new(),
            &LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
    }

    #[tokio::test]
    async fn advisories_parse_and_validate_their_columns() {
        let (d, ext) = load_x(
            &[],
            "advisories:\n  - id: low-coverage\n    on: post-tool-use\n    once_per: effort\n    query: SELECT 'add tests' AS message WHERE :effort_id IS NOT NULL\n  - id: crossings\n    on: prompt\n    once_per: row\n    heading: '# Metric thresholds'\n    query: SELECT 'x' AS message, 'k' AS key\n  - id: nokey\n    on: prompt\n    once_per: row\n    query: SELECT 'x' AS message\n  - id: Bad Id\n    on: prompt\n    query: SELECT 1\n",
        );
        let ids: Vec<&str> = ext.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["low-coverage", "crossings", "nokey"]);
        assert_eq!(ext.advisories[0].on, AdvisoryOn::PostToolUse);
        assert_eq!(ext.advisories[1].once_per, AdvisoryOncePer::Row);
        assert_eq!(
            ext.advisories[1].heading.as_deref(),
            Some("# Metric thresholds")
        );
        assert!(ext.errors.join("\n").contains("Bad Id"), "{:?}", ext.errors);

        let v = validate_extension(&layer().await, &cat(), d.path(), "x")
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(errs.contains("nokey") && errs.contains("`key`"), "{errs}");
        assert!(
            !errs.contains("low-coverage") && !errs.contains("crossings"),
            "{errs}"
        );
    }

    #[test]
    fn change_links_and_slots() {
        let (_d, ext) = load_x(
            &[
                (
                    "files",
                    "title: F\nparams: [{ name: change_id }]\nquery: SELECT 1\ncolumns:\n  - { key: path, link: { kind: diff-at, line: ln, base: b, head: h } }\n  - { key: dup, link: { kind: compare, head: h } }\n",
                ),
                ("both", "title: B\nparams: [{ name: effort_id }]\nquery: SELECT 1\n"),
                ("none", "title: N\nparams: [{ name: other }]\nquery: SELECT 1\n"),
            ],
            "slots:\n  - { slot: commit, lens: files }\n  - { slot: uncommitted, lens: files }\n  - { slot: effort-review, lens: files }\n  - { slot: effort-review, lens: both }\n  - { slot: commit, lens: none }\n",
        );
        let link = ext
            .lenses
            .iter()
            .find(|l| l.slug == "files")
            .unwrap()
            .columns[0]
            .link
            .clone()
            .unwrap();
        assert_eq!(link.kind, LensLinkKind::DiffAt);
        assert_eq!(
            (link.base.as_deref(), link.head.as_deref()),
            (Some("b"), Some("h"))
        );
        let mounted: Vec<(&str, &str)> = ext
            .slots
            .iter()
            .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
            .collect();
        assert_eq!(
            mounted,
            vec![
                ("commit", "x/files"),
                ("uncommitted", "x/files"),
                ("effort-review", "x/files"),
                ("effort-review", "x/both")
            ],
            "a slot lens needs at least one of the slot's params"
        );
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("none") && errs.contains("change_id"),
            "{errs}"
        );
    }
}
