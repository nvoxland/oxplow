//! Reviewing an extension by its **effects** (P6b.E1): what installing or
//! updating it would change — lenses' rendered text, models and their
//! contracts (and what reads them), collectors' and providers' grants, a
//! provider's commands and features, the instance config schema — rather
//! than only what it declares, plus models' rows before and after
//! (P8.C3), derived collectors' and effects' dry runs on the same inputs
//! (P8.C4, D12) — each version on its own models' overlay. It never
//! runs a program, a provider or an exec collector (consent forbids
//! running an unapproved version), and stores nothing. See
//! `.context/extensions.md` → "Reviewing by effect".

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
    /// What its rows would become (P8.C3), each side read through its own
    /// models; `None` for an unchanged model.
    pub rows: Option<RowDiff>,
}

/// The most rows a side's diff reads; past it, counts only. It is the
/// read gateway's own cap: a review reads through the same read-only path
/// as everything else, and a larger number would be one it never returns
/// (tsk779).
pub const ROW_DIFF_LIMIT: usize = oxplow_db::semantic_layer::MAX_ROW_LIMIT;
/// The most sample changes a keyed diff keeps.
pub const ROW_DIFF_SAMPLES: usize = 20;

/// A model's rows before and after (P8.C3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RowDiff {
    /// Rows on each side (`None`: it isn't on that side).
    pub before: Option<i64>,
    pub after: Option<i64>,
    /// Row by row, when both sides declare the same key.
    pub keyed: Option<KeyedDiff>,
    /// Why the diff is only counts, or that a side's query failed.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyedDiff {
    pub key: Vec<String>,
    pub added: i64,
    pub removed: i64,
    pub changed: i64,
    /// Up to [`ROW_DIFF_SAMPLES`] of them, in key order.
    pub samples: Vec<RowSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RowSample {
    pub change: Change,
    /// The row's key, column → value.
    #[specta(type = oxplow_domain::Json)]
    pub key: Value,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub before: Option<Value>,
    #[specta(type = Option<oxplow_domain::Json>)]
    pub after: Option<Value>,
}

/// One side's rows of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// It has more than [`ROW_DIFF_LIMIT`].
    pub truncated: bool,
}

impl Rows {
    fn object(&self, row: &[Value]) -> serde_json::Map<String, Value> {
        self.columns
            .iter()
            .cloned()
            .zip(row.iter().cloned())
            .collect()
    }
}

/// Diff a model's rows: each side `(rows, its key)`. With the same
/// non-empty key on both and neither past the limit, a merge-join by key;
/// otherwise counts with a note saying why.
pub fn row_diff(before: Option<(&Rows, &[String])>, after: Option<(&Rows, &[String])>) -> RowDiff {
    let count = |s: Option<(&Rows, &[String])>| s.map(|(r, _)| r.rows.len() as i64);
    let mut diff = RowDiff {
        before: count(before),
        after: count(after),
        keyed: None,
        note: None,
    };
    let (Some((b, bk)), Some((a, ak))) = (before, after) else {
        return diff;
    };
    if b.truncated || a.truncated {
        diff.note = Some(format!(
            "over {ROW_DIFF_LIMIT} rows: counts only (at least that many)"
        ));
        return diff;
    }
    if bk.is_empty() || bk != ak {
        diff.note = Some(
            "no key the two versions share: counts only — declare the same `key:` on both to see rows".into(),
        );
        return diff;
    }
    let (old, new) = match (keyed(b, bk, "before"), keyed(a, bk, "after")) {
        (Ok(old), Ok(new)) => (old, new),
        (Err(why), _) | (_, Err(why)) => {
            diff.note = Some(format!("{why}: counts only"));
            return diff;
        }
    };
    let mut out = KeyedDiff {
        key: bk.to_vec(),
        added: 0,
        removed: 0,
        changed: 0,
        samples: Vec::new(),
    };
    let (mut old, mut new) = (old.into_iter().peekable(), new.into_iter().peekable());
    loop {
        let order = match (old.peek(), new.peek()) {
            (None, None) => break,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => cmp_key(&x.parts, &y.parts),
        };
        let (change, row) = match order {
            std::cmp::Ordering::Less => {
                let x = old.next().expect("peeked");
                out.removed += 1;
                (Change::Removed, (x.key, Some(x.row), None))
            }
            std::cmp::Ordering::Greater => {
                let y = new.next().expect("peeked");
                out.added += 1;
                (Change::Added, (y.key, None, Some(y.row)))
            }
            std::cmp::Ordering::Equal => {
                let (x, y) = (old.next().expect("peeked"), new.next().expect("peeked"));
                if x.row == y.row {
                    continue;
                }
                out.changed += 1;
                (Change::Changed, (x.key, Some(x.row), Some(y.row)))
            }
        };
        if out.samples.len() < ROW_DIFF_SAMPLES {
            let (key, before, after) = row;
            out.samples.push(RowSample {
                change,
                key,
                before,
                after,
            });
        }
    }
    diff.keyed = Some(out);
    diff
}

/// A row by its key: the key's parts (for order), the key as an object,
/// the row as an object.
struct KeyedRow {
    parts: Vec<Value>,
    key: Value,
    row: Value,
}

/// `rows` in key order — or why they can't be joined on `key`: a NULL
/// part, or a key two rows share (one row would stand for several).
fn keyed(rows: &Rows, key: &[String], side: &str) -> Result<Vec<KeyedRow>, String> {
    let mut out = Vec::with_capacity(rows.rows.len());
    for row in &rows.rows {
        let object = rows.object(row);
        let parts: Vec<Value> = key
            .iter()
            .map(|k| object.get(k).cloned().unwrap_or(Value::Null))
            .collect();
        if parts.iter().any(Value::is_null) {
            return Err(format!("a NULL in key `{}` {side}", key.join(", ")));
        }
        let key_object = Value::Object(key.iter().cloned().zip(parts.iter().cloned()).collect());
        out.push(KeyedRow {
            parts,
            key: key_object,
            row: Value::Object(object),
        });
    }
    out.sort_by(|x, y| cmp_key(&x.parts, &y.parts));
    if let Some(w) = out
        .windows(2)
        .find(|w| cmp_key(&w[0].parts, &w[1].parts).is_eq())
    {
        return Err(format!("key {} repeats {side}", w[0].key));
    }
    Ok(out)
}

/// Keys in SQLite's order: numbers (by value), then text, then the rest
/// by their JSON text.
fn cmp_key(a: &[Value], b: &[Value]) -> std::cmp::Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Number(_) | Value::Bool(_) => 0,
            Value::String(_) => 1,
            _ => 2,
        }
    }
    fn number(v: &Value) -> Option<f64> {
        match v {
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            Value::Number(n) => n.as_f64(),
            _ => None,
        }
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            rank(x).cmp(&rank(y)).then_with(|| match (x, y) {
                (Value::Number(m), Value::Number(n)) if m.is_i64() && n.is_i64() => {
                    m.as_i64().cmp(&n.as_i64())
                }
                (Value::String(m), Value::String(n)) => m.cmp(n),
                _ => match (number(x), number(y)) {
                    (Some(m), Some(n)) => m.total_cmp(&n),
                    _ => x.to_string().cmp(&y.to_string()),
                },
            })
        })
        .find(|o| o.is_ne())
        .unwrap_or_else(|| a.len().cmp(&b.len()))
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
    /// What each version makes of the same inputs (P8.C4) — its fixtures
    /// and the latest events it'd run on — for a derived collector whose
    /// script or declaration changed; storing nothing, asking no model.
    pub outputs: Vec<CollectorOutput>,
    /// Why it wasn't run: it runs a program or reads a provider, which a
    /// review never does, approved or not.
    pub not_run: Option<String>,
    /// Its entry's text changed though its declaration didn't (tsk783).
    pub script_changed: bool,
}

/// An effect before and after: when it reacts, and what it composes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectEffect {
    pub id: String,
    pub change: Change,
    pub before: Option<EffectTrigger>,
    pub after: Option<EffectTrigger>,
    /// What each version composes on the same inputs — its fixtures and
    /// the latest events it'd react to — when its script or declaration
    /// changed; running nothing.
    pub outputs: Vec<EffectOutput>,
}

/// When an effect reacts, and what it reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectTrigger {
    pub on: Vec<String>,
    /// Its `where`.
    pub filter: BTreeMap<String, String>,
    pub input: Option<String>,
}

/// One input, composed by each version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectOutput {
    /// `fixture <name>` or `event #<seq>`.
    pub input: String,
    pub change: Change,
    pub before: Option<Composes>,
    pub after: Option<Composes>,
}

/// What one version's effect makes of an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Composes {
    /// The commands it runs, by name.
    pub commands: Vec<String>,
    /// Why it skips.
    pub skip: Option<String>,
    /// Why it fails.
    pub error: Option<String>,
}

fn effect_trigger(d: &crate::effects::EffectDecl) -> EffectTrigger {
    EffectTrigger {
        on: d.on.clone(),
        filter: d.filter.clone(),
        input: d.input.clone(),
    }
}

/// Effects before and after, by id: their triggers.
pub fn effects_diff(
    before: &[crate::effects::EffectDecl],
    after: &[crate::effects::EffectDecl],
) -> Vec<EffectEffect> {
    pair_by(before, after, |d| d.id.clone())
        .into_iter()
        .map(|(id, b, a)| {
            let (tb, ta) = (b.map(effect_trigger), a.map(effect_trigger));
            EffectEffect {
                id,
                change: match change_of(tb.as_ref(), ta.as_ref()) {
                    Change::Unchanged if b.map(|d| &d.script) != a.map(|d| &d.script) => {
                        Change::Changed
                    }
                    other => other,
                },
                before: tb,
                after: ta,
                outputs: Vec::new(),
            }
        })
        .collect()
}

/// The fixtures of `v`'s intent examples that run effect `id`: the
/// example's name, its event and its `rows`.
fn effect_fixtures(v: &Version<'_>, id: &str) -> Vec<(String, Value, Option<Vec<Value>>)> {
    let examples = v
        .extension
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    examples
        .into_iter()
        .filter_map(|ex| {
            let text = (v.read)(&format!("fixtures/{}.yaml", ex.name))?;
            let doc: Value = serde_yaml::from_str(&text).ok()?;
            let input = doc.get("input")?;
            (input.get("effect")?.as_str()? == id).then(|| {
                let event = input.get("event").cloned().unwrap_or_default();
                let rows = input.get("rows").and_then(Value::as_array).cloned();
                (
                    ex.name.clone(),
                    serde_json::json!({
                        "id": "fixture",
                        "type": event.get("type").cloned().unwrap_or_default(),
                        "v": 1,
                        "seq": 0,
                        "source": "fixture",
                        "subject": event.get("subject").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "payload": event.get("payload").cloned().unwrap_or_else(|| serde_json::json!({})),
                    }),
                    rows,
                )
            })
        })
        .collect()
}

/// Fill an effect's outputs when it changed: each input — both versions'
/// fixtures for it, the latest events it'd react to — composed by each
/// version that reacts to it.
async fn effect_outputs(
    layer: &crate::sql_gateway::SqlGateway,
    before: Option<&Version<'_>>,
    after: &Version<'_>,
    effect: &mut EffectEffect,
    deadline: std::time::Instant,
) -> DryRuns {
    if effect.change == Change::Unchanged {
        return DryRuns::All;
    }
    let decl_of = |v: &Version<'_>| {
        v.extension
            .effects
            .iter()
            .find(|d| d.id == effect.id)
            .cloned()
    };
    let (db, da) = (before.and_then(decl_of), decl_of(after));
    let mut inputs: BTreeMap<String, (Value, Option<Vec<Value>>)> = BTreeMap::new();
    for v in before.into_iter().chain(std::iter::once(after)) {
        for (name, event, rows) in effect_fixtures(v, &effect.id) {
            inputs.insert(format!("fixture {name}"), (event, rows));
        }
    }
    let types: Vec<String> = da
        .iter()
        .chain(db.iter())
        .flat_map(|d| d.on.clone())
        .collect();
    for e in layer
        .recent_events(types, COLLECTOR_EVENTS)
        .await
        .unwrap_or_default()
    {
        inputs.insert(
            format!("event #{}", e.seq),
            (crate::effects::event_json(&e), None),
        );
    }
    // Each side reads through its own models (its `input:` may read them).
    let (layer_b, layer_a) = (
        layer.with_overlay(before.map(|v| v.overlay.to_vec()).unwrap_or_default()),
        layer.with_overlay(after.overlay.to_vec()),
    );
    for (label, (event, rows)) in inputs {
        if std::time::Instant::now() >= deadline {
            return DryRuns::OutOfTime;
        }
        let run = |layer: &crate::sql_gateway::SqlGateway,
                   decl: &Option<crate::effects::EffectDecl>| {
            let (layer, decl, event, rows) =
                (layer.clone(), decl.clone(), event.clone(), rows.clone());
            async move {
                let layer = &layer;
                let decl = decl?;
                let event_type = event["type"].as_str().unwrap_or_default().to_string();
                if !crate::effects::reacts_to(&decl, &event_type, &event["payload"]) {
                    return None;
                }
                Some(
                    match crate::effects::dry_run(layer, &decl, &decl.script, event, rows, None)
                        .await
                    {
                        Ok(crate::effects::Reaction::Skip(why)) => Composes {
                            commands: Vec::new(),
                            skip: Some(why),
                            error: None,
                        },
                        Ok(r) => Composes {
                            commands: r.command_names(),
                            skip: None,
                            error: None,
                        },
                        Err(e) => Composes {
                            commands: Vec::new(),
                            skip: None,
                            error: Some(e),
                        },
                    },
                )
            }
        };
        let (b, a) = (run(&layer_b, &db).await, run(&layer_a, &da).await);
        if b.is_none() && a.is_none() {
            continue;
        }
        effect.outputs.push(EffectOutput {
            input: label,
            change: change_of(b.as_ref(), a.as_ref()),
            before: b,
            after: a,
        });
    }
    DryRuns::All
}

fn trigger_text(t: &EffectTrigger) -> String {
    let filter = if t.filter.is_empty() {
        String::new()
    } else {
        format!(
            " where {}",
            t.filter
                .iter()
                .map(|(k, v)| format!("{k} = {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!("on {}{filter}", t.on.join(", "))
}

fn composes_text(c: Option<&Composes>) -> String {
    match c {
        None => "doesn't react".into(),
        Some(Composes { error: Some(e), .. }) => format!("fails ({e})"),
        Some(Composes {
            skip: Some(why), ..
        }) => format!("skips ({why})"),
        Some(c) => format!("runs [{}]", c.commands.join(", ")),
    }
}

/// The most inputs a collector's outputs are compared on, and the most
/// rows shown per entity.
pub const COLLECTOR_EVENTS: usize = 5;
const COLLECTOR_ROWS: usize = 20;

/// One input, run on each version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorOutput {
    /// `fixture <name>`, `event #<seq>` or `input query`.
    pub input: String,
    pub change: Change,
    pub before: Option<Ran>,
    pub after: Option<Ran>,
}

/// What one version's collector made of an input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Ran {
    /// Rows per entity.
    pub counts: BTreeMap<String, i64>,
    /// The rows per entity (the first 20).
    #[specta(type = oxplow_domain::Json)]
    pub rows: Value,
    /// Why it failed (a model call it tried was refused, say).
    pub error: Option<String>,
}

fn ran(run: crate::collector_runner::DryRun) -> Result<Ran, String> {
    use crate::collector_runner::DryRun;
    match run {
        DryRun::Output(out) => Ok(Ran {
            counts: out
                .entities
                .iter()
                .map(|(e, rows)| (e.clone(), rows.len() as i64))
                .collect(),
            rows: Value::Object(
                out.entities
                    .into_iter()
                    .map(|(e, rows)| {
                        (
                            e,
                            Value::Array(rows.into_iter().take(COLLECTOR_ROWS).collect()),
                        )
                    })
                    .collect(),
            ),
            error: None,
        }),
        DryRun::Failed(e) => Ok(Ran {
            counts: BTreeMap::new(),
            rows: Value::Null,
            error: Some(e),
        }),
        DryRun::NotRun(why) => Err(why),
    }
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
    /// What approving it would change, as sentences ([`approval_lines`]):
    /// what Settings → Data shows before a person approves.
    pub lines: Vec<String>,
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
    /// Its effects (P8.D12): what each reacts to, and what each version
    /// composes on the same events.
    pub effects: Vec<EffectEffect>,
    pub config: Option<ConfigEffect>,
    /// What its dry runs didn't get to (`collector <id>`, `effect <id>`):
    /// the review stops running scripts at its deadline.
    pub out_of_time: Vec<String>,
    /// The report as lines ([`summary`]): what the install review, `plugin
    /// check --effects` and an effort's review say, in one wording.
    pub lines: Vec<String>,
}

/// `none` for an empty list, else `a, b`.
fn listed_or_none(xs: &[String]) -> String {
    if xs.is_empty() {
        "none".into()
    } else {
        xs.join(", ")
    }
}

/// `definition`, `a`, `a and b`, `a, b and c`.
fn listed_and(parts: &[String]) -> String {
    match parts {
        [] => "definition".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn run_line(g: &Grants) -> String {
    std::iter::once(g.entry.clone())
        .chain(g.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A program's grants: "runs x · reaches y · reads z".
pub fn grants_line(g: &Grants) -> String {
    format!(
        "runs {} · reaches {} · reads {}",
        run_line(g),
        listed_or_none(&g.hosts),
        listed_or_none(&g.credentials)
    )
}

/// What a program's grants became, as phrases ("now reaches x (was y)").
pub fn grant_changes(before: Option<&Grants>, after: Option<&Grants>) -> Vec<String> {
    let (Some(b), Some(a)) = (before, after) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if b.entry != a.entry || b.runtime != a.runtime || b.args != a.args {
        out.push(format!("now runs {} (was {})", run_line(a), run_line(b)));
    }
    let pairs = [
        ("now reaches", &b.hosts, &a.hosts),
        ("now reads", &b.credentials, &a.credentials),
        ("now reads env", &b.env, &a.env),
    ];
    for (what, was, now) in pairs {
        if was != now {
            out.push(format!(
                "{what} {} (was {})",
                listed_or_none(now),
                listed_or_none(was)
            ));
        }
    }
    out
}

fn destructive(c: &CommandChange) -> &'static str {
    match c
        .after
        .as_ref()
        .and_then(|a| a.get("confirm"))
        .and_then(Value::as_str)
    {
        Some("destructive") => " (destructive)",
        _ => "",
    }
}

/// A provider's changes as phrases: everything it declares when it's new,
/// else its grants, commands, MCP tools and features that differ — and,
/// when none of those shows a change, where its declarations first differ.
pub fn provider_phrases(e: &ProviderEffect) -> Vec<String> {
    match e.change {
        Change::Added => {
            let commands: Vec<String> = e
                .commands
                .iter()
                .map(|c| format!("{}{}", c.name, destructive(c)))
                .collect();
            let mut out = vec![
                format!(
                    "added — {}",
                    e.after.as_ref().map_or("no grants".into(), grants_line)
                ),
                format!("commands: {}", listed_or_none(&commands)),
            ];
            // Behind oxplow's MCP adapter: the server's tools, as pinned.
            if !e.tools.is_empty() {
                let tools: Vec<String> = e.tools.iter().map(|t| t.name.clone()).collect();
                out.push(format!("MCP tools: {}", listed_or_none(&tools)));
            }
            out
        }
        Change::Removed => vec!["removed".into()],
        Change::Changed | Change::Unchanged => {
            let mut out = grant_changes(e.before.as_ref(), e.after.as_ref());
            for c in e.commands.iter().filter(|c| c.change != Change::Unchanged) {
                let tail = if c.change == Change::Removed {
                    ""
                } else {
                    destructive(c)
                };
                out.push(format!(
                    "command `{}` {}{tail}",
                    c.name,
                    change_word(c.change)
                ));
            }
            for t in e.tools.iter().filter(|t| t.change != Change::Unchanged) {
                out.push(format!("MCP tool `{}` {}", t.name, change_word(t.change)));
            }
            if e.features_before != e.features_after {
                let shown = |v: &Option<Value>| v.as_ref().map_or("null".into(), Value::to_string);
                out.push(format!(
                    "features now {} (were {})",
                    shown(&e.features_after),
                    shown(&e.features_before)
                ));
            }
            if out.is_empty() && e.change == Change::Changed {
                out.push(
                    e.first_difference
                        .clone()
                        .unwrap_or_else(|| "its declarations changed".into()),
                );
            }
            out
        }
    }
}

/// What approving a provider would change, as sentences: against what was
/// approved last, or everything it declares at a first approval.
pub fn approval_lines(e: &ProviderEffect) -> Vec<String> {
    let lines: Vec<String> = provider_phrases(e)
        .into_iter()
        .map(|l| {
            let mut chars = l.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => l,
            }
        })
        .collect();
    if lines.is_empty() {
        vec!["Nothing changed since it was last approved.".into()]
    } else {
        lines
    }
}

fn change_word(c: Change) -> &'static str {
    match c {
        Change::Added => "added",
        Change::Removed => "removed",
        Change::Changed => "changed",
        Change::Unchanged => "unchanged",
    }
}

fn counts(c: &BTreeMap<String, i64>) -> String {
    if c.is_empty() {
        return "nothing".into();
    }
    c.iter()
        .map(|(e, n)| format!("{n} {e}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ran_text(r: Option<&Ran>) -> String {
    match r {
        None => "—".into(),
        Some(Ran { error: Some(e), .. }) => format!("fails ({e})"),
        Some(r) => counts(&r.counts),
    }
}

/// The report as lines: collectors' and providers' grants first (what a
/// person approves), then what the collectors make of their inputs, the
/// models (and their rows), the lenses and the config schema.
pub fn summary(report: &EffectReport) -> Vec<String> {
    let mut out = Vec::new();
    for c in &report.collectors {
        match c.change {
            Change::Added => out.push(format!(
                "Collector {}: added — {}",
                c.id,
                c.after.as_ref().map_or("no grants".into(), grants_line)
            )),
            Change::Removed => out.push(format!("Collector {}: removed", c.id)),
            _ => {
                out.extend(
                    grant_changes(c.before.as_ref(), c.after.as_ref())
                        .into_iter()
                        .map(|g| format!("Collector {}: {g}", c.id)),
                );
                if c.script_changed {
                    out.push(format!("Collector {}: its script changed", c.id));
                }
            }
        }
    }
    for p in &report.providers {
        out.extend(
            provider_phrases(p)
                .into_iter()
                .map(|l| format!("Provider {}: {l}", p.id)),
        );
    }
    for e in &report.effects {
        match (e.change, &e.before, &e.after) {
            (Change::Added, _, Some(a)) => {
                out.push(format!("Effect {}: added — {}", e.id, trigger_text(a)))
            }
            (Change::Removed, _, _) => out.push(format!("Effect {}: removed", e.id)),
            (Change::Changed, Some(b), Some(a)) if b != a => out.push(format!(
                "Effect {}: {} → {}",
                e.id,
                trigger_text(b),
                trigger_text(a)
            )),
            (Change::Changed, _, _) => out.push(format!("Effect {}: its script changed", e.id)),
            _ => {}
        }
        for o in e.outputs.iter().filter(|o| o.change != Change::Unchanged) {
            out.push(format!(
                "Effect {} on {}: {} → {}",
                e.id,
                o.input,
                composes_text(o.before.as_ref()),
                composes_text(o.after.as_ref())
            ));
        }
    }
    for c in &report.collectors {
        if let Some(why) = &c.not_run {
            out.push(format!("Collector {}: not run — {why}", c.id));
        }
        for o in c.outputs.iter().filter(|o| o.change != Change::Unchanged) {
            out.push(format!(
                "Collector {} on {}: {} → {}",
                c.id,
                o.input,
                ran_text(o.before.as_ref()),
                ran_text(o.after.as_ref())
            ));
        }
    }
    for m in report
        .models
        .iter()
        .filter(|m| m.change != Change::Unchanged)
    {
        let what = match m.change {
            Change::Changed => m.contract_change.clone().unwrap_or_else(|| {
                format!("its {} changed (same columns)", listed_and(&m.changed))
            }),
            other => change_word(other).into(),
        };
        let read_by = if m.downstream.is_empty() {
            String::new()
        } else {
            format!("; read by {}", m.downstream.join(", "))
        };
        out.push(format!("Model {}: {what}{read_by}", m.view));
        if let Some(rows) = &m.rows {
            let side = |n: Option<i64>| n.map_or("—".into(), |n| n.to_string());
            let detail = match (&rows.keyed, &rows.note) {
                (Some(k), _) => format!(
                    " ({} added, {} removed, {} changed)",
                    k.added, k.removed, k.changed
                ),
                (None, Some(note)) => format!(" ({note})"),
                (None, None) => String::new(),
            };
            out.push(format!(
                "Model {} rows: {} → {}{detail}",
                m.view,
                side(rows.before),
                side(rows.after)
            ));
        }
    }
    for l in &report.lenses {
        match &l.error {
            Some(e) => {
                let change = if l.change == Change::Unchanged {
                    String::new()
                } else {
                    format!("{}; ", change_word(l.change))
                };
                out.push(format!("Lens {}: {change}its query fails: {e}", l.id));
            }
            None if l.change != Change::Unchanged => {
                out.push(format!("Lens {}: {}", l.id, change_word(l.change)))
            }
            None => {}
        }
    }
    if let Some(config) = &report.config {
        if let Some(other) = &config.other_change {
            out.push(format!("Config: {other}"));
        }
        if !config.changed_keys.is_empty() {
            out.push(format!(
                "Config: {} changed",
                config.changed_keys.join(", ")
            ));
        }
    }
    if !report.out_of_time.is_empty() {
        out.push(format!(
            "Out of time: {} — the review stops running scripts after {}s",
            listed_and(&report.out_of_time),
            REVIEW_DEADLINE.as_secs()
        ));
    }
    out
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
                rows: None,
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
            outputs: Vec::new(),
            not_run: None,
            script_changed: false,
        })
        .collect()
}

/// The fixtures of `v`'s intent examples that run collector `id`: the
/// example's name and its `rows`.
fn collector_fixtures(v: &Version<'_>, id: &str) -> Vec<(String, Option<Vec<Value>>)> {
    let examples = v
        .extension
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    examples
        .into_iter()
        .filter_map(|ex| {
            let text = (v.read)(&format!("fixtures/{}.yaml", ex.name))?;
            let doc: Value = serde_yaml::from_str(&text).ok()?;
            let input = doc.get("input")?;
            (input.get("collector")?.as_str()? == id).then(|| {
                let rows = input.get("rows").and_then(Value::as_array).cloned();
                (ex.name.clone(), rows)
            })
        })
        .collect()
}

/// Whether a collector's or effect's dry runs all ran, or the review's
/// deadline came first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DryRuns {
    All,
    OutOfTime,
}

/// Fill a collector's outputs: each input — both versions' fixtures for it,
/// the latest events it'd run on — run on each version.
async fn collector_outputs(
    layer: &crate::sql_gateway::SqlGateway,
    before: Option<&Version<'_>>,
    after: &Version<'_>,
    effect: &mut CollectorEffect,
    deadline: std::time::Instant,
) -> DryRuns {
    use crate::collector_runner::dry_run_collector;
    let spec_of = |v: &Version<'_>| {
        v.extension
            .collectors
            .iter()
            .find(|c| c.id == effect.id)
            .cloned()
    };
    let (spec_b, spec_a) = (before.and_then(spec_of), spec_of(after));
    let script = |v: Option<&Version<'_>>, spec: &Option<CollectorSpec>| {
        let (v, spec) = (v?, spec.as_ref()?);
        (v.read)(spec.entry.as_deref().unwrap_or_default())
    };
    let (script_b, script_a) = (script(before, &spec_b), script(Some(after), &spec_a));
    if effect.change == Change::Unchanged && script_b == script_a {
        return DryRuns::All;
    }
    // A rewritten script is a change even when nothing runs it here.
    if spec_b.is_some() && spec_a.is_some() && script_b != script_a {
        effect.script_changed = true;
        if effect.change == Change::Unchanged {
            effect.change = Change::Changed;
        }
    }
    let Some(spec) = spec_a.clone().or(spec_b.clone()) else {
        return DryRuns::All;
    };
    if !spec.runtime.is_derived() {
        if let crate::collector_runner::DryRun::NotRun(why) =
            dry_run_collector(layer, &spec, None, None, None).await
        {
            effect.not_run = Some(why);
        }
        return DryRuns::All;
    }
    // The inputs: fixtures (the candidate's first), then the events.
    let mut fixtures: BTreeMap<String, Option<Vec<Value>>> = BTreeMap::new();
    for v in before.into_iter().chain(std::iter::once(after)) {
        for (name, rows) in collector_fixtures(v, &effect.id) {
            fixtures.insert(name, rows);
        }
    }
    let events = match &spec.trigger {
        oxplow_config::collectors::Trigger::On { events, .. } => layer
            .recent_events(events.clone(), COLLECTOR_EVENTS)
            .await
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut inputs: Vec<(
        String,
        Option<oxplow_domain::StoredEvent>,
        Option<Vec<Value>>,
    )> = fixtures
        .into_iter()
        .map(|(name, rows)| (format!("fixture {name}"), None, rows))
        .chain(
            events
                .into_iter()
                .map(|e| (format!("event #{}", e.seq), Some(e), None)),
        )
        .collect();
    if inputs.is_empty() && spec.input.is_some() {
        inputs.push(("input query".into(), None, None));
    }
    // Each side reads through its own models (its `input:` may read them).
    let (layer_b, layer_a) = (
        layer.with_overlay(before.map(|v| v.overlay.to_vec()).unwrap_or_default()),
        layer.with_overlay(after.overlay.to_vec()),
    );
    for (label, event, rows) in inputs {
        if std::time::Instant::now() >= deadline {
            return DryRuns::OutOfTime;
        }
        let run = |layer: &crate::sql_gateway::SqlGateway,
                   spec: &Option<CollectorSpec>,
                   script: &Option<String>| {
            let (layer, spec, script, rows, event) = (
                layer.clone(),
                spec.clone(),
                script.clone(),
                rows.clone(),
                event.clone(),
            );
            async move {
                let spec = spec?;
                ran(dry_run_collector(&layer, &spec, script, event.as_ref(), rows).await).ok()
            }
        };
        let (b, a) = (
            run(&layer_b, &spec_b, &script_b).await,
            run(&layer_a, &spec_a, &script_a).await,
        );
        effect.outputs.push(CollectorOutput {
            input: label,
            change: change_of(b.as_ref(), a.as_ref()),
            before: b,
            after: a,
        });
    }
    DryRuns::All
}

fn provider_grants(p: &ProviderSpec) -> Grants {
    let (entry, args) = p.program();
    Grants {
        // A server by url isn't a program that runs here: what runs is
        // oxplow's adapter, against it.
        entry: if p.is_remote() {
            format!("oxplow's MCP adapter against {entry}")
        } else {
            entry
        },
        runtime: CollectorRuntime::Exec,
        args,
        hosts: p.network.clone(),
        credentials: p.credential_grants(),
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
            let mut effect = ProviderEffect {
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
                lines: Vec::new(),
            };
            effect.lines = approval_lines(&effect);
            effect
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
    /// The temp views its models compile to (`extensions::Prepared`), so
    /// its rows are its own version's.
    pub overlay: &'a [oxplow_db::TempView],
}

/// A model's rows on one side, read through that side's overlay (up to
/// [`ROW_DIFF_LIMIT`]); `Err` when the query failed.
async fn rows_of(
    layer: &crate::sql_gateway::SqlGateway,
    overlay: &[oxplow_db::TempView],
    view: &str,
) -> Result<Rows, String> {
    let result = layer
        .with_overlay(overlay.to_vec())
        .run(
            oxplow_db::SqlQuery::new(format!("SELECT * FROM \"{}\"", view.replace('"', "\"\"")))
                .limit(Some(ROW_DIFF_LIMIT)),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(Rows {
        columns: result.columns,
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
                    .collect()
            })
            .collect(),
        truncated: result.truncated,
    })
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
/// change. Lenses are rendered from each version's runs (on its own
/// overlay); models' rows read and diffed; derived collectors and effects
/// dry-run on the same inputs, within [`REVIEW_DEADLINE`]; exec
/// collectors and providers only compared.
pub async fn effects(
    layer: &crate::sql_gateway::SqlGateway,
    before: Option<Version<'_>>,
    after: Version<'_>,
) -> EffectReport {
    effects_within(layer, before, after, REVIEW_DEADLINE).await
}

/// How long a review spends running scripts (collectors' and effects' dry
/// runs), all told; each run has the command scripts' own budget.
pub const REVIEW_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// [`effects`], running scripts for at most `deadline`.
pub async fn effects_within(
    layer: &crate::sql_gateway::SqlGateway,
    before: Option<Version<'_>>,
    after: Version<'_>,
    deadline: std::time::Duration,
) -> EffectReport {
    let deadline = std::time::Instant::now() + deadline;
    let mut out_of_time = Vec::new();
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
                    // The same text, or the same failure.
                    (Some(x), Some(y)) if x == y => Change::Unchanged,
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
    let key_of = |v: &Version<'_>, view: &str| -> Vec<String> {
        v.extension
            .models
            .iter()
            .find(|m| extension_view(name, &m.decl.name) == view)
            .map(|m| m.decl.key.clone())
            .unwrap_or_default()
    };
    for m in &mut models {
        m.downstream = downstream_of(layer, &m.view)
            .await
            .into_iter()
            .filter(|v| !own.contains(v))
            .collect();
        if m.change == Change::Unchanged {
            continue;
        }
        let read = |v: Option<&Version<'_>>| {
            let view = m.view.clone();
            let overlay = v.map(|v| v.overlay.to_vec());
            async move {
                match overlay {
                    Some(o) => Some(rows_of(layer, &o, &view).await),
                    None => None,
                }
            }
        };
        let side_b = if m.change == Change::Added {
            None
        } else {
            before.as_ref()
        };
        let side_a = if m.change == Change::Removed {
            None
        } else {
            Some(&after)
        };
        let (rb, ra) = (read(side_b).await, read(side_a).await);
        let failed: Vec<String> = [("before", &rb), ("after", &ra)]
            .into_iter()
            .filter_map(|(side, r)| match r {
                Some(Err(e)) => Some(format!("{side}: {e}")),
                _ => None,
            })
            .collect();
        let (kb, ka) = (
            side_b.map(|v| key_of(v, &m.view)).unwrap_or_default(),
            key_of(&after, &m.view),
        );
        let ok =
            |r: &Option<Result<Rows, String>>| r.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        let (ob, oa) = (ok(&rb), ok(&ra));
        let mut diff = row_diff(
            ob.as_ref().map(|r| (r, kb.as_slice())),
            oa.as_ref().map(|r| (r, ka.as_slice())),
        );
        if !failed.is_empty() {
            diff.note = Some(format!("its query failed — {}", failed.join("; ")));
        }
        m.rows = Some(diff);
    }
    let no_collectors: Vec<CollectorSpec> = Vec::new();
    let mut collectors = collectors_diff(
        before
            .as_ref()
            .map_or(&no_collectors, |b| &b.extension.collectors),
        &after.extension.collectors,
    );
    for c in &mut collectors {
        if collector_outputs(layer, before.as_ref(), &after, c, deadline).await
            == DryRuns::OutOfTime
        {
            out_of_time.push(format!("collector {}", c.id));
        }
    }
    let mut effects = effects_diff(
        before
            .as_ref()
            .map_or(&[][..], |b| b.extension.effects.as_slice()),
        &after.extension.effects,
    );
    for e in &mut effects {
        if effect_outputs(layer, before.as_ref(), &after, e, deadline).await == DryRuns::OutOfTime {
            out_of_time.push(format!("effect {}", e.id));
        }
    }
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
    let mut report = EffectReport {
        lenses,
        models,
        collectors,
        providers,
        effects,
        config,
        out_of_time,
        lines: Vec::new(),
    };
    report.lines = summary(&report);
    report
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

    /// P9.B4: a server reached by url isn't a program that runs here —
    /// what runs is oxplow's adapter, against it — and moving it shows.
    #[test]
    fn a_url_servers_effects_say_what_runs_here() {
        let by_url = |url: &str| {
            let spec: ProviderSpec = serde_json::from_value(json!({
                "id": "notes", "capability": "work_items", "declarations": "provider.json",
                "adapter": { "mcp": { "url": url, "auth": "NOTES_TOKEN" },
                             "mapping": "mcp/x.star", "tools": "mcp/tools.json" },
                "credentials": ["NOTES_TOKEN"], "network": ["mcp.example.com"]
            }))
            .unwrap();
            DeclaredProvider {
                spec,
                declarations: Some(oxplow_provider_fake::declarations()),
                tools: Vec::new(),
            }
        };
        let added = providers_diff(&[], &[by_url("https://mcp.example.com/mcp")]);
        assert_eq!(
            provider_phrases(&added[0])[0],
            "added — runs oxplow's MCP adapter against https://mcp.example.com/mcp · reaches mcp.example.com · reads NOTES_TOKEN"
        );
        let moved = providers_diff(
            &[by_url("https://mcp.example.com/mcp")],
            &[by_url("https://mcp.example.com/v2")],
        );
        assert_eq!(
            provider_phrases(&moved[0]),
            vec![
                "now runs oxplow's MCP adapter against https://mcp.example.com/v2 (was oxplow's MCP adapter against https://mcp.example.com/mcp)"
            ]
        );
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

    /// tsk795: a lens that fails the same way in both versions didn't
    /// change; one whose failure changed did.
    #[tokio::test]
    async fn a_lens_failing_the_same_way_on_both_sides_is_unchanged() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let broken = "title: Broken\nquery: SELECT n FROM v_no_such_model\n";
        let (_b, before) = version(&[("broken", broken), ("moved", broken)]);
        let (_a, after) = version(&[
            ("broken", broken),
            (
                "moved",
                "title: Broken\nquery: SELECT n FROM v_another_missing_model\n",
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
                overlay: &[],
            }),
            Version {
                extension: &after,
                read: &none,
                lenses: &runs_after,
                overlay: &[],
            },
        )
        .await;
        let by: BTreeMap<&str, &LensEffect> =
            report.lenses.iter().map(|l| (l.id.as_str(), l)).collect();
        assert_eq!(by["x/broken"].change, Change::Unchanged);
        assert!(by["x/broken"].error.is_some(), "still says why it fails");
        assert_eq!(by["x/moved"].change, Change::Changed);
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
                overlay: &[],
            }),
            Version {
                extension: &after,
                read: &none,
                lenses: &runs_after,
                overlay: &[],
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
                overlay: &[],
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
                overlay: &[],
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

    fn side(columns: &[&str], rows: Vec<Vec<Value>>, truncated: bool) -> Rows {
        Rows {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
            truncated,
        }
    }

    /// P8.C3: with the same key on both sides, a changed model's rows are
    /// merge-joined — added, removed and changed counted, with samples.
    #[test]
    fn a_keyed_row_diff_counts_and_samples_each_change() {
        let key = vec!["id".to_string()];
        let before = side(
            &["id", "v"],
            vec![
                vec![json!(1), json!("a")],
                vec![json!(2), json!("b")],
                vec![json!(3), json!("c")],
            ],
            false,
        );
        let after = side(
            &["id", "v"],
            vec![
                vec![json!(1), json!("a")],
                vec![json!(2), json!("B")],
                vec![json!(4), json!("d")],
            ],
            false,
        );
        let diff = row_diff(Some((&before, &key)), Some((&after, &key)));
        assert_eq!(
            (diff.before, diff.after, diff.note.as_deref()),
            (Some(3), Some(3), None)
        );
        let keyed = diff.keyed.expect("keyed");
        assert_eq!((keyed.added, keyed.removed, keyed.changed), (1, 1, 1));
        let samples: Vec<(Change, Value, Option<Value>, Option<Value>)> = keyed
            .samples
            .into_iter()
            .map(|s| (s.change, s.key, s.before, s.after))
            .collect();
        assert_eq!(
            samples,
            vec![
                (
                    Change::Changed,
                    json!({"id": 2}),
                    Some(json!({"id": 2, "v": "b"})),
                    Some(json!({"id": 2, "v": "B"}))
                ),
                (
                    Change::Removed,
                    json!({"id": 3}),
                    Some(json!({"id": 3, "v": "c"})),
                    None
                ),
                (
                    Change::Added,
                    json!({"id": 4}),
                    None,
                    Some(json!({"id": 4, "v": "d"}))
                ),
            ]
        );
    }

    /// Without the same key on both sides there's nothing to join on:
    /// counts, and a note saying why; over the row limit, the same.
    #[test]
    fn an_unkeyed_or_oversized_row_diff_is_counts_with_a_note() {
        let before = side(&["n"], vec![vec![json!(1)]], false);
        let after = side(&["n"], vec![vec![json!(1)], vec![json!(2)]], false);
        let diff = row_diff(Some((&before, &[])), Some((&after, &[])));
        assert_eq!(
            (diff.before, diff.after, diff.keyed.is_none()),
            (Some(1), Some(2), true)
        );
        assert!(diff.note.unwrap().contains("no key"));

        let key = vec!["n".to_string()];
        let big = side(&["n"], vec![vec![json!(1)]], true);
        let diff = row_diff(Some((&before, &key)), Some((&big, &key)));
        assert!(diff.keyed.is_none());
        assert!(
            diff.note.unwrap().contains(&ROW_DIFF_LIMIT.to_string()),
            "names the limit"
        );

        let added = row_diff(None, Some((&after, &key)));
        assert_eq!((added.before, added.after), (None, Some(2)));
    }

    /// tsk792: a key that repeats or has a NULL part can't be joined on —
    /// one row would stand for several — so the diff is counts, saying why.
    #[test]
    fn a_repeated_or_null_key_is_counts_with_a_note() {
        let key = vec!["id".to_string()];
        let before = side(&["id", "v"], vec![vec![json!(1), json!("a")]], false);
        let repeats = side(
            &["id", "v"],
            vec![vec![json!(1), json!("a")], vec![json!(1), json!("b")]],
            false,
        );
        let diff = row_diff(Some((&before, &key)), Some((&repeats, &key)));
        assert!(diff.keyed.is_none(), "{diff:?}");
        assert_eq!((diff.before, diff.after), (Some(1), Some(2)));
        let note = diff.note.unwrap();
        assert!(note.contains("repeats") && note.contains("after"), "{note}");

        let null = side(&["id", "v"], vec![vec![json!(null), json!("a")]], false);
        let diff = row_diff(Some((&null, &key)), Some((&before, &key)));
        assert!(diff.keyed.is_none(), "{diff:?}");
        let note = diff.note.unwrap();
        assert!(note.contains("NULL") && note.contains("before"), "{note}");
    }

    /// tsk792: samples come in key order — 2 before 10 — not text order.
    #[test]
    fn samples_are_in_key_order() {
        let key = vec!["id".to_string()];
        let before = side(&["id"], vec![], false);
        let after = side(&["id"], vec![vec![json!(10)], vec![json!(2)]], false);
        let diff = row_diff(Some((&before, &key)), Some((&after, &key)));
        let keys: Vec<Value> = diff
            .keyed
            .unwrap()
            .samples
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert_eq!(keys, vec![json!({"id": 2}), json!({"id": 10})]);
    }

    /// One version of the `acme` extension under `root`: a Starlark
    /// collector labelling its fixture rows `label`, an exec one whose
    /// script would touch `sentinel`, and one asking a model.
    fn acme(root: &std::path::Path, label: &str, sentinel: &std::path::Path) {
        let dir = root.join("oxplow/extensions/acme");
        let w = |rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w(
            "extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: Things.\n  origin: thread:thr1\n  examples:\n    - { name: two, input: { collector: things }, expect: two things }\n    - { name: smart, input: { collector: smart }, expect: summaries }\ncollectors:\n  - id: things\n    runtime: starlark\n    entry: collectors/things.star\n    entities:\n      - { name: thing, key: id, columns: { id: int, label: text } }\n  - id: shell\n    runtime: exec\n    entry: collectors/shell.sh\n    entities:\n      - { name: line, key: n, columns: { n: int } }\n  - id: smart\n    runtime: starlark\n    entry: collectors/smart.star\n    entities:\n      - { name: summary, key: id, columns: { id: int, text: text } }\n",
        );
        w(
            "fixtures/two.yaml",
            "input: { collector: things, rows: [{ id: 1 }, { id: 2 }] }\nexpect: { entities: { thing: 2 } }\n",
        );
        w(
            "fixtures/smart.yaml",
            "input: { collector: smart, rows: [{ id: 1 }] }\nexpect: { entities: { summary: 1 } }\n",
        );
        w(
            "collectors/things.star",
            &format!("def transform(input):\n    return {{\"entities\": {{\"thing\": [{{\"id\": r[\"id\"], \"label\": \"{label}\"}} for r in input[\"rows\"]]}}}}\n"),
        );
        w(
            "collectors/shell.sh",
            &format!(
                "#!/bin/sh\ntouch '{}'\necho '{label}'\n",
                sentinel.display()
            ),
        );
        w(
            "collectors/smart.star",
            &format!("def transform(input):\n    return {{\"entities\": {{\"summary\": [{{\"id\": r[\"id\"], \"text\": ai_summarize(\"{label}\")}} for r in input[\"rows\"]]}}}}\n"),
        );
    }

    /// tsk782: a collector's `input:` reads each side's own models — the
    /// before side sees its version of a view, the after side its own.
    #[tokio::test]
    async fn a_collectors_input_reads_each_sides_own_models() {
        let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let write = |root: &std::path::Path, label: &str| {
            let dir = root.join("oxplow/extensions/acme");
            std::fs::create_dir_all(dir.join("collectors")).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                "manifest: 2\nname: acme\nintent:\n  purpose: Things.\n  origin: thread:thr1\n  examples: []\ncollectors:\n  - id: things\n    runtime: starlark\n    entry: collectors/things.star\n    input: \"SELECT id FROM v_acme_src\"\n    entities:\n      - { name: thing, key: id, columns: { id: int, label: text } }\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("collectors/things.star"),
                format!("def transform(input):\n    return {{\"entities\": {{\"thing\": [{{\"id\": r[\"id\"], \"label\": \"{label}\"}} for r in input[\"rows\"]]}}}}\n"),
            )
            .unwrap();
        };
        write(old.path(), "a");
        write(new.path(), "b");
        let (eb, ea) = (
            crate::extensions::load_project_extension(old.path(), "acme"),
            crate::extensions::load_project_extension(new.path(), "acme"),
        );
        let reader = |root: std::path::PathBuf| {
            move |rel: &str| {
                std::fs::read_to_string(root.join("oxplow/extensions/acme").join(rel)).ok()
            }
        };
        let (rb, ra) = (
            reader(old.path().to_path_buf()),
            reader(new.path().to_path_buf()),
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let runs = crate::extensions::LensRuns::new();
        let view = |sql: &str| {
            vec![oxplow_db::TempView {
                name: "v_acme_src".into(),
                sql: sql.into(),
            }]
        };
        let (ob, oa) = (
            view("SELECT 1 AS id"),
            view("SELECT 1 AS id UNION ALL SELECT 2"),
        );
        let report = effects(
            &layer,
            Some(Version {
                extension: &eb,
                read: &rb,
                lenses: &runs,
                overlay: &ob,
            }),
            Version {
                extension: &ea,
                read: &ra,
                lenses: &runs,
                overlay: &oa,
            },
        )
        .await;
        let out = &report.collectors[0].outputs;
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(
            out[0].before.as_ref().unwrap().counts["thing"],
            1,
            "{out:?}"
        );
        assert_eq!(out[0].after.as_ref().unwrap().counts["thing"], 2, "{out:?}");
    }

    /// tsk783: a collector whose script alone changed — no fixture, event or
    /// `input:` to run it on — still says so.
    #[tokio::test]
    async fn a_collector_whose_script_alone_changed_says_so() {
        let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let write = |root: &std::path::Path, label: &str| {
            let dir = root.join("oxplow/extensions/acme");
            std::fs::create_dir_all(dir.join("collectors")).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                "manifest: 2\nname: acme\nintent:\n  purpose: Things.\n  origin: thread:thr1\n  examples: []\ncollectors:\n  - id: things\n    runtime: starlark\n    entry: collectors/things.star\n    entities:\n      - { name: thing, key: id, columns: { id: int, label: text } }\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("collectors/things.star"),
                format!("def transform(input):\n    return {{\"entities\": {{\"thing\": [{{\"id\": 1, \"label\": \"{label}\"}}]}}}}\n"),
            )
            .unwrap();
        };
        write(old.path(), "a");
        write(new.path(), "b");
        let (eb, ea) = (
            crate::extensions::load_project_extension(old.path(), "acme"),
            crate::extensions::load_project_extension(new.path(), "acme"),
        );
        let reader = |root: std::path::PathBuf| {
            move |rel: &str| {
                std::fs::read_to_string(root.join("oxplow/extensions/acme").join(rel)).ok()
            }
        };
        let (rb, ra) = (
            reader(old.path().to_path_buf()),
            reader(new.path().to_path_buf()),
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let runs = crate::extensions::LensRuns::new();
        let report = effects(
            &layer,
            Some(Version {
                extension: &eb,
                read: &rb,
                lenses: &runs,
                overlay: &[],
            }),
            Version {
                extension: &ea,
                read: &ra,
                lenses: &runs,
                overlay: &[],
            },
        )
        .await;
        assert_eq!(report.collectors[0].change, Change::Changed);
        assert!(
            report
                .lines
                .contains(&"Collector things: its script changed".to_string()),
            "{:?}",
            report.lines
        );
    }

    /// tsk791: a review's dry run gets the command scripts' budget, not a
    /// collector's two minutes, and the review stops running scripts at
    /// its deadline — saying what it didn't run.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_runaway_collector_is_given_up_on_and_the_review_stops_at_its_deadline() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("collectors")).unwrap();
        std::fs::create_dir_all(dir.join("fixtures")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: Things.\n  origin: thread:thr1\n  examples:\n    - { name: one, input: { collector: things }, expect: x }\n    - { name: two, input: { collector: things }, expect: x }\ncollectors:\n  - id: things\n    runtime: starlark\n    entry: collectors/things.star\n    entities:\n      - { name: thing, key: id, columns: { id: int } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("collectors/things.star"),
            "def transform(input):\n    n = 0\n    for i in range(400000000):\n        n += i\n    return {\"entities\": {\"thing\": []}}\n",
        )
        .unwrap();
        for f in ["one", "two"] {
            std::fs::write(
                dir.join(format!("fixtures/{f}.yaml")),
                "input:\n  collector: things\n  rows: []\n",
            )
            .unwrap();
        }
        let ext = crate::extensions::load_project_extension(root.path(), "acme");
        let read = |rel: &str| std::fs::read_to_string(dir.join(rel)).ok();
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let runs = crate::extensions::LensRuns::new();
        let started = std::time::Instant::now();
        let report = effects_within(
            &layer,
            None,
            Version {
                extension: &ext,
                read: &read,
                lenses: &runs,
                overlay: &[],
            },
            std::time::Duration::from_secs(1),
        )
        .await;
        let budget = crate::extension_commands::COMMAND_SCRIPT_BUDGET.timeout;
        assert!(
            started.elapsed() < budget + std::time::Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        let c = &report.collectors[0];
        assert_eq!(c.outputs.len(), 1, "{:?}", c.outputs);
        let error = c.outputs[0].after.as_ref().and_then(|r| r.error.clone());
        assert!(error.is_some_and(|e| e.contains("time")), "{:?}", c.outputs);
        assert_eq!(report.out_of_time, vec!["collector things".to_string()]);
        assert!(
            report.lines.iter().any(|l| l.starts_with("Out of time")),
            "{:?}",
            report.lines
        );
    }

    /// tsk779: a side past the limit reads `limit` rows, truncated, and the
    /// limit is what the gateway really returns — the note's number is the
    /// count it reports, not a larger one the gateway never reaches.
    #[tokio::test]
    async fn a_side_past_the_row_limit_reads_exactly_the_limit() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let overlay = [oxplow_db::TempView {
            name: "v_many".into(),
            sql: format!(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {}) \
                 SELECT i AS id FROM n",
                ROW_DIFF_LIMIT + 1
            ),
        }];
        let rows = rows_of(&layer, &overlay, "v_many").await.unwrap();
        assert!(rows.truncated);
        assert_eq!(rows.rows.len(), ROW_DIFF_LIMIT);
        let diff = row_diff(
            Some((&rows, &["id".to_string()][..])),
            Some((&rows, &["id".to_string()][..])),
        );
        assert_eq!(diff.before, Some(ROW_DIFF_LIMIT as i64));
        assert!(
            diff.note
                .as_deref()
                .is_some_and(|n| n.contains(&format!("over {ROW_DIFF_LIMIT} rows"))),
            "{diff:?}"
        );
    }

    /// P8.D12: a changed effect shows what it reacts to and what each
    /// version composes on its fixture — nothing runs.
    #[tokio::test]
    async fn an_effects_trigger_and_composed_commands_are_compared() {
        let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let write = |root: &std::path::Path, filter: &str, script: &str| {
            let dir = root.join("oxplow/extensions/acme");
            std::fs::create_dir_all(dir.join("fixtures")).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                format!(
                    "manifest: 2\nname: acme\nsharing: private\nintent:\n  purpose: p\n  examples:\n    - {{ name: basic, input: x, expect: y }}\neffects:\n  - {{ id: on-done, summary: s, on: [work_item.transitioned]{filter}, entry: e.star }}\n"
                ),
            )
            .unwrap();
            std::fs::write(dir.join("e.star"), script).unwrap();
            std::fs::write(
                dir.join("fixtures/basic.yaml"),
                "input: { effect: on-done, event: { type: work_item.transitioned, payload: { work_item: \"work_item:oxplow:tsk1\", to: done } } }\nexpect: { commands: [work_item.comment] }\n",
            )
            .unwrap();
        };
        write(
            old.path(),
            "",
            "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.comment\", \"input\": {}}]}\n",
        );
        write(
            new.path(),
            ", where: { to: done }",
            "def transform(x):\n    return {\"skip\": \"not today\"}\n",
        );
        let (eb, ea) = (
            crate::extensions::load_project_extension(old.path(), "acme"),
            crate::extensions::load_project_extension(new.path(), "acme"),
        );
        assert!(ea.errors.is_empty(), "{:?}", ea.errors);
        let reader = |root: std::path::PathBuf| {
            move |rel: &str| {
                std::fs::read_to_string(root.join("oxplow/extensions/acme").join(rel)).ok()
            }
        };
        let (rb, ra) = (
            reader(old.path().to_path_buf()),
            reader(new.path().to_path_buf()),
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let runs = crate::extensions::LensRuns::new();
        let report = effects(
            &layer,
            Some(Version {
                extension: &eb,
                read: &rb,
                lenses: &runs,
                overlay: &[],
            }),
            Version {
                extension: &ea,
                read: &ra,
                lenses: &runs,
                overlay: &[],
            },
        )
        .await;
        assert_eq!(
            report
                .lines
                .iter()
                .filter(|l| l.starts_with("Effect"))
                .collect::<Vec<_>>(),
            vec![
                "Effect on-done: on work_item.transitioned → on work_item.transitioned where to = done",
                "Effect on-done on fixture basic: runs [work_item.comment] → skips (not today)",
            ]
        );
    }

    /// P8.C4: a review runs each changed derived collector on the same
    /// inputs in both versions — here its fixture — and shows how the
    /// output differs; it never runs an exec collector's program (approved
    /// or not), and a model call is refused, not made.
    #[tokio::test]
    async fn a_collectors_outputs_are_compared_without_running_a_program_or_a_model() {
        let (old, new, scratch) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let sentinel = scratch.path().join("ran");
        acme(old.path(), "a", &sentinel);
        acme(new.path(), "b", &sentinel);
        let (eb, ea) = (
            crate::extensions::load_project_extension(old.path(), "acme"),
            crate::extensions::load_project_extension(new.path(), "acme"),
        );
        let reader = |root: std::path::PathBuf| {
            move |rel: &str| {
                std::fs::read_to_string(root.join("oxplow/extensions/acme").join(rel)).ok()
            }
        };
        let (rb, ra) = (
            reader(old.path().to_path_buf()),
            reader(new.path().to_path_buf()),
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let runs = crate::extensions::LensRuns::new();
        let report = effects(
            &layer,
            Some(Version {
                extension: &eb,
                read: &rb,
                lenses: &runs,
                overlay: &[],
            }),
            Version {
                extension: &ea,
                read: &ra,
                lenses: &runs,
                overlay: &[],
            },
        )
        .await;
        let by_id = |id: &str| {
            report
                .collectors
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .clone()
        };

        let things = by_id("things");
        assert_eq!(things.outputs.len(), 1, "{things:?}");
        let out = &things.outputs[0];
        assert_eq!(
            (out.input.as_str(), out.change),
            ("fixture two", Change::Changed)
        );
        assert_eq!(out.before.as_ref().unwrap().counts["thing"], 2);
        assert_eq!(out.after.as_ref().unwrap().rows["thing"][0]["label"], "b");

        let shell = by_id("shell");
        assert!(shell.outputs.is_empty());
        assert!(shell.not_run.unwrap().contains("never"));
        assert!(
            !sentinel.exists(),
            "a review never spawns an exec collector"
        );

        let smart = by_id("smart");
        let error = smart.outputs[0]
            .after
            .as_ref()
            .unwrap()
            .error
            .clone()
            .unwrap();
        assert!(error.contains("refused"), "{error}");
    }

    // ---- P8.C5: the wording, ported from the desktop's line tests ----

    fn grants(hosts: &[&str], credentials: &[&str]) -> Grants {
        Grants {
            entry: "bin/p".into(),
            runtime: CollectorRuntime::Exec,
            args: vec![],
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            credentials: credentials.iter().map(|c| c.to_string()).collect(),
            env: vec![],
        }
    }

    fn command(name: &str, change: Change, after: Option<Value>) -> CommandChange {
        CommandChange {
            name: name.into(),
            change,
            before: None,
            after,
        }
    }

    fn effect(change: Change) -> ProviderEffect {
        ProviderEffect {
            id: "fake".into(),
            capability: "work_items".into(),
            change,
            before: Some(grants(&[], &["token"])),
            after: Some(grants(&[], &["token"])),
            commands: vec![],
            tools: vec![],
            features_before: None,
            features_after: None,
            first_difference: None,
            lines: vec![],
        }
    }

    #[test]
    fn a_new_provider_reads_as_everything_it_declares() {
        let added = ProviderEffect {
            before: None,
            after: Some(grants(&["api.example.com"], &["token"])),
            commands: vec![
                command("create", Change::Added, Some(json!({"confirm": "never"}))),
                command(
                    "delete",
                    Change::Added,
                    Some(json!({"confirm": "destructive"})),
                ),
            ],
            ..effect(Change::Added)
        };
        assert_eq!(
            provider_phrases(&added),
            vec![
                "added — runs bin/p · reaches api.example.com · reads token",
                "commands: create, delete (destructive)"
            ]
        );
    }

    #[test]
    fn a_changed_provider_reads_as_what_differs() {
        let changed = ProviderEffect {
            after: Some(grants(&["api.example.com"], &["token"])),
            commands: vec![
                command("create", Change::Unchanged, Some(json!({}))),
                command(
                    "archive",
                    Change::Added,
                    Some(json!({"confirm": "destructive"})),
                ),
            ],
            ..effect(Change::Changed)
        };
        assert_eq!(
            provider_phrases(&changed),
            vec![
                "now reaches api.example.com (was none)",
                "command `archive` added (destructive)"
            ]
        );
        assert_eq!(
            approval_lines(&changed)[0],
            "Now reaches api.example.com (was none)"
        );
        assert_eq!(
            approval_lines(&effect(Change::Unchanged)),
            vec!["Nothing changed since it was last approved."]
        );
    }

    #[test]
    fn a_change_no_grant_command_or_feature_shows_names_its_first_difference() {
        let quiet = ProviderEffect {
            first_difference: Some("`/declarations/version` was 1, now 2".into()),
            ..effect(Change::Changed)
        };
        assert_eq!(
            provider_phrases(&quiet),
            vec!["`/declarations/version` was 1, now 2"]
        );
        assert!(provider_phrases(&effect(Change::Unchanged)).is_empty());
    }

    #[test]
    fn an_mcp_adapter_provider_names_its_pinned_tools_and_each_that_changed() {
        let tool = |name: &str, change: Change| command(name, change, Some(json!({"name": name})));
        let added = ProviderEffect {
            before: None,
            after: Some(Grants {
                entry: "bin/server".into(),
                args: vec!["--stdio".into()],
                ..grants(&[], &["token"])
            }),
            tools: vec![
                tool("create_item", Change::Added),
                tool("list_items", Change::Added),
            ],
            ..effect(Change::Added)
        };
        assert_eq!(
            provider_phrases(&added),
            vec![
                "added — runs bin/server --stdio · reaches none · reads token",
                "commands: none",
                "MCP tools: create_item, list_items"
            ]
        );
        let changed = ProviderEffect {
            tools: vec![
                tool("list_items", Change::Changed),
                tool("drop_all", Change::Added),
                tool("create_item", Change::Unchanged),
            ],
            ..effect(Change::Changed)
        };
        assert_eq!(
            provider_phrases(&changed),
            vec!["MCP tool `list_items` changed", "MCP tool `drop_all` added"]
        );
    }

    /// The report as lines, grants first — what the install review, the
    /// CLI and an effort's review all say.
    #[test]
    fn effects_spell_out_each_change_grants_first() {
        let lens = |id: &str, change: Change, error: Option<&str>| LensEffect {
            id: id.into(),
            change,
            before: None,
            after: None,
            error: error.map(str::to_string),
        };
        let model = |view: &str, changed: &[&str], contract: Option<&str>, downstream: &[&str]| {
            ModelEffect {
                view: view.into(),
                change: Change::Changed,
                changed: changed.iter().map(|c| c.to_string()).collect(),
                before_columns: vec![],
                after_columns: vec![],
                contract_change: contract.map(str::to_string),
                downstream: downstream.iter().map(|d| d.to_string()).collect(),
                rows: None,
            }
        };
        let mut rows_model = model("v_shared_r", &["query"], None, &[]);
        rows_model.rows = Some(RowDiff {
            before: Some(3),
            after: Some(3),
            keyed: Some(KeyedDiff {
                key: vec!["id".into()],
                added: 1,
                removed: 1,
                changed: 1,
                samples: vec![],
            }),
            note: None,
        });
        let report = EffectReport {
            lenses: vec![
                lens("shared/count", Change::Changed, None),
                lens("shared/same", Change::Unchanged, None),
                lens("shared/new", Change::Added, None),
                lens("shared/bad", Change::Added, Some("no such table")),
            ],
            models: vec![
                model(
                    "v_shared_x",
                    &["columns"],
                    Some("column `y` added"),
                    &["v_b_y"],
                ),
                model("v_shared_z", &["query"], None, &[]),
                model("v_shared_t", &["description", "tests"], None, &[]),
                rows_model,
            ],
            collectors: vec![CollectorEffect {
                id: "gh".into(),
                change: Change::Changed,
                before: Some(Grants {
                    entry: "sync.sh".into(),
                    ..grants(&[], &[])
                }),
                after: Some(Grants {
                    entry: "sync.sh".into(),
                    ..grants(&["api.example.com"], &[])
                }),
                entities: vec![],
                outputs: vec![CollectorOutput {
                    input: "fixture two".into(),
                    change: Change::Changed,
                    before: Some(Ran {
                        counts: [("thing".to_string(), 2)].into(),
                        rows: Value::Null,
                        error: None,
                    }),
                    after: Some(Ran {
                        counts: BTreeMap::new(),
                        rows: Value::Null,
                        error: Some("boom".into()),
                    }),
                }],
                not_run: None,
                script_changed: false,
            }],
            providers: vec![ProviderEffect {
                commands: vec![
                    command(
                        "delete",
                        Change::Added,
                        Some(json!({"name": "delete", "confirm": "destructive"})),
                    ),
                    command("create", Change::Unchanged, Some(json!({}))),
                ],
                features_before: Some(json!({"comments": false})),
                features_after: Some(json!({"comments": false})),
                ..effect(Change::Changed)
            }],
            effects: vec![
                EffectEffect {
                    id: "on-done".into(),
                    change: Change::Changed,
                    before: Some(EffectTrigger {
                        on: vec!["work_item.transitioned".into()],
                        filter: BTreeMap::new(),
                        input: None,
                    }),
                    after: Some(EffectTrigger {
                        on: vec!["work_item.transitioned".into()],
                        filter: [("to".to_string(), "done".to_string())].into(),
                        input: None,
                    }),
                    outputs: vec![EffectOutput {
                        input: "fixture basic".into(),
                        change: Change::Changed,
                        before: Some(Composes {
                            commands: vec!["work_item.comment".into()],
                            skip: None,
                            error: None,
                        }),
                        after: Some(Composes {
                            commands: vec![],
                            skip: Some("not today".into()),
                            error: None,
                        }),
                    }],
                },
                EffectEffect {
                    id: "ping".into(),
                    change: Change::Added,
                    before: None,
                    after: Some(EffectTrigger {
                        on: vec!["vcs.head.moved".into()],
                        filter: BTreeMap::new(),
                        input: None,
                    }),
                    outputs: vec![],
                },
            ],
            config: Some(ConfigEffect {
                before: None,
                after: Some(json!({})),
                changed_keys: vec!["team".into()],
                other_change: None,
            }),
            out_of_time: vec![],
            lines: vec![],
        };
        assert_eq!(
            summary(&report),
            vec![
                "Collector gh: now reaches api.example.com (was none)",
                "Provider fake: command `delete` added (destructive)",
                "Effect on-done: on work_item.transitioned → on work_item.transitioned where to = done",
                "Effect on-done on fixture basic: runs [work_item.comment] → skips (not today)",
                "Effect ping: added — on vcs.head.moved",
                "Collector gh on fixture two: 2 thing → fails (boom)",
                "Model v_shared_x: column `y` added; read by v_b_y",
                "Model v_shared_z: its query changed (same columns)",
                "Model v_shared_t: its description and tests changed (same columns)",
                "Model v_shared_r: its query changed (same columns)",
                "Model v_shared_r rows: 3 → 3 (1 added, 1 removed, 1 changed)",
                "Lens shared/count: changed",
                "Lens shared/new: added",
                "Lens shared/bad: added; its query fails: no such table",
                "Config: team changed",
            ]
        );
    }
}
