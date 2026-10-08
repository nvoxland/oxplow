//! `oxplow-dev`: helpers for working on oxplow itself, never shipped.
//!
//! `oxplow-dev task …` reads and changes oxplow's tasks while the project
//! has no work list active (the bundled extension disabled to strip oxplow
//! to its core). It runs oxplow's own `work_item.*` commands on a command
//! bus over the project's database, as the app would — validation, audit,
//! events and all — on a bus that doesn't check what's active. So the rows
//! and the log are the task system's own, and turning the extension back
//! on shows all of it. The database is opened as it is
//! (`Database::open_existing`): nothing migrated or recompiled under the
//! running app. Its reads are oxplow's own task table, not the work-item
//! interface (which shows only the active list's items): the one place
//! outside oxplow's implementation that reads it, on purpose. See
//! `.context/working-in-this-repo.md` "oxplow-dev".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_app::commands::CommandBus;
use oxplow_db::{Database, SqliteEventLogStore};
use oxplow_domain::work_items::WorkItemsRegistry;
use oxplow_domain::{Actor, ThreadId};
use serde_json::{json, Map, Value};

const USAGE: &str = "\
oxplow-dev task list [--all]                 open tasks (--all: every state)
oxplow-dev task show <id>
oxplow-dev task create <title> [--body B] [--parent ID] [--state todo|in_progress]
oxplow-dev task transition <id> <todo|in_progress|blocked|done|canceled>
oxplow-dev task update <id> [--title T] [--body B] [--parent ID]
oxplow-dev task comment <id> <body>
oxplow-dev task link <id> <target-id> <blocks|discovered_from|relates_to|duplicates|supersedes|replies_to>

oxplow-dev --help                            this

<id> is a task's id (tsk12) or ref (work_item:oxplow:tsk12). The project is
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
            let tasks = Tasks::open(&project, thread.as_deref())?;
            tasks.run(args.split_off(1)).await
        }
        _ => Err(format!("usage:\n{USAGE}")),
    }
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

/// oxplow's tasks over the project's database.
struct Tasks {
    db: Database,
    bus: Arc<CommandBus>,
    actor: Actor,
}

impl Tasks {
    fn open(project: &Path, thread: Option<&str>) -> Result<Self, String> {
        let layout = oxplow_app::AppLayout::for_project(project);
        let db = Database::open_existing(&layout.state_db_path)
            .map_err(|e| format!("{}: {e}", layout.state_db_path.display()))?;
        let vocabulary = oxplow_domain::vocabulary::VocabularyHandle::core();
        let log = SqliteEventLogStore::new(db.clone(), vocabulary.clone());
        // The work-item interface's projections, so a write here reads
        // back at once (the app's pump delivers the rest).
        let pump = Arc::new(oxplow_app::event_pump::EventPump::new(
            db.clone(),
            log.clone(),
            vec![
                Arc::new(oxplow_app::work_items::WorkItemsProjection),
                Arc::new(oxplow_app::page_ref_consumers::PageRefWorkItemConsumer { vocabulary }),
            ],
        ));
        // No `with_capabilities`: every registered command is offered,
        // whatever the project has active.
        let bus = Arc::new(CommandBus::new(
            db.clone(),
            log,
            Arc::new(oxplow_app::agent_policy::AgentPolicy),
            pump,
        ));
        let work_items =
            WorkItemsRegistry::new(Arc::new(|| oxplow_app::work_items::PROVIDER.to_string()));
        work_items.register(
            oxplow_app::work_items::built_in_provider(oxplow_app::work_items::BUILT_IN, &db)
                .ok_or("oxplow's tasks aren't a built-in work list")?,
        );
        let links = oxplow_app::link_check::LinkDeps {
            project_dir: layout.project_dir.clone(),
            vcs: Arc::new(oxplow_app::vcs::GitProvider),
            db: db.clone(),
            vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
        };
        use oxplow_app::commands::work_item as w;
        for op in [
            w::transition_op(work_items.clone(), db.clone()),
            w::create_op(work_items.clone(), links.clone()),
            w::update_op(work_items.clone(), links),
            w::link_op(work_items.clone()),
            w::comment_op(work_items.clone()),
        ] {
            bus.add_op(op).map_err(|e| e.to_string())?;
        }
        // Declared as oxplow's own commands in its foundation extension.
        oxplow_app::extension_commands::register_declared(&bus);
        let actor = match thread {
            Some(t) => Actor::Agent {
                session_id: None,
                thread_id: Some(
                    t.parse::<ThreadId>()
                        .map_err(|_| format!("`{t}` isn't a thread id (thr1)"))?,
                ),
                stream_id: None,
            },
            None => Actor::Human,
        };
        Ok(Self { db, bus, actor })
    }

    async fn run(&self, mut args: Vec<String>) -> Result<String, String> {
        let body = take_flag(&mut args, "--body");
        let parent = take_flag(&mut args, "--parent");
        let state = take_flag(&mut args, "--state");
        let title = take_flag(&mut args, "--title");
        let all = args.iter().any(|a| a == "--all");
        args.retain(|a| a != "--all");
        // What's left is positional: an option it doesn't take would
        // otherwise become a title or an id.
        if let Some(unknown) = args.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option `{unknown}`\nusage:\n{USAGE}"));
        }
        let arg = |i: usize, what: &str| {
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("missing {what}\nusage:\n{USAGE}"))
        };
        match args.first().map(String::as_str) {
            Some("list") => self.list(all).await,
            Some("show") => self.show(&arg(1, "<id>")?).await,
            Some("create") => {
                let mut input = Map::new();
                input.insert("title".into(), json!(arg(1, "<title>")?));
                put(&mut input, "body", body);
                put(&mut input, "parent_ref", parent);
                put(&mut input, "state", state);
                self.command("oxplow.work_item.create", input).await
            }
            Some("transition") => {
                let mut input = Map::new();
                input.insert("ref".into(), json!(arg(1, "<id>")?));
                input.insert("to".into(), json!(arg(2, "<state>")?));
                self.command("oxplow.work_item.transition", input).await
            }
            Some("update") => {
                let mut input = Map::new();
                input.insert("ref".into(), json!(arg(1, "<id>")?));
                put(&mut input, "title", title);
                put(&mut input, "body", body);
                put(&mut input, "parent_ref", parent);
                self.command("oxplow.work_item.update", input).await
            }
            Some("comment") => {
                let mut input = Map::new();
                input.insert("ref".into(), json!(arg(1, "<id>")?));
                input.insert("body".into(), json!(arg(2, "<body>")?));
                self.command("oxplow.work_item.comment", input).await
            }
            Some("link") => {
                let mut input = Map::new();
                input.insert("ref".into(), json!(arg(1, "<id>")?));
                input.insert("target".into(), json!(arg(2, "<target-id>")?));
                input.insert("link_type".into(), json!(arg(3, "<link type>")?));
                self.command("oxplow.work_item.link", input).await
            }
            _ => Err(format!("usage:\n{USAGE}")),
        }
    }

    /// Run `name` through the bus; its result, pretty.
    async fn command(&self, name: &str, input: Map<String, Value>) -> Result<String, String> {
        let out = self
            .bus
            .run(&self.actor, name, Value::Object(input), false)
            .await
            .map_err(|e| e.to_string())?;
        // The interface answers `{ ref, state? }`; this is oxplow's task
        // tool, so it names the task's id too.
        let mut result = out.result;
        let task = result["ref"]
            .as_str()
            .and_then(oxplow_tasks::task_of_work_item_ref);
        if let (Some(task), Value::Object(fields)) = (task, &mut result) {
            fields.insert("id".into(), json!(task.to_string()));
        }
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    /// Oxplow's tasks, from its own table: whatever work list is active
    /// (the interface shows only the active one's), this is oxplow's.
    async fn list(&self, all: bool) -> Result<String, String> {
        let rows = self
            .rows(
                &format!(
                    "SELECT 'tsk' || id, status, title FROM task WHERE deleted_at IS NULL {}
                     ORDER BY id",
                    if all {
                        ""
                    } else {
                        "AND status NOT IN ('done', 'canceled', 'archived')"
                    }
                ),
                vec![],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| format!("{}\t{}\t{}", r[0], r[1], r[2]))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    async fn show(&self, id: &str) -> Result<String, String> {
        let id = id
            .strip_prefix("work_item:oxplow:")
            .unwrap_or(id)
            .trim_start_matches("tsk")
            .to_string();
        let rows = self
            .rows(
                "SELECT 'tsk' || id, status, title,
                        coalesce('tsk' || parent_id, ''), coalesce(description, '')
                   FROM task WHERE id = ?1 AND deleted_at IS NULL",
                vec![id.clone()],
            )
            .await?;
        let r = rows.first().ok_or_else(|| format!("no task `tsk{id}`"))?;
        Ok(format!(
            "{}\nstate: {}\ntitle: {}\nparent: {}\n\n{}",
            r[0], r[1], r[2], r[3], r[4]
        ))
    }

    /// `sql`'s rows as text.
    async fn rows(&self, sql: &str, params: Vec<String>) -> Result<Vec<Vec<String>>, String> {
        let sql = sql.to_string();
        self.db
            .read(move |c| {
                let mut stmt = c.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
                let n = stmt.column_count();
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        (0..n)
                            .map(|i| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .map_err(|e| e.to_string())
    }
}

fn put(input: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value {
        input.insert(key.into(), json!(v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let db =
            Database::open_existing(oxplow_app::AppLayout::for_project(p).state_db_path).unwrap();
        let logged: i64 = db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM event_log WHERE type = 'work_item.state_changed'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert!(logged >= 2, "{logged}");
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
        let err = dev(dir.path(), &["task", "frobnicate"]).await.unwrap_err();
        assert!(err.contains("usage"), "{err}");
        let err = dev(dir.path(), &["task", "transition", "ENG-1", "done"])
            .await
            .unwrap_err();
        assert!(err.contains("tsk"), "{err}");
    }
}
