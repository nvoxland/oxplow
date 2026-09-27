//! Source declarations in `extension.yaml`: code an extension runs to
//! bring external records (GitHub PRs, Linear issues, …) into the
//! semantic layer as entities. See `.context/semantic-layer.md` →
//! "User and extension sources".
//!
//! This module only parses and validates the declaration. Running a
//! source and storing its rows live elsewhere.

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
pub struct SourceColumn {
    pub name: String,
    pub col_type: ColumnType,
    pub doc: String,
}

/// A documented join from this entity to another view. Not executed;
/// it tells agents and lens authors how the data connects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRelation {
    /// The view it joins to, e.g. `v_task` or `v_github_review`.
    pub to: String,
    /// The SQL join condition, e.g. `v_github_pr.head_branch = v_stream.branch`.
    pub on: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceEntity {
    pub name: String,
    pub doc: String,
    /// Column that uniquely identifies a row.
    pub key: String,
    pub columns: Vec<SourceColumn>,
    pub relations: Vec<SourceRelation>,
    /// SQL name lenses and agents query: `v_<extension>_<entity>`.
    pub view: String,
}

/// When a source runs by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SourceSchedule {
    /// Only when someone asks (Sync Now / `run_source`).
    Manual,
    /// Every `minutes` minutes, once approved.
    Every { minutes: u32 },
}

/// What runs a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SourceRuntime {
    /// A program: can reach the network and credentials, so it needs a
    /// person's approval to run.
    Exec,
    /// A Starlark script deriving entities from its `input` rows. No I/O,
    /// so no approval.
    Starlark,
    /// A jq program deriving entities from its `input` rows. No I/O, so no
    /// approval.
    Jaq,
}

impl SourceRuntime {
    /// Sandboxed in-process: no I/O, no approval.
    pub fn is_derived(self) -> bool {
        !matches!(self, SourceRuntime::Exec)
    }
}

/// How a run's output lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SourceSync {
    /// Each run restates every entity (one it doesn't mention is emptied).
    Replace,
    /// Each run adds or updates rows by key, and removes the keys it lists
    /// under `deleted`; an entity it doesn't mention is left alone.
    Upsert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpec {
    pub id: String,
    pub doc: String,
    pub runtime: SourceRuntime,
    /// Path to the program or script, relative to the extension folder.
    pub entry: String,
    /// A derived source's input: read-only SQL over the semantic layer,
    /// handed to the script as `{"rows": [...]}`.
    pub input: Option<String>,
    pub sync: SourceSync,
    pub schedule: SourceSchedule,
    /// Host environment variables passed through to the entry
    /// (e.g. `GITHUB_TOKEN`). Nothing else from the host env is.
    pub env: Vec<String>,
    /// Hosts an exec source may reach (`api.github.com`,
    /// `*.githubusercontent.com`). Part of what a person approves; enforced
    /// where the OS allows (see `net_sandbox`). Empty = no network.
    pub network: Vec<String>,
    /// Secrets the entry gets as environment variables of these names.
    /// Values live in the OS keychain, set by a person in Settings →
    /// Extensions, scoped to this extension.
    pub credentials: Vec<String>,
    pub entities: Vec<SourceEntity>,
}

/// Parse the `sources:` value of an extension manifest. Invalid sources
/// are skipped and described in the returned errors; valid ones still
/// load.
pub fn parse_sources(extension: &str, value: &serde_yaml::Value) -> (Vec<SourceSpec>, Vec<String>) {
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    let Some(items) = value.as_sequence() else {
        return (sources, vec!["sources: must be a list".into()]);
    };
    let mut seen_entities = std::collections::HashSet::new();
    for (i, item) in items.iter().enumerate() {
        let raw: RawSource = match serde_yaml::from_value(item.clone()) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("sources[{i}]: {e}"));
                continue;
            }
        };
        match validate(extension, raw) {
            Ok(spec) => {
                if sources.iter().any(|s: &SourceSpec| s.id == spec.id) {
                    errors.push(format!("source `{}` is declared twice", spec.id));
                    continue;
                }
                if let Some(dup) = spec
                    .entities
                    .iter()
                    .find(|e| !seen_entities.insert(e.name.clone()))
                {
                    errors.push(format!(
                        "entity `{}` is declared by more than one source",
                        dup.name
                    ));
                    continue;
                }
                sources.push(spec);
            }
            Err(e) => errors.push(e),
        }
    }
    (sources, errors)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    id: String,
    #[serde(default)]
    doc: String,
    runtime: String,
    entry: String,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    sync: Option<String>,
    #[serde(default = "manual")]
    schedule: String,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    credentials: Vec<String>,
    #[serde(default)]
    network: Vec<String>,
    entities: Vec<RawEntity>,
}

fn manual() -> String {
    "manual".into()
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
    relations: Vec<SourceRelation>,
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

fn parse_schedule(s: &str) -> Option<SourceSchedule> {
    let s = s.trim();
    if s == "manual" {
        return Some(SourceSchedule::Manual);
    }
    let rest = s.strip_prefix("every ")?.trim();
    let (n, mult) = match rest.strip_suffix('m') {
        Some(n) => (n, 1),
        None => (rest.strip_suffix('h')?, 60),
    };
    let n: u32 = n.trim().parse().ok().filter(|n| *n > 0)?;
    Some(SourceSchedule::Every { minutes: n * mult })
}

/// SQL name of an entity's view: `v_<extension>_<entity>` (dashes in
/// the extension name become underscores; extension names can't contain
/// underscores, so the mapping is unambiguous).
pub fn entity_view_name(extension: &str, entity: &str) -> String {
    format!("v_{}_{}", extension.replace('-', "_"), entity)
}

fn validate(extension: &str, raw: RawSource) -> Result<SourceSpec, String> {
    let id = raw.id;
    if !is_ident(&id) {
        return Err(format!(
            "source id `{id}` must be lowercase letters, digits and underscores, starting with a letter"
        ));
    }
    let ctx = |m: String| format!("source `{id}`: {m}");
    let runtime = match raw.runtime.as_str() {
        "exec" => SourceRuntime::Exec,
        "starlark" => SourceRuntime::Starlark,
        "jaq" | "jq" => SourceRuntime::Jaq,
        other => {
            return Err(ctx(format!(
                "runtime `{other}` isn't supported (exec, starlark or jaq)"
            )))
        }
    };
    let sync = match raw.sync.as_deref().unwrap_or("replace") {
        "replace" => SourceSync::Replace,
        "upsert" => SourceSync::Upsert,
        other => return Err(ctx(format!("sync `{other}`: use `replace` or `upsert`"))),
    };
    let input = raw
        .input
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty());
    if runtime.is_derived() {
        // A derived source runs without anyone's approval, so it gets
        // nothing an approval would guard.
        if !raw.env.is_empty() || !raw.credentials.is_empty() || !raw.network.is_empty() {
            return Err(ctx(format!(
                "a `{}` source can't take `env`, `credentials` or `network`; those need an approved `exec` source",
                raw.runtime
            )));
        }
    } else if input.is_some() {
        return Err(ctx(
            "`input` is for starlark/jaq sources; an exec source fetches its own data".into(),
        ));
    }
    let entry = raw.entry;
    let entry_path = std::path::Path::new(&entry);
    if entry.is_empty()
        || entry_path.is_absolute()
        || entry_path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(ctx(format!(
            "entry `{entry}` must be a path inside the extension folder"
        )));
    }
    let schedule = parse_schedule(&raw.schedule).ok_or_else(|| {
        ctx(format!(
            "schedule `{}`: use `manual` or `every <n>m` / `every <n>h`",
            raw.schedule
        ))
    })?;
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
    if let Some(bad) = raw
        .network
        .iter()
        .find(|h| !crate::net_sandbox::valid_host_pattern(h))
    {
        return Err(ctx(format!(
            "network host `{bad}` must be a lowercase host name like `api.github.com` or `*.example.com` (no scheme or port)"
        )));
    }
    let mut network = raw.network;
    network.sort();
    network.dedup();
    if raw.entities.is_empty() {
        return Err(ctx("declares no entities".into()));
    }
    let mut entities = Vec::new();
    for e in raw.entities {
        let name = e.name;
        if !is_ident(&name) {
            return Err(ctx(format!(
                "entity name `{name}` must be lowercase letters, digits and underscores, starting with a letter"
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
                    "entity `{name}`, column `{col}`: unknown type `{type_name}` (text, int, real, bool, time)"
                ))
            })?;
            columns.push(SourceColumn {
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
        entities.push(SourceEntity {
            view: entity_view_name(extension, &name),
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
                "`input` reads its own entity `{}`; a source can't feed on itself",
                own.view
            )));
        }
    }
    Ok(SourceSpec {
        id,
        doc: raw.doc,
        runtime,
        entry,
        input,
        sync,
        schedule,
        env: raw.env,
        network,
        credentials: raw.credentials,
        entities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<SourceSpec>, Vec<String>) {
        let v: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        parse_sources("my-gh", &v)
    }

    const GOOD: &str = r#"
- id: github
  doc: Pull requests from GitHub.
  runtime: exec
  entry: bin/sync.sh
  schedule: every 10m
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
        let (sources, errors) = parse(GOOD);
        assert!(errors.is_empty(), "{errors:?}");
        let s = &sources[0];
        assert_eq!(s.id, "github");
        assert_eq!(s.entry, "bin/sync.sh");
        assert_eq!(s.schedule, SourceSchedule::Every { minutes: 10 });
        assert_eq!(s.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(s.credentials, vec!["GH_PAT"]);
        assert_eq!(
            s.network,
            vec!["*.githubusercontent.com", "api.github.com"],
            "sorted"
        );
        let e = &s.entities[0];
        assert_eq!(e.name, "pr");
        assert_eq!(e.key, "number");
        assert_eq!(e.view, "v_my_gh_pr");
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

    #[test]
    fn schedules() {
        let with = |s: &str| parse(&GOOD.replace("every 10m", s));
        assert_eq!(with("manual").0[0].schedule, SourceSchedule::Manual);
        assert_eq!(
            with("every 2h").0[0].schedule,
            SourceSchedule::Every { minutes: 120 }
        );
        let (s, e) = with("hourly");
        assert!(s.is_empty());
        assert!(e[0].contains("schedule"), "{e:?}");
    }

    #[test]
    fn rejects_bad_declarations_one_by_one() {
        for (from, to, needle) in [
            ("runtime: exec", "runtime: python", "runtime"),
            ("key: number", "key: nope", "key"),
            ("number: int", "number: bigint", "type"),
            ("- name: pr", "- name: Pull-Requests", "entity name"),
            ("- id: github", "- id: Git Hub", "source id"),
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
  entry: sources/hot.star
  input: "SELECT id, title FROM v_task WHERE priority = 'high'"
  sync: upsert
  entities:
    - name: hot_task
      key: id
      columns: { id: int, title: text }
"#;

    #[test]
    fn derived_sources_take_an_input_and_no_secrets() {
        let (sources, errors) = parse(DERIVED);
        assert!(errors.is_empty(), "{errors:?}");
        let s = &sources[0];
        assert_eq!(
            (s.runtime, s.sync),
            (SourceRuntime::Starlark, SourceSync::Upsert)
        );
        assert!(s.runtime.is_derived());
        assert_eq!(
            s.input.as_deref(),
            Some("SELECT id, title FROM v_task WHERE priority = 'high'")
        );
        assert_eq!(parse(GOOD).0[0].sync, SourceSync::Replace);
        assert_eq!(
            parse(&DERIVED.replace("runtime: starlark", "runtime: jaq")).0[0].runtime,
            SourceRuntime::Jaq
        );
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

    #[test]
    fn unknown_keys_are_errors() {
        let (s, e) = parse(&GOOD.replace("  runtime: exec", "  runtime: exec\n  netwrk: [x]"));
        assert!(s.is_empty());
        assert!(e[0].contains("netwrk"), "{e:?}");
    }
}
