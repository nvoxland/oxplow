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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

pub mod custom_components;
pub mod decorators;
pub mod manifest_v2;
pub mod replacements;
pub mod ui_commands;
use manifest_v2::{at, entry_line, key_line, line_under, ManifestV2};
pub use manifest_v2::{
    Intent, IntentExample, IntentPrompt, LauncherEntry, LauncherTarget, Sharing,
};

/// Where project extensions live, relative to a worktree root.
pub const EXTENSIONS_DIR: &str = "oxplow/extensions";

/// How a lens renders its rows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
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
    /// Rows nested by `tree.parent` (a row's parent is the row whose
    /// `tree.id` it names; none, or one not in the result, is a root),
    /// each shown as `tree.label`.
    Tree,
    /// Rows in time order (`timeline.at`), each shown as
    /// `timeline.label`, linked through `timeline.ref` when set.
    Timeline,
    /// The first row as label/value pairs (the displayed columns).
    Detail,
    /// An ordered checklist: `steps.label`, with `steps.status` (`done`,
    /// `active`, `failed`, anything else pending).
    Steps,
    /// Per row, the diff of the file `hunks.path` between the revisions
    /// `hunks.from` and `hunks.to` (`working`, `snap:<id>`,
    /// `<vcs>:<rev>`), read-only.
    Hunks,
    /// A form for command `form.command`: its fields from the command's
    /// input schema, prefilled from `form.defaults` (placeholders bound
    /// like an action's) and the query's first row, if the lens has one.
    /// Submitting runs the command as the lens (P6.B2).
    Form,
    /// The extension's own web component (`custom.component`, P6b.D1), in
    /// a sandboxed frame; its rows are what it shows and what an agent
    /// reads (as a table).
    Custom,
}

/// A `custom` lens's component and the props it starts with.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensCustom {
    /// One of the extension's `custom_components` ids.
    pub component: Option<String>,
    /// Handed to the component as is.
    #[serde(default, serialize_with = "plain_json")]
    #[specta(type = Option<oxplow_domain::Json>)]
    pub props: Option<serde_json::Value>,
}

/// When a lens needs attention: its row count reaches `min_rows`, or the
/// first row's `column` goes `above` / `below` a threshold. A left-nav
/// panel's `badge` lens shows its count, and the Alerts panel lists it.
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
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
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

/// `tree` viz: which columns nest the rows.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensTree {
    pub id: Option<String>,
    pub parent: Option<String>,
    pub label: Option<String>,
}

/// `timeline` viz: when each row happened and what it says.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensTimeline {
    pub at: Option<String>,
    pub label: Option<String>,
    /// A column holding a canonical ref each entry links to.
    #[serde(rename = "ref")]
    pub ref_column: Option<String>,
}

/// `steps` viz: each step's text and status.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensSteps {
    pub label: Option<String>,
    pub status: Option<String>,
}

/// `form` viz: the command it submits and the values it starts from.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensForm {
    pub command: Option<String>,
    /// Input values the form starts with (`{{param.x}}` placeholders bound).
    #[serde(default, serialize_with = "plain_json")]
    #[specta(type = Option<oxplow_domain::Json>)]
    pub defaults: Option<serde_json::Value>,
}

/// Serialize a JSON value with its numbers as plain numbers, so a YAML
/// lens file gets `1`, not serde_json's arbitrary-precision number map.
fn plain_json<S: serde::Serializer>(
    v: &Option<serde_json::Value>,
    s: S,
) -> Result<S::Ok, S::Error> {
    struct Plain<'a>(&'a serde_json::Value);
    impl Serialize for Plain<'_> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::{SerializeMap, SerializeSeq};
            match self.0 {
                serde_json::Value::Null => s.serialize_none(),
                serde_json::Value::Bool(b) => s.serialize_bool(*b),
                serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
                    (Some(i), _) => s.serialize_i64(i),
                    (None, Some(f)) => s.serialize_f64(f),
                    _ => s.serialize_str(&n.to_string()),
                },
                serde_json::Value::String(t) => s.serialize_str(t),
                serde_json::Value::Array(items) => {
                    let mut seq = s.serialize_seq(Some(items.len()))?;
                    for i in items {
                        seq.serialize_element(&Plain(i))?;
                    }
                    seq.end()
                }
                serde_json::Value::Object(map) => {
                    let mut m = s.serialize_map(Some(map.len()))?;
                    for (k, v) in map {
                        m.serialize_entry(k, &Plain(v))?;
                    }
                    m.end()
                }
            }
        }
    }
    match v {
        Some(v) => s.serialize_some(&Plain(v)),
        None => s.serialize_none(),
    }
}

/// `hunks` viz: the file and the two revisions each row diffs.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensHunks {
    pub path: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
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
    /// change's `base_revision` / `head_revision` (join `v_change`).
    DiffAt,
    /// Two line ranges side by side: the value is
    /// `path:start-end|peer:start-end`; `head` names a column with the
    /// revision to read (`working`, `snap:<id>`, `git:<rev>`; the working
    /// tree if absent).
    Compare,
}

/// Makes a column's cells link to a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
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
    /// Required, except by a `form` (whose query, if any, prefills it).
    #[serde(default)]
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
    #[serde(default)]
    tree: Option<LensTree>,
    #[serde(default)]
    timeline: Option<LensTimeline>,
    #[serde(default)]
    steps: Option<LensSteps>,
    #[serde(default)]
    hunks: Option<LensHunks>,
    #[serde(default)]
    form: Option<LensForm>,
    #[serde(default)]
    custom: Option<LensCustom>,
    /// For `grid`: lens slugs in this extension, or `<ext>/<slug>` ids.
    #[serde(default)]
    children: Vec<String>,
    #[serde(default)]
    launcher: Option<LauncherFile>,
    /// Keep it out of the launcher (e.g. a lens only a slot shows).
    #[serde(default)]
    hidden: bool,
    /// Commands the lens offers (`{ id, label, command, input, row }`).
    /// Parsed by [`parse_actions`].
    #[serde(default)]
    actions: Vec<serde_yaml::Value>,
    #[serde(default)]
    alert: Option<AlertFile>,
}

/// A button on a lens: a command it runs (P6.B1, target §11.4). The
/// command goes through the bus as `Actor::Lens` acting for whoever
/// pressed it, so every policy — invokers, the agent policy, confirmation
/// — applies as if they had run it themselves; a lens can offer a button
/// but never grant a power. Copying the result and handing it to the
/// agent are on every lens, not declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensAction {
    /// Unique within the lens; `run_lens_action` names it.
    pub id: String,
    pub label: String,
    /// The command it runs (`work_item.transition`).
    pub command: String,
    /// The command's input. A string that is exactly `{{param.<name>}}`
    /// or `{{row.<column>}}` becomes that value (a number stays a number);
    /// one that contains them has them spliced in as text.
    #[specta(type = oxplow_domain::Json)]
    pub input: serde_json::Value,
    /// A row action: offered on each row (right-click), with `{{row.*}}`
    /// bound to that row. Otherwise it's a button above the result.
    pub row: bool,
}

/// One `{{scope.name}}` placeholder in a string: its scope and name
/// (trimmed; `name` is empty without a `.`) and its byte span, from the
/// opening `{{` to just past the closing `}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub scope: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// The placeholders in `s`, in order — the one tokenizer for lens action
/// templates: load-time validation (`action_templates`) and run-time
/// binding (`lens_actions::bind_input`) both read it. A `{{` without a
/// closing `}}` ends the scan.
pub fn placeholders(s: &str) -> Vec<Placeholder> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = s[at..].find("{{") {
        let start = at + open;
        let Some(close) = s[start..].find("}}") else {
            break;
        };
        let end = start + close + 2;
        let inner = s[start + 2..start + close].trim();
        let (scope, name) = inner.split_once('.').unwrap_or((inner, ""));
        out.push(Placeholder {
            scope: scope.to_string(),
            name: name.to_string(),
            start,
            end,
        });
        at = end;
    }
    out
}

/// The placeholder `s` is exactly (ignoring surrounding whitespace) — a
/// value bound typed rather than spliced into text.
pub fn whole_placeholder(s: &str) -> Option<Placeholder> {
    let t = s.trim();
    match placeholders(t).as_slice() {
        [only] if only.start == 0 && only.end == t.len() => Some(only.clone()),
        _ => None,
    }
}

/// The `{{param.x}}` / `{{row.x}}` placeholders in `input`, as
/// `(scope, name)`, in order.
pub fn action_templates(input: &serde_json::Value) -> Vec<(String, String)> {
    fn walk(v: &serde_json::Value, out: &mut Vec<(String, String)>) {
        match v {
            serde_json::Value::String(s) => {
                out.extend(placeholders(s).into_iter().map(|p| (p.scope, p.name)));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(input, &mut out);
    out
}

/// Parse a lens's `actions:`: each `{ id, label, command, input?, row? }`.
/// A placeholder must name a declared param (`{{param.x}}`) or, in a row
/// action, a column (`{{row.x}}`, checked against the result when the
/// extension is validated).
fn parse_actions(
    raw: Vec<serde_yaml::Value>,
    params: &[LensParam],
) -> Result<Vec<LensAction>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Full {
        id: String,
        label: String,
        command: String,
        #[serde(default)]
        input: Option<serde_json::Value>,
        #[serde(default)]
        row: bool,
    }
    let mut out: Vec<LensAction> = Vec::new();
    for v in raw {
        let f = serde_yaml::from_value::<Full>(v).map_err(|e| format!("actions: {e}"))?;
        if f.id.trim().is_empty() {
            return Err("actions: an action needs an `id`".into());
        }
        if out.iter().any(|a| a.id == f.id) {
            return Err(format!("actions: `{}` is declared twice", f.id));
        }
        oxplow_domain::CommandSpec::validate_name(&f.command)
            .map_err(|e| format!("actions: `{}`: {e}", f.id))?;
        let input = f.input.unwrap_or_else(|| serde_json::json!({}));
        if !input.is_object() {
            return Err(format!(
                "actions: `{}`: `input` must be a map (the command's input)",
                f.id
            ));
        }
        for (scope, name) in action_templates(&input) {
            match scope.as_str() {
                "param" if params.iter().any(|p| p.name == name) => {}
                "param" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{param.{name}}}}}` names no param of this lens",
                        f.id
                    ))
                }
                "row" if f.row => {}
                "row" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{row.{name}}}}}` needs `row: true`",
                        f.id
                    ))
                }
                other => {
                    return Err(format!(
                        "actions: `{}`: `{{{{{other}…}}}}` isn't `{{{{param.<name>}}}}` or \
                         `{{{{row.<column>}}}}`",
                        f.id
                    ))
                }
            }
        }
        out.push(LensAction {
            id: f.id,
            label: f.label,
            command: f.command,
            input,
            row: f.row,
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
/// Slot names are one dotted namespace, `<capability>.<page>.<region>`
/// (P6b): what a page shows a lens is named by what the page is about.
pub const SLOTS: &[(&str, &[&str])] = &[
    // An effort's review (its diff view).
    ("effort.review.details", &["effort_id", "change_id"]),
    // A work item's page, below its body; `task_id` is null for an item
    // that isn't an oxplow task.
    ("work_item.detail.body", &["ref", "task_id"]),
    // The same page's side rail.
    ("work_item.detail.sidebar", &["ref", "task_id"]),
    // A thread's plan, as a compact strip.
    ("thread.plan.header", &["thread_id"]),
    ("vcs.commit.details", &["change_id"]),
    // Uncommitted changes: a strip above them, and below the files.
    ("vcs.status.header", &["stream_id"]),
    ("vcs.status.details", &["change_id"]),
    // Git history's side column.
    ("vcs.history.sidebar", &["stream_id"]),
    // A file diff's header strip, above the editor: the file and the two
    // revisions the diff reads (`working`, `snap:<id>`, `git:<sha>`).
    (
        "diff.file.header",
        &["path", "left_revision", "right_revision", "stream_id"],
    ),
    // Settings: a section per extension with the lenses it mounts (its
    // own status or configuration views). No params.
    ("settings.section", &[]),
];

/// What a left-nav panel is bound to: the project, the current stream, or
/// the current thread — which of `stream_id` / `thread_id` its lenses get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum PanelScope {
    Project,
    Stream,
    Thread,
}

/// A left-nav panel an extension contributes (P6.G1, target §11.3): its
/// `body` lens renders compact in the nav; its `badge` lens's alert gives
/// the count shown on the panel and in the Alerts panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPanel {
    /// `<extension>/<panel>`.
    pub id: String,
    pub extension: String,
    pub title: String,
    pub icon: Option<String>,
    pub scope: PanelScope,
    /// The body lens (`<extension>/<slug>`).
    pub body: String,
    /// The badge lens, which declares an `alert`.
    pub badge: Option<String>,
}

/// A page an extension contributes (P6.G2, target §11.3): a lens shown
/// full-page at `page:ext.<extension>.<id>`, listed in the launcher under
/// its category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPage {
    pub id: String,
    pub extension: String,
    /// Its tab id: `page:ext.<extension>.<id>`.
    pub page_ref: String,
    pub title: String,
    pub icon: Option<String>,
    pub category: LauncherCategory,
    /// The lens it shows (`<extension>/<slug>`).
    pub lens: String,
}

/// A `pages:` entry as the manifest holds it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageFile {
    id: String,
    title: String,
    #[serde(default)]
    icon: Option<String>,
    category: LauncherCategory,
    lens: String,
}

/// A `panels:` entry as the manifest holds it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanelFile {
    id: String,
    title: String,
    #[serde(default)]
    icon: Option<String>,
    scope: PanelScope,
    body: String,
    #[serde(default)]
    badge: Option<String>,
}

/// A lens an extension mounts into a core page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensSlot {
    /// Which page region, from [`SLOTS`] (`effort.review.details`,
    /// `vcs.commit.details`, …); the lens takes the params it offers.
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
    pub tree: Option<LensTree>,
    pub timeline: Option<LensTimeline>,
    pub steps: Option<LensSteps>,
    pub hunks: Option<LensHunks>,
    pub form: Option<LensForm>,
    /// For `custom`: the component and its props.
    pub custom: Option<LensCustom>,
    /// For `grid`: child lens ids.
    pub children: Vec<String>,
    /// Launcher section; `None` = "Lenses".
    pub launcher_category: Option<LauncherCategory>,
    /// Not listed in the launcher.
    pub hidden: bool,
    /// Commands the lens offers, as buttons or row actions (P6.B1).
    pub actions: Vec<LensAction>,
    /// When the lens needs attention (its panel's badge).
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
    /// Things worth fixing that don't stop it loading: an intent with no
    /// examples.
    pub warnings: Vec<String>,
    /// The manifest version it was read as (`manifest:`).
    pub manifest_version: u32,
    pub sharing: Sharing,
    /// Why it exists (required; `None` only when the manifest didn't load).
    pub intent: Option<Intent>,
    pub lenses: Vec<Lens>,
    /// Where it was installed from, for extensions added with
    /// `install_extension`; `None` for ones written in this repo.
    pub source: Option<ExtensionSource>,
    /// Declared collectors (valid ones; invalid ones are in `errors`).
    pub collectors: Vec<oxplow_config::collectors::CollectorSpec>,
    /// Declared providers (experimental: a private extension's only;
    /// valid ones — invalid ones are in `errors`).
    pub providers: Vec<crate::providers::ProviderSpec>,
    /// Web components its `custom` lenses render, sandboxed (experimental:
    /// a private extension's only; valid ones).
    pub custom_components: Vec<custom_components::CustomComponent>,
    /// Event types it declares (experimental: a private extension's only;
    /// valid ones — the vocabulary registers them, `vocabulary_reactor`).
    pub event_types: crate::extension_event_types::EventTypes,
    /// Ref kinds it declares (experimental: a private extension's only;
    /// valid ones — the vocabulary registers them, `vocabulary_reactor`).
    pub ref_kinds: Vec<crate::extension_ref_kinds::RefKindDecl>,
    /// Effects it declares (experimental: a private extension's only;
    /// valid ones — each runs only once a person approves it, `effects`).
    pub effects: Vec<crate::effects::EffectDecl>,
    /// Another extension's event types its effects and collectors react
    /// to (P9.D1): each resolves when an enabled extension registers the
    /// type (`vocabulary_reactor`).
    pub subscriptions: Vec<ForeignSubscription>,
    /// `project` (in `oxplow/extensions/`) or `bundled` (ships with oxplow,
    /// read-only).
    pub origin: String,
    /// What it adds to the core UI (`ui:`).
    pub ui: ExtensionUi,
    /// False when `.oxplow/project.yaml` disables it; a disabled
    /// extension has no lenses, slots, sources or advisories.
    pub enabled: bool,
    /// Guidance for the coding agent (valid ones; invalid ones are in `errors`).
    pub advisories: Vec<Advisory>,
    /// Measures and metrics it contributes to the metric catalog (the
    /// `project.yaml` schema). Metrics are `key:` definitions and are on
    /// while the extension is enabled; the facts they read come from its
    /// collectors (`collectors:` with `facts:`).
    pub measures: Vec<oxplow_config::MeasureEntry>,
    pub metrics: Vec<oxplow_config::MetricEntry>,
    /// Dimensions it contributes (fact or entity), never promoted: an
    /// extension toggling would rebuild the metric cube each time.
    pub dimensions: Vec<oxplow_config::DimensionEntry>,
    /// Its SQL models (`models:` plus `models/<name>.sql`), published as
    /// `v_<extension>_<name>` (`extension_models`). Empty when any
    /// declaration is broken (see `errors`).
    pub models: Vec<oxplow_db::models::ModelSource>,
    /// Left-nav panels it contributes (valid ones; invalid ones are in
    /// `errors`).
    pub panels: Vec<ExtensionPanel>,
    /// Full pages it contributes (valid ones; invalid ones are in `errors`).
    pub pages: Vec<ExtensionPage>,
    /// Launcher entries for what isn't a lens: a page, a command, a
    /// prompt (P6.D1; valid ones — invalid ones are in `errors`).
    pub launcher: Vec<LauncherEntry>,
    /// Commands it registers on the bus, each a Starlark script composing
    /// core commands (P6b; valid ones — invalid ones are in `errors`).
    pub commands: Vec<crate::extension_commands::ExtensionCommand>,
}

/// What an extension adds to the core UI (`ui:` in its manifest).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUi {
    /// Lenses mounted into core pages (valid ones).
    pub slots: Vec<LensSlot>,
    /// Commands in core menus, for a page's or a row's ref (valid ones).
    pub commands: Vec<ui_commands::UiCommand>,
    /// Labels from its models on core refs (valid ones).
    pub decorators: Vec<decorators::UiDecorator>,
    /// Lenses that take the place of a core sub-component while its
    /// provider is the capability's active one (experimental: a private
    /// extension's only; valid ones).
    pub replacements: Vec<replacements::UiReplacement>,
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
    /// What's worth knowing about its data: a view it read comes from a
    /// collector failures disabled (P7.C2), so it isn't being refreshed.
    pub warnings: Vec<String>,
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
    let mut out: Vec<Extension> = out
        .into_iter()
        .map(|e| apply_disabled(e, &disabled))
        .collect();
    crate::extension_commands::refuse_shared_namespaces(&mut out);
    out
}

/// A `measures:` / `metrics:` block as typed entries.
fn parse_block<T: serde::de::DeserializeOwned>(
    v: Option<serde_yaml::Value>,
) -> Result<Option<Vec<T>>, String> {
    v.map(|v| serde_yaml::from_value(v).map_err(|e| e.to_string()))
        .transpose()
}

/// Project extension `name` alone, from `root/oxplow/extensions/<name>`
/// — a tree laid out like a project, e.g. a provider's approved copy.
pub fn load_project_extension(root: &Path, name: &str) -> Extension {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    load_one(&Disk(root.join(&rel)), name, &rel, "project")
}

/// A file inside extension `name` (bundled or in `oxplow/extensions/`),
/// e.g. a collector's script. `None` when there's no such extension or file.
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

/// The files at an extension's `path` as a listing gives it: a bundled
/// extension's embedded files for `bundled:<name>`, else the folder
/// `project_dir/path` on disk — one resolver, so what an approval covers is
/// read the same way wherever the extension lives (tsk953).
pub(crate) fn files_at(project_dir: &Path, path: &str) -> std::io::Result<Box<dyn ExtensionFiles>> {
    match path.strip_prefix("bundled:") {
        Some(name) => crate::bundled_extensions::BUNDLED
            .iter()
            .find(|b| b.name == name.trim_end_matches('/'))
            .map(|b| Box::new(Embedded(b)) as Box<dyn ExtensionFiles>)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no bundled extension `{name}`"),
                )
            }),
        None => Ok(Box::new(Disk(project_dir.join(path)))),
    }
}

/// Where an extension's files come from: a folder on disk, a bundled
/// extension's embedded files, or one revision's in memory.
pub(crate) trait ExtensionFiles {
    /// Contents of a file, by path inside the extension folder.
    fn read(&self, rel: &str) -> Option<String>;
    /// Every file's path inside the extension folder (`/`-separated), what
    /// an approval covers. On disk, dot-files included and macOS's
    /// `.DS_Store` left out; a symlink is an error — its target isn't what
    /// was approved, and can change freely.
    fn paths(&self) -> std::io::Result<Vec<String>>;
    /// A file's bytes, by path inside the extension folder.
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>>;
    /// File names directly inside `dir` (e.g. `lenses`).
    fn list(&self, dir: &str) -> Vec<String>;
    /// What the folder `rel` holds, for a custom component's bundle;
    /// `None` when it isn't a folder (or the files aren't on disk).
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook;
}

pub(crate) struct Disk(pub(crate) std::path::PathBuf);

impl ExtensionFiles for Disk {
    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(rel)).ok()
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&self.0).follow_links(false) {
            let entry = entry.map_err(std::io::Error::other)?;
            if entry.path_is_symlink() {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink; a program's folder can't contain symlinks to be approved \
                     (copy the file in instead)",
                    entry.path().display()
                )));
            }
            if entry.file_type().is_file() && entry.file_name() != ".DS_Store" {
                let rel = entry.path().strip_prefix(&self.0).unwrap_or(entry.path());
                out.push(rel.to_string_lossy().into_owned());
            }
        }
        Ok(out)
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.0.join(rel))
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
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook {
        match custom_components::stat_bundle(&self.0.join(rel)) {
            Some(stat) => custom_components::BundleLook::Found(stat),
            None => custom_components::BundleLook::Absent,
        }
    }
}

/// An extension's files held in memory — one revision's (P8.C1): paths
/// inside the extension folder → their text.
pub struct Tree(std::collections::BTreeMap<String, String>);

impl Tree {
    pub fn new(files: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(files.into_iter().collect())
    }

    /// A file's text, by path inside the extension folder.
    pub fn file(&self, rel: &str) -> Option<String> {
        self.0.get(rel).cloned()
    }

    /// The extension these files load as, its paths shown under `rel`.
    pub fn load(&self, name: &str, rel: &str) -> Extension {
        load_one(self, name, rel, "project")
    }
}

impl ExtensionFiles for Tree {
    fn read(&self, rel: &str) -> Option<String> {
        self.0.get(rel).cloned()
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        Ok(self.0.keys().cloned().collect())
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        self.0
            .get(rel)
            .map(|t| t.clone().into_bytes())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel.to_string()))
    }
    /// What `dir` holds directly: its files, and the folders on the way to
    /// deeper ones, as a directory listing would show them.
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        self.0
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    /// A bundle checked from the tree's own paths, when it holds them; a
    /// built bundle usually isn't committed, so none is `Unknown`, not an
    /// error (tsk784).
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook {
        custom_components::look_in_paths(rel, self.0.iter().map(|(p, b)| (p.as_str(), b.len())))
    }
}

/// Project extension `name` as it is at `rev` of the workspace `ws` — a
/// commit, a snapshot, the working tree — through `Trees` (P8.C1). `None`
/// when that revision has no `oxplow/extensions/<name>/extension.yaml`.
pub async fn extension_at(
    trees: &crate::trees::Trees,
    ws: &Path,
    rev: &oxplow_domain::vcs::Revision,
    name: &str,
) -> Result<Option<Extension>, DomainError> {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    Ok(extension_tree_at(trees, ws, rev, name)
        .await?
        .map(|tree| tree.load(name, &rel)))
}

/// Project extension `name`'s files at `rev` (P8.C1): `None` when that
/// revision has no `extension.yaml` for it.
pub async fn extension_tree_at(
    trees: &crate::trees::Trees,
    ws: &Path,
    rev: &oxplow_domain::vcs::Revision,
    name: &str,
) -> Result<Option<Tree>, DomainError> {
    let prefix = format!("{EXTENSIONS_DIR}/{name}/");
    let files = trees
        .corpus(ws, rev, |p| p.starts_with(&prefix))
        .await?
        .into_iter()
        .filter_map(|(path, text)| Some((path.strip_prefix(&prefix)?.to_string(), text)));
    let tree = Tree::new(files);
    Ok(tree.read("extension.yaml").is_some().then_some(tree))
}

/// One extension that changed between two revisions of a workspace
/// (P8.C7): what an effort's review shows for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionChange {
    pub name: String,
    /// `added`, `removed` or `changed`.
    pub change: crate::extension_effects::Change,
    /// What the change does; `None` for a removed extension.
    pub effects: Option<crate::extension_effects::EffectReport>,
    /// What's wrong with the later version.
    pub errors: Vec<String>,
}

/// The extensions under `oxplow/extensions/` whose files differ between
/// `start` (none: nothing before) and `end` of workspace `ws`, each with
/// what the change does — both versions loaded as their revision holds
/// them and reviewed on their own overlays.
pub async fn extension_changes_between(
    svc: &crate::Services,
    ws: &Path,
    start: Option<&oxplow_domain::vcs::Revision>,
    end: &oxplow_domain::vcs::Revision,
) -> Result<Vec<ExtensionChange>, DomainError> {
    let prefix = format!("{EXTENSIONS_DIR}/");
    let names: BTreeSet<String> = svc
        .trees
        .diff(ws, start, end)
        .await?
        .into_iter()
        .filter_map(|e| {
            let rest = e.path.strip_prefix(&prefix)?;
            let (name, _) = rest.split_once('/')?;
            Some(name.to_string())
        })
        .collect();
    let mut out = Vec::new();
    for name in names {
        let rel = format!("{EXTENSIONS_DIR}/{name}");
        let before = match start {
            Some(rev) => extension_tree_at(&svc.trees, ws, rev, &name).await?,
            None => None,
        };
        let after = extension_tree_at(&svc.trees, ws, end, &name).await?;
        let Some(after) = after else {
            if before.is_some() {
                out.push(ExtensionChange {
                    name,
                    change: crate::extension_effects::Change::Removed,
                    effects: None,
                    errors: Vec::new(),
                });
            }
            continue;
        };
        let read_before = |file: &str| before.as_ref().and_then(|t| t.file(file));
        let read_after = |file: &str| after.file(file);
        let mut side = ReviewSide {
            extension: after.load(&name, &rel),
            read: &read_after,
        };
        let effects = effects_between(
            &svc.sql,
            &svc.extension_catalog,
            ws,
            before.as_ref().map(|t| ReviewSide {
                extension: t.load(&name, &rel),
                read: &read_before,
            }),
            &mut side,
            svc.commands.as_ref(),
        )
        .await;
        out.push(ExtensionChange {
            change: if before.is_some() {
                crate::extension_effects::Change::Changed
            } else {
                crate::extension_effects::Change::Added
            },
            name,
            effects: Some(effects),
            errors: side.extension.errors,
        });
    }
    Ok(out)
}

/// One version of an extension under review: as loaded, and how to read
/// its files.
pub struct ReviewSide<'a> {
    pub extension: Extension,
    pub read: &'a (dyn Fn(&str) -> Option<String> + Sync),
}

/// What going from `before` (none: a first install) to `after` would
/// change (P8.C2–C5): each side on its own overlay — `after` checked,
/// `before` only read — then compared: lenses, models and their rows,
/// collectors and their outputs, providers, config. `after.extension`
/// gets its check's errors.
pub async fn effects_between(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    before: Option<ReviewSide<'_>>,
    after: &mut ReviewSide<'_>,
    commands: CommandSchemas<'_>,
) -> crate::extension_effects::EffectReport {
    let prepared_after = prepare(layer, catalog, root, &mut after.extension, Some(commands)).await;
    let prepared_before = match &before {
        Some(b) => Some(read_side(layer, catalog, root, &b.extension).await),
        None => None,
    };
    let no_runs = LensRuns::new();
    crate::extension_effects::effects(
        layer,
        before.as_ref().map(|b| crate::extension_effects::Version {
            extension: &b.extension,
            read: b.read,
            lenses: prepared_before.as_ref().map_or(&no_runs, |p| &p.lenses),
            overlay: prepared_before
                .as_ref()
                .map_or(&[], |p| p.overlay.as_slice()),
        }),
        crate::extension_effects::Version {
            extension: &after.extension,
            read: after.read,
            lenses: &prepared_after.lenses,
            overlay: &prepared_after.overlay,
        },
    )
    .await
}

pub(crate) struct Embedded(pub(crate) &'static crate::bundled_extensions::BundledExtension);

impl ExtensionFiles for Embedded {
    fn read(&self, rel: &str) -> Option<String> {
        self.0
            .files
            .iter()
            .find(|(p, _)| *p == rel)
            .map(|(_, c)| (*c).to_string())
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        Ok(self.0.files.iter().map(|(p, _)| (*p).to_string()).collect())
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        self.read(rel)
            .map(String::into_bytes)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel.to_string()))
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
    /// A bundled extension is shared: it has no custom components.
    fn bundle_stat(&self, _rel: &str) -> custom_components::BundleLook {
        custom_components::BundleLook::Absent
    }
}

/// An effect's or collector's `on:` naming an event type of another
/// extension's namespace (P9.D1). The type's name is the dependency —
/// there is no `depends:` key, as a model's `ref('<ext>/<name>')` has
/// none: it resolves when an enabled extension registers the type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ForeignSubscription {
    /// What reacts: "effect `note`", "collector `pull`".
    pub by: String,
    pub event_type: String,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

impl ForeignSubscription {
    /// What loading says: not an error — the owner may not be here yet.
    fn warning(&self) -> String {
        format!(
            "{}: {} reacts to `{}`, another extension's event type; it runs once an enabled \
             extension registers that type (`v_event_type`)",
            self.declared_at, self.by, self.event_type
        )
    }
}

/// How an `on:` type stands for the extension declaring the reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subscribes {
    /// A core type, or one it declares.
    Known,
    /// One in another extension's namespace: known when that one is here.
    Foreign,
    /// No type at all: a core namespace's that doesn't exist, its own
    /// that it doesn't declare, a name that isn't a type's.
    Unknown,
}

fn subscribes(
    extension: &str,
    declared: &[crate::extension_event_types::EventTypeDecl],
    event_type: &str,
) -> Subscribes {
    use oxplow_domain::events::schema::{is_core_type, plugin_namespace, plugin_type_namespace};
    if is_core_type(event_type) || declared.iter().any(|d| d.event_type == event_type) {
        return Subscribes::Known;
    }
    match plugin_type_namespace(event_type) {
        Some(ns) if ns != plugin_namespace(extension) => Subscribes::Foreign,
        _ => Subscribes::Unknown,
    }
}

pub(crate) fn empty_extension(name: &str, path: &str, origin: &str) -> Extension {
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
        collectors: Vec::new(),
        providers: Vec::new(),
        custom_components: Vec::new(),
        event_types: Default::default(),
        ref_kinds: Vec::new(),
        effects: Vec::new(),
        subscriptions: Vec::new(),
        origin: origin.to_string(),
        ui: ExtensionUi::default(),
        enabled: true,
        advisories: Vec::new(),
        measures: Vec::new(),
        dimensions: Vec::new(),
        models: Vec::new(),
        metrics: Vec::new(),
        launcher: Vec::new(),
        panels: Vec::new(),
        pages: Vec::new(),
        commands: Vec::new(),
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
    // `manifest:` says which shape to read; this oxplow reads one.
    if key_line(&manifest, "manifest").is_none() {
        ext.errors.push(at(
            &file,
            Some(1),
            format!(
                "`manifest: {}` is required: it says which shape the file is in",
                manifest_v2::CURRENT
            ),
        ));
        return ext;
    }
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
    ext.manifest_version = m.manifest;
    ext.sharing = m.sharing;
    ext.intent = m.intent.clone();
    ext.description = m.description.clone();
    let (errors, warnings) = manifest_v2::check(&m, &file, &manifest, origin == "bundled");
    ext.errors.extend(errors);
    ext.warnings.extend(warnings);
    let (launcher, errors) = manifest_v2::launcher_entries(&m, &file, &manifest);
    ext.launcher = launcher;
    ext.errors.extend(errors);
    let panel_files = m.panels.clone();
    let ui_command_files = m.ui.commands.clone();
    let decorator_files = m.ui.decorators.clone();
    // An experimental kind: a shared manifest's is refused by `check`.
    let replacement_files =
        m.ui.replacements
            .clone()
            .filter(|_| m.sharing == Sharing::Private);
    let page_files = m.pages.clone();
    // A stable kind (P10): a shared extension's load too.
    let ref_kind_files = m.ref_kinds.clone();
    let slot_files = {
        // A stable kind (P9.D6): a shared extension's load too.
        if let Some(v) = m.event_types.as_ref() {
            let (declared, errors) = crate::extension_event_types::parse_event_types(
                name,
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
            );
            ext.event_types = declared;
            ext.errors.extend(errors);
        }
        // An experimental kind: a shared manifest's is refused by `check`.
        // After `event_types`: an effect may react to its own types — and
        // to another extension's (P9.D1), which resolves when that one is
        // here (`subscriptions`).
        let declared = ext.event_types.types.clone();
        let subscribable = |t: &str| subscribes(name, &declared, t) != Subscribes::Unknown;
        if let Some(v) = m.effects.as_ref() {
            let (effects, errors) = crate::effects::parse_effects(
                name,
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
                &subscribable,
            );
            for e in &effects {
                for t in &e.on {
                    if subscribes(name, &declared, t) == Subscribes::Foreign {
                        ext.subscriptions.push(ForeignSubscription {
                            by: format!("effect `{}`", e.id),
                            event_type: t.clone(),
                            declared_at: e.declared_at.clone(),
                        });
                    }
                }
            }
            ext.effects = effects;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.collectors {
            // A collector may follow core types, its own extension's, and
            // another's.
            let (collectors, errors) =
                oxplow_config::collectors::parse_collectors(name, v, &subscribable);
            let line = key_line(&manifest, "collectors");
            ext.errors
                .extend(errors.into_iter().map(|e| at(&file, line, e)));
            // A script or program the collector runs must be in the folder,
            // and a Starlark script must parse and define `transform`.
            for c in collectors {
                let c_line = entry_line(&manifest, "collectors", "id", &c.id).or(line);
                match c.entry.as_deref().map(|entry| (entry, files.read(entry))) {
                    Some((entry, None)) => ext.errors.push(at(
                        &file,
                        c_line,
                        format!(
                            "collector `{}`: entry `{entry}` isn't in the extension",
                            c.id
                        ),
                    )),
                    Some((entry, Some(script)))
                        if c.runtime == oxplow_config::collectors::CollectorRuntime::Starlark =>
                    {
                        match oxplow_collect_plugin::runtime::check_starlark(entry, &script) {
                            Ok(()) => ext.collectors.push(c),
                            Err(e) => ext.errors.push(at(
                                &file,
                                c_line,
                                format!("collector `{}`: `{entry}` {e}", c.id),
                            )),
                        }
                    }
                    _ => ext.collectors.push(c),
                }
            }
            for c in &ext.collectors {
                let oxplow_config::collectors::Trigger::On { events, .. } = &c.trigger else {
                    continue;
                };
                for t in events {
                    if subscribes(name, &declared, t) == Subscribes::Foreign {
                        let c_line = entry_line(&manifest, "collectors", "id", &c.id).or(line);
                        ext.subscriptions.push(ForeignSubscription {
                            by: format!("collector `{}`", c.id),
                            event_type: t.clone(),
                            declared_at: at(&file, c_line, "").trim_end_matches(": ").to_string(),
                        });
                    }
                }
            }
        }
        let foreign: Vec<String> = ext
            .subscriptions
            .iter()
            .map(ForeignSubscription::warning)
            .collect();
        ext.warnings.extend(foreign);
        // An experimental kind: a shared manifest's is refused by `check`.
        if let Some(v) = m
            .providers
            .as_ref()
            .filter(|_| m.sharing == Sharing::Private)
        {
            let (providers, errors) = crate::providers::parse_providers(v, &|rel| files.read(rel));
            ext.providers = providers;
            ext.errors.extend(
                errors
                    .into_iter()
                    .map(|e| at(&file, key_line(&manifest, "providers"), e)),
            );
        }
        if let Some(v) = &m.commands {
            let (commands, errors) =
                crate::extension_commands::parse_commands(name, v, &file, &manifest, &|rel| {
                    files.read(rel)
                });
            ext.commands = commands;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.custom_components {
            let (components, errors) =
                custom_components::parse_custom_components(name, v, &file, &manifest, &|rel| {
                    files.bundle_stat(rel)
                });
            ext.custom_components = components;
            ext.errors.extend(errors);
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
        match parse_block::<oxplow_db::models::ModelDecl>(m.models.clone()) {
            Ok(decls) => {
                let dir = format!("{rel}/models");
                let joined = oxplow_db::models::join_sources(
                    decls.unwrap_or_default(),
                    &dir,
                    &file,
                    |sql_file| files.read(&format!("models/{sql_file}")),
                    || {
                        files
                            .list("models")
                            .into_iter()
                            .filter_map(|f| f.strip_suffix(".sql").map(str::to_string))
                            .collect()
                    },
                );
                match joined {
                    Ok(models) => ext.models = models,
                    Err(e) => ext.errors.push(err_at("models", e.to_string())),
                }
            }
            Err(e) => ext.errors.push(err_at("models", format!("models: {e}"))),
        }
        // Cross-references inside the catalog: a metric's source measure
        // and a collector's fact measures are normally ones this extension
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
        for c in &ext.collectors {
            for fact in &c.facts {
                if !measure_known(fact) {
                    ext.warnings.push(err_at(
                        "collectors",
                        format!(
                            "collector `{}`: facts `{fact}`, which is not a measure this extension declares (or a built-in `oxplow.*`); it must come from the project's or another extension's `measures:`",
                            c.id
                        ),
                    ));
                }
            }
        }
        m.ui.slots.clone()
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
                let actions = parse_actions(std::mem::take(&mut l.actions), &l.params)?;
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
                tree: l.tree,
                timeline: l.timeline,
                steps: l.steps,
                hunks: l.hunks,
                form: l.form,
                custom: l.custom,
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

    // A component's assets in this extension must be its lenses.
    let ids: Vec<String> = ext.lenses.iter().map(|l| l.id.clone()).collect();
    let mut missing_assets = Vec::new();
    for c in &ext.custom_components {
        let prefix = format!("{name}/");
        if let Some(a) = c
            .assets
            .iter()
            .find(|a| a.starts_with(&prefix) && !ids.contains(a))
        {
            ext.errors.push(at(
                &file,
                entry_line(&manifest, "custom_components", "id", &c.id),
                format!(
                    "custom component `{}`: asset `{a}` isn't in this extension's lenses/",
                    c.id
                ),
            ));
            missing_assets.push(c.id.clone());
        }
    }
    ext.custom_components
        .retain(|c| !missing_assets.contains(&c.id));

    // Drop lenses whose viz lacks what it needs, so every loaded lens renders.
    let components: Vec<String> = ext.custom_components.iter().map(|c| c.id.clone()).collect();
    let mut bad = Vec::new();
    for l in &ext.lenses {
        if let Some(problem) = shape_problem(l, &ids, &components) {
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
        let mount_line = entry_line(&manifest, "ui", "lens", &s.lens);
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
        } else {
            ext.ui.slots.push(LensSlot {
                slot: s.slot,
                lens_id: format!("{name}/{}", s.lens),
            });
        }
    }
    if let Some(v) = ui_command_files {
        let provider_ids: Vec<String> = ext.providers.iter().map(|p| p.id.clone()).collect();
        let (commands, errors) =
            ui_commands::parse_ui_commands(name, &provider_ids, &v, &file, &manifest);
        ext.ui.commands = commands;
        ext.errors.extend(errors);
    }
    if let Some(v) = decorator_files {
        let (decorators, errors) =
            decorators::parse_decorators(name, &ext.models, &v, &file, &manifest);
        ext.ui.decorators = decorators;
        ext.errors.extend(errors);
    }
    if let Some(v) = replacement_files {
        let capabilities: Vec<String> =
            ext.providers.iter().map(|p| p.capability.clone()).collect();
        let (replaced, errors) = replacements::parse_replacements(
            name,
            &capabilities,
            &ext.lenses,
            &v,
            &file,
            &manifest,
        );
        ext.ui.replacements = replaced;
        ext.errors.extend(errors);
    }
    if let Some(v) = page_files {
        let (pages, errors) = parse_pages(name, &ext.lenses, v);
        ext.pages = pages;
        ext.errors.extend(errors.into_iter().map(|(needle, e)| {
            at(
                &file,
                line_under(&manifest, "pages", &needle).or(key_line(&manifest, "pages")),
                e,
            )
        }));
    }
    // After models and pages: a ref kind names one of each.
    if let Some(v) = ref_kind_files {
        let (kinds, errors) = crate::extension_ref_kinds::parse_ref_kinds(
            name,
            &ext.models,
            &ext.pages,
            &v,
            &file,
            &manifest,
        );
        ext.ref_kinds = kinds;
        ext.errors.extend(errors);
    }
    if let Some(v) = panel_files {
        let (panels, errors) = parse_panels(name, &ext.lenses, v);
        ext.panels = panels;
        ext.errors.extend(errors.into_iter().map(|(needle, e)| {
            at(
                &file,
                line_under(&manifest, "panels", &needle).or(key_line(&manifest, "panels")),
                e,
            )
        }));
    }
    ext
}

/// Check the manifest's `pages:`: a kebab-case, unique id, a launcher
/// category, and a lens that exists. Errors carry text to find the line.
fn parse_pages(
    extension: &str,
    lenses: &[Lens],
    raw: serde_yaml::Value,
) -> (Vec<ExtensionPage>, Vec<(String, String)>) {
    let mut pages: Vec<ExtensionPage> = Vec::new();
    let mut errors = Vec::new();
    let Some(list) = raw.as_sequence() else {
        return (
            pages,
            vec![(String::new(), "`pages` must be a list".into())],
        );
    };
    for v in list {
        let needle = v
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or_default()
            .to_string();
        let p = match serde_yaml::from_value::<PageFile>(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                errors.push((needle, format!(
                    "pages: {e} (a page is `{{ id, title, icon?, category, lens }}`; a category is one of Work, Code, Git, Activity, Knowledge, Data, Lenses, System)"
                )));
                continue;
            }
        };
        let err = |e: String| (p.id.clone(), format!("page `{}`: {e}", p.id));
        if !is_advisory_id(&p.id) {
            errors.push(err("a page id is lowercase letters, digits and `-`".into()));
        } else if pages.iter().any(|q| q.id == p.id) {
            errors.push(err("declared twice".into()));
        } else if !lenses.iter().any(|l| l.slug == p.lens) {
            errors.push(err(format!("its lens `{}` isn't in lenses/", p.lens)));
        } else {
            pages.push(ExtensionPage {
                page_ref: format!("page:ext.{extension}.{}", p.id),
                id: p.id,
                extension: extension.to_string(),
                title: p.title,
                icon: p.icon,
                category: p.category,
                lens: format!("{extension}/{}", p.lens),
            });
        }
    }
    (pages, errors)
}

/// Check the manifest's `panels:` against the extension's lenses: a panel
/// id is kebab-case and unique, its lenses exist, a badge declares an
/// `alert`, and a `stream` / `thread` scope's lenses declare `stream_id` /
/// `thread_id` (what the nav binds). Errors carry text to find the line.
fn parse_panels(
    extension: &str,
    lenses: &[Lens],
    raw: serde_yaml::Value,
) -> (Vec<ExtensionPanel>, Vec<(String, String)>) {
    let mut panels: Vec<ExtensionPanel> = Vec::new();
    let mut errors = Vec::new();
    let Some(list) = raw.as_sequence() else {
        return (
            panels,
            vec![(String::new(), "`panels` must be a list".into())],
        );
    };
    for v in list {
        let p = match serde_yaml::from_value::<PanelFile>(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                let needle = v
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string();
                errors.push((needle, format!("panels: {e} (a panel is `{{ id, title, icon?, scope: project | stream | thread, body, badge? }}`)")));
                continue;
            }
        };
        let err = |e: String| (p.id.clone(), format!("panel `{}`: {e}", p.id));
        if !is_advisory_id(&p.id) {
            errors.push(err("a panel id is lowercase letters, digits and `-`".into()));
            continue;
        }
        if panels
            .iter()
            .any(|q| q.id == format!("{extension}/{}", p.id))
        {
            errors.push(err("declared twice".into()));
            continue;
        }
        let needs = match p.scope {
            PanelScope::Project => None,
            PanelScope::Stream => Some("stream_id"),
            PanelScope::Thread => Some("thread_id"),
        };
        let mut ok = true;
        for (role, slug) in [("body", Some(&p.body)), ("badge", p.badge.as_ref())] {
            let Some(slug) = slug else { continue };
            let Some(lens) = lenses.iter().find(|l| &l.slug == slug) else {
                errors.push(err(format!("its {role} lens `{slug}` isn't in lenses/")));
                ok = false;
                continue;
            };
            if role == "badge" && lens.alert.is_none() {
                errors.push(err(format!(
                    "its badge lens `{slug}` declares no `alert` (the badge is its count)"
                )));
                ok = false;
            }
            if let Some(param) = needs {
                if !lens.params.iter().any(|lp| lp.name == param) {
                    let scope = if param == "stream_id" {
                        "stream"
                    } else {
                        "thread"
                    };
                    errors.push(err(format!(
                        "a `{scope}` panel's lenses get `{param}`; lens `{slug}` must declare `{param}` in `params`"
                    )));
                    ok = false;
                }
            }
        }
        if ok {
            panels.push(ExtensionPanel {
                id: format!("{extension}/{}", p.id),
                extension: extension.to_string(),
                title: p.title,
                icon: p.icon,
                scope: p.scope,
                body: format!("{extension}/{}", p.body),
                badge: p.badge.map(|b| format!("{extension}/{b}")),
            });
        }
    }
    (panels, errors)
}

/// Why `lens` can't render with its viz, if it can't.
/// A grid's problem, if any: it names children, and each one in its own
/// extension exists. It needs no `query` — it renders no rows of its own.
fn grid_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    if lens.children.is_empty() {
        return Some("viz `grid` needs `children: [lens, ...]`".into());
    }
    lens.children
        .iter()
        .find(|c| {
            c.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str())
                && !ids_in_extension.contains(c)
        })
        .map(|c| format!("child lens `{c}` isn't in this extension's lenses/"))
}

fn shape_problem(
    lens: &Lens,
    ids_in_extension: &[String],
    components: &[String],
) -> Option<String> {
    // `block` names the lens key the missing columns go under.
    let need = |block: &str, fields: &[(&str, &Option<String>)]| -> Option<String> {
        let missing: Vec<&str> = fields
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        (!missing.is_empty()).then(|| {
            format!(
                "viz `{}` needs `{block}: {{ {} }}`",
                format!("{:?}", lens.viz).to_lowercase(),
                missing
                    .iter()
                    .map(|m| format!("{m}: <column>"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    };
    let chart = lens.chart.clone().unwrap_or_default();
    match lens.viz {
        LensViz::Bar | LensViz::Line => need("chart", &[("x", &chart.x), ("y", &chart.y)]),
        LensViz::Treemap => need("chart", &[("label", &chart.label), ("size", &chart.size)]),
        LensViz::Tree => {
            let t = lens.tree.clone().unwrap_or_default();
            need(
                "tree",
                &[("id", &t.id), ("parent", &t.parent), ("label", &t.label)],
            )
        }
        LensViz::Timeline => {
            let t = lens.timeline.clone().unwrap_or_default();
            need("timeline", &[("at", &t.at), ("label", &t.label)])
        }
        LensViz::Steps => {
            let s = lens.steps.clone().unwrap_or_default();
            need("steps", &[("label", &s.label)])
        }
        LensViz::Hunks => {
            let h = lens.hunks.clone().unwrap_or_default();
            need(
                "hunks",
                &[("path", &h.path), ("from", &h.from), ("to", &h.to)],
            )
        }
        LensViz::Form => {
            let f = lens.form.clone().unwrap_or_default();
            need("form", &[("command", &f.command)]).or_else(|| {
                let command = f.command.as_deref().unwrap_or_default();
                oxplow_domain::CommandSpec::validate_name(command)
                    .err()
                    .map(|e| format!("form: {e}"))
            })
        }
        // A grid composes its children; it renders no rows of its own.
        LensViz::Grid => grid_problem(lens, ids_in_extension),
        // Its rows are what the component shows and what an agent reads.
        LensViz::Custom if lens.query.trim().is_empty() => {
            Some("viz `custom` needs a `query`: its rows are what the component shows".into())
        }
        LensViz::Custom => match lens.custom.as_ref().and_then(|c| c.component.as_ref()) {
            None => Some("viz `custom` needs `custom: { component: <id> }`".into()),
            Some(c) if !components.contains(c) => Some(format!(
                "viz `custom`: `{c}` isn't one of this extension's `custom_components`"
            )),
            Some(_) => None,
        },
        _ if lens.query.trim().is_empty() => Some(format!(
            "viz `{}` needs a `query`",
            format!("{:?}", lens.viz).to_lowercase()
        )),
        LensViz::Table | LensViz::List | LensViz::Number | LensViz::Markdown | LensViz::Detail => {
            None
        }
    }
}

impl Lens {
    /// Every result column the lens's component blocks name (`chart`,
    /// `tree`, `timeline`, `steps`, `hunks`) — what `validate` checks the
    /// query returns.
    pub fn role_columns(&self) -> Vec<(&'static str, &String)> {
        let mut out: Vec<(&'static str, &String)> = Vec::new();
        if let Some(c) = &self.chart {
            out.extend(c.columns().into_iter().map(|k| ("chart", k)));
        }
        if let Some(t) = &self.tree {
            out.extend(
                [&t.id, &t.parent, &t.label]
                    .into_iter()
                    .flatten()
                    .map(|k| ("tree", k)),
            );
        }
        if let Some(t) = &self.timeline {
            out.extend(
                [&t.at, &t.label, &t.ref_column]
                    .into_iter()
                    .flatten()
                    .map(|k| ("timeline", k)),
            );
        }
        if let Some(s) = &self.steps {
            out.extend(
                [&s.label, &s.status]
                    .into_iter()
                    .flatten()
                    .map(|k| ("steps", k)),
            );
        }
        if let Some(h) = &self.hunks {
            out.extend(
                [&h.path, &h.from, &h.to]
                    .into_iter()
                    .flatten()
                    .map(|k| ("hunks", k)),
            );
        }
        out
    }
}

/// Clear what a disabled extension contributes; it stays listed so
/// Settings can turn it back on.
fn apply_disabled(mut ext: Extension, disabled: &[String]) -> Extension {
    if disabled.iter().any(|d| d == &ext.name) {
        ext.enabled = false;
        ext.lenses.clear();
        ext.ui = ExtensionUi::default();
        ext.collectors.clear();
        ext.providers.clear();
        ext.custom_components.clear();
        ext.event_types = Default::default();
        ext.ref_kinds.clear();
        ext.effects.clear();
        ext.advisories.clear();
        ext.measures.clear();
        ext.dimensions.clear();
        ext.metrics.clear();
        ext.models.clear();
        ext.launcher.clear();
        ext.panels.clear();
        ext.pages.clear();
        ext.commands.clear();
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
/// Run a lens made from `spec` (an answer's own lens) as `id`.
pub async fn run_spec(
    layer: &crate::sql_gateway::SqlGateway,
    id: &str,
    spec: &LensSpec,
    params: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    execute(layer, Lens::from_spec(id, spec), params, ctx).await
}

pub async fn run_lens(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    id: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    let lens = catalog.find_lens(root, id)?;
    let mut run = execute(layer, lens, params, ctx)
        .await
        .map_err(|e| match e {
            DomainError::Invalid(m) => DomainError::Invalid(explain_unsynced(catalog, root, &m)),
            other => other,
        })?;
    run.warnings = disabled_sources(layer, catalog, root, &run.result.reads.models).await;
    Ok(run)
}

/// For each view in `models` an extension's collector writes, a warning
/// when failures disabled that collector: its rows aren't refreshing.
async fn disabled_sources(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    models: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    for ext in catalog.get(root).iter() {
        for c in &ext.collectors {
            let Some(view) = c
                .entities
                .iter()
                .map(|e| &e.view)
                .find(|v| models.contains(v))
            else {
                continue;
            };
            let reason = layer
                .query_sql(
                    "SELECT reason FROM v_plugin_health
                      WHERE plugin = ?1 AND contribution = ?2 AND state = 'disabled'",
                    vec![SqlCell::Text(ext.name.clone()), SqlCell::Text(c.id.clone())],
                    Some(1),
                )
                .await
                .ok()
                .and_then(|r| r.rows.into_iter().next())
                .and_then(|row| row.into_iter().next());
            if let Some(SqlCell::Text(reason)) = reason {
                out.push(format!(
                    "`{view}` comes from the collector `{}/{}`, which is disabled ({reason}); \
                     its rows aren't being refreshed",
                    ext.name, c.id
                ));
            }
        }
    }
    out
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
        for collector in &ext.collectors {
            if collector.entities.iter().any(|e| e.view == view) {
                let lens = message.split(':').next().unwrap_or("lens");
                return format!(
                    "{lens}: reads `{view}`, which hasn't been collected yet. Run collector \
                     `{}/{}` (Settings → Extensions → Approve & Run, or Sync Now).",
                    ext.name, collector.id
                );
            }
        }
    }
    message.to_string()
}

/// The value of each of `lens`'s params: as supplied, else from the
/// context (`stream_id` / `thread_id`), else its default, else NULL. An
/// unknown supplied name is refused, naming the lens's params.
pub fn resolve_params(
    lens: &Lens,
    supplied: &BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<BTreeMap<String, SqlCell>, DomainError> {
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
    Ok(params)
}

/// Run `lens` with its default params and no viewer context — how a
/// review renders a lens it isn't showing anyone.
pub async fn run_lens_spec(
    layer: &crate::sql_gateway::SqlGateway,
    lens: Lens,
) -> Result<LensRun, DomainError> {
    execute(layer, lens, BTreeMap::new(), &LensContext::default()).await
}

/// Each of an extension's lenses run once with default params, by slug: a
/// failure is its message. What `check_extension` checks and what the
/// effect report renders — one run feeds both.
pub type LensRuns = BTreeMap<String, Result<LensRun, String>>;

/// [`LensRuns`] for `ext`.
pub async fn run_lenses(layer: &crate::sql_gateway::SqlGateway, ext: &Extension) -> LensRuns {
    let mut out = LensRuns::new();
    for lens in &ext.lenses {
        let run = run_lens_spec(layer, lens.clone())
            .await
            .map_err(|e| e.to_string().replacen("invalid value: ", "", 1));
        out.insert(lens.slug.clone(), run);
    }
    out
}

async fn execute(
    layer: &crate::sql_gateway::SqlGateway,
    lens: Lens,
    supplied: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    let params = resolve_params(&lens, &supplied, ctx)?;
    let named: Vec<(String, SqlCell)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    // A lens's `:stream_id` is the stream its metrics (`metric_grid()`)
    // are read over.
    let stream = match params.get("stream_id") {
        Some(SqlCell::Int(id)) => Some(*id),
        _ => None,
    };
    // A form without a query has no rows to read.
    let result = if lens.query.trim().is_empty() {
        oxplow_db::SqlQueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            reads: oxplow_db::Reads::default(),
            freshness: Vec::new(),
        }
    } else {
        layer
            .run(
                oxplow_db::SqlQuery::new(&lens.query)
                    .named(named)
                    .limit(None)
                    .stream(stream),
            )
            .await
            .map_err(|e| match e {
                DomainError::Invalid(m) => DomainError::Invalid(format!("lens {}: {m}", lens.id)),
                other => other,
            })?
    };
    let alert = lens.alert.as_ref().map(|a| evaluate_alert(a, &result));
    Ok(LensRun {
        lens,
        params,
        result,
        alert,
        warnings: Vec::new(),
    })
}

/// Load one extension and dry-run every lens with default params,
/// reporting query failures and `columns` keys the query doesn't
/// return as errors.
pub async fn validate_extension(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
    commands: Option<CommandSchemas<'_>>,
) -> Result<Extension, DomainError> {
    let mut ext = catalog.named(root, name)?;
    prepare(layer, catalog, root, &mut ext, commands).await;
    Ok(ext)
}

/// What a check asks the running oxplow's command registry (the
/// `CommandBus`): a command's input schema by name — what a launcher,
/// `ui.commands` or example command is checked against — and who holds a
/// command namespace. A plain schema closure is one with no namespaces.
/// `None` where there's no running app to ask (the CLI).
pub trait RunningCommands: Sync {
    fn input_schema(&self, name: &str) -> Option<serde_json::Value>;
    /// Who holds `namespace` (`oxplow`, `extension:<name>`,
    /// `provider:<instance>`), `None` when it's free.
    fn namespace_owner(&self, _namespace: &str) -> Option<String> {
        None
    }
}

impl<F: Fn(&str) -> Option<serde_json::Value> + Sync> RunningCommands for F {
    fn input_schema(&self, name: &str) -> Option<serde_json::Value> {
        self(name)
    }
}

/// The registry a check is given.
pub type CommandSchemas<'a> = &'a dyn RunningCommands;

/// A command an extension names — a launcher entry's or a `ui.commands`
/// entry's — is registered and its input fits. A command in one of the
/// extension's own providers' namespaces isn't on the bus until its
/// instance runs, so it is checked against that provider's declarations.
/// Without a registry (the CLI) the check is skipped, and says so.
pub fn check_commands(ext: &mut Extension, root: &Path, commands: Option<CommandSchemas<'_>>) {
    let mut entries: Vec<(String, String, serde_json::Value)> = ext
        .launcher
        .iter()
        .filter_map(|e| match &e.target {
            LauncherTarget::Command { command, input } => Some((
                format!("launcher entry `{}`", e.label),
                command.clone(),
                input.clone(),
            )),
            _ => None,
        })
        .collect();
    entries.extend(ext.ui.commands.iter().map(|c| {
        (
            format!("`ui.commands` `{}`", c.label),
            c.command.clone(),
            c.input.clone(),
        )
    }));
    if entries.is_empty() {
        return;
    }
    let Some(schema_of) = commands else {
        ext.warnings.push(format!(
            "{}/extension.yaml: its commands weren't checked (no running oxplow to ask which \
             commands exist) — check the extension from inside oxplow (Settings → Extensions)",
            ext.path
        ));
        return;
    };
    for (what, command, input) in entries {
        let schema = schema_of
            .input_schema(&command)
            .or_else(|| provider_command_schema(ext, root, &command));
        match schema {
            None => ext.errors.push(format!(
                "{}/extension.yaml: {what}: no command `{command}` — name a registered one",
                ext.path
            )),
            Some(schema) => {
                let fits = oxplow_domain::InputValidator::compile(&schema)
                    .map_err(|e| e.to_string())
                    .and_then(|v| v.check(&input).map_err(|e| e.to_string()));
                if let Err(e) = fits {
                    ext.errors.push(format!(
                        "{}/extension.yaml: {what}: the input doesn't fit `{command}`: {e}",
                        ext.path
                    ));
                }
            }
        }
    }
}

/// A custom component's declared commands must exist; a `custom` lens
/// that also fills a kit role block gets a nudge — the kit may already
/// render it (the honest extent of a "this reimplements the kit" lint).
fn check_components(ext: &mut Extension, root: &Path, commands: Option<CommandSchemas<'_>>) {
    let pages = custom_components::bundle_problems(ext, root);
    ext.errors.extend(pages);
    if let Some(schema_of) = commands {
        let missing: Vec<(String, String)> = ext
            .custom_components
            .iter()
            .flat_map(|c| {
                c.commands
                    .iter()
                    .filter(|n| schema_of.input_schema(n).is_none())
                    .map(|n| (c.id.clone(), n.clone()))
            })
            .collect();
        for (component, command) in missing {
            ext.errors.push(format!(
                "{}/extension.yaml: custom component `{component}` declares command `{command}`, \
                 which isn't registered",
                ext.path
            ));
        }
    }
    let lookalikes: Vec<(String, &'static str)> = ext
        .lenses
        .iter()
        .filter(|l| l.viz == LensViz::Custom)
        .flat_map(|l| {
            [
                ("chart", l.chart.is_some()),
                ("tree", l.tree.is_some()),
                ("timeline", l.timeline.is_some()),
                ("steps", l.steps.is_some()),
                ("hunks", l.hunks.is_some()),
            ]
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(block, _)| (l.path.clone(), block))
        })
        .collect();
    for (path, block) in lookalikes {
        ext.warnings.push(format!(
            "{path}: a custom lens that fills `{block}`: the kit's `{block}` viz may already cover \
             it — prefer the kit where it does"
        ));
    }
}

/// `command`'s input schema from the declarations of one of `ext`'s own
/// providers, when the command is in its namespace — one of its own
/// commands, not its capability's verbs (those run as `work_item.<verb>`).
fn provider_command_schema(
    ext: &Extension,
    root: &Path,
    command: &str,
) -> Option<serde_json::Value> {
    let (namespace, verb) = command.split_once('.')?;
    let spec = ext.providers.iter().find(|p| p.id == namespace)?;
    if spec.capability == crate::providers::spec::WORK_ITEMS
        && oxplow_domain::work_items::VERBS.contains(&verb)
    {
        return None;
    }
    let declared = crate::providers::spec::read_declarations(spec, &|rel| {
        read_extension_file(root, &ext.name, rel)
    })
    .ok()?;
    declared
        .commands
        .into_iter()
        .find(|c| c.name == verb)
        .map(|c| c.input_schema)
}

/// The empty stand-ins a check gives `ext`'s declared entities (P7.C6).
fn entity_stubs(ext: &Extension) -> Vec<oxplow_db::models::EntityStub> {
    ext.collectors
        .iter()
        .flat_map(|c| &c.entities)
        .map(|e| oxplow_db::models::EntityStub {
            owner: ext.name.clone(),
            name: e.name.clone(),
            view: e.view.clone(),
            columns: e
                .columns
                .iter()
                .map(|c| (c.name.clone(), crate::collector_runner::stored(c.col_type)))
                .collect(),
        })
        .collect()
}

/// Dry-run a loaded extension's models, commands, advisories and lenses,
/// appending what's wrong to its `errors`. `root` names sources for an
/// unsynced view. What it declares but hasn't published — its collectors'
/// entities (empty), its models, the other enabled extensions' — is an
/// overlay of temp views every query reads through (P7.C6), so a fresh
/// extension checks clean before its first sync.
/// An extension version checked against a layer (P8.C2): its lenses, run
/// through its own models, and that overlay — the temp views its models
/// (and the other enabled extensions') compile to, published nowhere —
/// for whatever else reads this version (a review's row counts).
pub struct Prepared {
    pub lenses: LensRuns,
    pub overlay: Vec<oxplow_db::TempView>,
}

/// The temp views `ext`'s models compile to — beside the other enabled
/// extensions' — and what's wrong with them.
async fn model_overlay(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &Extension,
) -> (Vec<oxplow_db::TempView>, Vec<String>) {
    let others: Vec<Extension> = catalog
        .get(root)
        .iter()
        .filter(|e| e.enabled && e.name != ext.name)
        .cloned()
        .collect();
    let models: Vec<oxplow_db::models::ExtensionModels> = others
        .iter()
        .chain(std::iter::once(ext))
        .filter(|e| !e.models.is_empty())
        .map(|e| oxplow_db::models::ExtensionModels {
            extension: e.name.clone(),
            sources: e.models.clone(),
        })
        .collect();
    let stubs: Vec<oxplow_db::models::EntityStub> = others
        .iter()
        .chain(std::iter::once(ext))
        .flat_map(entity_stubs)
        .collect();
    match layer.check_extension_models(models, stubs).await {
        Ok(mut checked) => (
            checked.views,
            checked.errors.remove(&ext.name).unwrap_or_default(),
        ),
        Err(e) => (Vec::new(), vec![format!("models: {e}")]),
    }
}

/// A version as a review reads it: its lenses rendered on its own models'
/// overlay, nothing checked — the earlier side of a review is what it was,
/// not a candidate (tsk791).
async fn read_side(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &Extension,
) -> Prepared {
    let (overlay, _) = model_overlay(layer, catalog, root, ext).await;
    let lenses = run_lenses(&layer.with_overlay(overlay.clone()), ext).await;
    Prepared { lenses, overlay }
}

/// Check `ext` — its commands, components, models, advisories and lenses —
/// reading through its own models' overlay, and report what's wrong in
/// `ext.errors`. Each side of a review prepares its own, so a lens renders
/// against its version's models, whichever are published.
async fn prepare(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
    commands: Option<CommandSchemas<'_>>,
) -> Prepared {
    check_commands(ext, root, commands);
    check_components(ext, root, commands);
    let (overlay, errors) = model_overlay(layer, catalog, root, ext).await;
    ext.errors.extend(errors);
    let layer = &layer.with_overlay(overlay.clone());
    crate::extension_commands::check_extension_commands(layer, ext, commands).await;
    for a in ext.advisories.clone() {
        let run = layer
            .run(
                oxplow_db::SqlQuery::new(&a.query)
                    .named(vec![("effort_id".into(), SqlCell::Null(()))])
                    .limit(None),
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
    let runs = run_lenses(layer, ext).await;
    for lens in ext.lenses.clone() {
        let id = lens.id.clone();
        match &runs[&lens.slug] {
            Err(e) => ext.errors.push(e.clone()),
            Ok(run) => {
                let cols = &run.result.columns;
                for (block, k) in run.lens.role_columns() {
                    if !cols.contains(k) {
                        ext.errors.push(format!(
                            "lens {id}: {block} column `{k}` isn't in the query result (columns: {})",
                            cols.join(", ")
                        ));
                    }
                }
                for a in &run.lens.actions {
                    for (scope, name) in action_templates(&a.input) {
                        if scope == "row" && !cols.contains(&name) {
                            ext.errors.push(format!(
                                "lens {id}: action `{}` uses `{{{{row.{name}}}}}`, which isn't in \
                                 the query result (columns: {})",
                                a.id,
                                cols.join(", ")
                            ));
                        }
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
    Prepared {
        lenses: runs,
        overlay,
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
    /// What installing it would change, against the installed version
    /// when it replaces one (P6b.E2); `None` when the candidate doesn't
    /// load (its `problems` say why).
    pub effects: Option<crate::extension_effects::EffectReport>,
}

/// Clone an extension and report what it declares, installing nothing.
/// `replacing` names the installed extension an update must match.
pub async fn review_extension(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    git_url: &str,
    git_ref: Option<&str>,
    replacing: Option<&str>,
    commands: CommandSchemas<'_>,
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
    let installed: Option<Extension> = replacing.and_then(|name| {
        catalog
            .get(root)
            .iter()
            .find(|e| e.name == name && e.origin == "project")
            .cloned()
    });
    let name = extension.name.clone();
    let read_installed = |rel: &str| read_extension_file(root, &name, rel);
    let clone = Disk(fetched.clone.clone());
    let read_candidate = |rel: &str| clone.read(rel);
    let effects = if load_errors > 0 {
        // A candidate that doesn't load has nothing reliable to compare;
        // its check still says what else is wrong.
        prepare(layer, catalog, root, &mut extension, Some(commands)).await;
        None
    } else {
        // The installed side through its own models, too: what's
        // published may not be it (a disabled extension, a model that
        // failed, another worktree's copy).
        let mut after = ReviewSide {
            extension,
            read: &read_candidate,
        };
        let report = effects_between(
            layer,
            catalog,
            root,
            installed.map(|e| ReviewSide {
                extension: e,
                read: &read_installed,
            }),
            &mut after,
            commands,
        )
        .await;
        extension = after.extension;
        Some(report)
    };
    let problems = extension.errors.split_off(load_errors);
    Ok(ExtensionReview {
        extension,
        git: git_url.to_string(),
        git_ref: git_ref.map(str::to_string),
        sha: fetched.sha,
        problems,
        effects,
    })
}

/// [`review_extension`] for an update: the installed extension's recorded
/// source.
pub async fn review_update(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
    commands: CommandSchemas<'_>,
) -> Result<ExtensionReview, DomainError> {
    let source = installed_source(root, name)?;
    review_extension(
        layer,
        catalog,
        root,
        &source.git,
        source.git_ref.as_deref(),
        Some(name),
        commands,
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

/// A lowercase letter, then lowercase letters, digits and single dashes —
/// safe as a folder name, a lens-id prefix and (dashes as underscores) a
/// command namespace.
pub fn is_valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
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
            "extension name `{name}` must start with a letter and be lowercase letters, digits and single dashes"
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

/// What a lens shows — the view part of a lens file, and the one shape a
/// lens is made from: a lens file's body, an agent's answer
/// (`thread_answer.spec`, P6.C1), what Explore Data saves, and what
/// [`save_lens`] writes. The rest of a lens file (launcher placement,
/// actions, an alert) is added by editing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensSpec {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// SQL over the `v_*` models, with `:param` bindings. Only a `form`
    /// may leave it empty.
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_viz")]
    pub viz: LensViz,
    #[serde(default)]
    pub params: Vec<LensParam>,
    #[serde(default)]
    pub columns: Vec<LensColumn>,
    #[serde(default)]
    pub empty: Option<String>,
    #[serde(default)]
    pub chart: Option<LensChart>,
    #[serde(default)]
    pub tree: Option<LensTree>,
    #[serde(default)]
    pub timeline: Option<LensTimeline>,
    #[serde(default)]
    pub steps: Option<LensSteps>,
    #[serde(default)]
    pub hunks: Option<LensHunks>,
    #[serde(default)]
    pub form: Option<LensForm>,
}

impl Lens {
    /// A lens made from `spec`, as `id` (`<extension>/<slug>`, or an
    /// answer's `answer/<n>`); it has no actions, alert or children.
    pub fn from_spec(id: &str, spec: &LensSpec) -> Lens {
        let (extension, slug) = id.split_once('/').unwrap_or((id, id));
        Lens {
            id: id.to_string(),
            extension: extension.to_string(),
            slug: slug.to_string(),
            title: spec.title.clone(),
            description: spec.description.clone(),
            query: spec.query.clone(),
            viz: spec.viz,
            params: spec.params.clone(),
            columns: spec.columns.clone(),
            empty: spec.empty.clone(),
            chart: spec.chart.clone(),
            tree: spec.tree.clone(),
            timeline: spec.timeline.clone(),
            steps: spec.steps.clone(),
            hunks: spec.hunks.clone(),
            form: spec.form.clone(),
            custom: None,
            children: Vec::new(),
            launcher_category: None,
            hidden: false,
            actions: Vec::new(),
            alert: None,
            path: String::new(),
        }
    }

    /// The spec this lens was made from (its view part).
    pub fn spec(&self) -> LensSpec {
        LensSpec {
            title: self.title.clone(),
            description: self.description.clone(),
            query: self.query.clone(),
            viz: self.viz,
            params: self.params.clone(),
            columns: self.columns.clone(),
            empty: self.empty.clone(),
            chart: self.chart.clone(),
            tree: self.tree.clone(),
            timeline: self.timeline.clone(),
            steps: self.steps.clone(),
            hunks: self.hunks.clone(),
            form: self.form.clone(),
        }
    }
}

/// What's wrong with `spec` as a standalone lens, if anything: no title,
/// or a component missing what it needs. (A `grid` composes other lenses,
/// so a standalone spec can't be one.)
pub fn spec_problem(spec: &LensSpec) -> Option<String> {
    if spec.title.trim().is_empty() {
        return Some("a lens needs a `title`".into());
    }
    if spec.viz == LensViz::Grid {
        return Some("a `grid` composes lens files; show each lens instead".into());
    }
    if spec.viz == LensViz::Custom {
        return Some(
            "a `custom` lens renders an extension's component; write it as a lens file in a \
             private extension that declares the component"
                .into(),
        );
    }
    shape_problem(&Lens::from_spec("spec/spec", spec), &[], &[])
}

/// A lens slug from a title: lowercase letters and digits, runs of
/// anything else one dash.
pub fn slug_of(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "lens".into()
    } else {
        out
    }
}

/// Who asked for a new lens, for its extension's intent when the
/// extension is created.
pub struct LensOrigin<'a> {
    /// What the lens answers (the extension's `intent.purpose`).
    pub purpose: &'a str,
    /// The thread or effort ref that asked for it (`intent.origin`).
    pub origin: Option<&'a str>,
}

/// Write `spec` as a new lens file
/// `oxplow/extensions/<extension>/lenses/<slug>.yaml`, creating the
/// extension (a private v2 manifest whose intent is `origin`) if needed.
/// Refuses a spec with a problem, overwriting a lens, and bundled or
/// git-installed extensions (their files are replaced on update).
/// The directory of extension `name` under `root`, if oxplow may write
/// lens files into it: a valid name, not a bundled (read-only) extension,
/// not an installed one (its files are replaced on update). The one check
/// every writer of an extension's files — `save_lens`, `lens.share` —
/// runs first, so a path or a reserved name never reaches the filesystem.
pub fn writable_extension_dir(root: &Path, name: &str) -> Result<PathBuf, DomainError> {
    if !is_valid_name(name) {
        return Err(DomainError::Invalid(format!(
            "extension name `{name}` must start with a letter and be lowercase letters, digits and single dashes"
        )));
    }
    if crate::bundled_extensions::is_reserved(name) {
        return Err(DomainError::Invalid(format!(
            "`{name}` is a bundled extension (read-only); save to another extension"
        )));
    }
    let dir = root.join(EXTENSIONS_DIR).join(name);
    if dir.join(SOURCE_FILE).exists() {
        return Err(DomainError::Invalid(format!(
            "`{name}` is an installed extension (its files are replaced on update); save to another extension"
        )));
    }
    Ok(dir)
}

pub fn save_lens(
    root: &Path,
    extension: &str,
    slug: &str,
    spec: &LensSpec,
    origin: &LensOrigin<'_>,
) -> Result<Lens, DomainError> {
    let invalid = |m: String| DomainError::Invalid(m);
    let storage = |e: std::io::Error| DomainError::Storage(format!("save lens: {e}"));
    let dir = writable_extension_dir(root, extension)?;
    if !is_valid_name(slug) {
        return Err(invalid(format!(
            "lens slug `{slug}` must start with a letter and be lowercase letters, digits and single dashes"
        )));
    }
    if let Some(problem) = spec_problem(spec) {
        return Err(invalid(problem));
    }
    let file = dir.join("lenses").join(format!("{slug}.yaml"));
    if file.exists() {
        return Err(invalid(format!("lens `{extension}/{slug}` already exists")));
    }
    std::fs::create_dir_all(file.parent().unwrap_or(&dir)).map_err(storage)?;
    let manifest = dir.join("extension.yaml");
    if !manifest.exists() {
        // The same v2 manifest `oxplow plugin new` writes, so a lens saved
        // from Explore Data starts as a checkable extension with an intent.
        std::fs::write(
            &manifest,
            scaffold_manifest(&ManifestScaffold {
                name: extension,
                description: "TODO: one line on what this extension shows or does",
                purpose: origin.purpose,
                origin: origin.origin,
                example_name: slug,
                example_input: &format!("{{ lens: {slug} }}"),
                example_expect: "TODO: what a run should show",
                shared: false,
            }),
        )
        .map_err(storage)?;
    }
    let body = lens_file_yaml(spec)?;
    std::fs::write(&file, body).map_err(storage)?;
    let ext = load_fresh(root, extension)?;
    match ext.lenses.into_iter().find(|l| l.slug == slug) {
        Some(lens) => Ok(lens),
        None => {
            // It didn't load: take it back out and say why.
            let _ = std::fs::remove_file(&file);
            let why: Vec<String> = ext
                .errors
                .into_iter()
                .filter(|e| e.contains(&format!("{slug}.yaml")))
                .collect();
            Err(invalid(format!(
                "the lens `{extension}/{slug}` doesn't load: {}",
                if why.is_empty() {
                    "no reason given".to_string()
                } else {
                    why.join("; ")
                }
            )))
        }
    }
}

/// `spec` as a lens file: what it sets, without the empty and absent
/// keys at any depth (the file reads like one written by hand). Built as
/// YAML directly — a JSON detour would carry serde_json's number
/// representation into the file.
fn lens_file_yaml(spec: &LensSpec) -> Result<String, DomainError> {
    fn prune(v: serde_yaml::Value) -> Option<serde_yaml::Value> {
        use serde_yaml::Value as Y;
        match v {
            Y::Null => None,
            Y::String(s) if s.is_empty() => None,
            Y::Sequence(items) => {
                let items: Vec<Y> = items.into_iter().filter_map(prune).collect();
                (!items.is_empty()).then_some(Y::Sequence(items))
            }
            Y::Mapping(map) => {
                let map: serde_yaml::Mapping = map
                    .into_iter()
                    .filter_map(|(k, v)| prune(v).map(|v| (k, v)))
                    .collect();
                (!map.is_empty()).then_some(Y::Mapping(map))
            }
            other => Some(other),
        }
    }
    let value =
        serde_yaml::to_value(spec).map_err(|e| DomainError::Storage(format!("save lens: {e}")))?;
    serde_yaml::to_string(&prune(value).unwrap_or(serde_yaml::Value::Null))
        .map_err(|e| DomainError::Storage(format!("save lens: {e}")))
}

/// What a scaffolded `extension.yaml` says. One template for
/// `oxplow plugin new` and `save_lens`, so every new extension starts
/// with an intent and passes `check`.
pub struct ManifestScaffold<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub purpose: &'a str,
    /// The thread/effort ref that asked for it, when known.
    pub origin: Option<&'a str>,
    pub example_name: &'a str,
    /// YAML flow text for the example's input (`{ lens: demo }`).
    pub example_input: &'a str,
    pub example_expect: &'a str,
    /// A shared extension (`sharing: shared`, targeting this oxplow's
    /// engine) rather than a private one.
    pub shared: bool,
}

/// A v2 manifest with an `intent` and one example — private, or shared
/// with the `engine` it targets.
pub fn scaffold_manifest(m: &ManifestScaffold<'_>) -> String {
    let quote = |s: &str| {
        serde_yaml::to_string(s)
            .expect("string serializes")
            .trim_end()
            .to_string()
    };
    format!(
        "manifest: 2\nname: {name}\ndescription: {description}\n{sharing}intent:\n  purpose: {purpose}\n  origin: {origin}\n  examples:\n    - name: {example_name}\n      input: {input}\n      expect: {expect}\n",
        sharing = if m.shared {
            let version = manifest_v2::current_engine();
            let major_minor: Vec<&str> = version.split('.').take(2).collect();
            format!("sharing: shared\nengine: \">={}\"\n", major_minor.join("."))
        } else {
            "sharing: private\n".to_string()
        },
        name = m.name,
        description = quote(m.description),
        purpose = quote(m.purpose),
        origin = m.origin.unwrap_or("null"),
        example_name = m.example_name,
        input = m.example_input,
        expect = quote(m.example_expect),
    )
}

#[cfg(test)]
mod tests {

    /// One tokenizer for `{{scope.name}}` placeholders: load-time
    /// validation (`action_templates`) and run-time binding
    /// (`lens_actions::bind_input`) both read it, so they can't disagree —
    /// on spans, on a string that is exactly one placeholder (bound typed),
    /// or on a stray `{{`.
    #[test]
    fn placeholders_are_one_tokenizer() {
        let ps = placeholders("Fix {{row.title}} in {{ param.stream_id }}");
        assert_eq!(
            ps.iter()
                .map(|p| (p.scope.as_str(), p.name.as_str(), p.start, p.end))
                .collect::<Vec<_>>(),
            vec![("row", "title", 4, 17), ("param", "stream_id", 21, 42)]
        );
        assert_eq!(
            whole_placeholder("  {{row.id}} ").map(|p| (p.scope, p.name)),
            Some(("row".to_string(), "id".to_string()))
        );
        assert!(whole_placeholder("{{row.a}} {{row.b}}").is_none());
        assert!(whole_placeholder("x {{row.a}}").is_none());
        // A stray opener is part of the next placeholder's text, for both.
        let nested = placeholders("{{{{row.a}}");
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].scope, "{{row");
        assert_eq!(
            action_templates(&serde_json::json!({ "t": "{{{{row.a}}" })),
            vec![("{{row".to_string(), "a".to_string())]
        );
    }
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

    const EXT: &str =
        "manifest: 2\nname: review\nintent:\n  purpose: test\ndescription: Review helpers\n";
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

    async fn layer() -> crate::sql_gateway::SqlGateway {
        crate::sql_gateway::SqlGateway::new(Database::in_memory())
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
            "manifest: 2\nname: other\nintent:\n  purpose: test\n",
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

        let e = validate_extension(&sl, &cat(), dir.path(), "review", None)
            .await
            .unwrap();
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("review/broken")));
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("review/badcol") && m.contains("missing")));

        assert!(matches!(
            validate_extension(&sl, &cat(), dir.path(), "nope", None).await,
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
            "manifest: 2\nname: shared\nintent:\n  purpose: test\ndescription: Shared lenses\n",
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

    /// P8.C2: each side of a review reads through its own overlay — a lens
    /// over a model whose SQL changed renders against each version's own
    /// SQL, though neither version's models are published.
    #[tokio::test]
    async fn each_side_of_a_review_reads_its_own_models() {
        let project = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            "extension.yaml",
            "manifest: 2\nname: shared\nintent:\n  purpose: test\ndescription: Shared lenses\nmodels:\n  - name: x\n    version: 1\n    description: X.\n    columns:\n      - { name: n, type: \"\", doc: N. }\n",
        );
        write(repo.path(), "models/x.sql", "SELECT 1 AS n\n");
        write(
            repo.path(),
            "lenses/count.yaml",
            "title: Count\nquery: SELECT n FROM v_shared_x\nviz: number\n",
        );
        git(repo.path(), &["init", "-q", "-b", "main"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let url = repo.path().to_string_lossy().to_string();
        let sl = layer().await;
        let none = &|_: &str| -> Option<serde_json::Value> { None };
        let first = review_extension(&sl, &cat(), project.path(), &url, None, None, none)
            .await
            .unwrap();
        install_extension(project.path(), &url, None, &first.sha).unwrap();
        write(repo.path(), "models/x.sql", "SELECT 2 AS n\n");
        git(repo.path(), &["commit", "-qam", "two"]);
        let update = review_update(&sl, &cat(), project.path(), "shared", none)
            .await
            .unwrap();
        let count = update
            .effects
            .as_ref()
            .unwrap()
            .lenses
            .iter()
            .find(|l| l.id == "shared/count")
            .unwrap();
        assert_eq!(
            (
                count.before.as_deref(),
                count.after.as_deref(),
                count.error.as_deref()
            ),
            (Some("1"), Some("2"), None)
        );
        // P8.C3: its rows, each side's own — counts, since it has no key.
        let x = update
            .effects
            .as_ref()
            .unwrap()
            .models
            .iter()
            .find(|m| m.view == "v_shared_x")
            .unwrap();
        let rows = x.rows.as_ref().expect("a changed model's rows");
        assert_eq!((rows.before, rows.after), (Some(1), Some(1)));
        assert!(
            rows.note.as_deref().unwrap_or("").contains("no key"),
            "{rows:?}"
        );
    }

    /// P6b.E2: an update is reviewed against the installed version — a
    /// changed lens shows its text before and after.
    #[tokio::test]
    async fn review_update_shows_before_and_after_against_the_installed_version() {
        let project = tempfile::tempdir().unwrap();
        let repo = published_repo("Count");
        let url = repo.path().to_string_lossy().to_string();
        let sl = layer().await;
        let first = review_extension(
            &sl,
            &cat(),
            project.path(),
            &url,
            None,
            None,
            &|_: &str| -> Option<serde_json::Value> { None },
        )
        .await
        .unwrap();
        assert!(first
            .effects
            .as_ref()
            .unwrap()
            .lenses
            .iter()
            .all(|l| l.change == crate::extension_effects::Change::Added));
        install_extension(project.path(), &url, None, &first.sha).unwrap();
        write(
            repo.path(),
            "lenses/count.yaml",
            "title: Count\nquery: SELECT 2 AS n\nviz: number\n",
        );
        git(repo.path(), &["commit", "-qam", "two"]);
        let update = review_update(&sl, &cat(), project.path(), "shared", &|_: &str| -> Option<
            serde_json::Value,
        > { None })
        .await
        .unwrap();
        let count = update
            .effects
            .as_ref()
            .unwrap()
            .lenses
            .iter()
            .find(|l| l.id == "shared/count")
            .unwrap();
        assert_eq!(count.change, crate::extension_effects::Change::Changed);
        assert_eq!(
            (count.before.as_deref(), count.after.as_deref()),
            (Some("1"), Some("2"))
        );
        // A candidate that doesn't load has no effects to show: its
        // problems say why.
        write(
            repo.path(),
            "extension.yaml",
            "manifest: 2\nname: shared\nbogus_kind: 1\n",
        );
        git(repo.path(), &["commit", "-qam", "broken"]);
        let broken = review_update(&sl, &cat(), project.path(), "shared", &|_: &str| -> Option<
            serde_json::Value,
        > { None })
        .await
        .unwrap();
        assert!(!broken.extension.errors.is_empty());
        assert_eq!(broken.effects, None);
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
            "manifest: 2\nname: shared\nintent:\n  purpose: test\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    network: [api.github.com]\n    credentials: [TOKEN]\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
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

        let review = review_extension(
            &sl,
            &cat(),
            project.path(),
            &url,
            None,
            None,
            &|_: &str| -> Option<serde_json::Value> { None },
        )
        .await
        .unwrap();
        assert_eq!(review.extension.name, "shared");
        assert_eq!(review.sha, head(repo.path()));
        let source = &review.extension.collectors[0];
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
        write(
            repo.path(),
            "extension.yaml",
            "manifest: 2\nname: shared\nintent:\n  purpose: test\nbogus: 1\n",
        );
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
            "manifest: 2\nname: local\nintent:\n  purpose: test\n",
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

    fn spec_base() -> LensSpec {
        LensSpec {
            title: String::new(),
            description: String::new(),
            query: String::new(),
            viz: LensViz::Table,
            params: Vec::new(),
            columns: Vec::new(),
            empty: None,
            chart: None,
            tree: None,
            timeline: None,
            steps: None,
            hunks: None,
            form: None,
        }
    }

    fn todo_origin() -> LensOrigin<'static> {
        LensOrigin {
            purpose: "TODO: what it answers",
            origin: None,
        }
    }

    /// P6.C1: a spec round-trips through a lens file, and one with a
    /// problem is refused before anything is written.
    #[test]
    fn a_spec_is_a_lens_file() {
        let project = tempfile::tempdir().unwrap();
        let spec = LensSpec {
            title: "Busy Files".into(),
            query: "SELECT path, n FROM v_x".into(),
            viz: LensViz::Bar,
            chart: Some(LensChart {
                x: Some("path".into()),
                y: Some("n".into()),
                ..LensChart::default()
            }),
            ..spec_base()
        };
        let origin = LensOrigin {
            purpose: "Busy Files",
            origin: Some("thread:thr1"),
        };
        let lens = save_lens(project.path(), "my-lenses", "busy", &spec, &origin).unwrap();
        assert_eq!(lens.spec(), spec);
        let manifest = std::fs::read_to_string(
            project
                .path()
                .join("oxplow/extensions/my-lenses/extension.yaml"),
        )
        .unwrap();
        assert!(manifest.contains("origin: thread:thr1"), "{manifest}");
        let bad = LensSpec {
            viz: LensViz::Bar,
            chart: None,
            ..spec.clone()
        };
        let err = save_lens(project.path(), "my-lenses", "bad", &bad, &origin).unwrap_err();
        assert!(err.to_string().contains("chart"), "{err}");
        assert!(!project
            .path()
            .join("oxplow/extensions/my-lenses/lenses/bad.yaml")
            .exists());
    }

    /// A form's numeric defaults are written as numbers and read back.
    #[test]
    fn a_form_specs_defaults_round_trip_through_its_file() {
        let project = tempfile::tempdir().unwrap();
        let spec = LensSpec {
            title: "New Task".into(),
            viz: LensViz::Form,
            form: Some(LensForm {
                command: Some("work_item.create".into()),
                defaults: Some(serde_json::json!({ "title": "x", "n": 3, "tags": ["a"] })),
            }),
            ..spec_base()
        };
        let lens = save_lens(project.path(), "mine", "new-task", &spec, &todo_origin()).unwrap();
        assert_eq!(lens.spec(), spec);
    }

    #[test]
    fn save_lens_creates_the_extension_and_refuses_overwrites() {
        let project = tempfile::tempdir().unwrap();
        let new = || LensSpec {
            title: "Open Tasks".into(),
            description: "From Explore Data".into(),
            query: "SELECT id, title FROM v_task".into(),
            ..spec_base()
        };
        let lens = save_lens(project.path(), "mine", "open-tasks", &new(), &todo_origin()).unwrap();
        assert_eq!(lens.id, "mine/open-tasks");
        assert_eq!(lens.title, "Open Tasks");
        assert!(project
            .path()
            .join("oxplow/extensions/mine/extension.yaml")
            .is_file());
        let ext = &project_extensions(project.path())[0];
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);

        let err =
            save_lens(project.path(), "mine", "open-tasks", &new(), &todo_origin()).unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("already exists")),
            "{err:?}"
        );
        let err =
            save_lens(project.path(), "mine", "Bad Slug", &new(), &todo_origin()).unwrap_err();
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
            "manifest: 2\nname: shared\nintent:\n  purpose: test\n",
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
            &LensSpec {
                title: "X".into(),
                query: "SELECT 1".into(),
                viz: LensViz::Number,
                ..spec_base()
            },
            &todo_origin(),
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
            "manifest: 2\nname: review\nintent:\n  purpose: test\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: bin/sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n  - id: bad\n    runtime: python\n    entry: x\n    entities: []\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/bin/sync.sh",
            "#!/bin/sh\n",
        );
        let e = &project_extensions(dir.path())[0];
        assert_eq!(e.collectors.len(), 1, "{:?}", e.errors);
        assert_eq!(e.collectors[0].entities[0].view, "v_review_pr");
        assert_eq!(e.errors.len(), 1, "{:?}", e.errors);
        assert!(
            e.errors[0].contains("extension.yaml") && e.errors[0].contains("runtime"),
            "{:?}",
            e.errors
        );
        assert_eq!(e.lenses.len(), 1);
    }

    #[tokio::test]
    async fn an_uncollected_entity_gets_a_helpful_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/gh/extension.yaml",
            "manifest: 2\nname: gh\nintent:\n  purpose: test\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(dir.path(), "oxplow/extensions/gh/sync.sh", "#!/bin/sh\n");
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
            msg.contains("v_gh_pr") && msg.contains("prs") && msg.contains("hasn't been collected"),
            "{msg}"
        );
        // A check stands an empty view in for the entity (P7.C6): the
        // lens is fine before its first sync.
        let e = validate_extension(&sl, &cat(), dir.path(), "gh", None)
            .await
            .unwrap();
        assert!(e.errors.is_empty(), "{:?}", e.errors);
    }

    /// P7.C2: a lens over a disabled collector's view says so — its rows
    /// aren't being refreshed. Nothing cascades: the lens still runs.
    #[tokio::test]
    async fn a_lens_over_a_disabled_collectors_view_warns() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/gh/extension.yaml",
            "manifest: 2\nname: gh\nintent:\n  purpose: test\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(dir.path(), "oxplow/extensions/gh/sync.sh", "#!/bin/sh\n");
        write(
            dir.path(),
            "oxplow/extensions/gh/lenses/all.yaml",
            "title: All\nquery: SELECT number FROM v_gh_pr\n",
        );
        let db = Database::in_memory();
        oxplow_db::SqliteCollectorStore::new(db.clone())
            .replace_rows(vec![(
                oxplow_db::EntityTable {
                    extension: "gh".into(),
                    entity: "pr".into(),
                    view: "v_gh_pr".into(),
                    key: "number".into(),
                    description: String::new(),
                    columns: vec![oxplow_db::EntityColumn {
                        name: "number".into(),
                        stored: oxplow_db::StoredType::Integer,
                        doc: String::new(),
                    }],
                },
                vec![vec![SqlCell::Int(1)]],
            )])
            .await
            .unwrap();
        db.transaction(|tx| {
            oxplow_db::plugin_health_store::disable_tx(
                tx,
                &crate::collector_runner::plugin_key("gh", "prs"),
                "3 failures in a row; the last: boom",
                "t",
            )
        })
        .await
        .unwrap();
        let layer = crate::sql_gateway::SqlGateway::new(db);
        let run = run_lens(
            &layer,
            &cat(),
            dir.path(),
            "gh/all",
            BTreeMap::new(),
            &LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(run.warnings.len(), 1, "{:?}", run.warnings);
        assert!(
            run.warnings[0].contains("gh/prs") && run.warnings[0].contains("3 failures in a row"),
            "{:?}",
            run.warnings
        );
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
        let manifest = "manifest: 2\nname: review\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nproviders:\n  - id: ticket\n";
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            manifest,
        );
        let e = only(dir.path(), "review");
        let err = e
            .errors
            .iter()
            .find(|m| m.contains("`providers` is experimental"))
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
        let manifest = format!("{EXT_V2}ui:\n  slots:\n    - {{ slot: rail, lens: tasks }}\n    - {{ slot: work_item.detail.body, lens: nope }}\n");
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
            err.starts_with("oxplow/extensions/review/extension.yaml:11:"),
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

    /// tsk865: a manifest says its version, or it doesn't load — there is
    /// no v1 reader any more.
    #[test]
    fn a_manifest_without_a_version_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            "name: review\ndescription: Review helpers\n",
        );
        let e = only(dir.path(), "review");
        assert!(
            e.errors
                .iter()
                .any(|m| m.contains("extension.yaml:1") && m.contains("`manifest: 2` is required")),
            "{:?}",
            e.errors
        );
        assert!(e.intent.is_none());
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
            "title: All\nviz: grid\nchildren: [review/tasks, review/missing]\n",
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

    /// The tally extension: a starlark collector declaring entity `thing`,
    /// a model over it, lenses over both, and a command reading the model.
    fn write_tally(root: &Path, script: &str) {
        write(
            root,
            "oxplow/extensions/tally/extension.yaml",
            "manifest: 2
name: tally
intent: { purpose: x, examples: [{ name: a }] }
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT id FROM v_task\"
    entities:
      - { name: thing, key: id, columns: { id: int, label: text } }
models:
  - name: labelled
    version: 1
    description: Labelled things.
    columns:
      - { name: id, type: INTEGER, doc: The thing. }
      - { name: label, type: TEXT, doc: Its label. }
commands:
  - name: note
    summary: Note a thing.
    input_schema: { type: object, required: [id], properties: { id: { type: integer } } }
    entry: handlers/note.star
    input: \"SELECT id, label FROM v_tally_labelled WHERE id = :id\"
",
        );
        write(
            root,
            "oxplow/extensions/tally/collectors/things.star",
            script,
        );
        write(
            root,
            "oxplow/extensions/tally/models/labelled.sql",
            "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
        );
        write(
            root,
            "oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": []}\n",
        );
        write(
            root,
            "oxplow/extensions/tally/lenses/labelled.yaml",
            "title: Labelled\nquery: SELECT id, label FROM v_tally_labelled\n",
        );
        write(
            root,
            "oxplow/extensions/tally/lenses/things.yaml",
            "title: Things\nquery: SELECT id, label FROM v_tally_thing\n",
        );
    }

    /// P7.C6: a check sees what the extension declares before it ever ran
    /// or published — its collectors' entities (empty stand-ins) and its
    /// own models (compiled for the check) — from its lenses and command
    /// inputs, on a database that has neither.
    #[tokio::test]
    async fn a_check_sees_the_extensions_own_models_and_unsynced_entities() {
        let dir = tempfile::tempdir().unwrap();
        write_tally(
            dir.path(),
            "def transform(x):\n    return {\"entities\": {\"thing\": []}}\n",
        );
        let v = validate_extension(&layer().await, &cat(), dir.path(), "tally", None)
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);

        // Still a real check: a column the model doesn't have is an error.
        write(
            dir.path(),
            "oxplow/extensions/tally/lenses/labelled.yaml",
            "title: Labelled\nquery: SELECT id, colour FROM v_tally_labelled\n",
        );
        let v = validate_extension(&layer().await, &cat(), dir.path(), "tally", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(errs.contains("colour"), "{errs}");
    }

    /// P7.C6: a starlark collector's script is parsed at load — one that
    /// doesn't define `transform` is an error at the collector's line.
    #[test]
    fn a_collector_script_without_transform_is_an_error_at_its_line() {
        let dir = tempfile::tempdir().unwrap();
        write_tally(dir.path(), "def other(x):\n    return {}\n");
        let ext = only(dir.path(), "tally");
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("extension.yaml:5:")
                && errs.contains("collector `things`")
                && errs.contains("must define `transform`"),
            "{errs}"
        );
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
            .ui
            .slots
            .iter()
            .any(|s| s.slot == "effort.review.details" && s.lens_id == "oxplow-review/decisions"));
        assert!(review
            .ui
            .slots
            .iter()
            .any(|s| s.slot == "effort.review.details"
                && s.lens_id == "oxplow-review/inferred-decisions"));
        // P9.A2: the file diff's header strip has its first user.
        let analytics = exts.iter().find(|e| e.name == "oxplow-analytics").unwrap();
        assert!(analytics.ui.slots.iter().any(
            |s| s.slot == "diff.file.header" && s.lens_id == "oxplow-analytics/file-co-change"
        ));
        // The analytics extension's advisory and lens SQL runs too.
        let a = validate_extension(&layer().await, &cat(), dir.path(), "oxplow-analytics", None)
            .await
            .unwrap();
        assert!(a.errors.is_empty(), "{:?}", a.errors);
        // Every bundled lens's SQL runs against a real schema — its own
        // models compiled for the check, nothing published.
        let v = validate_extension(&layer().await, &cat(), dir.path(), "oxplow-review", None)
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
    fn a_name_starts_with_a_letter() {
        for ok in ["review", "review-notes", "a1", "x"] {
            assert!(is_valid_name(ok), "{ok}");
        }
        // `1x` would be the command namespace `1x`, which no command
        // name can have.
        for bad in ["1x", "9-lives", "", "-a", "a-", "a--b", "A", "a_b"] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn bundled_names_are_reserved() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/oxplow-review/extension.yaml",
            "manifest: 2\nname: oxplow-review\nintent:\n  purpose: test\n",
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
            &LensSpec {
                title: "X".into(),
                query: "SELECT 1".into(),
                viz: LensViz::Number,
                ..spec_base()
            },
            &todo_origin(),
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
            "manifest: 2\nname: mine\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: effort.review.details, lens: nope }\n    - { slot: sidebar, lens: a }\n    - { slot: effort.review.details, lens: a }\n",
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
        assert_eq!(e.ui.slots.len(), 1);
        assert_eq!(e.ui.slots[0].lens_id, "mine/a");
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
            "manifest: 2\nname: mine\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: settings.section, lens: status }\n",
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
        assert_eq!(e.ui.slots[0].slot, "settings.section");
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
            // P10 (K3): the github example's pull-request kind is the use
            // that made `ref_kinds` stable.
            if name == "github" {
                let kinds: Vec<&str> = ext.ref_kinds.iter().map(|k| k.kind.as_str()).collect();
                assert_eq!(kinds, vec!["github_pr"]);
            }
        }
    }

    /// One project extension `x` with the given lens files and
    /// extension.yaml tail; returns its load result.
    fn load_x(files: &[(&str, &str)], manifest_tail: &str) -> (tempfile::TempDir, Extension) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            &if manifest_tail.starts_with("manifest:") {
                format!("name: x\n{manifest_tail}")
            } else {
                format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest_tail}")
            },
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

    /// A grid composes its children and never renders rows of its own, so
    /// it needs no `query` (like a form); every other viz still does.
    #[test]
    fn a_grid_needs_no_query() {
        let (_d, ext) = load_x(
            &[
                ("n", "title: N\nquery: SELECT 1 AS n\nviz: number\n"),
                ("all", "title: All\nviz: grid\nchildren: [n]\n"),
                ("bare", "title: B\nviz: table\n"),
            ],
            "",
        );
        assert!(
            ext.lenses.iter().any(|l| l.slug == "all"),
            "{:?}",
            ext.errors
        );
        assert!(!ext.lenses.iter().any(|l| l.slug == "bare"));
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("viz `table` needs a `query`")),
            "{:?}",
            ext.errors
        );
    }

    #[test]
    fn chart_lenses_parse_and_missing_fields_are_errors() {
        let (_d, ext) = load_x(
            &[
                ("visits", "title: V\nquery: SELECT 'a' AS day, 1 AS n\nviz: bar\nchart: { x: day, y: n }\n"),
                ("trend", "title: T\nquery: SELECT 1 AS at, 2 AS v, 'm' AS s\nviz: line\nchart: { x: at, y: v, series: s }\n"),
                ("map", "title: M\nquery: SELECT 'p' AS path, 3 AS churn, 'core' AS zone\nviz: treemap\nchart: { label: path, size: churn, group: zone }\n"),
                ("all", "title: All\nviz: grid\nchildren: [visits, trend]\n"),
                ("nobar", "title: N\nquery: SELECT 1\nviz: bar\n"),
                ("badgrid", "title: G\nviz: grid\nchildren: [nope]\n"),
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

    /// P6.A2: the structure components parse their blocks, and a missing
    /// column role is refused naming the block it goes under.
    #[test]
    fn structure_lenses_parse_and_missing_roles_are_errors() {
        let (_d, ext) = load_x(
            &[
                ("t", "title: T\nquery: SELECT 1 AS id, NULL AS p, 'a' AS n\nviz: tree\ntree: { id: id, parent: p, label: n }\n"),
                ("tl", "title: TL\nquery: SELECT 'x' AS at, 'y' AS w\nviz: timeline\ntimeline: { at: at, label: w }\n"),
                ("d", "title: D\nquery: SELECT 1 AS a\nviz: detail\n"),
                ("s", "title: S\nquery: SELECT 'a' AS l\nviz: steps\nsteps: { label: l }\n"),
                ("h", "title: H\nquery: SELECT 'f' AS p, 'working' AS a, 'working' AS b\nviz: hunks\nhunks: { path: p, from: a, to: b }\n"),
                ("notree", "title: N\nquery: SELECT 1\nviz: tree\ntree: { id: id, label: n }\n"),
                ("nohunks", "title: N\nquery: SELECT 1\nviz: hunks\n"),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        for slug in ["t", "tl", "d", "s", "h"] {
            assert!(lens(slug).is_some(), "{slug}: {:?}", ext.errors);
        }
        assert_eq!(
            lens("t").unwrap().tree.as_ref().unwrap().parent.as_deref(),
            Some("p")
        );
        assert!(lens("notree").is_none() && lens("nohunks").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("notree") && errs.contains("`tree: { parent: <column> }`"),
            "{errs}"
        );
        assert!(
            errs.contains("nohunks")
                && errs.contains("`hunks: { path: <column>, from: <column>, to: <column> }`"),
            "{errs}"
        );
    }

    /// Validation checks every block's columns, not only the chart's.
    #[tokio::test]
    async fn validate_checks_structure_columns_exist() {
        let (d, _) = load_x(
            &[(
                "a",
                "title: A\nquery: SELECT 'x' AS l\nviz: steps\nsteps: { label: l, status: gone }\n",
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(errs.contains("steps column `gone`"), "{errs}");
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
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
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
            "ui:\n  slots:\n    - { slot: work_item.detail.body, lens: t }\n    - { slot: thread.plan.header, lens: th }\n    - { slot: work_item.detail.body, lens: plain }\n",
        );
        let mounted: Vec<(&str, &str)> = ext
            .ui
            .slots
            .iter()
            .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
            .collect();
        assert_eq!(
            mounted,
            vec![
                ("work_item.detail.body", "x/t"),
                ("thread.plan.header", "x/th")
            ]
        );
        let errs = ext.errors.join("\n");
        assert!(errs.contains("task_id") && errs.contains("plain"), "{errs}");
    }

    /// P9.A2: a file diff's header strip takes lenses about the file — its
    /// path and the two revisions the diff reads.
    #[test]
    fn the_diff_file_header_slot_passes_the_path_and_both_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{EXT_V2}ui:\n  slots:\n    - {{ slot: diff.file.header, lens: partners }}\n    - {{ slot: diff.file.header, lens: plain }}\n"
        );
        write(
            dir.path(),
            "oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/partners.yaml",
            "title: Partners\nparams: [{ name: path }]\nquery: SELECT :path AS path\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/plain.yaml",
            "title: Plain\nquery: SELECT 1 AS n\n",
        );
        let e = only(dir.path(), "review");
        let mounted: Vec<(&str, &str)> =
            e.ui.slots
                .iter()
                .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
                .collect();
        assert_eq!(mounted, vec![("diff.file.header", "review/partners")]);
        let errs = e.errors.join("\n");
        for param in ["path", "left_revision", "right_revision", "stream_id"] {
            assert!(errs.contains(param), "{param}: {errs}");
        }
        assert!(errs.contains("plain"), "{errs}");
    }

    #[test]
    fn extensions_declare_measures_metrics_and_fact_collectors() {
        let manifest = [
            "measures:",
            "  - { key: acme.todo, title: TODOs }",
            "metrics:",
            "  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum }",
            "  - { use: oxplow.rust.unsafe_blocks }",
            "collectors:",
            "  - { id: acme.missing, runtime: starlark, entry: collectors/nope.star, facts: [acme.todo] }",
            "  - { id: acme.shell, runtime: exec, entry: collectors/todo.sh, facts: [acme.todo] }",
            "  - { id: acme.stray, runtime: starlark, entry: collectors/todo.star, facts: [acme.other] }",
            "",
        ]
        .join("\n");
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            &format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest}"),
        );
        write(
            dir.path(),
            "oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert_eq!(ext.measures.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.metrics.len(), 1, "{:?}", ext.errors);
        // A missing script and an extension's program recording facts are
        // refused; so is a `use:`, which only a project can write.
        assert!(ext.collectors.iter().all(|c| c.id == "acme.stray"));
        for (needle, also) in [
            ("acme.missing", "collectors/nope.star"),
            ("acme.shell", "sandboxed"),
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
        // A measure it doesn't declare is a warning, not an error.
        assert!(
            ext.warnings.iter().any(|w| w.contains("acme.other")),
            "{:?}",
            ext.warnings
        );
        // `gauges:` is a key the manifest doesn't have.
        let (_d, ext) = load_x(&[], "gauges:\n  - { key: acme.x }\n");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("unknown field `gauges`")),
            "{:?}",
            ext.errors
        );
    }

    /// P4.9 (tsk494): `models:` entries with their `models/<name>.sql`
    /// load as the extension's models; a declaration without its file, a
    /// file without its declaration, and a malformed entry are errors.
    #[test]
    fn extensions_declare_models_with_their_sql() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = [
            "manifest: 2",
            "name: x",
            "intent:",
            "  purpose: test",
            "models:",
            "  - name: late",
            "    version: 1",
            "    description: Blocked tasks.",
            "    columns:",
            "      - { name: id, type: INTEGER, doc: Task id. }",
            "  - { name: gone, version: 1, description: No file., columns: [] }",
            "",
        ]
        .join("\n");
        write(dir.path(), "oxplow/extensions/x/extension.yaml", &manifest);
        write(
            dir.path(),
            "oxplow/extensions/x/models/late.sql",
            "SELECT id FROM ref('task') WHERE status = 'blocked'",
        );
        write(
            dir.path(),
            "oxplow/extensions/x/models/stray.sql",
            "SELECT 1",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert_eq!(
            ext.models
                .iter()
                .map(|m| m.decl.name.as_str())
                .collect::<Vec<_>>(),
            Vec::<&str>::new(),
            "a broken declaration set loads no models"
        );
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains("oxplow/extensions/x/models/gone.sql is missing"),
            "{errors}"
        );

        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            &manifest.replace(
                "  - { name: gone, version: 1, description: No file., columns: [] }\n",
                "",
            ),
        );
        std::fs::remove_file(dir.path().join("oxplow/extensions/x/models/stray.sql")).unwrap();
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.models.len(), 1);
        assert_eq!(ext.models[0].file, "oxplow/extensions/x/models/late.sql");
        assert!(ext.models[0].sql.contains("ref('task')"));

        write(
            dir.path(),
            "oxplow/extensions/x/models/stray.sql",
            "SELECT 1",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(
            ext.errors.join("\n").contains("stray.sql has no entry"),
            "{:?}",
            ext.errors
        );

        let (_d, ext) = load_x(&[], "models:\n  - { name: late, colour: red }\n");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("models") && e.contains("colour")),
            "{:?}",
            ext.errors
        );
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
    fn an_extension_fact_collector_reads_its_own_script() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\nmeasures:\n  - { key: acme.todo }\ncollectors:\n  - { id: acme.todo_scan, runtime: starlark, entry: collectors/todo.star, trigger: { on: [snapshot.taken] }, facts: [acme.todo] }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.collectors.len(), 1);
        assert_eq!(
            read_extension_file(dir.path(), "x", "collectors/todo.star").as_deref(),
            Some("def transform(input):\n    return {\"facts\": []}\n")
        );
        assert_eq!(
            read_extension_file(dir.path(), "x", "collectors/none.star"),
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

    /// P6b.C1: slot names are one dotted namespace — any other is an
    /// unknown slot — and `ui:` keys are unknown at the top.
    #[test]
    fn an_unknown_slot_is_an_error_and_ui_keys_stay_under_ui() {
        let (_d, ext) = load_x(
            &[(
                "c",
                "title: C\nparams: [{ name: change_id }]\nquery: SELECT 1\n",
            )],
            "manifest: 2\nintent:\n  purpose: p\nui:\n  slots:\n    - { slot: commit, lens: c }\n",
        );
        assert!(ext.ui.slots.is_empty());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("unknown slot `commit`") && errs.contains("extension.yaml:"),
            "{errs}"
        );
        // Keys that live under `ui:` are unknown at the top.
        for key in ["slot_mounts", "decorators", "replacements"] {
            let (_d, ext) = load_x(
                &[],
                &format!("manifest: 2\nintent:\n  purpose: p\n{key}: []\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(&format!("unknown field `{key}`")),
                "{key}: {errs}"
            );
        }
    }

    /// P6.G1: a panel is a lens in the left nav, bound to a scope, with an
    /// optional badge lens whose alert gives its count.
    #[test]
    fn panels_are_scoped_lenses_with_a_badge() {
        let lenses = [
            ("open", "title: Open\nquery: SELECT 1\nparams: [{ name: stream_id, default: '' }]\n"),
            ("count", "title: Count\nquery: SELECT 1\nparams: [{ name: stream_id, default: '' }]\nalert: { min_rows: 1 }\n"),
            ("quiet", "title: Quiet\nquery: SELECT 1\n"),
        ];
        let (_d, ext) = load_x(
            &lenses,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: prs, title: PRs, icon: git-pull-request, scope: stream, body: open, badge: count }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.panels,
            vec![ExtensionPanel {
                id: "x/prs".into(),
                extension: "x".into(),
                title: "PRs".into(),
                icon: Some("git-pull-request".into()),
                scope: PanelScope::Stream,
                body: "x/open".into(),
                badge: Some("x/count".into()),
            }]
        );
        for (panel, says) in [
            (
                "{ id: a, title: A, scope: project, body: nope }",
                "isn't in lenses/",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, badge: quiet }",
                "declares no `alert`",
            ),
            (
                "{ id: a, title: A, scope: thread, body: open }",
                "must declare `thread_id`",
            ),
            ("{ id: a, title: A, scope: galaxy, body: open }", "scope"),
            (
                "{ id: Bad Id, title: A, scope: project, body: quiet }",
                "panel id",
            ),
        ] {
            let (_d, ext) = load_x(
                &lenses,
                &format!("manifest: 2\nintent:\n  purpose: p\npanels:\n  - {panel}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml"),
                "{panel}: {errs}"
            );
            assert!(ext.panels.is_empty(), "{panel}");
        }
    }

    /// P6.B1: an action is a command. Its placeholders must name a param
    /// (or, in a row action, a column); anything else is refused.
    #[test]
    fn lens_actions_are_commands() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 1 AS id\nparams: [{ name: item, default: '' }]\nactions:\n  - { id: finish, label: Finish, command: work_item.transition, input: { ref: '{{param.item}}', to: done } }\n  - { id: row, label: Row, command: work_item.transition, row: true, input: { ref: 'work_item:oxplow:tsk{{row.id}}', to: done } }\n",
                ),
                ("b", "title: B\nquery: SELECT 1\nactions: [copy]\n"),
                ("c", "title: C\nquery: SELECT 1\nactions: [{ action: run-source, source: a/b }]\n"),
                ("d", "title: D\nquery: SELECT 1\nactions: [{ id: x, label: X, command: work_item.create, input: { title: '{{param.nope}}' } }]\n"),
                ("e", "title: E\nquery: SELECT 1\nactions: [{ id: x, label: X, command: work_item.create, input: { title: '{{row.id}}' } }]\n"),
                ("f", "title: F\nquery: SELECT 1\nactions: [{ id: x, label: X, command: Not-A-Name }]\n"),
                ("g", "title: G\nquery: SELECT 1\nactions: [{ id: x, label: X, command: a.b }, { id: x, label: Y, command: a.c }]\n"),
            ],
            "",
        );
        let a = ext
            .lenses
            .iter()
            .find(|l| l.slug == "a")
            .expect("valid actions load");
        assert_eq!(
            a.actions
                .iter()
                .map(|x| (x.id.as_str(), x.command.as_str(), x.row))
                .collect::<Vec<_>>(),
            vec![
                ("finish", "work_item.transition", false),
                ("row", "work_item.transition", true)
            ]
        );
        for (slug, needle) in [
            ("b", "actions:"),
            ("c", "unknown field `action`"),
            ("d", "names no param"),
            ("e", "needs `row: true`"),
            ("f", "command name"),
            ("g", "twice"),
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

    /// A row action's `{{row.x}}` must be a column the query returns.
    #[tokio::test]
    async fn validate_checks_row_action_columns() {
        let (d, _) = load_x(
            &[(
                "a",
                "title: A\nquery: SELECT 1 AS id\nactions: [{ id: x, label: X, command: a.b, row: true, input: { v: '{{row.gone}}' } }]\n",
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            v.errors.join("\n").contains("`{{row.gone}}`"),
            "{:?}",
            v.errors
        );
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
                e.lenses.is_empty() && e.ui.slots.is_empty() && e.collectors.is_empty(),
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

        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
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
            "ui:\n  slots:\n    - { slot: vcs.commit.details, lens: files }\n    - { slot: vcs.status.details, lens: files }\n    - { slot: effort.review.details, lens: files }\n    - { slot: effort.review.details, lens: both }\n    - { slot: vcs.commit.details, lens: none }\n",
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
            .ui
            .slots
            .iter()
            .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
            .collect();
        assert_eq!(
            mounted,
            vec![
                ("vcs.commit.details", "x/files"),
                ("vcs.status.details", "x/files"),
                ("effort.review.details", "x/files"),
                ("effort.review.details", "x/both")
            ],
            "a slot lens needs at least one of the slot's params"
        );
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("none") && errs.contains("change_id"),
            "{errs}"
        );
    }

    const LAUNCHER_MANIFEST: &str = "manifest: 2\nintent:\n  purpose: p\nlauncher:\n";

    /// P6.D1: a launcher entry opens a ref, runs a command, or puts a
    /// prompt in the agent's input — each form checked for shape.
    #[test]
    fn launcher_entries_are_refs_commands_or_prompts() {
        let (_d, ext) = load_x(
            &[],
            &format!(
                "{LAUNCHER_MANIFEST}  - {{ label: Settings, category: System, target: {{ ref: 'page:settings' }} }}\n  - {{ label: New Task, category: Work, target: {{ command: work_item.create, input: {{ title: x }} }} }}\n  - {{ label: Why slow, category: Code, target: {{ prompt: 'Why is the build slow?' }} }}\n"
            ),
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.launcher
                .iter()
                .map(|e| e.target.clone())
                .collect::<Vec<_>>(),
            vec![
                LauncherTarget::Ref {
                    r#ref: "page:settings".into()
                },
                LauncherTarget::Command {
                    command: "work_item.create".into(),
                    input: serde_json::json!({ "title": "x" }),
                },
                LauncherTarget::Prompt {
                    prompt: "Why is the build slow?".into()
                },
            ]
        );

        for (entry, want) in [
            ("{ ref: 'not a ref' }", "not a canonical ref"),
            // Grammar isn't enough: the kind must be registered and the id
            // well-formed, or the launcher would drop the entry silently.
            ("{ ref: 'bogus:thing' }", "unknown kind `bogus`"),
            ("{ ref: 'commit:not-hex' }", "not a valid `commit` id"),
            ("{ ref: 'thread:thr1' }", "doesn't open as a page"),
            ("{ command: Not-A-Name }", "`Not-A-Name`"),
            ("{ command: a.b, input: [1] }", "`input` must be a map"),
            ("{ prompt: '  ' }", "an empty prompt"),
            // A line break pasted into a terminal is Enter: it would send.
            ("{ prompt: \"Why?\\nAnd how?\" }", "one line"),
            (
                "{ ref: 'page:settings', prompt: hi }",
                "one of `ref`, `command` or `prompt`",
            ),
            ("'page:settings'", "one of `ref`, `command` or `prompt`"),
        ] {
            let (_d, ext) = load_x(
                &[],
                &format!(
                    "{LAUNCHER_MANIFEST}  - {{ label: Bad, category: Work, target: {entry} }}\n"
                ),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(want) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(ext.launcher.is_empty(), "{entry}");
        }
    }

    /// A command entry names a registered command and its input fits;
    /// without a registry (the CLI) that half isn't checked, and says so.
    #[tokio::test]
    async fn validate_checks_launcher_commands_against_the_registry() {
        let (d, _) = load_x(
            &[],
            &format!(
                "{LAUNCHER_MANIFEST}  - {{ label: A, category: Work, target: {{ command: no.such }} }}\n  - {{ label: B, category: Work, target: {{ command: work_item.create, input: {{ nope: 1 }} }} }}\n  - {{ label: C, category: Work, target: {{ command: work_item.create, input: {{ title: ok }} }} }}\n"
            ),
        );
        let schema = |name: &str| {
            (name == "work_item.create").then(|| {
                serde_json::json!({
                    "type": "object",
                    "properties": { "title": { "type": "string" } },
                    "required": ["title"],
                    "additionalProperties": false
                })
            })
        };
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", Some(&schema))
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("launcher entry `A`: no command `no.such`"),
            "{errs}"
        );
        assert!(
            errs.contains("launcher entry `B`: the input doesn't fit `work_item.create`"),
            "{errs}"
        );
        assert!(!errs.contains("entry `C`"), "{errs}");

        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            !v.errors.join("\n").contains("launcher entry"),
            "{:?}",
            v.errors
        );
        assert!(
            v.warnings
                .join("\n")
                .contains("its commands weren't checked"),
            "{:?}",
            v.warnings
        );
    }

    /// P6.G2: an extension's page is a lens under a launcher category, at
    /// `page:ext.<extension>.<page>`.
    #[test]
    fn pages_are_lenses_with_a_route() {
        let lenses = [("open", "title: Open\nquery: SELECT 1\n")];
        let (_d, ext) = load_x(
            &lenses,
            "manifest: 2\nintent:\n  purpose: p\npages:\n  - { id: open-prs, title: Open PRs, icon: git-pull-request, category: Work, lens: open }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.pages,
            vec![ExtensionPage {
                id: "open-prs".into(),
                extension: "x".into(),
                page_ref: "page:ext.x.open-prs".into(),
                title: "Open PRs".into(),
                icon: Some("git-pull-request".into()),
                category: LauncherCategory::Work,
                lens: "x/open".into(),
            }]
        );
        for (page, says) in [
            (
                "{ id: a, title: A, category: Work, lens: nope }",
                "isn't in lenses/",
            ),
            (
                "{ id: Bad Id, title: A, category: Work, lens: open }",
                "page id",
            ),
            (
                "{ id: a, title: A, category: Nowhere, lens: open }",
                "category",
            ),
        ] {
            let (_d, ext) = load_x(
                &lenses,
                &format!("manifest: 2\nintent:\n  purpose: p\npages:\n  - {page}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml"),
                "{page}: {errs}"
            );
            assert!(ext.pages.is_empty(), "{page}");
        }
    }

    /// P8.C1: an extension loads from any revision — at `git:HEAD` of a
    /// clean worktree exactly as from disk, and at a snapshot as that
    /// snapshot holds it, whatever the disk says now.
    #[tokio::test]
    async fn an_extension_loads_at_any_revision() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        let manifest = |purpose: &str| {
            format!(
                "manifest: 2\nname: acme\ndescription: {purpose}\nintent:\n  purpose: {purpose}\n  origin: thread:thr1\n  examples: []\n"
            )
        };
        let dir = ws.join("oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("lenses")).unwrap();
        std::fs::write(dir.join("extension.yaml"), manifest("Committed.")).unwrap();
        std::fs::write(
            dir.join("lenses/streams.yaml"),
            "title: Streams\nquery: SELECT id FROM v_stream\n",
        )
        .unwrap();
        crate::test_fixtures::commit_all(&ws, "acme");

        let at_head = extension_at(&f.svc.trees, &ws, &Revision::git("HEAD"), "acme")
            .await
            .unwrap()
            .expect("acme is at HEAD");
        assert_eq!(at_head, load_project_extension(&ws, "acme"));
        assert_eq!(at_head.lenses.len(), 1);
        assert!(
            extension_at(&f.svc.trees, &ws, &Revision::git("HEAD"), "nope")
                .await
                .unwrap()
                .is_none()
        );

        // A snapshot holding an edited manifest loads the edit, though the
        // disk holds the committed one.
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snap = f.svc.snapshot_store.create_snapshot(stream).await.unwrap();
        let edited = manifest("Edited.");
        let hash = f.svc.blobs.write(edited.as_bytes()).unwrap();
        f.svc
            .snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "oxplow/extensions/acme/extension.yaml".into(),
                blob_hash: Some(hash),
                size_bytes: edited.len() as i64,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        let at_snapshot = extension_at(&f.svc.trees, &ws, &Revision::Snapshot(snap), "acme")
            .await
            .unwrap()
            .expect("acme is in the snapshot");
        assert_eq!(at_snapshot.description, "Edited.");
    }

    /// P8.C7: an effort's review names the extensions that changed between
    /// its two revisions, each with what the change does; files outside
    /// `oxplow/extensions/` change nothing here.
    #[tokio::test]
    async fn an_effort_review_names_the_extensions_that_changed() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snapshot = |files: Vec<(&'static str, String)>| {
            let svc = f.svc.clone();
            async move {
                let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
                for (path, body) in files {
                    let hash = svc.blobs.write(body.as_bytes()).unwrap();
                    svc.snapshot_store
                        .capture(oxplow_db::FileSnapshot {
                            id: 0,
                            stream_id: stream,
                            path: path.into(),
                            blob_hash: Some(hash),
                            size_bytes: body.len() as i64,
                            captured_at: oxplow_domain::Timestamp::now(),
                            storage: oxplow_db::SnapshotStorage::Oxplow,
                            snapshot_id: Some(id),
                            mtime_ms: None,
                            content_hash: None,
                        })
                        .await
                        .unwrap();
                }
                id
            }
        };
        let manifest = "manifest: 2\nname: acme\nintent:\n  purpose: Count.\n  origin: thread:thr1\n  examples: []\n".to_string();
        let lens = |n: u32| format!("title: Count\nquery: SELECT {n} AS n\nviz: number\n");
        let start = snapshot(vec![
            ("oxplow/extensions/acme/extension.yaml", manifest.clone()),
            ("oxplow/extensions/acme/lenses/count.yaml", lens(1)),
            ("src/a.rs", "fn a() {}\n".into()),
        ])
        .await;
        let end = snapshot(vec![
            ("oxplow/extensions/acme/lenses/count.yaml", lens(2)),
            ("src/a.rs", "fn a() { 1 }\n".into()),
        ])
        .await;
        let changes = extension_changes_between(
            &f.svc,
            &ws,
            Some(&Revision::Snapshot(start)),
            &Revision::Snapshot(end),
        )
        .await
        .unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].name, "acme");
        assert_eq!(changes[0].change, crate::extension_effects::Change::Changed);
        let report = changes[0].effects.as_ref().unwrap();
        assert!(
            report.lines.iter().any(|l| l == "Lens acme/count: changed"),
            "{:?}",
            report.lines
        );
    }

    /// tsk791: the earlier side of a review is only read — its lenses
    /// rendered on its own models — never checked: its command examples
    /// don't run (here, a runaway one the later version removed).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_earlier_side_of_a_review_is_not_checked() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snapshot = |files: Vec<(&'static str, String)>| {
            let svc = f.svc.clone();
            async move {
                let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
                for (path, body) in files {
                    let hash = svc.blobs.write(body.as_bytes()).unwrap();
                    svc.snapshot_store
                        .capture(oxplow_db::FileSnapshot {
                            id: 0,
                            stream_id: stream,
                            path: path.into(),
                            blob_hash: Some(hash),
                            size_bytes: body.len() as i64,
                            captured_at: oxplow_domain::Timestamp::now(),
                            storage: oxplow_db::SnapshotStorage::Oxplow,
                            snapshot_id: Some(id),
                            mtime_ms: None,
                            content_hash: None,
                        })
                        .await
                        .unwrap();
                }
                id
            }
        };
        let manifest = "manifest: 2\nname: acme\nintent:\n  purpose: Count.\n  origin: thread:thr1\n  examples: []\n";
        let command = "commands:\n  - name: spin\n    summary: Spin.\n    input_schema: { type: object }\n    entry: handlers/spin.star\n    examples:\n      - { name: once, input: {}, expect_commands: [] }\n";
        let start = snapshot(vec![
            (
                "oxplow/extensions/acme/extension.yaml",
                format!("{manifest}{command}"),
            ),
            (
                "oxplow/extensions/acme/handlers/spin.star",
                "def transform(x):\n    n = 0\n    for i in range(400000000):\n        n += i\n    return {\"commands\": []}\n".into(),
            ),
        ])
        .await;
        let end = snapshot(vec![(
            "oxplow/extensions/acme/extension.yaml",
            manifest.to_string(),
        )])
        .await;
        let started = std::time::Instant::now();
        let changes = extension_changes_between(
            &f.svc,
            &ws,
            Some(&Revision::Snapshot(start)),
            &Revision::Snapshot(end),
        )
        .await
        .unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }
}
