//! `oxplow-dev`: helpers for working on oxplow itself, never shipped.
//!
//! `oxplow-dev task …` works over oxtask's own file (`oxtask_file.rs`)
//! when the project has the oxtask extension enabled; without it there is
//! no task tracker for it to reach. Nothing is migrated or recompiled under
//! the running app. See `.context/working-in-this-repo.md` "oxplow-dev".

use std::path::{Path, PathBuf};
use std::sync::Arc;

mod oxtask_file;

use oxplow_db::Database;

const USAGE: &str = "\
oxplow-dev task list [--all]                 open tasks (--all: every state)
oxplow-dev task show <id>
oxplow-dev task create <title> [--body B] [--parent ID] [--state todo|in_progress]
oxplow-dev task transition <id> <todo|in_progress|blocked|done|canceled>
oxplow-dev task update <id> [--title T] [--body B] [--parent ID]
oxplow-dev task comment <id> <body>
oxplow-dev task link <id> <target> <blocks|discovered_from|relates_to|duplicates|supersedes|replies_to|about>
oxplow-dev task unlink <id> <target> <link type>

oxplow-dev extension status                  what the next boot would run, and what it
                                             would hold back and why (safe while oxplow runs)
oxplow-dev extension deploy [<name>]         deploy <name>'s source as the app does,
                                             confirming its permissions and build as you;
                                             with no name, every extension that is on and
                                             whose source changed (foundation first): the
                                             first run of a checkout, or after a core change;
                                             only while oxplow is quit
oxplow-dev extension test [<name> …] [--only <example,…>] [--require-ui] [--junit F]
                                             run each extension's tests (check, fixtures,
                                             page answers) on a throwaway oxplow — no
                                             compile for an extension-only change, safe
                                             while oxplow runs; no name: every extension
                                             with a source here; --only: the check and
                                             just those examples (fixture file names);
                                             --require-ui: `ui` steps that can't draw
                                             fail, not skip;
                                             --junit writes JUnit to F

oxplow-dev --help                            this

<id> is a task's id (tsk12) or ref (task:oxplow:tsk12); a link's
<target> is a task too, or any ref the task is `about` (collector:x/y). The project is
the current directory's (or --project <dir>); the acting thread is
--thread <thrN>, else $OXPLOW_THREAD_ID, else none (as a person).";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args, std::env::var("OXPLOW_THREAD_ID").ok()).await {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("oxplow-dev: {e}");
            std::process::exit(1);
        }
    }
}

/// Run `args`, acting as `--thread`, else `env_thread` (main passes
/// `$OXPLOW_THREAD_ID`; read there, not here, so a test isn't acting as
/// whatever thread its shell runs in).
async fn run(mut args: Vec<String>, env_thread: Option<String>) -> Result<String, String> {
    let project = take_flag(&mut args, "--project")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    let thread = take_flag(&mut args, "--thread").or(env_thread);
    // Asking how to call it is answered before anything opens the
    // database — at any schema version — and never taken as an argument
    // (it once filed tasks titled `--help`).
    if args.first().is_some_and(|a| a == "help") || args.iter().any(|a| a == "--help" || a == "-h")
    {
        return Ok(USAGE.to_string());
    }
    match args.first().map(String::as_str) {
        Some("task") => {
            // The tasks are oxtask's file's.
            if !oxplow_config::extension_settings(&project).is_enabled("oxtask") {
                return Err(
                    "there is no task tracker to reach: the oxtask extension is off in this \
                     project (turn it on in Settings → Extensions)"
                        .to_string(),
                );
            }
            let file = oxtask_file::OxtaskFile::open(&project, thread.as_deref())?;
            file.run(&args[1..], USAGE)
        }
        Some("extension") => extension(&project, &args[1..]).await,
        _ => Err(format!("usage:\n{USAGE}")),
    }
}

/// Deploy from outside oxplow, as the app does: `name`'s source, or with
/// none every extension that is on (oxplow's required ones first) whose
/// source isn't what runs. Boot deploys nothing, so this is how a checkout
/// gets its first deployments, and how oxplow-foundation comes back after
/// a core change its recorded deployment doesn't activate against.
async fn deploy_from_outside(project: &Path, name: Option<&String>) -> Result<String, String> {
    let layout = oxplow_app::AppLayout::for_project(project);
    let running = "oxplow is running on this project: quit it first, or deploy from \
                   inside it (`oxplow.extension.deploy`) if its foundation runs";
    if oxplow_app::is_project_locked(project) {
        return Err(running.into());
    }
    // Held while this boots and deploys, as a daemon holds it.
    let _lock = oxplow_app::try_acquire_instance_lock(&layout)
        .map_err(|e| e.to_string())?
        .ok_or(running)?;
    let svc = Arc::new(
        oxplow_app::Services::boot(
            layout,
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .map_err(|e| e.to_string())?,
    );
    // Startup, as a boot runs it: the live set it starts from is what
    // the deploys replace.
    svc.extension_activation
        .initialize(svc.clone())
        .await
        .map_err(|e| e.to_string())?;
    let required = |n: &str| {
        oxplow_app::bundled_extensions::BUNDLED
            .iter()
            .any(|b| b.required && b.name == n)
    };
    let name_given = name;
    let names = match name {
        Some(name) => vec![name.clone()],
        None => {
            // Each with a source here: a project that runs the copies
            // built into oxplow has no source for them, and nothing to do.
            let settings = oxplow_config::extension_settings(project);
            let mut names: Vec<String> = oxplow_app::bundled_extensions::BUNDLED
                .iter()
                .filter(|b| b.required)
                .map(|b| b.name.to_string())
                .collect();
            names.extend(settings.enabled().into_iter().map(|(n, _)| n));
            let mut seen = std::collections::BTreeSet::new();
            names.retain(|n| {
                seen.insert(n.clone())
                    && settings.source(project, n).join("extension.yaml").is_file()
            });
            names
        }
    };
    let mut out = Vec::new();
    let mut failed = Vec::new();
    // One change to what runs: the set is checked against itself.
    let results =
        oxplow_app::extension::extension_deploy::deploy_together(&svc, &names, true).await;
    for (name, result) in &results {
        let name = name.as_str();
        match result {
            Ok(deployed) => {
                out.push(format!(
                    "{name}: {} {}{}",
                    if deployed.already {
                        "already runs"
                    } else {
                        "deployed"
                    },
                    deployed.hash,
                    deployed
                        .provenance()
                        .map(|p| format!(" ({p})"))
                        .unwrap_or_default()
                ));
                out.extend(deployed.warnings.iter().map(|w| format!("  warning: {w}")));
            }
            // With no name, one that is on but doesn't deploy is held
            // back, as boot holds it, so the rest still runs; a required
            // one, or the one named, fails the deploy.
            Err(e) if name_given.is_none() && !required(name) => {
                out.push(format!("held back: {name}"));
                out.push(format!("  - {e}"));
            }
            Err(e) => failed.push(format!("deploy `{name}`: {e}")),
        }
    }
    out.extend(held_back(&svc.extension_activation.problems(), project));
    if failed.is_empty() {
        Ok(out.join("\n"))
    } else {
        out.extend(failed);
        Err(out.join("\n"))
    }
}

/// What held each extension back, as lines: `held back: <name>`, then a
/// `  - <file:line: what — fix>` per problem, each under the folder of the
/// extension it belongs to (the one a person edits).
fn held_back(
    problems: &oxplow_app::vocabulary_reactor::VocabularyErrors,
    project: &Path,
) -> Vec<String> {
    let extensions = oxplow_app::extension::extensions::load_extensions(project);
    problems
        .iter()
        .flat_map(|(owner, why)| {
            let root = extensions
                .iter()
                .find(|e| &e.name == owner)
                .map_or("", |e| e.root());
            std::iter::once(format!("held back: {owner}"))
                .chain(why.iter().map(move |d| format!("  - {}", d.render(root))))
        })
        .collect()
}

/// `extension status` and `extension deploy <name>`: an agent outside
/// oxplow finding and fixing what the next boot would run — a deployment
/// an older oxplow recorded that no longer activates (foundation's, while
/// developing oxplow, takes the agents and Alerts with it), or a source
/// changed and never deployed.
async fn extension(project: &Path, args: &[String]) -> Result<String, String> {
    match args {
        [status] if status == "status" => {
            let layout = oxplow_app::AppLayout::for_project(project);
            let db = Database::open_existing(&layout.state_db_path)
                .map_err(|e| format!("{}: {e}", layout.state_db_path.display()))?;
            let rows = db
                .read(|c| oxplow_db::extension_deployment_store::list_tx(c, None))
                .await
                .map_err(|e| e.to_string())?;
            let next = oxplow_app::extension::extension_deploy::next_boot(project, &rows).await?;
            let mut out: Vec<String> = next
                .runs
                .iter()
                .map(|(name, what)| format!("{name}: {what}"))
                .collect();
            for line in held_back(&next.problems, project) {
                out.push(match line.strip_prefix("held back: ") {
                    Some(_) => format!("\n{line}"),
                    None => line,
                });
            }
            if next.problems.is_empty() {
                out.push("\nnothing held back".into());
            }
            Ok(out.join("\n"))
        }
        [test, rest @ ..] if test == "test" => extension_test(project, rest).await,
        [deploy, name] if deploy == "deploy" => deploy_from_outside(project, Some(name)).await,
        [deploy] if deploy == "deploy" => deploy_from_outside(project, None).await,
        _ => Err(format!("usage:\n{USAGE}")),
    }
}

/// `extension test [<name> …] [--only <example,…>] [--require-ui] [--junit <file>]`: each
/// named extension's tests (`oxplow extension test`'s), else every extension
/// with a source here; with `--only`, the check and just those examples. The
/// findings as text, and with `--junit` one JUnit file over all of them. Any
/// failing extension fails the command.
async fn extension_test(project: &Path, args: &[String]) -> Result<String, String> {
    use oxplow_sdk::extension_test::{render, test_extension_only, test_extension_with};
    let mut args = args.to_vec();
    let junit = take_flag(&mut args, "--junit");
    let require_ui = take_switch(&mut args, "--require-ui");
    let only: Option<Vec<String>> = take_flag(&mut args, "--only")
        .map(|names| names.split(',').map(|n| n.trim().to_string()).collect());
    let names = if args.is_empty() {
        let settings = oxplow_config::extension_settings(project);
        oxplow_app::extension::extensions::known_names(project, &settings)
            .into_iter()
            .filter(|n| settings.source(project, n).join("extension.yaml").is_file())
            .filter(|n| !settings.turned_off(n))
            .collect()
    } else {
        args
    };
    let (mut text, mut suites, mut failed) = (String::new(), String::new(), false);
    for name in &names {
        let tested = match &only {
            Some(only) => test_extension_only(project, name, only, require_ui).await,
            None => test_extension_with(project, name, false, require_ui).await,
        };
        match tested {
            Ok(report) => {
                failed |= !report.ok;
                text.push_str(&render(&report, oxplow_sdk::Format::Text));
                suites.push_str(&render(&report, oxplow_sdk::Format::Junit));
            }
            Err(e) => {
                failed = true;
                text.push_str(&format!("{name}: {e}\n"));
            }
        }
    }
    if let Some(file) = junit {
        let file = project.join(file);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(
            &file,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n{suites}</testsuites>\n"
            ),
        )
        .map_err(|e| format!("{}: {e}", file.display()))?;
    }
    let text = text.trim_end().to_string();
    if failed {
        Err(text)
    } else {
        Ok(text)
    }
}

/// A bare `--name` out of `args`: whether it was there.
fn take_switch(args: &mut Vec<String>, name: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != name);
    args.len() != before
}

/// `--name value` out of `args`, if there.
fn take_flag(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    if i + 1 >= args.len() {
        args.remove(i);
        return None;
    }
    let value = args.remove(i + 1);
    args.remove(i);
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::Value;

    /// A project the app has set up: its database migrated with its
    /// models compiled, and its primary stream.
    async fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let svc = oxplow_app::Services::boot(
            oxplow_app::AppLayout::for_project(dir.path()),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .unwrap();
        svc.streams.ensure_primary().await.unwrap();
        drop(svc);
        dir
    }

    /// Develop oxplow: its extensions run from their sources. Each test is
    /// its own process, so the variable reaches only this test.
    fn from_source() {
        std::env::set_var("OXPLOW_EXTENSIONS_FROM_SOURCE", "true");
    }

    /// Turn oxtask on for the project: the `task` subcommands reach its file.
    fn with_oxtask(dir: &Path) {
        std::fs::write(
            dir.join(".oxplow/local.yaml"),
            "oxplow.extensions.oxtask.enabled: true\n",
        )
        .unwrap();
    }

    async fn dev(dir: &Path, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["--project".to_string(), dir.display().to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        run(all, None).await
    }

    /// Tasks are filed, moved, commented on and listed through the task
    /// system's own commands — audited and logged like the app's.
    #[tokio::test]
    async fn tasks_are_managed_through_the_task_systems_commands() {
        let dir = project().await;
        let p = dir.path();
        with_oxtask(p);
        let created: Value = serde_json::from_str(
            &dev(
                p,
                &["task", "create", "Write the docs", "--body", "All of them"],
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        dev(p, &["task", "create", "Child", "--parent", &id])
            .await
            .unwrap();
        dev(p, &["task", "transition", &id, "in_progress"])
            .await
            .unwrap();
        dev(p, &["task", "comment", &id, "Started."]).await.unwrap();
        let list = dev(p, &["task", "list"]).await.unwrap();
        assert!(
            list.contains(&format!("{id}\tin_progress\tWrite the docs")),
            "{list}"
        );
        dev(p, &["task", "transition", &id, "done"]).await.unwrap();
        assert!(!dev(p, &["task", "list"])
            .await
            .unwrap()
            .contains("Write the docs"));
        assert!(dev(p, &["task", "list", "--all"])
            .await
            .unwrap()
            .contains("Write the docs"));
        let shown = dev(p, &["task", "show", &id]).await.unwrap();
        assert!(
            shown.contains("state: done") && shown.contains("All of them"),
            "{shown}"
        );
    }

    /// `--help` anywhere prints how to call, before the database is
    /// opened (so it works at any schema version) — it used to become the
    /// title of a task named `--help`.
    #[tokio::test]
    async fn help_prints_the_usage_and_does_nothing() {
        let nowhere = std::path::Path::new("/nonexistent/oxplow-dev-help");
        for args in [
            &["--help"][..],
            &["-h"],
            &["help"],
            &["task", "--help"],
            &["task", "create", "--help"],
            &["task", "create", "A title", "-h"],
        ] {
            let out = dev(nowhere, args).await.unwrap();
            assert!(out.contains("oxplow-dev task create"), "{args:?}: {out}");
        }
    }

    /// An option the command doesn't take is refused, not filed as a title.
    #[tokio::test]
    async fn an_unknown_option_is_refused() {
        let dir = project().await;
        with_oxtask(dir.path());
        let err = dev(dir.path(), &["task", "create", "--titel", "x"])
            .await
            .unwrap_err();
        assert!(err.contains("unknown option `--titel`"), "{err}");
        let err = dev(dir.path(), &["task", "create", "Real", "--prio", "high"])
            .await
            .unwrap_err();
        assert!(err.contains("unknown option `--prio`"), "{err}");
        let list = dev(dir.path(), &["task", "list", "--all"]).await.unwrap();
        assert!(list.is_empty(), "nothing filed: {list}");
    }

    #[tokio::test]
    async fn a_bad_call_says_how_to_call() {
        let dir = project().await;
        with_oxtask(dir.path());
        let err = dev(dir.path(), &["task", "frobnicate"]).await.unwrap_err();
        assert!(err.contains("usage"), "{err}");
        let err = dev(dir.path(), &["task", "transition", "ENG-1", "done"])
            .await
            .unwrap_err();
        assert!(err.contains("tsk"), "{err}");
    }

    /// A project developing oxplow (`OXPLOW_EXTENSIONS_FROM_SOURCE=true`):
    /// foundation's source as it ships (it has no build step), set up by the app, and an older
    /// foundation recorded as its active deployment that no longer loads.
    async fn developing_oxplow_with_a_stale_foundation() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(p)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let shipped = oxplow_app::bundled_extensions::find("oxplow-foundation").unwrap();
        let write_files = |to: &Path, edit: &dyn Fn(String) -> String| {
            for (rel, bytes) in shipped.files() {
                let file = to.join(rel);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                let text = String::from_utf8_lossy(bytes).into_owned();
                let text = if *rel == "extension.yaml" {
                    edit(text)
                } else {
                    text
                };
                std::fs::write(file, text).unwrap();
            }
        };
        write_files(&p.join(".oxplow/extensions/oxplow-foundation"), &|t| t);
        from_source();
        let svc = oxplow_app::Services::boot(
            oxplow_app::AppLayout::for_project(p),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .unwrap();
        svc.streams.ensure_primary().await.unwrap();
        let hash = "00000000000000ba";
        write_files(
            &oxplow_app::extension::extension_store::deployment_dir(p, "oxplow-foundation", hash),
            &|t| format!("{t}not_a_key: 1\n"),
        );
        let row = oxplow_db::extension_deployment_store::DeploymentRow {
            name: "oxplow-foundation".into(),
            hash: hash.into(),
            deployed_at: "2026-10-19T00:00:00Z".into(),
            commit: None,
            dirty: false,
            source: "extensions/oxplow-foundation".into(),
            active: false,
            permissions: Default::default(),
            build_run: None,
        };
        svc.db
            .transaction(move |tx| {
                oxplow_db::extension_deployment_store::record_tx(tx, &row)?;
                oxplow_db::extension_deployment_store::set_active_tx(
                    tx,
                    &[("oxplow-foundation".into(), hash.into())],
                )
            })
            .await
            .unwrap();
        drop(svc);
        dir
    }

    /// From outside oxplow: `extension status` says a recorded foundation
    /// won't activate against this oxplow and that its source moved on;
    /// `extension deploy` deploys the source as the app would — refused
    /// while oxplow holds the project — and the next boot runs it.
    #[tokio::test]
    async fn a_stale_foundation_is_found_and_deployed_from_outside() {
        if !cfg!(debug_assertions) {
            return;
        }
        let dir = developing_oxplow_with_a_stale_foundation().await;
        let p = dir.path();
        let status = dev(p, &["extension", "status"]).await.unwrap();
        for said in [
            "00000000000000ba doesn't activate",
            "not_a_key",
            "source has changed",
        ] {
            assert!(status.contains(said), "{said}: {status}");
        }

        let layout = oxplow_app::AppLayout::for_project(p);
        let held = oxplow_app::try_acquire_instance_lock(&layout)
            .unwrap()
            .unwrap();
        let err = dev(p, &["extension", "deploy", "oxplow-foundation"])
            .await
            .unwrap_err();
        assert!(err.contains("oxplow is running"), "{err}");
        drop(held);

        let source = p.join(".oxplow/extensions/oxplow-foundation");
        let hash = oxplow_app::extension::extension_store::source_hash(&source).unwrap();
        let deployed = dev(p, &["extension", "deploy", "oxplow-foundation"])
            .await
            .unwrap();
        assert!(deployed.contains(&hash), "{deployed}");
        let status = dev(p, &["extension", "status"]).await.unwrap();
        assert!(!status.contains("doesn't activate"), "{status}");
        assert!(status.contains(&hash), "{status}");
    }

    /// With no name, `extension deploy` deploys every extension that is on
    /// whose source isn't what runs, foundation first: a checkout's first
    /// run, since boot deploys nothing. Run again, each already runs.
    #[tokio::test]
    async fn deploy_with_no_name_deploys_what_is_on() {
        if !cfg!(debug_assertions) {
            return;
        }
        let dir = developing_oxplow_with_a_stale_foundation().await;
        let p = dir.path();
        let acme = p.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(acme.join("pages")).unwrap();
        std::fs::write(
            acme.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            acme.join("pages/one.yaml"),
            "title: One\nloaders:\n  q: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: q }\n",
        )
        .unwrap();
        from_source();
        std::fs::write(
            p.join(".oxplow/project.yaml"),
            "oxplow.extensions.acme.enabled: true\n",
        )
        .unwrap();
        let first = dev(p, &["extension", "deploy"]).await.unwrap();
        let at = |what: &str| {
            first
                .find(what)
                .unwrap_or_else(|| panic!("{what}: {first}"))
        };
        assert!(
            at("oxplow-foundation: deployed") < at("acme: deployed"),
            "{first}"
        );
        let again = dev(p, &["extension", "deploy"]).await.unwrap();
        for name in ["oxplow-foundation", "acme"] {
            assert!(again.contains(&format!("{name}: already runs")), "{again}");
        }
    }

    /// With no name, the extensions deploy as one change to what runs:
    /// oxplow's own two moving their shared settings to a new version
    /// together are checked against each other, never against the running
    /// copy of the other (foundation, first, once was refused for
    /// bundled's old version, and needed a second run).
    #[tokio::test]
    async fn deploy_with_no_name_deploys_a_shared_settings_version_together() {
        if !cfg!(debug_assertions) {
            return;
        }
        let dir = developing_oxplow_with_a_stale_foundation().await;
        let p = dir.path();
        let foundation = p.join(".oxplow/extensions/oxplow-foundation/extension.yaml");
        let bundled = p.join(".oxplow/extensions/oxplow-bundled/extension.yaml");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        let bundled_at = |version: u32| {
            format!(
                "manifest: 2\nname: oxplow-bundled\nnamespace: oxplow\nintent:\n  purpose: test\n\
                 configuration:\n  version: {version}\n  schema:\n    type: object\n    \
                 properties:\n      bundledKey: {{ type: integer }}\n"
            )
        };
        std::fs::write(&bundled, bundled_at(2)).unwrap();
        from_source();
        std::fs::write(
            p.join(".oxplow/project.yaml"),
            "oxplow.extensions.oxplow-bundled.enabled: true\n",
        )
        .unwrap();
        let first = dev(p, &["extension", "deploy"]).await.unwrap();
        for name in ["oxplow-foundation", "oxplow-bundled"] {
            assert!(first.contains(&format!("{name}: deployed")), "{first}");
        }

        let text = std::fs::read_to_string(&foundation).unwrap();
        assert!(text.contains("\n  version: 2\n"), "{text}");
        std::fs::write(
            &foundation,
            text.replace("\n  version: 2\n", "\n  version: 3\n"),
        )
        .unwrap();
        std::fs::write(&bundled, bundled_at(3)).unwrap();
        let both = dev(p, &["extension", "deploy"]).await.unwrap();
        for name in ["oxplow-foundation", "oxplow-bundled"] {
            assert!(both.contains(&format!("{name}: deployed")), "{both}");
        }
    }

    /// With no name, an extension that is on but doesn't load is held
    /// back, as boot holds it: the deploy says why and still succeeds, so
    /// `run:dev` starts oxplow. A required extension that fails still
    /// fails the deploy, and naming the broken one fails it too.
    #[tokio::test]
    async fn deploy_with_no_name_holds_back_what_does_not_load() {
        if !cfg!(debug_assertions) {
            return;
        }
        let dir = developing_oxplow_with_a_stale_foundation().await;
        let p = dir.path();
        let old = p.join(".oxplow/extensions/old");
        std::fs::create_dir_all(old.join("lenses")).unwrap();
        std::fs::write(old.join("lenses/one.yaml"), "title: One\n").unwrap();
        std::fs::write(
            old.join("extension.yaml"),
            "manifest: 2\nname: old\nintent:\n  purpose: test\n",
        )
        .unwrap();
        from_source();
        std::fs::write(
            p.join(".oxplow/project.yaml"),
            "oxplow.extensions.old.enabled: true\n",
        )
        .unwrap();
        let out = dev(p, &["extension", "deploy"]).await.unwrap();
        assert!(out.contains("oxplow-foundation: deployed"), "{out}");
        assert!(out.contains("held back: old"), "{out}");
        assert!(out.contains("lenses are pages now"), "{out}");
        let err = dev(p, &["extension", "deploy", "old"]).await.unwrap_err();
        assert!(err.contains("lenses are pages now"), "{err}");
    }

    /// A project that runs the extensions built into oxplow has no source
    /// for them: deploying with no name deploys nothing, and succeeds.
    #[tokio::test]
    async fn deploy_with_no_name_skips_what_has_no_source() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(p)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        drop(
            oxplow_app::Services::boot(
                oxplow_app::AppLayout::for_project(p),
                Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            )
            .unwrap(),
        );
        let out = dev(p, &["extension", "deploy"]).await.unwrap();
        assert!(!out.contains("deployed"), "{out}");
    }

    /// `extension test` runs an extension's tests on a throwaway oxplow,
    /// prints what they found and, with `--junit`, writes a report oxplow's
    /// Tests panel reads; a failing extension fails the command.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_runs_an_extensions_tests_and_writes_junit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let write = |rel: &str, text: &str| {
            let path = p.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n  examples:\n    - { name: one, input: { page: items }, expect: a row }\n",
        );
        write(
            ".oxplow/extensions/acme/pages/items.yaml",
            "title: Items\nloaders:\n  items: { sql: \"SELECT 1 AS n\" }\nview: { from: items, viz: table }\n",
        );
        write(
            ".oxplow/extensions/acme/fixtures/one.yaml",
            "when: { page: items }\nexpect: { loaders: { items: { rows: 1 } } }\n",
        );
        let junit = p.join("target/extension-tests/junit.xml");
        let out = dev(
            p,
            &[
                "extension",
                "test",
                "acme",
                "--junit",
                junit.to_str().unwrap(),
            ],
        )
        .await
        .unwrap();
        assert!(out.contains("acme: 0 errors"), "{out}");
        let xml = std::fs::read_to_string(&junit).unwrap();
        assert!(
            xml.starts_with("<?xml") && xml.contains("<testsuites>"),
            "{xml}"
        );
        assert!(
            xml.contains("<testcase name=\"example one\" classname=\"acme\""),
            "{xml}"
        );

        write(
            ".oxplow/extensions/acme/fixtures/one.yaml",
            "when: { page: items }\nexpect: { loaders: { items: { rows: 2 } } }\n",
        );
        let err = dev(p, &["extension", "test", "acme"]).await.unwrap_err();
        assert!(err.contains("loader `items` returned"), "{err}");

        // `--only` runs the check and the named examples alone.
        let err = dev(p, &["extension", "test", "acme", "--only", "one"])
            .await
            .unwrap_err();
        assert!(err.ends_with("; ran check, 1 example"), "{err}");
        let err = dev(p, &["extension", "test", "acme", "--only", "two"])
            .await
            .unwrap_err();
        assert!(err.contains("no example `two`"), "{err}");
    }

    fn store_file(p: &Path) -> PathBuf {
        oxplow_app::extension::store_path(p, "oxtask")
    }

    /// With oxtask off there is no task tracker to reach and its file is not
    /// made.
    #[tokio::test]
    async fn with_oxtask_off_there_is_no_tracker_and_its_file_is_not_made() {
        let dir = project().await;
        std::fs::write(
            dir.path().join(".oxplow/local.yaml"),
            "oxplow.extensions.oxtask.enabled: false\n",
        )
        .unwrap();
        let err = dev(dir.path(), &["task", "create", "Nowhere"])
            .await
            .unwrap_err();
        assert!(err.contains("oxtask"), "{err}");
        assert!(!store_file(dir.path()).exists());
    }

    /// With oxtask enabled the same subcommands work on its file, through
    /// its triggers: a change number on every write, a soft delete only.
    #[tokio::test]
    async fn with_oxtask_on_the_tasks_are_its_files() {
        let dir = project().await;
        let p = dir.path();
        std::fs::write(
            p.join(".oxplow/local.yaml"),
            "oxplow.extensions.oxtask.enabled: true\n",
        )
        .unwrap();
        let created: Value = serde_json::from_str(
            &dev(
                p,
                &["task", "create", "Write the docs", "--body", "All of them"],
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(created["id"], "tsk1");
        assert_eq!(created["uri"], "oxplow:oxplow/task?id=tsk1&provider=oxtask");
        dev(
            p,
            &[
                "task",
                "create",
                "Child",
                "--parent",
                "tsk1",
                "--state",
                "in_progress",
            ],
        )
        .await
        .unwrap();
        dev(p, &["task", "transition", "tsk1", "in_progress"])
            .await
            .unwrap();
        let commented: Value = serde_json::from_str(
            &dev(p, &["task", "comment", "tsk1", "Started."])
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(commented["comment"], "cmt1");
        dev(p, &["task", "update", "tsk2", "--title", "Child, renamed"])
            .await
            .unwrap();
        dev(p, &["task", "link", "tsk2", "tsk1", "blocks"])
            .await
            .unwrap();
        let list = dev(p, &["task", "list"]).await.unwrap();
        assert!(
            list.contains("tsk1\tin_progress\tWrite the docs")
                && list.contains("tsk2\tin_progress\tChild, renamed"),
            "{list}"
        );
        dev(p, &["task", "transition", "tsk1", "done"])
            .await
            .unwrap();
        assert!(!dev(p, &["task", "list"])
            .await
            .unwrap()
            .contains("Write the docs"));
        assert!(dev(p, &["task", "list", "--all"])
            .await
            .unwrap()
            .contains("Write the docs"));
        let shown = dev(p, &["task", "show", "tsk1"]).await.unwrap();
        assert!(
            shown.contains("state: done") && shown.contains("All of them"),
            "{shown}"
        );
        let child = dev(p, &["task", "show", "tsk2"]).await.unwrap();
        assert!(child.contains("parent: tsk1"), "{child}");
        dev(p, &["task", "unlink", "tsk2", "tsk1", "blocks"])
            .await
            .unwrap();
        // A missing task or link, and a bad stage, are refused.
        for args in [
            vec!["task", "transition", "tsk99", "done"],
            vec!["task", "comment", "tsk99", "x"],
            vec!["task", "unlink", "tsk2", "tsk1", "blocks"],
            vec!["task", "transition", "tsk1", "sideways"],
        ] {
            assert!(dev(p, &args).await.is_err(), "{args:?}");
        }

        // The file's own triggers numbered every write (the collector
        // follows `seq`) and guard it against a hard delete; core's tables
        // were not touched.
        let file = Connection::open(store_file(p)).unwrap();
        let (top, seq): (i64, i64) = file
            .query_row(
                "SELECT (SELECT v FROM change_seq), (SELECT seq FROM task WHERE id = 'tsk1')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(seq > 1 && top >= seq, "{seq} {top}");
        assert!(file
            .execute("DELETE FROM task WHERE id = 'tsk1'", [])
            .is_err());
        let author: (String, String) = file
            .query_row(
                "SELECT author, author_kind FROM task WHERE id = 'tsk1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(author, ("oxplow:oxplow/person".into(), "person".into()));
    }

    /// A run with `--thread` writes as that thread's agent.
    #[tokio::test]
    async fn with_oxtask_on_a_thread_run_is_that_agents() {
        let dir = project().await;
        let p = dir.path();
        std::fs::write(
            p.join(".oxplow/local.yaml"),
            "oxplow.extensions.oxtask.enabled: true\n",
        )
        .unwrap();
        dev(p, &["--thread", "thr1", "task", "create", "By the agent"])
            .await
            .unwrap();
        let author: (String, String) = Connection::open(store_file(p))
            .unwrap()
            .query_row("SELECT author, author_kind FROM task", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(
            author,
            ("oxplow:oxplow/thread?id=thr1".into(), "agent".into())
        );
    }
}
