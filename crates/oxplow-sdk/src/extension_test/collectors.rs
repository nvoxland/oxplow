use super::*;

/// Run fact collector `spec` over the fixture's `files` (path → content;
/// its other input keys — `report`, `rows`, `event` — are the collector's
/// input), storing nothing; its facts against `expect: { facts: [...] }`,
/// each compared on the keys its expected fact names (tsk1047).
pub(super) async fn fact_collector_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    spec: &oxplow_config::collectors::CollectorSpec,
    report: &mut TestReport,
) {
    let (shown, id) = (ex.shown, spec.id.as_str());
    let files: std::collections::HashMap<String, String> = ex
        .when
        .get("files")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let mut input: serde_json::Map<String, Value> = ex
        .when
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "collector" | "files"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    // Its `event`, as the runtime would hand it over.
    if input.contains_key("event") {
        let Some(event) = example_event(host, ex, report) else {
            return;
        };
        input.insert("event".into(), event.to_json());
    }
    let facts = match host
        .svc
        .metrics
        .preview_fact_collector(
            &ext.name,
            spec,
            files,
            Value::Object(input),
            oxplow_domain::run::Mode::External,
        )
        .await
    {
        Ok(f) => f,
        Err(why) => {
            report.errors.push(format!(
                "{shown}:1: collector `{id}` failed: {why} — fix: its script, or the example's files"
            ));
            return;
        }
    };
    // Each fact compared on what its expected fact names.
    let wanted = ex.expect.get("facts").and_then(Value::as_array);
    let got: Vec<Value> = facts
        .iter()
        .enumerate()
        .map(
            |(i, f)| match wanted.and_then(|w| w.get(i)).and_then(Value::as_object) {
                Some(keys) => Value::Object(
                    keys.keys()
                        .map(|k| (k.clone(), f.get(k).cloned().unwrap_or(Value::Null)))
                        .collect(),
                ),
                None => f.clone(),
            },
        )
        .collect();
    if let Some((path, want, got)) = compare(&ex.expect, &json!({ "facts": got })) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ facts: [{{ measure, value?, path?, \
             subject?, line?, rule?, dims? }}] }}`)"
        ));
    }
}

/// Run derived collector `id` over the fixture's `rows` (else its `input`
/// query, on the empty throwaway), storing nothing; the rows it would
/// store per entity — typed against the declaration — against `expect`.
pub(super) async fn collector_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let (shown, example, expect) = (ex.shown, ex.name, &ex.expect);
    let Some(spec) = ext.collectors.iter().find(|c| c.id == id) else {
        report.errors.push(format!(
            "{shown}:1: example `{example}` names collector `{id}`, which `{}` doesn't declare — \
             fix: the collector's `id`",
            ext.name
        ));
        return;
    };
    {
        use oxplow_config::collectors::CollectorRuntime as R;
        let does = match spec.runtime {
            R::Read => Some("reads a provider"),
            R::Exec if spec.bundled_parser().is_none() => Some("runs a program"),
            _ => None,
        };
        if let Some(does) = does {
            report.warnings.push(format!(
                "{shown}:1: example `{example}` runs collector `{id}`, which {does}: `extension \
                 test` doesn't run programs or providers — fix: none; run it with \
                 `oxplow.collector.sync` once it's deployed"
            ));
            return;
        }
    }
    if !spec.facts.is_empty() {
        return fact_collector_example(host, ext, ex, spec, report).await;
    }
    let rows = ex
        .when
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| r.to_vec());
    let preview = oxplow_app::collector_runner::preview_collector(
        &oxplow_app::collector_runner::Collectors::of(&host.svc, &host.root),
        &ext.name,
        id,
        ex.when.get("event"),
        rows,
    )
    .await;
    let preview = match preview {
        Ok(p) => p,
        Err(e) => {
            use oxplow_app::collector_runner::RunCollectorError as E;
            let why = match e {
                E::NotFound => "it isn't loaded".to_string(),
                E::Denied(m) | E::Failed(m) | E::Disabled(m) => m,
                E::Storage(e) => e.to_string(),
            };
            report.errors.push(format!(
                "{shown}:1: collector `{id}` failed: {why} — fix: its script, or the example's rows"
            ));
            return;
        }
    };
    let entities: serde_json::Map<String, Value> = preview
        .entities
        .iter()
        .map(|e| (e.entity.clone(), json!(e.total)))
        .collect();
    // The events it would log (its extension's own types, checked as a
    // run's are): `expect: { events: [{ type, payload? }] }`.
    let events: Vec<Value> = preview
        .events
        .iter()
        .map(|e| json!({ "type": e.event_type, "v": e.v, "payload": e.payload, "subject": e.subject }))
        .collect();
    // Said only when there is something to say (or the example asks):
    // an example of a collector that emits must say what it expects.
    let mut got = json!({ "entities": entities });
    if !events.is_empty() || expect.get("events").is_some() {
        got["events"] = Value::Array(events);
    }
    if let Some((path, want, got)) = compare(expect, &got) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ entities: {{ <name>: n | $any }}, \
             events?: [{{ type, payload? }}] }}`)"
        ));
    }
}

/// Each incremental dataset (P8.B5), on its fixture `fixtures/dataset-<name>.yaml`
/// (`before:` and `after:`, each `{ <table>: [rows] }`): built whole on
/// `before`, appended to past its watermark after `after` is written, and
/// compared with a full refill — in a rehearsal, so nothing stays. A
/// watermark that doesn't grow as rows arrive drops a row below it
/// unseen; nothing at run time can tell, so this is where it shows.
pub(super) async fn incremental_datasets(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap_or_default();
    for dataset in &ext.datasets {
        let decl = &dataset.decl;
        if decl
            .materialize
            .as_ref()
            .and_then(oxplow_db::datasets::Materialize::incremental)
            .is_none()
        {
            continue;
        }
        let line = oxplow_app::extension::extensions::manifest_v2::entry_line(
            &manifest, "datasets", "name", &decl.name,
        )
        .unwrap_or(1);
        let at = format!("{rel}/extension.yaml:{line}");
        let fixture = format!("fixtures/dataset-{}.yaml", decl.name);
        let Ok(text) = std::fs::read_to_string(dir.join(&fixture)) else {
            report.warnings.push(format!(
                "{at}: incremental dataset `{}` has no {rel}/{fixture}, so appending isn't checked \
                 against a full refill — fix: write its `before:` and `after:` rows \
                 (`{{ <table>: [{{ column: value }}] }}`)",
                decl.name
            ));
            continue;
        };
        let rows = |doc: &Value, part: &str| -> Result<oxplow_db::datasets::FixtureRows, String> {
            let Some(tables) = doc.get(part) else {
                return Ok(Vec::new());
            };
            let tables = tables
                .as_object()
                .ok_or_else(|| format!("`{part}` isn't a map of table → rows"))?;
            tables
                .iter()
                .map(|(table, rows)| {
                    let rows = rows
                        .as_array()
                        .ok_or_else(|| format!("`{part}.{table}` isn't a list of rows"))?
                        .iter()
                        .map(|r| {
                            r.as_object()
                                .cloned()
                                .ok_or_else(|| format!("a row of `{part}.{table}` isn't a map"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((table.clone(), rows))
                })
                .collect()
        };
        let parsed = serde_yaml::from_str::<Value>(&text)
            .map_err(|e| e.to_string())
            .and_then(|doc| Ok((rows(&doc, "before")?, rows(&doc, "after")?)));
        let (before, after) = match parsed {
            Ok(p) => p,
            Err(e) => {
                report
                    .errors
                    .push(format!("{rel}/{fixture}:1: {e} — fix: correct the fixture"));
                continue;
            }
        };
        report.ran.push(format!("incremental {}", decl.name));
        let view = oxplow_db::datasets::extension_view(&ext.name, &decl.name);
        let decl_c = decl.clone();
        let checked = host
            .svc
            .db
            .rehearse(move |tx| {
                oxplow_db::datasets::incremental_matches_full(tx, &view, &decl_c, &before, &after)
            })
            .await;
        match checked {
            Ok(None) => {}
            Ok(Some(problem)) => report.errors.push(format!(
                "{at}: incremental dataset `{}` {problem} — fix: `materialize: on_change`, or a \
                 watermark that only grows as rows arrive",
                decl.name
            )),
            Err(e) => report.errors.push(format!(
                "{rel}/{fixture}:1: incremental dataset `{}` couldn't run on the fixture: {e} — \
                 fix: the fixture's rows",
                decl.name
            )),
        }
    }
}

/// The extension's own `questions.yaml` (the answerability check): each
/// question's `skill` is a markdown file in the extension, its SQL runs
/// on the throwaway oxplow, and a command is one on its bus (core's, the
/// extension's own `commands:`) or one of its providers' (`<id>.<name>`,
/// not on the bus until an instance runs).
pub(super) async fn questions(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let file = format!("{rel}/questions.yaml");
    let Ok(text) = std::fs::read_to_string(dir.join("questions.yaml")) else {
        return;
    };
    report.ran.push("questions".into());
    let questions = match crate::answerability::parse(&text) {
        Ok(q) => q,
        Err(e) => {
            report.errors.push(format!(
                "{file}:1: {e} — fix: a list of {{ question, skill, reaches, shape }}"
            ));
            return;
        }
    };
    let skill_text = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let commands: Vec<(String, Value)> = ext
        .providers
        .iter()
        .filter_map(|spec| {
            providers::spec::read_declarations(spec, &|f| std::fs::read_to_string(dir.join(f)).ok())
                .ok()
                .map(|d| {
                    d.commands
                        .into_iter()
                        .map(|c| (format!("{}.{}", spec.id, c.name), c.input_schema))
                        .collect::<Vec<_>>()
                })
        })
        .flatten()
        .collect();
    let command_schema = |name: &str| {
        commands
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s.clone())
            .or_else(|| host.svc.commands.spec(name).map(|s| s.input_schema))
    };
    let tools: std::collections::BTreeMap<String, Value> = oxplow_app::agent_tools::offered(
        &host.svc,
        &oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        },
    )
    .await
    .map(|tools| {
        tools
            .into_iter()
            .map(|t| (t.name, t.input_schema))
            .collect()
    })
    .unwrap_or_default();
    let tool_schema = |name: &str| tools.get(name).cloned();
    let checked = crate::answerability::check(
        &file,
        &questions,
        &crate::answerability::Checker {
            skill_text: &skill_text,
            sql: Some(&host.svc.sql),
            command_schema: &command_schema,
            tool_schema: &tool_schema,
        },
    )
    .await;
    report.errors.extend(checked.errors);
    report.warnings.extend(checked.warnings);
}
