//! `collectors:` (P7.B3, `.context/semantic-layer.md` "Collectors"): the
//! one declaration for bringing data in — an extension's in
//! `extension.yaml`, the project's in `.oxplow/project.yaml`.
//!
//! A collector is a program (`exec`, approved by a person), a sandboxed
//! script (`starlark` / `jaq`, no I/O, no approval) or a provider's read
//! (`read`). It writes **entities** (rows a model can `ref()`) and/or
//! **facts** (measurements on declared measures), and it runs when its
//! **trigger** says: `manual`, `{ every: 15m }`, or
//! `{ on: [<event types>], where: { <payload field>: <value> } }` — the
//! last, when such an event is logged, after the consumers named in
//! `after:` have handled it.
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
}

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
    /// `<extension>/<provider id>`.
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
    /// `:effort_id`, `:thread_id`, `:turn_id` and `:event_id` bind the
    /// triggering event's anchors (NULL otherwise).
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
    runtime: String,
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
    report: Option<ReportInput>,
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

/// `15m`, `2h` → minutes.
fn parse_every(s: &str) -> Option<u32> {
    let s = s.trim();
    let (n, mult) = match s.strip_suffix('m') {
        Some(n) => (n, 1),
        None => (s.strip_suffix('h')?, 60),
    };
    n.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .map(|n| n * mult)
}

/// The `trigger:` value: `manual`, `{ every: 15m }` or
/// `{ on: [types], where?: { field: value } }`.
fn parse_trigger(
    value: Option<&serde_yaml::Value>,
    knows_event: &dyn Fn(&str) -> bool,
) -> Result<Trigger, String> {
    use serde_yaml::Value;
    let shapes = "use `manual`, `{ every: 15m }` or `{ on: [<event type>], where?: {…} }`";
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
        .find(|k| !matches!(*k, "every" | "on" | "where"))
    {
        return Err(format!("trigger: unknown key `{stray}`; {shapes}"));
    }
    match (get("every"), get("on")) {
        (Some(every), None) => {
            if get("where").is_some() {
                return Err("trigger: `where` goes with `on`".into());
            }
            let minutes = every.as_str().and_then(parse_every).ok_or_else(|| {
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
    raw: RawCollector,
    knows_event: &dyn Fn(&str) -> bool,
) -> Result<CollectorSpec, String> {
    let id = raw.id;
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
    let runtime = match raw.runtime.as_str() {
        "exec" => CollectorRuntime::Exec,
        "starlark" => CollectorRuntime::Starlark,
        "jaq" | "jq" => CollectorRuntime::Jaq,
        "read" => CollectorRuntime::Read,
        other => {
            return Err(ctx(format!(
                "runtime `{other}` isn't supported (exec, starlark, jaq or read)"
            )))
        }
    };
    let trigger = parse_trigger(raw.trigger.as_ref(), knows_event).map_err(&ctx)?;
    let records_facts = !raw.facts.is_empty();
    if records_facts && !raw.entities.is_empty() {
        return Err(ctx(
            "records entities or facts, not both: split it into two collectors".into(),
        ));
    }
    if owner == PROJECT && !records_facts {
        return Err(ctx(
            "a project collector records facts (`facts:`); one that writes entities belongs in \
             an extension"
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
    } else if raw.report.is_some() {
        return Err(ctx("`report` is read by fact collectors (`facts:`)".into()));
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
                || raw.report.is_some()
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
            raw.runtime
        )));
    }
    if let Some(report) = &raw.report {
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
        report: raw.report,
        sync,
        env: raw.env,
        network,
        credentials: raw.credentials,
        entities,
        facts,
    })
}

/// Rewrite a `gauges:` block (the retired fact producers, in
/// `.oxplow/project.yaml` or an `extension.yaml`) as `collectors:`, in
/// place: every other line is left as it is. Idempotent: text without a
/// top-level `gauges:` comes back unchanged. Refused, naming the gauge,
/// for what a collector can't express (`args`, an `on-report` or
/// `continuous` trigger), and when the text already has `collectors:`.
pub fn migrate_gauges_text(text: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let top = |l: &str| !l.starts_with([' ', '\t', '-', '#']) && !l.trim().is_empty();
    let key = |l: &str, k: &str| top(l) && l.strip_prefix(k).is_some_and(|r| r.starts_with(':'));
    let Some(start) = lines.iter().position(|l| key(l, "gauges")) else {
        return Ok(text.to_string());
    };
    if lines.iter().any(|l| key(l, "collectors")) {
        return Err(
            "this file has both `gauges:` and `collectors:`; move the gauges into \
             `collectors:` by hand"
                .into(),
        );
    }
    let end = lines[start + 1..]
        .iter()
        .position(|l| top(l))
        .map_or(lines.len(), |i| start + 1 + i);
    let block: serde_yaml::Value =
        serde_yaml::from_str(&lines[start..end].concat()).map_err(|e| format!("gauges: {e}"))?;
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("collectors".into(), gauges_to_collectors(&block["gauges"])?);
    let rendered = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))
        .map_err(|e| format!("collectors: {e}"))?;
    Ok(format!(
        "{}{rendered}{}",
        lines[..start].concat(),
        lines[end..].concat()
    ))
}

/// A `gauges:` list as the `collectors:` list it becomes: `key` → `id`,
/// `title` → `doc`, `on-snapshot` (the default) → `{ on: [snapshot.taken] }`,
/// `on-effort-complete` → `{ on: [effort.finished] }`, `emits` → `facts`,
/// `compute.{runtime, entryFile}` → `runtime` / `entry`, and
/// `compute.report` with its `input` → `report: { path, format }`.
pub fn gauges_to_collectors(gauges: &serde_yaml::Value) -> Result<serde_yaml::Value, String> {
    use serde_yaml::{Mapping, Value};
    let items = match gauges {
        Value::Sequence(items) => items.as_slice(),
        Value::Null => &[],
        _ => return Err("gauges: must be a list".into()),
    };
    let mut out = Vec::new();
    for (i, g) in items.iter().enumerate() {
        let id = g["key"]
            .as_str()
            .ok_or_else(|| format!("gauges[{i}] has no `key`"))?;
        let ctx = |m: &str| format!("gauge `{id}`: {m}");
        let compute = &g["compute"];
        let mut c = Mapping::new();
        c.insert("id".into(), id.into());
        if let Some(title) = g["title"].as_str() {
            c.insert("doc".into(), title.into());
        }
        let runtime = compute["runtime"]
            .as_str()
            .ok_or_else(|| ctx("no `compute.runtime`"))?;
        c.insert("runtime".into(), runtime.into());
        let entry = compute["entryFile"]
            .as_str()
            .ok_or_else(|| ctx("no `compute.entryFile`"))?;
        c.insert("entry".into(), entry.into());
        let on = |event: &str| {
            let mut t = Mapping::new();
            t.insert("on".into(), Value::Sequence(vec![event.into()]));
            Value::Mapping(t)
        };
        let trigger = match g["trigger"].as_str() {
            None | Some("on-snapshot") => on("snapshot.taken"),
            Some("on-effort-complete") => on("effort.finished"),
            Some("manual") => "manual".into(),
            Some(other) => {
                return Err(ctx(&format!(
                    "trigger `{other}` has no collector equivalent (it never ran): drop the \
                     gauge, or make it on-snapshot, on-effort-complete or manual first"
                )))
            }
        };
        c.insert("trigger".into(), trigger);
        if compute["args"].as_sequence().is_some_and(|a| !a.is_empty()) {
            return Err(ctx(
                "`compute.args` has no collector equivalent: read what the program needs from \
                 its own files",
            ));
        }
        if let Some(path) = compute["report"].as_str() {
            let mut r = Mapping::new();
            r.insert("path".into(), path.into());
            r.insert(
                "format".into(),
                compute["input"].as_str().unwrap_or("text").into(),
            );
            c.insert("report".into(), Value::Mapping(r));
        }
        c.insert("facts".into(), g["emits"].clone());
        out.push(Value::Mapping(c));
    }
    Ok(Value::Sequence(out))
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

    const GAUGES: &str = "name: x\ngauges:\n- key: repo.scan_clone\n  title: Rust .clone() scan\n  trigger: on-snapshot\n  emits:\n  - repo.rust_clone\n  compute:\n    runtime: starlark\n    entryFile: oxplow/gauges/repo_clone.star\n- key: repo.done\n  trigger: on-effort-complete\n  emits: [repo.done]\n  compute: { runtime: starlark, entryFile: d.star }\n- key: repo.scan_type_coverage\n  emits:\n  - repo.type_coverage\n  compute:\n    runtime: jaq\n    input: json\n    entryFile: oxplow/plugins/type_coverage.jq\n    report: target/type-coverage.json\nmeasures:\n- key: repo.rust_clone\n";

    /// `gauges:` becomes `collectors:` in place: the rest of the text is
    /// untouched, the result parses, and running it again changes nothing.
    #[test]
    fn gauges_migrate_to_collectors() {
        let out = migrate_gauges_text(GAUGES).unwrap();
        assert!(out.starts_with("name: x\ncollectors:\n"), "{out}");
        assert!(
            out.ends_with("measures:\n- key: repo.rust_clone\n"),
            "{out}"
        );
        assert!(!out.contains("\ngauges:"), "{out}");
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let (specs, errors) = parse_collectors(PROJECT, &doc["collectors"], &knows);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(specs[0].id, "repo.scan_clone");
        assert_eq!(specs[0].doc, "Rust .clone() scan");
        assert_eq!(
            specs[0].trigger,
            Trigger::On {
                events: vec!["snapshot.taken".into()],
                filter: BTreeMap::new()
            }
        );
        assert_eq!(
            specs[0].entry.as_deref(),
            Some("oxplow/gauges/repo_clone.star")
        );
        assert_eq!(specs[0].facts, vec!["repo.rust_clone"]);
        assert_eq!(
            specs[1].trigger,
            Trigger::On {
                events: vec!["effort.finished".into()],
                filter: BTreeMap::new()
            }
        );
        assert_eq!(
            specs[2].report,
            Some(ReportInput {
                path: "target/type-coverage.json".into(),
                format: "json".into()
            })
        );
        assert_eq!(migrate_gauges_text(&out).unwrap(), out);
        assert_eq!(migrate_gauges_text("name: x\n").unwrap(), "name: x\n");
    }

    #[test]
    fn a_gauge_the_migration_cant_express_is_named() {
        for (from, to, needle) in [
            ("trigger: on-snapshot", "trigger: continuous", "continuous"),
            (
                "    entryFile: oxplow/gauges/repo_clone.star",
                "    entryFile: a.sh\n    args: [x]",
                "args",
            ),
        ] {
            let e = migrate_gauges_text(&GAUGES.replace(from, to)).unwrap_err();
            assert!(e.contains("repo.scan_clone") && e.contains(needle), "{e}");
        }
        let both = format!("{GAUGES}collectors: []\n");
        assert!(migrate_gauges_text(&both).unwrap_err().contains("both"));
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
