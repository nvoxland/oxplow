//! `collectors:` (P7.B3, `.context/semantic-layer.md` "Collectors"): the
//! one declaration for bringing data in — an extension's in
//! `extension.yaml`, the project's in `.oxplow/project.yaml`.
//!
//! A collector is a program (`exec`, approved by a person), a sandboxed
//! script (`starlark` / `jaq`, no I/O, no approval) or a provider's read
//! (`read`). It writes **entities** (rows a model can `ref()`),
//! **facts** (measurements on declared measures) or **records** (a test,
//! coverage or analysis report it parses — the project's, tsk863), and it
//! runs when its **trigger** says: `manual`, `{ every: 15m }`,
//! `{ on: [<event types>], where: { <payload field>: <value> } }` — when
//! such an event is logged, after the consumers named in `after:` have
//! handled it — or, for a report collector, `{ on_run: test | analysis }`:
//! when the agent runs the project's tests or an analyzer.
//!
//! ```yaml
//! collectors:
//!   - id: repo.scan_clone            # a dotted id keeps a producer's name
//!     runtime: starlark
//!     entry: oxplow/collectors/repo_clone.star
//!     trigger: { on: [snapshot.taken] }
//!     facts: [repo.rust_clone]
//!   - id: prs
//!     runtime: exec
//!     entry: sync.sh
//!     trigger: { every: 15m }
//!     credentials: [GITHUB_TOKEN]
//!     network: [api.github.com]
//!     entities: [...]
//!   - id: tests.rust_coverage         # a report collector
//!     records: coverage
//!     entry: oxplow:lcov              # a bundled parser
//!     report: { path: target/coverage/lcov.info }
//!     trigger: { on_run: test }
//! ```
//!
//! This module parses and validates; running one is the app's
//! (`collector_runner`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Declared type of an entity column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    Text,
    Int,
    Real,
    Bool,
    /// An RFC 3339 timestamp, stored as text.
    Time,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EntityColumn {
    pub name: String,
    pub col_type: ColumnType,
    pub doc: String,
}

/// A documented join from an entity to another view. Not executed; it
/// tells agents and lens authors how the data connects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityRelation {
    /// The view it joins to, e.g. `v_task` or `v_github_review`.
    pub to: String,
    /// The SQL join condition, e.g. `v_github_pr.head_branch = v_stream.branch`.
    pub on: String,
}

/// An entity a collector writes: its rows, published as a view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EntityDecl {
    pub name: String,
    pub doc: String,
    /// Column that uniquely identifies a row.
    pub key: String,
    pub columns: Vec<EntityColumn>,
    pub relations: Vec<EntityRelation>,
    /// SQL name lenses and agents query: `v_<owner>_<entity>`.
    pub view: String,
}

/// When a collector runs by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Trigger {
    /// Only when someone runs it (`collector.sync`).
    Manual,
    /// Every `minutes` minutes (an exec collector once approved).
    Every { minutes: u32 },
    /// When an event of one of these types is logged and its payload has
    /// each `filter` field equal to its value — after the consumers in
    /// the collector's `after` have handled the event.
    On {
        events: Vec<String>,
        filter: BTreeMap<String, String>,
    },
    /// A report collector's: when the agent runs the project's tests or an
    /// analyzer (the `collection` reactor detects the run), if its report
    /// was written by that run.
    OnRun { run: RunKind },
}

/// The kind of run a report collector reads after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    Test,
    Analysis,
}

/// What a report collector records: its parser's typed output, merged
/// into the run (`.context/collection.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Records {
    /// A suite/case tree of test outcomes (a `test-run`).
    Tests,
    /// Per-file instrumented/covered lines (a coverage capture).
    Coverage,
    /// Linter/analyzer findings (a `static-analysis` capture).
    Analysis,
}

/// A parser oxplow ships (tsk935: the one table — the config check and
/// the parser runtime both read it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledParser {
    /// What `entry: oxplow:<name>` names.
    pub name: &'static str,
    /// What it records.
    pub records: Records,
    /// How its report is read (`report.format`) before it sees it.
    pub format: &'static str,
    /// Its jq program.
    pub program: &'static str,
}

/// The parsers oxplow ships, named by a report collector's
/// `entry: oxplow:<name>`.
pub const BUNDLED_PARSERS: &[BundledParser] = &[
    BundledParser {
        name: "junit",
        records: Records::Tests,
        format: "xml",
        program: include_str!("parsers/junit.jq"),
    },
    BundledParser {
        name: "lcov",
        records: Records::Coverage,
        format: "lcov",
        program: include_str!("parsers/lcov.jq"),
    },
    BundledParser {
        name: "cobertura",
        records: Records::Coverage,
        format: "xml",
        program: include_str!("parsers/cobertura.jq"),
    },
    BundledParser {
        name: "jacoco",
        records: Records::Coverage,
        format: "xml",
        program: include_str!("parsers/jacoco.jq"),
    },
    BundledParser {
        name: "clippy",
        records: Records::Analysis,
        format: "lines",
        program: include_str!("parsers/clippy.jq"),
    },
    BundledParser {
        name: "eslint",
        records: Records::Analysis,
        format: "json",
        program: include_str!("parsers/eslint.jq"),
    },
];

/// The bundled parser `name`, if oxplow ships one.
pub fn bundled_parser(name: &str) -> Option<&'static BundledParser> {
    BUNDLED_PARSERS.iter().find(|p| p.name == name)
}

/// The prefix of a bundled parser's `entry`.
pub const BUNDLED_ENTRY: &str = "oxplow:";

/// What runs a collector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CollectorRuntime {
    /// A program: can reach the network and credentials, so it needs a
    /// person's approval to run.
    Exec,
    /// A sandboxed Starlark script: no I/O, so no approval.
    Starlark,
    /// A sandboxed jq program: no I/O, so no approval.
    Jaq,
    /// A provider instance's collector (`provider.sync`): its records
    /// land in the capability's model (`v_work_item`).
    Read,
}

impl CollectorRuntime {
    /// Sandboxed in-process: no I/O, no approval.
    pub fn is_derived(self) -> bool {
        matches!(self, CollectorRuntime::Starlark | CollectorRuntime::Jaq)
    }
}

/// How a run's entities land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CollectorSync {
    /// Each run restates every entity (one it doesn't mention is emptied).
    Replace,
    /// Each run adds or updates rows by key, and removes the keys it lists
    /// under `deleted`; an entity it doesn't mention is left alone.
    Upsert,
}

/// A provider's collector a `read` collector runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRead {
    /// The instance, `<extension>/<instance id>` (a provider's default
    /// instance has the provider's id).
    pub instance: String,
    pub collector: String,
}

/// A report file a collector reads as part of its input
/// (`input.report`): a tool's output in the worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportInput {
    /// Relative to the project (`target/type-coverage.json`).
    pub path: String,
    /// How it's parsed before the script sees it: `text` (the default),
    /// `json`, `xml`, `lcov` or `lines`.
    #[serde(default = "text_format")]
    pub format: String,
}

fn text_format() -> String {
    "text".into()
}

/// The report formats a collector can read.
pub const REPORT_FORMATS: &[&str] = &["text", "json", "xml", "lcov", "lines"];

/// One declared collector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorSpec {
    /// Unique in its owner; a dotted id (`repo.scan_clone`) is fine.
    pub id: String,
    pub doc: String,
    pub runtime: CollectorRuntime,
    /// The program or script, relative to its owner's folder (the
    /// extension's, or the project's); absent for `read`.
    pub entry: Option<String>,
    /// For `read`: the provider collector it runs.
    pub provider: Option<ProviderRead>,
    pub trigger: Trigger,
    /// With an `on:` trigger: the pump consumers that must have handled
    /// the event first (`change.analyze`).
    pub after: Vec<String>,
    /// A script's input query: read-only SQL over the semantic layer,
    /// handed over as `input.rows`. `:stream_id`, `:snapshot_id`,
    /// `:effort_id`, `:thread_id` and `:turn_id` bind the triggering
    /// event's anchors, `:event_id` its id and `:event_seq` its seq (NULL
    /// otherwise).
    pub input: Option<String>,
    /// A report file handed over as `input.report`.
    pub report: Option<ReportInput>,
    pub sync: CollectorSync,
    /// Host environment variables an exec collector gets.
    pub env: Vec<String>,
    /// Hosts an exec collector may reach; part of what a person approves.
    pub network: Vec<String>,
    /// Keychain secrets an exec collector gets as environment variables.
    pub credentials: Vec<String>,
    pub entities: Vec<EntityDecl>,
    /// The measures its facts land on (`repo.rust_clone`); a fact on any
    /// other is dropped.
    pub facts: Vec<String>,
    /// A report collector's kind: it parses `report` with `entry` and its
    /// output joins the run (`trigger: { on_run }`).
    pub records: Option<Records>,
}

impl CollectorSpec {
    /// The bundled parser a report collector names (`entry: oxplow:lcov`).
    pub fn bundled_parser(&self) -> Option<&str> {
        self.entry.as_deref()?.strip_prefix(BUNDLED_ENTRY)
    }
}

/// Whether `p` is a host pattern a program may be allowed to reach: a
/// lowercase host name (`api.github.com`, `localhost`) or a `*.` wildcard
/// over one; no scheme or port.
pub fn valid_host_pattern(p: &str) -> bool {
    let host = p.strip_prefix("*.").unwrap_or(p);
    !host.is_empty()
        && (host.contains('.') || host == "localhost")
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

/// SQL name of an entity's view: `v_<owner>_<entity>` (dashes in the
/// owner become underscores; extension names have none, so the mapping
/// is unambiguous). The project's own collectors' owner is `project`.
pub fn entity_view_name(owner: &str, entity: &str) -> String {
    format!("v_{}_{}", owner.replace('-', "_"), entity)
}

/// The owner of the collectors in `.oxplow/project.yaml`.
pub const PROJECT: &str = "project";

/// The owner of oxplow's own collectors (the bundled code metrics, run
/// when `metrics: - use: oxplow.<x>` enables one).
pub const BUILT_IN: &str = "built-in";

/// Parse a `collectors:` list owned by `owner` (an extension's name, or
/// [`PROJECT`]). `knows_event` says whether an `on:` type is registered.
/// Invalid collectors are skipped and described in the errors; valid
/// ones still load.
pub fn parse_collectors(
    owner: &str,
    value: &serde_yaml::Value,
    knows_event: &dyn Fn(&str) -> bool,
) -> (Vec<CollectorSpec>, Vec<String>) {
    let mut out: Vec<CollectorSpec> = Vec::new();
    let mut errors = Vec::new();
    let Some(items) = value.as_sequence() else {
        return (out, vec!["collectors: must be a list".into()]);
    };
    let mut seen_entities = std::collections::HashSet::new();
    for (i, item) in items.iter().enumerate() {
        let raw: RawCollector = match serde_yaml::from_value(item.clone()) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("collectors[{i}]: {e}"));
                continue;
            }
        };
        match validate(owner, raw, knows_event) {
            Ok(spec) => {
                if out.iter().any(|s| s.id == spec.id) {
                    errors.push(format!("collector `{}` is declared twice", spec.id));
                    continue;
                }
                if let Some(dup) = spec
                    .entities
                    .iter()
                    .find(|e| !seen_entities.insert(e.name.clone()))
                {
                    errors.push(format!(
                        "entity `{}` is declared by more than one collector",
                        dup.name
                    ));
                    continue;
                }
                out.push(spec);
            }
            Err(e) => errors.push(e),
        }
    }
    (out, errors)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCollector {
    id: String,
    #[serde(default)]
    doc: String,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    provider: Option<ProviderRead>,
    #[serde(default)]
    trigger: Option<serde_yaml::Value>,
    #[serde(default)]
    after: Vec<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    report: Option<RawReport>,
    #[serde(default)]
    sync: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    credentials: Vec<String>,
    #[serde(default)]
    network: Vec<String>,
    #[serde(default)]
    entities: Vec<RawEntity>,
    #[serde(default)]
    facts: Vec<String>,
    #[serde(default)]
    records: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReport {
    path: String,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntity {
    name: String,
    #[serde(default)]
    doc: String,
    key: String,
    /// Ordered `name: type` or `name: {type, doc}`.
    columns: serde_yaml::Mapping,
    #[serde(default)]
    relations: Vec<EntityRelation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColumn {
    #[serde(rename = "type")]
    col_type: String,
    #[serde(default)]
    doc: String,
}

/// Lowercase SQL-safe identifier: `[a-z][a-z0-9_]*`.
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A collector id: identifiers joined by dots (`prs`, `repo.scan_clone`).
fn is_collector_id(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_ident)
}

fn is_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_type(t: &str) -> Option<ColumnType> {
    Some(match t {
        "text" => ColumnType::Text,
        "int" => ColumnType::Int,
        "real" => ColumnType::Real,
        "bool" => ColumnType::Bool,
        "time" => ColumnType::Time,
        _ => return None,
    })
}

/// The `trigger:` value: `manual`, `{ every: 15m }` or
/// `{ on: [types], where?: { field: value } }`. An extension's effect
/// (`effects:`) reads its `on`/`where` through it too.
pub fn parse_trigger(
    value: Option<&serde_yaml::Value>,
    knows_event: &dyn Fn(&str) -> bool,
) -> Result<Trigger, String> {
    use serde_yaml::Value;
    let shapes = "use `manual`, `{ every: 15m }`, `{ on: [<event type>], where?: {…} }` or \
                  `{ on_run: test | analysis }`";
    let Some(value) = value else {
        return Ok(Trigger::Manual);
    };
    if value.as_str() == Some("manual") {
        return Ok(Trigger::Manual);
    }
    let Some(map) = value.as_mapping() else {
        return Err(format!("trigger: {shapes}"));
    };
    let get = |k: &str| map.get(Value::String(k.into()));
    if let Some(stray) = map
        .keys()
        .filter_map(Value::as_str)
        .find(|k| !matches!(*k, "every" | "on" | "where" | "on_run"))
    {
        return Err(format!("trigger: unknown key `{stray}`; {shapes}"));
    }
    if let Some(run) = get("on_run") {
        if map.len() > 1 {
            return Err(format!("trigger: `on_run` stands alone; {shapes}"));
        }
        return match run.as_str() {
            Some("test") => Ok(Trigger::OnRun { run: RunKind::Test }),
            Some("analysis") => Ok(Trigger::OnRun {
                run: RunKind::Analysis,
            }),
            _ => Err("trigger: `on_run` is `test` or `analysis`".into()),
        };
    }
    match (get("every"), get("on")) {
        (Some(every), None) => {
            if get("where").is_some() {
                return Err("trigger: `where` goes with `on`".into());
            }
            let minutes = every
                .as_str()
                .and_then(oxplow_domain::time::parse_every)
                .and_then(|d| u32::try_from(d.as_secs() / 60).ok())
                .ok_or_else(|| {
                    "trigger: `every` takes a duration like `15m` or `2h`".to_string()
                })?;
            Ok(Trigger::Every { minutes })
        }
        (None, Some(on)) => {
            let events: Vec<String> = match on {
                Value::String(s) => vec![s.clone()],
                Value::Sequence(items) => items
                    .iter()
                    .map(|i| i.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(|| "trigger: `on` lists event types".to_string())?,
                _ => return Err("trigger: `on` lists event types".into()),
            };
            if events.is_empty() {
                return Err("trigger: `on` names no event type".into());
            }
            if events.iter().any(|t| t == "collector.synced") {
                return Err(
                    "trigger: a collector can't run on `collector.synced` (a run would trigger \
                     itself); name the consumer to wait for in `after:` instead"
                        .into(),
                );
            }
            if let Some(unknown) = events.iter().find(|t| !knows_event(t)) {
                return Err(format!(
                    "trigger: `{unknown}` isn't a registered event type (see `v_event_type`)"
                ));
            }
            let mut filter = BTreeMap::new();
            if let Some(w) = get("where") {
                let w = w
                    .as_mapping()
                    .ok_or_else(|| "trigger: `where` maps payload fields to values".to_string())?;
                for (k, v) in w {
                    let (Some(k), Some(v)) = (k.as_str(), yaml_scalar(v)) else {
                        return Err("trigger: `where` maps payload fields to values".into());
                    };
                    filter.insert(k.to_string(), v);
                }
            }
            Ok(Trigger::On { events, filter })
        }
        _ => Err(format!("trigger: {shapes}")),
    }
}

fn yaml_scalar(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A path inside its owner's folder: relative, no `..`.
fn is_inside(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.is_empty()
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn validate(
    owner: &str,
    mut raw: RawCollector,
    knows_event: &dyn Fn(&str) -> bool,
) -> Result<CollectorSpec, String> {
    let id = std::mem::take(&mut raw.id);
    if !is_collector_id(&id) {
        return Err(format!(
            "collector id `{id}` must be lowercase identifiers (letters, digits, `_`), joined by \
             dots"
        ));
    }
    if id.starts_with("oxplow.") {
        return Err(format!(
            "collector id `{id}`: `oxplow.` is oxplow's own collectors' (enable one with \
             `metrics: - use: {id}`)"
        ));
    }
    let ctx = |m: String| format!("collector `{id}`: {m}");
    let trigger = parse_trigger(raw.trigger.as_ref(), knows_event).map_err(&ctx)?;
    if raw.records.is_some() {
        return validate_report_collector(owner, id, raw, trigger);
    }
    if matches!(trigger, Trigger::OnRun { .. }) {
        return Err(ctx(
            "`on_run` is a report collector's trigger (`records:`)".into()
        ));
    }
    let runtime = match raw.runtime.as_deref() {
        Some("exec") => CollectorRuntime::Exec,
        Some("starlark") => CollectorRuntime::Starlark,
        Some("jaq" | "jq") => CollectorRuntime::Jaq,
        Some("read") => CollectorRuntime::Read,
        Some(other) => {
            return Err(ctx(format!(
                "runtime `{other}` isn't supported (exec, starlark, jaq or read)"
            )))
        }
        None => return Err(ctx("needs a `runtime` (exec, starlark, jaq or read)".into())),
    };
    if raw
        .entry
        .as_deref()
        .is_some_and(|e| e.starts_with(BUNDLED_ENTRY))
    {
        return Err(ctx(format!(
            "`{BUNDLED_ENTRY}<parser>` names a report parser; it goes with `records:`"
        )));
    }
    let report = match raw.report.take() {
        None => None,
        Some(r) => Some(ReportInput {
            path: r.path,
            format: r.format.unwrap_or_else(text_format),
        }),
    };
    let records_facts = !raw.facts.is_empty();
    if records_facts && !raw.entities.is_empty() {
        return Err(ctx(
            "records entities or facts, not both: split it into two collectors".into(),
        ));
    }
    if owner == PROJECT && !records_facts {
        return Err(ctx(
            "a project collector records facts (`facts:`) or a report (`records:`); one that \
             writes entities belongs in an extension"
                .into(),
        ));
    }
    if records_facts {
        if runtime == CollectorRuntime::Exec && owner != PROJECT {
            return Err(ctx(
                "an extension's fact collector runs sandboxed (starlark or jaq); a program that \
                 fetches data writes entities"
                    .into(),
            ));
        }
        if !raw.env.is_empty() || !raw.credentials.is_empty() || !raw.network.is_empty() {
            return Err(ctx(
                "a fact collector gets no `env`, `credentials` or `network`: it reads the tree \
                 (`files()`), its `input` and its `report`"
                    .into(),
            ));
        }
    } else if report.is_some() {
        return Err(ctx(
            "`report` is read by fact collectors (`facts:`) and report collectors (`records:`)"
                .into(),
        ));
    }
    if !raw.after.is_empty() && !matches!(trigger, Trigger::On { .. }) {
        return Err(ctx("`after` goes with an `on:` trigger".into()));
    }
    let sync = match raw.sync.as_deref().unwrap_or("replace") {
        "replace" => CollectorSync::Replace,
        "upsert" => CollectorSync::Upsert,
        other => return Err(ctx(format!("sync `{other}`: use `replace` or `upsert`"))),
    };
    let input = raw
        .input
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty());
    match runtime {
        CollectorRuntime::Read => {
            if raw.provider.is_none() {
                return Err(ctx(
                    "a `read` collector names `provider: { instance, collector }`".into(),
                ));
            }
            if raw.entry.is_some()
                || input.is_some()
                || report.is_some()
                || !raw.entities.is_empty()
                || !raw.facts.is_empty()
            {
                return Err(ctx(
                    "a `read` collector takes only `provider` and `trigger`: its records land in \
                     the provider's capability (`v_work_item`)"
                        .into(),
                ));
            }
        }
        _ => {
            if raw.provider.is_some() {
                return Err(ctx("`provider` is for a `read` collector".into()));
            }
            match raw.entry.as_deref() {
                Some(entry) if is_inside(entry) => {}
                Some(entry) => {
                    return Err(ctx(format!(
                        "entry `{entry}` must be a path inside its folder"
                    )))
                }
                None => return Err(ctx("needs an `entry` (its program or script)".into())),
            }
            if raw.entities.is_empty() && raw.facts.is_empty() {
                return Err(ctx("declares no `entities` and no `facts`".into()));
            }
        }
    }
    if runtime == CollectorRuntime::Exec {
        if input.is_some() {
            return Err(ctx(
                "`input` is for starlark/jaq collectors; an exec collector fetches its own data"
                    .into(),
            ));
        }
    } else if !raw.env.is_empty() || !raw.credentials.is_empty() || !raw.network.is_empty() {
        // A sandboxed or read collector runs without anyone's approval, so
        // it gets nothing an approval would guard.
        return Err(ctx(format!(
            "a `{}` collector can't take `env`, `credentials` or `network`; those need an \
             approved `exec` collector",
            raw.runtime.as_deref().unwrap_or_default()
        )));
    }
    if let Some(report) = &report {
        if !is_inside(&report.path) {
            return Err(ctx(format!(
                "report `{}` must be a path inside the project",
                report.path
            )));
        }
        if !REPORT_FORMATS.contains(&report.format.as_str()) {
            return Err(ctx(format!(
                "report format `{}` isn't one of {}",
                report.format,
                REPORT_FORMATS.join(", ")
            )));
        }
    }
    if let Some(bad) = raw.env.iter().find(|e| !is_env_name(e)) {
        return Err(ctx(format!(
            "env name `{bad}` isn't a valid environment variable"
        )));
    }
    for c in &raw.credentials {
        if !is_env_name(c) {
            return Err(ctx(format!(
                "credential `{c}` isn't a valid environment variable name"
            )));
        }
        if raw.env.contains(c) {
            return Err(ctx(format!(
                "`{c}` is in both `env` and `credentials`; pick one"
            )));
        }
        if matches!(c.as_str(), "PATH" | "HOME") || c.starts_with("OXPLOW_") {
            return Err(ctx(format!(
                "credential `{c}` uses a reserved name (PATH, HOME, OXPLOW_*)"
            )));
        }
    }
    if let Some(bad) = raw.network.iter().find(|h| !valid_host_pattern(h)) {
        return Err(ctx(format!(
            "network host `{bad}` must be a lowercase host name like `api.github.com` or \
             `*.example.com` (no scheme or port)"
        )));
    }
    let mut network = raw.network;
    network.sort();
    network.dedup();
    let mut facts: Vec<String> = raw.facts.iter().map(|f| f.trim().to_string()).collect();
    if let Some(bad) = facts.iter().find(|f| !f.contains('.') || f.contains(' ')) {
        return Err(ctx(format!(
            "facts: `{bad}` isn't a measure key (`<namespace>.<name>`)"
        )));
    }
    facts.sort();
    facts.dedup();
    let mut entities = Vec::new();
    for e in raw.entities {
        let name = e.name;
        if !is_ident(&name) {
            return Err(ctx(format!(
                "entity name `{name}` must be lowercase letters, digits and underscores, \
                 starting with a letter"
            )));
        }
        let mut columns = Vec::new();
        for (k, v) in &e.columns {
            let col = k.as_str().unwrap_or_default().to_string();
            if !is_ident(&col) {
                return Err(ctx(format!(
                    "entity `{name}`: column name `{col}` must be a lowercase identifier"
                )));
            }
            let (type_name, doc) = match v {
                serde_yaml::Value::String(t) => (t.clone(), String::new()),
                other => match serde_yaml::from_value::<RawColumn>(other.clone()) {
                    Ok(c) => (c.col_type, c.doc),
                    Err(err) => return Err(ctx(format!("entity `{name}`, column `{col}`: {err}"))),
                },
            };
            let col_type = parse_type(&type_name).ok_or_else(|| {
                ctx(format!(
                    "entity `{name}`, column `{col}`: unknown type `{type_name}` (text, int, \
                     real, bool, time)"
                ))
            })?;
            columns.push(EntityColumn {
                name: col,
                col_type,
                doc,
            });
        }
        if columns.is_empty() {
            return Err(ctx(format!("entity `{name}` declares no columns")));
        }
        if !columns.iter().any(|c| c.name == e.key) {
            return Err(ctx(format!(
                "entity `{name}`: key `{}` isn't one of its columns",
                e.key
            )));
        }
        entities.push(EntityDecl {
            view: entity_view_name(owner, &name),
            name,
            doc: e.doc,
            key: e.key,
            columns,
            relations: e.relations,
        });
    }
    if let Some(sql) = &input {
        let lower = sql.to_ascii_lowercase();
        if let Some(own) = entities.iter().find(|e| lower.contains(&e.view)) {
            return Err(ctx(format!(
                "`input` reads its own entity `{}`; a collector can't feed on itself",
                own.view
            )));
        }
    }
    Ok(CollectorSpec {
        id,
        doc: raw.doc,
        runtime,
        entry: raw.entry,
        provider: raw.provider,
        trigger,
        after: raw.after,
        input,
        report,
        sync,
        env: raw.env,
        network,
        credentials: raw.credentials,
        entities,
        facts,
        records: None,
    })
}

/// A report collector (`records:`, tsk863): the project's, reading one
/// report file with a bundled parser (`entry: oxplow:<name>`) or its own
/// script or program, on `{ on_run: test | analysis }` or by hand. It gets
/// no `input`, `env`, `credentials` or `network`: its whole input is the
/// report.
fn validate_report_collector(
    owner: &str,
    id: String,
    raw: RawCollector,
    trigger: Trigger,
) -> Result<CollectorSpec, String> {
    let ctx = |m: String| format!("collector `{id}`: {m}");
    let records = match raw.records.as_deref() {
        Some("tests") => Records::Tests,
        Some("coverage") => Records::Coverage,
        Some("analysis") => Records::Analysis,
        other => {
            return Err(ctx(format!(
                "records `{}`: use `tests`, `coverage` or `analysis`",
                other.unwrap_or_default()
            )))
        }
    };
    if owner != PROJECT {
        return Err(ctx(
            "a report collector (`records:`) is the project's, in `.oxplow/project.yaml`".into(),
        ));
    }
    if !raw.facts.is_empty() || !raw.entities.is_empty() {
        return Err(ctx(
            "records a report, or facts, or entities: one per collector".into(),
        ));
    }
    if raw.input.is_some()
        || raw.provider.is_some()
        || raw.sync.is_some()
        || !raw.after.is_empty()
        || !raw.env.is_empty()
        || !raw.credentials.is_empty()
        || !raw.network.is_empty()
    {
        return Err(ctx(
            "a report collector reads its `report` and nothing else: no `input`, `provider`, \
             `sync`, `after`, `env`, `credentials` or `network`"
                .into(),
        ));
    }
    if !matches!(trigger, Trigger::OnRun { .. } | Trigger::Manual) {
        return Err(ctx(
            "a report collector runs `{ on_run: test | analysis }` or by hand (`manual`)".into(),
        ));
    }
    // A test run's reports are tests and coverage, an analysis run's are
    // analysis (tsk892): another pairing would be parsed and dropped.
    let reads = |run: RunKind| match run {
        RunKind::Test => records != Records::Analysis,
        RunKind::Analysis => records == Records::Analysis,
    };
    if let Trigger::OnRun { run } = trigger {
        if !reads(run) {
            let (name, by) = match records {
                Records::Tests => ("tests", "a test run"),
                Records::Coverage => ("coverage", "a test run"),
                Records::Analysis => ("analysis", "an analysis run"),
            };
            return Err(ctx(format!(
                "records `{name}`, which {by} reads: use `trigger: {{ on_run: {} }}`",
                if records == Records::Analysis {
                    "analysis"
                } else {
                    "test"
                }
            )));
        }
    }
    let Some(report) = raw.report else {
        return Err(ctx(
            "names the `report: { path }` it reads, relative to the project".into(),
        ));
    };
    if !is_inside(&report.path) {
        return Err(ctx(format!(
            "report `{}` must be a path inside the project",
            report.path
        )));
    }
    let Some(entry) = raw.entry else {
        return Err(ctx(format!(
            "needs an `entry`: a bundled parser (`{BUNDLED_ENTRY}<{}>`) or its own script",
            BUNDLED_PARSERS
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join("|")
        )));
    };
    let (runtime, format) = if let Some(name) = entry.strip_prefix(BUNDLED_ENTRY) {
        let Some(&BundledParser {
            records: parses,
            format,
            ..
        }) = bundled_parser(name)
        else {
            return Err(ctx(format!(
                "`{entry}` isn't a bundled parser ({})",
                BUNDLED_PARSERS
                    .iter()
                    .map(|p| format!("{BUNDLED_ENTRY}{}", p.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if parses != records {
            return Err(ctx(format!(
                "`{entry}` parses {}, not {}",
                records_name(parses),
                records_name(records)
            )));
        }
        if raw.runtime.is_some() || report.format.is_some() {
            return Err(ctx(format!(
                "`{entry}` brings its own runtime and report format; name neither"
            )));
        }
        (CollectorRuntime::Jaq, format.to_string())
    } else {
        if !is_inside(&entry) {
            return Err(ctx(format!(
                "entry `{entry}` must be a path inside the project"
            )));
        }
        let runtime = match raw.runtime.as_deref() {
            Some("exec") => CollectorRuntime::Exec,
            Some("starlark") => CollectorRuntime::Starlark,
            Some("jaq" | "jq") => CollectorRuntime::Jaq,
            other => {
                return Err(ctx(format!(
                    "a report parser runs as `jaq`, `starlark` or `exec` (got `{}`)",
                    other.unwrap_or("nothing")
                )))
            }
        };
        let format = match (runtime, report.format) {
            (CollectorRuntime::Exec, Some(_)) => {
                return Err(ctx(
                    "an `exec` parser reads the raw report on stdin; it takes no `format`".into(),
                ))
            }
            (CollectorRuntime::Exec, None) => text_format(),
            (_, format) => {
                let format = format.unwrap_or_else(text_format);
                if !REPORT_FORMATS.contains(&format.as_str()) {
                    return Err(ctx(format!(
                        "report format `{format}` isn't one of {}",
                        REPORT_FORMATS.join(", ")
                    )));
                }
                format
            }
        };
        (runtime, format)
    };
    Ok(CollectorSpec {
        id,
        doc: raw.doc,
        runtime,
        entry: Some(entry),
        provider: None,
        trigger,
        after: Vec::new(),
        input: None,
        report: Some(ReportInput {
            path: report.path,
            format,
        }),
        sync: CollectorSync::Replace,
        env: Vec::new(),
        network: Vec::new(),
        credentials: Vec::new(),
        entities: Vec::new(),
        facts: Vec::new(),
        records: Some(records),
    })
}

fn records_name(r: Records) -> &'static str {
    match r {
        Records::Tests => "tests",
        Records::Coverage => "coverage",
        Records::Analysis => "analysis",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knows(t: &str) -> bool {
        matches!(t, "snapshot.taken" | "effort.finished" | "vcs.head.moved")
    }

    fn parse(yaml: &str) -> (Vec<CollectorSpec>, Vec<String>) {
        let v: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        parse_collectors("my-gh", &v, &knows)
    }

    const GOOD: &str = r#"
- id: github
  doc: Pull requests from GitHub.
  runtime: exec
  entry: bin/sync.sh
  trigger: { every: 10m }
  env: [GITHUB_TOKEN]
  credentials: [GH_PAT]
  network: [api.github.com, "*.githubusercontent.com"]
  entities:
    - name: pr
      doc: One pull request.
      key: number
      columns:
        number: int
        title: { type: text, doc: PR title }
        merged_at: time
        draft: bool
      relations:
        - { to: v_task, on: "v_my_gh_pr.title LIKE '%tsk' || v_task.id || '%'" }
"#;

    #[test]
    fn parses_a_full_declaration() {
        let (out, errors) = parse(GOOD);
        assert!(errors.is_empty(), "{errors:?}");
        let s = &out[0];
        assert_eq!(s.id, "github");
        assert_eq!(s.entry.as_deref(), Some("bin/sync.sh"));
        assert_eq!(s.trigger, Trigger::Every { minutes: 10 });
        assert_eq!(s.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(s.credentials, vec!["GH_PAT"]);
        assert_eq!(
            s.network,
            vec!["*.githubusercontent.com", "api.github.com"],
            "sorted"
        );
        let e = &s.entities[0];
        assert_eq!(
            (e.name.as_str(), e.key.as_str(), e.view.as_str()),
            ("pr", "number", "v_my_gh_pr")
        );
        let cols: Vec<(&str, ColumnType)> = e
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.col_type))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("number", ColumnType::Int),
                ("title", ColumnType::Text),
                ("merged_at", ColumnType::Time),
                ("draft", ColumnType::Bool)
            ]
        );
        assert_eq!(e.columns[1].doc, "PR title");
        assert_eq!(e.relations[0].to, "v_task");
    }

    /// Every trigger shape parses; an `on:` type must be registered;
    /// `after` and `where` go only with `on:`.
    #[test]
    fn triggers() {
        let with = |t: &str| parse(&GOOD.replace("trigger: { every: 10m }", t));
        assert_eq!(with("trigger: manual").0[0].trigger, Trigger::Manual);
        assert_eq!(
            with("trigger: { every: 2h }").0[0].trigger,
            Trigger::Every { minutes: 120 }
        );
        assert_eq!(
            with("trigger: { on: [snapshot.taken], where: { trigger: git_refs } }").0[0].trigger,
            Trigger::On {
                events: vec!["snapshot.taken".into()],
                filter: [("trigger".to_string(), "git_refs".to_string())].into()
            }
        );
        assert_eq!(
            with("trigger: { on: effort.finished }\n  after: [change.analyze]").0[0].after,
            vec!["change.analyze"]
        );
        for (t, needle) in [
            ("trigger: hourly", "trigger"),
            ("trigger: { every: soon }", "duration"),
            (
                "trigger: { on: [nope.happened] }",
                "isn't a registered event type",
            ),
            (
                "trigger: { every: 5m, where: { a: b } }",
                "`where` goes with `on`",
            ),
            ("trigger: { every: 5m }\n  after: [x]", "`after` goes with"),
            ("trigger: { at: noon }", "unknown key `at`"),
            (
                "trigger: { on: [collector.synced] }",
                "can't run on `collector.synced`",
            ),
        ] {
            let (s, e) = with(t);
            assert!(s.is_empty(), "{t}");
            assert!(e.iter().any(|m| m.contains(needle)), "{t}: {e:?}");
        }
        // No trigger: manual.
        let (s, _) = parse(&GOOD.replace("  trigger: { every: 10m }\n", ""));
        assert_eq!(s[0].trigger, Trigger::Manual);
    }

    #[test]
    fn rejects_bad_declarations_one_by_one() {
        for (from, to, needle) in [
            ("runtime: exec", "runtime: python", "runtime"),
            ("key: number", "key: nope", "key"),
            ("number: int", "number: bigint", "type"),
            ("- name: pr", "- name: Pull-Requests", "entity name"),
            ("- id: github", "- id: Git Hub", "collector id"),
            ("- id: github", "- id: oxplow.todos", "oxplow's own"),
            ("entry: bin/sync.sh", "entry: ../outside.sh", "entry"),
            ("env: [GITHUB_TOKEN]", "env: [\"not ok\"]", "env"),
            (
                "credentials: [GH_PAT]",
                "credentials: [\"a-b\"]",
                "credential",
            ),
            (
                "credentials: [GH_PAT]",
                "credentials: [GITHUB_TOKEN]",
                "both",
            ),
            ("credentials: [GH_PAT]", "credentials: [PATH]", "reserved"),
            (
                "network: [api.github.com",
                "network: [\"https://api.github.com\"",
                "network host",
            ),
            (
                "credentials: [GH_PAT]",
                "credentials: [OXPLOW_X]",
                "reserved",
            ),
        ] {
            let (s, e) = parse(&GOOD.replace(from, to));
            assert!(s.is_empty(), "{to}: should be rejected");
            assert!(e.iter().any(|m| m.contains(needle)), "{to}: {e:?}");
        }
    }

    const DERIVED: &str = r#"
- id: hot
  runtime: starlark
  entry: collectors/hot.star
  input: "SELECT id, title FROM v_task WHERE priority = 'high'"
  sync: upsert
  entities:
    - name: hot_task
      key: id
      columns: { id: int, title: text }
"#;

    #[test]
    fn derived_collectors_take_an_input_and_no_secrets() {
        let (out, errors) = parse(DERIVED);
        assert!(errors.is_empty(), "{errors:?}");
        let s = &out[0];
        assert_eq!(
            (s.runtime, s.sync),
            (CollectorRuntime::Starlark, CollectorSync::Upsert)
        );
        assert!(s.runtime.is_derived());
        assert_eq!(
            s.input.as_deref(),
            Some("SELECT id, title FROM v_task WHERE priority = 'high'")
        );
        assert_eq!(parse(GOOD).0[0].sync, CollectorSync::Replace);
        for (from, to, needle) in [
            ("sync: upsert", "sync: merge", "sync"),
            ("sync: upsert", "sync: upsert\n  env: [HOME]", "can't take"),
            (
                "sync: upsert",
                "sync: upsert\n  credentials: [TOKEN]",
                "can't take",
            ),
            ("FROM v_task", "FROM v_my_gh_hot_task", "feed on itself"),
        ] {
            let (s, e) = parse(&DERIVED.replace(from, to));
            assert!(s.is_empty(), "{to}: should be rejected");
            assert!(e.iter().any(|m| m.contains(needle)), "{to}: {e:?}");
        }
        let (s, e) = parse(&GOOD.replace(
            "  entry: bin/sync.sh",
            "  entry: bin/sync.sh\n  input: SELECT 1",
        ));
        assert!(s.is_empty());
        assert!(e[0].contains("`input` is for"), "{e:?}");
    }

    /// tsk935: a report collector's report and its own parser are paths
    /// inside the project — `..` or an absolute path is refused at load
    /// (and a symlink out, when it is read: `in_checkout`).
    #[test]
    fn a_report_collectors_paths_stay_in_the_project() {
        for (yaml, needle) in [
            (
                "- { id: t.a, records: tests, entry: oxplow:junit, report: { path: ../out.xml }, trigger: { on_run: test } }",
                "report `../out.xml` must be a path inside the project",
            ),
            (
                "- { id: t.a, records: tests, entry: oxplow:junit, report: { path: /tmp/out.xml }, trigger: { on_run: test } }",
                "report `/tmp/out.xml` must be a path inside the project",
            ),
            (
                "- { id: t.a, records: tests, runtime: jaq, entry: ../p.jq, report: { path: out.xml, format: xml }, trigger: { on_run: test } }",
                "entry `../p.jq` must be a path inside the project",
            ),
            (
                "- { id: t.a, records: tests, runtime: jaq, entry: /p.jq, report: { path: out.xml, format: xml }, trigger: { on_run: test } }",
                "entry `/p.jq` must be a path inside the project",
            ),
        ] {
            let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
            let (s, e) = parse_collectors(PROJECT, &value, &|_| true);
            assert!(s.is_empty(), "{yaml}");
            assert!(e.iter().any(|m| m.contains(needle)), "{yaml}: {e:?}");
        }
    }

    /// A fact collector (what a gauge was) declares its measures; a
    /// report is a path inside the project in a known format; a collector
    /// with neither entities nor facts is refused.
    #[test]
    fn fact_collectors_and_reports() {
        let (out, errors) = parse(
            r#"
- id: repo.scan_clone
  runtime: starlark
  entry: oxplow/gauges/repo_clone.star
  trigger: { on: [snapshot.taken] }
  facts: [repo.rust_clone]
- id: repo.type_coverage
  runtime: jaq
  entry: oxplow/plugins/type_coverage.jq
  report: { path: target/type-coverage.json, format: json }
  trigger: { on: [snapshot.taken] }
  facts: [repo.type_coverage]
"#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(out[0].facts, vec!["repo.rust_clone"]);
        assert_eq!(out[1].report.as_ref().unwrap().format, "json");
        for (yaml, needle) in [
            (
                "- { id: a.b, runtime: starlark, entry: x.star }",
                "no `entities` and no `facts`",
            ),
            (
                "- { id: a.b, runtime: starlark, entry: x.star, facts: [nodot] }",
                "isn't a measure key",
            ),
            (
                "- { id: a.b, runtime: jaq, entry: x.jq, facts: [a.b], report: { path: /etc/x } }",
                "inside the project",
            ),
            (
                "- { id: a.b, runtime: jaq, entry: x.jq, facts: [a.b], report: { path: r, format: csv } }",
                "format `csv`",
            ),
        ] {
            let (s, e) = parse(yaml);
            assert!(s.is_empty(), "{yaml}");
            assert!(e.iter().any(|m| m.contains(needle)), "{yaml}: {e:?}");
        }
    }

    /// A `read` collector names a provider's collector and nothing a
    /// script would need.
    #[test]
    fn read_collectors_name_a_provider_collector() {
        let (out, errors) = parse(
            "- { id: issues, runtime: read, provider: { instance: tracker/linear, collector: issues }, trigger: { every: 10m } }",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(out[0].provider.as_ref().unwrap().collector, "issues");
        let (s, e) = parse("- { id: issues, runtime: read, entry: x.sh }");
        assert!(s.is_empty());
        assert!(e[0].contains("names `provider"), "{e:?}");
        let (s, e) = parse(
            "- { id: issues, runtime: read, provider: { instance: a/b, collector: c }, facts: [a.b] }",
        );
        assert!(s.is_empty());
        assert!(e[0].contains("takes only `provider`"), "{e:?}");
    }

    fn parse_project(yaml: &str) -> (Vec<CollectorSpec>, Vec<String>) {
        let v: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        parse_collectors(PROJECT, &v, &knows)
    }

    /// tsk863: a report collector names a bundled parser or its own
    /// script, the report it reads, and the run it reads after.
    #[test]
    fn report_collectors() {
        let (out, errors) = parse_project(
            r#"
- id: tests.rust_coverage
  records: coverage
  entry: oxplow:lcov
  report: { path: target/coverage/lcov.info }
  trigger: { on_run: test }
- id: lint.custom
  records: analysis
  runtime: starlark
  entry: oxplow/parsers/lint.star
  report: { path: target/lint.json, format: json }
  trigger: { on_run: analysis }
- id: tests.by_hand
  records: tests
  runtime: exec
  entry: bin/parse
  report: { path: out.txt }
"#,
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(out[0].records, Some(Records::Coverage));
        assert_eq!(out[0].bundled_parser(), Some("lcov"));
        assert_eq!(out[0].runtime, CollectorRuntime::Jaq);
        assert_eq!(out[0].report.as_ref().unwrap().format, "lcov");
        assert_eq!(out[0].trigger, Trigger::OnRun { run: RunKind::Test });
        assert_eq!(out[1].runtime, CollectorRuntime::Starlark);
        assert_eq!(out[1].report.as_ref().unwrap().format, "json");
        assert_eq!(
            out[1].trigger,
            Trigger::OnRun {
                run: RunKind::Analysis
            }
        );
        assert_eq!(out[2].runtime, CollectorRuntime::Exec);
        assert_eq!(out[2].trigger, Trigger::Manual);
        assert_eq!(out[2].bundled_parser(), None);
    }

    #[test]
    fn report_collectors_refuse_what_they_cant_be() {
        let refused = |yaml: &str, needle: &str| {
            let (out, errors) = parse_project(yaml);
            assert!(out.is_empty(), "{yaml}");
            assert!(
                errors.iter().any(|e| e.contains(needle)),
                "{yaml}: {errors:?}"
            );
        };
        let base = "- id: r\n  report: { path: r.xml }\n";
        refused(
            &format!("{base}  records: coverage\n  entry: oxplow:junit\n"),
            "parses tests, not coverage",
        );
        refused(
            &format!("{base}  records: tests\n  entry: oxplow:nunit\n"),
            "isn't a bundled parser",
        );
        refused(
            &format!("{base}  records: tests\n  entry: oxplow:junit\n  runtime: jaq\n"),
            "name neither",
        );
        refused(
            "- id: r\n  records: tests\n  entry: oxplow:junit\n  report: { path: r.xml, format: xml }\n",
            "name neither",
        );
        refused(
            "- id: r\n  records: tests\n  runtime: exec\n  entry: p\n  report: { path: r.xml, format: xml }\n",
            "stdin",
        );
        refused(
            &format!("{base}  records: tests\n  entry: oxplow:junit\n  trigger: {{ on: [snapshot.taken] }}\n"),
            "on_run",
        );
        refused(
            "- id: r\n  records: tests\n  entry: oxplow:junit\n",
            "report: { path }",
        );
        refused(
            &format!("{base}  records: tests\n  entry: oxplow:junit\n  network: [api.x.com]\n"),
            "nothing else",
        );
        refused(
            &format!("{base}  records: logs\n  entry: oxplow:junit\n"),
            "records `logs`",
        );
        // tsk892: a test run's leg reads tests and coverage, an analysis
        // run's analysis — anything else would be parsed and dropped.
        refused(
            &format!("{base}  records: analysis\n  entry: oxplow:eslint\n  trigger: {{ on_run: test }}\n"),
            "records `analysis`, which an analysis run reads",
        );
        refused(
            &format!("{base}  records: coverage\n  entry: oxplow:lcov\n  trigger: {{ on_run: analysis }}\n"),
            "records `coverage`, which a test run reads",
        );
        refused(
            "- id: r\n  runtime: jaq\n  entry: r.jq\n  facts: [a.b]\n  trigger: { on_run: test }\n",
            "report collector's trigger",
        );
        refused(
            "- id: r\n  runtime: jaq\n  entry: oxplow:lcov\n  facts: [a.b]\n",
            "goes with `records:`",
        );
        // An extension's collectors don't read reports.
        let (_, errors) = parse(&format!("{base}  records: tests\n  entry: oxplow:junit\n"));
        assert!(errors.iter().any(|e| e.contains("project's")), "{errors:?}");
    }

    #[test]
    fn unknown_keys_are_errors() {
        let (s, e) = parse(&GOOD.replace("  runtime: exec", "  runtime: exec\n  netwrk: [x]"));
        assert!(s.is_empty());
        assert!(e[0].contains("netwrk"), "{e:?}");
    }

    /// A collector records entities or facts, not both. A fact collector
    /// is sandboxed (an extension's) or a project's own program; it gets
    /// no env, credentials or network. The project's collectors record
    /// facts.
    #[test]
    fn fact_and_entity_collectors_are_separate() {
        let (s, e) = parse(
            "- { id: a.b, runtime: starlark, entry: x.star, facts: [a.b], entities: [{ name: t, key: k, columns: { k: int } }] }",
        );
        assert!(s.is_empty());
        assert!(e[0].contains("entities or facts, not both"), "{e:?}");
        let (s, e) = parse("- { id: a.b, runtime: exec, entry: x.sh, facts: [a.b] }");
        assert!(s.is_empty());
        assert!(e[0].contains("sandboxed"), "{e:?}");
        let project = |yaml: &str| {
            let v: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
            parse_collectors(PROJECT, &v, &knows)
        };
        let (s, e) = project("- { id: a.b, runtime: exec, entry: x.sh, facts: [a.b] }");
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(s[0].runtime, CollectorRuntime::Exec);
        let (s, e) = project(
            "- { id: a.b, runtime: exec, entry: x.sh, facts: [a.b], network: [api.github.com] }",
        );
        assert!(s.is_empty());
        assert!(e[0].contains("gets no `env`"), "{e:?}");
        let (s, e) = project(GOOD);
        assert!(s.is_empty());
        assert!(e[0].contains("records facts"), "{e:?}");
    }

    #[test]
    fn host_patterns() {
        for ok in ["api.github.com", "*.githubusercontent.com", "localhost"] {
            assert!(valid_host_pattern(ok), "{ok}");
        }
        for bad in ["https://x.com", "x.com:443", "UPPER.com", "*.", "nodot", ""] {
            assert!(!valid_host_pattern(bad), "{bad}");
        }
    }
}
