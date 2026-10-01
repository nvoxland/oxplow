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
use oxplow_provider_protocol::model::InitializeResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::extension_sources::SourceSpec;
use crate::providers::ProviderSpec;

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
    pub runtime: String,
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

/// One of a provider's declared commands.
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
    #[specta(type = Option<oxplow_domain::Json>)]
    pub features_before: Option<Value>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub features_after: Option<Value>,
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
            let same = |x: &ModelSource, y: &ModelSource| x.decl == y.decl && x.sql == y.sql;
            let change = match (b, a) {
                (None, _) => Change::Added,
                (_, None) => Change::Removed,
                (Some(x), Some(y)) if same(x, y) => Change::Unchanged,
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
                before_columns: cols(b),
                after_columns: cols(a),
                contract_change,
                downstream: Vec::new(),
            }
        })
        .collect()
}

fn collector_grants(s: &SourceSpec) -> Grants {
    Grants {
        entry: s.entry.clone(),
        runtime: serde_json::to_value(s.runtime)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        args: Vec::new(),
        hosts: s.network.clone(),
        credentials: s.credentials.clone(),
        env: s.env.clone(),
    }
}

/// Collectors before and after, by id: their grants and the views they fill.
pub fn collectors_diff(before: &[SourceSpec], after: &[SourceSpec]) -> Vec<CollectorEffect> {
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
    Grants {
        entry: p.entry.clone(),
        runtime: "exec".into(),
        args: p.args.clone(),
        hosts: p.network.clone(),
        credentials: p.credentials.clone(),
        env: p.env.clone(),
    }
}

/// A provider as declared: its spec and its checked-in declarations (`None`
/// when they can't be read).
pub type DeclaredProvider = (ProviderSpec, Option<InitializeResult>);

fn features_of(p: &DeclaredProvider) -> Option<Value> {
    p.1.as_ref().and_then(|d| {
        d.capabilities
            .iter()
            .find(|c| c.capability == p.0.capability)
            .map(|c| c.features.clone())
    })
}

/// Providers before and after, by id: grants, each declared command, and
/// the capability's features.
pub fn providers_diff(
    before: &[DeclaredProvider],
    after: &[DeclaredProvider],
) -> Vec<ProviderEffect> {
    pair_by(before, after, |p| p.0.id.clone())
        .into_iter()
        .map(|(id, b, a)| {
            let commands = |p: Option<&DeclaredProvider>| -> Vec<(String, Value)> {
                p.and_then(|p| p.1.as_ref())
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
            let (cb, ca) = (commands(b), commands(a));
            let names: BTreeSet<&String> = cb.iter().chain(ca.iter()).map(|(n, _)| n).collect();
            let commands = names
                .into_iter()
                .map(|name| {
                    let find = |list: &[(String, Value)]| {
                        list.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
                    };
                    let (before, after) = (find(&cb), find(&ca));
                    CommandChange {
                        name: name.clone(),
                        change: change_of(before.as_ref(), after.as_ref()),
                        before,
                        after,
                    }
                })
                .collect();
            let declared = |p: &DeclaredProvider| {
                (
                    serde_json::to_value(&p.0).unwrap_or(Value::Null),
                    p.1.as_ref()
                        .map(|d| serde_json::to_value(d).unwrap_or(Value::Null)),
                )
            };
            ProviderEffect {
                id,
                capability: a.or(b).map(|p| p.0.capability.clone()).unwrap_or_default(),
                change: change_of(b.map(declared).as_ref(), a.map(declared).as_ref()),
                before: b.map(|p| provider_grants(&p.0)),
                after: a.map(|p| provider_grants(&p.0)),
                commands,
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
    })
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
                tests: Vec::new(),
                deprecated: Vec::new(),
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
    }

    fn source(id: &str, hosts: &[&str]) -> SourceSpec {
        serde_json::from_value(json!({
            "id": id, "doc": "", "runtime": "exec", "entry": "sync.sh", "input": null,
            "sync": "replace", "schedule": { "kind": "manual" }, "env": [], "network": hosts,
            "credentials": ["token"], "entities": []
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
        (spec, Some(declared))
    }

    #[test]
    fn collectors_and_providers_diff_their_grants() {
        let collectors = collectors_diff(
            &[source("prs", &["api.github.com"]), source("old", &[])],
            &[
                source("prs", &["api.github.com", "*.githubusercontent.com"]),
                source("new", &[]),
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
}
