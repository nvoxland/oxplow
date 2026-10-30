use super::*;

/// An intent example's fixture (`fixtures/<name>.yaml`): `None` when it
/// has none; the parsed document, or why it isn't one.
pub(super) fn example_fixture(
    read: &(dyn Fn(&str) -> Option<String> + Sync),
    rel: &str,
    example: &str,
) -> Option<(String, Result<Value, String>)> {
    let file = format!("fixtures/{example}.yaml");
    let shown = format!("{rel}/{file}");
    let text = read(&file)?;
    let doc = serde_yaml::from_str::<Value>(&text)
        .map_err(|e| format!("{shown}:1: not YAML: {e} — fix: correct it"));
    Some((shown, doc))
}

/// The keys of a fixture file.
pub(super) const FIXTURE_KEYS: [&str; 8] = [
    "requires", "given", "when", "expect", "steps", "view", "model", "temp",
];

/// An example's fixture, owned, so it can run on a task of its own.
pub(super) struct Fixture {
    pub(super) name: String,
    /// The fixture file, as findings name it.
    pub(super) shown: String,
    pub(super) when: Value,
    pub(super) expect: Value,
    pub(super) given: Value,
    pub(super) steps: Option<Value>,
    pub(super) view: Value,
    pub(super) model: Value,
    pub(super) temp: Value,
    /// The extensions it turns on beyond the one tested.
    pub(super) requires: Vec<String>,
}

impl Fixture {
    pub(super) fn of(name: String, shown: String, doc: &Value) -> Fixture {
        Fixture {
            name,
            shown,
            when: doc.get("when").cloned().unwrap_or(Value::Null),
            expect: doc.get("expect").cloned().unwrap_or(Value::Null),
            given: doc.get("given").cloned().unwrap_or(Value::Null),
            steps: doc.get("steps").cloned(),
            view: doc.get("view").cloned().unwrap_or(Value::Null),
            model: doc.get("model").cloned().unwrap_or(Value::Null),
            temp: doc.get("temp").cloned().unwrap_or(Value::Null),
            requires: doc
                .get("requires")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect(),
        }
    }

    pub(super) fn example(&self) -> Example<'_> {
        Example {
            name: &self.name,
            shown: &self.shown,
            when: self.when.clone(),
            expect: self.expect.clone(),
            given: self.given.clone(),
            steps: self.steps.clone(),
            view: self.view.clone(),
            model: self.model.clone(),
            temp: self.temp.clone(),
        }
    }
}

/// The example's `temp: { <name>: <text> }`: each written to `oxplow:temp`
/// (a scratch file a page reads as `oxplow:oxplow/file?path=<name>&filesystem=oxplow:temp`).
/// `false`, with the error reported, when it isn't that shape or a name is
/// refused.
pub(super) async fn seed_temp(host: &Host, ex: &Example<'_>, report: &mut TestReport) -> bool {
    if ex.temp.is_null() {
        return true;
    }
    let files = match serde_json::from_value::<std::collections::BTreeMap<String, String>>(
        ex.temp.clone(),
    ) {
        Ok(files) => files,
        Err(e) => {
            report.errors.push(format!(
                "{}:1: `temp` maps each file name to its text — fix: `temp: {{ view-1.yaml: \"title: One\" }}` ({e})",
                ex.shown
            ));
            return false;
        }
    };
    for (name, text) in files {
        if let Err(e) = host.svc.temp.put(&name, text.as_bytes()).await {
            report
                .errors
                .push(format!("{}:1: `temp.{name}`: {e}", ex.shown));
            return false;
        }
    }
    true
}

/// The example's `rows: { <entity>: [ … ] }`, published for the
/// extension's own entities in place of what a collector would have
/// brought. `false`, with the error reported, when they couldn't be.
pub(super) async fn seed_rows(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> bool {
    match ex.when.get("rows") {
        Some(rows) => publish_rows(host, ext, ex.shown, rows, report).await,
        None => true,
    }
}

/// `rows: { <entity>: [ … ] }` published for the extension's own entities.
pub(super) async fn publish_rows(
    host: &Host,
    ext: &Extension,
    shown: &str,
    rows: &Value,
    report: &mut TestReport,
) -> bool {
    let rows = match serde_json::from_value::<std::collections::BTreeMap<String, Vec<Value>>>(
        rows.clone(),
    ) {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(format!(
                "{shown}:1: `rows` must map each entity to its rows: {e}"
            ));
            return false;
        }
    };
    if let Err(e) =
        oxplow_app::collector_runner::publish_declared_rows(&host.svc.db, ext, &rows).await
    {
        report.errors.push(format!("{shown}:1: `rows`: {e}"));
        return false;
    }
    true
}

/// The example's `setup: [{ command, input?, as? }]`: oxplow's own
/// commands, run in order before the example, to make the data its
/// contribution reads (a work item, an effort, a test run). Each runs as a
/// person, or with `as: agent` as the agent of `oxplow:oxplow/thread?id=thr1`. `false`,
/// with the error reported, when one fails.
pub(super) async fn run_setup(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> bool {
    match ex.when.get("setup") {
        Some(steps) => {
            let mut binds = Binds::default();
            run_steps(host, ext, ex.shown, "setup", steps, &mut binds, report).await
        }
        None => true,
    }
}

/// `project: { <key>: <value>, .. }`: keys of `.oxplow/project.yaml` set on
/// the throwaway's root, as a person's edit of the file, and the config read
/// again from disk, as core does when the file changes.
pub(super) fn given_project(host: &Host, project: &Value) -> Result<(), String> {
    let Some(keys) = project.as_object() else {
        return Err(
            "`project` is a map of project.yaml keys — fix: `project: { lspServers: [..] }`".into(),
        );
    };
    let file = host.root.join(".oxplow/project.yaml");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut doc: serde_yaml::Mapping = serde_yaml::from_str(&text).unwrap_or_default();
    for (key, value) in keys {
        doc.insert(
            serde_yaml::Value::String(key.clone()),
            serde_yaml::to_value(value).map_err(|e| e.to_string())?,
        );
    }
    std::fs::write(
        &file,
        serde_yaml::to_string(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    host.svc
        .reload_config_from_disk()
        .map_err(|e| e.to_string())
}

/// One intent example with its fixture.
#[derive(Clone)]
pub(super) struct Example<'a> {
    pub(super) name: &'a str,
    /// Its fixture file, as findings name it.
    pub(super) shown: &'a str,
    /// What it does: one key names the kind (`page`, `command`, `event`,
    /// …), the rest are that kind's.
    pub(super) when: Value,
    pub(super) expect: Value,
    /// Steps run before it (`given:`).
    pub(super) given: Value,
    /// A page example's scenario (`steps:`): `{ given }` changes the state,
    /// `{ expect, params?, bind? }` runs the page again and checks it.
    pub(super) steps: Option<Value>,
    /// The stream and thread a page is viewed from (`view: { stream,
    /// thread }`).
    pub(super) view: Value,
    /// What the fixture model answers its model calls with (`model: {
    /// replies }`).
    pub(super) model: Value,
    /// Files on `oxplow:temp`, made before the example runs (`temp: { <name>:
    /// <text> }`).
    pub(super) temp: Value,
}
