//! Reviewing an extension by its **effects** (P6b.E1): what installing or
//! updating it would change — lenses' rendered text, models and their
//! contracts (and what reads them), collectors' and providers' grants, a
//! provider's commands and features, the instance config schema — rather
//! than only what it declares. Built from the two versions' loaded
//! extensions; **it never runs a collector or a provider** (consent
//! forbids running an unapproved version). See `.context/extensions.md`
//! → "Reviewing by effect".

use std::collections::{BTreeMap, BTreeSet};

use oxplow_db::models::{contract_change, extension_view, ModelSource};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::providers::ProviderSpec;
use oxplow_config::collectors::{CollectorRuntime, CollectorSpec};

/// How one thing differs between the installed version and the candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
    Unchanged,
}

fn change_of<T: PartialEq>(before: Option<&T>, after: Option<&T>) -> Change {
    match (before, after) {
        (None, Some(_)) => Change::Added,
        (Some(_), None) => Change::Removed,
        (Some(a), Some(b)) if a == b => Change::Unchanged,
        _ => Change::Changed,
    }
}

/// What a program may reach: what a person approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Grants {
    pub entry: String,
    pub runtime: CollectorRuntime,
    pub args: Vec<String>,
    pub hosts: Vec<String>,
    pub credentials: Vec<String>,
    pub env: Vec<String>,
}

/// A lens, by its rendered text before and after (P6b.E2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensEffect {
    pub id: String,
    pub change: Change,
    pub before: Option<String>,
    pub after: Option<String>,
    /// Why a side couldn't render (its query failed).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelEffect {
    pub view: String,
    pub change: Change,
    /// For a changed model, the parts that differ, in order: `query`,
    /// `columns`, `description`, `tests`, `version`, `deprecated`.
    pub changed: Vec<String>,
    pub before_columns: Vec<String>,
    pub after_columns: Vec<String>,
    /// The first difference in its contract (columns, types, docs).
    pub contract_change: Option<String>,
    /// Models that read it, which a contract change can break (P6b.E2).
    pub downstream: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorEffect {
    pub id: String,
    pub change: Change,
    pub before: Option<Grants>,
    pub after: Option<Grants>,
    /// The views it fills.
    pub entities: Vec<String>,
}

/// One of a provider's declared commands, or of its pinned MCP tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandChange {
    pub name: String,
    pub change: Change,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub before: Option<Value>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub after: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEffect {
    pub id: String,
    pub capability: String,
    pub change: Change,
    pub before: Option<Grants>,
    pub after: Option<Grants>,
    pub commands: Vec<CommandChange>,
    /// Behind the MCP adapter (P7.A6): each pinned tool of its server.
    pub tools: Vec<CommandChange>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub features_before: Option<Value>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub features_after: Option<Value>,
    /// For a changed provider, where its spec and declarations first
    /// differ — what a person reads when no grant, command or feature
    /// line shows the change.
    pub first_difference: Option<String>,
}

/// The instance config schema (`config:`), by property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEffect {
    #[specta(type = Option<oxplow_domain::Json>)]
    pub before: Option<Value>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub after: Option<Value>,
    pub changed_keys: Vec<String>,
    /// The first difference outside `properties` (`required`, …).
    pub other_change: Option<String>,
}

/// Everything installing or updating an extension would change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectReport {
    pub lenses: Vec<LensEffect>,
    pub models: Vec<ModelEffect>,
    pub collectors: Vec<CollectorEffect>,
    pub providers: Vec<ProviderEffect>,
    pub config: Option<ConfigEffect>,
}

/// Pair two lists by key: every key either side has, in order.
fn pair_by<'a, T, K: Ord + Clone>(
    before: &'a [T],
    after: &'a [T],
    key: impl Fn(&T) -> K,
) -> Vec<(K, Option<&'a T>, Option<&'a T>)> {
    let mut keys: BTreeMap<K, (Option<&T>, Option<&T>)> = BTreeMap::new();
    for b in before {
        keys.entry(key(b)).or_default().0 = Some(b);
    }
    for a in after {
        keys.entry(key(a)).or_default().1 = Some(a);
    }
    keys.into_iter().map(|(k, (b, a))| (k, b, a)).collect()
}

/// `extension`'s models before and after: added, removed, and a changed
/// one's first contract difference (a SQL-only change keeps the contract).
pub fn models_diff(
    extension: &str,
    before: &[ModelSource],
    after: &[ModelSource],
) -> Vec<ModelEffect> {
    let cols = |m: Option<&ModelSource>| -> Vec<String> {
        m.map(|m| m.decl.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    };
    pair_by(before, after, |m| m.decl.name.clone())
        .into_iter()
        .map(|(name, b, a)| {
            let changed: Vec<String> = match (b, a) {
                (Some(x), Some(y)) => [
                    ("query", x.sql != y.sql),
                    ("columns", x.decl.columns != y.decl.columns),
                    ("description", x.decl.description != y.decl.description),
                    ("tests", x.decl.tests != y.decl.tests),
                    ("version", x.decl.version != y.decl.version),
                    ("deprecated", x.decl.deprecated != y.decl.deprecated),
                ]
                .into_iter()
                .filter(|(_, differs)| *differs)
                .map(|(part, _)| part.to_string())
                .collect(),
                _ => Vec::new(),
            };
            let change = match (b, a) {
                (None, _) => Change::Added,
                (_, None) => Change::Removed,
                _ if changed.is_empty() => Change::Unchanged,
                _ => Change::Changed,
            };
            let contract_change = match (b, a) {
                (Some(x), Some(y)) if x.decl.columns != y.decl.columns => {
                    Some(contract_change(&x.decl.columns, &y.decl.columns))
                }
                _ => None,
            };
            ModelEffect {
                view: extension_view(extension, &name),
                change,
                changed,
                before_columns: cols(b),
                after_columns: cols(a),
                contract_change,
                downstream: Vec::new(),
            }
        })
        .collect()
}

fn collector_grants(s: &CollectorSpec) -> Grants {
    Grants {
        entry: s.entry.clone().unwrap_or_default(),
        runtime: s.runtime,
        args: Vec::new(),
        hosts: s.network.clone(),
        credentials: s.credentials.clone(),
        env: s.env.clone(),
    }
}

/// Collectors before and after, by id: their grants and the views they fill.
pub fn collectors_diff(before: &[CollectorSpec], after: &[CollectorSpec]) -> Vec<CollectorEffect> {
    pair_by(before, after, |s| s.id.clone())
        .into_iter()
        .map(|(id, b, a)| CollectorEffect {
            id,
            change: change_of(b, a),
            before: b.map(collector_grants),
            after: a.map(collector_grants),
            entities: a
                .or(b)
                .map(|s| s.entities.iter().map(|e| e.view.clone()).collect())
                .unwrap_or_default(),
        })
        .collect()
}

fn provider_grants(p: &ProviderSpec) -> Grants {
    let (entry, args) = p.program();
    Grants {
        entry,
        runtime: CollectorRuntime::Exec,
        args,
        hosts: p.network.clone(),
        credentials: p.credentials.clone(),
        env: p.env.clone(),
    }
}

/// A provider as declared: its spec and its checked-in declarations (`None`
/// when they can't be read).
pub use crate::providers::spec::DeclaredProvider;

fn features_of(p: &DeclaredProvider) -> Option<Value> {
    p.declarations.as_ref().and_then(|d| {
        d.capabilities
            .iter()
            .find(|c| c.capability == p.spec.capability)
            .map(|c| c.features.clone())
    })
}

/// Named things before and after (a provider's commands, its tools), by
/// name: each added, removed, changed or not.
fn named_changes(before: Vec<(String, Value)>, after: Vec<(String, Value)>) -> Vec<CommandChange> {
    let names: BTreeSet<&String> = before.iter().chain(after.iter()).map(|(n, _)| n).collect();
    names
        .into_iter()
        .map(|name| {
            let find = |list: &[(String, Value)]| {
                list.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
            };
            let (b, a) = (find(&before), find(&after));
            CommandChange {
                name: name.clone(),
                change: change_of(b.as_ref(), a.as_ref()),
                before: b,
                after: a,
            }
        })
        .collect()
}

/// Providers before and after, by id: grants, each declared command, and
/// the capability's features.
pub fn providers_diff(
    before: &[DeclaredProvider],
    after: &[DeclaredProvider],
) -> Vec<ProviderEffect> {
    pair_by(before, after, |p| p.spec.id.clone())
        .into_iter()
        .map(|(id, b, a)| {
            let commands = |p: Option<&DeclaredProvider>| -> Vec<(String, Value)> {
                p.and_then(|p| p.declarations.as_ref())
                    .map(|d| {
                        d.commands
                            .iter()
                            .map(|c| {
                                (
                                    c.name.clone(),
                                    serde_json::to_value(c).unwrap_or(Value::Null),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let tools = |p: Option<&DeclaredProvider>| -> Vec<(String, Value)> {
                p.map(|p| {
                    p.tools
                        .iter()
                        .map(|t| {
                            (
                                t["name"].as_str().unwrap_or_default().to_string(),
                                t.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
            };
            let declared = |p: &DeclaredProvider| {
                json!({
                    "spec": serde_json::to_value(&p.spec).unwrap_or(Value::Null),
                    "declarations": p.declarations.as_ref()
                        .map(|d| serde_json::to_value(d).unwrap_or(Value::Null)),
                    "tools": p.tools,
                })
            };
            let (db, da) = (b.map(declared), a.map(declared));
            let first_difference = match (&db, &da) {
                (Some(x), Some(y)) => described_difference(x, y),
                _ => None,
            };
            ProviderEffect {
                id,
                capability: a
                    .or(b)
                    .map(|p| p.spec.capability.clone())
                    .unwrap_or_default(),
                change: change_of(b, a),
                first_difference,
                before: b.map(|p| provider_grants(&p.spec)),
                after: a.map(|p| provider_grants(&p.spec)),
                commands: named_changes(commands(b), commands(a)),
                tools: named_changes(tools(b), tools(a)),
                features_before: b.and_then(features_of),
                features_after: a.and_then(features_of),
            }
        })
        .collect()
}

/// The instance config schema before and after, by property: which keys
/// were added, removed or changed. `None` when neither version has one.
pub fn config_diff(before: Option<&Value>, after: Option<&Value>) -> Option<ConfigEffect> {
    if before.is_none() && after.is_none() {
        return None;
    }
    let props = |v: Option<&Value>| -> BTreeMap<String, Value> {
        v.and_then(|v| v.get("properties"))
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    };
    let (pb, pa) = (props(before), props(after));
    let rest = |v: Option<&Value>| -> Value {
        let mut v = v.cloned().unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.remove("properties");
        }
        v
    };
    let other_change = described_difference(&rest(before), &rest(after));
    let changed_keys = pb
        .keys()
        .chain(pa.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| pb.get(*k) != pa.get(*k))
        .cloned()
        .collect();
    Some(ConfigEffect {
        before: before.cloned(),
        after: after.cloned(),
        changed_keys,
        other_change,
    })
}

/// Where `b` first differs from `a`: its JSON pointer and the two values
/// there (objects by key, equal-length arrays by index; anything else is
/// compared whole). `None` when they're equal.
pub fn json_difference(a: &Value, b: &Value) -> Option<(String, Value, Value)> {
    fn walk(path: &str, a: &Value, b: &Value) -> Option<(String, Value, Value)> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
                keys.into_iter().find_map(|k| {
                    walk(
                        &format!("{path}/{k}"),
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    )
                })
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| walk(&format!("{path}/{i}"), p, q)),
            _ if a == b => None,
            _ => Some((
                if path.is_empty() {
                    "/".into()
                } else {
                    path.to_string()
                },
                a.clone(),
                b.clone(),
            )),
        }
    }
    walk("", a, b)
}

/// [`json_difference`] for a person: "`/path` was X, now Y".
fn described_difference(before: &Value, after: &Value) -> Option<String> {
    json_difference(before, after).map(|(path, b, a)| format!("`{path}` was {b}, now {a}"))
}

/// A loaded extension and how to read its files (the installed one from
/// the project, a candidate from its clone).
pub struct Version<'a> {
    pub extension: &'a crate::extensions::Extension,
    pub read: &'a (dyn Fn(&str) -> Option<String> + Sync),
    /// Its lenses, already run once (`extensions::run_lenses`).
    pub lenses: &'a crate::extensions::LensRuns,
}

/// A version's instance config schema (its manifest's `config:`), read
/// from its files — only a review needs it, so the loaded `Extension`
/// doesn't carry it.
fn config_schema(v: &Version<'_>) -> Option<Value> {
    let manifest: serde_yaml::Value = serde_yaml::from_str(&(v.read)("extension.yaml")?).ok()?;
    serde_json::to_value(manifest.get("config")?).ok()
}

/// Each lens's rendered text, by slug: what `lens_text` gives an agent,
/// from the run the version's check already made; a grid renders its
/// children empty. A query that failed is the lens's error.
fn lens_texts(runs: &crate::extensions::LensRuns) -> BTreeMap<String, Result<String, String>> {
    runs.iter()
        .map(|(slug, run)| {
            let text = run
                .as_ref()
                .map(|run| crate::lens_text::render(run, &Default::default()))
                .map_err(Clone::clone);
            (slug.clone(), text)
        })
        .collect()
}

/// The views that read `view` directly (`v_model_lineage`).
pub async fn downstream_of(layer: &crate::sql_gateway::SqlGateway, view: &str) -> Vec<String> {
    layer
        .query_sql(
            "SELECT DISTINCT view FROM v_model_lineage WHERE input = ?1 AND kind = 'ref' ORDER BY view",
            vec![oxplow_db::SqlCell::Text(view.to_string())],
            None,
        )
        .await
        .map(|r| {
            r.rows
                .into_iter()
                .filter_map(|row| match row.into_iter().next() {
                    Some(oxplow_db::SqlCell::Text(v)) => Some(v),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What installing `after` — over `before`, when one is installed — would
/// change. Lenses are rendered (their queries run read-only, against the
/// models as published now); collectors and providers are only compared.
pub async fn effects(
    layer: &crate::sql_gateway::SqlGateway,
    before: Option<Version<'_>>,
    after: Version<'_>,
) -> EffectReport {
    let name = &after.extension.name;
    let texts_before = before
        .as_ref()
        .map(|b| lens_texts(b.lenses))
        .unwrap_or_default();
    let texts_after = lens_texts(after.lenses);
    let slugs: BTreeSet<&String> = texts_before.keys().chain(texts_after.keys()).collect();
    let lenses = slugs
        .into_iter()
        .map(|slug| {
            let (b, a) = (texts_before.get(slug), texts_after.get(slug));
            let ok = |r: Option<&Result<String, String>>| r.and_then(|r| r.as_ref().ok().cloned());
            let error = a.or(b).and_then(|r| r.as_ref().err().cloned());
            LensEffect {
                id: format!("{name}/{slug}"),
                change: match (b, a) {
                    (None, _) => Change::Added,
                    (_, None) => Change::Removed,
                    (Some(Ok(x)), Some(Ok(y))) if x == y => Change::Unchanged,
                    _ => Change::Changed,
                },
                before: ok(b),
                after: ok(a),
                error,
            }
        })
        .collect();
    let empty: Vec<ModelSource> = Vec::new();
    let mut models = models_diff(
        name,
        before.as_ref().map_or(&empty, |b| &b.extension.models),
        &after.extension.models,
    );
    // Readers outside this extension: its own views, by exact name (a
    // prefix would also drop another extension's `v_<name>_<…>` views).
    let own: BTreeSet<String> = models.iter().map(|m| m.view.clone()).collect();
    for m in &mut models {
        m.downstream = downstream_of(layer, &m.view)
            .await
            .into_iter()
            .filter(|v| !own.contains(v))
            .collect();
    }
    let no_collectors: Vec<CollectorSpec> = Vec::new();
    let collectors = collectors_diff(
        before
            .as_ref()
            .map_or(&no_collectors, |b| &b.extension.collectors),
        &after.extension.collectors,
    );
    let declared = |v: &Version<'_>| -> Vec<DeclaredProvider> {
        v.extension
            .providers
            .iter()
            .map(|p| DeclaredProvider::read(p, v.read))
            .collect()
    };
    let providers = providers_diff(
        &before.as_ref().map(declared).unwrap_or_default(),
        &declared(&after),
    );
    let config = config_diff(
        before.as_ref().and_then(config_schema).as_ref(),
        config_schema(&after).as_ref(),
    );
    EffectReport {
        lenses,
        models,
        collectors,
        providers,
        config,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::models::{ColumnDecl, ModelDecl};
    use serde_json::json;

    fn model(name: &str, columns: &[(&str, &str)], sql: &str) -> ModelSource {
        ModelSource {
            decl: ModelDecl {
                name: name.into(),
                version: 1,
                description: "d".into(),
                columns: columns
                    .iter()
                    .map(|(n, t)| ColumnDecl {
                        name: (*n).into(),
                        sql_type: (*t).into(),
                        doc: "d".into(),
                    })
                    .collect(),
                key: vec![],
                tests: Vec::new(),
                deprecated: Vec::new(),
                materialize: None,
            },
            file: format!("models/{name}.sql"),
            sql: sql.into(),
            twin: None,
        }
    }

    #[test]
    fn models_diff_names_added_removed_and_contract_changes() {
        let before = [
            model("kept", &[("a", "TEXT")], "SELECT 1"),
            model("sql_only", &[("a", "TEXT")], "SELECT 1"),
            model("retyped", &[("a", "TEXT")], "SELECT 1"),
            model("gone", &[("a", "TEXT")], "SELECT 1"),
        ];
        let after = [
            model("kept", &[("a", "TEXT")], "SELECT 1"),
            model("sql_only", &[("a", "TEXT")], "SELECT 2"),
            model("retyped", &[("a", "INTEGER")], "SELECT 1"),
            model("new", &[("b", "TEXT")], "SELECT 1"),
        ];
        let out: BTreeMap<String, ModelEffect> = models_diff("my-ext", &before, &after)
            .into_iter()
            .map(|m| (m.view.clone(), m))
            .collect();
        assert_eq!(out["v_my_ext_kept"].change, Change::Unchanged);
        assert_eq!(out["v_my_ext_sql_only"].change, Change::Changed);
        assert_eq!(out["v_my_ext_sql_only"].contract_change, None, "SQL only");
        assert_eq!(out["v_my_ext_retyped"].change, Change::Changed);
        assert_eq!(
            out["v_my_ext_retyped"].contract_change.as_deref(),
            Some("column `a` changed type from `TEXT` to `INTEGER`")
        );
        assert_eq!(out["v_my_ext_gone"].change, Change::Removed);
        assert_eq!(out["v_my_ext_gone"].before_columns, vec!["a"]);
        assert_eq!(out["v_my_ext_new"].change, Change::Added);
        assert_eq!(out["v_my_ext_new"].after_columns, vec!["b"]);
        assert_eq!(out["v_my_ext_sql_only"].changed, vec!["query"]);
        assert_eq!(out["v_my_ext_retyped"].changed, vec!["columns"]);
        assert!(out["v_my_ext_kept"].changed.is_empty());
    }

    /// A change outside the query and the columns still says what moved.
    #[test]
    fn models_diff_names_each_part_that_changed() {
        let base = model("m", &[("a", "TEXT")], "SELECT 1");
        let mut tests = base.clone();
        tests.decl.tests = serde_json::from_value(json!([{ "not_null": "a" }])).unwrap();
        let mut described = base.clone();
        described.decl.description = "new words".into();
        described.decl.version = 2;
        for (after, want) in [
            (tests, vec!["tests"]),
            (described, vec!["description", "version"]),
        ] {
            let out = models_diff("x", std::slice::from_ref(&base), &[after]);
            assert_eq!(out[0].change, Change::Changed);
            assert_eq!(out[0].contract_change, None);
            assert_eq!(out[0].changed, want);
        }
    }

    fn collector(id: &str, hosts: &[&str]) -> CollectorSpec {
        serde_json::from_value(json!({
            "id": id, "doc": "", "runtime": "exec", "entry": "sync.sh", "provider": null,
            "trigger": { "kind": "manual" }, "after": [], "input": null, "report": null,
            "sync": "replace", "env": [], "network": hosts, "credentials": ["token"],
            "entities": [], "facts": []
        }))
        .unwrap()
    }

    fn provider(id: &str, network: &[&str], comments: bool, commands: &[&str]) -> DeclaredProvider {
        let spec: ProviderSpec = serde_json::from_value(json!({
            "id": id, "capability": "work_items", "entry": "bin/p", "network": network,
            "declarations": "provider.json"
        }))
        .unwrap();
        let mut declared = oxplow_provider_fake::declarations();
        declared.capabilities[0].features = json!({ "comments": comments });
        declared
            .commands
            .retain(|c| commands.contains(&c.name.as_str()));
        DeclaredProvider {
            spec,
            declarations: Some(declared),
            tools: Vec::new(),
        }
    }

    /// P7.A6: an adapter provider's approval shows its server and each
    /// pinned tool added, removed or changed.
    #[test]
    fn an_adapter_providers_effects_show_its_server_and_pinned_tools() {
        let adapter = |tools: Value| {
            let spec: ProviderSpec = serde_json::from_value(json!({
                "id": "notes", "capability": "work_items", "declarations": "provider.json",
                "adapter": { "mcp": { "command": ["bin/server", "--stdio"] },
                             "mapping": "mcp/x.star", "tools": "mcp/tools.json" }
            }))
            .unwrap();
            DeclaredProvider {
                spec,
                declarations: Some(oxplow_provider_fake::declarations()),
                tools: tools.as_array().unwrap().clone(),
            }
        };
        let tool = |name: &str, description: &str| json!({ "name": name, "description": description, "inputSchema": { "type": "object" } });
        let effects = providers_diff(
            &[adapter(json!([
                tool("list_items", "List."),
                tool("drop_all", "Drop.")
            ]))],
            &[adapter(json!([
                tool("list_items", "List them all."),
                tool("create_item", "Create.")
            ]))],
        );
        let p = &effects[0];
        assert_eq!(p.change, Change::Changed);
        let grants = p.after.as_ref().unwrap();
        assert_eq!(
            (grants.entry.as_str(), grants.args.clone()),
            ("bin/server", vec!["--stdio".to_string()])
        );
        let tools: Vec<(&str, Change)> = p
            .tools
            .iter()
            .map(|t| (t.name.as_str(), t.change))
            .collect();
        assert_eq!(
            tools,
            [
                ("create_item", Change::Added),
                ("drop_all", Change::Removed),
                ("list_items", Change::Changed)
            ]
        );
    }

    #[test]
    fn collectors_and_providers_diff_their_grants() {
        let collectors = collectors_diff(
            &[collector("prs", &["api.github.com"]), collector("old", &[])],
            &[
                collector("prs", &["api.github.com", "*.githubusercontent.com"]),
                collector("new", &[]),
            ],
        );
        let by: BTreeMap<&str, &CollectorEffect> =
            collectors.iter().map(|c| (c.id.as_str(), c)).collect();
        assert_eq!(by["prs"].change, Change::Changed);
        assert_eq!(
            by["prs"].before.as_ref().unwrap().hosts,
            vec!["api.github.com"]
        );
        assert_eq!(by["prs"].after.as_ref().unwrap().hosts.len(), 2);
        assert_eq!(by["prs"].after.as_ref().unwrap().credentials, vec!["token"]);
        assert_eq!(by["old"].change, Change::Removed);
        assert_eq!(by["new"].change, Change::Added);

        let providers = providers_diff(
            &[provider(
                "fake",
                &[],
                false,
                &["create", "update", "transition"],
            )],
            &[provider(
                "fake",
                &["api.example.com"],
                true,
                &["create", "update", "transition", "comment"],
            )],
        );
        let p = &providers[0];
        assert_eq!(p.change, Change::Changed);
        assert_eq!(p.after.as_ref().unwrap().hosts, vec!["api.example.com"]);
        assert_eq!(p.features_before, Some(json!({ "comments": false })));
        assert_eq!(p.features_after, Some(json!({ "comments": true })));
        let comment = p.commands.iter().find(|c| c.name == "comment").unwrap();
        assert_eq!(comment.change, Change::Added);
        assert!(p
            .commands
            .iter()
            .filter(|c| c.name != "comment")
            .all(|c| c.change == Change::Unchanged));
        let first = providers_diff(&[], &[provider("fake", &[], false, &["create"])]);
        assert_eq!(
            first[0].change,
            Change::Added,
            "a first approval: everything is new"
        );
    }

    /// A provider whose change no grant, command or feature shows still
    /// says where its declarations first differ.
    #[test]
    fn a_provider_change_names_its_first_difference() {
        let before = provider("fake", &[], true, &["create"]);
        let mut after = before.clone();
        after.declarations.as_mut().unwrap().capabilities[0].capability = "work_items_v2".into();
        let effect = providers_diff(std::slice::from_ref(&before), &[after]).remove(0);
        assert_eq!(effect.change, Change::Changed);
        assert!(effect
            .commands
            .iter()
            .all(|c| c.change == Change::Unchanged));
        let first = effect.first_difference.unwrap();
        assert!(
            first.contains("/declarations/capabilities/0/capability"),
            "{first}"
        );
        assert!(first.contains("work_items_v2"), "{first}");
        let same =
            providers_diff(std::slice::from_ref(&before), std::slice::from_ref(&before)).remove(0);
        assert_eq!(same.first_difference, None);
    }

    /// A schema change outside `properties` (what's required, say) is a
    /// change too.
    #[test]
    fn config_diff_reports_a_change_beyond_properties() {
        let before = json!({ "properties": { "team": { "type": "string" } } });
        let after = json!({ "properties": { "team": { "type": "string" } }, "required": ["team"] });
        let c = config_diff(Some(&before), Some(&after)).unwrap();
        assert!(c.changed_keys.is_empty());
        let other = c.other_change.unwrap();
        assert!(other.contains("/required"), "{other}");
        assert_eq!(
            config_diff(Some(&before), Some(&before))
                .unwrap()
                .other_change,
            None
        );
    }

    #[test]
    fn config_diff_lists_changed_keys() {
        assert_eq!(config_diff(None, None), None);
        let before =
            json!({ "properties": { "team": { "type": "string" }, "old": { "type": "string" } } });
        let after = json!({ "properties": { "team": { "type": "string", "enum": ["a"] }, "new": { "type": "number" } } });
        let c = config_diff(Some(&before), Some(&after)).unwrap();
        assert_eq!(c.changed_keys, vec!["new", "old", "team"]);
        let first = config_diff(None, Some(&after)).unwrap();
        assert_eq!(first.changed_keys, vec!["new", "team"]);
    }

    fn write(root: &std::path::Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Extension `x` with `lenses` (slug, body), loaded from its own folder.
    fn version(lenses: &[(&str, &str)]) -> (tempfile::TempDir, crate::extensions::Extension) {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: p\n",
        );
        for (slug, body) in lenses {
            write(
                d.path(),
                &format!("oxplow/extensions/x/lenses/{slug}.yaml"),
                body,
            );
        }
        let ext = crate::extensions::load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        (d, ext)
    }

    #[tokio::test]
    async fn a_changed_lens_renders_before_and_after_text() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let (_b, before) = version(&[
            ("same", "title: Same\nquery: SELECT 1 AS n\nviz: number\n"),
            ("count", "title: Count\nquery: SELECT 1 AS n\nviz: number\n"),
            ("old", "title: Old\nquery: SELECT 1 AS n\n"),
        ]);
        let (_a, after) = version(&[
            ("same", "title: Same\nquery: SELECT 1 AS n\nviz: number\n"),
            ("count", "title: Count\nquery: SELECT 2 AS n\nviz: number\n"),
            (
                "broken",
                "title: Broken\nquery: SELECT n FROM v_no_such_model\n",
            ),
        ]);
        let none = |_: &str| None;
        let (runs_before, runs_after) = (
            crate::extensions::run_lenses(&layer, &before).await,
            crate::extensions::run_lenses(&layer, &after).await,
        );
        let report = effects(
            &layer,
            Some(Version {
                extension: &before,
                read: &none,
                lenses: &runs_before,
            }),
            Version {
                extension: &after,
                read: &none,
                lenses: &runs_after,
            },
        )
        .await;
        let by: BTreeMap<&str, &LensEffect> =
            report.lenses.iter().map(|l| (l.id.as_str(), l)).collect();
        assert_eq!(by["x/same"].change, Change::Unchanged);
        assert_eq!(by["x/count"].change, Change::Changed);
        assert_eq!(by["x/count"].before.as_deref(), Some("1"));
        assert_eq!(by["x/count"].after.as_deref(), Some("2"));
        assert_eq!(by["x/old"].change, Change::Removed);
        assert_eq!(by["x/broken"].change, Change::Added);
        assert!(
            by["x/broken"]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("v_no_such_model")),
            "{:?}",
            by["x/broken"]
        );
        // A first install: everything is added.
        let fresh = effects(
            &layer,
            None,
            Version {
                extension: &after,
                read: &none,
                lenses: &runs_after,
            },
        )
        .await;
        assert!(fresh.lenses.iter().all(|l| l.change == Change::Added));
    }

    /// A candidate that declares a provider: its grants, commands and
    /// features come from the files it brings (read through the version),
    /// and none of it runs.
    #[tokio::test]
    async fn a_candidate_declaring_a_provider_shows_what_it_would_add() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: p\nconfig:\n  type: object\n  properties:\n    team: { type: string }\nproviders:\n  - id: fake\n    capability: work_items\n    entry: bin/p\n    network: [api.example.com]\n    declarations: provider.json\n",
        );
        write(d.path(), "oxplow/extensions/x/bin/p", "#!/bin/sh\nexit 1\n");
        let declared = serde_json::to_string(&oxplow_provider_fake::declarations()).unwrap();
        write(d.path(), "oxplow/extensions/x/provider.json", &declared);
        let ext = crate::extensions::load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let root = d.path().join("oxplow/extensions/x");
        let read = move |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
        let runs = crate::extensions::LensRuns::new();
        let report = effects(
            &layer,
            None,
            Version {
                extension: &ext,
                read: &read,
                lenses: &runs,
            },
        )
        .await;
        let p = &report.providers[0];
        assert_eq!((p.id.as_str(), p.change), ("fake", Change::Added));
        assert_eq!(p.after.as_ref().unwrap().hosts, vec!["api.example.com"]);
        assert!(!p.commands.is_empty());
        assert!(p.commands.iter().all(|c| c.change == Change::Added));
        assert!(p.features_after.is_some());
        // Its config schema is read from its manifest.
        assert_eq!(report.config.unwrap().changed_keys, vec!["team"]);
    }

    #[tokio::test]
    async fn downstream_models_come_from_the_lineage() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let readers = downstream_of(&fx.svc.sql, "v_knowledge_ref").await;
        assert!(
            readers.contains(&"v_knowledge_page".to_string()),
            "{readers:?}"
        );
        assert!(downstream_of(&fx.svc.sql, "v_nothing").await.is_empty());
    }
}
