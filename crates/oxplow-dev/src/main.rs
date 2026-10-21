//! `oxplow-dev`: helpers for working on oxplow itself, never shipped.
//!
//! `oxplow-dev wiki …` does the same for the project's wiki, through the
//! `knowledge.*` commands (the wiki as the store, whatever the project
//! has active).
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
oxplow-dev task link <id> <target> <blocks|discovered_from|relates_to|duplicates|supersedes|replies_to|about>
oxplow-dev task unlink <id> <target> <link type>

oxplow-dev wiki list                         the pages (slug, title)
oxplow-dev wiki show <slug>
oxplow-dev wiki write <slug> [--title T] [--file F]   body from F, else stdin
oxplow-dev wiki delete <slug>
oxplow-dev wiki note <body>                  a note on the acting thread (needs --thread)
oxplow-dev wiki search <text>                pages whose slug, title or body has it
oxplow-dev uri convert [<kind>=<extension> …] each stdin line that is an old ref, as its URI

oxplow-dev extension status                  what the next boot would run, and what it
                                             would hold back and why (safe while oxplow runs)
oxplow-dev extension deploy <name>           deploy <name>'s source as the app does,
                                             confirming its permissions and build as you;
                                             only while oxplow is quit

oxplow-dev --help                            this

<id> is a task's id (tsk12) or ref (work_item:oxplow:tsk12); a link's
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
            let tasks = Tasks::open(&project, thread.as_deref())?;
            tasks.run(args.split_off(1)).await
        }
        Some("wiki") => {
            let tasks = Tasks::open(&project, thread.as_deref())?;
            tasks.wiki(args.split_off(1)).await
        }
        Some("extension") => extension(&project, &args[1..]).await,
        Some("uri") if args.get(1).is_some_and(|a| a == "convert") => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
                .map_err(|e| e.to_string())?;
            uri_convert(&input, &args[2..])
        }
        _ => Err(format!("usage:\n{USAGE}")),
    }
}

/// `uri convert [<kind>=<extension> …]`: each line of `input` that is a
/// ref of the old grammar, as its URI (the same table the migration that
/// rewrote the stored refs used, `oxplow_db::uri_migration`); any other
/// line as it was. An extension's kind names its extension
/// (`acme_pr=acme`).
fn uri_convert(input: &str, args: &[String]) -> Result<String, String> {
    let extensions: Vec<(&str, &str)> = args
        .iter()
        .map(|a| {
            a.split_once('=')
                .ok_or_else(|| format!("`{a}`: expected <kind>=<extension>"))
        })
        .collect::<Result<_, _>>()?;
    Ok(input
        .lines()
        .map(|line| {
            let kind = line.split_once(':').map(|(k, _)| k);
            let extension = extensions
                .iter()
                .find(|(k, _)| Some(*k) == kind)
                .map(|(_, e)| *e);
            oxplow_db::uri_migration::v1_to_uri(line, extension).unwrap_or_else(|| line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n"))
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
            let next = oxplow_app::extension_deploy::next_boot(project, &rows).await?;
            let mut out: Vec<String> = next
                .runs
                .iter()
                .map(|(name, what)| format!("{name}: {what}"))
                .collect();
            for (name, why) in &next.problems {
                out.push(format!("\nheld back: {name}"));
                out.extend(why.iter().map(|w| format!("  - {w}")));
            }
            if next.problems.is_empty() {
                out.push("\nnothing held back".into());
            }
            Ok(out.join("\n"))
        }
        [deploy, name] if deploy == "deploy" => {
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
            // Startup, as a boot runs it: the live set it starts from is
            // what the deploy replaces.
            svc.extension_activation
                .initialize(svc.clone())
                .await
                .map_err(|e| e.to_string())?;
            let deployed = oxplow_app::extension_deploy::deploy(&svc, name, project, None, true)
                .await
                .map_err(|e| format!("deploy `{name}`: {e}"))?;
            let mut out = vec![format!(
                "{name}: {} {}",
                if deployed.already {
                    "already runs"
                } else {
                    "deployed"
                },
                deployed.hash
            )];
            out.extend(deployed.warnings.iter().map(|w| format!("  warning: {w}")));
            for (other, why) in svc.extension_activation.problems() {
                out.push(format!("held back: {other}"));
                out.extend(why.iter().map(|w| format!("  - {w}")));
            }
            Ok(out.join("\n"))
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
        let links_for_notes = links.clone();
        use oxplow_app::commands::work_item as w;
        for op in [
            w::transition_op(work_items.clone(), db.clone()),
            w::create_op(work_items.clone(), links.clone()),
            w::update_op(work_items.clone(), links.clone()),
            w::link_op(work_items.clone(), links.clone()),
            w::unlink_op(work_items.clone(), links),
            w::comment_op(work_items.clone()),
        ] {
            bus.add_op(op).map_err(|e| e.to_string())?;
        }
        // The wiki as the knowledge store, whatever the project has
        // active: pages are `.oxplow/wiki/<slug>.md`, recorded as the app does.
        let wiki_id = "oxplow-dev-wiki";
        let stores = Arc::new(oxplow_domain::knowledge::KnowledgeRegistry::new(Arc::new(
            move || wiki_id.to_string(),
        )));
        stores.register(Arc::new(oxplow_app::knowledge::WikiKnowledge {
            id: wiki_id.into(),
            project_dir: layout.project_dir.clone(),
        }));
        for op in oxplow_app::knowledge::ops(oxplow_app::knowledge::KnowledgeTarget {
            project_dir: layout.project_dir.clone(),
            vcs: Arc::new(oxplow_app::vcs::GitProvider),
            db: db.clone(),
            vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
            stores,
            wiki: Arc::new(oxplow_app::knowledge::WikiFollow::default()),
        })
        .into_iter()
        .chain(oxplow_app::commands::note::ops(links_for_notes))
        {
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
            Some(verb @ ("link" | "unlink")) => {
                let mut input = Map::new();
                input.insert("ref".into(), json!(arg(1, "<id>")?));
                input.insert("target".into(), json!(arg(2, "<target>")?));
                input.insert("link_type".into(), json!(arg(3, "<link type>")?));
                self.command(&format!("oxplow.work_item.{verb}"), input)
                    .await
            }
            _ => Err(format!("usage:\n{USAGE}")),
        }
    }

    /// `oxplow-dev wiki …`: the pages through the knowledge commands.
    async fn wiki(&self, mut args: Vec<String>) -> Result<String, String> {
        let title = take_flag(&mut args, "--title");
        let file = take_flag(&mut args, "--file");
        if let Some(unknown) = args.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option `{unknown}`\nusage:\n{USAGE}"));
        }
        let arg = |i: usize, what: &str| {
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("missing {what}\nusage:\n{USAGE}"))
        };
        match args.first().map(String::as_str) {
            Some("list") => {
                let rows = self
                    .rows("SELECT slug, title FROM wiki_page ORDER BY slug", vec![])
                    .await?;
                Ok(rows
                    .iter()
                    .map(|r| format!("{}\t{}", r[0], r[1]))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            Some("show") => {
                let slug = arg(1, "<slug>")?;
                let rows = self
                    .rows(
                        "SELECT title, body FROM wiki_page WHERE slug = ?1",
                        vec![slug.clone()],
                    )
                    .await?;
                let r = rows.first().ok_or_else(|| format!("no page `{slug}`"))?;
                Ok(r[1].clone())
            }
            Some("search") => {
                let needle = format!("%{}%", arg(1, "<text>")?);
                let rows = self
                    .rows(
                        "SELECT slug, title FROM wiki_page
                          WHERE slug LIKE ?1 OR title LIKE ?1 OR body LIKE ?1 ORDER BY slug",
                        vec![needle],
                    )
                    .await?;
                Ok(rows
                    .iter()
                    .map(|r| format!("{}\t{}", r[0], r[1]))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            Some("write") => {
                let slug = arg(1, "<slug>")?;
                let body = match file {
                    Some(path) => {
                        std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?
                    }
                    None => {
                        let mut body = String::new();
                        std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)
                            .map_err(|e| e.to_string())?;
                        body
                    }
                };
                let mut input = Map::new();
                input.insert("slug".into(), json!(slug));
                input.insert("body".into(), json!(body));
                put(&mut input, "title", title);
                self.command("oxplow.knowledge.write_page", input).await
            }
            Some("delete") => {
                let mut input = Map::new();
                input.insert("slug".into(), json!(arg(1, "<slug>")?));
                // Naming the page to delete is the confirmation.
                self.confirmed_command("oxplow.knowledge.delete_page", input)
                    .await
            }
            Some("note") => {
                let mut input = Map::new();
                input.insert("body".into(), json!(arg(1, "<body>")?));
                self.command("oxplow.knowledge.add_note", input).await
            }
            _ => Err(format!("usage:\n{USAGE}")),
        }
    }

    /// Run `name` through the bus; its result, pretty.
    async fn command(&self, name: &str, input: Map<String, Value>) -> Result<String, String> {
        self.run_command(name, input, false).await
    }

    /// [`Self::command`] for one that asks to be confirmed, which the person
    /// running it here has just done.
    async fn confirmed_command(
        &self,
        name: &str,
        input: Map<String, Value>,
    ) -> Result<String, String> {
        self.run_command(name, input, true).await
    }

    async fn run_command(
        &self,
        name: &str,
        input: Map<String, Value>,
        confirmed: bool,
    ) -> Result<String, String> {
        let out = self
            .bus
            .run(&self.actor, name, Value::Object(input), confirmed)
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
        let id = oxplow_domain::uri::build::work_item_id_of_uri(id)
            .and_then(|rest| rest.strip_prefix("oxplow:"))
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

    /// The wiki is read and written through the knowledge commands: a page
    /// written (body from a file) is listed, shown and found by search, its
    /// links are checked, a note lands on the thread, and a deleted page is
    /// gone — the record and the file as the app keeps them.
    #[tokio::test]
    async fn the_wiki_is_read_and_written_through_the_knowledge_commands() {
        let dir = project().await;
        let p = dir.path();
        let body = p.join("body.md");
        std::fs::write(&body, "# Wombats\n\nThey dig burrows.\n").unwrap();
        let body = body.display().to_string();
        let written = dev(p, &["wiki", "write", "wombats", "--file", &body])
            .await
            .unwrap();
        assert!(written.contains("wiki:wombats"), "{written}");
        assert!(p.join(".oxplow/wiki/wombats.md").is_file());
        assert!(dev(p, &["wiki", "list"])
            .await
            .unwrap()
            .contains("wombats\tWombats"));
        let shown = dev(p, &["wiki", "show", "wombats"]).await.unwrap();
        assert!(shown.contains("They dig burrows."), "{shown}");
        assert!(dev(p, &["wiki", "search", "BURROWS"])
            .await
            .unwrap()
            .contains("wombats"));
        assert!(dev(p, &["wiki", "search", "platypus"])
            .await
            .unwrap()
            .is_empty());
        // A title replaces the heading; a link that doesn't resolve is refused.
        dev(
            p,
            &[
                "wiki",
                "write",
                "wombats",
                "--title",
                "Wombat facts",
                "--file",
                &body,
            ],
        )
        .await
        .unwrap();
        assert!(dev(p, &["wiki", "list"])
            .await
            .unwrap()
            .contains("wombats\tWombat facts"));
        std::fs::write(&body, "# Bad\n\nSee [[file:nope/missing.rs]].\n").unwrap();
        let err = dev(p, &["wiki", "write", "bad", "--file", &body])
            .await
            .unwrap_err();
        assert!(err.contains("nope/missing.rs"), "{err}");
        assert!(!dev(p, &["wiki", "list"]).await.unwrap().contains("bad"));
        // A note goes on the thread the call acts as.
        let noted = dev(
            p,
            &["--thread", "thr1", "wiki", "note", "Burrows are deep."],
        )
        .await
        .unwrap();
        assert!(noted.contains("Burrows are deep."), "{noted}");
        dev(p, &["wiki", "delete", "wombats"]).await.unwrap();
        assert!(!p.join(".oxplow/wiki/wombats.md").exists());
        assert!(dev(p, &["wiki", "list"]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_bad_wiki_call_says_how_to_call() {
        let dir = project().await;
        for args in [
            &["wiki", "frobnicate"][..],
            &["wiki", "show"],
            &["wiki", "write", "x", "--bogus", "1"],
        ] {
            let err = dev(dir.path(), args).await.unwrap_err();
            assert!(
                err.contains("usage") || err.contains("missing"),
                "{args:?}: {err}"
            );
        }
    }

    /// A project developing oxplow (`enableBundledExtensions: false`):
    /// foundation's source as it ships (its build a no-op, the output
    /// cargo built already there), set up by the app, and an older
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
            for (rel, bytes) in shipped.files {
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
        write_files(&p.join(".oxplow/extensions/oxplow-foundation"), &|t| {
            t.replacen(
                "\"bun install --frozen-lockfile && bun run build\"",
                "\"true\"",
                1,
            )
        });
        std::fs::write(
            p.join(".oxplow/project.yaml"),
            "enableBundledExtensions: false\n",
        )
        .unwrap();
        let svc = oxplow_app::Services::boot(
            oxplow_app::AppLayout::for_project(p),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .unwrap();
        svc.streams.ensure_primary().await.unwrap();
        let hash = "00000000000000ba";
        write_files(
            &oxplow_app::extension_store::deployment_dir(p, "oxplow-foundation", hash),
            &|t| format!("{t}not_a_key: 1\n"),
        );
        let row = oxplow_db::extension_deployment_store::DeploymentRow {
            name: "oxplow-foundation".into(),
            hash: hash.into(),
            deployed_at: "2026-10-19T00:00:00Z".into(),
            from_stream: None,
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
        let hash = oxplow_app::extension_store::source_hash(&source).unwrap();
        let deployed = dev(p, &["extension", "deploy", "oxplow-foundation"])
            .await
            .unwrap();
        assert!(deployed.contains(&hash), "{deployed}");
        let status = dev(p, &["extension", "status"]).await.unwrap();
        assert!(!status.contains("doesn't activate"), "{status}");
        assert!(status.contains(&hash), "{status}");
    }
}
