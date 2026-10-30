use super::*;

/// Where `got` first differs from what `want` (an example's `expect`, or
/// a part of it) says, as a JSON pointer with the two values. The one rule
/// for every expectation: an object compares only the keys it names (one
/// `got` lacks matches only `null`, not even `$any`); a list compares every item, in order;
/// `{ contains: [..] }` holds against a list having each item (compared
/// the same way) in any order, among others; `$any` matches anything;
/// `{ $exact: v }` compares the whole value, every key. `refused: <text>`
/// holds when the reason it said no contains that text.
pub(super) fn compare(want: &Value, got: &Value) -> Option<(String, Value, Value)> {
    fn walk(path: &str, want: &Value, got: &Value) -> Option<(String, Value, Value)> {
        let differs = || {
            Some((
                if path.is_empty() { "/" } else { path }.to_string(),
                want.clone(),
                got.clone(),
            ))
        };
        match want {
            Value::String(any) if any == ANY => None,
            Value::Object(o) if o.len() == 1 && o.contains_key("$exact") => {
                first_mismatch(&o["$exact"], got).map(|(at, w, g)| {
                    let at = if at == "/" { String::new() } else { at };
                    (format!("{path}{at}"), w, g)
                })
            }
            Value::Object(o) if o.len() == 1 && o.contains_key("contains") && got.is_array() => {
                let items = got.as_array().expect("a list");
                o["contains"].as_array().map_or_else(differs, |wanted| {
                    wanted.iter().enumerate().find_map(|(i, item)| {
                        (!items.iter().any(|g| walk("", item, g).is_none()))
                            .then(|| (format!("{path}/contains/{i}"), item.clone(), got.clone()))
                    })
                })
            }
            Value::Object(o) => match got {
                Value::Object(g) => o.iter().find_map(|(k, w)| {
                    let at = format!("{path}/{k}");
                    match g.get(k) {
                        // Why it said no: any reason holding that text.
                        Some(Value::String(why))
                            if k == "refused" && w.as_str().is_some_and(|w| why.contains(w)) =>
                        {
                            None
                        }
                        Some(g) => walk(&at, w, g),
                        None if w.is_null() => None,
                        None => Some((at, w.clone(), Value::Null)),
                    }
                }),
                _ => differs(),
            },
            Value::Array(w) => match got {
                Value::Array(g) if g.len() == w.len() => w
                    .iter()
                    .zip(g)
                    .enumerate()
                    .find_map(|(i, (w, g))| walk(&format!("{path}/{i}"), w, g)),
                _ => differs(),
            },
            // A number is its value: a fixture's 2.0 is a script's 2.
            Value::Number(w) if got.as_f64().is_some_and(|g| w.as_f64() == Some(g)) => None,
            _ if want == got => None,
            _ => differs(),
        }
    }
    walk("", want, got)
}

/// `steps` (the example's `key`, `setup` or `given`), run in order on the
/// throwaway's own database: each `{ command, input?, as?, confirmed?,
/// bind? }` or, for a `given`, `{ rows: { <entity>: [ … ] } }`. A step's
/// input may use what an earlier one bound (`$name.path`); `bind: <name>`
/// keeps a command's result for later. `false`, with the error reported,
/// when one fails.
pub(super) async fn run_steps(
    host: &Host,
    ext: &Extension,
    shown: &str,
    key: &str,
    steps: &Value,
    binds: &mut Binds,
    report: &mut TestReport,
) -> bool {
    let Some(steps) = steps.as_array() else {
        report.errors.push(
            error(shown, None, "fixture.step", format!("`{key}` isn't a list"))
                .fix(format!("`{key}: [{{ command, input?, as? }}]`")),
        );
        return false;
    };
    if let Err(e) = host.svc.streams.ensure_primary().await {
        report.errors.push(error(
            shown,
            None,
            "example.thread_failed",
            format!("`{key}`: its thread: {e}"),
        ));
        return false;
    }
    for (index, step) in steps.iter().enumerate() {
        if let Some(rows) = step.get("rows") {
            if !publish_rows(host, ext, shown, rows, report).await {
                return false;
            }
            continue;
        }
        if let Some(project) = step.get("project") {
            if let Err(e) = given_project(host, project) {
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.step",
                    format!("`{key}[{index}]` `project`: {e}"),
                ));
                return false;
            }
            host.settle().await;
            continue;
        }
        if let Some(observed) = step.get("observe") {
            let observed = match binds.substitute(observed) {
                Ok(observed) => observed,
                Err(e) => {
                    report.errors.push(error(
                        shown,
                        None,
                        "fixture.step",
                        format!("`{key}[{index}]`: {e}"),
                    ));
                    return false;
                }
            };
            if let Err(e) = observe(host, ext, &observed).await {
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.step",
                    format!("`{key}[{index}]` `observe`: {e}"),
                ));
                return false;
            }
            host.settle().await;
            continue;
        }
        let Some(command) = step.get("command").and_then(Value::as_str) else {
            report.errors.push(error(shown, None, "fixture.step", format!("`{key}[{index}]` names no command")).fix("`{ command, input?, as? }`, `{ observe: { <kind>: { .. } } }` or `{ rows: { <entity>: [..] } }`"));
            return false;
        };
        let actor = match step_actor(step) {
            Ok(actor) => actor,
            Err(e) => {
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.step",
                    format!("`{key}[{index}]`: {e}"),
                ));
                return false;
            }
        };
        let input = match binds.substitute(&step.get("input").cloned().unwrap_or_else(|| json!({})))
        {
            Ok(input) => input,
            Err(e) => {
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.step",
                    format!("`{key}[{index}]`: {e}"),
                ));
                return false;
            }
        };
        let confirmed = step
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match host
            .svc
            .commands
            .run(&actor, command, input, confirmed)
            .await
        {
            Ok(outcome) => {
                if let Some(name) = step.get("bind").and_then(Value::as_str) {
                    binds.0.insert(name.to_string(), outcome.result);
                }
                // What it set off — a reactor hearing its event, the
                // generation it asked for — happens before the next step.
                host.settle().await;
            }
            Err(e) => {
                report.errors.push(
                    error(
                        shown,
                        None,
                        "fixture.step",
                        format!("`{key}[{index}]` `{command}` failed: {e}"),
                    )
                    .fix(format!(
                        "its input, or the steps before it{}",
                        if confirmed {
                            ""
                        } else {
                            " (a command that asks first runs with `confirmed: true`)"
                        }
                    )),
                );
                return false;
            }
        }
    }
    true
}

/// Who runs a step's command (`as:`): a person, or `agent` (the primary
/// thread's).
pub(super) fn step_actor(step: &Value) -> Result<Actor, String> {
    match step.get("as").and_then(Value::as_str) {
        None | Some("person") => Ok(Actor::Human),
        Some("agent") => Ok(Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(1)),
            stream_id: None,
        }),
        Some(other) => Err(format!("`as: {other}` — fix: `person` or `agent`")),
    }
}

/// Values a fixture bound (`bind:`), by name: a step's command result, or
/// a value of a page run. `$name` followed by `.key` and `[n]` parts reads
/// one; `$any` is never a name.
#[derive(Default)]
pub(super) struct Binds(pub(super) std::collections::BTreeMap<String, Value>);

impl Binds {
    /// `value` with every string that is exactly a reference replaced by
    /// what it reads, its type kept.
    pub(super) fn substitute(&self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::String(text) => match reference(text) {
                Some(reference) => self.resolve(reference)?,
                None => value.clone(),
            },
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.substitute(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), self.substitute(v)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }

    /// What `name.path` reads.
    pub(super) fn resolve(&self, reference: &str) -> Result<Value, String> {
        let end = reference.find(['.', '[']).unwrap_or(reference.len());
        let (name, path) = reference.split_at(end);
        let bound = self.0.get(name).ok_or_else(|| {
            format!("`${name}` isn't bound — fix: `bind: {name}` on an earlier step")
        })?;
        read_path(bound, path).ok_or_else(|| format!("`${reference}` reads nothing"))
    }
}

/// The name and path a string refers to (`$first.uri` → `first.uri`):
/// `$` and a letter or `_`, and not `$any`.
pub(super) fn reference(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('$')?;
    (text != crate::conformance::ANY
        && rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
    .then_some(rest)
}

/// The value at `path` (`.key` and `[n]` parts, or none) in `value`.
pub(super) fn read_path(value: &Value, path: &str) -> Option<Value> {
    let mut at = value;
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let close = after.find(']')?;
            at = at.get(after[..close].parse::<usize>().ok()?)?;
            rest = &after[close + 1..];
        } else {
            let after = rest.strip_prefix('.').unwrap_or(rest);
            let end = after.find(['.', '[']).unwrap_or(after.len());
            at = at.get(&after[..end])?;
            rest = &after[end..];
        }
    }
    Some(at.clone())
}

/// A run as a fixture's `bind` reads it: each loader's rows as maps by
/// column, and its alert, badge and what keeps it inactive.
pub(super) fn run_json(run: &oxplow_app::pages::runner::PageRun) -> Value {
    let loaders: serde_json::Map<String, Value> = run
        .loaders
        .iter()
        .map(|(name, loader)| {
            let rows: Vec<Value> = loader
                .result
                .rows
                .iter()
                .map(|row| Value::Object(loader.result.json_row(row)))
                .collect();
            (
                name.clone(),
                json!({ "rows": rows, "columns": loader.result.columns }),
            )
        })
        .collect();
    json!({ "loaders": loaders, "alert": run.alert, "badge": run.badge, "inactive": run.inactive })
}

/// Keep what a step's `bind: { <name>: <path> }` reads in `seen` (a run,
/// or what a press answered). False, with the finding, when a path reads
/// nothing.
pub(super) fn bind_from(
    step: &Value,
    seen: &Value,
    binds: &mut Binds,
    shown: &str,
    label: &str,
    report: &mut TestReport,
) -> bool {
    for (name, path) in step
        .get("bind")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        match path
            .as_str()
            .and_then(|path| read_path(seen, &format!(".{path}")))
        {
            Some(value) => {
                binds.0.insert(name.clone(), value);
            }
            None => {
                report.errors.push(error(shown, None, "fixture.step", format!("{label}: `bind.{name}` reads nothing in {seen}")).fix("a path like `loaders.<name>.rows[0].<column>` (a run) or `result.<key>` (a press)"));
                return false;
            }
        }
    }
    true
}
