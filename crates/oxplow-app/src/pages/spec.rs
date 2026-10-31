//! The page and viz spec: how a page's viz nodes are declared (`Viz` and its
//! role blocks, params, columns, actions), the config a run carries
//! (`VizConfig`, `VizRun`) and how params resolve against a viewer
//! (`ViewContext`). The extension loader builds these; pages read them. See
//! `.context/pages-and-tabs.md` "Page files".

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

/// How a viz renders its rows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Viz {
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
    /// Rows nested by `tree.parent` (a row's parent is the row whose
    /// `tree.id` it names; none, or one not in the result, is a root),
    /// each shown as `tree.label`.
    Tree,
    /// Rows in time order (`timeline.at`), each shown as
    /// `timeline.label`, linked through `timeline.ref` when set.
    Timeline,
    /// The first row as label/value pairs (the displayed columns).
    Detail,
    /// One subsection per row, in row order: a heading (`sections.title`,
    /// linked through `sections.link`), the row's `columns` as a short
    /// facts list and its `sections.body` as markdown. The subsections
    /// are real sections of the page: in Contents (up to a cap), each
    /// with the viz's row actions.
    Sections,
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
    /// The working tree's file `editor.path` (the first row's), edited at
    /// full size: its buffer shared with every tab showing it, saved
    /// through `oxplow.file.save`, `editor.lines` revealed.
    Editor,
    /// Per row, the diff of the file `hunks.path` between the revisions
    /// `hunks.from` and `hunks.to` (`working`, `snap:<id>`,
    /// `<vcs>:<rev>`), read-only.
    Hunks,
    /// A form for command `form.command`: its fields from the command's
    /// input schema, prefilled from `form.defaults` (placeholders bound
    /// like an action's) and the query's first row, if the viz has one.
    /// Submitting runs the command as the viz.
    Form,
    /// The pending proposals an agent raised, each with Approve and
    /// Decline: core's consent component. The viz reads
    /// `v_command_proposal` itself and decides through the person-only
    /// path, so the viz needs no query beyond `SELECT 1`; its params
    /// `thread` (a thread ref) and `setting` (a config key) narrow the
    /// list.
    Approvals,
    /// A terminal per row (`terminal.of`: an `oxplow/terminal` shell's URI
    /// or an `oxplow/agent_session`'s), kept running while hidden, at full
    /// size; `terminal.title` names it in the strip that picks among
    /// several; an agent row's `conversation` says what it is bound to.
    Terminal,
    /// A web page (`embed.url`, the first row's), opened in oxplow's
    /// sandboxed window: the page is a status panel with Open again, Open in
    /// Browser and Copy URL — core's component for what an `http(s)` link
    /// opens.
    Embed,
    /// The extension's own web component (`custom.component`), in
    /// a sandboxed frame; its rows are what it shows and what an agent
    /// reads (as a table).
    Custom,
}

impl Viz {
    /// It takes its page's whole height (a file, a diff): the page draws
    /// it filling the body, its title in the chrome.
    pub fn fills(self) -> bool {
        matches!(
            self,
            Viz::Code | Viz::Diff | Viz::Editor | Viz::Terminal | Viz::Embed
        )
    }
}

/// A `custom` viz's component and the props it starts with.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizCustom {
    /// One of the extension's `custom_components` ids.
    pub component: Option<String>,
    /// Handed to the component as is.
    #[serde(default, serialize_with = "plain_json")]
    #[specta(type = Option<oxplow_domain::Json>)]
    pub props: Option<serde_json::Value>,
}

/// When a viz needs attention: its row count reaches `min_rows`, or the
/// first row's `column` goes `above` / `below` a threshold. A left-nav
/// panel's `badge` viz shows its count, and the Alerts panel lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageAlert {
    #[specta(type = Option<oxplow_domain::JsNumber>)]
    pub min_rows: Option<i64>,
    pub column: Option<String>,
    pub above: Option<f64>,
    pub below: Option<f64>,
    /// Badge text; else the row count or the column's name.
    pub label: Option<String>,
}

/// `alert:` as written in a viz file (snake_case keys).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlertFile {
    min_rows: Option<i64>,
    column: Option<String>,
    above: Option<f64>,
    below: Option<f64>,
    label: Option<String>,
}

impl AlertFile {
    pub(crate) fn into_alert(self) -> Result<PageAlert, String> {
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
        Ok(PageAlert {
            min_rows: self.min_rows,
            column: self.column,
            above: self.above,
            below: self.below,
            label: self.label,
        })
    }
}

/// A viz's alert, evaluated on one run.
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

pub(crate) fn cell_number(c: &SqlCell) -> Option<f64> {
    match c {
        SqlCell::Int(i) => Some(*i as f64),
        SqlCell::Real(r) => Some(*r),
        SqlCell::Text(t) => t.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Evaluate `alert` against a run's result.
pub(crate) fn evaluate_alert(alert: &PageAlert, result: &SqlQueryResult) -> AlertState {
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
pub struct VizChart {
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
pub struct VizTree {
    pub id: Option<String>,
    pub parent: Option<String>,
    pub label: Option<String>,
    /// A lazy tree: the viz param a branch's children are read with. The
    /// run's rows are one level (the param's `default` is the top level);
    /// a branch opens by running the viz again with the param set to the
    /// row's `id`, its rows spliced under it. Needs `branch`.
    #[serde(default)]
    pub lazy: Option<String>,
    /// A lazy tree's rows that open: the column that is truthy on them. A
    /// row with children in the result nests them as an eager tree does.
    #[serde(default)]
    pub branch: Option<String>,
}

/// `timeline` viz: when each row happened and what it says.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizTimeline {
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
pub struct VizSteps {
    pub label: Option<String>,
    pub status: Option<String>,
}

/// `sections` viz: the columns of each row's subsection. The row's other
/// displayed `columns` are its facts.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizSections {
    /// The column holding the heading's text.
    pub title: Option<String>,
    /// A column holding a stable key for the row (names its subsection's
    /// `data-node`): `id` when the result has one, else the title.
    #[serde(default)]
    pub id: Option<String>,
    /// A column holding a URI the heading's arrow opens.
    #[serde(default)]
    pub link: Option<String>,
    /// A column of markdown drawn under the heading and the facts.
    #[serde(default)]
    pub body: Option<String>,
    /// A column whose value groups the rows: rows with the same value (in
    /// the order each first appears) go under one heading, a section one
    /// level above the rows' subsections. A row with no value is drawn
    /// outside any group, at the groups' level.
    #[serde(default)]
    pub group: Option<String>,
    /// A column holding a URI the group heading's arrow opens (read from
    /// the group's first row).
    #[serde(default)]
    pub group_link: Option<String>,
}

/// What becomes of the page a viz is on after one of its actions (or its
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
    /// The page opens in this tab; Back returns to the viz's page.
    #[default]
    Open,
    /// The page takes this tab and the viz's page leaves its history: Back
    /// never returns to a submitted form. A page that can't live in a tab
    /// (an agent session) closes this one and is focused.
    Replace,
    /// Nothing opens; the viz's page stays (a form for the next entry).
    Stay,
    /// The viz's page closes (it showed what the command removed).
    Close,
}

/// `form` viz: the command it submits and the values it starts from.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct FormSpec {
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
    pub fields: Option<Vec<FormField>>,
    /// Fields picked from a list: each names a viz (`<extension>/<slug>`)
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
pub enum FormField {
    Name(String),
    Spec(FormFieldSpec),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormFieldSpec {
    pub name: String,
    /// What the person reads over the field; absent, the schema's `title`,
    /// else the name humanized (`native_rename` → "Native rename").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A line under the field, written for the person. Absent, none: the
    /// command's schema `description` is for agents and is never shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Shown only while each named field's value is one of these; hidden,
    /// it is neither required nor submitted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visible_if: BTreeMap<String, FieldValues>,
    /// Masked, never kept in a draft, and blank leaves it out of the
    /// input; the command must keep it out of its record (`unrecorded`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    /// An object field whose children are the rows of this viz
    /// (`<extension>/<slug>`: `name`, `title`, `kind` of `enum`, `text` or
    /// `number`, `values`, `read_only`), run with the form's params. The
    /// field's value is the object of those children; it is read again
    /// when the viz's data changes.
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

pub(crate) static NO_CONDITIONS: BTreeMap<String, FieldValues> = BTreeMap::new();

impl FormField {
    pub fn name(&self) -> &str {
        match self {
            FormField::Name(n) => n,
            FormField::Spec(s) => &s.name,
        }
    }
    pub fn secret(&self) -> bool {
        matches!(self, FormField::Spec(s) if s.secret)
    }
    /// The viz its children are read from, for an object field.
    pub fn from(&self) -> Option<&str> {
        match self {
            FormField::Spec(s) => s.from.as_deref(),
            FormField::Name(_) => None,
        }
    }
    pub fn visible_if(&self) -> &BTreeMap<String, FieldValues> {
        match self {
            FormField::Name(_) => &NO_CONDITIONS,
            FormField::Spec(s) => &s.visible_if,
        }
    }
}

/// Serialize a JSON value with its numbers as plain numbers, so a YAML
/// viz file gets `1`, not serde_json's arbitrary-precision number map.
pub(crate) fn plain_json<S: serde::Serializer>(
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
pub struct VizCode {
    pub path: Option<String>,
    /// A column holding the filesystem's id; none or empty, the local one.
    #[serde(default)]
    pub filesystem: Option<String>,
    /// A column holding the revision, the filesystem's own word; none or
    /// empty, its current content (the working tree locally).
    pub revision: Option<String>,
    /// A column holding `12-40` (or `12`): the lines marked and revealed.
    #[serde(default)]
    pub lines: Option<String>,
    /// Several rows' editors scroll together, each from its marked lines.
    #[serde(default)]
    pub sync: bool,
}

/// `terminal` viz: the columns of each row's terminal.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizTerminal {
    /// Its URI: an `oxplow/terminal` or `oxplow/agent_session`.
    pub of: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// What the agent is bound to; a change opens it again.
    #[serde(default)]
    pub conversation: Option<String>,
    /// The agent's thread (its URI): where its Escape interrupt is logged.
    #[serde(default)]
    pub thread: Option<String>,
}

/// `embed` viz: the web page the first row names.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizEmbed {
    /// The column holding the page's URL (`http` or `https`).
    pub url: Option<String>,
}

/// `editor` viz: the working tree's file the first row names, and the
/// lines it reveals.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizEditor {
    pub path: Option<String>,
    /// A column holding the filesystem's id; the editor edits only the
    /// local working tree, so any other is shown as a refusal.
    #[serde(default)]
    pub filesystem: Option<String>,
    /// A column holding `12` (or `12-40`: its start): the line revealed.
    #[serde(default)]
    pub lines: Option<String>,
}

/// `diff` viz: the first row's file, between two revisions, at full size.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizDiff {
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
/// it, the first column. A viz with an `edit` action names it: the text
/// is what the editor saves.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizMarkdown {
    #[serde(default)]
    pub text: Option<String>,
}

/// `hunks` viz: the file and the two revisions each row diffs.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct VizHunks {
    pub path: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl VizChart {
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

/// Launcher (Cmd+K) sections a viz can be listed under. Mirrors the
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
    Dashboards,
    System,
}

/// A page a column value can link to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
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
pub struct ColumnLink {
    pub kind: LinkKind,
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
pub struct PageColumn {
    /// Result column name.
    pub key: String,
    /// Header text; defaults to `key`.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub link: Option<ColumnLink>,
    /// A column of the same row holding this one's unit (`ms`, `%`,
    /// `lines`): a number shows as a metric value in it.
    #[serde(default)]
    pub unit: Option<String>,
    /// A column of the same row naming an icon drawn before the cell
    /// (`done`, `in_progress`, `epic`, `wiki`, …; the UI's fixed
    /// vocabulary, `viz/vizIcons.tsx` — an unknown name draws none).
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
    /// A param of the viz: the cell's text is drawn with every
    /// case-insensitive occurrence of its current value marked.
    #[serde(default)]
    pub highlight: Option<String>,
    /// A param of the viz that clicking a row sets to this column's
    /// value, re-running what reads it (a document's other sections):
    /// master and detail. The row whose value it holds is drawn selected.
    #[serde(default)]
    pub select: Option<String>,
    /// A viz (`<slug>` of this extension or `<extension>/<slug>`) the row
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
    /// A list of items (a JSON array): each element is a string or
    /// `{ "text": …, "hint"?: …, "uri"?: … }`, drawn inline separated by
    /// ` · `; `hint` is the item's hover text, `uri` makes it a link. An
    /// agent reads them joined by ` · `, a hint in parentheses.
    Items,
}

/// `group` on a `list` or `table`: the rows under a heading per distinct
/// value of `by`, in the order each first appears; `link` makes the
/// heading a link, read from the group's first row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VizGroup {
    /// The column whose value groups the rows (not shown as a cell).
    pub by: String,
    /// The heading's arrow (its page); `from` defaults to `by`.
    #[serde(default)]
    pub link: Option<ColumnLink>,
    /// The groups in this order, shown even when no row has the value
    /// (an empty board column is still somewhere to drop); values the
    /// rows have beyond these follow, in the order each first appears.
    #[serde(default)]
    pub values: Vec<String>,
    /// The heading to show for a value, where the value itself (what a
    /// drop binds as `{{group.value}}`) reads badly: `in_progress: In
    /// progress`. A value with none shows as it is.
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// A value the viewer (or an agent) can set when running the viz,
/// bound into the query as `:name`.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct PageParam {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Used when the caller doesn't supply the param.
    #[serde(default)]
    pub default: Option<SqlCell>,
    /// The values it takes, when it's a choice: a panel shows
    /// them as a toggle in its header, a viz page as a select. Its
    /// `default` must be one; a run supplying another is refused.
    #[serde(default)]
    pub options: Vec<ChoiceOption>,
    /// A viz whose rows are the values it takes, when it's a choice
    /// whose options aren't fixed (`value`, `label`, optional `group` —
    /// the form `choices` contract): a slug of this extension, or
    /// `<extension>/<slug>`; a loaded viz has the whole id. The page runs
    /// it with the params it declares of the page's, so an `end` picker's
    /// viz can leave out what `start` is. Its options aren't known at
    /// load, so no `default` is needed and any text is taken. Exclusive
    /// with `options`.
    #[serde(default)]
    pub options_from: Option<String>,
    /// What it holds, and so how a page draws it. Left out, it is a
    /// `choice` when it has `options` or `options_from`, else `text`; a
    /// loaded viz always says.
    #[serde(default, rename = "type")]
    pub kind: Option<ParamType>,
    /// What it is to the page, and so where it rides in the page's URI:
    /// a `resource` (the default) names what the page shows, in the URI's
    /// `?`; `filter` narrows how it shows it, in the URI's `#` view. A
    /// `selection` follows from the columns and is never written: a param
    /// a column of the viz (or of a part of the page) selects, in the
    /// view too. A loaded viz always says.
    #[serde(default)]
    pub role: Option<ParamRole>,
    /// A `date_range`'s quick picks, of `1d 2d 3d 7d 30d 90d all`; the
    /// default is `1d 7d 30d all`.
    #[serde(default)]
    pub presets: Vec<String>,
}

/// What a viz param holds.
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

/// What a viz param is to its page.
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
    /// State the page or the system sets (its URI's `#` view, like a
    /// filter), never a person: the rail does not draw it.
    View,
}

impl ParamRole {
    /// Whether a param of this role rides in the URI's `#` view (every
    /// role but `resource`).
    pub fn rides_view(self) -> bool {
        !matches!(self, ParamRole::Resource)
    }
}

/// A `date_range`'s presets and how far back each reaches (`all`: no
/// start).
pub(crate) const RANGE_PRESETS: &[(&str, Option<i64>)] = &[
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
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

impl PageParam {
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
    pub(crate) fn take(&self, value: &SqlCell) -> Result<SqlCell, String> {
        let shown = || match value {
            SqlCell::Text(v) => format!("`{v}`"),
            other => format!("`{other:?}`"),
        };
        if matches!(value, SqlCell::Null(())) {
            return Ok(value.clone());
        }
        match self.kind() {
            ParamType::Text => Ok(value.clone()),
            // Options a viz lists aren't known here: the query says what
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
pub(crate) fn range_ends(
    value: &str,
    now: time::OffsetDateTime,
) -> Option<(Option<String>, Option<String>)> {
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
pub(crate) fn param_problem(params: &[PageParam]) -> Option<String> {
    params.iter().find_map(|p| {
        let kind = p.kind();
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
            // options a viz lists may start open.
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

/// The keys of a page's `#` view that are the page's, never a viz
/// param's: `section`, the section a link lands on. (`comment` is the
/// page's only on a page that takes comments; the Comments viz nodes pick a
/// comment by that name.)
pub const RESERVED_VIEW_KEYS: &[&str] = &["section"];

/// `params` with each one's type written out, so a page never infers it.
pub(crate) fn typed(params: Vec<PageParam>) -> Vec<PageParam> {
    params
        .into_iter()
        .map(|p| PageParam {
            kind: Some(p.kind()),
            ..p
        })
        .collect()
}

/// What a query binds: the values of `declared` params, and each
/// date_range's `<name>_from` / `<name>_to`.
pub(crate) fn bind_params(
    declared: &[PageParam],
    params: &BTreeMap<String, SqlCell>,
    now: time::OffsetDateTime,
) -> Vec<(String, SqlCell)> {
    let mut out: Vec<(String, SqlCell)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let text = |t: Option<String>| t.map_or(SqlCell::Null(()), SqlCell::Text);
    for p in declared.iter().filter(|p| p.kind() == ParamType::DateRange) {
        let (from, to) = match params.get(&p.name) {
            Some(SqlCell::Text(v)) => range_ends(v, now).unwrap_or((None, None)),
            _ => (None, None),
        };
        out.push((format!("{}_from", p.name), text(from)));
        out.push((format!("{}_to", p.name), text(to)));
    }
    out
}

/// A button on a viz: a command it runs (P6.B1, target §11.4). The
/// command goes through the bus as `Actor::Page` acting for whoever
/// pressed it, so every policy — the agent policy, confirmation
/// — applies as if they had run it themselves; a viz can offer a button
/// but never grant a power. Copying the result and handing it to the
/// agent are on every viz, not declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageAction {
    /// Unique within the viz; `oxplow_run_page_action` names it.
    pub id: String,
    pub label: String,
    /// The command it runs (`oxplow.work_item.transition`). None for an
    /// `open` action, which runs none.
    #[serde(default)]
    pub command: Option<String>,
    /// An `open` action: the column of the row it is offered on that holds
    /// the URI it opens. It runs no command (no bus run, no audit): the
    /// app navigates, and an agent pressing it is told the URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
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
    /// person to fill in before it runs (`the action form`), instead of
    /// running at once.
    pub form: bool,
    /// Fields of a form action the person sees but can't change: their
    /// values are `input`'s, imposed again when it runs (an item's `ref`).
    pub fixed: Vec<String>,
    /// A form action's fields as the person sees them, in this order: the
    /// only ones shown (the others keep `input`'s value), each with its
    /// own `label` and `help`. Absent, every field of the command but the
    /// `fixed` ones, labelled from the schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FormField>,
    /// A row action's form: the column of the row that says, in words, what
    /// the form is about (a setting's name): the form's subtitle, since its
    /// `fixed` fields (ids) are never shown. A page action's form is
    /// about the page, named by its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// A form action's fields picked from a list: each names a viz
    /// (`<extension>/<slug>`) run with the viz's params, whose rows are
    /// the choices (`value`, `label`, optional `group`).
    pub choices: BTreeMap<String, String>,
    /// What becomes of the viz's page when the command's `ui.open_after`
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
    /// A page action (not a row, group or edit action) that stays a button
    /// in the page's nav bar instead of going in its ☰ menu: what a person
    /// reaches for on this page (a list's New). Several may be.
    #[serde(default)]
    pub prominent: bool,
    /// A page action's submenu in the page's ☰ menu: the actions (and the
    /// commands whose `ui.group` is the same name) under one item there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submenu: Option<String>,
    /// An icon oxplow draws (`extension_resources::ICONS`): alone on the
    /// action's button in the bar (its label the hover), beside its label
    /// in a menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
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
    /// A whole-viz action over a viz that holds one row, that names
    /// `when` or `{{row.*}}`: the viz's one row. Derived at load, not
    /// declared: an action without `row: true` that names `when:` or
    /// `{{row.*}}`. It runs on the row the server reads from the viz
    /// itself, and only while the viz has exactly one.
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

/// A viz action as written in a viz file.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionFile {
    /// Unique within the viz.
    id: String,
    label: String,
    /// The command it runs (`oxplow.work_item.transition`). One of
    /// `command` or `open`.
    #[serde(default)]
    command: Option<String>,
    /// Instead of a command: the column of the row (the page's, the viz's
    /// or the row's own) that holds the URI to open. No command runs.
    #[serde(default)]
    open: Option<String>,
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
    /// A form action's fields as the person sees them: the only ones
    /// shown, in this order, each a name or `{ name, label, help }`.
    #[serde(default)]
    fields: Vec<FormField>,
    /// A row action's form: the row's column that says what it is about.
    #[serde(default)]
    about: Option<String>,
    /// A form action's fields picked from a list: field → the viz whose
    /// rows are its choices.
    #[serde(default)]
    choices: BTreeMap<String, String>,
    /// What becomes of the viz's page when the command opens one:
    /// `open` (default), `replace`, `stay` or `close`.
    #[serde(default)]
    then: ActionThen,
    /// A row action's column: offered only on rows where it is truthy.
    #[serde(default)]
    when: Option<String>,
    /// The row action a click on the row runs.
    #[serde(default)]
    primary: bool,
    /// A page action shown as a button in the nav bar, not in its ☰.
    #[serde(default)]
    prominent: bool,
    /// The ☰ submenu a page action is in.
    #[serde(default)]
    submenu: Option<String>,
    /// The icon drawn for it.
    #[serde(default)]
    icon: Option<String>,
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
pub(crate) const KIT_KEYS: &[&str] = &[
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
pub(crate) const NAMED_KEYS: &[&str] = &["Delete", "Backspace"];

/// What's wrong with `key` as a row action's key, if anything.
pub(crate) fn key_problem(key: &str) -> Option<String> {
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

/// Parse a viz's `actions:`: each `{ id, label, command, input?, row? }`.
/// A placeholder must name a declared param (`{{param.x}}`) or, in a row
/// action, a column (`{{row.x}}`, checked against the result when the
/// extension is validated). A viz that `holds_one_row` (detail, form,
/// markdown, a document's row) lets a whole-viz action name `when` and
/// `{{row.*}}` too: that row.
pub(crate) fn parse_actions(
    raw: Vec<serde_yaml::Value>,
    params: &[PageParam],
    grouped: bool,
    holds_one_row: bool,
) -> Result<Vec<PageAction>, String> {
    let mut out: Vec<PageAction> = Vec::new();
    for v in raw {
        let f = serde_yaml::from_value::<ActionFile>(v).map_err(|e| format!("actions: {e}"))?;
        if f.id.trim().is_empty() {
            return Err("actions: an action needs an `id`".into());
        }
        if out.iter().any(|a| a.id == f.id) {
            return Err(format!("actions: `{}` is declared twice", f.id));
        }
        match (&f.command, &f.open) {
            (None, None) => {
                return Err(format!(
                    "actions: `{}` needs a `command` or an `open` (the column that holds the URI to open)",
                    f.id
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!(
                    "actions: `{}`: give a `command` or `open`, not both",
                    f.id
                ))
            }
            (Some(command), None) => oxplow_domain::CommandSpec::validate_id(command)
                .map_err(|e| format!("actions: `{}`: {e}", f.id))?,
            (None, Some(column)) => {
                if column.trim().is_empty() {
                    return Err(format!("actions: `{}`: `open` needs a column", f.id));
                }
                if !(f.row || holds_one_row) {
                    return Err(format!(
                        "actions: `{}`: `open` names a column of a row: use it on a row action (`row: true`) or on a viz that holds one row (detail, form, markdown, a page's row)",
                        f.id
                    ));
                }
                if f.input.is_some()
                    || f.form
                    || f.edit
                    || f.drag
                    || f.bulk
                    || !f.fixed.is_empty()
                    || !f.choices.is_empty()
                    || f.group.is_some()
                {
                    return Err(format!(
                        "actions: `{}`: an `open` action runs no command, so it takes no `input`, `form`, `edit`, `drag`, `bulk`, `fixed`, `choices` or `group`",
                        f.id
                    ));
                }
            }
        }
        if let Some(g) = &f.group {
            if !grouped {
                return Err(format!(
                    "actions: `{}`: `group: {g}` needs the viz to have a `group`",
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
        if f.about.is_some() && !(f.form && f.row) {
            return Err(format!(
                "actions: `{}`: `about` is for a row action's form (`form: true`, `row: true`)",
                f.id
            ));
        }
        if !f.fields.is_empty() {
            if !f.form {
                return Err(format!(
                    "actions: `{}`: `fields` is for a form action (`form: true`)",
                    f.id
                ));
            }
            if let Some(why) = action_fields_problem(&f.fields, &f.fixed) {
                return Err(format!("actions: `{}`: {why}", f.id));
            }
        }
        if f.when.is_some() && !(f.row || f.drag || holds_one_row) {
            return Err(format!(
                "actions: `{}`: `when` is for a row action (`row: true`), or a viz that holds one row (detail, form, markdown, a document's row)",
                f.id
            ));
        }
        // A drag action is a row action: its row is the dragged one. An
        // edit action is a form over the viz's row.
        let row = f.row || f.drag || f.edit;
        let form = f.form || f.edit;
        // Derived: a whole-viz action that names `when` or the row.
        let one_row = !row
            && holds_one_row
            && (f.when.is_some()
                || f.open.is_some()
                || action_templates(&input)
                    .iter()
                    .any(|(scope, _)| scope == "row"));
        if let Some(icon) = &f.icon {
            if !crate::extension::extension_resources::ICONS.contains(&icon.as_str()) {
                return Err(format!(
                    "actions: `{}`: `icon: {icon}` isn't one oxplow draws; use one of {}",
                    f.id,
                    crate::extension::extension_resources::ICONS.join(", ")
                ));
            }
        }
        if let Some(name) = &f.submenu {
            if name.trim().is_empty() {
                return Err(format!("actions: `{}`: `submenu` needs a name", f.id));
            }
            if row || f.group.is_some() || f.edit {
                return Err(format!(
                    "actions: `{}`: `submenu` is for a page action (not `row`, `group`, `drag` or `edit`): it groups it in the page's ☰ menu",
                    f.id
                ));
            }
            if f.prominent {
                return Err(format!(
                    "actions: `{}`: a prominent action is a button in the bar, never in a ☰ submenu: drop `prominent` or `submenu`",
                    f.id
                ));
            }
        }
        if f.prominent && (row || f.group.is_some() || f.edit) {
            return Err(format!(
                "actions: `{}`: `prominent` is for a page action (not `row`, `group`, `drag` or `edit`): it keeps that action a button in the page's nav bar",
                f.id
            ));
        }
        if f.edit && (f.drag || f.bulk || f.key.is_some() || f.group.is_some() || f.primary) {
            return Err(format!(
                "actions: `{}`: an edit action is the editor's Save: it can't be `drag`, `bulk`, `key`, `group` or `primary`",
                f.id
            ));
        }
        if f.edit {
            if let Some(other) = out.iter().find(|a| a.edit) {
                return Err(format!(
                    "actions: `{}`: the viz already saves with `{}`",
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
                        "actions: `{}`: `{{{{param.{name}}}}}` names no param of this viz",
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
                        "actions: `{}`: `{{{{group.value}}}}` needs the viz to have a `group`",
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
                        "actions: `{}`: `{{{{row.{name}}}}}` needs `row: true`, or a viz that holds one row (detail, form, markdown, a document's row)",
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
        out.push(PageAction {
            id: f.id,
            label: f.label,
            command: f.command,
            open: f.open,
            input,
            row,
            group: f.group,
            form,
            fixed: f.fixed,
            fields: f.fields,
            about: f.about,
            choices: f.choices,
            then: f.then,
            when: f.when,
            primary: f.primary,
            prominent: f.prominent,
            submenu: f.submenu.map(|n| n.trim().to_string()),
            icon: f.icon,
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

/// What's wrong with a form action's `fields`: a field twice, a fixed one
/// (never drawn), a secret or object one (the action's form has neither),
/// or a condition on a field it doesn't show.
fn action_fields_problem(fields: &[FormField], fixed: &[String]) -> Option<String> {
    let names: Vec<&str> = fields.iter().map(|f| f.name()).collect();
    for (i, field) in fields.iter().enumerate() {
        let name = field.name();
        if names[..i].contains(&name) {
            return Some(format!("`fields` names `{name}` twice"));
        }
        if fixed.iter().any(|k| k == name) {
            return Some(format!(
                "`fields` names `{name}`, which is `fixed` (a fixed field is never shown)"
            ));
        }
        if field.secret() || field.from().is_some() {
            return Some(format!(
                "`fields`: `{name}` can't be `secret` or have a `from` in an action's form"
            ));
        }
        if let Some(on) = field
            .visible_if()
            .keys()
            .find(|on| !names.contains(&on.as_str()) || on.as_str() == name)
        {
            return Some(format!(
                "`fields`: `{name}`'s `visible_if` names `{on}`, which isn't another of its `fields`"
            ));
        }
    }
    None
}

/// Whether a viz of this kind holds one row, so a whole-node action can
/// name it: a detail, a form, a markdown.
pub(crate) fn holds_one_row(viz: Viz) -> bool {
    matches!(viz, Viz::Detail | Viz::Form | Viz::Markdown)
}

/// An edit action's edited field: the one input field that is exactly a
/// `{{row.<column>}}` and isn't `fixed`.
pub(crate) fn edit_field(
    id: &str,
    input: &serde_json::Value,
    fixed: &[String],
) -> Result<String, String> {
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
pub fn edit_column(action: &PageAction) -> Option<String> {
    let field = action.edit_field.as_deref()?;
    let value = action.input.get(field)?.as_str()?;
    oxplow_domain::template::whole_placeholder(value).map(|p| p.name)
}

/// Where a viz comes from. An agent edits `source`, never `running`: a
/// deployed extension runs a copy in the store that the next deploy
/// replaces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageProvenance {
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
    /// The file URI of the copy that runs, pinned to its deployment (on
    /// `oxplow:extensions`); none when it runs no deployment.
    pub running_uri: Option<String>,
}

/// How a page's viz node draws its loader's rows: the viz, its role
/// block, columns, row styling and actions. It runs no query of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VizConfig {
    /// `<extension>/<slug>`: the stable id of the page this viz is on.
    pub id: String,
    pub extension: String,
    pub slug: String,
    pub title: String,
    pub viz: Viz,
    pub params: Vec<PageParam>,
    pub columns: Vec<PageColumn>,
    pub empty: Option<String>,
    /// Left out of the page when its loader has no rows: a section about
    /// something the subject may not have (a kind of contribution).
    #[serde(default)]
    pub hide_empty: bool,
    /// A column of the loader whose first row's text is a paragraph under
    /// the node's heading, above its viz (what a thing is, above its
    /// facts). Null or empty: nothing is drawn.
    pub lead: Option<String>,
    /// Columns a chart viz draws from.
    pub chart: Option<VizChart>,
    pub tree: Option<VizTree>,
    pub timeline: Option<VizTimeline>,
    pub steps: Option<VizSteps>,
    pub sections: Option<VizSections>,
    pub hunks: Option<VizHunks>,
    pub code: Option<VizCode>,
    pub diff: Option<VizDiff>,
    pub editor: Option<VizEditor>,
    pub embed: Option<VizEmbed>,
    pub terminal: Option<VizTerminal>,
    pub markdown: Option<VizMarkdown>,
    pub form: Option<FormSpec>,
    /// For `custom`: the component and its props.
    pub custom: Option<VizCustom>,
    /// For `list` / `table`: rows under a heading per value.
    pub group: Option<VizGroup>,
    /// For `list` / `table` / `tree`: a column whose truthy value
    /// highlights the row.
    pub emphasis: Option<String>,
    /// For `list` / `table`: a column whose integer value indents the row.
    pub depth: Option<String>,
    /// It takes its page's whole height: its viz does
    /// ([`Viz::fills`]), or it is a document whose last section's viz
    /// fills, so that section takes the height left (set once the catalog
    /// is placed, `fill_document_fills`).
    pub fills: bool,
    /// Commands the viz offers, as buttons or row actions (P6.B1).
    pub actions: Vec<PageAction>,
    /// Where it comes from: the file that runs and the source to edit.
    pub origin: PageProvenance,
}

/// One viz node of a page run: its viz, the page's params and the rows
/// of the loader it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VizRun {
    pub config: VizConfig,
    /// The parameter values actually used (supplied or default).
    pub params: BTreeMap<String, SqlCell>,
    pub result: SqlQueryResult,
}

impl PageProvenance {
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
        crate::extension::extension_store::source_hash(dir).is_ok_and(|hash| &hash != deployment)
    }

    /// As an agent reads it: these fields and `outOfDate`.
    pub fn to_json(&self, project: &Path) -> serde_json::Value {
        let mut out = serde_json::to_value(self).unwrap_or_default();
        out["outOfDate"] = serde_json::Value::Bool(self.out_of_date(project));
        out
    }
}

impl VizConfig {
    /// Every result column the viz's component blocks name (`chart`,
    /// `tree`, `tile`, `timeline`, `steps`, `hunks`) and its row styling
    /// (`group`, `emphasis`, `depth`) — what `validate` checks the query
    /// returns.
    pub fn role_columns(&self) -> Vec<(&'static str, &String)> {
        let mut out: Vec<(&'static str, &String)> = Vec::new();
        if let Some(l) = &self.lead {
            out.push(("lead", l));
        }
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
        if let Some(s) = &self.sections {
            out.extend(
                [&s.title, &s.id, &s.link, &s.body, &s.group, &s.group_link]
                    .into_iter()
                    .flatten()
                    .map(|k| ("sections", k)),
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
                [&c.path, &c.filesystem, &c.revision, &c.lines]
                    .into_iter()
                    .flatten()
                    .map(|k| ("code", k)),
            );
        }
        if let Some(t) = &self.terminal {
            out.extend(
                [&t.of, &t.title, &t.conversation, &t.thread]
                    .into_iter()
                    .flatten()
                    .map(|k| ("terminal", k)),
            );
        }
        if let Some(e) = &self.embed {
            out.extend(e.url.iter().map(|k| ("embed", k)));
        }
        if let Some(e) = &self.editor {
            out.extend(
                [&e.path, &e.filesystem, &e.lines]
                    .into_iter()
                    .flatten()
                    .map(|k| ("editor", k)),
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

/// Where a viz is being looked at from: the viewer's current stream and
/// thread (numeric ids, as the `v_*` views use). A viz that declares a
/// `stream_id` or `thread_id` param gets these unless it's given another
/// value (tsk375).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewContext {
    pub stream_id: Option<i64>,
    pub thread_id: Option<i64>,
    /// What's active, to check a viz's needs against; `None` checks
    /// nothing.
    pub active: Option<crate::capabilities::Active>,
}

impl ViewContext {
    /// The params the context supplies when the caller doesn't.
    const SUPPLIED: &[&str] = &["stream_id", "thread_id"];

    /// Whether a param named `name` is one the context supplies.
    pub fn supplies(name: &str) -> bool {
        Self::SUPPLIED.contains(&name)
    }

    /// The implicit value for a param named `name`, if it is one.
    pub(crate) fn value(&self, name: &str) -> Option<SqlCell> {
        match name {
            "stream_id" => self.stream_id.map(SqlCell::Int),
            "thread_id" => self.thread_id.map(SqlCell::Int),
            _ => None,
        }
    }
}

/// The context a viz is viewed from: `stream` (else `thread`'s stream,
/// else the primary) and `thread` (else that stream's selected or active
/// thread). Lookups that fail leave the value unset.
pub async fn view_context(
    svc: &crate::Services,
    stream: Option<oxplow_domain::StreamId>,
    thread: Option<oxplow_domain::ThreadId>,
) -> ViewContext {
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
    ViewContext {
        stream_id: stream.map(|s| s.value()),
        thread_id: thread.map(|t| t.value()),
        active: Some(
            svc.capabilities
                .snapshot(&crate::config_service::read_config(&svc.config)),
        ),
    }
}

/// What a page's `label` fills from: its URI's params, or a column of the
/// one row its viz returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabelFrom {
    Param,
    Row,
}

/// The label filters: `short` is the first 7 characters (a sha),
/// `basename` the last path segment.
pub(crate) const LABEL_FILTERS: &[&str] = &["short", "basename", "url"];

/// What a page's `label` names: each `{{param.<name>}}` or
/// `{{row.<column>}}`, optionally filtered (`{{param.sha|short}}`), spaces
/// allowed inside the braces; any other `{{…}}` is refused.
pub(crate) fn label_refs(label: Option<&str>) -> Result<Vec<(LabelFrom, String)>, String> {
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

/// The value of each of `viz`'s params: as supplied, else from the
/// context (`stream_id` / `thread_id`), else its default, else NULL, each
/// as its type binds it (`PageParam::take`). An unknown supplied name, or
/// a value its param doesn't take, is refused.
pub fn resolve_params(
    cfg: &VizConfig,
    supplied: &BTreeMap<String, SqlCell>,
    ctx: &ViewContext,
) -> Result<BTreeMap<String, SqlCell>, DomainError> {
    resolve_param_values(&format!("viz {}", cfg.id), &cfg.params, supplied, ctx)
}

/// `declared`'s values for a run: supplied, else the viewer's context,
/// else the default, else NULL. An unknown or malformed supplied value is
/// refused, named by `whose` (`viz acme/x`, `page acme/y`).
pub(crate) fn resolve_param_values(
    whose: &str,
    declared: &[PageParam],
    supplied: &BTreeMap<String, SqlCell>,
    ctx: &ViewContext,
) -> Result<BTreeMap<String, SqlCell>, DomainError> {
    if let Some(unknown) = supplied
        .keys()
        .find(|k| !declared.iter().any(|p| &p.name == *k))
    {
        let known: Vec<&str> = declared.iter().map(|p| p.name.as_str()).collect();
        return Err(DomainError::Invalid(format!(
            "{whose}: unknown param `{unknown}` (params: {})",
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        )));
    }
    let mut params = BTreeMap::new();
    for p in declared {
        let v = match supplied.get(&p.name) {
            Some(v) => p.take(v).map_err(|why| {
                DomainError::Invalid(format!("{whose}: param `{}` {why}", p.name))
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
