//! Project extensions and their lenses.
//!
//! An extension is a folder `.oxplow/extensions/<name>/` in a stream's
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

pub mod ai_profiles;
pub(crate) mod candidates;
pub mod config;
pub mod config_schema;
pub mod custom_components;
pub mod decorators;
pub mod descriptors;
pub mod implementations;
pub mod imports;
pub mod jobs;
pub mod manifest_v2;
pub mod public_contracts;
pub mod replacements;
pub mod schema;
pub mod skills;
use manifest_v2::{at, entry_line, item_lines, key_line, line_under, ManifestV2};
pub use manifest_v2::{Intent, IntentExample, IntentPrompt};
pub use oxplow_domain::permissions::Permissions;

/// Where an extension's source is by default, relative to a checkout
/// (`extensionSources` may point a name elsewhere).
pub const EXTENSIONS_DIR: &str = oxplow_config::EXTENSION_SOURCES_DIR;

/// Where a loaded extension's files came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deployment {
    /// Read from its source folder: an authoring check, the CLI, a
    /// standalone load.
    #[default]
    Source,
    /// The deployment that ships inside oxplow (a bundled extension with no
    /// deployment of its own), written to the store once per hash.
    Shipped { hash: String },
    /// A deployment in the store.
    Deployed { hash: String },
    /// Its source is here but it has no deployment yet: listed, not run.
    Undeployed,
    /// Listed as on, but neither its source nor a deployment is on this
    /// machine: listed, not run, and not an error.
    NotHere,
}

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
    /// The first row's text (`markdown: { text }`, else its first column), rendered as markdown; with an `edit` action, edited.
    Markdown,
    /// Bars: `chart.x` labels, `chart.y` values.
    Bar,
    /// A line over `chart.x` (a time or number), `chart.y` values, one
    /// line per `chart.series` value when set.
    Line,
    /// Nested rectangles sized by `chart.size`, labelled by `chart.label`,
    /// grouped (and coloured) by `chart.group`.
    Treemap,
    /// A page of sections (`sections`), wiki style: each another lens,
    /// given the params it declares from this lens's, or markdown. Other
    /// extensions add sections at `<extension>/<slug>.sections`.
    Document,
    /// A grid of tiles (`tiles`), each another lens (or markdown) at a
    /// size, given the params it declares from this lens's. Other
    /// extensions add tiles at `<extension>/<slug>.tiles`.
    Dashboard,
    /// Rows nested by `tree.parent` (a row's parent is the row whose
    /// `tree.id` it names; none, or one not in the result, is a root),
    /// each shown as `tree.label`.
    Tree,
    /// Rows in time order (`timeline.at`), each shown as
    /// `timeline.label`, linked through `timeline.ref` when set.
    Timeline,
    /// The first row as label/value pairs (the displayed columns).
    Detail,
    /// The rows as cards in columns, one column per `group` value (its
    /// `values` first, in order, empty ones too); a card is drawn as a
    /// list row. A drag action moves a card between columns.
    Board,
    /// An ordered checklist: `steps.label`, with `steps.status` (`done`,
    /// `active`, `failed`, anything else pending).
    Steps,
    /// Per row, the file `code.path` at revision `code.revision`,
    /// read-only, at full size: its `code.lines` marked and revealed, the
    /// editors side by side (scrolling together with `code.sync`).
    Code,
    /// The first row's file `diff.path` between revisions `diff.from` and
    /// `diff.to`, at full size, read-only; `diff.lines` revealed on the
    /// newer side.
    Diff,
    /// Per row, the diff of the file `hunks.path` between the revisions
    /// `hunks.from` and `hunks.to` (`working`, `snap:<id>`,
    /// `<vcs>:<rev>`), read-only.
    Hunks,
    /// A form for command `form.command`: its fields from the command's
    /// input schema, prefilled from `form.defaults` (placeholders bound
    /// like an action's) and the query's first row, if the lens has one.
    /// Submitting runs the command as the lens.
    Form,
    /// The pending proposals an agent raised, each with Approve and
    /// Decline: core's consent component. The viz reads
    /// `v_command_proposal` itself and decides through the person-only
    /// path, so the lens needs no query beyond `SELECT 1`; its params
    /// `thread` (a thread ref) and `setting` (a config key) narrow the
    /// list.
    Approvals,
    /// The extension's own web component (`custom.component`), in
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
    #[specta(type = Option<oxplow_domain::JsNumber>)]
    pub min_rows: Option<i64>,
    pub column: Option<String>,
    pub above: Option<f64>,
    pub below: Option<f64>,
    /// Badge text; else the row count or the column's name.
    pub label: Option<String>,
}

/// `alert:` as written in a lens file (snake_case keys).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, schemars::JsonSchema)]
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
    #[specta(type = oxplow_domain::JsNumber)]
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
    /// `line` only: a column whose values each draw a horizontal target
    /// line across the chart (a metric's target, a budget).
    #[serde(default)]
    pub targets: Option<String>,
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
    /// A lazy tree: the lens param a branch's children are read with. The
    /// run's rows are one level (the param's `default` is the top level);
    /// a branch opens by running the lens again with the param set to the
    /// row's `id`, its rows spliced under it. Needs `branch`.
    #[serde(default)]
    pub lazy: Option<String>,
    /// A lazy tree's rows that open: the column that is truthy on them. A
    /// row with children in the result nests them as an eager tree does.
    #[serde(default)]
    pub branch: Option<String>,
}

/// `dashboard` viz: its query's rows are tiles, one per row, each value
/// naming a column. A row draws its `lens` (run with the page's params it
/// declares), its pinned `sql` (shown as `display`, a chart by `chart`,
/// the metric card by `metric` and `viz`) or its `text` (a heading band).
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensTile {
    /// The tile's id: what its actions bind as `{{row.<id column>}}`.
    pub id: Option<String>,
    pub title: Option<String>,
    /// `small`, `wide`, `tall` or `full` (unknown: `small`; a text tile's
    /// default is `full`).
    pub size: Option<String>,
    /// A lens id (`<extension>/<slug>`).
    pub lens: Option<String>,
    /// Read-only SQL over the published models.
    pub sql: Option<String>,
    /// How a `sql` tile shows its rows: a viz name, or `metric`.
    pub display: Option<String>,
    /// A `sql` tile's chart roles, as JSON (`{"x": …, "y": …}`).
    pub chart: Option<String>,
    /// A `metric` display's metric key.
    pub metric: Option<String>,
    /// A `metric` display's card: `line` or `number`.
    pub viz: Option<String>,
    pub text: Option<String>,
}

impl LensTile {
    /// Each role and the column it names.
    pub fn roles(&self) -> [(&'static str, &Option<String>); 10] {
        [
            ("id", &self.id),
            ("title", &self.title),
            ("size", &self.size),
            ("lens", &self.lens),
            ("sql", &self.sql),
            ("display", &self.display),
            ("chart", &self.chart),
            ("metric", &self.metric),
            ("viz", &self.viz),
            ("text", &self.text),
        ]
    }
}

/// `timeline` viz: when each row happened and what it says.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensTimeline {
    pub at: Option<String>,
    pub label: Option<String>,
    /// A column holding the URI each entry links to.
    #[serde(rename = "uri")]
    pub uri_column: Option<String>,
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

/// What becomes of the page a lens is on after one of its actions (or its
/// form) ran a command whose `ui.open_after` names a page.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ActionThen {
    /// The page opens in this tab; Back returns to the lens's page.
    #[default]
    Open,
    /// The page takes this tab and the lens's page leaves its history: Back
    /// never returns to a submitted form. A page that can't live in a tab
    /// (an agent session) closes this one and is focused.
    Replace,
    /// Nothing opens; the lens's page stays (a form for the next entry).
    Stay,
    /// The lens's page closes (it showed what the command removed).
    Close,
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
    /// Fields shown but not editable: their values are `defaults`', imposed
    /// again on submit (an item's `ref`).
    #[serde(default)]
    pub fixed: Vec<String>,
    /// Whether it gathers something new (`create`) or edits what is saved
    /// (`edit`: Save when changed, Discard).
    #[serde(default)]
    pub mode: FormMode,
    /// The fields to show, in order: a name, or `{ name, visible_if,
    /// secret }`. The rest keep their starting values and are submitted
    /// with them. None: every field.
    #[serde(default)]
    pub fields: Option<Vec<LensFormField>>,
    /// Fields picked from a list: each names a lens (`<extension>/<slug>`)
    /// run with the form's params, whose rows are the choices (`value`,
    /// `label`, optional `group`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, String>,
    /// What becomes of the page after a submit whose command opens one.
    #[serde(default)]
    pub then: ActionThen,
    /// A second submit: it runs the same command, then the form starts
    /// again with the `keep` fields' values carried over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub again: Option<FormAgain>,
}

/// A form's second submit (Save and Another).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormAgain {
    /// Its button.
    pub label: String,
    /// The fields whose values the next entry starts with; the rest start
    /// from the form's defaults.
    #[serde(default)]
    pub keep: Vec<String>,
}

/// What a form is for.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum FormMode {
    /// Gathers something new; after a submit it starts again.
    #[default]
    Create,
    /// Edits what is saved: it starts from the saved values, Save is
    /// offered only when changed, and Discard puts them back.
    Edit,
}

/// One of a form's `fields`: its name, or its name and how it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum LensFormField {
    Name(String),
    Spec(LensFormFieldSpec),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensFormFieldSpec {
    pub name: String,
    /// Shown only while each named field's value is one of these; hidden,
    /// it is neither required nor submitted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visible_if: BTreeMap<String, FieldValues>,
    /// Masked, never kept in a draft, and blank leaves it out of the
    /// input; the command must keep it out of its record (`unrecorded`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    /// An object field whose children are the rows of this lens
    /// (`<extension>/<slug>`: `name`, `title`, `kind` of `enum`, `text` or
    /// `number`, `values`, `read_only`), run with the form's params. The
    /// field's value is the object of those children; it is read again
    /// when the lens's data changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// A `visible_if` condition: one value or any of several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum FieldValues {
    Any(Vec<FieldValue>),
    One(FieldValue),
}

/// A field's value a condition compares with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum FieldValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

static NO_CONDITIONS: BTreeMap<String, FieldValues> = BTreeMap::new();

impl LensFormField {
    pub fn name(&self) -> &str {
        match self {
            LensFormField::Name(n) => n,
            LensFormField::Spec(s) => &s.name,
        }
    }
    pub fn secret(&self) -> bool {
        matches!(self, LensFormField::Spec(s) if s.secret)
    }
    /// The lens its children are read from, for an object field.
    pub fn from(&self) -> Option<&str> {
        match self {
            LensFormField::Spec(s) => s.from.as_deref(),
            LensFormField::Name(_) => None,
        }
    }
    pub fn visible_if(&self) -> &BTreeMap<String, FieldValues> {
        match self {
            LensFormField::Name(_) => &NO_CONDITIONS,
            LensFormField::Spec(s) => &s.visible_if,
        }
    }
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

/// `code` viz: the file each row shows, read-only, at a revision; the
/// lines it marks and reveals; whether the editors scroll together.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensCode {
    pub path: Option<String>,
    pub revision: Option<String>,
    /// A column holding `12-40` (or `12`): the lines marked and revealed.
    #[serde(default)]
    pub lines: Option<String>,
    /// Several rows' editors scroll together, each from its marked lines.
    #[serde(default)]
    pub sync: bool,
}

/// `diff` viz: the first row's file, between two revisions, at full size.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensDiff {
    pub path: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// A column holding a line (or `12-40`: its start) to reveal on the
    /// newer side.
    #[serde(default)]
    pub lines: Option<String>,
    /// A column naming the older side as the header says it (`vs HEAD`);
    /// else the `from` revision, shortened.
    #[serde(default)]
    pub base: Option<String>,
}

/// `markdown` viz: which column of the first row is the text. Without
/// it, the first column. A lens with an `edit` action names it: the text
/// is what the editor saves.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensMarkdown {
    #[serde(default)]
    pub text: Option<String>,
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
            &self.targets,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Launcher (Cmd+K) sections a lens can be listed under. Mirrors the
/// renderer's `PageCategory`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
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
    /// `oxplow:oxplow/file?path=<path>`; the value is a repo-relative path.
    File,
    /// `oxplow:oxplow/wiki?slug=<slug>`.
    Wiki,
    /// The effort's diff view; the value is an effort id.
    EffortDiff,
    /// A git commit; the value is a sha.
    Commit,
    /// A metric's page; the value is a metric key.
    Metric,
    /// Any oxplow page by its tab id (`oxplow:oxplow/work_item?id=<id>&provider=oxplow`,
    /// `oxplow:oxplow/commit?sha=<sha>`, `oxplow:oxplow/page?page=git-dashboard`, …),
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
    /// A column of the same row holding this one's unit (`ms`, `%`,
    /// `lines`): a number shows as a metric value in it.
    #[serde(default)]
    pub unit: Option<String>,
    /// A column of the same row naming an icon drawn before the cell
    /// (`done`, `in_progress`, `epic`, `wiki`, …; the UI's fixed
    /// vocabulary, `lens/lensIcons.tsx` — an unknown name draws none).
    #[serde(default)]
    pub icon: Option<String>,
    /// A column of the same row naming the cell's tone: `accent`,
    /// `success`, `warning`, `danger` or `muted` (anything else is plain).
    #[serde(default)]
    pub tone: Option<String>,
    /// Its width in pixels; the columns without one share the rest.
    #[serde(default)]
    pub width: Option<u32>,
    /// How its value reads; else as the query returned it (a timestamp in
    /// local time, a number with its `unit`).
    #[serde(default)]
    pub format: Option<ColumnFormat>,
    /// A param of the lens: the cell's text is drawn with every
    /// case-insensitive occurrence of its current value marked.
    #[serde(default)]
    pub highlight: Option<String>,
    /// A param of the lens that clicking a row sets to this column's
    /// value, re-running what reads it (a document's other sections):
    /// master and detail. The row whose value it holds is drawn selected.
    #[serde(default)]
    pub select: Option<String>,
    /// A lens (`<slug>` of this extension or `<extension>/<slug>`) the row
    /// opens into: run with the row's values (the params it declares), its
    /// result drawn under the row.
    #[serde(default)]
    pub expand: Option<String>,
}

/// How a column's value reads in the UI. An agent reads the raw value.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ColumnFormat {
    /// A timestamp's date.
    Date,
    /// A timestamp's date and time.
    Datetime,
    /// A timestamp as how long ago (`3 h ago`).
    Relative,
    /// A byte count (`1.2 MB`).
    Bytes,
    /// Milliseconds as a duration (`1.4 s`, `3 m`).
    Duration,
    /// A whole number with separators (`12,345`).
    Integer,
    /// A fraction 0–1 as a percentage (`42%`).
    Percent,
    /// A series (a JSON array of numbers) as a small line, with its
    /// latest value.
    Sparkline,
    /// A short value as a pill, in the column's `tone`.
    Badge,
}

/// One part of a `document` (a section) or a `dashboard` (a tile):
/// another lens, or markdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensPart {
    /// Stable within the lens: what another extension's mount names to go
    /// beside it (`at: { after: <id> }`).
    pub id: String,
    /// Its heading; else its lens's title.
    pub title: Option<String>,
    /// The lens it shows (`<extension>/<slug>`).
    pub lens: Option<String>,
    /// Or prose it shows.
    pub markdown: Option<String>,
    /// A tile's size.
    pub size: Option<TileSize>,
    /// The core component its lens stands in for (a target of
    /// `oxplow_domain::replaceable::REPLACEABLE`): its lens is oxplow's own,
    /// shown unless an extension replaces the target.
    pub replaceable: Option<String>,
    /// The extension that mounted it, when it isn't the lens's own.
    pub from: Option<String>,
}

/// A part as a lens file writes it (`sections:` / `tiles:` entries).
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PartFile {
    /// Defaults to its lens's slug; required for markdown.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    /// A lens slug in this extension, or `<ext>/<slug>`.
    #[serde(default)]
    lens: Option<String>,
    #[serde(default)]
    markdown: Option<String>,
    /// A tile's size (`tiles:` only).
    #[serde(default)]
    size: Option<TileSize>,
    /// The core component this part's lens stands in for (a lens part's).
    #[serde(default)]
    replaceable: Option<String>,
}

/// A lens file's `sections:` (for `document`) or `tiles:` (for
/// `dashboard`) as parts: ids defaulted and unique, exactly one of `lens`
/// and `markdown`, a size only on a tile. A dashboard whose rows are its
/// tiles (`row_tiles`) may have none.
fn parse_parts(
    extension: &str,
    viz: LensViz,
    sections: Vec<PartFile>,
    tiles: Vec<PartFile>,
    row_tiles: bool,
) -> Result<Vec<LensPart>, String> {
    let (key, files) = match viz {
        LensViz::Document if tiles.is_empty() => ("sections", sections),
        LensViz::Dashboard if sections.is_empty() => ("tiles", tiles),
        LensViz::Document => return Err("a `document` has `sections`, not `tiles`".into()),
        LensViz::Dashboard => return Err("a `dashboard` has `tiles`, not `sections`".into()),
        _ if sections.is_empty() && tiles.is_empty() => return Ok(Vec::new()),
        _ => return Err("`sections` are a `document`'s and `tiles` a `dashboard`'s".into()),
    };
    if files.is_empty() && !row_tiles {
        return Err(format!("viz `{}` needs `{key}: [...]`", viz_name(viz)));
    }
    let mut out: Vec<LensPart> = Vec::new();
    for (i, f) in files.into_iter().enumerate() {
        let lens = f.lens.map(|c| {
            if c.contains('/') {
                c
            } else {
                format!("{extension}/{c}")
            }
        });
        if lens.is_some() == f.markdown.is_some() {
            return Err(format!(
                "{key}[{i}]: needs exactly one of `lens` and `markdown`"
            ));
        }
        if let Some(target) = &f.replaceable {
            if lens.is_none() {
                return Err(format!("{key}[{i}]: `replaceable` is a lens part's"));
            }
            if oxplow_domain::replaceable::replaceable(target).is_none() {
                return Err(format!(
                    "{key}[{i}]: `replaceable: {target}` isn't a replaceable component ({})",
                    oxplow_domain::replaceable::targets()
                ));
            }
        }
        if key == "sections" && f.size.is_some() {
            return Err(format!("{key}[{i}]: `size` is a dashboard tile's"));
        }
        let id = match (f.id, &lens) {
            (Some(id), _) => id,
            (None, Some(l)) => l.rsplit('/').next().unwrap_or(l).to_string(),
            (None, None) => return Err(format!("{key}[{i}]: a markdown part needs an `id`")),
        };
        if out.iter().any(|p| p.id == id) {
            return Err(format!("{key}: two parts are `{id}`; give one an `id`"));
        }
        out.push(LensPart {
            id,
            title: f.title,
            lens,
            markdown: f.markdown,
            size: f.size,
            replaceable: f.replaceable,
            from: None,
        });
    }
    Ok(out)
}

/// A viz as a lens file names it.
fn viz_name(viz: LensViz) -> String {
    serde_json::to_value(viz)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// `group` on a `list` or `table`: the rows under a heading per distinct
/// value of `by`, in the order each first appears; `link` makes the
/// heading a link, read from the group's first row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensGroup {
    /// The column whose value groups the rows (not shown as a cell).
    pub by: String,
    /// The heading's link; `from` defaults to `by`.
    #[serde(default)]
    pub link: Option<LensLink>,
    /// The groups in this order, shown even when no row has the value
    /// (an empty board column is still somewhere to drop); values the
    /// rows have beyond these follow, in the order each first appears.
    #[serde(default)]
    pub values: Vec<String>,
}

/// A value the viewer (or an agent) can set when running the lens,
/// bound into the query as `:name`.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LensParam {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Used when the caller doesn't supply the param.
    #[serde(default)]
    pub default: Option<SqlCell>,
    /// The values it takes, when it's a choice: a panel shows
    /// them as a toggle in its header, a lens page as a select. Its
    /// `default` must be one; a run supplying another is refused.
    #[serde(default)]
    pub options: Vec<LensParamOption>,
    /// A lens whose rows are the values it takes, when it's a choice
    /// whose options aren't fixed (`value`, `label`, optional `group` —
    /// the form `choices` contract): a slug of this extension, or
    /// `<extension>/<slug>`; a loaded lens has the whole id. The page runs
    /// it with the params it declares of the page's, so an `end` picker's
    /// lens can leave out what `start` is. Its options aren't known at
    /// load, so no `default` is needed and any text is taken. Exclusive
    /// with `options`.
    #[serde(default)]
    pub options_from: Option<String>,
    /// What it holds, and so how a page draws it. Left out, it is a
    /// `choice` when it has `options` or `options_from`, else `text`; a
    /// loaded lens always says.
    #[serde(default, rename = "type")]
    pub kind: Option<ParamType>,
    /// What it is to the page, and so where it rides in the page's URI:
    /// a `resource` (the default) names what the page shows, in the URI's
    /// `?`; `filter` narrows how it shows it, in the URI's `#` view. A
    /// `selection` follows from the columns and is never written: a param
    /// a column of the lens (or of a part of the page) selects, in the
    /// view too. A loaded lens always says.
    #[serde(default)]
    pub role: Option<ParamRole>,
    /// A `date_range`'s quick picks, of `1d 2d 3d 7d 30d 90d all`; the
    /// default is `1d 7d 30d all`.
    #[serde(default)]
    pub presets: Vec<String>,
}

/// What a lens param holds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    /// Any value; a text box.
    Text,
    /// One of its `options`; a select.
    Choice,
    /// True or false, bound as 1 or 0; a checkbox.
    Bool,
    /// A span of time: a preset (`7d`, `all`) or `<from>..<to>` (dates or
    /// times, either end empty for open). Its own value is that text;
    /// the query also gets `:<name>_from` and `:<name>_to` (RFC 3339,
    /// NULL for an open end), a date's `to` the day after so the day
    /// counts.
    DateRange,
    /// A branch's name, NULL for every branch; a select of the branches.
    Branch,
    /// Text to look for, trimmed, NULL when empty; a box that applies as
    /// you type.
    Search,
}

/// What a lens param is to its page.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ParamRole {
    /// Names what the page shows (its URI's `?`); a control in the
    /// page's rail whose change opens another resource.
    Resource,
    /// Narrows how the page shows it (its URI's `#` view); a control in
    /// the page's rail whose change replaces the view.
    Filter,
    /// The row a click picked; the rail shows that row, never an input.
    Selection,
}

/// A `date_range`'s presets and how far back each reaches (`all`: no
/// start).
const RANGE_PRESETS: &[(&str, Option<i64>)] = &[
    ("1d", Some(1)),
    ("2d", Some(2)),
    ("3d", Some(3)),
    ("7d", Some(7)),
    ("30d", Some(30)),
    ("90d", Some(90)),
    ("all", None),
];

/// One value a choice param takes, and what the viewer sees for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensParamOption {
    pub value: String,
    pub label: String,
}

impl LensParam {
    /// Its type, as written or as it follows from `options` /
    /// `options_from`.
    pub fn kind(&self) -> ParamType {
        self.kind
            .unwrap_or(if self.options.is_empty() && self.options_from.is_none() {
                ParamType::Text
            } else {
                ParamType::Choice
            })
    }

    /// `value` as this param binds it, or why it doesn't take it. NULL is
    /// always taken (no value).
    fn take(&self, value: &SqlCell) -> Result<SqlCell, String> {
        let shown = || match value {
            SqlCell::Text(v) => format!("`{v}`"),
            other => format!("`{other:?}`"),
        };
        if matches!(value, SqlCell::Null(())) {
            return Ok(value.clone());
        }
        match self.kind() {
            ParamType::Text => Ok(value.clone()),
            // Options a lens lists aren't known here: the query says what
            // an unknown value means.
            ParamType::Choice if self.options_from.is_some() => match value {
                SqlCell::Text(_) => Ok(value.clone()),
                _ => Err(format!("is text, not {}", shown())),
            },
            ParamType::Choice => match value {
                SqlCell::Text(v) if self.options.iter().any(|o| &o.value == v) => Ok(value.clone()),
                _ => Err(format!(
                    "takes one of {}, not {}",
                    self.options
                        .iter()
                        .map(|o| o.value.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    shown()
                )),
            },
            ParamType::Bool => match value {
                SqlCell::Bool(b) => Ok(SqlCell::Int(i64::from(*b))),
                SqlCell::Int(n @ (0 | 1)) => Ok(SqlCell::Int(*n)),
                SqlCell::Text(t) if matches!(t.as_str(), "true" | "1") => Ok(SqlCell::Int(1)),
                SqlCell::Text(t) if matches!(t.as_str(), "false" | "0") => Ok(SqlCell::Int(0)),
                _ => Err(format!("is true or false, not {}", shown())),
            },
            ParamType::Search | ParamType::Branch => match value {
                SqlCell::Text(t) if t.trim().is_empty() => Ok(SqlCell::Null(())),
                SqlCell::Text(t) => Ok(SqlCell::Text(t.trim().to_string())),
                _ => Err(format!("is text, not {}", shown())),
            },
            ParamType::DateRange => match value {
                SqlCell::Text(t) if range_ends(t, time::OffsetDateTime::now_utc()).is_some() => {
                    Ok(value.clone())
                }
                _ => Err(format!(
                    "is a preset ({}) or `<from>..<to>`, not {}",
                    RANGE_PRESETS
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", "),
                    shown()
                )),
            },
        }
    }
}

/// A date_range value's two ends as RFC 3339 text (`None` for an open
/// end), or `None` when it isn't one: a preset back from `now`, or
/// `<from>..<to>` of dates or times, `to` not before `from`. A date's
/// `to` is the next day's start, so the day counts.
fn range_ends(value: &str, now: time::OffsetDateTime) -> Option<(Option<String>, Option<String>)> {
    use time::format_description::well_known::Rfc3339;
    if let Some((_, days)) = RANGE_PRESETS.iter().find(|(k, _)| *k == value) {
        let from = match days {
            Some(d) => Some((now - time::Duration::days(*d)).format(&Rfc3339).ok()?),
            None => None,
        };
        return Some((from, None));
    }
    let (from, to) = value.split_once("..")?;
    let date = time::macros::format_description!("[year]-[month]-[day]");
    // An end as given (its text, its instant), the day after for a `to` date.
    let end = |text: &str, last: bool| -> Option<Option<(String, time::OffsetDateTime)>> {
        let text = text.trim();
        if text.is_empty() {
            return Some(None);
        }
        if let Ok(at) = time::OffsetDateTime::parse(text, &Rfc3339) {
            return Some(Some((text.to_string(), at)));
        }
        let day = time::Date::parse(text, &date).ok()?;
        let day = if last { day.next_day()? } else { day };
        Some(Some((day.format(&date).ok()?, day.midnight().assume_utc())))
    };
    let (from, to) = (end(from, false)?, end(to, true)?);
    if let (Some((_, a)), Some((_, b))) = (&from, &to) {
        if b < a {
            return None;
        }
    }
    Some((from.map(|(t, _)| t), to.map(|(t, _)| t)))
}

/// A param whose type its other fields or its default don't fit: a
/// choice with no options or a default not among them, options or
/// presets on another type, an unknown preset.
fn param_problem(lens: &Lens) -> Option<String> {
    lens.params.iter().find_map(|p| {
        let kind = p.kind();
        // A document's view param is a choice of its sections, checked by
        // `view_problem`: its options aren't filled until mounts are placed.
        if lens.view.as_deref() == Some(p.name.as_str()) {
            return None;
        }
        let problem = if RESERVED_VIEW_KEYS.contains(&p.name.as_str()) {
            Some(format!(
                "`{0}` is the page's view key (`#{0}=`): rename the param",
                p.name
            ))
        } else if p.role == Some(ParamRole::Selection) {
            Some(
                "`role: selection` follows from the columns (a param a column `select`s is \
                 one); leave it out"
                    .to_string(),
            )
        } else if kind == ParamType::Choice && p.options.is_empty() && p.options_from.is_none() {
            Some("a choice needs `options` or `options_from`".to_string())
        } else if !p.options.is_empty() && p.options_from.is_some() {
            Some("a choice takes `options` or `options_from`, not both".to_string())
        } else if kind != ParamType::Choice && !p.options.is_empty() {
            Some("`options` are only for a choice".to_string())
        } else if kind != ParamType::Choice && p.options_from.is_some() {
            Some("`options_from` is only for a choice".to_string())
        } else if kind != ParamType::DateRange && !p.presets.is_empty() {
            Some("`presets` are only for a date_range".to_string())
        } else if let Some(bad) = p
            .presets
            .iter()
            .find(|k| !RANGE_PRESETS.iter().any(|(r, _)| r == k))
        {
            Some(format!("unknown preset `{bad}`"))
        } else if kind == ParamType::Choice && p.default.is_none() && p.options_from.is_none() {
            // A choice with fixed options starts somewhere; one whose
            // options a lens lists may start open.
            Some("a choice needs a `default`, one of its options".to_string())
        } else {
            p.default
                .as_ref()
                .and_then(|d| p.take(d).err())
                .map(|why| format!("its default {why}"))
        };
        problem.map(|why| format!("param `{}`: {why}", p.name))
    })
}

/// The keys of a page's `#` view that are the page's, never a lens
/// param's: `section`, the section a link lands on. (`comment` is the
/// page's only on a page that takes comments; the Comments lenses pick a
/// comment by that name.)
pub const RESERVED_VIEW_KEYS: &[&str] = &["section"];

/// What's wrong with a lens's `view`: it is a document's, names a declared
/// param that rides in the `#` (a `filter`) as a choice of the sections
/// (no `options` of its own), and starts on one of its own sections.
fn view_problem(lens: &Lens) -> Option<String> {
    let name = lens.view.as_deref()?;
    if lens.viz != LensViz::Document {
        return Some("`view` is for a document".into());
    }
    let Some(p) = lens.params.iter().find(|p| p.name == name) else {
        return Some(format!(
            "`view` names param `{name}`, which the lens doesn't declare"
        ));
    };
    if p.role != Some(ParamRole::Filter) {
        return Some(format!(
            "`view` param `{name}` rides in the `#`: leave `role` out or say `filter`"
        ));
    }
    if p.kind != Some(ParamType::Choice) {
        return Some(format!(
            "`view` param `{name}` is a choice of the sections: leave `type` out"
        ));
    }
    if !p.options.is_empty() || p.options_from.is_some() || !p.presets.is_empty() {
        return Some(format!(
            "`view` param `{name}`: the sections are its choices, not `options`"
        ));
    }
    let Some(default) = &p.default else {
        return Some(format!(
            "`view` param `{name}` needs a `default`: the section shown first"
        ));
    };
    let shown = match default {
        SqlCell::Text(t) => t.clone(),
        other => format!("{other:?}"),
    };
    if !lens
        .parts
        .iter()
        .any(|part| matches!(default, SqlCell::Text(t) if *t == part.id))
    {
        return Some(format!(
            "`view` param `{name}`'s default `{shown}` isn't a section"
        ));
    }
    None
}

/// `params` with a document's view param (`view: <name>`) a `filter`
/// choice when it doesn't say otherwise: its choices are the sections,
/// filled once mounts are placed ([`fill_view_options`]).
fn view_param(params: Vec<LensParam>, view: Option<&str>) -> Vec<LensParam> {
    params
        .into_iter()
        .map(|p| {
            if Some(p.name.as_str()) != view {
                return p;
            }
            LensParam {
                kind: Some(p.kind.unwrap_or(ParamType::Choice)),
                role: Some(p.role.unwrap_or(ParamRole::Filter)),
                ..p
            }
        })
        .collect()
}

/// `params` with each one's type written out, so a page never infers it.
fn typed(params: Vec<LensParam>) -> Vec<LensParam> {
    params
        .into_iter()
        .map(|p| LensParam {
            kind: Some(p.kind()),
            ..p
        })
        .collect()
}

/// What a lens's query binds: its params, and each date_range's
/// `<name>_from` / `<name>_to`.
fn binds(
    lens: &Lens,
    params: &BTreeMap<String, SqlCell>,
    now: time::OffsetDateTime,
) -> Vec<(String, SqlCell)> {
    let mut out: Vec<(String, SqlCell)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let text = |t: Option<String>| t.map_or(SqlCell::Null(()), SqlCell::Text);
    for p in lens
        .params
        .iter()
        .filter(|p| p.kind() == ParamType::DateRange)
    {
        let (from, to) = match params.get(&p.name) {
            Some(SqlCell::Text(v)) => range_ends(v, now).unwrap_or((None, None)),
            _ => (None, None),
        };
        out.push((format!("{}_from", p.name), text(from)));
        out.push((format!("{}_to", p.name), text(to)));
    }
    out
}

/// A lens file as written on disk (`lenses/<slug>.yaml`).
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
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
    /// `dashboard` only: its rows are its tiles.
    #[serde(default)]
    tile: Option<LensTile>,
    #[serde(default)]
    timeline: Option<LensTimeline>,
    #[serde(default)]
    steps: Option<LensSteps>,
    #[serde(default)]
    hunks: Option<LensHunks>,
    #[serde(default)]
    code: Option<LensCode>,
    #[serde(default)]
    diff: Option<LensDiff>,
    #[serde(default)]
    markdown: Option<LensMarkdown>,
    #[serde(default)]
    form: Option<LensForm>,
    #[serde(default)]
    custom: Option<LensCustom>,
    #[serde(default)]
    group: Option<LensGroup>,
    #[serde(default)]
    emphasis: Option<String>,
    #[serde(default)]
    depth: Option<String>,
    /// For `document`: its sections, each a lens (a slug in this
    /// extension, or `<ext>/<slug>`) or markdown.
    #[serde(default)]
    sections: Vec<PartFile>,
    /// For `document`: the param whose value names the one section in
    /// view (a filter whose choices are the sections); the others are the
    /// page's rail.
    #[serde(default)]
    view: Option<String>,
    /// For `dashboard`: its tiles.
    #[serde(default)]
    tiles: Vec<PartFile>,
    #[serde(default)]
    launcher: Option<LauncherFile>,
    /// Keep it out of the launcher (e.g. a lens only a slot shows).
    #[serde(default)]
    hidden: bool,
    /// Commands the lens offers (`{ id, label, command, input, row }`).
    /// Parsed by [`parse_actions`].
    #[serde(default)]
    #[schemars(with = "Vec<ActionFile>")]
    actions: Vec<serde_yaml::Value>,
    #[serde(default)]
    alert: Option<AlertFile>,
    /// Capabilities or features it needs (`work_items`,
    /// `snapshots.contents`).
    #[serde(default)]
    needs: Vec<String>,
    /// Offer it to agents as an MCP tool too, named `<namespace>_<tool>`;
    /// its params are the tool's input.
    #[serde(default)]
    tool: Option<String>,
}

/// A button on a lens: a command it runs (P6.B1, target §11.4). The
/// command goes through the bus as `Actor::Lens` acting for whoever
/// pressed it, so every policy — the agent policy, confirmation
/// — applies as if they had run it themselves; a lens can offer a button
/// but never grant a power. Copying the result and handing it to the
/// agent are on every lens, not declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensAction {
    /// Unique within the lens; `oxplow_run_lens_action` names it.
    pub id: String,
    pub label: String,
    /// The command it runs (`oxplow.work_item.transition`).
    pub command: String,
    /// The command's input. A string that is exactly `{{param.<name>}}`
    /// or `{{row.<column>}}` becomes that value (a number stays a number);
    /// one that contains them has them spliced in as text.
    #[specta(type = oxplow_domain::Json)]
    pub input: serde_json::Value,
    /// A row action: offered on each row (right-click), with `{{row.*}}`
    /// bound to that row. Otherwise it's a button above the result.
    pub row: bool,
    /// A group action: a button in the heading of the group whose `by`
    /// value (as text) this is, instead of above the result.
    pub group: Option<String>,
    /// Opens the command's form, its fields starting from `input`, for the
    /// person to fill in before it runs (`lens_action_form`), instead of
    /// running at once.
    pub form: bool,
    /// Fields of a form action the person sees but can't change: their
    /// values are `input`'s, imposed again when it runs (an item's `ref`).
    pub fixed: Vec<String>,
    /// A form action's fields picked from a list: each names a lens
    /// (`<extension>/<slug>`) run with the lens's params, whose rows are
    /// the choices (`value`, `label`, optional `group`).
    pub choices: BTreeMap<String, String>,
    /// What becomes of the lens's page when the command's `ui.open_after`
    /// names a page.
    pub then: ActionThen,
    /// A row action offered only on rows whose column of this name is
    /// truthy (not null, 0, `""` or false): an Enable on rows that are off.
    pub when: Option<String>,
    /// The row action a click on the row (not on a link or control in it),
    /// or Enter on it, runs: the one a person most often wants. The first
    /// `primary` action a row is offered (`when`) is the one.
    #[serde(default)]
    pub primary: bool,
    /// What a drop runs: the dragged row is `{{row.*}}` (a drag action is
    /// a row action), and where it landed binds `{{before.*}}` (the row
    /// below the drop), `{{after.*}}` (the row above), `{{onto.*}}` (the
    /// row dropped on) and `{{group.value}}` (the group dropped in). A
    /// field that is exactly one of these with nothing there is left out.
    #[serde(default)]
    pub drag: bool,
    /// A row action offered for every marked row at once: it runs once per
    /// row, in list order, asking once.
    #[serde(default)]
    pub bulk: bool,
    /// A row action's key: pressed with the rows focused, it runs on the
    /// cursor row (or the marked rows, when `bulk`).
    #[serde(default)]
    pub key: Option<String>,
    /// The Save of a `markdown` viz's editor: a form over the first row,
    /// the person's text laid over `edit_field`.
    #[serde(default)]
    pub edit: bool,
    /// An edit action's field the editor's text fills: the one input field
    /// bound exactly to a `{{row.<column>}}` and not `fixed`.
    #[serde(default)]
    pub edit_field: Option<String>,
    /// A whole-lens action over a lens that holds one row, that names
    /// `when` or `{{row.*}}`: the lens's one row. Derived at load, not
    /// declared: an action without `row: true` that names `when:` or
    /// `{{row.*}}`. It runs on the row the server reads from the lens
    /// itself, and only while the lens has exactly one.
    #[serde(default)]
    pub one_row: bool,
}

/// The `{{param.x}}` / `{{row.x}}` placeholders in `input`, as
/// `(scope, name)`, in order.
pub fn action_templates(input: &serde_json::Value) -> Vec<(String, String)> {
    use oxplow_domain::template;
    template::strings(input)
        .into_iter()
        .flat_map(template::placeholders)
        .map(|p| (p.scope, p.name))
        .collect()
}

/// A lens action as written in a lens file.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActionFile {
    /// Unique within the lens.
    id: String,
    label: String,
    /// The command it runs (`oxplow.work_item.transition`).
    command: String,
    /// The command's input; `{{param.<name>}}` / `{{row.<column>}}` bound.
    #[serde(default)]
    input: Option<serde_json::Value>,
    /// Offered on each row (right-click), `{{row.*}}` bound to it.
    #[serde(default)]
    row: bool,
    /// The `group` heading (its `by` value) it's a button in.
    #[serde(default)]
    group: Option<String>,
    /// Open the command's form, starting from `input`, instead of running.
    #[serde(default)]
    form: bool,
    /// A form action's fields that keep `input`'s values (shown, not
    /// editable).
    #[serde(default)]
    fixed: Vec<String>,
    /// A form action's fields picked from a list: field → the lens whose
    /// rows are its choices.
    #[serde(default)]
    choices: BTreeMap<String, String>,
    /// What becomes of the lens's page when the command opens one:
    /// `open` (default), `replace`, `stay` or `close`.
    #[serde(default)]
    then: ActionThen,
    /// A row action's column: offered only on rows where it is truthy.
    #[serde(default)]
    when: Option<String>,
    /// The row action a click on the row runs.
    #[serde(default)]
    primary: bool,
    /// A row action a drop runs, binding where the row landed.
    #[serde(default)]
    drag: bool,
    /// Offered for every marked row at once.
    #[serde(default)]
    bulk: bool,
    /// The key that runs it on the cursor (or marked) rows.
    #[serde(default)]
    key: Option<String>,
    /// The Save of a `markdown` viz's editor: the text fills the field
    /// bound to `{{row.<the text column>}}`.
    #[serde(default)]
    edit: bool,
}

/// The keys a row list keeps for itself: an action can't take one.
const KIT_KEYS: &[&str] = &[
    "Enter",
    "Escape",
    " ",
    "Space",
    "Tab",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ContextMenu",
    "F10",
];

/// The named keys an action may take besides one printable character.
const NAMED_KEYS: &[&str] = &["Delete", "Backspace"];

/// What's wrong with `key` as a row action's key, if anything.
fn key_problem(key: &str) -> Option<String> {
    if KIT_KEYS.contains(&key) {
        return Some(format!("`{key}` is the kit's (moving, picking, the menu)"));
    }
    let one_char = key.chars().count() == 1 && !key.chars().all(char::is_whitespace);
    (!one_char && !NAMED_KEYS.contains(&key)).then(|| {
        format!(
            "`{key}` isn't one key: give one character or one of {}",
            NAMED_KEYS.join(", ")
        )
    })
}

/// Parse a lens's `actions:`: each `{ id, label, command, input?, row? }`.
/// A placeholder must name a declared param (`{{param.x}}`) or, in a row
/// action, a column (`{{row.x}}`, checked against the result when the
/// extension is validated). A lens that `holds_one_row` (detail, form,
/// markdown, a document's row) lets a whole-lens action name `when` and
/// `{{row.*}}` too: that row.
fn parse_actions(
    raw: Vec<serde_yaml::Value>,
    params: &[LensParam],
    grouped: bool,
    holds_one_row: bool,
) -> Result<Vec<LensAction>, String> {
    let mut out: Vec<LensAction> = Vec::new();
    for v in raw {
        let f = serde_yaml::from_value::<ActionFile>(v).map_err(|e| format!("actions: {e}"))?;
        if f.id.trim().is_empty() {
            return Err("actions: an action needs an `id`".into());
        }
        if out.iter().any(|a| a.id == f.id) {
            return Err(format!("actions: `{}` is declared twice", f.id));
        }
        oxplow_domain::CommandSpec::validate_id(&f.command)
            .map_err(|e| format!("actions: `{}`: {e}", f.id))?;
        if let Some(g) = &f.group {
            if !grouped {
                return Err(format!(
                    "actions: `{}`: `group: {g}` needs the lens to have a `group`",
                    f.id
                ));
            }
            if f.row {
                return Err(format!(
                    "actions: `{}`: a group action isn't a row action (drop `row` or `group`)",
                    f.id
                ));
            }
        }
        let input = f.input.unwrap_or_else(|| serde_json::json!({}));
        if !input.is_object() {
            return Err(format!(
                "actions: `{}`: `input` must be a map (the command's input)",
                f.id
            ));
        }
        if !f.fixed.is_empty() && !(f.form || f.edit) {
            return Err(format!(
                "actions: `{}`: `fixed` is for a form action (`form: true`)",
                f.id
            ));
        }
        if f.when.is_some() && !(f.row || f.drag || holds_one_row) {
            return Err(format!(
                "actions: `{}`: `when` is for a row action (`row: true`), or a lens that holds one row (detail, form, markdown, a document's row)",
                f.id
            ));
        }
        // A drag action is a row action: its row is the dragged one. An
        // edit action is a form over the viz's row.
        let row = f.row || f.drag || f.edit;
        let form = f.form || f.edit;
        // Derived: a whole-lens action that names `when` or the row.
        let one_row = !row
            && holds_one_row
            && (f.when.is_some()
                || action_templates(&input)
                    .iter()
                    .any(|(scope, _)| scope == "row"));
        if f.edit && (f.drag || f.bulk || f.key.is_some() || f.group.is_some() || f.primary) {
            return Err(format!(
                "actions: `{}`: an edit action is the editor's Save: it can't be `drag`, `bulk`, `key`, `group` or `primary`",
                f.id
            ));
        }
        if f.edit {
            if let Some(other) = out.iter().find(|a| a.edit) {
                return Err(format!(
                    "actions: `{}`: the lens already saves with `{}`",
                    f.id, other.id
                ));
            }
        }
        if f.drag && f.form {
            return Err(format!(
                "actions: `{}`: a drag action can't be a form",
                f.id
            ));
        }
        if f.drag && f.primary {
            return Err(format!(
                "actions: `{}`: a drag action can't be `primary` (a click doesn't drop)",
                f.id
            ));
        }
        if f.drag && f.group.is_some() {
            return Err(format!(
                "actions: `{}`: a drag action binds `{{{{group.value}}}}`, it isn't a heading's button",
                f.id
            ));
        }
        if f.primary && !row {
            return Err(format!(
                "actions: `{}`: `primary` is for a row action (`row: true`)",
                f.id
            ));
        }
        if f.bulk && !row {
            return Err(format!(
                "actions: `{}`: `bulk` is for a row action (`row: true`)",
                f.id
            ));
        }
        if let Some(key) = &f.key {
            if !row {
                return Err(format!(
                    "actions: `{}`: `key` is for a row action (`row: true`)",
                    f.id
                ));
            }
            if let Some(why) = key_problem(key) {
                return Err(format!("actions: `{}`: key {why}", f.id));
            }
            if let Some(other) = out.iter().find(|a| a.key.as_deref() == Some(key)) {
                return Err(format!(
                    "actions: `{}`: key `{key}` is already `{}`'s",
                    f.id, other.id
                ));
            }
        }
        if !f.choices.is_empty() && !(f.form || f.edit) {
            return Err(format!(
                "actions: `{}`: `choices` is for a form action (`form: true`)",
                f.id
            ));
        }
        if let Some(missing) = f.fixed.iter().find(|k| input.get(k.as_str()).is_none()) {
            return Err(format!(
                "actions: `{}`: fixed field `{missing}` needs a value in `input`",
                f.id
            ));
        }
        for s in oxplow_domain::template::strings(&input) {
            if let Some(filter) = oxplow_domain::template::placeholders(s)
                .into_iter()
                .find_map(|p| p.filter.filter(|f| f != "json"))
            {
                return Err(format!(
                    "actions: `{}`: unknown placeholder filter `{filter}` (the only one is `json`)",
                    f.id
                ));
            }
        }
        for (scope, name) in action_templates(&input) {
            match scope.as_str() {
                "param"
                    if params.iter().any(|p| {
                        p.name == name || name.strip_suffix(".id") == Some(p.name.as_str())
                    }) => {}
                "param" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{param.{name}}}}}` names no param of this lens",
                        f.id
                    ))
                }
                "row" if row || one_row => {}
                "before" | "after" | "onto" | "order" if f.drag => {}
                "before" | "after" | "onto" | "order" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{{scope}.{name}}}}}` needs `drag: true` (it is where a drop landed)",
                        f.id
                    ))
                }
                "group" if name != "value" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{group.{name}}}}}` isn't `{{{{group.value}}}}`",
                        f.id
                    ))
                }
                "group" if !grouped => {
                    return Err(format!(
                        "actions: `{}`: `{{{{group.value}}}}` needs the lens to have a `group`",
                        f.id
                    ))
                }
                "group" if f.drag => {}
                "group" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{group.value}}}}` needs `drag: true` (it is the group a drop landed in)",
                        f.id
                    ))
                }
                "row" => {
                    return Err(format!(
                        "actions: `{}`: `{{{{row.{name}}}}}` needs `row: true`, or a lens that holds one row (detail, form, markdown, a document's row)",
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
        let edit_field = if f.edit {
            Some(edit_field(&f.id, &input, &f.fixed)?)
        } else {
            None
        };
        out.push(LensAction {
            id: f.id,
            label: f.label,
            command: f.command,
            input,
            row,
            group: f.group,
            form,
            fixed: f.fixed,
            choices: f.choices,
            then: f.then,
            when: f.when,
            primary: f.primary,
            drag: f.drag,
            bulk: f.bulk,
            key: f.key,
            edit: f.edit,
            edit_field,
            one_row,
        });
    }
    Ok(out)
}

/// Whether a lens of this `viz` holds one row, so a whole-lens action can
/// name it: a detail, a form, a markdown, or a document with a `query:`.
fn holds_one_row(viz: LensViz, has_query: bool) -> bool {
    matches!(viz, LensViz::Detail | LensViz::Form | LensViz::Markdown)
        || (viz == LensViz::Document && has_query)
}

/// An edit action's edited field: the one input field that is exactly a
/// `{{row.<column>}}` and isn't `fixed`.
fn edit_field(id: &str, input: &serde_json::Value, fixed: &[String]) -> Result<String, String> {
    let fields: Vec<&String> = input
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, v)| {
            !fixed.contains(k)
                && v.as_str()
                    .and_then(oxplow_domain::template::whole_placeholder)
                    .is_some_and(|p| p.scope == "row")
        })
        .map(|(k, _)| k)
        .collect();
    match fields.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "actions: `{id}` edits nothing: bind one input field to `{{{{row.<the text column>}}}}`"
        )),
        many => Err(format!(
            "actions: `{id}` edits {}: fix all but one",
            many.iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    }
}

/// The column a markdown editor saves: what its edit action's edited
/// field is bound to (`{{row.<column>}}`).
pub fn edit_column(action: &LensAction) -> Option<String> {
    let field = action.edit_field.as_deref()?;
    let value = action.input.get(field)?.as_str()?;
    oxplow_domain::template::whole_placeholder(value).map(|p| p.name)
}

fn default_viz() -> LensViz {
    LensViz::Table
}

/// When core runs an advisory.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOn {
    /// After each agent tool call; results go to the agent as that call's
    /// context and are recorded as nudges.
    PostToolUse,
    /// On each prompt the human sends; results join the prompt's context.
    Prompt,
    /// When a turn ends (its `thread.checkpoint`): what the turn left
    /// behind, delivered when the agent next hears from oxplow (the next
    /// prompt, else the next tool call).
    TurnEnd,
}

/// How often the same advisory may reach the agent.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOncePer {
    /// Once per effort, the first time the query returns rows.
    Effort,
    /// Once per thread per `key` value: each row's `key` column fires
    /// once (put `:effort_id` in the key for once per effort per row).
    Row,
    /// Every time, whenever the query returns rows.
    Turn,
    /// Once per thread, the first time the query returns rows.
    Thread,
    /// Once per agent session on the thread (the harness's session id).
    Session,
    /// Once per thread per day (UTC).
    Day,
}

/// Who a hint is for.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryAudience {
    /// The coding agent: delivered on its next prompt or tool call.
    #[default]
    Agent,
    /// A person: raised in Alerts until they dismiss it.
    Person,
}

fn default_once_per() -> AdvisoryOncePer {
    AdvisoryOncePer::Effort
}

/// Guidance an extension gives the coding agent: a query over the semantic
/// layer, run by core at `on`, with `:thread_id`, `:stream_id`, `:turn_id`
/// and `:effort_id` bound (`:effort_id` NULL when the thread has no
/// effort, `:turn_id` NULL at a prompt). Each result row's `message`
/// column is a line of guidance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Advisory {
    pub id: String,
    pub on: AdvisoryOn,
    pub query: String,
    pub once_per: AdvisoryOncePer,
    /// Line put above the messages (e.g. `# Metric deltas (this effort)`).
    pub heading: Option<String>,
    pub audience: AdvisoryAudience,
    /// The capabilities (or features) it needs; without them it doesn't
    /// run.
    pub needs: Vec<String>,
}

/// An advisory as written in `extension.yaml`.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AdvisoryFile {
    id: String,
    on: AdvisoryOn,
    query: String,
    #[serde(default = "default_once_per")]
    once_per: AdvisoryOncePer,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    audience: AdvisoryAudience,
    #[serde(default)]
    needs: Vec<String>,
}

/// Places in core pages an extension can mount a lens, and the params each
/// offers (`change_id` = the page's `v_change` row). A mounted lens gets
/// the ones it declares and must declare at least one.
/// Slot names are one dotted namespace, `<capability>.<page>.<region>`
/// (P6b): what a page shows a lens is named by what the page is about.
/// A page's own parts are slots too (`<page>.sections`, `<page>.tiles`):
/// a mount there goes `at:` the start, the end, or beside one of the
/// page's parts, named by the stable ids listed here.
pub const SLOTS: &[SlotSpec] = &[
    // Uncommitted changes: a strip above them, and below the files.
    // A file diff's header strip, above the editor: the file and the two
    // revisions the diff reads (`working`, `snap:<id>`, `git:<sha>`).
    SlotSpec::region(
        "diff.file.header",
        &["path", "left_revision", "right_revision", "stream_id"],
    ),
    // Settings: each area's page and its sections (`pages/settingsAreas.ts`
    // lists the same ids). An extension's own status or configuration
    // views go beside the area they belong to. No params.
    SlotSpec::sections("settings.overview.sections", &["q"], &[]),
    SlotSpec::sections("settings.agents.sections", &[], &[]),
    SlotSpec::sections("settings.lsp.sections", &[], &[]),
    SlotSpec::sections("settings.extensions.sections", &[], &[]),
    SlotSpec::sections("settings.data.sections", &[], &[]),
    SlotSpec::sections("settings.integrations.sections", &[], &["integrations"]),
    SlotSpec::sections("settings.ai.sections", &[], &[]),
];

/// A place in a core page a lens can be mounted ([`SLOTS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSpec {
    pub name: &'static str,
    /// The params the page binds; a mounted lens takes the ones it declares.
    pub params: &'static [&'static str],
    pub parts: SlotParts,
}

impl SlotSpec {
    const fn region(name: &'static str, params: &'static [&'static str]) -> Self {
        Self {
            name,
            params,
            parts: SlotParts::Region,
        }
    }

    const fn sections(
        name: &'static str,
        params: &'static [&'static str],
        ids: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            params,
            parts: SlotParts::Sections(ids),
        }
    }

    /// The core slot named `name`.
    pub fn core(name: &str) -> Option<&'static SlotSpec> {
        SLOTS.iter().find(|s| s.name == name)
    }
}

/// What a slot holds: a fixed region of the page, or the page's own list
/// of parts (a document's sections, a dashboard's tiles), among which a
/// mount is placed by `at:` relative to the parts' stable ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotParts {
    Region,
    Sections(&'static [&'static str]),
    Tiles(&'static [&'static str]),
}

/// Where a mount goes among a page's parts: `start`, `end` (the default),
/// or `{ after: <id> }` / `{ before: <id> }` beside one of them. An id the
/// page no longer has means the end (and a load warning), so a page that
/// changes never loses what an extension adds.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(untagged)]
pub enum SlotAt {
    Edge(SlotEdge),
    After { after: String },
    Before { before: String },
}

/// The ends of a page's parts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SlotEdge {
    Start,
    End,
}

impl Default for SlotAt {
    fn default() -> Self {
        SlotAt::Edge(SlotEdge::End)
    }
}

impl SlotAt {
    /// The part it's placed beside, if any.
    pub fn anchor(&self) -> Option<&str> {
        match self {
            SlotAt::After { after: id } | SlotAt::Before { before: id } => Some(id),
            SlotAt::Edge(_) => None,
        }
    }
}

/// How much of a dashboard a tile takes: one cell, two across, two down,
/// or the whole row.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum TileSize {
    #[default]
    Small,
    Wide,
    Tall,
    Full,
}

/// What a slot holds, as a mount is checked against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartsKind {
    Region,
    Sections,
    Tiles,
}

/// A slot naming a lens's parts — `<extension>/<slug>.sections` (a
/// document's) or `.tiles` (a dashboard's): the lens id and which.
fn lens_slot_target(slot: &str) -> Option<(&str, PartsKind)> {
    let (lens, which) = slot.rsplit_once('.')?;
    if !lens.contains('/') {
        return None;
    }
    match which {
        "sections" => Some((lens, PartsKind::Sections)),
        "tiles" => Some((lens, PartsKind::Tiles)),
        _ => None,
    }
}

/// What's wrong with mounting `lens` in a slot that binds `params` and
/// holds `kind` of parts with stable `ids`: an error, or a warning (an
/// `at` beside a part the page doesn't have goes at the end).
fn mount_problem(
    mount: &LensSlot,
    lens: &Lens,
    params: &[&str],
    row: &[&str],
    kind: PartsKind,
    ids: &[&str],
) -> Result<Option<String>, String> {
    // A slot offers params (and a page's query row its names); a mounted
    // lens takes the ones it declares and must declare at least one, or it
    // can't relate to the page.
    let offered: Vec<&str> = params.iter().chain(row).copied().collect();
    if !offered.is_empty()
        && !offered
            .iter()
            .any(|p| lens.params.iter().any(|lp| lp.name == *p))
    {
        let list = |names: &[&str]| {
            names
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let row_note = if row.is_empty() {
            String::new()
        } else {
            format!(" and the page's row gives {}", list(row))
        };
        return Err(format!(
            "slot `{}` passes {}{row_note}; lens `{}` must declare at least one in `params`",
            mount.slot,
            list(params),
            lens.slug
        ));
    }
    if kind == PartsKind::Region && mount.at.is_some() {
        return Err(format!(
            "slot `{}` is one region of its page; `at` places a mount among a page's \
             sections or tiles",
            mount.slot
        ));
    }
    if kind != PartsKind::Tiles && mount.size.is_some() {
        return Err(format!(
            "slot `{}` holds no tiles; `size` is a dashboard tile's",
            mount.slot
        ));
    }
    Ok(mount
        .at
        .as_ref()
        .and_then(SlotAt::anchor)
        .filter(|id| !ids.contains(id))
        .map(|id| {
            format!(
                "slot `{}` has no part `{id}` (it has {}); lens `{}` goes at the end",
                mount.slot,
                if ids.is_empty() {
                    "none".to_string()
                } else {
                    ids.iter()
                        .map(|i| format!("`{i}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                lens.slug
            )
        }))
}

/// What a left-nav panel is bound to: the project, the current stream, or
/// the current thread — which of `stream_id` / `thread_id` its lenses get.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
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
    /// The page its header opens (`oxplow:oxplow/page?page=<kind>`, a canonical ref): a core
    /// page the panel summarizes, as Comments opens the inbox.
    /// Absent, the body lens's own page.
    pub open: Option<String>,
    /// The lens shown, compact, while the panel is collapsed: its one-line
    /// summary. Absent, a collapsed panel shows only its header.
    pub collapsed: Option<String>,
    /// The lens whose row count (a `number` lens: its value) is the
    /// header's count, without raising an alert. It wins over
    /// the badge's count; the badge still feeds Alerts.
    pub count: Option<String>,
}

/// A page an extension contributes (P6.G2, target §11.3): a lens shown
/// full-page at `oxplow:oxplow/page?page=<extension>/<id>`, listed in the launcher under
/// its category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPage {
    pub id: String,
    pub extension: String,
    /// Its tab id: `oxplow:oxplow/page?page=<id>` for the oxplow-namespace extension (its
    /// pages are core-named, as its commands are), `oxplow:oxplow/page?page=<extension>/<id>`
    /// for any other.
    pub page_ref: String,
    /// Its name: the launcher's, Open with's, a kind label's.
    pub title: String,
    /// What its tab says, when not its title: `{{param.<name>}}` placeholders
    /// over the params of what it shows (`label: "{{param.path}}"`).
    pub label: Option<String>,
    pub icon: Option<String>,
    /// Its launcher section; none = not in the launcher (a page only
    /// opened from elsewhere: a diff, a form).
    pub category: Option<LauncherCategory>,
    /// More words the launcher matches it on.
    pub keywords: Option<String>,
    /// The params naming what it shows, in its URI's `?`
    /// (`oxplow:oxplow/page?page=settings&stream=str1`); its lens's filters and
    /// selections ride in the `#` view.
    pub params: Vec<String>,
    /// The lens whose value (a `number` lens) is the launcher's badge
    /// (`<extension>/<slug>`).
    pub count: Option<String>,
    /// The resources it renders (`oxplow/file`, `acme/pr`; `https`, `http`
    /// for a web page): a URI of one opens on it, the URI its tab's id.
    /// Empty: a route, opened at `page_ref`.
    #[serde(rename = "for")]
    pub renders: Vec<String>,
    /// Which URIs of those it takes, by param: `present`, `absent`, or an
    /// anchored pattern the value matches (one form the desktop's JS and
    /// Rust read alike). Empty: every one.
    pub accepts: BTreeMap<String, String>,
    /// What its resource opens on when the URI's view names no renderer
    /// (`#oxplow.renderer=`): only a page of the extension declaring it.
    pub default: bool,
    /// Its tab leads the strip and isn't evicted (an agent session's).
    pub pinned: bool,
    /// Its text takes comments: its URI (its tab id) is their target, the
    /// nav bar steps through them, and a selection in its text (a markdown
    /// viz, a code viz, plain text) starts one.
    pub comments: bool,
    /// What draws it.
    pub body: PageBody,
}

/// The schemes a page may render besides oxplow's resources: a web page.
pub const RENDERED_SCHEMES: &[&str] = &["https", "http"];

/// The namespace oxplow's own extension declares (`namespace: oxplow`): its
/// commands are `oxplow.<area>.<verb>` and its pages `oxplow:oxplow/page?page=<id>`.
pub const CORE_NAMESPACE: &str = "oxplow";

/// What draws a page: a lens core renders through the kit, or — for the
/// required bundled extension only, and shrinking — a built-in component
/// core still renders (`built_in_pages::BUILT_IN_PAGES`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageBody {
    Lens {
        /// `<extension>/<slug>`.
        lens: String,
    },
    BuiltIn {
        /// A `BUILT_IN_PAGES` name.
        name: String,
    },
    /// One of the extension's own custom components, filling the page
    /// body: a custom page (`.context/extensions.md` "Pages").
    Component {
        /// `<extension>/<component id>`.
        component: String,
    },
}

impl ExtensionPage {
    /// The lens it shows, when it is a lens page.
    pub fn lens(&self) -> Option<&str> {
        match &self.body {
            PageBody::Lens { lens } => Some(lens),
            _ => None,
        }
    }
}

/// A `pages:` entry as the manifest holds it.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageFile {
    id: String,
    title: String,
    /// What its tab says, when not its `title`: `{{param.<name>}}` filled
    /// from what it shows (a renderer's URI, a route's `params`).
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    /// Its launcher section; leave it out for a page only opened from
    /// elsewhere.
    #[serde(default)]
    category: Option<LauncherCategory>,
    #[serde(default)]
    keywords: Option<String>,
    /// The params naming what it shows, in its URI's `?`; its lens's filters
    /// and selections ride in the `#` view.
    #[serde(default)]
    params: Vec<String>,
    /// A `number` lens of this extension whose value is the launcher badge.
    #[serde(default)]
    count: Option<String>,
    /// A lens of this extension that draws the page …
    #[serde(default)]
    lens: Option<String>,
    /// … or `oxplow:<name>`, a built-in (oxplow's own extension only).
    #[serde(default)]
    component: Option<String>,
    /// The resources it renders — one or a list: `oxplow/file`, its own
    /// `<extension>/<name>`, another extension's, or `https` / `http` (a
    /// web page). A URI of one opens on it; without `for` it is a route.
    #[serde(default, rename = "for")]
    renders: Option<Renders>,
    /// Which URIs of those it takes, by param: `present`, `absent`, or an
    /// anchored pattern (`^git:`) the value matches.
    #[serde(default)]
    accepts: BTreeMap<String, String>,
    /// What its resource opens on when the URI's view names no renderer:
    /// only for a resource this extension declares, one per resource.
    #[serde(default)]
    default: bool,
    /// Its tab leads the strip and isn't evicted (a renderer only).
    #[serde(default)]
    pinned: bool,
    /// Its text takes comments, anchored to its URI.
    #[serde(default)]
    comments: bool,
}

/// A page's `for`: one resource or several.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum Renders {
    One(String),
    Many(Vec<String>),
}

impl Renders {
    fn into_vec(self) -> Vec<String> {
        match self {
            Renders::One(r) => vec![r],
            Renders::Many(rs) => rs,
        }
    }
}

/// A `panels:` entry as the manifest holds it.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
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
    #[serde(default)]
    open: Option<String>,
    #[serde(default)]
    collapsed: Option<String>,
    #[serde(default)]
    count: Option<String>,
}

/// A lens an extension mounts into a core page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensSlot {
    /// Which page region, from [`SLOTS`] (`diff.file.header`,
    /// `vcs.commit.details`, …), or a document or dashboard lens's parts
    /// (`<extension>/<slug>.sections`, `.tiles`); the lens takes the params
    /// it offers.
    pub slot: String,
    pub lens_id: String,
    /// Where it goes among the page's parts; `None` in a region.
    pub at: Option<SlotAt>,
    /// Its heading; else its lens's title.
    pub title: Option<String>,
    /// A tile's size; `None` outside a dashboard.
    pub size: Option<TileSize>,
}

impl LensSlot {
    /// Its contribution's local id, as the `n`th mount of its extension:
    /// `<slot>-<n>`, a lens's `/` written `.` (`base.page.sections-0`).
    pub fn local_id(&self, n: usize) -> String {
        format!("{}-{n}", self.slot.replace('/', "."))
    }
}

/// Where a lens comes from. An agent edits `source`, never `running`: a
/// deployed extension runs a copy in the store that the next deploy
/// replaces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensProvenance {
    /// The extension it belongs to.
    pub extension: String,
    /// Its source file, relative to the project when inside it; none when
    /// no source is here (an extension that ships with oxplow, or a
    /// deployment whose source is gone).
    pub source: Option<String>,
    /// The file it runs from: `source` when loaded from it, else the
    /// deployment's copy.
    pub running: String,
    /// The deployment it runs, when it runs one (its hash).
    pub deployment: Option<String>,
    /// It ships with oxplow.
    pub bundled: bool,
}

/// A loaded lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Lens {
    /// `<extension>/<slug>`: the stable id used by `lens:` page refs,
    /// `oxplow_run_lens` and `get_lens`.
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
    /// `dashboard` only: its query's rows are its tiles.
    pub tile: Option<LensTile>,
    pub timeline: Option<LensTimeline>,
    pub steps: Option<LensSteps>,
    pub hunks: Option<LensHunks>,
    pub code: Option<LensCode>,
    pub diff: Option<LensDiff>,
    pub markdown: Option<LensMarkdown>,
    pub form: Option<LensForm>,
    /// For `custom`: the component and its props.
    pub custom: Option<LensCustom>,
    /// For `list` / `table`: rows under a heading per value.
    pub group: Option<LensGroup>,
    /// For `list` / `table` / `tree`: a column whose truthy value
    /// highlights the row.
    pub emphasis: Option<String>,
    /// For `list` / `table`: a column whose integer value indents the row.
    pub depth: Option<String>,
    /// For `document` its sections, for `dashboard` its tiles: its own,
    /// then what other extensions mount, placed by their `at`.
    pub parts: Vec<LensPart>,
    /// For `document`: the param whose value names the one section in
    /// view. It is a `filter` choice whose options are the sections (its
    /// own and mounted ones, filled once mounts are placed).
    pub view: Option<String>,
    /// Launcher section; `None` = "Lenses".
    pub launcher_category: Option<LauncherCategory>,
    /// Not listed in the launcher.
    pub hidden: bool,
    /// Commands the lens offers, as buttons or row actions (P6.B1).
    pub actions: Vec<LensAction>,
    /// When the lens needs attention (its panel's badge).
    pub alert: Option<LensAlert>,
    /// The capabilities (or their features) it needs (`needs:
    /// [work_items]`): without them its run says so instead of running.
    pub needs: Vec<String>,
    /// Where it comes from: the file that runs and the source to edit.
    pub origin: LensProvenance,
    /// The MCP tool it is also offered as (`tool:`), prefixed with its
    /// extension's namespace (`extension_commands::tool_name`): a call
    /// runs it and returns its text rendering.
    pub tool: Option<String>,
}

impl Lens {
    /// The lenses its parts show (a document's sections, a dashboard's
    /// tiles).
    pub fn part_lenses(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().filter_map(|p| p.lens.as_deref())
    }

    /// The parts a run with `params` shows: a view document's one section
    /// in view (its view param's value, else its default), every part of
    /// any other lens. What its text resolves and renders, so the two stay
    /// in step; the desktop's `viewedParts` is the same rule.
    pub fn viewed_parts(&self, params: &BTreeMap<String, SqlCell>) -> Vec<&LensPart> {
        let Some(view) = &self.view else {
            return self.parts.iter().collect();
        };
        let default = self
            .params
            .iter()
            .find(|p| &p.name == view)
            .and_then(|p| p.default.clone());
        let id = match params.get(view).cloned().or(default) {
            Some(SqlCell::Text(t)) => t,
            _ => return Vec::new(),
        };
        self.parts.iter().filter(|p| p.id == id).collect()
    }
}

/// A loaded extension and anything wrong with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub name: String,
    /// The namespace its commands' ids are under: the manifest's
    /// `namespace:`, else the name with `-` → `_`.
    pub namespace: String,
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
    /// Why it exists (required; `None` only when the manifest didn't load).
    pub intent: Option<Intent>,
    pub lenses: Vec<Lens>,
    /// Declared collectors (valid ones; invalid ones are in `errors`).
    pub collectors: Vec<oxplow_config::collectors::CollectorSpec>,
    /// Declared providers (valid ones — invalid ones are in `errors`).
    pub providers: Vec<crate::providers::ProviderSpec>,
    /// Web components its `custom` lenses render, sandboxed (valid ones).
    pub custom_components: Vec<custom_components::CustomComponent>,
    /// Event types it declares (valid ones — the vocabulary registers
    /// them, `vocabulary_reactor`).
    pub event_types: crate::extension_event_types::EventTypes,
    /// Resources it declares (valid ones — the vocabulary registers
    /// them, `vocabulary_reactor`).
    pub resources: Vec<crate::extension_resources::ExtensionResource>,
    /// Effects it declares (valid ones — each runs only once a person
    /// approves it, `effects`).
    pub effects: Vec<crate::effects::EffectDecl>,
    /// The capability implementations it declares (`implementations:`,
    /// each a built-in of core's).
    pub implementations: Vec<implementations::ImplementationDecl>,
    /// Skills and slash commands it gives the coding agent (`skills:`;
    /// valid ones — invalid ones are in `errors`).
    pub skills: Vec<skills::SkillDecl>,
    /// Another extension's event types its effects and collectors react
    /// to (P9.D1): each resolves when an enabled extension registers the
    /// type (`vocabulary_reactor`).
    pub subscriptions: Vec<ForeignSubscription>,
    /// `project` (in `.oxplow/extensions/`) or `bundled` (ships with oxplow,
    /// read-only).
    pub origin: String,
    /// What it adds to the core UI (`ui:`).
    pub ui: ExtensionUi,
    /// False when configuration disables it or public contracts block it;
    /// an inactive extension contributes nothing to the runtime catalog.
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
    /// Commands it registers on the bus, each a Starlark script composing
    /// core commands (P6b; valid ones — invalid ones are in `errors`).
    pub commands: Vec<crate::extension_commands::ExtensionCommand>,
    /// Durable host operations declared by this extension.
    pub jobs: Vec<jobs::JobDecl>,
    /// Named model needs this extension declares.
    pub ai_profiles: Vec<ai_profiles::AiProfileDecl>,
    /// Its `version:` (semver), when it declares one; required to share.
    pub version: Option<String>,
    /// Its contributions as descriptors (`extensions::descriptors`): the
    /// kinds that have moved onto them so far.
    pub contributions: Vec<oxplow_domain::contribution::Descriptor>,
    /// Validated non-secret settings declaration, not resolved values or grants.
    #[serde(default)]
    pub configuration: Option<config_schema::ConfigurationFile>,
    /// The sources of its configuration migration steps, by `entry`: read
    /// and parsed at load so activation runs them without touching files.
    #[serde(default, skip_serializing)]
    pub config_scripts: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub public_contracts: public_contracts::PublicContracts,
    /// Where its files came from.
    #[serde(default)]
    pub deployment: Deployment,
    /// What its manifest declares it may reach beyond the baseline
    /// (`permissions:`).
    #[serde(default)]
    pub permissions: Permissions,
    /// The other extensions it needs on (`requires:`).
    #[serde(default)]
    pub requires: Vec<String>,
    /// What a deploy builds before it copies (`build:`).
    #[serde(default)]
    pub build: Option<manifest_v2::Build>,
    /// What it waits on a person for when it is on but undeployed: the
    /// permissions its source would add, one line each
    /// ([`Permissions::added_over`]), and its build command when it has one
    /// (`extension_deploy::build_line`); startup deploys only one that
    /// asks for nothing.
    #[serde(default)]
    pub pending: Vec<String>,
    /// It is the team's: the project's own list names it
    /// (`oxplow.extension.share`).
    #[serde(default)]
    pub shared: bool,
}

impl Extension {
    /// Its declared configuration with no project or personal override: what a
    /// check, which reads no stored settings, compiles `config()` reads against.
    pub fn default_configuration(&self) -> Option<serde_json::Value> {
        self.configuration
            .as_ref()
            .and_then(|declared| declared.resolve(&self.name, None, None).ok())
            .map(|resolved| resolved.values)
    }

    /// The project's own: not one that ships with oxplow. Only it may run
    /// its own programs to record facts and parse reports, each once a
    /// person approved it.
    pub fn is_own(&self) -> bool {
        self.origin == "project"
    }
}

/// What an extension adds to the core UI (`ui:` in its manifest).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUi {
    /// Lenses mounted into core pages (valid ones).
    pub slots: Vec<LensSlot>,
    /// Labels from its models on core refs (valid ones).
    pub decorators: Vec<decorators::UiDecorator>,
    /// Lenses that take the place of a core sub-component while its
    /// provider is the capability's active one (valid ones).
    pub replacements: Vec<replacements::UiReplacement>,
}

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
    /// Set when the lens needs a capability that isn't active: it didn't
    /// run, and says what it needs instead of showing nothing.
    pub inactive: Option<LensInactive>,
}

/// A lens that didn't run for want of a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensInactive {
    /// The needs that aren't met (`work_items`, `snapshots.contents`).
    pub needs: Vec<String>,
    /// What a person reads.
    pub message: String,
}

/// Load every extension of the project at `root` from its source:
/// bundled ones from their source when `root` has one (`extensionSources`,
/// or `.oxplow/extensions/<name>/`), else as they ship; every other one found
/// in `.oxplow/extensions/` or named by the settings. A source load is what
/// an author's sources would do ([`sources_on`]); what runs is the deployed
/// load ([`load_deployed`]).
pub fn load_extensions(root: &Path) -> Vec<Extension> {
    load_extensions_in(root, root)
}

/// [`load_extensions`] of `root`'s sources, with `project`'s settings
/// saying which are on and where sources are — a stream's working copy
/// read under the main checkout's config.
pub fn load_extensions_in(root: &Path, project: &Path) -> Vec<Extension> {
    let settings = oxplow_config::extension_settings(project);
    let sources = load_sources(root, &settings);
    let settings = sources_on(settings, &sources);
    resolve_catalog(sources, &settings)
}

/// `settings` for a source load: every project source loaded is on unless
/// the person turned it off (`disableExtensions`); for the extensions that
/// ship with oxplow, the lists decide as always.
pub(crate) fn sources_on(
    mut settings: oxplow_config::ExtensionSettings,
    sources: &[Extension],
) -> oxplow_config::ExtensionSettings {
    for ext in sources {
        let present = ext.origin == "project" && ext.deployment == Deployment::Source;
        if present
            && !settings.personal.disable.contains(&ext.name)
            && !settings.is_enabled(&ext.name)
        {
            settings.personal.enable.push(ext.name.clone());
        }
    }
    settings
}

/// How a folder is shown: relative to `root` when inside it, else whole.
pub(crate) fn shown(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| dir.to_string_lossy().into_owned())
}

/// Every extension name the project at `root` knows: the bundled ones, the
/// folders in `.oxplow/extensions/`, the names `extensionSources` points
/// at and the names listed as on. Bundled first, then the rest by name.
pub fn known_names(root: &Path, settings: &oxplow_config::ExtensionSettings) -> Vec<String> {
    let mut names: BTreeSet<String> = std::fs::read_dir(root.join(EXTENSIONS_DIR))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.extend(settings.team_sources.keys().cloned());
    names.extend(settings.personal_sources.keys().cloned());
    names.extend(settings.enabled().into_iter().map(|(name, _)| name));
    let bundled = crate::bundled_extensions::BUNDLED
        .iter()
        .map(|b| b.name.to_string());
    let rest: Vec<String> = names
        .into_iter()
        .filter(|n| crate::bundled_extensions::find(n).is_none())
        .collect();
    bundled.chain(rest).collect()
}

/// A bundled extension as it ships, from its shipped deployment in
/// `project`'s store (written there the first time).
fn shipped(project: &Path, b: &crate::bundled_extensions::BundledExtension) -> Extension {
    match crate::extension_store::extract_shipped(project, b) {
        Ok(hash) => {
            let dir = crate::extension_store::deployment_dir(project, b.name, &hash);
            Extension {
                deployment: Deployment::Shipped { hash },
                ..load_one(
                    &Disk(dir.clone()),
                    b.name,
                    &dir.to_string_lossy(),
                    "bundled",
                )
            }
        }
        Err(e) => Extension {
            errors: vec![format!(
                "{}: the copy that ships with oxplow couldn't be written to {}: {e}",
                b.name,
                crate::extension_store::STORE_DIR
            )],
            ..empty_extension(b.name, b.name, "bundled")
        },
    }
}

/// A listed extension that has nothing to run here.
fn listed_only(name: &str, path: &str, deployment: Deployment) -> Extension {
    Extension {
        enabled: false,
        deployment,
        ..empty_extension(name, path, "project")
    }
}

fn origin_of(name: &str) -> &'static str {
    if crate::bundled_extensions::find(name).is_some() {
        "bundled"
    } else {
        "project"
    }
}

pub(super) fn load_sources(
    root: &Path,
    settings: &oxplow_config::ExtensionSettings,
) -> Vec<Extension> {
    let mut out = Vec::new();
    for name in known_names(root, settings) {
        let dir = settings.source(root, &name);
        let bundled = crate::bundled_extensions::shipped(&name, settings);
        if dir.is_dir() {
            out.push(load_one(
                &Disk(dir.clone()),
                &name,
                &shown(root, &dir),
                origin_of(&name),
            ));
        } else if let Some(b) = bundled {
            out.push(shipped(root, b));
        } else if settings.is_enabled(&name) {
            out.push(listed_only(&name, &shown(root, &dir), Deployment::NotHere));
        }
    }
    for ext in &mut out {
        locate(ext, root, settings);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    out
}

/// Load the extensions the project at `project` runs: each one's
/// deployment in `live` (`name → hash`), a bundled one without a
/// deployment as it ships, and the rest it knows listed but not run
/// (undeployed when its source is here, not here otherwise).
pub fn load_deployed(
    project: &Path,
    settings: &oxplow_config::ExtensionSettings,
    live: &BTreeMap<String, String>,
) -> Vec<Extension> {
    let mut names = known_names(project, settings);
    for name in live.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    let mut out: Vec<Extension> = names
        .iter()
        .filter_map(|name| {
            if let Some(hash) = live.get(name) {
                let dir = crate::extension_store::deployment_dir(project, name, hash);
                return Some(Extension {
                    deployment: Deployment::Deployed { hash: hash.clone() },
                    ..load_one(
                        &Disk(dir.clone()),
                        name,
                        &dir.to_string_lossy(),
                        origin_of(name),
                    )
                });
            }
            if let Some(b) = crate::bundled_extensions::shipped(name, settings) {
                return Some(shipped(project, b));
            }
            let source = settings.source(project, name);
            if source.is_dir() {
                let loaded = load_extension_at(&source, name);
                let mut pending = loaded.permissions.added_over(&Permissions::default());
                if let Some(build) = &loaded.build {
                    pending.push(crate::extension_deploy::build_line(
                        &build.run,
                        &shown(project, &source),
                    ));
                }
                Some(Extension {
                    pending,
                    permissions: loaded.permissions,
                    ..listed_only(name, &shown(project, &source), Deployment::Undeployed)
                })
            } else if settings.is_enabled(name) || crate::bundled_extensions::is_required(name) {
                Some(listed_only(
                    name,
                    &shown(project, &source),
                    Deployment::NotHere,
                ))
            } else {
                None
            }
        })
        .collect();
    for ext in &mut out {
        ext.shared = settings.team.contains(&ext.name);
        locate(ext, project, settings);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    resolve_catalog(out, settings)
}

/// Fill in where each of `ext`'s lenses comes from: the deployment it
/// runs, and its source file when the source is here.
fn locate(ext: &mut Extension, project: &Path, settings: &oxplow_config::ExtensionSettings) {
    let deployment = match &ext.deployment {
        Deployment::Shipped { hash } | Deployment::Deployed { hash } => Some(hash.clone()),
        _ => None,
    };
    let dir = settings.source(project, &ext.name);
    let source = dir.is_dir().then(|| shown(project, &dir));
    // What it reports names the source files, the ones to edit, not the
    // copy that runs.
    if let Some(source) = source.as_ref().filter(|s| **s != ext.path) {
        let (from, to) = (format!("{}/", ext.path), format!("{source}/"));
        for message in ext.errors.iter_mut().chain(ext.warnings.iter_mut()) {
            if let Some(rest) = message.strip_prefix(&from) {
                *message = format!("{to}{rest}");
            }
        }
    }
    for lens in &mut ext.lenses {
        let file = lens.origin.running.rsplit('/').next().unwrap_or_default();
        lens.origin.source = source.as_ref().map(|dir| format!("{dir}/lenses/{file}"));
        lens.origin.deployment = deployment.clone();
    }
}

/// `rel` is `dir` or inside it, by `/` segments.
fn under(dir: &str, rel: &str) -> bool {
    let dir: Vec<&str> = dir.trim_end_matches('/').split('/').collect();
    let rel: Vec<&str> = rel.trim_end_matches('/').split('/').collect();
    rel.starts_with(&dir)
}

/// Extensions whose `requires` lead back to themselves could never be
/// turned on: each member of the cycle is an error naming the next.
fn refuse_requires_cycles(out: &mut [Extension]) {
    let requires: BTreeMap<String, Vec<String>> = out
        .iter()
        .map(|e| (e.name.clone(), e.requires.clone()))
        .collect();
    for ext in out.iter_mut() {
        for next in &ext.requires {
            // Walk on from `next`; reaching `ext` again is a cycle.
            let mut seen = vec![next.as_str()];
            let mut i = 0;
            let mut back = false;
            while i < seen.len() {
                for r in requires.get(seen[i]).into_iter().flatten() {
                    if *r == ext.name {
                        back = true;
                    } else if !seen.contains(&r.as_str()) {
                        seen.push(r.as_str());
                    }
                }
                i += 1;
            }
            if back {
                ext.errors.push(format!(
                    "{}/extension.yaml: it requires `{next}`, which requires it back",
                    ext.path
                ));
            }
        }
    }
}

/// Whether the settings leave `name` off. A required extension is never off.
pub(crate) fn is_off(settings: &oxplow_config::ExtensionSettings, name: &str) -> bool {
    !crate::bundled_extensions::is_required(name) && !settings.is_enabled(name)
}

pub(super) fn resolve_catalog(
    mut out: Vec<Extension>,
    settings: &oxplow_config::ExtensionSettings,
) -> Vec<Extension> {
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    // A document's or dashboard's part from ANOTHER extension resolves
    // here, once every extension is loaded; a missing one is an error and
    // the lens is dropped so every loaded lens renders.
    let all_ids: Vec<String> = out
        .iter()
        .flat_map(|e| e.lenses.iter().map(|l| l.id.clone()))
        .collect();
    for ext in &mut out {
        let mut bad = Vec::new();
        for l in &ext.lenses {
            for c in l.part_lenses() {
                if !all_ids.iter().any(|id| id == c) {
                    ext.errors.push(format!(
                        "{}: part lens `{c}` isn't in any loaded extension",
                        l.origin.running
                    ));
                    bad.push(l.id.clone());
                }
            }
        }
        ext.lenses.retain(|l| !bad.contains(&l.id));
    }
    refuse_part_cycles(&mut out);
    for ext in &mut out {
        ext.contributions.retain(|descriptor| {
            descriptor.id.kind != oxplow_domain::contribution::ContributionKind::Lens
                || ext
                    .lenses
                    .iter()
                    .any(|lens| lens.slug == descriptor.id.local)
        });
    }
    refuse_requires_cycles(&mut out);
    let mut out: Vec<Extension> = out
        .into_iter()
        .map(|mut e| {
            if is_off(settings, &e.name) {
                e.enabled = false;
            }
            e
        })
        .collect();
    public_contracts::stage(&mut out);
    crate::extension_commands::refuse_shared_namespaces(&mut out);
    crate::extension_commands::refuse_duplicate_tools(&mut out);
    for ext in &mut out {
        ext.contributions.retain(|descriptor| {
            descriptor.id.kind != oxplow_domain::contribution::ContributionKind::Command
                || ext
                    .commands
                    .iter()
                    .any(|command| descriptor.global.as_deref() == Some(&command.name))
        });
    }
    descriptors::check_catalog(&mut out);
    public_contracts::activate(&mut out);
    let mut out: Vec<Extension> = out
        .into_iter()
        .map(|mut ext| {
            if ext.public_contracts.blocked {
                clear_contributions(&mut ext);
            }
            apply_disabled(ext, settings)
        })
        .collect();
    place_lens_mounts(&mut out);
    fill_view_options(&mut out);
    warn_unread_row_columns(&mut out);
    out
}

/// Refuse the lenses whose parts lead back to themselves (A's section is
/// B, B's is A): a page would hold itself without end. Each lens on a
/// cycle is an error of its extension and is dropped.
fn refuse_part_cycles(out: &mut [Extension]) {
    let parts: BTreeMap<String, Vec<String>> = out
        .iter()
        .flat_map(|e| e.lenses.iter())
        .map(|l| (l.id.clone(), l.part_lenses().map(str::to_string).collect()))
        .collect();
    // Whether `from` reaches `to` through parts, never walking a lens twice.
    let reaches = |from: &str, to: &str| -> bool {
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![from.to_string()];
        while let Some(at) = stack.pop() {
            for next in parts.get(&at).into_iter().flatten() {
                if next == to {
                    return true;
                }
                if seen.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        false
    };
    let cyclic: Vec<String> = parts.keys().filter(|id| reaches(id, id)).cloned().collect();
    for ext in out.iter_mut() {
        for l in ext.lenses.iter().filter(|l| cyclic.contains(&l.id)) {
            ext.errors.push(format!(
                "{}: its parts lead back to `{}`; a page can't hold itself",
                l.origin.running, l.id
            ));
        }
        ext.lenses.retain(|l| !cyclic.contains(&l.id));
    }
}

/// A view document's view param takes its sections as its choices, its
/// own and the mounted ones: each part's id, labelled by its title, else
/// its lens's title, else its id. Replaced, never added to: a catalog is
/// resolved again from resolved extensions.
fn fill_view_options(out: &mut [Extension]) {
    let titles: BTreeMap<String, String> = out
        .iter()
        .flat_map(|e| e.lenses.iter())
        .map(|l| (l.id.clone(), l.title.clone()))
        .collect();
    for lens in out.iter_mut().flat_map(|e| e.lenses.iter_mut()) {
        let Some(view) = lens.view.clone() else {
            continue;
        };
        let options: Vec<LensParamOption> = lens
            .parts
            .iter()
            .map(|part| LensParamOption {
                value: part.id.clone(),
                label: part
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .or_else(|| part.lens.as_ref().and_then(|l| titles.get(l).cloned()))
                    .unwrap_or_else(|| part.id.clone()),
            })
            .collect();
        if let Some(p) = lens.params.iter_mut().find(|p| p.name == view) {
            p.options = options;
        }
    }
}

/// A composite's row names a value its parts take as params: one none of
/// its parts (its own and the mounted ones, once placed) declares is read
/// by nothing. Said only when every part's lens is loaded.
fn warn_unread_row_columns(out: &mut [Extension]) {
    let params: BTreeMap<String, Vec<String>> = out
        .iter()
        .flat_map(|e| e.lenses.iter())
        .map(|l| {
            (
                l.id.clone(),
                l.params.iter().map(|p| p.name.clone()).collect(),
            )
        })
        .collect();
    for ext in out.iter_mut() {
        // The columns a page's label reads off a lens's row.
        let labelled: BTreeMap<String, Vec<String>> = ext
            .pages
            .iter()
            .filter_map(|p| match &p.body {
                PageBody::Lens { lens } => {
                    Some((lens.clone(), label_row_columns(p.label.as_deref())))
                }
                _ => None,
            })
            .collect();
        let warnings: Vec<String> = ext
            .lenses
            .iter()
            .filter(|l| matches!(l.viz, LensViz::Document | LensViz::Dashboard))
            .flat_map(|l| {
                let parts: Option<Vec<&Vec<String>>> =
                    l.part_lenses().map(|p| params.get(p)).collect();
                let read_by_label = labelled.get(&l.id).cloned().unwrap_or_default();
                let taken = move |name: &str| {
                    read_by_label.iter().any(|x| x == name)
                        || parts
                            .as_deref()
                            .is_none_or(|p| p.iter().any(|ps| ps.iter().any(|x| x == name)))
                };
                l.columns
                    .iter()
                    .filter(move |c| !taken(&c.key))
                    .map(|c| {
                        format!(
                            "{}: column `{}` of the row is a param of none of its parts — \
                             remove it, or declare it on the part that reads it",
                            l.origin.running, c.key
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        ext.warnings.extend(warnings);
    }
}

/// Mounts among another lens's parts (`<extension>/<slug>.sections` of a
/// document, `.tiles` of a dashboard), once the catalog is final: each is
/// checked against that lens and placed among its parts by its `at`, so
/// the lens's page, its text and `get_lens` all show it. A bad one is an
/// error of the extension that mounts it, and goes.
fn place_lens_mounts(out: &mut [Extension]) {
    // A catalog resolved again from resolved extensions places its mounts
    // afresh: the ones placed before go first.
    for lens in out.iter_mut().flat_map(|e| e.lenses.iter_mut()) {
        lens.parts.retain(|p| p.from.is_none());
    }
    struct Target {
        params: Vec<String>,
        /// The names of its query's row (`columns:`), given to its parts.
        row: Vec<String>,
        kind: PartsKind,
        ids: Vec<String>,
        /// A view document: each section is a page of its own, so a mount
        /// needn't read any of the document's params.
        view: bool,
    }
    let targets: BTreeMap<String, Target> = out
        .iter()
        .flat_map(|e| e.lenses.iter())
        .filter_map(|l| {
            let kind = match l.viz {
                LensViz::Document => PartsKind::Sections,
                LensViz::Dashboard => PartsKind::Tiles,
                _ => return None,
            };
            Some((
                l.id.clone(),
                Target {
                    params: l.params.iter().map(|p| p.name.clone()).collect(),
                    row: l.columns.iter().map(|c| c.key.clone()).collect(),
                    kind,
                    ids: l.parts.iter().map(|p| p.id.clone()).collect(),
                    view: l.view.is_some(),
                },
            ))
        })
        .collect();
    let mut placed: BTreeMap<String, Vec<(LensPart, SlotAt)>> = BTreeMap::new();
    for ext in out.iter_mut() {
        let manifest = format!("{}/extension.yaml", ext.path);
        let mut dropped = Vec::new();
        for (index, mount) in ext.ui.slots.iter().enumerate() {
            let Some((target, kind)) = lens_slot_target(&mount.slot) else {
                continue;
            };
            let Some(lens) = ext.lenses.iter().find(|l| l.id == mount.lens_id) else {
                continue;
            };
            let checked = match targets.get(target) {
                None => Err(format!(
                    "slot `{}` names lens `{target}`, which isn't a loaded document or dashboard",
                    mount.slot
                )),
                Some(t) if t.kind != kind => Err(format!(
                    "slot `{}`: lens `{target}` is a {}, so its parts are at `{target}.{}`",
                    mount.slot,
                    if t.kind == PartsKind::Sections {
                        "document"
                    } else {
                        "dashboard"
                    },
                    if t.kind == PartsKind::Sections {
                        "sections"
                    } else {
                        "tiles"
                    },
                )),
                Some(t) => {
                    fn offered(names: &[String], view: bool) -> Vec<&str> {
                        if view {
                            Vec::new()
                        } else {
                            names.iter().map(String::as_str).collect()
                        }
                    }
                    let params = offered(&t.params, t.view);
                    let row = offered(&t.row, t.view);
                    let ids: Vec<&str> = t.ids.iter().map(String::as_str).collect();
                    mount_problem(mount, lens, &params, &row, kind, &ids)
                }
            };
            match checked {
                Err(problem) => {
                    ext.errors.push(format!("{manifest}: {problem}"));
                    dropped.push(index);
                }
                Ok(warning) => {
                    ext.warnings
                        .extend(warning.map(|w| format!("{manifest}: {w}")));
                    placed.entry(target.to_string()).or_default().push((
                        LensPart {
                            id: mount.lens_id.clone(),
                            title: mount.title.clone(),
                            lens: Some(mount.lens_id.clone()),
                            markdown: None,
                            size: mount.size,
                            replaceable: None,
                            from: Some(ext.name.clone()),
                        },
                        mount.at.clone().unwrap_or_default(),
                    ));
                }
            }
        }
        for index in dropped.into_iter().rev() {
            let mount = ext.ui.slots.remove(index);
            let id = oxplow_domain::contribution::ContributionId::new(
                &ext.name,
                oxplow_domain::contribution::ContributionKind::Slot,
                &mount.local_id(index),
            );
            ext.contributions.retain(|d| d.id != id);
        }
    }
    for lens in out.iter_mut().flat_map(|e| e.lenses.iter_mut()) {
        if let Some(mounts) = placed.remove(&lens.id) {
            lens.parts = place_parts(std::mem::take(&mut lens.parts), mounts);
        }
    }
}

/// A page's own parts with mounts placed among them: `start` ones first,
/// each `before`/`after` one beside its part, `end` ones (and any beside a
/// part the page doesn't have) last; mounts at one place keep their order.
/// The UI places mounts into core pages the same way (`kit/placeParts.ts`).
fn place_parts(own: Vec<LensPart>, mounts: Vec<(LensPart, SlotAt)>) -> Vec<LensPart> {
    let ids: Vec<String> = own.iter().map(|p| p.id.clone()).collect();
    let mounts: Vec<(LensPart, SlotAt)> = mounts
        .into_iter()
        .map(|(part, at)| match at.anchor() {
            Some(id) if !ids.iter().any(|i| i == id) => (part, SlotAt::default()),
            _ => (part, at),
        })
        .collect();
    let at = |pred: &dyn Fn(&SlotAt) -> bool| -> Vec<LensPart> {
        mounts
            .iter()
            .filter(|(_, a)| pred(a))
            .map(|(p, _)| p.clone())
            .collect()
    };
    let mut out = at(&|a| *a == SlotAt::Edge(SlotEdge::Start));
    for part in own {
        let id = part.id.clone();
        out.extend(at(
            &|a| matches!(a, SlotAt::Before { before } if *before == id),
        ));
        out.push(part);
        out.extend(at(
            &|a| matches!(a, SlotAt::After { after } if *after == id),
        ));
    }
    out.extend(at(&|a| *a == SlotAt::default()));
    out
}

/// A catalog block's error as the manifest's, not project.yaml's.
fn manifest_message(e: oxplow_config::ConfigError) -> String {
    match e {
        oxplow_config::ConfigError::Invalid(m) => m,
        other => other.to_string(),
    }
}

/// A `measures:` / `metrics:` block as typed entries.
fn parse_block<T: serde::de::DeserializeOwned>(
    v: Option<serde_yaml::Value>,
) -> Result<Option<Vec<T>>, String> {
    v.map(|v| serde_yaml::from_value(v).map_err(|e| e.to_string()))
        .transpose()
}

/// Where `name`'s files are for a preview in the checkout at `root`: its
/// source when the checkout has one, else, for an extension that ships with
/// oxplow, its shipped deployment in `project`'s store.
pub fn source_or_shipped(project: &Path, root: &Path, name: &str) -> String {
    let settings = oxplow_config::extension_settings(project);
    let source = settings.source(root, name);
    if !source.is_dir() {
        if let Some(b) = crate::bundled_extensions::shipped(name, &settings) {
            if let Ok(hash) = crate::extension_store::extract_shipped(project, b) {
                return crate::extension_store::deployment_dir(project, name, &hash)
                    .to_string_lossy()
                    .into_owned();
            }
        }
    }
    shown(root, &source)
}

/// Extension `name` alone, from its default source folder under `root`
/// (`.oxplow/extensions/<name>/`).
pub fn load_project_extension(root: &Path, name: &str) -> Extension {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    load_one(&Disk(root.join(&rel)), name, &rel, origin_of(name))
}

/// Extension `name` alone, from the folder `dir` (its source, or a copy
/// laid out like one).
pub fn load_extension_at(dir: &Path, name: &str) -> Extension {
    load_one(
        &Disk(dir.to_path_buf()),
        name,
        &dir.to_string_lossy(),
        origin_of(name),
    )
}

/// A file inside the extension whose listed `path` is `path` (its
/// folder), e.g. a collector's script. `None` when there's no such file.
pub fn read_extension_file(root: &Path, path: &str, rel: &str) -> Option<String> {
    if !oxplow_domain::service::valid_path(rel) {
        return None;
    }
    files_at(root, path).ok()?.read(rel)
}

/// The most an extension file read from disk may hold.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The files at an extension's `path` as a listing gives it: the folder
/// `project_dir/path` on disk (an absolute `path`, a deployment's, is
/// itself) — one resolver, so what an approval covers is read the same way
/// wherever the extension lives.
pub(crate) fn files_at(project_dir: &Path, path: &str) -> std::io::Result<Box<dyn ExtensionFiles>> {
    Ok(Box::new(Disk(project_dir.join(path))))
}

/// Where an extension's files come from: a folder on disk, or one
/// revision's in memory.
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

impl Disk {
    fn checked_path(&self, rel: &str) -> std::io::Result<std::path::PathBuf> {
        if !rel.is_empty() && !oxplow_domain::service::valid_path(rel) {
            return Err(std::io::Error::other(
                "extension path must be relative without traversal",
            ));
        }
        let mut path = self.0.clone();
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(std::io::Error::other("extension folder is a symlink"));
        }
        for segment in rel
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
        {
            path.push(segment);
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(std::io::Error::other("extension path contains a symlink"));
            }
        }
        Ok(path)
    }
}

impl ExtensionFiles for Disk {
    fn read(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.bytes(rel).ok()?).ok()
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
            if !entry.file_type().is_file() && !entry.file_type().is_dir() {
                return Err(std::io::Error::other(format!(
                    "{} is not a regular file or directory",
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
        use std::io::Read;

        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(self.checked_path(rel)?)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other(
                "extension path is not a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::other(
                "extension file exceeds the 16 MiB limit",
            ));
        }
        Ok(bytes)
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let Ok(path) = self.checked_path(dir) else {
            return Vec::new();
        };
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook {
        // `stat_bundle` never follows a symlink and reports one (the folder
        // itself included), so a symlinked bundle says so rather than
        // looking absent; only the path's shape is checked here.
        if !oxplow_domain::service::valid_path(rel) {
            return custom_components::BundleLook::Absent;
        }
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
        custom_components::look_in_paths(rel, self.0.keys().map(String::as_str))
    }
}

/// Project extension `name` as it is at `rev` of the workspace `ws` — a
/// commit, a snapshot, the working tree — through `Trees` (P8.C1). `None`
/// when that revision has no `.oxplow/extensions/<name>/extension.yaml`.
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

/// One extension that changed between two revisions of a workspace:
/// what an effort's review shows for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionChange {
    pub name: String,
    /// `added`, `removed` or `changed`.
    pub change: crate::extension_impact::Change,
    /// What the change does; `None` for a removed extension.
    pub impact: Option<crate::extension_impact::ImpactReport>,
    /// What's wrong with the later version.
    pub errors: Vec<String>,
}

/// The extensions under `.oxplow/extensions/` whose files differ between
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
                    change: crate::extension_impact::Change::Removed,
                    impact: None,
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
        let impact = impact_between(
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
                crate::extension_impact::Change::Changed
            } else {
                crate::extension_impact::Change::Added
            },
            name,
            impact: Some(impact),
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
/// change: each side on its own overlay — `after` checked,
/// `before` only read — then compared: lenses, models and their rows,
/// collectors and their outputs, providers, config. `after.extension`
/// gets its check's errors.
pub async fn impact_between(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    mut before: Option<ReviewSide<'_>>,
    after: &mut ReviewSide<'_>,
    commands: CommandSchemas<'_>,
) -> crate::extension_impact::ImpactReport {
    let prepared_after = prepare(
        layer,
        catalog,
        root,
        &mut after.extension,
        after.read,
        Some(commands),
    )
    .await;
    let prepared_before = match &mut before {
        Some(b) => Some(read_side(layer, catalog, root, &mut b.extension).await),
        None => None,
    };
    let no_runs = LensRuns::new();
    crate::extension_impact::impact(
        layer,
        before.as_ref().map(|b| crate::extension_impact::Version {
            extension: &b.extension,
            read: b.read,
            lenses: prepared_before.as_ref().map_or(&no_runs, |p| &p.lenses),
            overlay: prepared_before
                .as_ref()
                .map_or(&[], |p| p.overlay.as_slice()),
        }),
        crate::extension_impact::Version {
            extension: &after.extension,
            read: after.read,
            lenses: &prepared_after.lenses,
            overlay: &prepared_after.overlay,
        },
    )
    .await
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
    use oxplow_domain::events::schema::{
        extension_namespace, extension_type_namespace, is_core_type,
    };
    if is_core_type(event_type) || declared.iter().any(|d| d.event_type == event_type) {
        return Subscribes::Known;
    }
    match extension_type_namespace(event_type) {
        Some(ns) if ns != extension_namespace(extension) => Subscribes::Foreign,
        _ => Subscribes::Unknown,
    }
}

pub(crate) fn empty_extension(name: &str, path: &str, origin: &str) -> Extension {
    Extension {
        name: name.to_string(),
        namespace: oxplow_domain::contribution::extension_namespace(name),
        description: String::new(),
        path: path.to_string(),
        errors: Vec::new(),
        warnings: Vec::new(),
        manifest_version: manifest_v2::CURRENT,
        intent: None,
        lenses: Vec::new(),
        collectors: Vec::new(),
        providers: Vec::new(),
        custom_components: Vec::new(),
        event_types: Default::default(),
        resources: Vec::new(),
        effects: Vec::new(),
        implementations: Vec::new(),
        skills: Vec::new(),
        subscriptions: Vec::new(),
        origin: origin.to_string(),
        ui: ExtensionUi::default(),
        enabled: true,
        advisories: Vec::new(),
        measures: Vec::new(),
        dimensions: Vec::new(),
        models: Vec::new(),
        metrics: Vec::new(),
        panels: Vec::new(),
        pages: Vec::new(),
        commands: Vec::new(),
        jobs: Vec::new(),
        ai_profiles: Vec::new(),
        version: None,
        contributions: Vec::new(),
        configuration: None,
        config_scripts: Default::default(),
        public_contracts: Default::default(),
        deployment: Deployment::Source,
        permissions: Permissions::default(),
        requires: Vec::new(),
        build: None,
        pending: Vec::new(),
        shared: false,
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
    // A bundled extension ships with oxplow: its version is oxplow's.
    ext.version = if origin == "bundled" {
        Some(env!("CARGO_PKG_VERSION").to_string())
    } else {
        m.version.clone()
    };
    if origin != "bundled" && !oxplow_domain::contribution::valid_extension_name(name) {
        ext.errors.push(format!(
            "{file}: the name `{name}` must be lowercase letters, digits and single dashes, \
             starting with a letter (rename the folder)"
        ));
    }
    if let Some(ns) = &m.namespace {
        match crate::extension_commands::check_namespace(ns, origin == "bundled") {
            Ok(()) => ext.namespace = ns.clone(),
            Err(e) => ext
                .errors
                .push(at(&file, key_line(&manifest, "namespace"), e)),
        }
    }
    for required in &m.requires {
        let problem = if required == name {
            Some("an extension can't require itself".to_string())
        } else if !oxplow_domain::contribution::valid_extension_name(required) {
            Some(format!("`{required}` isn't an extension name"))
        } else {
            None
        };
        match problem {
            Some(p) => ext.errors.push(at(
                &file,
                key_line(&manifest, "requires"),
                format!("requires: {p}"),
            )),
            None => ext.requires.push(required.clone()),
        }
    }
    let (build, errors) = manifest_v2::build_of(&m, &file, &manifest);
    ext.build = build;
    ext.errors.extend(errors);
    ext.intent = manifest_v2::intent_of(&m, &file, &manifest).0;
    ext.description = m.description.clone();
    ext.configuration = manifest_v2::configuration_of(&m, &file, &manifest).0;
    if let Some(configuration) = &ext.configuration {
        let line = key_line(&manifest, "configuration");
        for step in &configuration.migrations {
            match files.read(&step.entry) {
                None => ext.errors.push(at(
                    &file,
                    line,
                    format!(
                        "configuration migration {} -> {}: `{}` isn't in the extension",
                        step.from, step.to, step.entry
                    ),
                )),
                Some(script) => {
                    match oxplow_script::runtime::check_starlark(&step.entry, &script) {
                        Ok(()) => {
                            ext.config_scripts.insert(step.entry.clone(), script);
                        }
                        Err(e) => ext.errors.push(at(
                            &file,
                            line,
                            format!(
                                "configuration migration {} -> {}: `{}` {e}",
                                step.from, step.to, step.entry
                            ),
                        )),
                    }
                }
            }
        }
    }
    public_contracts::load(&mut ext, &m, &manifest, &file);
    let (errors, warnings) = manifest_v2::check(&m, &file, &manifest);
    if !errors.is_empty()
        && (!m.exports.is_empty() || !m.imports.is_empty() || !m.import_guards.is_empty())
    {
        ext.public_contracts.invalid = true;
    }
    ext.errors.extend(errors);
    ext.warnings.extend(warnings);
    let panel_files = m.panels.clone();
    let decorator_files = m.ui.decorators.clone();
    let replacement_files = m.ui.replacements.clone();
    let page_files = m.pages.clone();
    let resource_files = m.resources.clone();
    let slot_files = {
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
        if let Some(v) = m.skills.as_ref() {
            let (declared, errors) = skills::parse_skills(v, &file, &manifest, files);
            ext.skills = declared;
            ext.errors.extend(errors);
        }
        if let Some(v) = m.implementations.as_ref() {
            let (declared, errors) =
                implementations::parse_implementations(v, &file, &manifest, &|rel| files.read(rel));
            // An implementation owns the skills it lists: they must be its
            // extension's.
            for d in &declared {
                for skill in d
                    .skills
                    .iter()
                    .filter(|s| !ext.skills.iter().any(|k| &k.name == *s))
                {
                    ext.errors.push(at(
                        &file,
                        entry_line(&manifest, "implementations", "id", &d.id),
                        format!("implementation `{}` lists skill `{skill}`, which `skills:` doesn't declare", d.id),
                    ));
                }
            }
            ext.implementations = declared;
            ext.errors.extend(errors);
        }
        // The project's own: not one that ships with oxplow.
        let own = origin == "project";
        if let Some(v) = &m.collectors {
            // A collector may follow core types, its own extension's, and
            // another's.
            let (collectors, errors) = oxplow_config::collectors::parse_collectors_indexed(
                &oxplow_config::collectors::Declarer {
                    extension: name.to_string(),
                    namespace: ext.namespace.clone(),
                    own,
                },
                v,
                &subscribable,
            );
            let line = key_line(&manifest, "collectors");
            let lines = item_lines(&manifest, "collectors");
            ext.errors.extend(errors.into_iter().map(|(i, e)| {
                let at_entry = i.and_then(|i| lines.get(i).copied());
                at(&file, at_entry.or(line), format!("collector: {e}"))
            }));
            // A script or program the collector runs must be in the folder,
            // and a Starlark script must parse and define `transform`; a
            // report read by one of oxplow's own parsers (`oxplow:<name>`)
            // runs nothing of the extension's.
            for c in collectors {
                let c_line = entry_line(&manifest, "collectors", "id", &c.id).or(line);
                if c.bundled_parser().is_some() {
                    ext.collectors.push(c);
                    continue;
                }
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
                        let needs = crate::services::collector_needs(&c, Some(&script));
                        let checked = oxplow_script::runtime::check_starlark(entry, &script)
                            .and_then(|()| {
                                crate::services::check_uses(&script, &needs, |_| Ok(()))
                            });
                        match checked {
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
        if let Some(v) = &m.providers {
            let (providers, errors) = crate::providers::parse_providers(v, &|rel| files.read(rel));
            ext.providers = providers;
            ext.errors.extend(
                errors
                    .into_iter()
                    .map(|e| at(&file, key_line(&manifest, "providers"), e)),
            );
        }
        if let Some(v) = &m.commands {
            let providers = ext.providers.clone();
            let provider_ops = |provider: &str, op: &str| {
                let spec = providers
                    .iter()
                    .find(|p| p.id == provider)
                    .ok_or_else(|| format!("no provider `{provider}` in this extension"))?;
                let contract = crate::providers::spec::contract_of(spec);
                if contract.verb(op).is_some() {
                    return Err(match contract.dispatch {
                        Some(family) => format!(
                            "`{op}` is a {} verb: it runs as `{family}.{op}`",
                            spec.capability
                        ),
                        None => format!("`{op}` is a {} verb: core calls it", spec.capability),
                    });
                }
                crate::providers::spec::read_declarations(spec, &|rel| files.read(rel))?
                    .commands
                    .into_iter()
                    .find(|c| c.name == op)
                    .ok_or_else(|| format!("its declarations have no command `{op}`"))
            };
            let (commands, errors) = crate::extension_commands::parse_commands(
                &ext.namespace,
                origin == "bundled",
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
                &provider_ops,
            );
            ext.commands = commands;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.jobs {
            let (jobs, errors) =
                jobs::parse_jobs(v, &file, &manifest, &ext.namespace, &ext.commands);
            ext.jobs = jobs;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.ai_profiles {
            let (profiles, errors) = ai_profiles::parse_ai_profiles(v, &file, &manifest);
            ext.ai_profiles = profiles;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.custom_components {
            let (components, errors) =
                custom_components::parse_custom_components(name, v, &file, &manifest, &|rel| {
                    match (files.bundle_stat(rel), &ext.build) {
                        // Not built yet (no output at all): the deployment's
                        // load checks it.
                        (custom_components::BundleLook::Absent, Some(build))
                            if under(&build.output, rel)
                                && matches!(
                                    files.bundle_stat(&build.output),
                                    custom_components::BundleLook::Absent
                                ) =>
                        {
                            custom_components::BundleLook::Unknown
                        }
                        (look, _) => look,
                    }
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
                audience: a.audience,
                needs: a.needs,
            }) {
                Ok(a)
                    if a.needs
                        .iter()
                        .any(|n| oxplow_domain::capability::check_need(n).is_err()) =>
                {
                    for need in &a.needs {
                        if let Err(e) = oxplow_domain::capability::check_need(need) {
                            ext.errors.push(at(
                                &file,
                                line_under(&manifest, "advisories", &a.id),
                                format!("advisory {}: {e}", a.id),
                            ));
                        }
                    }
                }
                Ok(a)
                    if !oxplow_domain::contribution::valid_local(
                        oxplow_domain::contribution::ContributionKind::Advisory,
                        &a.id,
                    ) =>
                {
                    ext.errors.push(at(
                        &file,
                        line_under(&manifest, "advisories", &a.id),
                        format!(
                            "advisory id `{}` must be lowercase letters, digits and dashes",
                            a.id
                        ),
                    ))
                }
                Ok(a) => ext.advisories.push(a),
                Err(e) => ext.errors.push(at(
                    &file,
                    key_line(&manifest, "advisories"),
                    format!("advisory: {e}"),
                )),
            }
        }
        let err_at = |key: &str, e: String| at(&file, key_line(&manifest, key), e);
        match parse_block(m.measures.clone()).and_then(|v| {
            oxplow_config::validate_measures(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => ext.measures = v,
            Err(e) => ext.errors.push(err_at("measures", e)),
        }
        match parse_block(m.dimensions.clone()).and_then(|v| {
            oxplow_config::validate_dimensions(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => {
                for d in v {
                    // Promoting rebuilds the metric cube: the project's call,
                    // or oxplow's own (the foundation's dimensions are the
                    // cube's grain from the first boot).
                    if d.promote && !own && origin != "bundled" {
                        ext.errors.push(err_at(
                            "dimensions",
                            format!(
                                "dimension `{}`: `promote` is the project's own extension's (it rebuilds the metric cube)",
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
        match parse_block(m.metrics.clone()).and_then(|v| {
            oxplow_config::validate_metrics(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => {
                for e in v {
                    let unknown_collector = e
                        .collector
                        .as_deref()
                        .filter(|c| !ext.collectors.iter().any(|x| x.id == *c));
                    if let Some(c) = unknown_collector {
                        ext.errors.push(err_at(
                            "metrics",
                            format!(
                                "metric `{}`: collector `{c}` is not a collector this extension declares",
                                e.key.clone().unwrap_or_default()
                            ),
                        ));
                    } else if e.key.is_some() {
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
        // declares. A measure from another scope
        // (the project's or another extension's `measures:`) resolves
        // when the catalog is assembled, so that is a warning, not an
        // error — but a typo would be silent otherwise.
        let measure_known =
            |key: &str| ext.measures.iter().any(|mm| mm.key.as_deref() == Some(key));
        for spec in &ext.metrics {
            if let Some(sm) = spec.source_measure.as_deref() {
                if !measure_known(sm) {
                    ext.warnings.push(err_at(
                        "metrics",
                        format!(
                            "metric `{}`: sourceMeasure `{sm}` is not a measure this extension declares; it must come from the project's or another extension's `measures:`",
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
                            "collector `{}`: facts `{fact}`, which is not a measure this extension declares; it must come from the project's or another extension's `measures:`",
                            c.id
                        ),
                    ));
                }
            }
        }
        let (slots, errors) = manifest_v2::slots_of(&m, &file, &manifest);
        ext.errors.extend(errors);
        slots
    };
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
                for need in &l.needs {
                    oxplow_domain::capability::check_need(need)?;
                }
                let actions = parse_actions(
                    std::mem::take(&mut l.actions),
                    &l.params,
                    l.group.is_some(),
                    holds_one_row(l.viz, !l.query.trim().is_empty()),
                )?;
                let parts = parse_parts(
                    name,
                    l.viz,
                    std::mem::take(&mut l.sections),
                    std::mem::take(&mut l.tiles),
                    l.tile.is_some(),
                )?;
                let tool = l
                    .tool
                    .take()
                    .map(|t| crate::extension_commands::tool_name(&ext.namespace, &t))
                    .transpose()?;
                Ok((l, alert, actions, tool, parts))
            });
        match parsed {
            Ok((l, alert, actions, tool, parts)) => ext.lenses.push(Lens {
                tool,
                id: format!("{name}/{slug}"),
                extension: name.to_string(),
                slug,
                title: l.title,
                description: l.description,
                query: l.query,
                viz: l.viz,
                params: typed(view_param(l.params, l.view.as_deref())),
                columns: l.columns,
                empty: l.empty,
                chart: l.chart,
                tree: l.tree,
                tile: l.tile,
                timeline: l.timeline,
                steps: l.steps,
                hunks: l.hunks,
                code: l.code,
                diff: l.diff,
                markdown: l.markdown,
                form: l.form,
                custom: l.custom,
                group: l.group,
                emphasis: l.emphasis,
                depth: l.depth,
                parts,
                view: l.view,
                launcher_category: l.launcher.map(|la| la.category),
                hidden: l.hidden,
                actions,
                alert,
                needs: l.needs,
                origin: LensProvenance {
                    extension: name.to_string(),
                    source: None,
                    running: lens_rel,
                    deployment: None,
                    bundled: origin == "bundled",
                },
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

    // A column's `expand` and a param's `options_from` name a lens by slug
    // or `<extension>/<slug>`; the loaded lens has the whole id.
    for l in &mut ext.lenses {
        let extension = l.extension.clone();
        for c in &mut l.columns {
            if let Some(e) = c.expand.as_mut().filter(|e| !e.contains('/')) {
                *e = format!("{extension}/{e}");
            }
        }
        for p in &mut l.params {
            if let Some(o) = p.options_from.as_mut().filter(|o| !o.contains('/')) {
                *o = format!("{extension}/{o}");
            }
        }
    }
    // Drop lenses whose viz lacks what it needs, so every loaded lens renders.
    let components: Vec<String> = ext.custom_components.iter().map(|c| c.id.clone()).collect();
    let mut bad = Vec::new();
    for l in &ext.lenses {
        if let Some(problem) = shape_problem(l, &ids, &components)
            .or_else(|| choices_problem(l, &ids))
            .or_else(|| expand_problem(l, &ids))
            .or_else(|| options_from_problem(l, &ids))
        {
            ext.errors.push(format!("{}: {problem}", l.origin.running));
            bad.push(l.id.clone());
        }
    }
    ext.lenses.retain(|l| !bad.contains(&l.id));
    let selects: Vec<(String, String)> = ext
        .lenses
        .iter()
        .filter_map(|l| {
            Some((
                l.id.clone(),
                format!("{}: {}", l.origin.running, selects_problem(l, &ext.lenses)?),
            ))
        })
        .collect();
    for (_, problem) in &selects {
        ext.errors.push(problem.clone());
    }
    ext.lenses
        .retain(|l| !selects.iter().any(|(id, _)| id == &l.id));
    let replaceable: Vec<(String, String)> = ext
        .lenses
        .iter()
        .filter_map(|l| {
            Some((
                l.id.clone(),
                format!(
                    "{}: {}",
                    l.origin.running,
                    replaceable_problem(l, &ext.lenses)?
                ),
            ))
        })
        .collect();
    for (_, problem) in &replaceable {
        ext.errors.push(problem.clone());
    }
    ext.lenses
        .retain(|l| !replaceable.iter().any(|(id, _)| id == &l.id));
    let selected: Vec<Vec<String>> = ext
        .lenses
        .iter()
        .map(|l| selected(l, &ext.lenses))
        .collect();
    for (lens, names) in ext.lenses.iter_mut().zip(selected) {
        for p in &mut lens.params {
            p.role = Some(if names.contains(&p.name) {
                ParamRole::Selection
            } else {
                p.role.unwrap_or(ParamRole::Resource)
            });
        }
    }

    for s in slot_files {
        let mount_line = entry_line(&manifest, "ui", "lens", &s.lens);
        let Some(lens) = ext.lenses.iter().find(|l| l.slug == s.lens) else {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "slot `{}` mounts lens `{}`, which isn't in lenses/",
                    s.slot, s.lens
                ),
            ));
            continue;
        };
        let mount = LensSlot {
            slot: s.slot,
            lens_id: lens.id.clone(),
            at: s.at,
            title: s.title,
            size: s.size,
        };
        // Another lens's parts resolve once every extension is loaded
        // (`place_lens_mounts`).
        if lens_slot_target(&mount.slot).is_some() {
            ext.ui.slots.push(mount);
            continue;
        }
        let Some(spec) = SlotSpec::core(&mount.slot) else {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "unknown slot `{}` (known: {}, or a document's `<extension>/<lens>.sections` \
                     and a dashboard's `<extension>/<lens>.tiles`)",
                    mount.slot,
                    SLOTS.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
                ),
            ));
            continue;
        };
        let (kind, ids) = match spec.parts {
            SlotParts::Region => (PartsKind::Region, Vec::new()),
            SlotParts::Sections(ids) => (PartsKind::Sections, ids.to_vec()),
            SlotParts::Tiles(ids) => (PartsKind::Tiles, ids.to_vec()),
        };
        match mount_problem(&mount, lens, spec.params, &[], kind, &ids) {
            Err(problem) => ext.errors.push(at(&file, mount_line, problem)),
            Ok(warning) => {
                ext.warnings
                    .extend(warning.map(|w| at(&file, mount_line, w)));
                ext.ui.slots.push(mount);
            }
        }
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
    // After models: a resource is titled by one.
    if let Some(v) = resource_files {
        let (resources, errors) = crate::extension_resources::parse_resources(
            name,
            &ext.namespace,
            &ext.models,
            &v,
            &file,
            &manifest,
        );
        ext.resources = resources;
        ext.errors.extend(errors);
    }
    // After resources: a page renders them.
    if let Some(v) = page_files {
        let (pages, errors) = parse_pages(
            name,
            &ext.namespace,
            &ext.lenses,
            &ext.custom_components,
            &ext.resources,
            v,
        );
        ext.pages = pages;
        let lines = item_lines(&manifest, "pages");
        ext.errors.extend(errors.into_iter().map(|(i, e)| {
            let line = i.and_then(|i| lines.get(i).copied());
            at(&file, line.or(key_line(&manifest, "pages")), e)
        }));
    }
    // Once the pages are known: a renderer's lens is told by its page.
    let layout: Vec<String> = ext
        .lenses
        .iter()
        .flat_map(|l| {
            layout_warnings(l, &ext.lenses, renders_a_resource(&ext, l))
                .into_iter()
                .map(|w| format!("{}: {w}", l.origin.running))
        })
        .collect();
    ext.warnings.extend(layout);
    if let Some(v) = panel_files {
        let (panels, errors) = parse_panels(name, &ext.lenses, v);
        ext.panels = panels;
        let lines = item_lines(&manifest, "panels");
        ext.errors.extend(errors.into_iter().map(|(i, e)| {
            let line = i.and_then(|i| lines.get(i).copied());
            at(&file, line.or(key_line(&manifest, "panels")), e)
        }));
    }
    check_config_reads(&mut ext);
    ext.permissions = m.permissions.clone();
    check_permissions(&mut ext, files, &file, &manifest);
    ext.contributions = descriptors::describe(&ext);
    descriptors::check_ids(&mut ext);
    public_contracts::prepare(&mut ext);
    ext
}

/// What a contribution's declared `needs` ask beyond the baseline: a
/// model (`ai.compute`, `ai.discover`), its own SQL (`sql.read`), the
/// project's files (`tree.read`).
fn needs_permissions(needs: &[String]) -> Permissions {
    let mut p = Permissions::default();
    for need in needs {
        match oxplow_domain::service::need_scope(need) {
            "ai.compute" | "ai.discover" => p.ai = true,
            "sql.read" => p.sql = Some(oxplow_domain::permissions::SqlAccess::Read),
            "tree.read" => p.files = Some(oxplow_domain::permissions::FilesAccess::Read),
            _ => {}
        }
    }
    p
}

/// Each of `ext`'s contributions that needs a permission, with the
/// manifest line it is declared at and what it is.
fn needed_permissions(
    ext: &Extension,
    files: &dyn ExtensionFiles,
    manifest: &str,
) -> Vec<(Option<usize>, String, Permissions)> {
    let mut out = Vec::new();
    let line = |kind: &str, i: usize| item_lines(manifest, kind).get(i).copied();
    for (i, p) in ext.providers.iter().enumerate() {
        let runs = p.entry.is_some()
            || matches!(
                p.adapter.as_ref().map(|a| &a.mcp),
                Some(crate::providers::spec::McpServer::Command { .. })
            );
        let needed = Permissions {
            process: runs,
            network: p.network.clone(),
            secrets: p.credentials.iter().map(|c| c.name.clone()).collect(),
            ..Permissions::default()
        };
        out.push((line("providers", i), format!("provider `{}`", p.id), needed));
    }
    for (i, c) in ext.collectors.iter().enumerate() {
        // Its declared needs and what its kind implies: a Starlark fact
        // collector reads the project's code (`tree.read`).
        let mut needed = needs_permissions(&c.service_needs());
        if c.runtime == oxplow_config::collectors::CollectorRuntime::Exec
            && c.bundled_parser().is_none()
        {
            needed.process = true;
            needed.network = c.network.clone();
            needed.secrets = c.credentials.clone();
        }
        if c.runtime == oxplow_config::collectors::CollectorRuntime::Starlark
            && c.entry
                .as_deref()
                .and_then(|entry| files.read(entry))
                .is_some_and(|script| oxplow_script::ai::calls_ai(&script))
        {
            needed.ai = true;
        }
        out.push((
            line("collectors", i),
            format!("collector `{}`", c.id),
            needed,
        ));
    }
    for (i, e) in ext.effects.iter().enumerate() {
        out.push((
            line("effects", i),
            format!("effect `{}`", e.id),
            needs_permissions(&e.needs),
        ));
    }
    for (i, j) in ext.jobs.iter().enumerate() {
        out.push((
            line("jobs", i),
            format!("job `{}`", j.id),
            needs_permissions(&j.needs),
        ));
    }
    for (i, c) in ext.commands.iter().enumerate() {
        out.push((
            line("commands", i),
            format!("command `{}`", c.name),
            needs_permissions(&c.needs),
        ));
    }
    for (i, imp) in ext.implementations.iter().enumerate() {
        let mut needed = needs_permissions(&imp.needs);
        needed.ai |= imp.capability == "ai_provider";
        needed.agent |= !imp.skills.is_empty();
        out.push((
            line("implementations", i),
            format!("implementation `{}`", imp.id),
            needed,
        ));
    }
    for (i, a) in ext.advisories.iter().enumerate() {
        let needed = Permissions {
            agent: true,
            ..needs_permissions(&a.needs)
        };
        out.push((
            line("advisories", i),
            format!("advisory `{}`", a.id),
            needed,
        ));
    }
    for (i, skill) in ext.skills.iter().enumerate() {
        let needed = Permissions {
            agent: true,
            ..needs_permissions(&skill.needs)
        };
        out.push((line("skills", i), format!("skill `{}`", skill.name), needed));
    }
    for (i, c) in ext.custom_components.iter().enumerate() {
        let needed = Permissions {
            commands: c
                .commands
                .iter()
                .filter(|cmd| !Permissions::default().command_allowed(&ext.namespace, cmd))
                .cloned()
                .collect(),
            sql: c.sql,
            ..Permissions::default()
        };
        out.push((
            line("custom_components", i),
            format!("component `{}`", c.id),
            needed,
        ));
    }
    out
}

/// Each contribution that needs a permission `ext` doesn't declare is an
/// error at its line, naming what to declare: the extension doesn't load
/// until its manifest says what it reaches.
fn check_permissions(ext: &mut Extension, files: &dyn ExtensionFiles, file: &str, manifest: &str) {
    let fallback = key_line(manifest, "permissions").or(Some(1));
    let errors: Vec<String> = needed_permissions(ext, files, manifest)
        .into_iter()
        .filter_map(|(line, what, needed)| {
            let missing = ext.permissions.missing(&needed);
            (!missing.is_empty()).then(|| {
                at(
                    file,
                    line.or(fallback),
                    format!("{what} needs {}", missing.join(", ")),
                )
            })
        })
        .collect();
    ext.errors.extend(errors);
}

/// The SQL of `ext` that may read its own configuration — a model, a lens, an
/// advisory — with where each is declared.
fn config_sql(ext: &Extension) -> Vec<(String, &str)> {
    let models = ext.models.iter().map(|m| (m.file.clone(), m.sql.as_str()));
    let lenses = ext
        .lenses
        .iter()
        .map(|l| (l.origin.running.clone(), l.query.as_str()));
    let advisories = ext
        .advisories
        .iter()
        .map(|a| (format!("advisory `{}`", a.id), a.query.as_str()));
    models.chain(lenses).chain(advisories).collect()
}

/// A `config('/pointer')` read names a scalar key the extension's own
/// `configuration.schema` declares, or the extension fails to load saying
/// which: an unknown key, a key that is an object or array (no SQL literal), or
/// a read in an extension that declares no configuration.
fn check_config_reads(ext: &mut Extension) {
    let mut problems = Vec::new();
    for (place, sql) in config_sql(ext) {
        let reads = match oxplow_db::models::config_reads(sql) {
            Ok(reads) => reads,
            Err(e) => {
                problems.push(format!("{place}: {e}"));
                continue;
            }
        };
        for read in reads {
            let Some(declaration) = &ext.configuration else {
                problems.push(format!(
                    "{place}: config('{}') reads the extension's own configuration, which it \
                     doesn't declare (`configuration:` in extension.yaml)",
                    read.pointer
                ));
                continue;
            };
            match config_key_type(&declaration.schema, &read.pointer) {
                Some("integer" | "number" | "string" | "boolean") => {}
                Some(other) => problems.push(format!(
                    "{place}: config('{}') is a `{other}` in the configuration schema; \
                     SQL reads a number, string or boolean",
                    read.pointer
                )),
                None => problems.push(format!(
                    "{place}: config('{}') names no key of `configuration.schema` (declare it \
                     under `properties` with a scalar `type`)",
                    read.pointer
                )),
            }
        }
    }
    ext.errors.extend(problems);
}

/// The `type` of the schema property at JSON pointer `pointer`, following
/// `properties`; `None` when the schema declares no such property or type.
fn config_key_type<'a>(schema: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    let mut node = schema;
    for token in pointer.split('/').skip(1) {
        let token = token.replace("~1", "/").replace("~0", "~");
        node = node.get("properties")?.get(&token)?;
    }
    node.get("type")?.as_str()
}

/// Check the manifest's `pages:`: a kebab-case, unique id, a launcher
/// category, and a lens that exists. Each error names the entry's index.
fn parse_pages(
    extension: &str,
    namespace: &str,
    lenses: &[Lens],
    components: &[custom_components::CustomComponent],
    resources: &[crate::extension_resources::ExtensionResource],
    raw: serde_yaml::Value,
) -> (Vec<ExtensionPage>, Vec<(Option<usize>, String)>) {
    let mut pages: Vec<ExtensionPage> = Vec::new();
    let mut errors = Vec::new();
    let Some(list) = raw.as_sequence() else {
        return (pages, vec![(None, "`pages` must be a list".into())]);
    };
    // Oxplow's own pages are core-named, as its commands are.
    let core_named = namespace == CORE_NAMESPACE;
    for (i, v) in list.iter().enumerate() {
        let p = match serde_yaml::from_value::<PageFile>(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                errors.push((Some(i), format!(
                    "pages: {e} (a page is `{{ id, title, icon?, category?, keywords?, params?, count?, for?, accepts?, default?, pinned?, comments?, lens | component }}`; a category is one of Work, Code, Git, Activity, Knowledge, Data, Lenses, System)"
                )));
                continue;
            }
        };
        let err = |e: String| (Some(i), format!("page `{}`: {e}", p.id));
        let body = match (&p.lens, &p.component) {
            (Some(_), Some(_)) | (None, None) => {
                Err("needs exactly one of `lens` and `component`".to_string())
            }
            (Some(lens), None) => {
                if lenses.iter().any(|l| &l.slug == lens) {
                    Ok(PageBody::Lens {
                        lens: format!("{extension}/{lens}"),
                    })
                } else {
                    Err(format!("its lens `{lens}` isn't in lenses/"))
                }
            }
            (None, Some(c)) if c.starts_with("oxplow:") => {
                if !crate::bundled_extensions::is_required(extension) {
                    Err(format!(
                        "`{c}` is a built-in page, which only oxplow's own extension declares; draw it with a lens"
                    ))
                } else {
                    match crate::built_in_pages::built_in(c) {
                        Some(name) => Ok(PageBody::BuiltIn {
                            name: name.to_string(),
                        }),
                        None => Err(format!("`{c}` isn't a built-in page")),
                    }
                }
            }
            (None, Some(c)) => {
                if components.iter().any(|k| &k.id == c) {
                    Ok(PageBody::Component {
                        component: format!("{extension}/{c}"),
                    })
                } else {
                    Err(format!(
                        "`{c}` isn't one of its custom_components or a built-in page \
                         (`oxplow:<name>`)"
                    ))
                }
            }
        };
        if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::Page,
            &p.id,
        ) {
            errors.push(err("a page id is lowercase letters, digits and `-`".into()));
        } else if pages.iter().any(|q| q.id == p.id) {
            errors.push(err("declared twice".into()));
        } else if let Err(e) = &body {
            errors.push(err(e.clone()));
        } else if let Some(c) = p
            .count
            .as_ref()
            .filter(|c| !lenses.iter().any(|l| &l.slug == *c))
        {
            errors.push(err(format!("its count lens `{c}` isn't in lenses/")));
        } else if p
            .icon
            .as_deref()
            .is_some_and(|i| !crate::extension_resources::RESOURCE_ICONS.contains(&i))
        {
            errors.push(err(format!(
                "its icon `{}` isn't one oxplow draws (one of {})",
                p.icon.as_deref().unwrap_or_default(),
                crate::extension_resources::RESOURCE_ICONS.join(", ")
            )));
        } else if let Some(bad) = p.params.iter().find(|n| {
            n.is_empty()
                || !n
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }) {
            errors.push(err(format!(
                "param `{bad}` must be lowercase letters, digits and `_`"
            )));
        } else if let Err(e) = renderer_problem(
            extension,
            namespace,
            resources,
            &p,
            &pages,
            p.lens
                .as_ref()
                .and_then(|l| lenses.iter().find(|x| &x.slug == l)),
        ) {
            errors.push(err(e));
        } else if let Some(lens) = p
            .lens
            .as_ref()
            .filter(|_| p.renders.is_some())
            .and_then(|l| lenses.iter().find(|x| &x.slug == l))
            .filter(|l| !l.params.iter().any(|q| q.name == "uri"))
        {
            // A renderer's lens is run with the URI it shows (`uri`).
            errors.push(err(format!(
                "its lens `{}` renders a resource, so it takes the URI: declare a `uri` param",
                lens.slug
            )));
        } else {
            let renders = p.renders.clone().map(Renders::into_vec).unwrap_or_default();
            let accepts = p
                .accepts
                .iter()
                .map(|(k, v)| {
                    let kept = match v.as_str() {
                        "present" | "absent" => v.clone(),
                        pattern => crate::param_pattern::param_pattern(pattern)
                            .expect("checked by renderer_problem"),
                    };
                    (k.clone(), kept)
                })
                .collect();
            pages.push(ExtensionPage {
                page_ref: if core_named {
                    oxplow_domain::uri::build::page_uri(&p.id, &[])
                } else {
                    oxplow_domain::uri::build::extension_page_uri(extension, &p.id)
                },
                id: p.id,
                extension: extension.to_string(),
                title: p.title,
                label: p.label,
                icon: p.icon,
                category: p.category,
                keywords: p.keywords,
                params: p.params,
                count: p.count.map(|c| format!("{extension}/{c}")),
                renders,
                accepts,
                default: p.default,
                pinned: p.pinned,
                comments: p.comments,
                body: body.expect("checked above"),
            });
        }
    }
    (pages, errors)
}

/// What's wrong with a page's `for`, `accepts`, `default` and `pinned`:
/// each resource it renders is a scheme oxplow renders (`https`), one this
/// extension declares, one of foundation's (`oxplow/*`) or another
/// extension's (`<extension>/<name>`); `accepts` names a param of each
/// (when they're known) with `present`, `absent` or an id pattern; its
/// lens takes no resource param the resource hasn't (it is run with the
/// URI's params);
/// `default` is for a resource of this extension's, one page per resource;
/// `pinned` and `accepts` are a renderer's.
fn renderer_problem(
    extension: &str,
    namespace: &str,
    resources: &[crate::extension_resources::ExtensionResource],
    p: &PageFile,
    pages: &[ExtensionPage],
    lens: Option<&Lens>,
) -> Result<(), String> {
    let renders = p.renders.clone().map(Renders::into_vec).unwrap_or_default();
    // What its lens takes from the URI it is run for: its resource params
    // but the URI itself and the context's.
    let takes: Vec<&str> = lens
        .map(|l| {
            l.params
                .iter()
                .filter(|q| q.role.unwrap_or(ParamRole::Resource) == ParamRole::Resource)
                .map(|q| q.name.as_str())
                .filter(|n| !matches!(*n, "uri" | "stream_id" | "thread_id"))
                .collect()
        })
        .unwrap_or_default();
    let refs = label_refs(p.label.as_deref())?;
    let named: Vec<String> = refs
        .iter()
        .filter(|(from, _)| *from == LabelFrom::Param)
        .map(|(_, n)| n.clone())
        .collect();
    // A row column is read from the one row of a document with a query.
    for (from, name) in &refs {
        if *from != LabelFrom::Row {
            continue;
        }
        let Some(l) = lens.filter(|l| l.viz == LensViz::Document && !l.query.trim().is_empty())
        else {
            return Err(format!(
                "`label` names `row.{name}`, which only a document with a `query:` has"
            ));
        };
        if !l.columns.iter().any(|c| &c.key == name) {
            return Err(format!(
                "`label` names `row.{name}`, which isn't a column of its lens's row (its `columns:`)"
            ));
        }
    }
    if renders.is_empty() {
        if p.default || p.pinned || !p.accepts.is_empty() {
            return Err(
                "`default`, `pinned` and `accepts` are for a renderer: give it `for`".into(),
            );
        }
        if let Some(name) = named.iter().find(|n| !p.params.contains(n)) {
            return Err(format!(
                "`label` names `{name}`, which isn't one of its `params`"
            ));
        }
        return Ok(());
    }
    let segment = crate::extension_resources::resource_extension(extension, namespace);
    let foundation = oxplow_domain::uri::resources::foundation();
    for r in &renders {
        let scheme = RENDERED_SCHEMES.contains(&r.as_str());
        let own = |name: &str| resources.iter().find(|x| x.decl.name.to_string() == name);
        let decl = if scheme {
            None
        } else {
            let Ok(name) = r.parse::<oxplow_domain::uri::oxplow_uri::ResourceName>() else {
                return Err(format!(
                    "`for: {r}` isn't a resource (`<extension>/<name>`) or a scheme oxplow renders ({})",
                    RENDERED_SCHEMES.join(", ")
                ));
            };
            match own(r) {
                Some(x) => Some(&x.decl),
                None if name.extension() == CORE_NAMESPACE => match foundation.get(&name) {
                    Some(d) => Some(d),
                    None => {
                        return Err(format!(
                            "`for: {r}` isn't a resource oxplow-foundation declares"
                        ))
                    }
                },
                None if name.extension() == segment => {
                    return Err(format!(
                        "`for: {r}` isn't one of this extension's `resources:`"
                    ))
                }
                // Another extension's: checked when the vocabulary is.
                None => None,
            }
        };
        let has = |name: &str| match decl {
            Some(d) => d.additional || d.params.contains_key(name),
            None => !scheme,
        };
        if let Some(name) = named.iter().find(|n| !has(n)) {
            return Err(format!(
                "`label` names `{name}`, which isn't a param of `{r}`"
            ));
        }
        if let Some(name) = takes.iter().find(|n| !has(n)) {
            return Err(format!(
                "its lens takes `{name}`, which isn't a param of `{r}`: a renderer's lens is run with its URI's params"
            ));
        }
        for (key, rule) in &p.accepts {
            if let Some(d) = decl {
                if !d.additional && !d.params.contains_key(key) {
                    return Err(format!("`accepts: {key}` isn't a param of `{r}`"));
                }
            }
            if !matches!(rule.as_str(), "present" | "absent") {
                crate::param_pattern::param_pattern(rule).map_err(|e| {
                    format!("`accepts: {key}: {rule}` is `present`, `absent` or an id pattern: {e}")
                })?;
            }
        }
        if p.default {
            let ours = if scheme {
                namespace == CORE_NAMESPACE
            } else {
                r.split('/').next() == Some(segment.as_str())
            };
            if !ours {
                return Err(format!(
                    "`default` is for a resource this extension declares; `{r}` is another's (a page of yours is one of its renderers to pick)"
                ));
            }
            if pages.iter().any(|q| q.default && q.renders.contains(r)) {
                return Err(format!("`{r}` has another default renderer"));
            }
        }
    }
    Ok(())
}

/// Check the manifest's `panels:` against the extension's lenses: a panel
/// id is kebab-case and unique, its lenses exist, a badge declares an
/// `alert`, and a `stream` / `thread` scope's lenses declare `stream_id` /
/// `thread_id` (what the nav binds). Each error names the entry's index.
fn parse_panels(
    extension: &str,
    lenses: &[Lens],
    raw: serde_yaml::Value,
) -> (Vec<ExtensionPanel>, Vec<(Option<usize>, String)>) {
    let mut panels: Vec<ExtensionPanel> = Vec::new();
    let mut errors = Vec::new();
    let Some(list) = raw.as_sequence() else {
        return (panels, vec![(None, "`panels` must be a list".into())]);
    };
    for (i, v) in list.iter().enumerate() {
        let p = match serde_yaml::from_value::<PanelFile>(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                errors.push((Some(i), format!("panels: {e} (a panel is `{{ id, title, icon?, scope: project | stream | thread, body, badge?, open?, collapsed?, count? }}`)")));
                continue;
            }
        };
        let err = |e: String| (Some(i), format!("panel `{}`: {e}", p.id));
        if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::Panel,
            &p.id,
        ) {
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
        if let Some(open) = &p.open {
            let page = oxplow_domain::uri::build::parse_resource(
                open,
                oxplow_domain::uri::resources::PAGE,
            )
            .is_ok();
            if !page {
                errors.push(err(format!(
                    "`open` must be a page URI (`oxplow:oxplow/page?page=<page>`), not `{open}`"
                )));
                ok = false;
            }
        }
        for (role, slug) in [
            ("body", Some(&p.body)),
            ("badge", p.badge.as_ref()),
            ("collapsed", p.collapsed.as_ref()),
            ("count", p.count.as_ref()),
        ] {
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
                open: p.open,
                collapsed: p.collapsed.map(|c| format!("{extension}/{c}")),
                count: p.count.map(|c| format!("{extension}/{c}")),
            });
        }
    }
    (panels, errors)
}

/// Why `lens` can't render with its viz, if it can't.
/// A grid's problem, if any: it names children, and each one in its own
/// extension exists. It needs no `query` — it renders no rows of its own.
/// A form's or a form action's `choices` naming a lens of this extension
/// that isn't there.
fn choices_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    let mine = |c: &&String| {
        c.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str())
            && !ids_in_extension.contains(c)
    };
    lens.form
        .iter()
        .flat_map(|f| f.choices.values())
        .find(mine)
        .map(|c| format!("form: choices lens `{c}` isn't in this extension's lenses/"))
        .or_else(|| {
            lens.form
                .iter()
                .flat_map(|f| f.fields.iter().flatten())
                .filter_map(|field| field.from())
                .map(str::to_string)
                .find(|c| mine(&c))
                .map(|c| format!("form: `from` lens `{c}` isn't in this extension's lenses/"))
        })
        .or_else(|| {
            lens.actions.iter().find_map(|a| {
                a.choices.values().find(mine).map(|c| {
                    format!(
                        "actions: `{}`: choices lens `{c}` isn't in this extension's lenses/",
                        a.id
                    )
                })
            })
        })
}

/// A column's `expand` naming a lens of this extension that isn't there
/// (another extension's resolves with the catalog), or a lens that expands
/// into itself.
fn expand_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    lens.columns.iter().find_map(|c| {
        let target = c.expand.as_deref()?;
        let mine = target.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str());
        if target == lens.id {
            Some(format!(
                "column `{}`: `expand` names the lens itself",
                c.key
            ))
        } else if mine && !ids_in_extension.iter().any(|id| id == target) {
            Some(format!(
                "column `{}`: `expand` lens `{target}` isn't in this extension's lenses/",
                c.key
            ))
        } else {
            None
        }
    })
}

/// A param's `options_from` naming a lens of this extension that isn't
/// there (another extension's resolves with the catalog), or the lens
/// itself.
fn options_from_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    lens.params.iter().find_map(|p| {
        let target = p.options_from.as_deref()?;
        let mine = target.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str());
        if target == lens.id {
            Some(format!(
                "param `{}`: `options_from` names the lens itself",
                p.name
            ))
        } else if mine && !ids_in_extension.iter().any(|id| id == target) {
            Some(format!(
                "param `{}`: `options_from` lens `{target}` isn't in this extension's lenses/",
                p.name
            ))
        } else {
            None
        }
    })
}

/// A composite's `columns:` name the row its `query` returns (what its
/// parts and mounts may take as params): only with a query, and only their
/// `key`s, which is all a row's name has.
fn row_columns_problem(lens: &Lens) -> Option<String> {
    if lens.columns.is_empty() {
        return None;
    }
    if lens.tile.is_some() {
        return Some(
            "a dashboard whose rows are tiles names them in `tile:`, not `columns:`".into(),
        );
    }
    let viz = format!("{:?}", lens.viz).to_lowercase();
    if lens.query.trim().is_empty() {
        return Some(format!("a `{viz}` without a query has no row to name"));
    }
    lens.columns
        .iter()
        .find(|c| {
            c.label.is_some()
                || c.link.is_some()
                || c.unit.is_some()
                || c.icon.is_some()
                || c.tone.is_some()
                || c.width.is_some()
                || c.format.is_some()
                || c.highlight.is_some()
                || c.select.is_some()
                || c.expand.is_some()
        })
        .map(|c| {
            format!(
                "column `{}`: a composite's column is only its `key`: it names a value of its row, not something shown",
                c.key
            )
        })
}

/// A dashboard whose rows are its tiles (`tile:`): only a dashboard's,
/// over a query, each row named by its `id` and drawn by its `lens`, `sql`
/// or `text`; how a `sql` tile shows only with `sql`.
fn tile_problem(lens: &Lens) -> Option<String> {
    let t = lens.tile.as_ref()?;
    if lens.viz != LensViz::Dashboard {
        return Some(format!(
            "`tile` is for viz `dashboard`, not `{}`",
            viz_name(lens.viz)
        ));
    }
    if lens.query.trim().is_empty() {
        return Some("a `dashboard` whose rows are tiles needs a `query`".into());
    }
    if t.id.is_none() {
        return Some("`tile` needs `id: <column>`".into());
    }
    if t.lens.is_none() && t.sql.is_none() && t.text.is_none() {
        return Some("`tile` needs one of `lens`, `sql` or `text`".into());
    }
    if t.sql.is_none() {
        let sql_only = [
            ("display", &t.display),
            ("chart", &t.chart),
            ("metric", &t.metric),
            ("viz", &t.viz),
        ];
        if let Some((role, _)) = sql_only.iter().find(|(_, c)| c.is_some()) {
            return Some(format!("`tile.{role}` is a `sql` tile's"));
        }
    }
    None
}

/// A document's or dashboard's part naming a lens of its own extension
/// that isn't there (another extension's resolves with the catalog).
fn parts_problem(lens: &Lens, ids_in_extension: &[String]) -> Option<String> {
    if lens.part_lenses().any(|c| c == lens.id) {
        return Some("a part names the lens itself; a page can't hold itself".into());
    }
    lens.part_lenses()
        .find(|c| {
            c.split_once('/').map(|(e, _)| e) == Some(lens.extension.as_str())
                && !ids_in_extension.iter().any(|id| id == c)
        })
        .map(|c| format!("part lens `{c}` isn't in this extension's lenses/"))
}

/// What's wrong with a form's `fields`, `mode` and their conditions.
fn form_fields_problem(f: &LensForm) -> Option<String> {
    let Some(fields) = &f.fields else {
        return (f.mode == FormMode::Edit)
            .then(|| "form: `mode: edit` needs `fields`: the ones it edits".to_string());
    };
    let names: Vec<&str> = fields.iter().map(|x| x.name()).collect();
    if let Some(missing) = f
        .again
        .iter()
        .flat_map(|a| &a.keep)
        .find(|k| !names.contains(&k.as_str()))
    {
        return Some(format!(
            "form: `again.keep` names `{missing}`, which isn't one of its `fields`"
        ));
    }
    fields.iter().find_map(|field| {
        let name = field.name();
        if field.secret() && f.fixed.iter().any(|k| k == name) {
            return Some(format!("form: `{name}` can't be both fixed and secret"));
        }
        field.visible_if().keys().find_map(|on| {
            if on == name {
                Some(format!("form: `{name}` can't show by its own value"))
            } else if !names.contains(&on.as_str()) {
                Some(format!(
                    "form: `{name}`'s `visible_if` names `{on}`, which isn't one of its `fields`"
                ))
            } else {
                None
            }
        })
    })
}

/// What's wrong with a lazy tree (`tree.lazy` / `tree.branch`): the param
/// must be the lens's own, with a `default` (the top level) and not one
/// the context supplies; `branch` says which rows open; a column that
/// `select`s the lazy param would re-root the page instead of opening.
fn lazy_tree_problem(lens: &Lens, t: &LensTree) -> Option<String> {
    let Some(lazy) = &t.lazy else {
        return t
            .branch
            .is_some()
            .then(|| "`tree.branch` is for a lazy tree (`tree.lazy: <param>`)".to_string());
    };
    if LensContext::supplies(lazy) {
        return Some(format!(
            "`tree.lazy` can't be `{lazy}`: the context supplies it"
        ));
    }
    let Some(param) = lens.params.iter().find(|p| p.name == *lazy) else {
        return Some(format!(
            "`tree.lazy` names param `{lazy}`, which the lens doesn't declare"
        ));
    };
    if param.default.is_none() {
        return Some(format!(
            "`tree.lazy` param `{lazy}` needs a `default`: the top level"
        ));
    }
    if t.branch.is_none() {
        return Some("`tree.lazy` needs `tree.branch: <column>` (the rows that open)".to_string());
    }
    lens.columns
        .iter()
        .find(|c| c.select.as_deref() == Some(lazy.as_str()))
        .map(|c| {
            format!(
                "column `{}`: `select` names the tree's `lazy` param; a click opens the branch instead",
                c.key
            )
        })
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
    if let Some(problem) = row_style_problem(lens)
        .or_else(|| view_problem(lens))
        .or_else(|| param_problem(lens))
        .or_else(|| highlight_problem(lens))
        .or_else(|| markdown_problem(lens))
        .or_else(|| tile_problem(lens))
    {
        return Some(problem);
    }
    let chart = lens.chart.clone().unwrap_or_default();
    if chart.targets.is_some() && lens.viz != LensViz::Line {
        return Some("chart: `targets` draws target lines on viz `line` only".to_string());
    }
    match lens.viz {
        LensViz::Bar | LensViz::Line => need("chart", &[("x", &chart.x), ("y", &chart.y)]),
        LensViz::Treemap => need("chart", &[("label", &chart.label), ("size", &chart.size)]),
        LensViz::Tree => {
            let t = lens.tree.clone().unwrap_or_default();
            need(
                "tree",
                &[("id", &t.id), ("parent", &t.parent), ("label", &t.label)],
            )
            .or_else(|| lazy_tree_problem(lens, &t))
        }
        LensViz::Timeline => {
            let t = lens.timeline.clone().unwrap_or_default();
            need("timeline", &[("at", &t.at), ("label", &t.label)])
        }
        LensViz::Steps => {
            let s = lens.steps.clone().unwrap_or_default();
            need("steps", &[("label", &s.label)])
        }
        LensViz::Code => {
            let c = lens.code.clone().unwrap_or_default();
            need("code", &[("path", &c.path), ("revision", &c.revision)])
        }
        LensViz::Diff => {
            let d = lens.diff.clone().unwrap_or_default();
            need(
                "diff",
                &[("path", &d.path), ("from", &d.from), ("to", &d.to)],
            )
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
            need("form", &[("command", &f.command)])
                .or_else(|| {
                    let command = f.command.as_deref().unwrap_or_default();
                    oxplow_domain::CommandSpec::validate_id(command)
                        .err()
                        .map(|e| format!("form: {e}"))
                })
                .or_else(|| {
                    f.fixed
                        .iter()
                        .find(|k| {
                            f.defaults
                                .as_ref()
                                .and_then(|d| d.get(k.as_str()))
                                .is_none()
                        })
                        .map(|k| format!("form: fixed field `{k}` needs a value in `defaults`"))
                })
                .or_else(|| form_fields_problem(&f))
        }
        // A grid composes its children; it renders no rows of its own.
        LensViz::Document | LensViz::Dashboard => {
            parts_problem(lens, ids_in_extension).or_else(|| row_columns_problem(lens))
        }
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
        LensViz::Board if lens.group.is_none() => {
            Some("`board` needs `group: { by: <column> }` (its columns)".to_string())
        }
        LensViz::Table
        | LensViz::List
        | LensViz::Number
        | LensViz::Markdown
        | LensViz::Detail
        | LensViz::Board
        | LensViz::Approvals => None,
    }
}

/// A `markdown` block off its viz, or an edit action that doesn't save the
/// text the viz shows: a markdown viz's text, or a row-tile dashboard's
/// text tiles (`tile.text`, edited in place).
fn markdown_problem(lens: &Lens) -> Option<String> {
    let viz = format!("{:?}", lens.viz).to_lowercase();
    if lens.markdown.is_some() && lens.viz != LensViz::Markdown {
        return Some(format!("`markdown` is for viz `markdown`, not `{viz}`"));
    }
    let edit = lens.actions.iter().find(|a| a.edit)?;
    if let Some(text) = lens.tile.as_ref().and_then(|t| t.text.as_deref()) {
        let saved = edit_column(edit)?;
        return (saved != text).then(|| {
            format!(
                "`{}` saves `{saved}`, but the tiles' text is `{text}`",
                edit.id
            )
        });
    }
    if lens.viz != LensViz::Markdown {
        return Some(format!(
            "an edit action (`{}`) is a `markdown` viz's, not `{viz}`'s",
            edit.id
        ));
    }
    let saved = edit_column(edit)?;
    match lens.markdown.as_ref().and_then(|m| m.text.as_deref()) {
        None => Some(format!(
            "`{}` saves the text: give the viz `markdown: {{ text: {saved} }}`",
            edit.id
        )),
        Some(shown) if shown != saved => Some(format!(
            "`{}` saves `{saved}`, but the viz shows `{shown}`",
            edit.id
        )),
        Some(_) => None,
    }
}

/// A column that highlights a param the lens doesn't declare.
fn highlight_problem(lens: &Lens) -> Option<String> {
    lens.columns.iter().find_map(|c| {
        [("highlight", &c.highlight), ("select", &c.select)]
            .into_iter()
            .find_map(|(field, name)| {
                let name = name.as_deref()?;
                (!lens.params.iter().any(|p| p.name == name)).then(|| {
                    format!(
                        "column `{}`: `{field}: {name}` names a param the lens doesn't declare",
                        c.key
                    )
                })
            })
    })
}

/// The most `filter` params a page draws, `choice` options a select
/// lists, characters a description runs, sections a document holds and
/// columns a table shows before the layout warnings say so.
const MAX_FILTERS: usize = 6;
const MAX_CHOICE_OPTIONS: usize = 12;
const MAX_DESCRIPTION: usize = 200;
const MAX_SECTIONS: usize = 8;
const MAX_COLUMNS: usize = 8;

/// What an agent should change about a lens's page so it reads well: a
/// warning each, with the fix after the dash. Never a reason to refuse
/// the lens.
fn layout_warnings(lens: &Lens, lenses: &[Lens], renderer: bool) -> Vec<String> {
    let mut out = Vec::new();
    // The props of a core component this lens stands in for on another
    // lens's page: the host's to give, so neither a filter a person sets
    // nor a param its query must read.
    let contract: Vec<&str> = lenses
        .iter()
        .flat_map(|l| &l.parts)
        .filter(|part| part.lens.as_deref() == Some(lens.id.as_str()))
        .filter_map(|part| oxplow_domain::replaceable::replaceable(part.replaceable.as_deref()?))
        .flat_map(|t| t.props.iter().copied())
        .collect();
    let filters = lens
        .params
        .iter()
        .filter(|p| p.role != Some(ParamRole::Selection) && !contract.contains(&p.name.as_str()))
        // A view document's view param is its section rail, not a filter.
        .filter(|p| lens.view.as_deref() != Some(p.name.as_str()))
        .count();
    if filters > MAX_FILTERS {
        out.push(format!(
            "{filters} filters; a page shows {MAX_FILTERS} at most — fold related ones into one `choice` (not several `bool`s), or split the page"
        ));
    }
    for p in &lens.params {
        if p.kind == Some(ParamType::Choice) && p.options.len() > MAX_CHOICE_OPTIONS {
            out.push(format!(
                "param `{}` has {} options — use a `search`, or a lens that lists them",
                p.name,
                p.options.len()
            ));
        }
    }
    // A form, a custom component and the approvals list take their params
    // by means other than a query.
    if !matches!(
        lens.viz,
        LensViz::Custom | LensViz::Form | LensViz::Approvals
    ) {
        // Its own query and its parts', through parts that are composites
        // themselves (a view document's section is often a document).
        let mut queries: Vec<&str> = vec![lens.query.as_str()];
        let mut seen: Vec<&str> = vec![lens.id.as_str()];
        let mut stack: Vec<&Lens> = vec![lens];
        while let Some(at) = stack.pop() {
            for part in at
                .part_lenses()
                .filter_map(|id| lenses.iter().find(|l| l.id == id))
            {
                if seen.contains(&part.id.as_str()) {
                    continue;
                }
                seen.push(part.id.as_str());
                queries.push(part.query.as_str());
                stack.push(part);
            }
        }
        let templated: Vec<String> = lens
            .actions
            .iter()
            .flat_map(|a| action_templates(&a.input))
            .filter(|(scope, _)| scope == "param")
            .map(|(_, name)| name)
            .collect();
        // A column that selects or highlights a param uses it.
        let marked: Vec<&str> = lens
            .columns
            .iter()
            .flat_map(|c| [c.select.as_deref(), c.highlight.as_deref()])
            .flatten()
            .collect();
        // The metric cards of a dashboard's row tiles window their samples
        // by the page's `range` and `branch`, on the page or a part of it.
        let windowed = std::iter::once(lens)
            .chain(
                lens.part_lenses()
                    .filter_map(|id| lenses.iter().find(|l| l.id == id)),
            )
            .any(|l| l.viz == LensViz::Dashboard && l.tile.is_some());
        for p in &lens.params {
            let read = |token: &str| queries.iter().any(|q| reads_param(q, token));
            let range = p.kind == Some(ParamType::DateRange);
            if !(read(&p.name)
                || lens.view.as_deref() == Some(p.name.as_str())
                || contract.contains(&p.name.as_str())
                || range && (read(&format!("{}_from", p.name)) || read(&format!("{}_to", p.name)))
                || templated.contains(&p.name)
                || marked.contains(&p.name.as_str())
                || windowed && ["range", "branch"].contains(&p.name.as_str())
                // A renderer's lens is run with the URI it shows and the
                // stream and thread it is shown in: the host's to give.
                || renderer && ["uri", "stream_id", "thread_id"].contains(&p.name.as_str()))
            {
                out.push(format!(
                    "param `{0}` isn't read by any query of the page — remove it, or read it as `:{0}`",
                    p.name
                ));
            }
        }
    }
    let described = lens.description.chars().count();
    if described > MAX_DESCRIPTION && !lens.hidden {
        out.push(format!(
            "the description is {described} characters — keep it to a sentence or two; put the rest in a markdown section"
        ));
    }
    // A view document shows one section at a time; the rest are its rail.
    if lens.viz == LensViz::Document && lens.view.is_none() && lens.parts.len() > MAX_SECTIONS {
        out.push(format!(
            "{} sections — a page reads best with {MAX_SECTIONS} at most; split it",
            lens.parts.len()
        ));
    }
    if matches!(lens.viz, LensViz::Table | LensViz::List)
        && lens.tool.is_none()
        && lens.columns.len() > MAX_COLUMNS
    {
        out.push(format!(
            "{} columns — show {MAX_COLUMNS} at most; put the rest in a section a row `select`s",
            lens.columns.len()
        ));
    }
    out
}

/// Whether `query` binds `:name` as a whole token (`:range` is not
/// `:range_from`).
fn reads_param(query: &str, name: &str) -> bool {
    let token = format!(":{name}");
    query.match_indices(&token).any(|(at, _)| {
        !query[at + token.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// A document's or dashboard's own part whose rows select a param the
/// page doesn't declare: the page holds the value its parts share, so a
/// click there would reach nothing.
fn selects_problem(lens: &Lens, lenses: &[Lens]) -> Option<String> {
    lens.part_lenses().find_map(|id| {
        let part = lenses.iter().find(|l| l.id == id)?;
        part.columns
            .iter()
            .filter_map(|c| c.select.as_deref())
            .find(|name| !lens.params.iter().any(|p| &p.name == name))
            .map(|name| format!("part `{id}` selects `{name}`, a param this lens doesn't declare"))
    })
}

/// A part standing in for a core component whose lens (one of this
/// extension's) lacks a prop of the target's contract, or two parts
/// standing in for the same target: the host gives a replacement exactly the
/// contract, so the part's lens must hold all of it.
fn replaceable_problem(lens: &Lens, lenses: &[Lens]) -> Option<String> {
    let mut seen: Vec<&str> = Vec::new();
    for part in &lens.parts {
        let (Some(target), Some(id)) = (part.replaceable.as_deref(), part.lens.as_deref()) else {
            continue;
        };
        if seen.contains(&target) {
            return Some(format!("`{target}` is placed once on a page"));
        }
        seen.push(target);
        let (Some(t), Some(part_lens)) = (
            oxplow_domain::replaceable::replaceable(target),
            lenses.iter().find(|l| l.id == id),
        ) else {
            continue;
        };
        let missing: Vec<String> = t
            .props
            .iter()
            .filter(|p| !part_lens.params.iter().any(|lp| lp.name == **p))
            .map(|p| format!("`{p}`"))
            .collect();
        if !missing.is_empty() {
            return Some(format!(
                "part lens `{id}` must declare {} (`params:`) — what `{target}` gives its replacement",
                missing.join(", ")
            ));
        }
    }
    None
}

impl LensProvenance {
    /// Whether the source changed after the deployment that runs was made
    /// from it: an edit an agent made there runs only once it is
    /// deployed. Read now, from the files; false with no deployment or no
    /// source.
    pub fn out_of_date(&self, project: &Path) -> bool {
        let (Some(source), Some(deployment)) = (&self.source, &self.deployment) else {
            return false;
        };
        let file = project.join(source);
        let Some(dir) = file.parent().and_then(Path::parent) else {
            return false;
        };
        crate::extension_store::source_hash(dir).is_ok_and(|hash| &hash != deployment)
    }

    /// As an agent reads it: these fields and `outOfDate`.
    pub fn to_json(&self, project: &Path) -> serde_json::Value {
        let mut out = serde_json::to_value(self).unwrap_or_default();
        out["outOfDate"] = serde_json::Value::Bool(self.out_of_date(project));
        out
    }
}

/// The params a click on `lens`'s rows sets: its own columns' `select`s
/// and its parts'.
fn selected(lens: &Lens, lenses: &[Lens]) -> Vec<String> {
    let parts = lens
        .part_lenses()
        .filter_map(|id| lenses.iter().find(|l| l.id == id));
    std::iter::once(lens)
        .chain(parts)
        .flat_map(|l| l.columns.iter().filter_map(|c| c.select.clone()))
        .collect()
}

/// Row styling on a viz that doesn't draw rows that way: `group` and
/// `depth` are for `list` / `table`, `emphasis` also for `tree`.
fn row_style_problem(lens: &Lens) -> Option<String> {
    let rows = matches!(lens.viz, LensViz::List | LensViz::Table);
    let viz = format!("{:?}", lens.viz).to_lowercase();
    if lens.group.is_some() && !rows && lens.viz != LensViz::Board {
        return Some(format!(
            "`group` is only for `list`, `table` and `board`, not `{viz}`"
        ));
    }
    if lens.depth.is_some() && !rows {
        return Some(format!(
            "`depth` is only for `list` and `table`, not `{viz}`"
        ));
    }
    if lens.emphasis.is_some() && !rows && lens.viz != LensViz::Tree {
        return Some(format!(
            "`emphasis` is only for `list`, `table` and `tree`, not `{viz}`"
        ));
    }
    None
}

impl Lens {
    /// Every result column the lens's component blocks name (`chart`,
    /// `tree`, `tile`, `timeline`, `steps`, `hunks`) and its row styling
    /// (`group`, `emphasis`, `depth`) — what `validate` checks the query
    /// returns.
    pub fn role_columns(&self) -> Vec<(&'static str, &String)> {
        let mut out: Vec<(&'static str, &String)> = Vec::new();
        if let Some(c) = &self.chart {
            out.extend(c.columns().into_iter().map(|k| ("chart", k)));
        }
        if let Some(t) = &self.tree {
            out.extend(
                [&t.id, &t.parent, &t.label, &t.branch]
                    .into_iter()
                    .flatten()
                    .map(|k| ("tree", k)),
            );
        }
        if let Some(t) = &self.tile {
            out.extend(
                t.roles()
                    .into_iter()
                    .filter_map(|(_, k)| k.as_ref())
                    .map(|k| ("tile", k)),
            );
        }
        if let Some(t) = &self.timeline {
            out.extend(
                [&t.at, &t.label, &t.uri_column]
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
        if let Some(c) = &self.code {
            out.extend(
                [&c.path, &c.revision, &c.lines]
                    .into_iter()
                    .flatten()
                    .map(|k| ("code", k)),
            );
        }
        if let Some(d) = &self.diff {
            out.extend(
                [&d.path, &d.from, &d.to, &d.lines, &d.base]
                    .into_iter()
                    .flatten()
                    .map(|k| ("diff", k)),
            );
        }
        if let Some(t) = self.markdown.as_ref().and_then(|m| m.text.as_ref()) {
            out.push(("markdown", t));
        }
        if let Some(g) = &self.group {
            out.push(("group", &g.by));
            if let Some(l) = &g.link {
                out.extend(
                    [&l.from, &l.line, &l.base, &l.head]
                        .into_iter()
                        .flatten()
                        .map(|k| ("group", k)),
                );
            }
        }
        out.extend(self.emphasis.iter().map(|k| ("emphasis", k)));
        out.extend(self.depth.iter().map(|k| ("depth", k)));
        out
    }
}

/// Clear what a disabled extension contributes; it stays listed so
/// Settings can turn it back on.
fn apply_disabled(mut ext: Extension, settings: &oxplow_config::ExtensionSettings) -> Extension {
    // A listed-only one runs nothing; one oxplow relies on is never off.
    let runs = !matches!(ext.deployment, Deployment::Undeployed | Deployment::NotHere);
    if !runs || is_off(settings, &ext.name) {
        ext.enabled = false;
        clear_contributions(&mut ext);
    }
    ext
}

pub(super) fn clear_contributions(ext: &mut Extension) {
    ext.lenses.clear();
    ext.ui = ExtensionUi::default();
    ext.collectors.clear();
    ext.providers.clear();
    ext.custom_components.clear();
    ext.event_types = Default::default();
    ext.resources.clear();
    ext.effects.clear();
    ext.implementations.clear();
    ext.skills.clear();
    ext.subscriptions.clear();
    ext.advisories.clear();
    ext.measures.clear();
    ext.dimensions.clear();
    ext.metrics.clear();
    ext.models.clear();
    ext.panels.clear();
    ext.pages.clear();
    ext.commands.clear();
    ext.jobs.clear();
    ext.ai_profiles.clear();
    ext.contributions.clear();
}

pub(crate) fn disabled_error(name: &str) -> DomainError {
    DomainError::Invalid(format!(
        "extension `{name}` is disabled (`disableExtensions` in .oxplow/personal.yaml, or left off the project's `extensions` list); enable it in Settings → Extensions"
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LensContext {
    pub stream_id: Option<i64>,
    pub thread_id: Option<i64>,
    /// What's active, to check a lens's needs against; `None` checks
    /// nothing.
    pub active: Option<crate::capabilities::Active>,
}

impl LensContext {
    /// The params the context supplies when the caller doesn't.
    const SUPPLIED: &[&str] = &["stream_id", "thread_id"];

    /// Whether a param named `name` is one the context supplies.
    pub fn supplies(name: &str) -> bool {
        Self::SUPPLIED.contains(&name)
    }

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
        (None, Some(s)) => svc.threads.selected_or_first(&s).await.ok().flatten(),
        (None, None) => None,
    };
    LensContext {
        stream_id: stream.map(|s| s.value()),
        thread_id: thread.map(|t| t.value()),
        active: Some(
            svc.capabilities
                .snapshot(&crate::config_service::read_config(&svc.config)),
        ),
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
    layer
        .read_generation(async { run_lens_unpinned(layer, catalog, root, id, params, ctx).await })
        .await?
}

async fn run_lens_unpinned(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    id: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    let mut lens = catalog.find_lens(root, id)?;
    let mut params = params;
    // Its own extension's settings, as they are stored now.
    if let Some(ext) = catalog
        .unpinned(root, "a lens reads its own extension's settings")
        .iter()
        .find(|e| e.name == lens.extension)
    {
        lens.query = layer.with_extension_config(ext, &lens.query).await?;
        if renders_a_resource(ext, &lens) {
            params = with_uri_params(&lens, params);
        }
    }
    if let Some(active) = &ctx.active {
        let unmet = active.unmet(&lens.needs);
        if !unmet.is_empty() {
            let params = resolve_params(&lens, &params, ctx)?;
            return Ok(LensRun {
                lens,
                params,
                result: SqlQueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    truncated: false,
                    reads: Default::default(),
                    freshness: Vec::new(),
                    generation: None,
                    coverage: Vec::new(),
                },
                alert: None,
                warnings: Vec::new(),
                inactive: Some(LensInactive {
                    message: crate::capabilities::needs_message(&unmet),
                    needs: unmet,
                }),
            });
        }
    }
    let mut run = execute(layer, lens, params, ctx)
        .await
        .map_err(|e| match e {
            DomainError::Invalid(m) => DomainError::Invalid(explain_unsynced(catalog, root, &m)),
            other => other,
        })?;
    run.warnings = disabled_sources(layer, catalog, root, &run.result.reads.models).await;
    Ok(run)
}

/// What a page's `label` fills from: its URI's params, or a column of the
/// one row its lens returns.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LabelFrom {
    Param,
    Row,
}

/// The label filters: `short` is the first 7 characters (a sha),
/// `basename` the last path segment.
const LABEL_FILTERS: &[&str] = &["short", "basename"];

/// What a page's `label` names: each `{{param.<name>}}` or
/// `{{row.<column>}}`, optionally filtered (`{{param.sha|short}}`), spaces
/// allowed inside the braces; any other `{{…}}` is refused.
fn label_refs(label: Option<&str>) -> Result<Vec<(LabelFrom, String)>, String> {
    let mut refs = Vec::new();
    let mut rest = label.unwrap_or_default();
    while let Some(at) = rest.find("{{") {
        let Some(end) = rest[at..].find("}}") else {
            return Err("`label` has a `{{` without its `}}`".into());
        };
        let inner = rest[at + 2..at + end].trim();
        let mut parts = inner.split('|').map(str::trim);
        let head = parts.next().unwrap_or_default();
        for filter in parts {
            if !LABEL_FILTERS.contains(&filter) {
                return Err(format!(
                    "`label` filter `{filter}`: the filters are {}",
                    LABEL_FILTERS
                        .iter()
                        .map(|f| format!("`{f}`"))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ));
            }
        }
        let named = head
            .strip_prefix("param.")
            .map(|n| (LabelFrom::Param, n))
            .or_else(|| head.strip_prefix("row.").map(|n| (LabelFrom::Row, n)));
        match named {
            Some((from, name))
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
            {
                refs.push((from, name.to_string()))
            }
            _ => {
                return Err(format!(
                    "`label` fills only `{{{{param.<name>}}}}` or `{{{{row.<column>}}}}`, not `{{{{{inner}}}}}`"
                ))
            }
        }
        rest = &rest[at + end + 2..];
    }
    Ok(refs)
}

/// The columns of its lens's row a page's `label` reads.
fn label_row_columns(label: Option<&str>) -> Vec<String> {
    label_refs(label)
        .unwrap_or_default()
        .into_iter()
        .filter(|(from, _)| *from == LabelFrom::Row)
        .map(|(_, name)| name)
        .collect()
}

/// Whether `lens` draws one of `ext`'s renderers (a page with `for:`): it
/// is run for a URI, its `uri` param.
fn renders_a_resource(ext: &Extension, lens: &Lens) -> bool {
    ext.pages.iter().any(|p| {
        !p.renders.is_empty() && matches!(&p.body, PageBody::Lens { lens: l } if *l == lens.id)
    })
}

/// A renderer lens's params as run for the URI in its `uri`: each param it
/// declares that the URI carries and the caller didn't supply takes the
/// URI's value (supplied → URI → context → default).
fn with_uri_params(
    lens: &Lens,
    mut params: BTreeMap<String, SqlCell>,
) -> BTreeMap<String, SqlCell> {
    let Some(SqlCell::Text(uri)) = params.get("uri") else {
        return params;
    };
    let Ok(uri) = oxplow_domain::uri::oxplow_uri::OxplowUri::parse(uri) else {
        return params;
    };
    for p in &lens.params {
        if p.name != "uri" && !params.contains_key(&p.name) {
            if let Some(v) = uri.param(&p.name) {
                params.insert(p.name.clone(), SqlCell::Text(v.to_string()));
            }
        }
    }
    params
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
    for ext in catalog
        .unpinned(root, "collectors are matched on their declarations")
        .iter()
    {
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
                    "SELECT reason FROM v_contribution_health
                      WHERE extension = ?1 AND contribution = ?2 AND state = 'disabled'",
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
    for ext in catalog
        .unpinned(root, "collectors are matched on their declarations")
        .iter()
    {
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
/// context (`stream_id` / `thread_id`), else its default, else NULL, each
/// as its type binds it (`LensParam::take`). An unknown supplied name, or
/// a value its param doesn't take, is refused.
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
        let v = match supplied.get(&p.name) {
            Some(v) => p.take(v).map_err(|why| {
                DomainError::Invalid(format!("lens {}: param `{}` {why}", lens.id, p.name))
            })?,
            None => ctx
                .value(&p.name)
                .or_else(|| p.default.as_ref().and_then(|d| p.take(d).ok()))
                .unwrap_or(SqlCell::Null(())),
        };
        params.insert(p.name.clone(), v);
    }
    Ok(params)
}

/// The params a `document`'s or `dashboard`'s parts are given: its query's
/// first row, column by column, under its own params — one of its own
/// wins when it holds a value. Each part takes the ones it declares. A
/// dashboard whose rows are its tiles has no row of params: its own alone.
pub fn part_params(run: &LensRun) -> BTreeMap<String, SqlCell> {
    let mut out = run.params.clone();
    if run.lens.tile.is_some() {
        return out;
    }
    if let Some(row) = run.result.rows.first() {
        for (column, cell) in run.result.columns.iter().zip(row) {
            match out.get(column) {
                Some(v) if !matches!(v, SqlCell::Null(())) => {}
                _ => {
                    out.insert(column.clone(), cell.clone());
                }
            }
        }
    }
    out
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
        // A check reads no stored settings: the declared defaults.
        let mut lens = lens.clone();
        match oxplow_db::models::substitute_config(
            &lens.query,
            ext.default_configuration().as_ref(),
        ) {
            Ok(query) => lens.query = query,
            Err(e) => {
                out.insert(
                    lens.slug.clone(),
                    Err(e.to_string().replacen("invalid value: ", "", 1)),
                );
                continue;
            }
        }
        let slug = lens.slug.clone();
        let run = run_lens_spec(layer, lens)
            .await
            .map_err(|e| e.to_string().replacen("invalid value: ", "", 1));
        out.insert(slug, run);
    }
    out
}

/// The HEAD commit of stream `stream_id`, when it is known.
async fn stream_head(
    layer: &crate::sql_gateway::SqlGateway,
    stream_id: i64,
) -> Result<Option<String>, DomainError> {
    let rows = layer
        .run(
            oxplow_db::SqlQuery::new("SELECT head_sha FROM v_stream WHERE id = :id")
                .named(vec![("id".into(), SqlCell::Int(stream_id))])
                .limit(Some(1)),
        )
        .await?;
    Ok(match rows.rows.first().and_then(|r| r.first()) {
        Some(SqlCell::Text(sha)) => Some(sha.clone()),
        _ => None,
    })
}

async fn execute(
    layer: &crate::sql_gateway::SqlGateway,
    lens: Lens,
    supplied: BTreeMap<String, SqlCell>,
    ctx: &LensContext,
) -> Result<LensRun, DomainError> {
    let mut params = resolve_params(&lens, &supplied, ctx)?;
    // A lens's `:stream_id` is the stream its metrics (`metric_grid()`)
    // are read over.
    let stream = match params.get("stream_id") {
        Some(SqlCell::Int(id)) => Some(*id),
        _ => None,
    };
    // `head_sha`, unless supplied, is the viewed stream's HEAD commit
    // (`v_stream.head_sha`), as `stream_id` is the viewer's stream.
    if let (Some(SqlCell::Null(())), Some(stream_id)) = (params.get("head_sha"), stream) {
        if let Some(head) = stream_head(layer, stream_id).await? {
            params.insert("head_sha".into(), SqlCell::Text(head));
        }
    }
    let named = binds(&lens, &params, time::OffsetDateTime::now_utc());
    // A form without a query has no rows to read.
    let result = if lens.query.trim().is_empty() {
        oxplow_db::SqlQueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            reads: oxplow_db::Reads::default(),
            freshness: Vec::new(),
            generation: None,
            coverage: Vec::new(),
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
        inactive: None,
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
    // A check reads the source whether or not the extension is on: it is
    // how an extension is made ready to deploy.
    let settings = oxplow_config::extension_settings(catalog.project_of(root));
    let mut declared = load_sources(root, &settings);
    let index = declared
        .iter()
        .position(|ext| ext.name == name && ext.origin == "bundled")
        .or_else(|| declared.iter().position(|ext| ext.name == name))
        .ok_or(DomainError::NotFound)?;
    let mut ext = declared.swap_remove(index);
    let path = ext.path.clone();
    let read = |rel: &str| files_at(root, &path).ok().and_then(|f| f.read(rel));
    prepare(layer, catalog, root, &mut ext, &read, commands).await;
    if let Ok(files) = files_at(root, &path) {
        check_fixture_pairing(&mut ext, files.as_ref());
        check_credentials(&mut ext, files.as_ref());
    }
    Ok(ext)
}

/// An extension's manifest, scripts and fixtures, read for what looks like
/// a credential (`credential_lint`): warnings, never the value — a source
/// is one `share` away from a commit.
fn check_credentials(ext: &mut Extension, files: &dyn ExtensionFiles) {
    let Ok(mut paths) = files.paths() else { return };
    paths.sort_unstable();
    let base = ext.path.trim_end_matches('/').to_string();
    for rel in paths {
        let scanned =
            rel == "extension.yaml" || rel.ends_with(".star") || rel.starts_with("fixtures/");
        if !scanned {
            continue;
        }
        if let Some(text) = files.read(&rel) {
            ext.warnings.extend(crate::credential_lint::warnings(
                &format!("{base}/{rel}"),
                &text,
            ));
        }
    }
}

/// A fixture runs only when an intent example names it, so a
/// `fixtures/<name>.yaml` no example lists silently runs nothing (the
/// reverse, an example with no fixture, is `extension test`'s warning: a
/// fresh scaffold has such examples). `model-*` and `provider-*` fixtures
/// belong to a model or a provider, not to an example.
fn check_fixture_pairing(ext: &mut Extension, files: &dyn ExtensionFiles) {
    let Some(intent) = &ext.intent else { return };
    let fixtures = files.list("fixtures");
    let mut orphans: Vec<&str> = fixtures
        .iter()
        .filter_map(|f| f.strip_suffix(".yaml"))
        .filter(|n| !n.starts_with("model-") && !n.starts_with("provider-"))
        .filter(|n| !intent.examples.iter().any(|e| e.name == *n))
        .collect();
    orphans.sort_unstable();
    for name in orphans {
        ext.warnings.push(format!(
            "{}/fixtures/{name}.yaml:1: no intent example names this fixture, so it never runs — \
             fix: add `{name}` to intent.examples",
            ext.path.trim_end_matches('/'),
        ));
    }
}

/// What a check asks the running oxplow's command registry (the
/// `CommandBus`): a command's input schema by name — what an example's or
/// a component's command is checked against — and who holds a
/// command namespace. A plain schema closure is one with no namespaces.
/// `None` where there's no running app to ask (the CLI).
pub trait RunningCommands: Sync {
    fn input_schema(&self, name: &str) -> Option<serde_json::Value>;
    /// Who holds `namespace` (`oxplow`, `oxplow:oxplow/extension?name=<name>`,
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

/// A custom component's declared commands must exist; a `custom` lens
/// that also fills a kit role block gets a nudge — the kit may already
/// render it (the honest extent of a "this reimplements the kit" lint).
fn check_components(
    ext: &mut Extension,
    read: &dyn Fn(&str) -> Option<String>,
    commands: Option<CommandSchemas<'_>>,
) {
    let pages = custom_components::bundle_problems(ext, read);
    ext.errors.extend(pages);
    let warnings = custom_components::bundle_warnings(ext, read);
    ext.warnings.extend(warnings);
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
                ("code", l.code.is_some()),
                ("diff", l.diff.is_some()),
            ]
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(block, _)| (l.origin.running.clone(), block))
        })
        .collect();
    for (path, block) in lookalikes {
        ext.warnings.push(format!(
            "{path}: a custom lens that fills `{block}`: the kit's `{block}` viz may already cover \
             it — prefer the kit where it does"
        ));
    }
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
    candidate: &candidates::CandidateCatalog,
) -> (Vec<oxplow_db::TempView>, Vec<String>) {
    let models = candidate
        .extensions
        .iter()
        .filter(|ext| ext.enabled && !ext.models.is_empty())
        .map(|ext| oxplow_db::models::ExtensionModels {
            extension: ext.name.clone(),
            sources: ext.models.clone(),
            config: ext.default_configuration(),
        })
        .collect();
    let stubs = candidate
        .extensions
        .iter()
        .filter(|ext| ext.enabled)
        .flat_map(entity_stubs)
        .collect();
    match layer.check_extension_models(models, stubs).await {
        Ok(checked) => {
            let target = &candidate.candidate().name;
            let errors = checked
                .errors
                .into_iter()
                .filter(|(owner, _)| candidate.dependents.contains(owner))
                .flat_map(|(owner, errors)| {
                    errors.into_iter().map(move |error| {
                        if &owner == target {
                            error
                        } else {
                            format!("consumer `{owner}`: {error}")
                        }
                    })
                })
                .collect();
            (checked.views, errors)
        }
        Err(e) => (Vec::new(), vec![format!("models: {e}")]),
    }
}

/// A version as a review reads it: its lenses rendered on its own models'
/// overlay, nothing checked — the earlier side of a review is what it was,
/// not a candidate (tsk791).
/// The temp views a lens of extension `name` under `root` reads through:
/// its models and the other enabled extensions' — and their collectors'
/// entities, empty, where none is published — compiled as `check` does
/// and published nowhere. So an author's lens over a model they haven't
/// published yet (a worktree's, or before the next activation) answers
/// rather than "no such table". Empty when the extension doesn't load.
pub async fn lens_overlay(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
) -> Vec<oxplow_db::TempView> {
    let Ok(ext) = catalog.named(root, name) else {
        return Vec::new();
    };
    let candidate = catalog.candidate(root, ext);
    model_overlay(layer, &candidate).await.0
}

async fn read_side(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
) -> Prepared {
    let candidate = catalog.candidate(root, ext.clone());
    *ext = candidate.candidate().clone();
    let (overlay, _) = model_overlay(layer, &candidate).await;
    let lenses = run_lenses(&layer.with_overlay(overlay.clone()), ext).await;
    Prepared { lenses, overlay }
}

/// Check `ext` — its commands, components, models, advisories and lenses —
/// reading through its own models' overlay, and report what's wrong in
/// `ext.errors`. `read` gives the files of the version under check (a
/// component's page is linted as that version has it). Each side of a review prepares its own, so a lens renders
/// against its version's models, whichever are published.
async fn prepare(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
    read: &(dyn Fn(&str) -> Option<String> + Sync),
    commands: Option<CommandSchemas<'_>>,
) -> Prepared {
    let candidate = catalog.candidate(root, ext.clone());
    *ext = candidate.candidate().clone();
    check_components(ext, read, commands);
    let (overlay, errors) = model_overlay(layer, &candidate).await;
    ext.errors.extend(errors);
    let layer = &layer.with_overlay(overlay.clone());
    crate::extension_commands::check_extension_commands(layer, ext, commands).await;
    for a in ext.advisories.clone() {
        // A check reads no stored settings: the declared defaults.
        let query = match oxplow_db::models::substitute_config(
            &a.query,
            ext.default_configuration().as_ref(),
        ) {
            Ok(query) => query,
            Err(e) => {
                ext.errors.push(format!(
                    "advisory {}: {}",
                    a.id,
                    e.to_string().replacen("invalid value: ", "", 1)
                ));
                continue;
            }
        };
        let run = layer
            .run(
                oxplow_db::SqlQuery::new(&query)
                    .named(
                        crate::advisories::PARAMS
                            .iter()
                            .map(|p| (p.to_string(), SqlCell::Null(())))
                            .collect(),
                    )
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
                    if let Some(when) = a.when.as_ref().filter(|w| !cols.contains(w)) {
                        ext.errors.push(format!(
                            "lens {id}: action `{}` is offered `when: {when}`, which isn't in the \
                             query result (columns: {})",
                            a.id,
                            cols.join(", ")
                        ));
                    }
                    for (scope, name) in action_templates(&a.input) {
                        // A drop's neighbours and its order are rows of the
                        // same result.
                        let a_row = matches!(
                            scope.as_str(),
                            "row" | "before" | "after" | "onto" | "order"
                        );
                        if a_row && !cols.contains(&name) {
                            ext.errors.push(format!(
                                "lens {id}: action `{}` uses `{{{{{scope}.{name}}}}}`, which isn't in \
                                 the query result (columns: {})",
                                a.id,
                                cols.join(", ")
                            ));
                        }
                    }
                }
                for c in &run.lens.columns {
                    let mut keys = vec![&c.key];
                    keys.extend(c.unit.iter());
                    keys.extend(c.icon.iter());
                    keys.extend(c.tone.iter());
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

/// One extension by name, read from its source under `root` now, with
/// `project`'s settings saying where that is — for the write paths
/// (`save_lens`), which must see their own result.
fn load_fresh(root: &Path, project: &Path, name: &str) -> Result<Extension, DomainError> {
    let settings = oxplow_config::extension_settings(project);
    let dir = settings.source(root, name);
    if !dir.join("extension.yaml").is_file() {
        return Err(DomainError::NotFound);
    }
    Ok(load_one(
        &Disk(dir.clone()),
        name,
        &shown(root, &dir),
        origin_of(name),
    ))
}

/// A lowercase letter, then lowercase letters, digits and single dashes —
/// safe as a folder name, a lens-id prefix and (dashes as underscores) a
/// command namespace.
pub fn is_valid_name(name: &str) -> bool {
    oxplow_domain::contribution::valid_extension_name(name)
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
    pub code: Option<LensCode>,
    #[serde(default)]
    pub diff: Option<LensDiff>,
    #[serde(default)]
    pub markdown: Option<LensMarkdown>,
    #[serde(default)]
    pub form: Option<LensForm>,
    #[serde(default)]
    pub group: Option<LensGroup>,
    #[serde(default)]
    pub emphasis: Option<String>,
    #[serde(default)]
    pub depth: Option<String>,
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
            params: typed(spec.params.clone()),
            columns: spec.columns.clone(),
            empty: spec.empty.clone(),
            chart: spec.chart.clone(),
            tree: spec.tree.clone(),
            tile: None,
            timeline: spec.timeline.clone(),
            steps: spec.steps.clone(),
            hunks: spec.hunks.clone(),
            code: spec.code.clone(),
            diff: spec.diff.clone(),
            markdown: spec.markdown.clone(),
            form: spec.form.clone(),
            custom: None,
            group: spec.group.clone(),
            emphasis: spec.emphasis.clone(),
            depth: spec.depth.clone(),
            parts: Vec::new(),
            view: None,
            launcher_category: None,
            hidden: false,
            actions: Vec::new(),
            alert: None,
            needs: Vec::new(),
            origin: LensProvenance::default(),
            tool: None,
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
            code: self.code.clone(),
            diff: self.diff.clone(),
            markdown: self.markdown.clone(),
            form: self.form.clone(),
            group: self.group.clone(),
            emphasis: self.emphasis.clone(),
            depth: self.depth.clone(),
        }
    }
}

/// What's wrong with `spec` as a standalone lens, if anything: no title,
/// or a component missing what it needs. A `document` or `dashboard`
/// composes other lenses and a `custom` lens an extension's component, so
/// a standalone spec can't be either; nor a `form`, whose submit runs its command as the
/// person — an answer an agent chose never does.
pub fn spec_problem(spec: &LensSpec) -> Option<String> {
    if spec.title.trim().is_empty() {
        return Some("a lens needs a `title`".into());
    }
    if matches!(spec.viz, LensViz::Document | LensViz::Dashboard) {
        return Some(format!(
            "a `{}` composes lens files; show each lens instead",
            viz_name(spec.viz)
        ));
    }
    if spec.viz == LensViz::Custom {
        return Some(
            "a `custom` lens renders an extension's component; write it as a lens file in a \
             private extension that declares the component"
                .into(),
        );
    }
    if spec.viz == LensViz::Form {
        return Some(
            "a `form` runs its command as whoever submits it, and an answer never runs a \
             command as the person; write the form as a lens file in an extension"
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

/// The source folder of extension `name` in the checkout at `root` (where
/// `project`'s settings say it is), if oxplow may write lens files into it:
/// a valid name, not a bundled extension. The one check every writer of an
/// extension's files (`save_lens`) runs first, so a
/// path or a reserved name never reaches the filesystem.
pub fn writable_extension_dir(
    root: &Path,
    project: &Path,
    name: &str,
) -> Result<PathBuf, DomainError> {
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
    Ok(oxplow_config::extension_settings(project).source(root, name))
}

/// Write `spec` as a new lens file `lenses/<slug>.yaml` in extension
/// `extension`'s source in the checkout at `root` (where `project`'s
/// settings say it is), creating the extension (a v2 manifest whose intent
/// is `origin`) if needed. Refuses a spec with a problem, overwriting a
/// lens, and the bundled extensions. Nothing runs it until it is deployed.
pub fn save_lens(
    root: &Path,
    project: &Path,
    extension: &str,
    slug: &str,
    spec: &LensSpec,
    origin: &LensOrigin<'_>,
) -> Result<Lens, DomainError> {
    let invalid = |m: String| DomainError::Invalid(m);
    let storage = |e: std::io::Error| DomainError::Storage(format!("save lens: {e}"));
    let dir = writable_extension_dir(root, project, extension)?;
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
    let manifest = dir.join("extension.yaml");
    std::fs::create_dir_all(file.parent().unwrap_or(&dir)).map_err(storage)?;
    if !manifest.exists() {
        // The same v2 manifest `oxplow extension new` writes, so a lens saved
        // from Explore Data starts as a checkable extension with an intent.
        std::fs::write(
            &manifest,
            scaffold_manifest(&ManifestScaffold {
                name: extension,
                description: "Lenses kept from answers in your threads",
                purpose: origin.purpose,
                origin: origin.origin,
                example_name: slug,
                example_input: &format!("{{ lens: {slug} }}"),
                example_expect: "the rows the answer showed",
            }),
        )
        .map_err(storage)?;
    }
    let body = lens_file_yaml(spec)?;
    std::fs::write(&file, body).map_err(storage)?;
    let ext = load_fresh(root, project, extension)?;
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

/// `spec` as a lens file, naming its schema on the first line: what it
/// sets, without the empty and absent keys at any depth (the file reads
/// like one written by hand). Built as
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
    let body = serde_yaml::to_string(&prune(value).unwrap_or(serde_yaml::Value::Null))
        .map_err(|e| DomainError::Storage(format!("save lens: {e}")))?;
    Ok(schema::modeline(schema::LENS_SCHEMA_URL) + &body)
}

/// What a scaffolded `extension.yaml` says. One template for
/// `oxplow extension new` and `save_lens`, so every new extension starts
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
}

/// A v2 manifest with an `intent` and one example, naming its schema on the
/// first line.
pub fn scaffold_manifest(m: &ManifestScaffold<'_>) -> String {
    let quote = |s: &str| {
        serde_yaml::to_string(s)
            .expect("string serializes")
            .trim_end()
            .to_string()
    };
    format!(
        "{modeline}manifest: 2\nname: {name}\ndescription: {description}\nintent:\n  purpose: {purpose}\n  origin: {origin}\n  examples:\n    - name: {example_name}\n      input: {input}\n      expect: {expect}\n",
        modeline = schema::modeline(schema::MANIFEST_SCHEMA_URL),
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

    /// A malformed list entry is reported on its own line — the shape
    /// error too, which has no name to find it by — in every list block.
    #[test]
    fn a_broken_list_entry_is_an_error_at_its_own_line() {
        for (block, says) in [
            ("commands:\n  - name: a.b\n    summary: S.\n    entry: h.star\n  - name: c.d\n    bogus: 1\n", "command"),
            ("effects:\n  - id: a\n    summary: S.\n    on: [work_item.state_changed]\n    entry: e.star\n  - id: b\n    bogus: 1\n", "effect"),
            ("collectors:\n  - id: a\n    runtime: starlark\n    entry: c.star\n  - id: b\n    bogus: 1\n", "collector"),
            ("skills:\n  - name: a\n    file: a.md\n  - name: b\n    bogus: 1\n", "skill"),
            ("implementations:\n  - id: a\n    capability: work_items\n  - id: b\n    bogus: 1\n", "implementation"),
            ("pages:\n  - { id: a, title: A, category: Work, lens: x }\n  - id: b\n    bogus: 1\n", "page"),
        ] {
            let d = tempfile::tempdir().unwrap();
            let manifest = format!("manifest: 2\nname: x\nintent:\n  purpose: p\n{block}");
            let second = manifest
                .lines()
                .position(|l| l.contains("bogus"))
                .expect("a broken entry")
                // Its `- ` line is the one before.
                ;
            std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
            std::fs::write(d.path().join(".oxplow/extensions/x/extension.yaml"), &manifest).unwrap();
            let ext = load_extensions(d.path())
                .into_iter()
                .find(|e| e.name == "x")
                .unwrap();
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains(&format!("extension.yaml:{second}:")) && errors.contains("bogus"),
                "{says}: want line {second}\n{errors}"
            );
        }
    }

    /// A broken entry inside the manifest's own nested lists — a slot, an
    /// intent example — is that entry's error: the rest of the extension
    /// still loads.
    #[test]
    fn a_broken_nested_entry_doesnt_sink_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let manifest = "manifest: 2\nname: x\nintent:\n  purpose: p\n  examples:\n    - { name: a }\n    - { name: b, bogus: 1 }\nui:\n  slots:\n    - { slot: diff.file.header, lens: l, bogus: 1 }\nskills:\n  - name: s\n    file: s.md\n";
        std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
        std::fs::write(
            d.path().join(".oxplow/extensions/x/extension.yaml"),
            manifest,
        )
        .unwrap();
        std::fs::write(
            d.path().join(".oxplow/extensions/x/s.md"),
            "---\nname: s\ndescription: S.\n---\n# s\n",
        )
        .unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains("extension.yaml:7:") && errors.contains("bogus"),
            "{errors}"
        );
        assert!(errors.contains("extension.yaml:10:"), "{errors}");
        assert_eq!(ext.skills.len(), 1, "{errors}");
        assert_eq!(
            ext.intent.as_ref().map(|i| i.examples.len()),
            Some(1),
            "the good example stays"
        );
    }

    /// The user guide's hand-written extension loads as written, lens and
    /// all: following it never makes a dead extension.
    #[test]
    fn the_guides_hand_written_extension_loads() {
        let guide = include_str!("../../../docs/guide/lenses.md");
        let section = &guide[guide.find("## Writing one by hand").expect("the section")..];
        let blocks: Vec<&str> = section
            .split("```yaml\n")
            .skip(1)
            .take(2)
            .map(|b| b.split("```").next().unwrap())
            .collect();
        let d = tempfile::tempdir().unwrap();
        for block in blocks {
            let path = block
                .lines()
                .next()
                .unwrap()
                .trim_start_matches("# ")
                .trim();
            let file = d.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, block).unwrap();
        }
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "review")
            .expect("it loads");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(ext.lenses.iter().any(|l| l.slug == "blocked"));
    }

    /// A lens action's placeholders are read by the domain's one
    /// tokenizer (`oxplow_domain::template`), a stray opener included.
    #[test]
    fn action_templates_read_the_one_tokenizer() {
        assert_eq!(
            action_templates(&serde_json::json!({ "t": ["{{param.x}}", "{{{{row.a}}"] })),
            vec![
                ("param".to_string(), "x".to_string()),
                ("{{row".to_string(), "a".to_string())
            ]
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

    /// `when` names a column a row action is offered on; it means nothing
    /// without `row: true`.
    #[test]
    fn when_needs_a_row_action() {
        let parse = |action: &str| {
            parse_actions(
                vec![serde_yaml::from_str(action).unwrap()],
                &[],
                false,
                false,
            )
        };
        assert!(parse("{ id: a, label: A, command: x.y.z, row: true, when: open }").is_ok());
        let err = parse("{ id: a, label: A, command: x.y.z, when: open }").unwrap_err();
        assert!(err.contains("`when` is for a row action"), "{err}");
    }

    /// A lens that holds one row (detail, form, markdown, a document's row)
    /// lets a whole-lens action name `when` and `{{row.*}}`: the lens's
    /// one row. Derived at load as `one_row`, not declared; any other lens
    /// refuses both.
    #[test]
    fn a_whole_lens_action_over_a_one_row_lens_may_name_when_and_the_row() {
        let parse = |action: &str, holds_one_row: bool| {
            parse_actions(
                vec![serde_yaml::from_str(action).unwrap()],
                &[],
                false,
                holds_one_row,
            )
        };
        let named =
            "{ id: a, label: A, command: x.y.z, when: open, input: { t: '{{row.title}}' } }";
        let ok = parse(named, true).unwrap();
        assert!(ok[0].one_row && !ok[0].row);
        let plain = parse("{ id: a, label: A, command: x.y.z }", true).unwrap();
        assert!(!plain[0].one_row, "naming neither is an ordinary button");
        let only_when = parse("{ id: a, label: A, command: x.y.z, when: open }", true).unwrap();
        assert!(only_when[0].one_row);
        let err = parse(named, false).unwrap_err();
        assert!(err.contains("holds one row"), "{err}");
        // A row action stays what it was, on any lens.
        let row = parse(
            "{ id: a, label: A, command: x.y.z, row: true, when: open }",
            false,
        )
        .unwrap();
        assert!(row[0].row && !row[0].one_row);
    }

    /// `primary` marks a row action a click on the row (and Enter) runs: it
    /// is a row action.
    #[test]
    fn primary_is_the_one_row_action_a_click_runs() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                false,
                false,
            )
        };
        let ok = parse(&["{ id: a, label: A, command: x.y.z, row: true, primary: true }"]).unwrap();
        assert!(ok[0].primary);
        let err = parse(&["{ id: a, label: A, command: x.y.z, primary: true }"]).unwrap_err();
        assert!(err.contains("`primary` is for a row action"), "{err}");
    }

    /// A drag action is a row action a drop runs, binding where the row
    /// landed (`{{before.*}}`, `{{after.*}}`, `{{onto.*}}`, `{{group.value}}`);
    /// a bulk action runs per marked row; a key action from the keyboard.
    /// Each refuses what it can't mean.
    #[test]
    fn drag_bulk_and_key_actions_say_what_they_mean() {
        let parse = |actions: &[&str], grouped: bool| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                grouped,
                false,
            )
        };
        let ok = parse(
            &[
                "{ id: move, label: Move, command: x.y.z, drag: true, input: { uri: '{{row.uri}}', to: '{{group.value}}' } }",
                "{ id: put, label: Put, command: x.y.z, drag: true, input: { uri: '{{row.uri}}', before: '{{before.uri}}', after: '{{after.uri}}', parent: '{{onto.uri}}' } }",
                "{ id: done, label: Done, command: x.y.z, row: true, bulk: true, key: f, input: { uri: '{{row.uri}}' } }",
                "{ id: del, label: Delete, command: x.y.z, row: true, key: Delete }",
                "{ id: order, label: Reorder, command: x.y.z, drag: true, input: { order: '{{order.uri}}' } }",
            ],
            true,
        )
        .unwrap();
        assert!(ok[0].drag && ok[0].row, "a drag action is a row action");
        assert!(ok[2].bulk && ok[2].key.as_deref() == Some("f"));
        assert_eq!(ok[3].key.as_deref(), Some("Delete"));

        for (actions, grouped, says) in [
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, form: true }"], true, "a drag action can't be a form"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, primary: true }"], true, "a drag action can't be `primary`"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, input: { b: '{{before.uri}}' } }"], true, "`{{before.uri}}` needs `drag: true`"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, input: { o: '{{order.uri}}' } }"], true, "`{{order.uri}}` needs `drag: true`"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, input: { g: '{{group.value}}' } }"], false, "`{{group.value}}` needs the lens to have a `group`"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, input: { g: '{{group.by}}' } }"], true, "`{{group.by}}` isn't `{{group.value}}`"),
            (vec!["{ id: a, label: A, command: x.y.z, bulk: true }"], true, "`bulk` is for a row action"),
            (vec!["{ id: a, label: A, command: x.y.z, key: f }"], true, "`key` is for a row action"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, key: Enter }"], true, "`Enter` is the kit's"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, key: ff }"], true, "`ff` isn't one key"),
            (
                vec![
                    "{ id: a, label: A, command: x.y.z, row: true, key: f }",
                    "{ id: b, label: B, command: x.y.z, row: true, key: f }",
                ],
                true,
                "key `f` is already `a`'s",
            ),
        ] {
            let err = parse(&actions, grouped).unwrap_err();
            assert!(err.contains(says), "{actions:?}: {err}");
        }
    }

    /// An edit action is the markdown viz's Save: a form over the first
    /// row, its edited field the one bound exactly to a row column and
    /// not fixed. It refuses what can't be an editor's save.
    #[test]
    fn an_edit_action_is_the_save_of_a_markdown_editor() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                true,
                false,
            )
        };
        let ok = parse(&["{ id: save, label: Save, command: x.y.z, edit: true, fixed: [uri], input: { uri: '{{row.uri}}', body: '{{row.body}}' } }"]).unwrap();
        assert!(
            ok[0].edit && ok[0].row && ok[0].form,
            "an edit action is a form over a row"
        );
        assert_eq!(ok[0].edit_field.as_deref(), Some("body"));

        for (actions, says) in [
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, fixed: [uri], input: { uri: '{{row.uri}}' } }"], "`a` edits nothing: bind one input field to `{{row.<the text column>}}`"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, input: { uri: '{{row.uri}}', body: '{{row.body}}' } }"], "`a` edits `uri` and `body`: fix all but one"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, drag: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, bulk: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, primary: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (
                vec![
                    "{ id: a, label: A, command: x.y.z, edit: true, input: { body: '{{row.body}}' } }",
                    "{ id: b, label: B, command: x.y.z, edit: true, input: { body: '{{row.body}}' } }",
                ],
                "`b`: the lens already saves with `a`",
            ),
        ] {
            let err = parse(&actions).unwrap_err();
            assert!(err.contains(says), "{actions:?}: {err}");
        }
    }

    /// A markdown viz names the column it shows; an edit action saves that
    /// column, on a markdown viz only.
    #[test]
    fn a_markdown_editor_saves_the_column_it_shows() {
        let save = "actions: [{ id: save, label: Save, command: x.y.z, edit: true, fixed: [slug], input: { slug: '{{row.slug}}', body: '{{row.body}}' } }]\n";
        let (_d, ext) = load_x(
            &[
                ("ok", &format!("title: O\nquery: SELECT 'a' AS slug, 'hi' AS body\nviz: markdown\nmarkdown: {{ text: body }}\n{save}")),
                ("view", "title: V\nquery: SELECT 'hi' AS body\nviz: markdown\nmarkdown: { text: body }\n"),
                ("notext", &format!("title: N\nquery: SELECT 'a' AS slug, 'hi' AS body\nviz: markdown\n{save}")),
                ("other", &format!("title: T\nquery: SELECT 'a' AS slug, 'hi' AS slug2\nviz: markdown\nmarkdown: {{ text: slug2 }}\n{save}")),
                ("table", &format!("title: T\nquery: SELECT 'a' AS slug, 'hi' AS body\nviz: table\n{save}")),
                ("block", "title: B\nquery: SELECT 'hi' AS body\nviz: table\nmarkdown: { text: body }\n"),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        let ok = lens("ok").expect("ok loads");
        assert_eq!(ok.markdown.as_ref().unwrap().text.as_deref(), Some("body"));
        assert!(lens("view").is_some());
        for slug in ["notext", "other", "table", "block"] {
            assert!(lens(slug).is_none(), "{slug} is refused");
        }
        let errs = ext.errors.join("\n");
        for says in [
            "`save` saves the text: give the viz `markdown: { text: body }`",
            "`save` saves `body`, but the viz shows `slug2`",
            "an edit action (`save`) is a `markdown` viz's",
            "`markdown` is for viz `markdown`, not `table`",
        ] {
            assert!(errs.contains(says), "{says}\n{errs}");
        }
    }

    /// A board draws its groups as columns, so it needs a `group`; its
    /// `values` fix their order.
    #[test]
    fn a_board_needs_a_group_and_its_values_fix_the_order() {
        let (_d, ext) = load_x(
            &[
                ("b", "title: B\nquery: SELECT 'a' AS uri, 'todo' AS state\nviz: board\ngroup: { by: state, values: [todo, done] }\n"),
                ("nob", "title: N\nquery: SELECT 'a' AS uri\nviz: board\n"),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        let b = lens("b").expect("b loads");
        assert_eq!(b.group.as_ref().unwrap().values, vec!["todo", "done"]);
        assert!(lens("nob").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`board` needs `group: { by: <column> }`"),
            "{errs}"
        );
    }

    /// `acme` at `dir` with `manifest` (and the files it names): loaded as
    /// a deployment is, its permission errors alone.
    fn permission_errors(dir: &Path, manifest: &str) -> Vec<String> {
        write(dir, "extension.yaml", manifest);
        load_extension_at(dir, "acme")
            .errors
            .into_iter()
            .filter(|e| e.contains("permissions."))
            .collect()
    }

    fn load_named(root: &Path, exts: &[(&str, &str)]) -> Vec<Extension> {
        for (name, body) in exts {
            let dir = root.join(".oxplow/extensions").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\n{body}"),
            )
            .unwrap();
        }
        load_extensions(root)
    }

    fn errors_of(exts: &[Extension], name: &str) -> String {
        exts.iter()
            .find(|e| e.name == name)
            .unwrap()
            .errors
            .join("\n")
    }

    /// `requires` names other extensions: a bad name or the extension's
    /// own is an error at its line.
    #[test]
    fn a_requires_entry_must_name_another_valid_extension() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("good", "requires: [other]\n"),
                ("bad", "requires: [Not_A_Name]\n"),
                ("selfish", "requires: [selfish]\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "good"), "");
        let good = exts.iter().find(|e| e.name == "good").unwrap();
        assert_eq!(good.requires, vec!["other".to_string()]);
        let bad = errors_of(&exts, "bad");
        assert!(
            bad.contains("extension.yaml:5:") && bad.contains("`Not_A_Name`"),
            "{bad}"
        );
        let selfish = errors_of(&exts, "selfish");
        assert!(
            selfish.contains("extension.yaml:5:") && selfish.contains("itself"),
            "{selfish}"
        );
    }

    /// A `build:` needs a command, and its output and excluded folders are
    /// relative folders inside the extension that leave its manifest alone.
    #[test]
    fn a_build_block_is_checked_for_shape() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                (
                    "good",
                    "build: { run: make, output: dist, exclude: [node_modules] }\n",
                ),
                ("norun", "build: { run: \"\", output: dist }\n"),
                ("escape", "build: { run: make, output: ../dist }\n"),
                ("dot", "build: { run: make, output: . }\n"),
                (
                    "same",
                    "build: { run: make, output: dist, exclude: [dist] }\n",
                ),
                ("unknown", "build: { run: make, output: dist, bogus: 1 }\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "good"), "");
        let good = exts.iter().find(|e| e.name == "good").unwrap();
        assert_eq!(good.build.as_ref().map(|b| b.run.as_str()), Some("make"));
        for (name, says) in [
            ("norun", "build: `run` is empty"),
            (
                "escape",
                "build: `../dist` isn't a folder inside the extension",
            ),
            ("dot", "build: `.` isn't a folder inside the extension"),
            (
                "same",
                "build: `dist` is the output; it can't be excluded too",
            ),
            ("unknown", "bogus"),
        ] {
            let errors = errors_of(&exts, name);
            assert!(
                errors.contains("extension.yaml:5:") && errors.contains(says),
                "{name}: {errors}"
            );
        }
    }

    /// A component bundle under the build's output isn't there until it is
    /// built: the source loads with it taken as declared.
    #[test]
    fn a_bundle_under_the_build_output_loads_as_declared_before_it_is_built() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("built", "build: { run: make, output: dist }\ncustom_components:\n  - { id: w, bundle: dist/w }\n"),
                ("unbuilt", "custom_components:\n  - { id: w, bundle: dist/w }\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "built"), "");
        assert!(!errors_of(&exts, "unbuilt").is_empty());
    }

    /// Two extensions that require each other could never be turned on:
    /// each is an error naming the other.
    #[test]
    fn a_requires_cycle_is_an_error_on_each_member() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("a", "requires: [b]\n"),
                ("b", "requires: [a]\n"),
                ("c", "requires: [a]\n"),
            ],
        );
        let a = errors_of(&exts, "a");
        assert!(a.contains("it requires `b`, which requires it back"), "{a}");
        let b = errors_of(&exts, "b");
        assert!(b.contains("it requires `a`, which requires it back"), "{b}");
        assert_eq!(errors_of(&exts, "c"), "");
    }

    /// A provider runs a program with the person's authority and reaches
    /// its hosts: without `process` and those hosts declared, the
    /// extension doesn't load, and the error is at the provider's line.
    #[test]
    fn a_provider_without_process_permission_is_a_load_error_at_its_line() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "provider.json",
            &serde_json::to_string(&oxplow_provider_fake::declarations()).unwrap(),
        );
        let body = "providers:\n  - id: tracker\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n    network: [api.example.com]\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("extension.yaml:6:")
                && errors[0].contains("`permissions.process`")
                && errors[0].contains("`permissions.network: [api.example.com]`")
                && errors[0].contains("provider `tracker`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: {{ process: true, network: [api.example.com] }}\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A component may invoke its own extension's commands; any other
    /// command a `permissions.commands` pattern must name.
    #[test]
    fn a_component_may_invoke_only_its_own_or_a_declared_command() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "components/board/index.html", "<!doctype html>");
        let body = "custom_components:\n  - { id: board, commands: [oxplow.work_item.transition, acme.item.tidy] }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`permissions.commands: [oxplow.work_item.transition]`")
                && errors[0].contains("component `board`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: {{ commands: [oxplow.work_item.*] }}\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A component that runs its own SQL (`sql: read`) needs
    /// `permissions.sql`, as a script's `sql.read` does.
    #[test]
    fn a_component_that_runs_sql_needs_the_sql_permission() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "components/board/index.html", "<!doctype html>");
        let body = "custom_components:\n  - { id: board, sql: read }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`permissions.sql: read`")
                && errors[0].contains("component `board`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: {{ sql: read }}\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A Starlark collector that asks a model spends the person's key.
    #[test]
    fn an_ai_calling_starlark_collector_needs_ai() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "sum.star",
            "def transform(x):\n    return {\"notes\": [{\"n\": 1, \"s\": ai_summarize(\"x\")}]}\n",
        );
        let body = "collectors:\n  - id: sum\n    runtime: starlark\n    entry: sum.star\n    needs: [ai.compute]\n    entities:\n      - { name: notes, key: n, columns: { n: int, s: text } }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`permissions.ai`") && errors[0].contains("collector `sum`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: {{ ai: true }}\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A collector's `input:` query is declared and reviewable: baseline.
    /// A script's own SQL (`needs: [sql.read]`) is `sql: read`.
    #[test]
    fn a_collector_input_query_is_baseline_but_needs_sql_read_is_not() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "sum.star",
            "def transform(x):\n    return {\"notes\": []}\n",
        );
        let collector = |needs: &str| {
            format!("manifest: 2\nname: acme\nintent:\n  purpose: x\ncollectors:\n  - id: sum\n    runtime: starlark\n    entry: sum.star\n    input: SELECT 1 AS n\n{needs}    entities:\n      - {{ name: notes, key: n, columns: {{ n: int }} }}\n")
        };
        let errors = permission_errors(dir.path(), &collector(""));
        assert!(errors.is_empty(), "{errors:?}");
        let errors = permission_errors(dir.path(), &collector("    needs: [sql.read]\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("`permissions.sql: read`"), "{errors:?}");
    }

    const EXT: &str =
        "manifest: 2\nname: review\nintent:\n  purpose: test\ndescription: Review helpers\n";
    const LENS: &str = r#"
title: Items by state
description: Every work item in a given state.
params:
  - { name: state, label: State, default: in_progress }
query: |
  SELECT uri, title FROM v_work_item WHERE state = :state ORDER BY title
viz: table
columns:
  - { key: title, label: Item, link: { kind: page, from: uri } }
empty: No items.
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );

        let exts = project_extensions(dir.path());
        assert_eq!(exts.len(), 1);
        let e = &exts[0];
        assert_eq!(e.name, "review");
        assert_eq!(e.description, "Review helpers");
        assert_eq!(e.path, ".oxplow/extensions/review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.lenses.len(), 1);
        let l = &e.lenses[0];
        assert_eq!(l.id, "review/by-status");
        assert_eq!(l.slug, "by-status");
        assert_eq!(l.title, "Items by state");
        assert_eq!(l.viz, LensViz::Table);
        assert_eq!(
            l.origin.running,
            ".oxplow/extensions/review/lenses/by-status.yaml"
        );
        assert_eq!(
            l.params[0].default,
            Some(SqlCell::Text("in_progress".into()))
        );
        assert_eq!(
            l.columns[0].link,
            Some(LensLink {
                kind: LensLinkKind::Page,
                from: Some("uri".into()),
                line: None,
                base: None,
                head: None,
            })
        );
    }

    #[test]
    fn a_broken_extension_reports_errors_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/good.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/bad.yaml",
            "title: x\nqueery: SELECT 1\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/nomanifest/lenses/a.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/misnamed/extension.yaml",
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        assert_eq!(
            find_lens(&cat(), dir.path(), "review/by-status")
                .unwrap()
                .title,
            "Items by state"
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/echo.yaml",
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/mine.yaml",
            "title: Mine\nparams:\n  - { name: stream_id }\n  - { name: thread_id, default: 9 }\nquery: SELECT :stream_id AS s, :thread_id AS t\n",
        );
        let sl = layer().await;
        let rows = |run: LensRun| serde_json::to_value(&run.result.rows).unwrap();
        let here = LensContext {
            stream_id: Some(2),
            thread_id: Some(5),
            active: None,
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
            active: Some(
                f.svc
                    .capabilities
                    .snapshot(&crate::config_service::read_config(&f.svc.config)),
            ),
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/broken.yaml",
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

    /// A document's query is one row whose columns its sections take as
    /// params: under the document's own, which win when they hold a value.
    #[test]
    fn a_documents_row_is_its_sections_params() {
        let (_d, ext) = load_x(
            &[(
                "doc",
                "title: D\nparams: [{ name: stream_id }, { name: sha }]\nquery: SELECT 7 AS change_id, 'b' AS sha\nviz: document\nsections: [{ markdown: hi, id: m }]\n",
            )],
            "",
        );
        let lens = ext.lenses.into_iter().find(|l| l.slug == "doc").unwrap();
        let run = |sha: SqlCell, rows: Vec<Vec<SqlCell>>| LensRun {
            lens: lens.clone(),
            params: BTreeMap::from([
                ("stream_id".to_string(), SqlCell::Int(1)),
                ("sha".to_string(), sha),
            ]),
            result: oxplow_db::SqlQueryResult {
                columns: vec!["change_id".into(), "sha".into()],
                rows,
                truncated: false,
                reads: oxplow_db::Reads::default(),
                freshness: Vec::new(),
                generation: None,
                coverage: Vec::new(),
            },
            alert: None,
            warnings: Vec::new(),
            inactive: None,
        };
        let row = vec![vec![SqlCell::Int(7), SqlCell::Text("b".into())]];
        assert_eq!(
            part_params(&run(SqlCell::Text("a".into()), row.clone())),
            BTreeMap::from([
                ("change_id".to_string(), SqlCell::Int(7)),
                ("sha".to_string(), SqlCell::Text("a".into())),
                ("stream_id".to_string(), SqlCell::Int(1)),
            ])
        );
        assert_eq!(
            part_params(&run(SqlCell::Null(()), row))["sha"],
            SqlCell::Text("b".into())
        );
        assert_eq!(
            part_params(&run(SqlCell::Null(()), vec![])).get("change_id"),
            None
        );
    }

    #[tokio::test]
    async fn validate_dry_runs_every_lens() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/broken.yaml",
            "title: Broken\nquery: SELECT nope FROM v_nothing\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/badcol.yaml",
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
            code: None,
            diff: None,
            markdown: None,
            form: None,
            group: None,
            emphasis: None,
            depth: None,
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
            origin: Some("oxplow:oxplow/thread?id=thr1"),
        };
        let lens = save_lens(
            project.path(),
            project.path(),
            "my-lenses",
            "busy",
            &spec,
            &origin,
        )
        .unwrap();
        assert_eq!(lens.spec(), spec);
        let manifest = std::fs::read_to_string(
            project
                .path()
                .join(".oxplow/extensions/my-lenses/extension.yaml"),
        )
        .unwrap();
        assert!(
            manifest.contains("origin: oxplow:oxplow/thread?id=thr1"),
            "{manifest}"
        );
        let bad = LensSpec {
            viz: LensViz::Bar,
            chart: None,
            ..spec.clone()
        };
        let err = save_lens(
            project.path(),
            project.path(),
            "my-lenses",
            "bad",
            &bad,
            &origin,
        )
        .unwrap_err();
        assert!(err.to_string().contains("chart"), "{err}");
        assert!(!project
            .path()
            .join(".oxplow/extensions/my-lenses/lenses/bad.yaml")
            .exists());
    }

    /// A kept spec is never a form: its submit would run the command as
    /// the person with defaults an agent chose.
    #[test]
    fn a_form_spec_isnt_kept() {
        let project = tempfile::tempdir().unwrap();
        let spec = LensSpec {
            title: "New Task".into(),
            viz: LensViz::Form,
            form: Some(LensForm {
                command: Some("oxplow.work_item.create".into()),
                defaults: Some(serde_json::json!({ "title": "x" })),
                ..LensForm::default()
            }),
            ..spec_base()
        };
        let err = save_lens(
            project.path(),
            project.path(),
            "mine",
            "new-task",
            &spec,
            &todo_origin(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("never runs a command as the person"),
            "{err}"
        );
        assert!(!project.path().join(".oxplow/extensions/mine").exists());
    }

    /// What a scaffold or a kept lens writes points an editor at its
    /// schema on the first line.
    #[test]
    fn written_files_name_their_schema() {
        let manifest = scaffold_manifest(&ManifestScaffold {
            name: "mine",
            description: "d",
            purpose: "p",
            origin: None,
            example_name: "e",
            example_input: "{}",
            example_expect: "x",
        });
        assert!(
            manifest.starts_with(&schema::modeline(schema::MANIFEST_SCHEMA_URL)),
            "{manifest}"
        );
        let project = tempfile::tempdir().unwrap();
        let spec = LensSpec {
            title: "Open Tasks".into(),
            query: "SELECT uri, title FROM v_work_item".into(),
            ..spec_base()
        };
        save_lens(
            project.path(),
            project.path(),
            "mine",
            "open-tasks",
            &spec,
            &todo_origin(),
        )
        .unwrap();
        let lens = fs::read_to_string(
            project
                .path()
                .join(".oxplow/extensions/mine/lenses/open-tasks.yaml"),
        )
        .unwrap();
        assert!(
            lens.starts_with(&schema::modeline(schema::LENS_SCHEMA_URL)),
            "{lens}"
        );
    }

    #[test]
    fn save_lens_creates_the_extension_and_refuses_overwrites() {
        let project = tempfile::tempdir().unwrap();
        let new = || LensSpec {
            title: "Open Tasks".into(),
            description: "From Explore Data".into(),
            query: "SELECT uri, title FROM v_work_item".into(),
            ..spec_base()
        };
        let lens = save_lens(
            project.path(),
            project.path(),
            "mine",
            "open-tasks",
            &new(),
            &todo_origin(),
        )
        .unwrap();
        assert_eq!(lens.id, "mine/open-tasks");
        assert_eq!(lens.title, "Open Tasks");
        assert!(project
            .path()
            .join(".oxplow/extensions/mine/extension.yaml")
            .is_file());
        let ext = &project_extensions(project.path())[0];
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);

        let err = save_lens(
            project.path(),
            project.path(),
            "mine",
            "open-tasks",
            &new(),
            &todo_origin(),
        )
        .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("already exists")),
            "{err:?}"
        );
        let err = save_lens(
            project.path(),
            project.path(),
            "mine",
            "Bad Slug",
            &new(),
            &todo_origin(),
        )
        .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("slug")),
            "{err:?}"
        );
    }

    #[test]
    fn loads_declared_sources_and_reports_bad_ones_without_dropping_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            "manifest: 2\nname: review\nintent:\n  purpose: test\npermissions: { process: true }\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: bin/sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n  - id: bad\n    runtime: python\n    entry: x\n    entities: []\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/bin/sync.sh",
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
            ".oxplow/extensions/gh/extension.yaml",
            "manifest: 2\nname: gh\nintent:\n  purpose: test\npermissions: { process: true }\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(dir.path(), ".oxplow/extensions/gh/sync.sh", "#!/bin/sh\n");
        write(
            dir.path(),
            ".oxplow/extensions/gh/lenses/all.yaml",
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
            ".oxplow/extensions/gh/extension.yaml",
            "manifest: 2\nname: gh\nintent:\n  purpose: test\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(dir.path(), ".oxplow/extensions/gh/sync.sh", "#!/bin/sh\n");
        write(
            dir.path(),
            ".oxplow/extensions/gh/lenses/all.yaml",
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
            oxplow_db::contribution_health_store::disable_tx(
                tx,
                &crate::collector_runner::contribution_key("gh", "prs"),
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
            ".oxplow/extensions/review/extension.yaml",
            "manifest: 2\nname: review\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        let e = only(dir.path(), "review");
        assert_eq!(e.manifest_version, 2);
        assert!(
            e.errors.iter().any(
                |m| m.starts_with(".oxplow/extensions/review/extension.yaml:1:")
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
            ".oxplow/extensions/review/extension.yaml",
            EXT_V2,
        );
        let e = only(dir.path(), "review");
        assert!(
            e.errors.is_empty() && e.warnings.is_empty(),
            "{:?} {:?}",
            e.errors,
            e.warnings
        );
        assert_eq!(e.intent.as_ref().unwrap().purpose, "Review agent work");
    }

    #[test]
    fn a_slot_mount_to_a_missing_lens_names_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!("{EXT_V2}ui:\n  slots:\n    - {{ slot: rail, lens: tasks }}\n    - {{ slot: diff.file.header, lens: nope }}\n");
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        let e = only(dir.path(), "review");
        let err = e
            .errors
            .iter()
            .find(|m| m.contains("mounts lens `nope`"))
            .unwrap_or_else(|| panic!("{:?}", e.errors));
        assert!(
            err.starts_with(".oxplow/extensions/review/extension.yaml:11:"),
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
            ".oxplow/extensions/review/extension.yaml",
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
            ".oxplow/extensions/review/extension.yaml",
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
            ".oxplow/extensions/review/extension.yaml",
            EXT_V2,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/extensions/board/extension.yaml",
            "manifest: 2\nname: board\nintent:\n  purpose: x\n  examples: [{ name: a }]\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/board/lenses/all.yaml",
            "title: All\nviz: document\nsections: [{ lens: review/tasks }, { lens: review/missing }]\n",
        );
        let board = only(dir.path(), "board");
        assert!(
            board
                .errors
                .iter()
                .any(|m| m.contains("part lens `review/missing` isn't in any loaded extension")),
            "{:?}",
            board.errors
        );
        assert!(board.lenses.is_empty(), "the broken document is dropped");
    }

    /// The tally extension: a starlark collector declaring entity `thing`,
    /// a model over it, lenses over both, and a command reading the model.
    fn write_tally(root: &Path, script: &str) {
        write(
            root,
            ".oxplow/extensions/tally/extension.yaml",
            "manifest: 2
name: tally
intent: { purpose: x, examples: [{ name: a }] }
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT uri FROM v_work_item\"
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
  - name: thing.note
    summary: Note a thing.
    input_schema: { type: object, required: [id], properties: { id: { type: integer } } }
    entry: handlers/note.star
",
        );
        write(
            root,
            ".oxplow/extensions/tally/collectors/things.star",
            script,
        );
        write(
            root,
            ".oxplow/extensions/tally/models/labelled.sql",
            "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": []}\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/lenses/labelled.yaml",
            "title: Labelled\nquery: SELECT id, label FROM v_tally_labelled\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/lenses/things.yaml",
            "title: Things\nquery: SELECT id, label FROM v_tally_thing\n",
        );
    }

    /// P7.C6: a check sees what the extension declares before it ever ran
    /// or published — its collectors' entities (empty stand-ins) and its
    /// own models (compiled for the check) — from its lenses (and its
    /// commands' examples' reads), on a database that has neither.
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
            ".oxplow/extensions/tally/lenses/labelled.yaml",
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

    /// An effect or a command may declare `needs: [ai.discover]`: it loads
    /// and its script can ask whether a model serves a role.
    #[test]
    fn a_script_can_declare_ai_discover() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".oxplow/extensions/x");
        std::fs::create_dir_all(dir.join("effects")).unwrap();
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\npermissions: { ai: true }\neffects:\n  - { id: e, summary: s, on: [effort.finished], entry: effects/e.star, needs: [ai.discover] }\ncommands:\n  - { name: c.go, summary: s, entry: commands/c.star, needs: [ai.discover], input_schema: { type: object, additionalProperties: false } }\n",
        )
        .unwrap();
        let script = "def transform(x):\n    r = scope(\"ai.discover\", {\"role\": \"summarize\"})\n    if not r[\"model\"]:\n        return {\"skip\": r[\"problem\"]}\n    return {\"commands\": []}\n";
        std::fs::write(dir.join("effects/e.star"), script).unwrap();
        std::fs::write(dir.join("commands/c.star"), script).unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        assert_eq!(ext.errors, Vec::<String>::new());
        assert_eq!(ext.effects.len(), 1);
        assert_eq!(ext.commands.len(), 1);
    }

    /// `ai_profiles:` load into the extension, each as a contribution
    /// `<ext>/ai_profile/<id>`, and a mistake is an error at its line.
    #[test]
    fn ai_profiles_load_and_are_described() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
        let manifest = "manifest: 2\nname: x\nintent:\n  purpose: p\nai_profiles:\n  - { id: review, description: Reviews., fallback: summarize, features: [tool_calling] }\n  - { id: bad, description: x, fallback: huge }\n";
        std::fs::write(
            d.path().join(".oxplow/extensions/x/extension.yaml"),
            manifest,
        )
        .unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        assert_eq!(ext.ai_profiles.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.ai_profiles[0].id, "review");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("extension.yaml:7:") && e.contains("`fallback` is `main`")),
            "{:?}",
            ext.errors
        );
        let described: Vec<String> = ext
            .contributions
            .iter()
            .map(|c| c.id.to_string())
            .filter(|id| id.contains("ai_profile"))
            .collect();
        assert_eq!(described, ["oxplow:oxplow/ai_profile?ai_profile=x/review"]);
    }

    /// Every command an agent is offered — core's, oxplow-foundation's and
    /// oxplow-bundled's — declares an example input beside its schema, and the
    /// example fits: an agent reads a whole right input instead of guessing a
    /// shape field by field (`.context/commands.md` "Exposure to agents").
    #[tokio::test(flavor = "multi_thread")]
    async fn every_command_an_agent_is_offered_has_an_example_that_fits() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let services = crate::Services::in_memory(dir.path()).await.unwrap();
        let offered = services.commands.list(&oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(7)),
            stream_id: None,
        });
        assert!(offered.len() > 40, "{}", offered.len());
        let mut missing = Vec::new();
        let mut misfit = Vec::new();
        for spec in &offered {
            match oxplow_domain::commands::schema_example(&spec.input_schema) {
                None => missing.push(spec.id.clone()),
                Some(example) => {
                    let validator =
                        oxplow_domain::InputValidator::compile(&spec.input_schema).unwrap();
                    if let Err(err) = validator.check(example) {
                        misfit.push(format!("{}: {err}", spec.id));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "no example: {missing:#?}");
        assert!(misfit.is_empty(), "example doesn't fit: {misfit:#?}");
    }

    /// Oxplow ships two extensions: `oxplow-foundation`, its own commands
    /// over the scopes' operations, and `oxplow-bundled`: the
    /// review packet, the analytics lenses and the agent's advisories
    /// together. They load clean and every lens's SQL runs.
    #[tokio::test]
    async fn the_bundled_extensions_ship_and_load_clean() {
        let dir = tempfile::tempdir().unwrap();
        let exts = load_extensions(dir.path());
        let bundled: Vec<&Extension> = exts.iter().filter(|e| e.origin == "bundled").collect();
        assert_eq!(
            bundled.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["oxplow-bundled", "oxplow-foundation"]
        );
        let foundation = bundled[1];
        assert!(foundation.errors.is_empty(), "{:?}", foundation.errors);
        assert!(foundation.warnings.is_empty(), "{:?}", foundation.warnings);
        assert_eq!(foundation.namespace, "oxplow");
        assert!(foundation.commands.iter().all(|c| matches!(
            c.handler,
            crate::extension_commands::CommandHandler::Scope { .. }
        )));
        // The agent harnesses and AI providers core runs are
        // foundation's declarations — many of each, all active.
        let declared = |capability: &str| {
            foundation
                .implementations
                .iter()
                .filter(|d| d.capability == capability)
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(declared("agent_harness"), ["claude", "codex", "opencode"]);
        assert_eq!(
            declared("ai_provider"),
            [
                "anthropic",
                "openai",
                "openai_compatible",
                "openrouter",
                "typesafe"
            ]
        );
        let b = bundled[0];
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        // Its manifest is v2, with an intent, and clean.
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.manifest_version, 2);
        assert!(!b.intent.as_ref().unwrap().examples.is_empty());
        // The review packet and the analytics lenses, mounted together.
        for (slot, lens) in [
            (
                "oxplow-foundation/changes.sections",
                "oxplow-bundled/decisions",
            ),
            (
                "oxplow-foundation/changes.sections",
                "oxplow-bundled/inferred-decisions",
            ),
            (
                "oxplow-foundation/changes.sections",
                "oxplow-bundled/change-review",
            ),
            (
                "oxplow-foundation/changes.sections",
                "oxplow-bundled/effort-tests",
            ),
            (
                "oxplow-foundation/commit.sections",
                "oxplow-bundled/tests-weakened",
            ),
            (
                "oxplow-foundation/work-item.sections",
                "oxplow-bundled/task-tokens",
            ),
            ("diff.file.header", "oxplow-bundled/file-co-change"),
        ] {
            assert!(
                b.ui.slots
                    .iter()
                    .any(|s| s.slot == slot && s.lens_id == lens),
                "{slot} {lens}"
            );
        }
        assert!(b.panels.iter().any(|p| p.id == "oxplow-bundled/waiting"));
        let commands: Vec<&str> = b.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            commands,
            [
                "oxplow.review_policy.read",
                "oxplow.review_policy.write",
                "oxplow.reasoning.ask",
                "oxplow.reasoning.status",
                "oxplow.reasoning.cancel",
                "oxplow.effort.infer_reasoning",
                "oxplow.review.accept",
                "oxplow.review.request_changes",
                "oxplow.work.clear_finished"
            ]
        );
        let advisories: Vec<&str> = b.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            advisories,
            [
                "large-uncommitted",
                "landed-in-progress",
                "metric-deltas",
                "threshold-crossed",
                "hint-never-fires",
                "shared-files",
                "shared-files-agent"
            ]
        );
        let mut models: Vec<&str> = b.models.iter().map(|m| m.decl.name.as_str()).collect();
        models.sort();
        assert_eq!(
            models,
            [
                "change_co_change",
                "change_interest",
                "claim_status",
                "co_change_pair",
                "deviation",
                "finished_cleared",
                "struggle",
                "thread_work",
                "verdict",
                "verdicts"
            ]
        );
        let effects: Vec<&str> = b.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            effects,
            [
                "verify-unchecked",
                "repair-disabled",
                "infer-reasoning",
                "store-reasoning"
            ]
        );
        assert!(b
            .collectors
            .iter()
            .any(|c| c.id == "oxplow_bundled.effort_churn"));
        // Every bundled lens's SQL and advisory runs against a real schema —
        // its own models compiled for the check, nothing published.
        let v = validate_extension(&layer().await, &cat(), dir.path(), "oxplow-bundled", None)
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(
            find_lens(&cat(), dir.path(), "oxplow-bundled/decisions")
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
            ".oxplow/extensions/oxplow-bundled/extension.yaml",
            "manifest: 2\nname: oxplow-bundled\nintent:\n  purpose: test\n",
        );
        let exts = load_extensions(dir.path());
        let named: Vec<&Extension> = exts.iter().filter(|e| e.name == "oxplow-bundled").collect();
        // A source under a bundled name is never loaded: the bundled one
        // is the only one.
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].origin, "bundled");
        let err = save_lens(
            dir.path(),
            dir.path(),
            "oxplow-bundled",
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
            ".oxplow/extensions/mine/extension.yaml",
            "manifest: 2\nname: mine\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: diff.file.header, lens: nope }\n    - { slot: sidebar, lens: a }\n    - { slot: diff.file.header, lens: a }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/mine/lenses/a.yaml",
            "title: A\nparams: [{ name: path }]\nquery: SELECT :path\n",
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

    /// A Settings area's sections take an extension's parameterless lens,
    /// placed beside one of the area's own sections.
    #[test]
    fn a_settings_area_takes_parameterless_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/mine/extension.yaml",
            "manifest: 2\nname: mine\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: settings.integrations.sections, lens: status, at: { after: integrations } }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/mine/lenses/status.yaml",
            "title: Status\nquery: SELECT 1\n",
        );
        let e = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "mine")
            .unwrap();
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert!(
            e.warnings.iter().all(|w| !w.contains("has no part")),
            "{:?}",
            e.warnings
        );
        assert_eq!(e.ui.slots[0].slot, "settings.integrations.sections");
    }

    /// A document's sections are lenses or markdown: an id defaults to the
    /// lens's slug (a lens shown twice needs an `id`), markdown needs one,
    /// ids are unique, a section has no
    /// size, and `tiles` are a dashboard's.
    #[test]
    fn document_sections_parse_and_bad_ones_are_errors() {
        let (_d, ext) = load_x(
            &[
                ("n", "title: N\nquery: SELECT 1 AS n\nviz: number\n"),
                (
                    "doc",
                    "title: D\nviz: document\nsections:\n  - { id: intro, title: About, markdown: Hello }\n  - { lens: n }\n  - { lens: x/n, id: again }\n",
                ),
                ("md-no-id", "title: D\nviz: document\nsections: [{ markdown: hi }]\n"),
                ("both", "title: D\nviz: document\nsections: [{ lens: n, markdown: hi }]\n"),
                ("twice", "title: D\nviz: document\nsections: [{ lens: n }, { lens: n }]\n"),
                ("sized", "title: D\nviz: document\nsections: [{ lens: n, size: wide }]\n"),
                ("tiled", "title: D\nviz: document\ntiles: [{ lens: n }]\n"),
                ("empty", "title: D\nviz: document\n"),
                ("stray", "title: T\nquery: SELECT 1\nsections: [{ lens: n }]\n"),
            ],
            "",
        );
        let doc = ext.lenses.iter().find(|l| l.slug == "doc").unwrap();
        let parts: Vec<_> = doc
            .parts
            .iter()
            .map(|p| (p.id.as_str(), p.lens.as_deref(), p.markdown.as_deref()))
            .collect();
        assert_eq!(
            parts,
            vec![
                ("intro", None, Some("Hello")),
                ("n", Some("x/n"), None),
                ("again", Some("x/n"), None),
            ]
        );
        let errs = ext.errors.join("\n");
        for (slug, want) in [
            ("md-no-id", "needs an `id`"),
            ("both", "exactly one of `lens` and `markdown`"),
            ("twice", "two parts are `n`"),
            ("sized", "`size` is a dashboard tile's"),
            ("tiled", "has `sections`, not `tiles`"),
            ("empty", "needs `sections: [...]`"),
            ("stray", "`sections` are a `document`'s"),
        ] {
            assert!(
                errs.lines().any(|l| l.contains(slug) && l.contains(want)),
                "{slug}: {errs}"
            );
            assert!(!ext.lenses.iter().any(|l| l.slug == slug), "{slug}");
        }
    }

    /// A part may declare the core component it stands in for
    /// (`replaceable: <target>`): its own lens is oxplow's, shown unless an
    /// extension replaces the target. The target must exist, the part must
    /// be a lens, and that lens must declare every prop of the target's
    /// contract, as a replacement lens must.
    #[test]
    fn a_replaceable_part_names_a_target_whose_props_its_lens_declares() {
        let (_d, ext) = load_x(
            &[
                (
                    "log",
                    "title: L\nparams: [{ name: head_sha }, { name: reveal_sha, default: \"\" }, { name: limit, default: 500 }]\nquery: SELECT 1\n",
                ),
                ("short", "title: S\nparams: [{ name: head_sha }]\nquery: SELECT 1\n"),
                ("doc", "title: D\nviz: document\nsections: [{ lens: log, replaceable: vcs.history.graph }]\n"),
                ("doc-short", "title: D\nviz: document\nsections: [{ lens: short, replaceable: vcs.history.graph }]\n"),
                ("doc-unknown", "title: D\nviz: document\nsections: [{ lens: log, replaceable: nope.nothing }]\n"),
                ("doc-md", "title: D\nviz: document\nsections: [{ id: m, markdown: hi, replaceable: vcs.history.graph }]\n"),
                ("doc-twice", "title: D\nviz: document\nsections: [{ lens: log, replaceable: vcs.history.graph }, { id: again, lens: log, replaceable: vcs.history.graph }]\n"),
            ],
            "",
        );
        let doc = ext.lenses.iter().find(|l| l.slug == "doc").unwrap();
        assert_eq!(
            doc.parts[0].replaceable.as_deref(),
            Some("vcs.history.graph")
        );
        let errs = ext.errors.join("\n");
        for (slug, want) in [
            ("doc-short", "must declare `reveal_sha`, `limit`"),
            ("doc-unknown", "isn't a replaceable component"),
            ("doc-md", "`replaceable` is a lens part's"),
            ("doc-twice", "`vcs.history.graph` is placed once"),
        ] {
            assert!(
                errs.lines().any(|l| l.contains(slug) && l.contains(want)),
                "{slug}: {errs}"
            );
            assert!(!ext.lenses.iter().any(|l| l.slug == slug), "{slug}");
        }
    }

    /// A page's sections are a slot: a mount says where among them it
    /// goes; a region takes no `at`, only tiles take a `size`, and an `at`
    /// beside a part the page doesn't have is a warning (it goes last).
    #[test]
    fn a_parts_slot_places_its_mounts() {
        let (_d, ext) = load_x(
            &[
                ("presets", "title: Presets\nquery: SELECT 1\n"),
                ("stray", "title: Stray\nquery: SELECT 1\n"),
                ("region", "title: R\nparams: [{ name: path }]\nquery: SELECT 1\n"),
                ("sized", "title: S\nquery: SELECT 1\n"),
            ],
            "ui:\n  slots:\n    - { slot: settings.integrations.sections, lens: presets, title: Team presets, at: { after: integrations } }\n    - { slot: settings.integrations.sections, lens: stray, at: { before: gone } }\n    - { slot: diff.file.header, lens: region, at: start }\n    - { slot: settings.integrations.sections, lens: sized, size: wide }\n",
        );
        let mounts: Vec<_> = ext
            .ui
            .slots
            .iter()
            .map(|m| (m.lens_id.as_str(), m.at.clone(), m.title.as_deref()))
            .collect();
        assert_eq!(
            mounts,
            vec![
                (
                    "x/presets",
                    Some(SlotAt::After {
                        after: "integrations".into()
                    }),
                    Some("Team presets")
                ),
                (
                    "x/stray",
                    Some(SlotAt::Before {
                        before: "gone".into()
                    }),
                    None
                ),
            ]
        );
        let errs = ext.errors.join("\n");
        assert!(errs.contains("is one region of its page"), "{errs}");
        assert!(errs.contains("`size` is a dashboard tile's"), "{errs}");
        let warns = ext.warnings.join("\n");
        assert!(
            warns.contains("has no part `gone` (it has `integrations`)"),
            "{warns}"
        );
    }

    /// Another extension adds to a document lens through its sections slot;
    /// the lens's parts then hold it, placed by its `at` and marked with
    /// who mounted it. A mount naming no document is an error and goes.
    #[test]
    fn an_extension_adds_a_section_to_another_extensions_document() {
        let dir = tempfile::tempdir().unwrap();
        let ext_yaml = |name: &str, tail: &str| {
            format!("manifest: 2\nname: {name}\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\n{tail}")
        };
        write(
            dir.path(),
            ".oxplow/extensions/base/extension.yaml",
            &ext_yaml("base", ""),
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/lenses/a.yaml",
            "title: A\nparams: [{ name: stream_id }]\nquery: SELECT 1\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/lenses/b.yaml",
            "title: B\nquery: SELECT 1\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/lenses/page.yaml",
            "title: Page\nparams: [{ name: stream_id }]\nviz: document\nsections: [{ lens: a }, { lens: b }]\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/extension.yaml",
            &ext_yaml(
                "addon",
                "ui:\n  slots:\n    - { slot: base/page.sections, lens: extra, at: { after: a } }\n    - { slot: base/a.sections, lens: extra }\n    - { slot: base/page.tiles, lens: extra }\n",
            ),
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/lenses/extra.yaml",
            "title: Extra\nparams: [{ name: stream_id }]\nquery: SELECT 1\n",
        );
        let base = only(dir.path(), "base");
        let page = base.lenses.iter().find(|l| l.slug == "page").unwrap();
        let parts: Vec<_> = page
            .parts
            .iter()
            .map(|p| (p.id.as_str(), p.from.as_deref()))
            .collect();
        assert_eq!(
            parts,
            vec![("a", None), ("addon/extra", Some("addon")), ("b", None)]
        );
        let addon = only(dir.path(), "addon");
        let errs = addon.errors.join("\n");
        assert!(
            errs.contains("lens `base/a`, which isn't a loaded document or dashboard"),
            "{errs}"
        );
        assert!(
            errs.contains("is a document, so its parts are at `base/page.sections`"),
            "{errs}"
        );
        assert_eq!(addon.ui.slots.len(), 1, "the bad mounts go");
    }

    /// A composite's `columns:` name its query's row: only with a query,
    /// and only their `key`s.
    #[test]
    fn a_composites_columns_name_its_row_and_nothing_else() {
        let (_d, ext) = load_x(
            &[
                ("part", "title: Part\nparams: [{ name: change_id }]\nquery: SELECT :change_id AS n\n"),
                ("ok", "title: Ok\nquery: SELECT 7 AS change_id\ncolumns: [{ key: change_id }]\nviz: document\nsections: [{ lens: part }]\n"),
                ("no-query", "title: NoQuery\ncolumns: [{ key: change_id }]\nviz: document\nsections: [{ lens: part }]\n"),
                ("shown", "title: Shown\nquery: SELECT 7 AS change_id\ncolumns: [{ key: change_id, label: Change }]\nviz: dashboard\ntiles: [{ lens: part }]\n"),
            ],
            "",
        );
        assert!(
            ext.lenses.iter().any(|l| l.slug == "ok"),
            "{:?}",
            ext.errors
        );
        for slug in ["no-query", "shown"] {
            assert!(ext.lenses.iter().all(|l| l.slug != slug), "{slug} loaded");
        }
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("no-query.yaml")
                && errs.contains("a `document` without a query has no row to name"),
            "{errs}"
        );
        assert!(
            errs.contains("shown.yaml") && errs.contains("a composite's column is only its `key`"),
            "{errs}"
        );
    }

    /// A dashboard that names its rows' tiles in `tile:` needs no static
    /// `tiles:`, may still have them, and its tile roles are columns its
    /// query must return.
    #[test]
    fn a_dashboards_rows_are_tiles_when_it_names_them() {
        let q =
            "query: SELECT 'dti1' AS id, 'T' AS title, NULL AS q, 'hi' AS body\nviz: dashboard\n";
        let (_d, ext) = load_x(
            &[
                ("n", "title: N\nquery: SELECT 1 AS n\nviz: number\n"),
                (
                    "rows",
                    &format!(
                        "title: Rows\n{q}tile: {{ id: id, title: title, sql: q, text: body }}\n"
                    ),
                ),
                (
                    "both",
                    &format!(
                        "title: Both\n{q}tile: {{ id: id, text: body }}\ntiles: [{{ lens: n }}]\n"
                    ),
                ),
            ],
            "",
        );
        let lens = |slug: &str| {
            ext.lenses
                .iter()
                .find(|l| l.slug == slug)
                .unwrap_or_else(|| panic!("{slug}: {:?}", ext.errors))
        };
        let rows = lens("rows");
        assert!(rows.parts.is_empty());
        assert_eq!(rows.tile.as_ref().unwrap().id.as_deref(), Some("id"));
        let roles: Vec<(&str, &str)> = rows
            .role_columns()
            .into_iter()
            .map(|(b, c)| (b, c.as_str()))
            .collect();
        assert_eq!(
            roles,
            vec![
                ("tile", "id"),
                ("tile", "title"),
                ("tile", "q"),
                ("tile", "body")
            ]
        );
        assert_eq!(lens("both").parts.len(), 1);
    }

    /// `tile:` is checked at load: a dashboard's, over a query, with an
    /// `id` and something to draw, a `sql` tile's display only with `sql`,
    /// and never beside `columns:`.
    #[test]
    fn a_row_tile_dashboard_is_checked_at_load() {
        let (_d, ext) = load_x(
            &[
                ("table", "title: A\nquery: SELECT 1 AS id\ntile: { id: id, text: id }\n"),
                ("no-query", "title: B\nviz: dashboard\ntile: { id: id, text: t }\n"),
                ("no-id", "title: C\nquery: SELECT 1 AS t\nviz: dashboard\ntile: { text: t }\n"),
                ("no-body", "title: D\nquery: SELECT 1 AS id\nviz: dashboard\ntile: { id: id, title: id }\n"),
                ("display", "title: E\nquery: SELECT 1 AS id\nviz: dashboard\ntile: { id: id, text: id, display: id }\n"),
                ("columns", "title: F\nquery: SELECT 1 AS id\nviz: dashboard\ncolumns: [{ key: id }]\ntile: { id: id, text: id }\n"),
            ],
            "",
        );
        assert!(
            ext.lenses.is_empty(),
            "{:?}",
            ext.lenses.iter().map(|l| &l.slug).collect::<Vec<_>>()
        );
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            ("table", "`tile` is for viz `dashboard`, not `table`"),
            (
                "no-query",
                "a `dashboard` whose rows are tiles needs a `query`",
            ),
            ("no-id", "`tile` needs `id: <column>`"),
            ("no-body", "`tile` needs one of `lens`, `sql` or `text`"),
            ("display", "`tile.display` is a `sql` tile's"),
            (
                "columns",
                "a dashboard whose rows are tiles names them in `tile:`, not `columns:`",
            ),
        ] {
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// A row-tile dashboard's edit action edits a text tile in place: it
    /// saves the `tile.text` column, and nothing else.
    #[test]
    fn a_row_tile_dashboards_edit_action_saves_its_text() {
        let q = "query: SELECT 'dti1' AS id, 'x' AS body, 'T' AS title\nviz: dashboard\ntile: { id: id, title: title, text: body }\n";
        let edit = |col: &str| {
            format!(
                "actions:\n  - {{ id: edit, label: Save, command: x.y.z, edit: true, fixed: [item], input: {{ item: '{{{{row.id}}}}', text: '{{{{row.{col}}}}}' }} }}\n"
            )
        };
        let (_d, ext) = load_x(
            &[
                ("ok", &format!("title: Ok\n{q}{}", edit("body"))),
                ("wrong", &format!("title: Wrong\n{q}{}", edit("title"))),
                (
                    "plain",
                    &format!(
                        "title: Plain\nviz: dashboard\ntiles: [{{ markdown: hi, id: m }}]\n{}",
                        edit("body")
                    ),
                ),
            ],
            "",
        );
        assert!(
            ext.lenses.iter().any(|l| l.slug == "ok"),
            "{:?}",
            ext.errors
        );
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("wrong.yaml")
                && errs.contains("`edit` saves `title`, but the tiles' text is `body`"),
            "{errs}"
        );
        assert!(
            errs.contains("plain.yaml")
                && errs.contains("an edit action (`edit`) is a `markdown` viz's"),
            "{errs}"
        );
    }

    /// A dashboard whose rows are its tiles gives its static parts its own
    /// params alone: its first row is a tile, not a row of params.
    #[test]
    fn a_row_tile_dashboard_gives_its_parts_no_row() {
        let (_d, ext) = load_x(
            &[(
                "dash",
                "title: D\nparams: [{ name: id }]\nquery: SELECT 'dti1' AS tid, 'x' AS body\nviz: dashboard\ntile: { id: tid, text: body }\n",
            )],
            "",
        );
        let lens = ext.lenses.into_iter().find(|l| l.slug == "dash").unwrap();
        let run = LensRun {
            lens,
            params: BTreeMap::from([("id".to_string(), SqlCell::Text("dsh1".into()))]),
            result: oxplow_db::SqlQueryResult {
                columns: vec!["tid".into(), "body".into()],
                rows: vec![vec![
                    SqlCell::Text("dti1".into()),
                    SqlCell::Text("x".into()),
                ]],
                truncated: false,
                reads: oxplow_db::Reads::default(),
                freshness: Vec::new(),
                generation: None,
                coverage: Vec::new(),
            },
            alert: None,
            warnings: Vec::new(),
            inactive: None,
        };
        assert_eq!(
            part_params(&run),
            BTreeMap::from([("id".to_string(), SqlCell::Text("dsh1".into()))])
        );
    }

    /// A mount into a document's `.sections` may take the page's own params
    /// or a name of its row; one that takes neither is refused, the error
    /// naming both. A row name no part takes warns.
    #[test]
    fn a_mount_may_take_a_documents_row_and_a_row_name_no_part_takes_warns() {
        let dir = tempfile::tempdir().unwrap();
        let ext_yaml = |name: &str, tail: &str| {
            format!("manifest: 2\nname: {name}\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\n{tail}")
        };
        write(
            dir.path(),
            ".oxplow/extensions/base/extension.yaml",
            &ext_yaml("base", ""),
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/lenses/a.yaml",
            "title: A\nparams: [{ name: change_id }]\nquery: SELECT :change_id AS n\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/lenses/page.yaml",
            "title: Page\nparams: [{ name: sha }]\nquery: SELECT 7 AS change_id, 1 AS stray, 2 AS taken_by_a_mount\ncolumns: [{ key: change_id }, { key: stray }, { key: taken_by_a_mount }]\nviz: document\nsections: [{ lens: a }]\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/extension.yaml",
            &ext_yaml(
                "addon",
                "ui:\n  slots:\n    - { slot: base/page.sections, lens: by-row }\n    - { slot: base/page.sections, lens: by-mount }\n    - { slot: base/page.sections, lens: neither }\n",
            ),
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/lenses/by-row.yaml",
            "title: ByRow\nparams: [{ name: change_id }]\nquery: SELECT 1\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/lenses/by-mount.yaml",
            "title: ByMount\nparams: [{ name: taken_by_a_mount }]\nquery: SELECT 1\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/lenses/neither.yaml",
            "title: Neither\nparams: [{ name: other }]\nquery: SELECT 1\n",
        );
        let addon = only(dir.path(), "addon");
        let errs = addon.errors.join("\n");
        assert!(
            errs.contains("lens `neither` must declare at least one in `params`")
                && errs.contains("`sha`")
                && errs.contains("`change_id`"),
            "{errs}"
        );
        assert_eq!(addon.ui.slots.len(), 2, "{errs}");
        let base = only(dir.path(), "base");
        let stray: Vec<&String> = base
            .warnings
            .iter()
            .filter(|w| w.contains("`stray`"))
            .collect();
        assert_eq!(stray.len(), 1, "{:?}", base.warnings);
        assert!(
            stray[0].contains("is a param of none of its parts"),
            "{stray:?}"
        );
        assert!(
            !base
                .warnings
                .iter()
                .any(|w| w.contains("`change_id`") || w.contains("`taken_by_a_mount`")),
            "{:?}",
            base.warnings
        );
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
            // The github example's pull requests are a resource of its own.
            if name == "github" {
                let resources: Vec<String> = ext
                    .resources
                    .iter()
                    .map(|r| r.decl.name.to_string())
                    .collect();
                assert_eq!(resources, vec!["github/pr"]);
            }
        }
    }

    /// One project extension `x` with the given lens files and
    /// extension.yaml tail; returns its load result.
    fn load_x(files: &[(&str, &str)], manifest_tail: &str) -> (tempfile::TempDir, Extension) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            &if manifest_tail.starts_with("manifest:") {
                format!("name: x\n{manifest_tail}")
            } else {
                format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest_tail}")
            },
        );
        for (slug, body) in files {
            write(
                dir.path(),
                &format!(".oxplow/extensions/x/lenses/{slug}.yaml"),
                body,
            );
        }
        let ext = project_extensions(dir.path()).remove(0);
        (dir, ext)
    }

    /// A document composes its parts and never renders rows of its own,
    /// so it needs no `query` (like a form); every other viz still does.
    #[test]
    fn a_document_needs_no_query() {
        let (_d, ext) = load_x(
            &[
                ("n", "title: N\nquery: SELECT 1 AS n\nviz: number\n"),
                (
                    "all",
                    "title: All\nviz: document\nsections: [{ lens: n }]\n",
                ),
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
                ("all", "title: All\nviz: dashboard\ntiles: [{ lens: visits, size: wide }, { lens: trend }]\n"),
                ("nobar", "title: N\nquery: SELECT 1\nviz: bar\n"),
                ("badgrid", "title: G\nviz: dashboard\ntiles: [{ lens: nope }]\n"),
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
        let tiles: Vec<_> = lens("all")
            .unwrap()
            .parts
            .iter()
            .map(|p| (p.id.as_str(), p.lens.as_deref(), p.size))
            .collect();
        assert_eq!(
            tiles,
            vec![
                ("visits", Some("x/visits"), Some(TileSize::Wide)),
                ("trend", Some("x/trend"), None)
            ]
        );
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

    /// A settings-like document: `area` picks the section in view.
    fn view_doc(extra: &str) -> String {
        format!(
            "title: Settings\nparams:\n  - {{ name: area, default: one }}\n{extra}viz: document\nview: area\nsections:\n  - {{ id: one, title: First, lens: a }}\n  - {{ id: two, lens: b }}\n"
        )
    }

    /// A document's `view` param is a `filter` choice whose options are its
    /// sections: each id, labelled by its title, else its lens's.
    #[test]
    fn a_view_document_names_a_filter_choice_whose_options_are_its_sections() {
        let (_d, ext) = load_x(
            &[
                ("a", "title: A\nquery: SELECT 1 AS n\n"),
                ("b", "title: Bee\nquery: SELECT 2 AS n\n"),
                ("s", &view_doc("")),
            ],
            "",
        );
        let s = ext
            .lenses
            .iter()
            .find(|l| l.slug == "s")
            .unwrap_or_else(|| panic!("{:?}", ext.errors));
        assert_eq!(s.view.as_deref(), Some("area"));
        let area = s.params.iter().find(|p| p.name == "area").unwrap();
        assert_eq!(area.role, Some(ParamRole::Filter));
        assert_eq!(area.kind, Some(ParamType::Choice));
        let options: Vec<(&str, &str)> = area
            .options
            .iter()
            .map(|o| (o.value.as_str(), o.label.as_str()))
            .collect();
        assert_eq!(options, [("one", "First"), ("two", "Bee")]);
        assert!(area.take(&SqlCell::Text("two".into())).is_ok());
        assert!(area.take(&SqlCell::Text("nope".into())).is_err());
    }

    /// `view` is a document's, names a declared filter choice with no
    /// options of its own, and starts on one of its sections.
    #[test]
    fn a_view_is_refused_off_a_document_or_mis_declared() {
        let a = ("a", "title: A\nquery: SELECT 1 AS n\n");
        let b = ("b", "title: B\nquery: SELECT 2 AS n\n");
        let doc = |params: &str| {
            format!(
                "title: S\nparams:\n{params}viz: document\nview: area\nsections:\n  - {{ id: one, lens: a }}\n  - {{ id: two, lens: b }}\n"
            )
        };
        let cases = [
            ("table", "title: T\nparams: [{ name: area, default: x }]\nquery: SELECT :area AS a\nview: area\n".to_string(), "`view` is for a document"),
            ("undeclared", doc("  - { name: q, default: \"\" }\n"), "`view` names param `area`, which the lens doesn't declare"),
            ("resource", doc("  - { name: area, role: resource, default: one }\n"), "`view` param `area` rides in the `#`: leave `role` out or say `filter`"),
            ("text", doc("  - { name: area, type: text, default: one }\n"), "`view` param `area` is a choice of the sections: leave `type` out"),
            ("options", doc("  - { name: area, default: one, options: [{ value: one, label: One }] }\n"), "`view` param `area`: the sections are its choices, not `options`"),
            ("nodefault", doc("  - { name: area }\n"), "`view` param `area` needs a `default`: the section shown first"),
            ("baddefault", doc("  - { name: area, default: three }\n"), "`view` param `area`'s default `three` isn't a section"),
        ];
        let mut files = vec![a, b];
        files.extend(cases.iter().map(|(slug, body, _)| (*slug, body.as_str())));
        let (_d, ext) = load_x(&files, "");
        let errs = ext.errors.join("\n");
        for (slug, _, says) in &cases {
            assert!(ext.lenses.iter().all(|l| l.slug != *slug), "{slug} loaded");
            assert!(
                errs.lines()
                    .any(|l| l.contains(&format!("{slug}.yaml")) && l.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// A section another extension mounts is one of the view's choices,
    /// and it needn't read the document's params: each section of a view
    /// document is a page of its own.
    #[test]
    fn a_view_documents_options_include_mounted_sections_and_take_parameterless_mounts() {
        let (_d, ext) = load_x(
            &[
                ("a", "title: A\nquery: SELECT 1 AS n\n"),
                ("b", "title: B\nquery: SELECT 2 AS n\n"),
                ("more", "title: More Things\nquery: SELECT 3 AS n\n"),
                (
                    "s",
                    &view_doc("  - { name: q, type: search, role: filter, default: \"\" }\n"),
                ),
            ],
            "ui:\n  slots:\n    - { slot: x/s.sections, lens: more, at: { after: one } }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let s = ext.lenses.iter().find(|l| l.slug == "s").unwrap();
        let area = s.params.iter().find(|p| p.name == "area").unwrap();
        let options: Vec<(&str, &str)> = area
            .options
            .iter()
            .map(|o| (o.value.as_str(), o.label.as_str()))
            .collect();
        assert_eq!(
            options,
            [("one", "First"), ("x/more", "More Things"), ("two", "B")]
        );
    }

    /// Resolving a resolved catalog again (a setting turned off) places
    /// its mounts once, and its view's choices stay the same.
    #[test]
    fn a_catalog_resolved_again_places_its_mounts_once() {
        let (_d, ext) = load_x(
            &[
                ("a", "title: A\nquery: SELECT 1 AS n\n"),
                ("b", "title: B\nquery: SELECT 2 AS n\n"),
                ("more", "title: More\nquery: SELECT 3 AS n\n"),
                ("s", &view_doc("")),
            ],
            "ui:\n  slots:\n    - { slot: x/s.sections, lens: more }\n",
        );
        let mut again = vec![ext.clone()];
        place_lens_mounts(&mut again);
        fill_view_options(&mut again);
        let parts = |e: &Extension| {
            let s = e.lenses.iter().find(|l| l.slug == "s").unwrap();
            (
                s.parts.len(),
                s.params
                    .iter()
                    .find(|p| p.name == "area")
                    .unwrap()
                    .options
                    .len(),
            )
        };
        assert_eq!(parts(&ext), (3, 3));
        assert_eq!(parts(&again[0]), (3, 3));
    }

    /// A view document doesn't warn for its view param (read by no query),
    /// for its sections' count, or for a filter a section two levels down
    /// reads.
    #[test]
    fn a_view_document_warns_for_neither_its_view_nor_its_section_count_and_sees_filters_read_below(
    ) {
        let mut files: Vec<(String, String)> = (0..9)
            .map(|i| {
                (
                    format!("l{i}"),
                    format!("title: L{i}\nquery: SELECT {i} AS n\n"),
                )
            })
            .collect();
        files.push(("leaf".into(), "title: Leaf\nparams: [{ name: q, type: search, role: filter, default: \"\" }]\nquery: SELECT :q AS n\n".into()));
        files.push(("inner".into(), "title: Inner\nparams: [{ name: q, type: search, role: filter, default: \"\" }]\nviz: document\nsections: [{ lens: leaf }]\n".into()));
        let sections: String = (0..9)
            .map(|i| format!("  - {{ id: s{i}, lens: l{i} }}\n"))
            .chain(std::iter::once(
                "  - { id: inner, lens: inner }\n".to_string(),
            ))
            .collect();
        files.push((
            "s".into(),
            format!("title: S\nparams:\n  - {{ name: area, default: s0 }}\n  - {{ name: q, type: search, role: filter, default: \"\" }}\nviz: document\nview: area\nsections:\n{sections}"),
        ));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (_d, ext) = load_x(&refs, "");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let warns: Vec<String> = layout_only(&ext.warnings)
            .into_iter()
            .filter(|w| w.contains("s.yaml"))
            .collect();
        assert!(warns.is_empty(), "{warns:?}");
    }

    /// A part that names its own lens, or parts that lead back round, would
    /// make a page that holds itself: refused.
    #[test]
    fn a_part_that_names_its_own_lens_or_a_cycle_is_refused() {
        let (_d, ext) = load_x(
            &[
                ("me", "title: Me\nviz: document\nsections: [{ lens: me }]\n"),
                (
                    "ping",
                    "title: Ping\nviz: document\nsections: [{ lens: pong }]\n",
                ),
                (
                    "pong",
                    "title: Pong\nviz: document\nsections: [{ lens: ping }]\n",
                ),
                ("fine", "title: Fine\nquery: SELECT 1 AS n\n"),
            ],
            "",
        );
        let errs = ext.errors.join("\n");
        assert!(errs.contains("a part names the lens itself"), "{errs}");
        assert!(errs.contains("its parts lead back to `x/ping`"), "{errs}");
        assert!(errs.contains("its parts lead back to `x/pong`"), "{errs}");
        let slugs: Vec<&str> = ext.lenses.iter().map(|l| l.slug.as_str()).collect();
        assert_eq!(slugs, ["fine"]);
    }

    /// `section` is the page's view key: no lens param may take it.
    #[test]
    fn section_is_the_pages_view_key_not_a_param() {
        let (_d, ext) = load_x(
            &[(
                "a",
                "title: A\nparams: [{ name: section, default: x }]\nquery: SELECT :section AS n\n",
            )],
            "",
        );
        assert!(ext.lenses.is_empty());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`section` is the page's view key (`#section=`): rename the param"),
            "{errs}"
        );
    }

    /// A lazy tree (`tree.lazy: <param>`, `tree.branch: <column>`) runs one
    /// level at a time: its param needs a default (the top level) and can't
    /// be one the context supplies; `branch` says which rows open; a
    /// column that `select`s the lazy param would re-root the page instead.
    #[test]
    fn a_lazy_tree_names_its_param_and_its_branch_column() {
        let tree = |extra: &str| {
            format!(
                "title: T\nquery: SELECT 'a' AS id, '' AS p, 'a' AS n, 1 AS d\nviz: tree\n{extra}"
            )
        };
        let folder = "params:\n  - { name: folder, default: \"\" }\n";
        let (_d, ext) = load_x(
            &[
                ("lazy", &tree(&format!("{folder}tree: {{ id: id, parent: p, label: n, lazy: folder, branch: d }}\n"))),
                ("undeclared", &tree("tree: { id: id, parent: p, label: n, lazy: nope, branch: d }\n")),
                ("nodefault", &tree("params:\n  - { name: folder }\ntree: { id: id, parent: p, label: n, lazy: folder, branch: d }\n")),
                ("context", &tree("params:\n  - { name: stream_id, default: 1 }\ntree: { id: id, parent: p, label: n, lazy: stream_id, branch: d }\n")),
                ("nobranch", &tree(&format!("{folder}tree: {{ id: id, parent: p, label: n, lazy: folder }}\n"))),
                ("branchonly", &tree("tree: { id: id, parent: p, label: n, branch: d }\n")),
                ("selectlazy", &tree(&format!("{folder}tree: {{ id: id, parent: p, label: n, lazy: folder, branch: d }}\ncolumns: [{{ key: n, select: folder }}]\n"))),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        let lazy = lens("lazy").unwrap_or_else(|| panic!("{:?}", ext.errors));
        let t = lazy.tree.as_ref().unwrap();
        assert_eq!(
            (t.lazy.as_deref(), t.branch.as_deref()),
            (Some("folder"), Some("d"))
        );
        for slug in [
            "undeclared",
            "nodefault",
            "context",
            "nobranch",
            "branchonly",
            "selectlazy",
        ] {
            assert!(lens(slug).is_none(), "{slug} loaded");
        }
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            ("undeclared", "`tree.lazy` names param `nope`, which the lens doesn't declare"),
            ("nodefault", "`tree.lazy` param `folder` needs a `default`: the top level"),
            ("context", "`tree.lazy` can't be `stream_id`: the context supplies it"),
            ("nobranch", "`tree.lazy` needs `tree.branch: <column>` (the rows that open)"),
            ("branchonly", "`tree.branch` is for a lazy tree (`tree.lazy: <param>`)"),
            ("selectlazy", "column `n`: `select` names the tree's `lazy` param; a click opens the branch instead"),
        ] {
            assert!(
                errs.lines().any(|l| l.contains(slug) && l.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// `code` shows each row's file at a revision, `diff` the first row's
    /// file between two: each needs its block's columns, and the rest of
    /// the block is optional.
    #[test]
    fn code_and_diff_lenses_name_their_columns() {
        let (_d, ext) = load_x(
            &[
                ("c", "title: C\nquery: SELECT 'f' AS p, 'working' AS r, '3-8' AS l\nviz: code\ncode: { path: p, revision: r, lines: l, sync: true }\n"),
                ("c2", "title: C\nquery: SELECT 'f' AS p, 'working' AS r\nviz: code\ncode: { path: p, revision: r }\n"),
                ("df", "title: D\nquery: SELECT 'f' AS p, 'git:HEAD' AS a, 'working' AS b\nviz: diff\ndiff: { path: p, from: a, to: b }\n"),
                ("nocode", "title: N\nquery: SELECT 'f' AS p\nviz: code\ncode: { path: p }\n"),
                ("nodiff", "title: N\nquery: SELECT 1\nviz: diff\n"),
            ],
            "",
        );
        let lens = |slug: &str| ext.lenses.iter().find(|l| l.slug == slug);
        let c = lens("c").expect("c loads");
        assert_eq!(
            c.code,
            Some(LensCode {
                path: Some("p".into()),
                revision: Some("r".into()),
                lines: Some("l".into()),
                sync: true,
            })
        );
        assert!(!lens("c2").unwrap().code.as_ref().unwrap().sync);
        assert_eq!(
            lens("df").unwrap().diff.as_ref().unwrap().to.as_deref(),
            Some("b")
        );
        assert!(lens("nocode").is_none() && lens("nodiff").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`code: { revision: <column> }`"), "{errs}");
        assert!(
            errs.contains("`diff: { path: <column>, from: <column>, to: <column> }`"),
            "{errs}"
        );
    }

    /// A param's `options` are the values it takes: its default
    /// must be one, and a run supplying another is refused.
    #[test]
    fn param_options_bound_their_values() {
        let (_d, ext) = load_x(
            &[
                ("ok", "title: OK\nquery: SELECT :mode AS m\nviz: list\nparams:\n  - { name: mode, default: recent, options: [{ value: recent, label: Recent }, { value: top, label: Most visited }] }\n"),
                ("bad", "title: Bad\nquery: SELECT :mode AS m\nviz: list\nparams:\n  - { name: mode, default: all, options: [{ value: recent, label: Recent }] }\n"),
            ],
            "",
        );
        let ok = ext
            .lenses
            .iter()
            .find(|l| l.slug == "ok")
            .expect("ok loads");
        assert_eq!(ok.params[0].options[1].label, "Most visited");
        assert!(ext.lenses.iter().all(|l| l.slug != "bad"));
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`mode`") && errs.contains("`all`"), "{errs}");
        let supplied = BTreeMap::from([("mode".to_string(), SqlCell::Text("top".into()))]);
        assert!(resolve_params(ok, &supplied, &LensContext::default()).is_ok());
        let supplied = BTreeMap::from([("mode".to_string(), SqlCell::Text("all".into()))]);
        assert!(resolve_params(ok, &supplied, &LensContext::default()).is_err());
    }

    /// A param's `type` says what it holds and how a page draws it; with
    /// none it's a choice when it has options, else text. What doesn't fit
    /// its type is refused at load.
    #[test]
    fn typed_params_are_checked_at_load() {
        let (_d, ext) = load_x(
            &[
                ("ok", "title: OK\nquery: SELECT 1 AS a\nparams:\n  - { name: range, type: date_range, presets: [7d, all], default: 7d }\n  - { name: flag, type: bool, default: true }\n  - { name: q, type: search }\n  - { name: br, type: branch }\n  - { name: mode, default: a, options: [{ value: a, label: A }] }\n  - { name: free }\n  - { name: end, options_from: ends }\n  - { name: start, type: choice, options_from: other/ends, default: anything }\n"),
                ("ends", "title: Ends\nquery: SELECT 'snap:1' AS value, 'One' AS label\n"),
                ("choice_without_options", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: choice }]\n"),
                ("both_option_sources", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, options_from: ends, options: [{ value: a, label: A }] }]\n"),
                ("options_from_on_text", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: text, options_from: ends }]\n"),
                ("options_from_missing", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, options_from: nowhere }]\n"),
                ("options_on_text", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: text, options: [{ value: a, label: A }] }]\n"),
                ("presets_on_text", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, presets: [7d] }]\n"),
                ("unknown_preset", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: date_range, presets: [5y] }]\n"),
                ("bad_range", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: date_range, default: soon }]\n"),
                ("bad_bool", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, type: bool, default: maybe }]\n"),
                ("placed", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, place: toolbar }]\n"),
                ("roled", "title: X\nquery: SELECT 1 AS a\nparams: [{ name: m, role: selection }]\n"),
            ],
            "",
        );
        let ok = ext
            .lenses
            .iter()
            .find(|l| l.slug == "ok")
            .expect("ok loads");
        let kinds: Vec<_> = ok.params.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [
                ParamType::DateRange,
                ParamType::Bool,
                ParamType::Search,
                ParamType::Branch,
                ParamType::Choice,
                ParamType::Text,
                ParamType::Choice,
                ParamType::Choice,
            ]
            .map(Some)
        );
        // A dynamic choice's lens is qualified as `expand` is; its options
        // aren't known at load, so its default (if any) isn't checked.
        let from: Vec<_> = ok
            .params
            .iter()
            .skip(6)
            .map(|p| p.options_from.as_deref())
            .collect();
        assert_eq!(from, [Some("x/ends"), Some("other/ends")]);
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            (
                "choice_without_options",
                "needs `options` or `options_from`",
            ),
            (
                "both_option_sources",
                "`options` or `options_from`, not both",
            ),
            (
                "options_from_on_text",
                "`options_from` is only for a choice",
            ),
            (
                "options_from_missing",
                "`options_from` lens `x/nowhere` isn't in this extension's lenses/",
            ),
            ("options_on_text", "`options` are only for a choice"),
            ("presets_on_text", "`presets` are only for a date_range"),
            ("unknown_preset", "`5y`"),
            ("bad_range", "`soon`"),
            ("bad_bool", "`maybe`"),
            // A page draws every param in its rail; where is not the
            // author's to say, and a selection follows from the columns.
            ("placed", "unknown field `place`"),
            ("roled", "`role: selection` follows from the columns"),
        ] {
            assert!(ext.lenses.iter().all(|l| l.slug != slug), "{slug} loaded");
            assert!(
                errs.lines()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// A choice whose options come from a lens takes any text (the query
    /// says what an unknown value means) and NULL, as every param.
    #[test]
    fn a_dynamic_choice_takes_any_text() {
        let p = LensParam {
            name: "end".into(),
            label: None,
            default: None,
            options: Vec::new(),
            options_from: Some("x/ends".into()),
            kind: None,
            role: None,
            presets: Vec::new(),
        };
        assert_eq!(p.kind(), ParamType::Choice);
        assert_eq!(
            p.take(&SqlCell::Text("snap:9".into())),
            Ok(SqlCell::Text("snap:9".into()))
        );
        assert_eq!(p.take(&SqlCell::Null(())), Ok(SqlCell::Null(())));
        assert!(p.take(&SqlCell::Int(3)).unwrap_err().contains("text"));
    }

    /// A bool binds 1 or 0, a search its trimmed text or NULL, and a
    /// date_range its own text plus `:<name>_from` / `:<name>_to`: a
    /// preset back from now, a custom range's days with the last one
    /// included, `all` neither end. What doesn't fit is refused.
    #[tokio::test]
    async fn typed_params_bind_by_type() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/typed.yaml",
            "title: Typed\nparams:\n  - { name: flag, type: bool, default: false }\n  - { name: q, type: search }\n  - { name: range, type: date_range, default: all }\nquery: SELECT :flag AS flag, :q AS q, :range AS range, :range_from AS f, :range_to AS t\n",
        );
        let sl = layer().await;
        let run = |over: Vec<(&str, &str)>| {
            let supplied: BTreeMap<String, SqlCell> = over
                .into_iter()
                .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
                .collect();
            let sl = &sl;
            let root = dir.path();
            async move {
                run_lens(
                    sl,
                    &cat(),
                    root,
                    "review/typed",
                    supplied,
                    &LensContext::default(),
                )
                .await
            }
        };
        let rows = |r: &LensRun| serde_json::to_value(&r.result.rows).unwrap();

        let first = run(vec![]).await.unwrap();
        assert_eq!(
            rows(&first),
            serde_json::json!([[0, null, "all", null, null]])
        );
        assert!(!first.params.contains_key("range_from"));

        let custom = run(vec![
            ("flag", "true"),
            ("q", "  fix "),
            ("range", "2026-10-01..2026-10-20"),
        ])
        .await
        .unwrap();
        assert_eq!(
            rows(&custom),
            serde_json::json!([[
                1,
                "fix",
                "2026-10-01..2026-10-20",
                "2026-10-01",
                "2026-10-21"
            ]])
        );
        assert_eq!(
            run(vec![("q", "  ")]).await.unwrap().result.rows[0][1],
            SqlCell::Null(())
        );

        let week = run(vec![("range", "7d")]).await.unwrap();
        let SqlCell::Text(from) = &week.result.rows[0][3] else {
            panic!("7d binds a from: {:?}", week.result.rows)
        };
        let from =
            time::OffsetDateTime::parse(from, &time::format_description::well_known::Rfc3339)
                .expect("an RFC 3339 time");
        let back = time::OffsetDateTime::now_utc() - from;
        assert!(
            back > time::Duration::days(7) - time::Duration::minutes(1)
                && back < time::Duration::days(7) + time::Duration::minutes(1),
            "{back}"
        );
        assert_eq!(week.result.rows[0][4], SqlCell::Null(()));

        for (k, v) in [
            ("flag", "maybe"),
            ("range", "soon"),
            ("range", "2026-10-20..2026-10-01"),
        ] {
            assert!(run(vec![(k, v)]).await.is_err(), "{k}={v} was taken");
        }
    }

    /// A column's `select` names a param the lens declares; a document's
    /// own section that selects needs the document to hold that param, or
    /// a click in it couldn't reach the other sections.
    #[test]
    fn a_select_names_a_param_its_page_holds() {
        let (_d, ext) = load_x(
            &[
                ("rows", "title: Rows\nquery: SELECT 'a' AS sha\nparams: [{ name: commit }]\ncolumns: [{ key: sha, select: commit }]\n"),
                ("nope", "title: Nope\nquery: SELECT 'a' AS sha\ncolumns: [{ key: sha, select: commit }]\n"),
                ("holds", "title: Holds\nviz: document\nparams: [{ name: commit }]\nsections: [{ lens: rows }]\n"),
                ("lacks", "title: Lacks\nviz: document\nsections: [{ lens: rows }]\n"),
            ],
            "",
        );
        let rows = ext
            .lenses
            .iter()
            .find(|l| l.slug == "rows")
            .expect("rows loads");
        assert_eq!(rows.columns[0].select.as_deref(), Some("commit"));
        assert!(ext.lenses.iter().any(|l| l.slug == "holds"));
        for slug in ["nope", "lacks"] {
            assert!(ext.lenses.iter().all(|l| l.slug != slug), "{slug} loaded");
        }
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("nope.yaml") && errs.contains("`select: commit`"),
            "{errs}"
        );
        assert!(
            errs.contains("lacks.yaml") && errs.contains("x/rows") && errs.contains("`commit`"),
            "{errs}"
        );
    }

    /// A placeholder's `|json` filter loads; any other filter is refused.
    #[test]
    fn an_action_placeholder_may_only_filter_with_json() {
        let (_d, ext) = load_x(
            &[
                ("ok", "title: Ok\nquery: SELECT '[1]' AS order_json\nactions:\n  - { id: a, label: A, command: oxplow.agent.set_agents, row: true, input: { agents: \"{{row.order_json|json}}\" } }\n"),
                ("bad", "title: Bad\nquery: SELECT 1 AS n\nactions:\n  - { id: a, label: A, command: oxplow.agent.set_agents, row: true, input: { agents: \"{{row.n|upper}}\" } }\n"),
            ],
            "",
        );
        assert!(
            ext.lenses.iter().any(|l| l.slug == "ok"),
            "{:?}",
            ext.errors
        );
        assert!(ext.lenses.iter().all(|l| l.slug != "bad"));
        assert!(
            ext.errors
                .join("\n")
                .contains("unknown placeholder filter `upper`"),
            "{:?}",
            ext.errors
        );
    }

    /// A column's `expand` names a lens of the extension (by slug or whole
    /// id); one that isn't there, or the lens itself, fails the load.
    #[test]
    fn an_expand_names_a_lens_of_its_extension() {
        let (_d, ext) = load_x(
            &[
                ("detail", "title: Detail\nquery: SELECT 1 AS n\nparams: [{ name: id }]\n"),
                ("by-slug", "title: By slug\nquery: SELECT 1 AS id\ncolumns: [{ key: id, expand: detail }]\n"),
                ("by-id", "title: By id\nquery: SELECT 1 AS id\ncolumns: [{ key: id, expand: x/detail }]\n"),
                ("missing", "title: Missing\nquery: SELECT 1 AS id\ncolumns: [{ key: id, expand: gone }]\n"),
                ("itself", "title: Itself\nquery: SELECT 1 AS id\ncolumns: [{ key: id, expand: itself }]\n"),
            ],
            "",
        );
        for slug in ["by-slug", "by-id"] {
            let lens = ext
                .lenses
                .iter()
                .find(|l| l.slug == slug)
                .unwrap_or_else(|| panic!("{slug} didn't load: {:?}", ext.errors));
            // Named whole, whichever way it was written.
            assert_eq!(lens.columns[0].expand.as_deref(), Some("x/detail"));
        }
        for slug in ["missing", "itself"] {
            assert!(ext.lenses.iter().all(|l| l.slug != slug), "{slug} loaded");
        }
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("missing.yaml") && errs.contains("`expand` lens `x/gone`"),
            "{errs}"
        );
        assert!(
            errs.contains("itself.yaml") && errs.contains("names the lens itself"),
            "{errs}"
        );
    }

    /// `targets` draws target lines on a `line` chart; on another viz it is
    /// refused. The formats `sparkline` and `badge` load.
    #[test]
    fn chart_targets_and_the_new_formats_load() {
        let (_d, ext) = load_x(
            &[
                (
                    "trend",
                    "title: Trend\nquery: SELECT 1 AS at, 2 AS v, 3 AS goal, '[1,2]' AS s, 'ok' AS level\nviz: line\nchart: { x: at, y: v, targets: goal }\ncolumns: [{ key: s, format: sparkline }, { key: level, format: badge }]\n",
                ),
                (
                    "bars",
                    "title: Bars\nquery: SELECT 1 AS at, 2 AS v, 3 AS goal\nviz: bar\nchart: { x: at, y: v, targets: goal }\n",
                ),
            ],
            "",
        );
        let trend = ext
            .lenses
            .iter()
            .find(|l| l.slug == "trend")
            .unwrap_or_else(|| panic!("trend didn't load: {:?}", ext.errors));
        assert_eq!(
            trend.chart.as_ref().unwrap().targets.as_deref(),
            Some("goal")
        );
        assert_eq!(trend.columns[0].format, Some(ColumnFormat::Sparkline));
        assert_eq!(trend.columns[1].format, Some(ColumnFormat::Badge));
        assert!(ext.lenses.iter().all(|l| l.slug != "bars"));
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("bars.yaml") && errs.contains("`targets`"),
            "{errs}"
        );
    }

    /// A param a column selects is a selection, on the lens whose rows
    /// set it and on the page that holds it for its parts; one that says
    /// `role: filter` is a filter; every other param names the resource.
    #[test]
    fn a_params_role_is_resource_unless_it_filters_or_a_column_selects_it() {
        let (_d, ext) = load_x(
            &[
                ("rows", "title: Rows\nquery: SELECT 'a' AS sha\nparams: [{ name: commit }, { name: q, type: search, role: filter }, { name: stream_id }]\ncolumns: [{ key: sha, select: commit }]\n"),
                ("page", "title: Page\nviz: document\nparams: [{ name: q, type: search, role: filter }, { name: commit }, { name: stream_id }]\nsections: [{ lens: rows }]\n"),
            ],
            "",
        );
        for slug in ["rows", "page"] {
            let lens = ext.lenses.iter().find(|l| l.slug == slug).expect(slug);
            let role = |name: &str| lens.params.iter().find(|p| p.name == name).unwrap().role;
            assert_eq!(role("commit"), Some(ParamRole::Selection), "{slug}");
            assert_eq!(role("q"), Some(ParamRole::Filter), "{slug}");
            assert_eq!(role("stream_id"), Some(ParamRole::Resource), "{slug}");
        }
    }

    /// A selection follows from the columns: a lens that writes one is
    /// refused.
    #[test]
    fn a_written_selection_role_is_refused() {
        let (_d, ext) = load_x(
            &[(
                "rows",
                "title: Rows\nquery: SELECT 1 AS n\nparams: [{ name: commit, role: selection }]\n",
            )],
            "",
        );
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`role: selection` follows from the columns"),
            "{errs}"
        );
    }

    /// A lens says where it comes from: the source file to edit, relative
    /// to the project, and the copy that runs. Loaded from its source,
    /// they are the same file.
    #[test]
    fn a_lens_from_its_source_names_that_file() {
        let (_d, ext) = load_x(&[("ok", "title: OK\nquery: SELECT 1 AS a\n")], "");
        let origin = &ext.lenses[0].origin;
        assert_eq!(origin.extension, "x");
        assert_eq!(
            origin.source.as_deref(),
            Some(".oxplow/extensions/x/lenses/ok.yaml")
        );
        assert_eq!(origin.running, ".oxplow/extensions/x/lenses/ok.yaml");
        assert_eq!(origin.deployment, None);
        assert!(!origin.bundled);
    }

    /// A deployed lens runs from its copy in the store, but its origin
    /// names the source to edit and the deployment it came from.
    #[test]
    fn a_deployed_lens_names_its_source_and_its_deployment() {
        let (d, _) = load_x(&[("ok", "title: OK\nquery: SELECT 1 AS a\n")], "");
        let source = d.path().join(".oxplow/extensions/x");
        let hash = crate::extension_store::source_hash(&source).unwrap();
        crate::extension_store::copy_into_store(d.path(), "x", &source, &hash).unwrap();
        let mut settings = oxplow_config::extension_settings(d.path());
        settings.personal.enable.push("x".into());
        let live = BTreeMap::from([("x".to_string(), hash.clone())]);
        let ext = load_deployed(d.path(), &settings, &live)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let origin = &ext.lenses[0].origin;
        assert_eq!(
            origin.source.as_deref(),
            Some(".oxplow/extensions/x/lenses/ok.yaml")
        );
        assert!(
            origin
                .running
                .ends_with(&format!("x/{hash}/lenses/ok.yaml")),
            "{}",
            origin.running
        );
        assert_eq!(origin.deployment.as_deref(), Some(hash.as_str()));
        assert!(!origin.bundled);
        // An edit to the source shows until it is deployed.
        assert!(!origin.out_of_date(d.path()));
        write(
            d.path(),
            ".oxplow/extensions/x/lenses/ok.yaml",
            "title: Edited\nquery: SELECT 1 AS a\n",
        );
        assert!(origin.out_of_date(d.path()));
        assert_eq!(origin.to_json(d.path())["outOfDate"], true);
    }

    /// The layout warnings of one lens file: its loaded extension's
    /// warnings, and whether the lens still loaded.
    fn layout_of(lens_body: &str) -> (Vec<String>, bool) {
        let (_d, ext) = load_x(&[("p", lens_body)], "");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        (
            layout_only(&ext.warnings),
            ext.lenses.iter().any(|l| l.slug == "p"),
        )
    }

    /// The layout warnings among `warnings`.
    fn layout_only(warnings: &[String]) -> Vec<String> {
        const MARKS: [&str; 6] = [
            " filters; ",
            "isn't read by any query",
            " options — ",
            "the description is",
            " sections — ",
            " columns — ",
        ];
        warnings
            .iter()
            .filter(|w| MARKS.iter().any(|m| w.contains(m)))
            .cloned()
            .collect()
    }

    fn filters(n: usize) -> String {
        let params: Vec<String> = (0..n).map(|i| format!("{{ name: f{i} }}")).collect();
        let reads: Vec<String> = (0..n).map(|i| format!(":f{i}")).collect();
        format!(
            "title: P\nparams: [{}]\nquery: SELECT {} AS a\n",
            params.join(", "),
            reads.join(" || ")
        )
    }

    #[test]
    fn more_than_six_filters_warn_and_six_do_not() {
        let (warns, loaded) = layout_of(&filters(7));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.starts_with(
                ".oxplow/extensions/x/lenses/p.yaml: 7 filters; a page shows 6 at most — fold"
            )),
            "{warns:?}"
        );
        assert!(layout_of(&filters(6)).0.is_empty());
    }

    fn choice(n: usize) -> String {
        let options: Vec<String> = (0..n)
            .map(|i| format!("{{ value: v{i}, label: V{i} }}"))
            .collect();
        format!(
            "title: P\nparams: [{{ name: c, default: v0, options: [{}] }}]\nquery: SELECT :c AS a\n",
            options.join(", ")
        )
    }

    #[test]
    fn a_choice_of_more_than_twelve_options_warns() {
        let (warns, loaded) = layout_of(&choice(13));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w
                .ends_with("param `c` has 13 options — use a `search`, or a lens that lists them")),
            "{warns:?}"
        );
        assert!(layout_of(&choice(12)).0.is_empty());
    }

    #[test]
    fn a_param_no_query_reads_warns_and_a_token_is_whole() {
        let (warns, loaded) = layout_of(
            "title: P\nparams: [{ name: q }, { name: r }]\nquery: SELECT :q AS a, :r_x AS b\n",
        );
        assert!(loaded);
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(
            warns[0].ends_with(
                "param `r` isn't read by any query of the page — remove it, or read it as `:r`"
            ),
            "{warns:?}"
        );
        // `:q` at the end of the text counts.
        assert!(
            layout_of("title: P\nparams: [{ name: q }]\nquery: SELECT 1 AS a WHERE 1 = :q\n")
                .0
                .is_empty()
        );
    }

    /// A param a column `select`s or `highlight`s is in use: a click sets
    /// it, or the cells mark it, whether or not this lens's query reads it.
    #[test]
    fn a_param_a_column_selects_or_highlights_is_read() {
        let (warns, loaded) = layout_of(
            "title: P\nparams: [{ name: commit }, { name: q }]\nquery: SELECT 'a' AS sha, 'b' AS s\ncolumns: [{ key: sha, select: commit }, { key: s, highlight: q }]\n",
        );
        assert!(loaded);
        assert!(warns.is_empty(), "{warns:?}");
    }

    /// A dashboard whose rows are tiles windows its metric cards by the
    /// page's `range` and `branch`: the cards read them, not the query.
    #[test]
    fn a_tile_dashboards_range_and_branch_are_read_by_its_metric_cards() {
        let (warns, loaded) = layout_of(
            "title: P\nparams: [{ name: range, type: date_range, role: filter }, { name: branch, type: branch, role: filter }, { name: z, role: filter }]\nquery: SELECT 'a' AS id, 'x' AS sql, 'metric' AS display\nviz: dashboard\ntile: { id: id, sql: sql, display: display }\n",
        );
        assert!(loaded);
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(warns[0].contains("param `z`"), "{warns:?}");
    }

    /// What a deployed extension reports names its source files, the
    /// ones to edit, not the copy in the store.
    #[test]
    fn a_deployed_extension_s_warnings_name_its_source() {
        let (d, _) = load_x(
            &[(
                "ok",
                "title: OK\nparams: [{ name: z }]\nquery: SELECT 1 AS a\n",
            )],
            "",
        );
        let source = d.path().join(".oxplow/extensions/x");
        let hash = crate::extension_store::source_hash(&source).unwrap();
        crate::extension_store::copy_into_store(d.path(), "x", &source, &hash).unwrap();
        let mut settings = oxplow_config::extension_settings(d.path());
        settings.personal.enable.push("x".into());
        let live = BTreeMap::from([("x".to_string(), hash.clone())]);
        let ext = load_deployed(d.path(), &settings, &live)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let unread: Vec<&String> = ext
            .warnings
            .iter()
            .filter(|w| w.contains("param `z`"))
            .collect();
        assert_eq!(unread.len(), 1, "{:?}", ext.warnings);
        assert!(
            unread[0].starts_with(".oxplow/extensions/x/lenses/ok.yaml: "),
            "{unread:?}"
        );
        assert!(
            ext.warnings.iter().all(|w| !w.contains(&hash)),
            "{:?}",
            ext.warnings
        );
    }

    #[test]
    fn a_range_is_read_through_its_from_or_to() {
        let body = |read: &str| {
            format!("title: P\nparams: [{{ name: range, type: date_range, default: all }}]\nquery: SELECT 1 AS a WHERE 1 {read}\n")
        };
        assert!(layout_of(&body("OR :range_from IS NULL")).0.is_empty());
        assert!(layout_of(&body("OR :range_to IS NULL")).0.is_empty());
        assert!(layout_of(&body("OR :range = 'all'")).0.is_empty());
    }

    #[test]
    fn a_part_lens_query_and_an_action_input_read_a_param() {
        let (_d, ext) = load_x(
            &[
                ("n", "title: N\nparams: [{ name: q }]\nquery: SELECT :q AS n\nviz: number\n"),
                ("doc", "title: D\nviz: document\nparams: [{ name: q }]\nsections: [{ lens: n }]\n"),
                (
                    "act",
                    "title: A\nparams: [{ name: k }]\nquery: SELECT 1 AS a\nactions:\n  - { id: go, label: Go, command: oxplow.work_item.transition, input: { uri: '{{param.k}}' } }\n",
                ),
            ],
            "",
        );
        assert!(
            ext.warnings.iter().all(|w| !w.contains("isn't read")),
            "{:?}",
            ext.warnings
        );
    }

    #[test]
    fn a_long_description_warns() {
        let body = |n: usize| {
            format!(
                "title: P\ndescription: {}\nquery: SELECT 1 AS a\n",
                "d".repeat(n)
            )
        };
        let (warns, loaded) = layout_of(&body(201));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.ends_with("the description is 201 characters — keep it to a sentence or two; put the rest in a markdown section")),
            "{warns:?}"
        );
        assert!(layout_of(&body(200)).0.is_empty());
    }

    #[test]
    fn a_document_of_more_than_eight_sections_warns() {
        let doc = |n: usize| {
            let parts: Vec<String> = (0..n)
                .map(|i| format!("{{ id: s{i}, markdown: hi }}"))
                .collect();
            format!(
                "title: P\nviz: document\nsections: [{}]\n",
                parts.join(", ")
            )
        };
        let (warns, loaded) = layout_of(&doc(9));
        assert!(loaded);
        assert!(
            warns
                .iter()
                .any(|w| w.ends_with("9 sections — a page reads best with 8 at most; split it")),
            "{warns:?}"
        );
        assert!(layout_of(&doc(8)).0.is_empty());
    }

    #[test]
    fn a_table_of_more_than_eight_columns_warns() {
        let table = |n: usize| {
            let cols: Vec<String> = (0..n).map(|i| format!("  - {{ key: c{i} }}")).collect();
            let sel: Vec<String> = (0..n).map(|i| format!("1 AS c{i}")).collect();
            format!(
                "title: P\nquery: SELECT {}\ncolumns:\n{}\n",
                sel.join(", "),
                cols.join("\n")
            )
        };
        let (warns, loaded) = layout_of(&table(9));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.ends_with(
                "9 columns — show 8 at most; put the rest in a section a row `select`s"
            )),
            "{warns:?}"
        );
        assert!(layout_of(&table(8)).0.is_empty());
    }

    #[tokio::test]
    async fn validate_returns_the_layout_warning() {
        let (d, _) = load_x(&[("p", &filters(7))], "");
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            v.warnings.iter().any(|w| w.contains("7 filters")),
            "{:?}",
            v.warnings
        );
        assert!(v.lenses.iter().any(|l| l.slug == "p"));
    }

    /// What the app ships follows its own page-layout rules.
    #[test]
    fn the_bundled_lenses_have_no_layout_warning() {
        let dir = tempfile::tempdir().unwrap();
        let exts = load_extensions(dir.path());
        let bundled: Vec<&Extension> = exts.iter().filter(|e| e.origin == "bundled").collect();
        assert_eq!(bundled.len(), 2);
        for e in bundled {
            let layout = layout_only(&e.warnings);
            assert!(layout.is_empty(), "{}: {layout:#?}", e.name);
        }
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
        let task_lens = "title: T\nparams: [{ name: path }]\nquery: SELECT :path\n";
        let thread_lens = "title: Th\nparams: [{ name: stream_id }]\nquery: SELECT :stream_id\n";
        let (_d, ext) = load_x(
            &[("t", task_lens), ("th", thread_lens), ("plain", "title: P\nquery: SELECT 1\n")],
            "ui:\n  slots:\n    - { slot: diff.file.header, lens: t }\n    - { slot: diff.file.header, lens: th }\n    - { slot: diff.file.header, lens: plain }\n",
        );
        let mounted: Vec<(&str, &str)> = ext
            .ui
            .slots
            .iter()
            .map(|s| (s.slot.as_str(), s.lens_id.as_str()))
            .collect();
        assert_eq!(
            mounted,
            vec![("diff.file.header", "x/t"), ("diff.file.header", "x/th")]
        );
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`path`") && errs.contains("plain"), "{errs}");
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
            ".oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/partners.yaml",
            "title: Partners\nparams: [{ name: path }]\nquery: SELECT :path AS path\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/plain.yaml",
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
    fn this_repos_own_extension_loads_clean() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ext = project_extensions(&repo)
            .into_iter()
            .find(|e| e.name == "project")
            .expect("the repo's own extension");
        assert_eq!(ext.errors, Vec::<String>::new());
        assert!(ext.is_own());
        assert!(ext
            .collectors
            .iter()
            .any(|c| c.id == "tests.rust_junit" && c.records.is_some()));
        assert!(ext.collectors.iter().any(|c| c.id == "repo.scan_clone"));
    }

    #[test]
    fn an_extension_metric_names_its_own_collector_and_keeps_out_of_oxplow_keys() {
        let (_d, ext) = load_x(
            &[],
            "measures:\n  - { key: oxplow.loc, title: Lines }\n\
             metrics:\n  - { key: acme.loc, title: Lines, sourceMeasure: acme.x, aggregation: sum, collector: acme.nope }\n",
        );
        assert!(ext.metrics.is_empty(), "{:?}", ext.metrics);
        for needle in [
            "uses the reserved",
            "collector `acme.nope` is not a collector",
        ] {
            assert!(
                ext.errors.iter().any(|e| e.contains(needle)),
                "{needle}: {:?}",
                ext.errors
            );
        }
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
            ".oxplow/extensions/x/extension.yaml",
            &format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest}"),
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert_eq!(ext.measures.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.metrics.len(), 1, "{:?}", ext.errors);
        // A missing script or program is refused; so is a `use:`, which
        // only the project's config can write.
        assert!(ext.collectors.iter().all(|c| c.id == "acme.stray"));
        for (needle, also) in [
            ("acme.missing", "collectors/nope.star"),
            ("acme.shell", "collectors/todo.sh"),
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
            "    description: Blocked work items.",
            "    columns:",
            "      - { name: uri, type: TEXT, doc: Work item ref. }",
            "  - { name: gone, version: 1, description: No file., columns: [] }",
            "",
        ]
        .join("\n");
        write(dir.path(), ".oxplow/extensions/x/extension.yaml", &manifest);
        write(
            dir.path(),
            ".oxplow/extensions/x/models/late.sql",
            "SELECT uri FROM ref('work_item') WHERE state = 'blocked'",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/models/stray.sql",
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
            errors.contains(".oxplow/extensions/x/models/gone.sql is missing"),
            "{errors}"
        );

        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            &manifest.replace(
                "  - { name: gone, version: 1, description: No file., columns: [] }\n",
                "",
            ),
        );
        std::fs::remove_file(dir.path().join(".oxplow/extensions/x/models/stray.sql")).unwrap();
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.models.len(), 1);
        assert_eq!(ext.models[0].file, ".oxplow/extensions/x/models/late.sql");
        assert!(ext.models[0].sql.contains("ref('work_item')"));

        write(
            dir.path(),
            ".oxplow/extensions/x/models/stray.sql",
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
    fn extensions_declare_dimensions() {
        let manifest = [
            "dimensions:",
            "  - { key: acme.team, label: Team }",
            "  - { key: acme.prio, entity: v_work_item, expr: \"json_extract(e.native, '$.priority')\" }",
            "  - { key: acme.hot, promote: true }",
            "",
        ]
        .join("\n");
        let (d, _) = load_x(&[], &manifest);
        let ext = project_extensions(d.path()).remove(0);
        let keys: Vec<_> = ext
            .dimensions
            .iter()
            .filter_map(|d| d.key.clone())
            .collect();
        assert_eq!(
            keys,
            vec!["acme.team", "acme.prio", "acme.hot"],
            "{:?}",
            ext.errors
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    }

    #[test]
    fn a_project_extension_declares_a_program_collector() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\nmeasures:\n  - { key: acme.todo }\ncollectors:\n  - { id: acme.shell, runtime: exec, entry: collectors/todo.sh, facts: [acme.todo] }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.sh",
            "echo",
        );
        let own = project_extensions(dir.path()).remove(0);
        assert!(own.is_own());
        assert!(
            own.collectors.iter().any(|c| c.id == "acme.shell"),
            "{:?}",
            own.errors
        );
    }

    #[test]
    fn an_extension_fact_collector_reads_its_own_script() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\npermissions: { files: read }\nmeasures:\n  - { key: acme.todo }\ncollectors:\n  - { id: acme.todo_scan, runtime: starlark, entry: collectors/todo.star, trigger: { on: [snapshot.taken] }, facts: [acme.todo] }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.collectors.len(), 1);
        assert_eq!(
            read_extension_file(dir.path(), &ext.path, "collectors/todo.star").as_deref(),
            Some("def transform(input):\n    return {\"facts\": []}\n")
        );
        assert_eq!(
            read_extension_file(dir.path(), &ext.path, "collectors/none.star"),
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/rows.yaml",
            "title: Rows\nparams: [{ name: n, default: 0 }]\nquery: SELECT value FROM json_each('[1,2,3]') WHERE value <= :n\nalert: { min_rows: 2 }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/cov.yaml",
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
                open: None,
                collapsed: None,
                count: None,
            }]
        );
        // A panel may name the page its header opens (a core page,
        // as the Comments panel opens the inbox); without one, its body lens.
        let (_d, ext) = load_x(
            &lenses,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: c, title: C, scope: project, body: quiet, open: 'oxplow:oxplow/page?page=comments' }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.panels[0].open.as_deref(),
            Some("oxplow:oxplow/page?page=comments")
        );
        for (panel, says) in [
            (
                "{ id: a, title: A, scope: project, body: quiet, open: comments }",
                "`open` must be a page URI",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, open: 'task:1' }",
                "`open` must be a page URI",
            ),
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

    /// A panel may name a lens its collapsed header shows (one
    /// line, compact) and a lens whose row count (or `number`) is its
    /// header count — checked like `body` and `badge`.
    #[test]
    fn panels_summarize_with_collapsed_and_count_lenses() {
        let lenses = [
            ("open", "title: Open\nquery: SELECT 1\nparams: [{ name: stream_id, default: '' }]\n"),
            ("line", "title: Line\nquery: SELECT 1\nparams: [{ name: stream_id, default: '' }]\n"),
            ("n", "title: N\nquery: SELECT 1\nviz: number\nparams: [{ name: stream_id, default: '' }]\n"),
            ("quiet", "title: Quiet\nquery: SELECT 1\n"),
        ];
        let (_d, ext) = load_x(
            &lenses,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: w, title: W, scope: stream, body: open, collapsed: line, count: n }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.panels[0].collapsed.as_deref(), Some("x/line"));
        assert_eq!(ext.panels[0].count.as_deref(), Some("x/n"));
        for (panel, says) in [
            (
                "{ id: a, title: A, scope: project, body: quiet, collapsed: nope }",
                "its collapsed lens `nope` isn't in lenses/",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, count: nope }",
                "its count lens `nope` isn't in lenses/",
            ),
            (
                "{ id: a, title: A, scope: stream, body: open, collapsed: quiet }",
                "lens `quiet` must declare `stream_id`",
            ),
            (
                "{ id: a, title: A, scope: stream, body: open, count: quiet }",
                "lens `quiet` must declare `stream_id`",
            ),
        ] {
            let (_d, ext) = load_x(
                &lenses,
                &format!("manifest: 2\nintent:\n  purpose: p\npanels:\n  - {panel}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{panel}: {errs}");
            assert!(ext.panels.is_empty(), "{panel}");
        }
    }

    /// A list or table can group its rows under headings
    /// (`group`), highlight one (`emphasis`), indent some (`depth`), and
    /// draw a cell's icon and tone from other columns; an action can sit in
    /// a group's heading. Shapes are checked at load; the columns they
    /// name, against the result by `oxplow_validate_extension`.
    #[test]
    fn lenses_group_and_style_their_rows() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nviz: list\nquery: SELECT 1 AS id, 'Ready' AS bucket, 'oxplow:oxplow/page?page=tasks' AS bucket_ref, 1 AS current, 0 AS indent, 'done' AS glyph, 'success' AS hue\nparams: [{ name: stream_id, default: '' }]\ngroup: { by: bucket, link: { kind: page, from: bucket_ref } }\nemphasis: current\ndepth: indent\ncolumns:\n  - { key: id, icon: glyph, tone: hue }\nactions:\n  - { id: add, label: '+', command: oxplow.work_item.create, group: Ready, input: { stream: '{{param.stream_id}}' } }\n",
                ),
                ("t", "title: T\nviz: tree\nquery: SELECT 1 AS id, NULL AS p, 'x' AS l, 1 AS on_\ntree: { id: id, parent: p, label: l }\nemphasis: on_\n"),
                ("b", "title: B\nviz: tree\nquery: SELECT 1 AS id, NULL AS p, 'x' AS l\ntree: { id: id, parent: p, label: l }\ngroup: { by: l }\n"),
                ("c", "title: C\nviz: list\nquery: SELECT 1\ngroup: { link: { kind: page } }\n"),
                ("d", "title: D\nviz: number\nquery: SELECT 1\ndepth: n\n"),
                ("e", "title: E\nviz: list\nquery: SELECT 1\nactions: [{ id: x, label: X, command: a.b.c, group: Ready }]\n"),
                ("f", "title: F\nviz: list\nquery: SELECT 1\ngroup: { by: g }\nactions: [{ id: x, label: X, command: a.b.c, group: Ready, row: true }]\n"),
                ("g", "title: G\nviz: bar\nquery: SELECT 1 AS x, 2 AS y\nchart: { x: x, y: y }\nemphasis: x\n"),
            ],
            "",
        );
        let a = ext
            .lenses
            .iter()
            .find(|l| l.slug == "a")
            .expect("a grouped, styled list loads");
        assert_eq!(
            a.group,
            Some(LensGroup {
                by: "bucket".into(),
                link: Some(LensLink {
                    kind: LensLinkKind::Page,
                    from: Some("bucket_ref".into()),
                    line: None,
                    base: None,
                    head: None,
                }),
                values: Vec::new(),
            })
        );
        assert_eq!(a.emphasis.as_deref(), Some("current"));
        assert_eq!(a.depth.as_deref(), Some("indent"));
        assert_eq!(a.columns[0].icon.as_deref(), Some("glyph"));
        assert_eq!(a.columns[0].tone.as_deref(), Some("hue"));
        assert_eq!(a.actions[0].group.as_deref(), Some("Ready"));
        assert!(
            ext.lenses.iter().any(|l| l.slug == "t"),
            "a tree may emphasize"
        );
        for (slug, needle) in [
            ("b", "`group` is only for `list`, `table` and `board`"),
            ("c", "missing field `by`"),
            ("d", "`depth` is only for `list` and `table`"),
            ("e", "`group: Ready` needs the lens to have a `group`"),
            ("f", "a group action isn't a row action"),
            ("g", "`emphasis` is only for `list`, `table` and `tree`"),
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

    /// The columns `group`, `emphasis`, `depth` and a column's `icon` /
    /// `tone` name must be in the result (`oxplow_validate_extension`).
    #[tokio::test]
    async fn validate_checks_row_style_columns() {
        let (d, _) = load_x(
            &[(
                "a",
                "title: A\nviz: table\nquery: SELECT 1 AS id\ngroup: { by: g1, link: { kind: page, from: g2 } }\nemphasis: e1\ndepth: d1\ncolumns:\n  - { key: id, icon: i1, tone: t1 }\n",
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        for k in ["g1", "g2", "e1", "d1", "i1", "t1"] {
            assert!(errs.contains(&format!("`{k}`")), "{k}: {errs}");
        }
    }

    /// A column's `highlight` names a param the lens declares: the text of
    /// the param's value is marked wherever the cell has it.
    #[test]
    fn a_columns_highlight_names_a_declared_param() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 'x' AS t\nparams: [{ name: filter, default: '' }]\ncolumns:\n  - { key: t, highlight: filter }\n",
                ),
                (
                    "b",
                    "title: B\nquery: SELECT 'x' AS t\ncolumns:\n  - { key: t, highlight: nope }\n",
                ),
            ],
            "",
        );
        let a = ext.lenses.iter().find(|l| l.slug == "a").expect("loads");
        assert_eq!(a.columns[0].highlight.as_deref(), Some("filter"));
        assert!(ext.lenses.iter().all(|l| l.slug != "b"));
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("b.yaml") && e.contains("`highlight: nope`")),
            "{:?}",
            ext.errors
        );
    }

    /// P6.B1: an action is a command. Its placeholders must name a param
    /// (or, in a row action, a column); anything else is refused.
    #[test]
    fn lens_actions_are_commands() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 1 AS id\nparams: [{ name: item, default: '' }]\nactions:\n  - { id: finish, label: Finish, command: oxplow.work_item.transition, input: { uri: '{{param.item}}', to: done } }\n  - { id: row, label: Row, command: oxplow.work_item.transition, row: true, input: { uri: 'work_item:oxplow:tsk{{row.id}}', to: done } }\n",
                ),
                ("b", "title: B\nquery: SELECT 1\nactions: [copy]\n"),
                ("c", "title: C\nquery: SELECT 1\nactions: [{ action: run-source, source: a/b }]\n"),
                ("d", "title: D\nquery: SELECT 1\nactions: [{ id: x, label: X, command: oxplow.work_item.create, input: { title: '{{param.nope}}' } }]\n"),
                ("e", "title: E\nquery: SELECT 1\nactions: [{ id: x, label: X, command: oxplow.work_item.create, input: { title: '{{row.id}}' } }]\n"),
                ("f", "title: F\nquery: SELECT 1\nactions: [{ id: x, label: X, command: Not-A-Name }]\n"),
                ("g", "title: G\nquery: SELECT 1\nactions: [{ id: x, label: X, command: a.b.c }, { id: x, label: Y, command: a.b.d }]\n"),
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
                ("finish", "oxplow.work_item.transition", false),
                ("row", "oxplow.work_item.transition", true)
            ]
        );
        for (slug, needle) in [
            ("b", "actions:"),
            ("c", "unknown field `action`"),
            ("d", "names no param"),
            ("e", "needs `row: true`"),
            ("f", "command id"),
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

    /// What becomes of the page after an action, or a form lens's submit:
    /// `then: open` (default) | `replace` | `stay` | `close`.
    #[test]
    fn an_action_says_what_becomes_of_its_page() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    "title: A\nquery: SELECT 1\nactions:\n  - { id: plain, label: P, command: a.b.c }\n  - { id: gone, label: G, command: a.b.c, then: replace }\n  - { id: again, label: A, command: a.b.c, then: stay }\n",
                ),
                (
                    "f",
                    "title: F\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, then: replace }\n",
                ),
                ("b", "title: B\nquery: SELECT 1\nactions: [{ id: x, label: X, command: a.b.c, then: away }]\n"),
            ],
            "",
        );
        let a = ext.lenses.iter().find(|l| l.slug == "a").expect("a loads");
        assert_eq!(
            a.actions.iter().map(|x| x.then).collect::<Vec<_>>(),
            vec![ActionThen::Open, ActionThen::Replace, ActionThen::Stay]
        );
        let f = ext.lenses.iter().find(|l| l.slug == "f").expect("f loads");
        assert_eq!(f.form.as_ref().map(|x| x.then), Some(ActionThen::Replace));
        assert!(ext.lenses.iter().all(|l| l.slug != "b"));
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("b.yaml") && e.contains("away")),
            "{:?}",
            ext.errors
        );
    }

    /// An edit form says so; a field may show only for some values of
    /// another and may be secret; each refusal names the form's problem.
    #[test]
    fn a_form_edits_shows_fields_by_value_and_keeps_secrets() {
        let (_d, ext) = load_x(
            &[
                (
                    "e",
                    "title: E\nquery: SELECT 1\nviz: form\nform:\n  command: a.b.c\n  mode: edit\n  fields:\n    - kind\n    - { name: url, visible_if: { kind: [http, https] } }\n    - { name: token, secret: true, visible_if: { kind: https } }\n",
                ),
                ("p", "title: P\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [title] }\n"),
                (
                    "self",
                    "title: S\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [{ name: a, visible_if: { a: x } }] }\n",
                ),
                (
                    "unknown",
                    "title: U\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [a, { name: b, visible_if: { gone: x } }] }\n",
                ),
                (
                    "fixedsecret",
                    "title: F\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, defaults: { a: 1 }, fixed: [a], fields: [{ name: a, secret: true }] }\n",
                ),
                ("nofields", "title: N\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, mode: edit }\n"),
                (
                    "again",
                    "title: A\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [a, { name: n, from: x/again }], again: { label: Again, keep: [a, n] } }\n",
                ),
                (
                    "keepgone",
                    "title: K\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [a], again: { label: Again, keep: [gone] } }\n",
                ),
                (
                    "fromgone",
                    "title: G\nquery: SELECT 1\nviz: form\nform: { command: a.b.c, fields: [{ name: n, from: x/nothing }] }\n",
                ),
            ],
            "",
        );
        let e = ext.lenses.iter().find(|l| l.slug == "e").expect("e loads");
        let form = e.form.as_ref().expect("a form");
        assert_eq!(form.mode, FormMode::Edit);
        let fields = form.fields.as_ref().expect("fields");
        assert_eq!(
            fields
                .iter()
                .map(|f| (f.name(), f.secret()))
                .collect::<Vec<_>>(),
            vec![("kind", false), ("url", false), ("token", true)]
        );
        assert_eq!(
            fields[1].visible_if().get("kind"),
            Some(&FieldValues::Any(vec![
                FieldValue::Text("http".into()),
                FieldValue::Text("https".into())
            ]))
        );
        let p = ext.lenses.iter().find(|l| l.slug == "p").expect("p loads");
        assert_eq!(p.form.as_ref().map(|f| f.mode), Some(FormMode::Create));
        let said = |slug: &str| {
            ext.errors
                .iter()
                .find(|e| e.contains(&format!("{slug}.yaml")))
                .cloned()
                .unwrap_or_default()
        };
        assert!(
            said("self").contains("`a` can't show by its own value"),
            "{:?}",
            ext.errors
        );
        assert!(
            said("unknown").contains("`visible_if` names `gone`, which isn't one of its `fields`"),
            "{:?}",
            ext.errors
        );
        assert!(
            said("fixedsecret").contains("`a` can't be both fixed and secret"),
            "{:?}",
            ext.errors
        );
        assert!(
            said("nofields").contains("`mode: edit` needs `fields`"),
            "{:?}",
            ext.errors
        );
        assert!(
            said("keepgone").contains("`again.keep` names `gone`"),
            "{:?}",
            ext.errors
        );
        assert!(
            said("fromgone").contains("`from` lens `x/nothing` isn't in this extension"),
            "{:?}",
            ext.errors
        );
        for slug in [
            "self",
            "unknown",
            "fixedsecret",
            "nofields",
            "keepgone",
            "fromgone",
        ] {
            assert!(ext.lenses.iter().all(|l| l.slug != slug), "{slug} loaded");
        }
        let again = ext.lenses.iter().find(|l| l.slug == "again");
        let form = again.and_then(|l| l.form.as_ref()).expect("again loads");
        assert_eq!(form.again.as_ref().unwrap().keep, ["a", "n"]);
        assert_eq!(form.fields.as_ref().unwrap()[1].from(), Some("x/again"));
    }

    /// A row action's `{{row.x}}` must be a column the query returns.
    #[tokio::test]
    async fn validate_checks_row_action_columns() {
        let (d, _) = load_x(
            &[(
                "a",
                "title: A\nquery: SELECT 1 AS id\nactions: [{ id: x, label: X, command: a.b.c, row: true, input: { v: '{{row.gone}}' } }]\n",
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
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [review, oxplow-bundled]\n",
        );
        let exts = load_extensions(dir.path());
        for name in ["review", "oxplow-bundled"] {
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
            "permissions: { agent: true }\nadvisories:\n  - id: low-coverage\n    on: post-tool-use\n    once_per: effort\n    query: SELECT 'add tests' AS message WHERE :effort_id IS NOT NULL\n  - id: crossings\n    on: prompt\n    once_per: row\n    heading: '# Metric thresholds'\n    query: SELECT 'x' AS message, 'k' AS key\n  - id: nokey\n    on: prompt\n    once_per: row\n    query: SELECT 'x' AS message\n  - id: Bad Id\n    on: prompt\n    query: SELECT 1\n",
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
                    "title: F\nparams: [{ name: path }]\nquery: SELECT 1\ncolumns:\n  - { key: path, link: { kind: diff-at, line: ln, base: b, head: h } }\n  - { key: dup, link: { kind: compare, head: h } }\n",
                ),
                ("both", "title: B\nparams: [{ name: left_revision }]\nquery: SELECT 1\n"),
                ("none", "title: N\nparams: [{ name: other }]\nquery: SELECT 1\n"),
            ],
            "ui:\n  slots:\n    - { slot: diff.file.header, lens: files }\n    - { slot: diff.file.header, lens: both }\n    - { slot: diff.file.header, lens: none }\n",
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
                ("diff.file.header", "x/files"),
                ("diff.file.header", "x/both")
            ],
            "a slot lens needs at least one of the slot's params"
        );
        let errs = ext.errors.join("\n");
        assert!(errs.contains("none") && errs.contains("path"), "{errs}");
    }

    /// A page's `label` is what its tab says, `{{param.<name>}}` filled from
    /// its URI: a renderer's names a param of what it renders, a route's
    /// one of its `params`. Its `title` stays its name (launcher, Open with).
    #[test]
    fn a_pages_label_names_its_params() {
        let lenses = [(
            "pr",
            "title: PR\nparams: [{ name: uri }, { name: id }]\nquery: SELECT :id\n",
        )];
        let resources =
            "resources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\n";
        let (_d, ext) = load_x(
            &lenses,
            &format!("{resources}pages:\n  - {{ id: pr, title: PR, label: 'PR {{{{param.id}}}}', lens: pr, for: x/pr, default: true }}\n  - {{ id: route, title: Route, label: '{{{{ param.id }}}}', params: [id], lens: pr }}\n"),
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let page = |id: &str| ext.pages.iter().find(|p| p.id == id).unwrap();
        assert_eq!(page("pr").label.as_deref(), Some("PR {{param.id}}"));
        assert_eq!(page("pr").title, "PR");
        for (page, says) in [
            (
                "{ id: a, title: A, label: '{{param.nope}}', lens: pr, for: x/pr }",
                "`label` names `nope`, which isn't a param of `x/pr`",
            ),
            (
                "{ id: a, title: A, label: '{{param.nope}}', params: [id], lens: pr }",
                "`label` names `nope`, which isn't one of its `params`",
            ),
            (
                "{ id: a, title: A, label: '{{nope.id}}', params: [id], lens: pr }",
                "`label` fills only `{{param.<name>}}`",
            ),
        ] {
            let (_d, ext) = load_x(&lenses, &format!("{resources}pages:\n  - {page}\n"));
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{page}: {errs}");
        }
    }

    /// A page's `label` may also name a column of its lens's row
    /// (`{{row.<col>}}`) when that lens is a document with a `query:` (the
    /// row its `columns:` name), and apply a filter (`short`, `basename`);
    /// anything else is refused, and the column the label reads is not
    /// "unread" in the document's row.
    #[test]
    fn a_pages_label_may_name_the_row_of_a_document_and_filters() {
        let doc = "title: D\nparams: [{ name: uri }, { name: id }]\nquery: SELECT :id AS id, 'T' AS title\ncolumns: [{ key: id }, { key: title }]\nviz: document\nsections: [{ id: p, lens: part }]\n";
        let lenses = [
            ("doc", doc),
            (
                "part",
                "title: P\nparams: [{ name: id }]\nquery: SELECT :id\n",
            ),
            (
                "table",
                "title: T\nparams: [{ name: uri }, { name: id }]\nquery: SELECT :id AS title\n",
            ),
        ];
        let resources =
            "resources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\n";
        let page = |p: &str| format!("{resources}pages:\n  - {p}\n");
        let (_d, ext) = load_x(
            &lenses,
            &page("{ id: a, title: A, label: '{{row.title}} {{param.id|short}}', lens: doc, for: x/pr, default: true }"),
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(
            !ext.warnings.iter().any(|w| w.contains("`title`")),
            "a column the label reads is read: {:?}",
            ext.warnings
        );
        for (p, says) in [
            (
                "{ id: a, title: A, label: '{{row.title}}', lens: table, for: x/pr }",
                "`row.title`, which only a document with a `query:` has",
            ),
            (
                "{ id: a, title: A, label: '{{row.nope}}', lens: doc, for: x/pr }",
                "`label` names `row.nope`, which isn't a column of its lens's row",
            ),
            (
                "{ id: a, title: A, label: '{{param.id|loud}}', lens: doc, for: x/pr }",
                "`label` filter `loud`",
            ),
        ] {
            let (_d, ext) = load_x(&lenses, &page(p));
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{p}: {errs}");
        }
    }

    /// A renderer's lens is run for a URI: each param it declares that the
    /// URI carries takes the URI's value (one supplied wins). A lens that
    /// renders nothing keeps its `uri` param to itself (a work-item picker).
    #[tokio::test]
    async fn a_renderers_lens_takes_its_uris_params() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\nresources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\npages:\n  - { id: pr, title: PR, lens: pr, for: x/pr, default: true }\n",
        );
        for slug in ["pr", "picker"] {
            write(
                dir.path(),
                &format!(".oxplow/extensions/x/lenses/{slug}.yaml"),
                "title: L\nparams: [{ name: uri }, { name: id }]\nquery: SELECT :id AS id\n",
            );
        }
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        crate::test_fixtures::activate(&svc).await;
        let id_of = |lens: &'static str, params: &[(&str, &str)]| {
            let svc = svc.clone();
            let params: BTreeMap<String, SqlCell> = params
                .iter()
                .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
                .collect();
            async move {
                let run = run_lens(
                    &svc.sql,
                    &svc.extension_catalog,
                    &svc.layout.project_dir,
                    lens,
                    params,
                    &Default::default(),
                )
                .await
                .unwrap();
                run.params.get("id").cloned()
            }
        };
        let uri = "oxplow:x/pr?id=7";
        assert_eq!(
            id_of("x/pr", &[("uri", uri)]).await,
            Some(SqlCell::Text("7".into()))
        );
        assert_eq!(
            id_of("x/pr", &[("uri", uri), ("id", "9")]).await,
            Some(SqlCell::Text("9".into()))
        );
        assert_eq!(
            id_of("x/picker", &[("uri", uri)]).await,
            Some(SqlCell::Null(()))
        );
    }

    /// A page that renders resources says which (`for`), which of their
    /// URIs it takes (`accepts`), and whether it is what they open on
    /// (`default`) — only for the extension's own resources.
    #[test]
    fn a_page_renders_the_resources_it_is_for() {
        let lenses = [
            (
                "pr",
                "title: PR\nparams: [{ name: uri }]\nquery: SELECT 1\n",
            ),
            ("plain", "title: Plain\nquery: SELECT 1\n"),
            (
                "wide",
                "title: Wide\nparams: [{ name: uri }, { name: id }, { name: stream_id }, { name: q, role: filter }, { name: nope }]\nquery: SELECT :id, :stream_id, :q, :nope\n",
            ),
            (
                "fits",
                "title: Fits\nparams: [{ name: uri }, { name: id }, { name: q, role: filter }]\nquery: SELECT :id, :q\n",
            ),
        ];
        let resources =
            "resources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\n";
        let (_d, ext) = load_x(
            &lenses,
            &format!("{resources}pages:\n  - {{ id: pr, title: PR, lens: pr, for: x/pr, default: true }}\n  - {{ id: pr-raw, title: Raw, lens: pr, for: x/pr, accepts: {{ id: '^\\d+$' }} }}\n  - {{ id: both, title: Both, lens: pr, for: [x/pr, oxplow/file] }}\n  - {{ id: web, title: Web, lens: pr, for: https, pinned: true }}\n"),
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let page = |id: &str| ext.pages.iter().find(|p| p.id == id).unwrap();
        assert_eq!(page("pr").renders, vec!["x/pr".to_string()]);
        assert!(page("pr").default);
        assert_eq!(
            page("both").renders,
            vec!["x/pr".to_string(), "oxplow/file".to_string()]
        );
        assert_eq!(
            page("pr-raw").accepts.get("id").map(String::as_str),
            Some("^[0-9]+$")
        );
        assert!(page("web").pinned);

        for (page, says) in [
            (
                "{ id: a, title: A, lens: pr, for: nope }",
                "isn't a resource",
            ),
            (
                "{ id: a, title: A, lens: pr, for: x/missing }",
                "isn't one of this extension's",
            ),
            (
                "{ id: a, title: A, lens: pr, for: oxplow/missing }",
                "isn't a resource oxplow-foundation declares",
            ),
            (
                "{ id: a, title: A, lens: pr, for: x/pr, accepts: { nope: present } }",
                "isn't a param of `x/pr`",
            ),
            (
                "{ id: a, title: A, lens: pr, for: x/pr, accepts: { id: '[0-9]+' } }",
                "an id pattern",
            ),
            (
                "{ id: a, title: A, lens: pr, for: oxplow/file, default: true }",
                "is another's",
            ),
            (
                "{ id: a, title: A, lens: pr, default: true }",
                "are for a renderer",
            ),
            (
                "{ id: a, title: A, lens: plain, for: x/pr }",
                "declare a `uri` param",
            ),
            // It's run with its URI's params: one the resource hasn't, it
            // would never get (a filter is its view's, not the URI's).
            (
                "{ id: a, title: A, lens: wide, for: x/pr }",
                "takes `nope`, which isn't a param of `x/pr`",
            ),
            (
                "{ id: a, title: A, lens: fits, for: https }",
                "takes `id`, which isn't a param of `https`",
            ),
        ] {
            let (_d, ext) = load_x(&lenses, &format!("{resources}pages:\n  - {page}\n"));
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{page}: {errs}");
        }
        let (_d, ext) = load_x(
            &lenses,
            &format!("{resources}pages:\n  - {{ id: a, title: A, lens: pr, for: x/pr, default: true }}\n  - {{ id: b, title: B, lens: pr, for: x/pr, default: true }}\n"),
        );
        assert!(
            ext.errors.join("\n").contains("another default renderer"),
            "{:?}",
            ext.errors
        );
    }

    /// An extension's page is a lens (or one of its custom components) at
    /// `oxplow:oxplow/page?page=<extension>/<page>`, in the launcher under its category when it
    /// has one.
    #[test]
    fn pages_are_lenses_with_a_route() {
        let lenses = [
            ("open", "title: Open\nquery: SELECT 1\n"),
            (
                "waiting",
                "title: Waiting\nquery: SELECT 3 AS n\nviz: number\n",
            ),
        ];
        let (_d, ext) = load_x(
            &lenses,
            "manifest: 2\nintent:\n  purpose: p\npages:\n  - { id: open-prs, title: Open PRs, icon: git-pull-request, category: Work, keywords: pull requests, params: [author], count: waiting, lens: open }\n  - { id: detail, title: Detail, lens: open, comments: true }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.pages,
            vec![
                ExtensionPage {
                    id: "open-prs".into(),
                    extension: "x".into(),
                    page_ref: "oxplow:oxplow/page?page=x/open-prs".into(),
                    title: "Open PRs".into(),
                    label: None,
                    icon: Some("git-pull-request".into()),
                    category: Some(LauncherCategory::Work),
                    keywords: Some("pull requests".into()),
                    params: vec!["author".into()],
                    count: Some("x/waiting".into()),
                    renders: vec![],
                    accepts: Default::default(),
                    default: false,
                    pinned: false,
                    comments: false,
                    body: PageBody::Lens {
                        lens: "x/open".into()
                    },
                },
                ExtensionPage {
                    id: "detail".into(),
                    extension: "x".into(),
                    page_ref: "oxplow:oxplow/page?page=x/detail".into(),
                    title: "Detail".into(),
                    label: None,
                    icon: None,
                    category: None,
                    keywords: None,
                    params: vec![],
                    count: None,
                    renders: vec![],
                    accepts: Default::default(),
                    default: false,
                    pinned: false,
                    comments: true,
                    body: PageBody::Lens {
                        lens: "x/open".into()
                    },
                }
            ]
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
            (
                "{ id: a, title: A, icon: nope, category: Work, lens: open }",
                "icon",
            ),
            (
                "{ id: a, title: A }",
                "exactly one of `lens` and `component`",
            ),
            (
                "{ id: a, title: A, lens: open, component: c }",
                "exactly one of `lens` and `component`",
            ),
            (
                "{ id: a, title: A, component: \"oxplow:tasks\" }",
                "only oxplow's own extension",
            ),
            (
                "{ id: a, title: A, component: nope }",
                "isn't one of its custom_components or a built-in page",
            ),
            ("{ id: a, title: A, count: nope, lens: open }", "count lens"),
            (
                "{ id: a, title: A, params: [Bad], lens: open }",
                "param `Bad`",
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

    /// A page may be one of its extension's own custom components: the
    /// component owns the page body (a custom page). A name that isn't
    /// one of them is refused.
    #[test]
    fn a_page_may_be_one_of_its_custom_components() {
        let page = |pages: &str| {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                ".oxplow/extensions/x/extension.yaml",
                &format!(
                    "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - {{ id: board }}\npages:\n  - {pages}\n"
                ),
            );
            write(
                dir.path(),
                ".oxplow/extensions/x/components/board/index.html",
                "<!doctype html>",
            );
            (
                dir.path().to_path_buf(),
                project_extensions(dir.path()).remove(0),
                dir,
            )
        };
        let (_, ext, _d) =
            page("{ id: board, title: Board, category: Work, params: [from], component: board }");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.pages[0].body,
            PageBody::Component {
                component: "x/board".into()
            }
        );
        let (_, ext, _d) = page("{ id: board, title: Board, component: nope }");
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`nope` isn't one of its custom_components or a built-in page"),
            "{errs}"
        );
        assert!(ext.pages.is_empty());
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
        // Git sees an extension's source once it is shared.
        crate::extension_deploy::share(&f.svc, "acme").unwrap();
        let manifest = |purpose: &str| {
            format!(
                "manifest: 2\nname: acme\ndescription: {purpose}\nintent:\n  purpose: {purpose}\n  origin: thread:thr1\n  examples: []\n"
            )
        };
        let dir = ws.join(".oxplow/extensions/acme");
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
                path: ".oxplow/extensions/acme/extension.yaml".into(),
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

    /// An effort's review names the extensions that changed between
    /// its two revisions, each with what the change does; files outside
    /// `.oxplow/extensions/` change nothing here.
    #[tokio::test]
    async fn an_effort_review_names_the_extensions_that_changed() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        // Git sees an extension's source once it is shared.
        crate::extension_deploy::share(&f.svc, "acme").unwrap();
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
            (".oxplow/extensions/acme/extension.yaml", manifest.clone()),
            (".oxplow/extensions/acme/lenses/count.yaml", lens(1)),
            ("src/a.rs", "fn a() {}\n".into()),
        ])
        .await;
        let end = snapshot(vec![
            (".oxplow/extensions/acme/lenses/count.yaml", lens(2)),
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
        assert_eq!(changes[0].change, crate::extension_impact::Change::Changed);
        let report = changes[0].impact.as_ref().unwrap();
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
        // Git sees an extension's source once it is shared.
        crate::extension_deploy::share(&f.svc, "acme").unwrap();
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
                ".oxplow/extensions/acme/extension.yaml",
                format!("{manifest}{command}"),
            ),
            (
                ".oxplow/extensions/acme/handlers/spin.star",
                "def transform(x):\n    n = 0\n    for i in range(400000000):\n        n += i\n    return {\"commands\": []}\n".into(),
            ),
        ])
        .await;
        let end = snapshot(vec![(
            ".oxplow/extensions/acme/extension.yaml",
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
