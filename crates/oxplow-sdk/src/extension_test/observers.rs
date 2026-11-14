use super::*;
use oxplow_app::collector_runner::{FileChange, RunCollectorError, RunTrigger};
use oxplow_config::collectors::ObserverSource;

/// Where a files step's paths are: the project, or with `root:` (`home`,
/// `agent:<harness id>`) that root's folder in the test's oxplow — never
/// the person's own.
pub(super) fn root_base(host: &Host, root: Option<&Value>) -> Result<std::path::PathBuf, String> {
    let fix = "`root: home` or `root: \"agent:claude\"` (the project's when left out)";
    let root = match root {
        None => return Ok(host.root.clone()),
        Some(Value::String(root)) => root
            .parse::<oxplow_domain::folder_root::FolderRoot>()
            .map_err(|e| format!("{e} — fix: {fix}"))?,
        Some(_) => return Err(format!("`root` names a folder root — fix: {fix}")),
    };
    host.svc
        .folder_base(&root)
        .ok_or_else(|| format!("`{root}` has no folder in a test's oxplow"))
}

/// A path under `base` (the project, or a root's folder) the fixture may
/// write: relative, inside it.
fn project_path(base: &std::path::Path, rel: &str) -> Result<std::path::PathBuf, String> {
    let inside = !rel.starts_with('/')
        && !rel.contains('\\')
        && !rel.split('/').next().is_some_and(|first| first == ".git")
        && oxplow_domain::service::valid_path(rel);
    if inside {
        Ok(base.join(rel))
    } else {
        Err(format!(
            "`{rel}` isn't a path inside its root — fix: relative to the project root (or the \
             step's `root`), no `..`"
        ))
    }
}

/// `files: { "<path>": "<body>" }`: files written under `host` (the
/// project, or the step's `root`), folders made.
pub(super) fn given_files(host: &std::path::Path, files: &Value) -> Result<(), String> {
    let files = files
        .as_object()
        .ok_or("`files` maps each path to its text — fix: `files: { \"shelf/a.md\": \"# A\" }`")?;
    for (rel, body) in files {
        let body = body
            .as_str()
            .ok_or_else(|| format!("`files.{rel}` is the file's text"))?;
        let path = project_path(host, rel)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("`{rel}`: {e}"))?;
        }
        std::fs::write(&path, body).map_err(|e| format!("`{rel}`: {e}"))?;
    }
    Ok(())
}

/// `store_sql: { extension, sql, params? }`: one statement run on that
/// extension's own SQLite file from outside it, as a person's sqlite3 or
/// `oxplow-dev` would — no command, no run, no event; a collector that
/// follows the file (`on: [{ sqlite: own }]`) sees it on its next `sqlite`
/// trigger. The extension must be loaded with `storage: { sqlite: true }`
/// (the fixture's own, or one it requires); its migrations are run first
/// when the file isn't made yet, as its first use would.
pub(super) fn given_store_sql(host: &Host, spec: &Value) -> Result<(), String> {
    let fix = "`store_sql: { extension: <name>, sql: \"INSERT …\", params: [..] }`";
    let text = |key: &str| spec.get(key).and_then(Value::as_str);
    let (Some(extension), Some(sql)) = (text("extension"), text("sql")) else {
        return Err(format!("names `extension` and `sql` — fix: {fix}"));
    };
    let params = match spec.get("params") {
        None => Vec::new(),
        Some(Value::Array(params)) => params.clone(),
        Some(_) => return Err(format!("`params` is a list — fix: {fix}")),
    };
    let catalog = host.svc.extension_catalog.get(&host.root);
    let Some(loaded) = catalog
        .iter()
        .find(|e| e.name == extension && e.errors.is_empty())
    else {
        return Err(format!(
            "no loaded extension `{extension}` has a SQLite file — fix: the fixture's own \
             extension, or one it `requires:`"
        ));
    };
    if !loaded.storage.sqlite {
        return Err(format!(
            "`{extension}` has no SQLite file — fix: declare `storage: {{ sqlite: true }}` in it"
        ));
    }
    let path = oxplow_app::extension::store_path(&host.root, extension);
    let store = oxplow_db::extension_store::ExtensionStore::open(&path)
        .map_err(|e| format!("`{extension}`: {e}"))?;
    store
        .migrate(&loaded.storage.migrations)
        .map_err(|e| format!("`{extension}`: {e}"))?;
    store
        .write(&[oxplow_db::extension_store::Statement {
            sql: sql.to_string(),
            params,
            expect_changes: None,
        }])
        .map(|_| ())
        .map_err(|e| format!("`{extension}`: {e}"))
}

/// `remove_files: ["<path>"]`: project files removed; one that isn't there is
/// an error (a fixture removes what it made).
pub(super) fn given_remove_files(host: &std::path::Path, files: &Value) -> Result<(), String> {
    let files = files
        .as_array()
        .ok_or("`remove_files` lists paths — fix: `remove_files: [\"shelf/a.md\"]`")?;
    for rel in files {
        let rel = rel.as_str().ok_or("`remove_files` lists paths")?;
        std::fs::remove_file(project_path(host, rel)?).map_err(|e| format!("`{rel}`: {e}"))?;
    }
    Ok(())
}

/// `rename_files: [{ from, to }]`: project files renamed, in order.
pub(super) fn given_rename_files(host: &std::path::Path, files: &Value) -> Result<(), String> {
    let fix = "`rename_files: [{ from: \"shelf/a.md\", to: \"shelf/b.md\" }]`";
    let files = files
        .as_array()
        .ok_or_else(|| format!("`rename_files` lists renames — fix: {fix}"))?;
    for rename in files {
        let named = |key: &str| rename.get(key).and_then(Value::as_str);
        let (Some(from), Some(to)) = (named("from"), named("to")) else {
            return Err(format!("a rename names `from` and `to` — fix: {fix}"));
        };
        let to_path = project_path(host, to)?;
        if let Some(dir) = to_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("`{to}`: {e}"))?;
        }
        std::fs::rename(project_path(host, from)?, to_path)
            .map_err(|e| format!("`{from}` → `{to}`: {e}"))?;
    }
    Ok(())
}

/// `trigger: start | sqlite | { files: [{ path, op }] }`.
fn trigger_of(trigger: &Value) -> Result<RunTrigger, String> {
    let fix = "`trigger: start`, `trigger: sqlite` or `trigger: { files: [{ path, op: created | modified | removed }] }`";
    match trigger {
        Value::String(s) if s == "start" => Ok(RunTrigger::Start),
        Value::String(s) if s == "sqlite" => Ok(RunTrigger::Sqlite),
        Value::Object(o) if o.len() == 1 && o.contains_key("files") => {
            let changed: Vec<FileChange> = serde_json::from_value(o["files"].clone())
                .map_err(|e| format!("`trigger.files` lists changes: {e} — fix: {fix}"))?;
            Ok(RunTrigger::Files(changed))
        }
        other => Err(format!("trigger {other} isn't one of — fix: {fix}")),
    }
}

/// Whether the collector's `on:` lists what `trigger` is.
fn listens_to(
    spec: &oxplow_config::collectors::CollectorSpec,
    trigger: &RunTrigger,
) -> Result<(), String> {
    let sources = spec.trigger.sources();
    match trigger {
        RunTrigger::Start if !sources.contains(&ObserverSource::Start) => {
            Err("it doesn't run on `start`".into())
        }
        RunTrigger::Sqlite
            if !sources
                .iter()
                .any(|s| matches!(s, ObserverSource::Sqlite { .. })) =>
        {
            Err("it doesn't run on a `sqlite` trigger".into())
        }
        RunTrigger::Files(changed) => {
            let globs: Vec<String> = sources
                .iter()
                .filter_map(|s| match s {
                    ObserverSource::Files { glob, .. } => Some(glob.clone()),
                    _ => None,
                })
                .collect();
            if globs.is_empty() {
                return Err("it doesn't run on a `files` trigger".into());
            }
            let set = oxplow_config::collectors::files_matcher(&globs)
                .map_err(|e| format!("its globs: {e}"))?;
            match changed.iter().find(|c| !set.is_match(&c.path)) {
                Some(c) => Err(format!(
                    "`{}` isn't matched by its `files:` globs ({})",
                    c.path,
                    globs.join(", ")
                )),
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// `when: { collector: <id>, trigger: … }`: the collector run for real as
/// that trigger fires, its output applied (rows, models, cursor), checked
/// against `expect: { ok?, result?, problems? }`: `ok` (default true) that it
/// ran well, `result` the counts it stored (an entity's rows, `<model>.<op>`
/// for shared models) on the keys named, `problems` text its problems hold
/// (a failure's, or the rows an ok run quarantined; `[]` says none). Live triggers are off in a fixture, but a store command's own
/// write is observed when the command runs; any other run needs a step.
pub(super) async fn observer_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    // `<namespace>/<id>` names a collector of another enabled extension
    // (a store the fixture's extension requires); a bare id is its own.
    let catalog = host.svc.extension_catalog.get(&host.root);
    let (ext, id) = match id.split_once('/') {
        Some((namespace, local)) => (
            catalog
                .iter()
                .find(|e| e.namespace == namespace && e.errors.is_empty())
                .unwrap_or(ext),
            local,
        ),
        None => (ext, id),
    };
    let Some(spec) = ext.collectors.iter().find(|c| c.id == id) else {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!(
                    "example `{}` names collector `{id}`, which `{}` doesn't declare",
                    ex.name, ext.name
                ),
            )
            .fix("the collector's `id`, or `<namespace>/<id>` for a required extension's"),
        );
        return;
    };
    let trigger =
        match trigger_of(&ex.when["trigger"]).and_then(|t| listens_to(spec, &t).map(|()| t)) {
            Ok(t) => t,
            Err(why) => {
                report.errors.push(
                    error(
                        shown,
                        None,
                        "fixture.trigger",
                        format!("collector `{id}`: {why}"),
                    )
                    .fix("a trigger its `on:` lists"),
                );
                return;
            }
        };
    let ctx = oxplow_app::collector_runner::Collectors::of(&host.svc, &host.root);
    let parent = oxplow_domain::run::RunContext::core(
        oxplow_domain::Actor::System,
        oxplow_domain::run::Mode::External,
    );
    let ran =
        oxplow_app::collector_runner::run_collector(&ctx, &parent, &ext.namespace, id, trigger)
            .await;
    host.settle().await;
    let got = match ran {
        Ok(done) => json!({ "ok": true, "result": done.row_counts, "problems": done.problems }),
        Err(RunCollectorError::NotFound) => {
            report.errors.push(error(
                shown,
                None,
                "collector.failed",
                format!("collector `{id}` isn't loaded"),
            ));
            return;
        }
        Err(RunCollectorError::Storage(e)) => {
            report.errors.push(error(
                shown,
                None,
                "collector.failed",
                format!("collector `{id}`: {e}"),
            ));
            return;
        }
        Err(
            RunCollectorError::Denied(m)
            | RunCollectorError::Failed(m)
            | RunCollectorError::Disabled(m),
        ) => {
            json!({ "ok": false, "problems": [m] })
        }
    };
    let mut want = ex.expect.clone();
    if want.get("ok").is_none() {
        want["ok"] = json!(true);
    }
    // `problems` are texts the run's problems hold, not whole messages: the
    // failure's, or what an ok run quarantined. An empty list says none.
    let wanted_problems = want
        .as_object_mut()
        .and_then(|o| o.remove("problems"))
        .and_then(|p| p.as_array().cloned());
    let said = got["problems"].to_string();
    if wanted_problems.as_ref().is_some_and(Vec::is_empty)
        && got["problems"].as_array().is_some_and(|p| !p.is_empty())
    {
        report.errors.push(
            error(
                shown,
                ex.line("expect"),
                "fixture.compare",
                format!("collector `{id}`'s problems are {said}, the example expects none"),
            )
            .fix("the script, or the example's `expect.problems`"),
        );
    }
    if let Some(missing) = wanted_problems
        .iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|text| !said.contains(text))
    {
        report.errors.push(
            error(
                shown,
                ex.line("expect"),
                "fixture.compare",
                format!("collector `{id}`'s problems are {said}, the example expects one holding `{missing}`"),
            )
            .fix("the script, or the example's `expect.problems`"),
        );
    }
    if let Some((path, want, got)) = compare(&want, &got) {
        report.errors.push(
            error(
                shown,
                ex.line("expect"),
                "fixture.compare",
                format!("collector `{id}` returned {got} at `{path}`, the example expects {want}"),
            )
            .fix("the script, or the example's `expect` (`{ ok?, result?, problems? }`)")
            .mismatch(path, want, got),
        );
    }
}
