//! `oxplow-dev task …` over oxtask's own file, for a project that has the
//! oxtask extension enabled: written to
//! `.oxplow/extensions-data/oxtask.sqlite` (created with the extension's first
//! migration). Every write is one statement through the
//! file's own triggers (`seq`, the delete guard), so the app's collector sees
//! it on its next `sqlite` trigger.
//!
//! The SQL is the extension's handlers' (`extensions/oxtask/handlers/*.star`):
//! the same rows, whoever writes them. Nothing here runs a script or opens
//! the main database; a person's run is `person`, a run with `--thread` is
//! that thread's agent.

use std::path::Path;

use oxplow_db::extension_store::{ExtensionStore, Statement};
use serde_json::{json, Value};

/// oxtask's first migration: the file is made the way the extension makes it.
/// (Later migrations are applied by the extension's own boot.)
pub(crate) const STORE_MIGRATIONS: &[(&str, &str)] = &[(
    "V1__tasks.sql",
    include_str!("../../../extensions/oxtask/storage/migrations/V1__tasks.sql"),
)];

const PERSON: &str = "oxplow:oxplow/person";
const STAGES: [&str; 6] = [
    "backlog",
    "ready",
    "in_progress",
    "blocked",
    "done",
    "canceled",
];
const LINK_TYPES: [&str; 7] = [
    "blocks",
    "discovered_from",
    "relates_to",
    "duplicates",
    "supersedes",
    "replies_to",
    "about",
];
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// oxtask's file, open, with who is writing.
pub struct OxtaskFile {
    store: ExtensionStore,
    author: String,
    kind: &'static str,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn stmt(sql: &str, params: Vec<Value>, expect_changes: Option<usize>) -> Statement {
    Statement {
        sql: sql.to_string(),
        params,
        expect_changes,
    }
}

/// `tsk42`, `12` or a task URI, as the file's id (`tsk42`).
fn task_id(raw: &str) -> Result<String, String> {
    let id = oxplow_domain::uri::OxplowUri::parse(raw)
        .ok()
        .and_then(|uri| uri.param("id").map(str::to_string))
        .unwrap_or_else(|| raw.to_string());
    let digits = id.strip_prefix("tsk").unwrap_or(&id);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("`{raw}` isn't a task id (tsk12)"));
    }
    Ok(format!("tsk{digits}"))
}

/// A stage, from a bucket (`todo` is `ready`) or a stage of oxtask's.
fn stage_of(raw: &str) -> Result<&'static str, String> {
    let named = if raw == "todo" { "ready" } else { raw };
    STAGES
        .iter()
        .copied()
        .find(|s| *s == named)
        .ok_or_else(|| format!("`{raw}` isn't a stage (todo, {})", STAGES.join(", ")))
}

fn uri_of(id: &str) -> String {
    oxplow_domain::uri::build::of("oxplow/task", &[("provider", "oxtask"), ("id", id)])
}

impl OxtaskFile {
    /// Open (creating and migrating) oxtask's file in `project`.
    pub fn open(project: &Path, thread: Option<&str>) -> Result<Self, String> {
        let path = oxplow_app::extension::store_path(project, "oxtask");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let store = ExtensionStore::open(&path).map_err(err)?;
        let migrations: Vec<(String, String)> = STORE_MIGRATIONS
            .iter()
            .map(|(f, s)| (f.to_string(), s.to_string()))
            .collect();
        store.migrate(&migrations).map_err(err)?;
        let (author, kind) = match thread {
            Some(t) => (
                oxplow_domain::uri::build::thread_uri(
                    t.parse::<oxplow_domain::ThreadId>()
                        .map_err(|_| format!("`{t}` isn't a thread id (thr1)"))?,
                ),
                "agent",
            ),
            None => (PERSON.to_string(), "person"),
        };
        Ok(Self {
            store,
            author,
            kind,
        })
    }

    fn rows(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String> {
        let answer = self.store.read(&stmt(sql, params, None)).map_err(err)?;
        let rows = answer.rows.into_iter().next().unwrap_or_default();
        Ok(rows
            .into_iter()
            .map(|r| match r {
                Value::Object(o) => o.into_iter().map(|(_, v)| v).collect(),
                other => vec![other],
            })
            .collect())
    }

    fn text(v: &Value) -> String {
        match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    fn exists(&self, id: &str) -> Result<(), String> {
        let there = self.rows(
            "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0",
            vec![json!(id)],
        )?;
        if there.is_empty() {
            Err(format!("no such task {id}"))
        } else {
            Ok(())
        }
    }

    /// One subcommand of `oxplow-dev task`.
    pub fn run(&self, args: &[String], usage: &str) -> Result<String, String> {
        let mut args = args.to_vec();
        let mut flag = |name: &str| -> Option<String> {
            let i = args.iter().position(|a| a == name)?;
            if i + 1 >= args.len() {
                args.remove(i);
                return None;
            }
            let v = args.remove(i + 1);
            args.remove(i);
            Some(v)
        };
        let (body, parent, state, title) = (
            flag("--body"),
            flag("--parent"),
            flag("--state"),
            flag("--title"),
        );
        let all = args.iter().any(|a| a == "--all");
        args.retain(|a| a != "--all");
        if let Some(unknown) = args.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option `{unknown}`\nusage:\n{usage}"));
        }
        let arg = |i: usize, what: &str| {
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("missing {what}\nusage:\n{usage}"))
        };
        match args.first().map(String::as_str) {
            Some("list") => self.list(all),
            Some("show") => self.show(&task_id(&arg(1, "<id>")?)?),
            Some("create") => self.create(&arg(1, "<title>")?, body, parent, state),
            Some("transition") => self.transition(&task_id(&arg(1, "<id>")?)?, &arg(2, "<state>")?),
            Some("update") => self.update(&task_id(&arg(1, "<id>")?)?, title, body, parent),
            Some("comment") => self.comment(&task_id(&arg(1, "<id>")?)?, &arg(2, "<body>")?),
            Some(verb @ ("link" | "unlink")) => self.link(
                verb == "link",
                &task_id(&arg(1, "<id>")?)?,
                &arg(2, "<target>")?,
                &arg(3, "<link type>")?,
            ),
            _ => Err(format!("usage:\n{usage}")),
        }
    }

    fn list(&self, all: bool) -> Result<String, String> {
        let sql = format!(
            "SELECT id, stage, title FROM task WHERE deleted = 0 {} ORDER BY n",
            if all {
                ""
            } else {
                "AND stage NOT IN ('done', 'canceled')"
            }
        );
        Ok(self
            .rows(&sql, vec![])?
            .iter()
            .map(|r| {
                format!(
                    "{}\t{}\t{}",
                    Self::text(&r[0]),
                    Self::text(&r[1]),
                    Self::text(&r[2])
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn show(&self, id: &str) -> Result<String, String> {
        let rows = self.rows(
            "SELECT id, stage, title, coalesce(parent, ''), coalesce(body, '')
               FROM task WHERE id = ?1 AND deleted = 0",
            vec![json!(id)],
        )?;
        let r = rows.first().ok_or_else(|| format!("no task `{id}`"))?;
        Ok(format!(
            "{}\nstate: {}\ntitle: {}\nparent: {}\n\n{}",
            Self::text(&r[0]),
            Self::text(&r[1]),
            Self::text(&r[2]),
            Self::text(&r[3]),
            Self::text(&r[4])
        ))
    }

    fn create(
        &self,
        title: &str,
        body: Option<String>,
        parent: Option<String>,
        state: Option<String>,
    ) -> Result<String, String> {
        let stage = stage_of(state.as_deref().unwrap_or("todo"))?;
        let parent = parent.as_deref().map(task_id).transpose()?;
        if let Some(p) = &parent {
            self.exists(p)?;
        }
        let sql = format!(
            "INSERT INTO task (title, body, stage, parent, author, author_kind, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                     CASE WHEN ?3 IN ('done', 'canceled') THEN {NOW} END)
             RETURNING id"
        );
        let answer = self
            .store
            .write(&[stmt(
                &sql,
                vec![
                    json!(title),
                    json!(body),
                    json!(stage),
                    json!(parent),
                    json!(self.author),
                    json!(self.kind),
                ],
                None,
            )])
            .map_err(err)?;
        let id = answer.rows[0][0]["id"]
            .as_str()
            .ok_or("the insert answered no id")?
            .to_string();
        pretty(json!({ "uri": uri_of(&id), "id": id, "state": stage }))
    }

    fn transition(&self, id: &str, to: &str) -> Result<String, String> {
        let stage = stage_of(to)?;
        self.exists(id)?;
        let sql = format!(
            "UPDATE task SET stage = ?2,
               closed_at = CASE WHEN ?2 IN ('done', 'canceled') THEN {NOW} END,
               updated_at = {NOW}
             WHERE id = ?1 AND deleted = 0"
        );
        self.store
            .write(&[stmt(&sql, vec![json!(id), json!(stage)], Some(1))])
            .map_err(err)?;
        pretty(json!({ "uri": uri_of(id), "id": id, "state": stage }))
    }

    fn update(
        &self,
        id: &str,
        title: Option<String>,
        body: Option<String>,
        parent: Option<String>,
    ) -> Result<String, String> {
        self.exists(id)?;
        let parent = parent.as_deref().map(task_id).transpose()?;
        if let Some(p) = &parent {
            self.exists(p)?;
        }
        let mut sets = Vec::new();
        let mut params = vec![json!(id)];
        for (column, value) in [
            ("title", title.map(Value::from)),
            ("body", body.map(Value::from)),
            ("parent", parent.map(Value::from)),
        ] {
            if let Some(v) = value {
                params.push(v);
                sets.push(format!("{column} = ?{}", params.len()));
            }
        }
        if sets.is_empty() {
            return Err("give at least one of --title, --body, --parent".into());
        }
        sets.push(format!("updated_at = {NOW}"));
        let sql = format!(
            "UPDATE task SET {} WHERE id = ?1 AND deleted = 0",
            sets.join(", ")
        );
        self.store
            .write(&[stmt(&sql, params, Some(1))])
            .map_err(err)?;
        pretty(json!({ "uri": uri_of(id), "id": id }))
    }

    fn comment(&self, id: &str, body: &str) -> Result<String, String> {
        self.exists(id)?;
        let answer = self
            .store
            .write(&[stmt(
                "INSERT INTO task_comment (task_id, author, author_kind, body)
                 VALUES (?1, ?2, ?3, ?4) RETURNING id",
                vec![json!(id), json!(self.author), json!(self.kind), json!(body)],
                Some(1),
            )])
            .map_err(err)?;
        let comment = answer.rows[0][0]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        pretty(json!({ "uri": uri_of(id), "id": id, "comment": comment }))
    }

    fn link(&self, add: bool, id: &str, target: &str, kind: &str) -> Result<String, String> {
        if !LINK_TYPES.contains(&kind) {
            return Err(format!(
                "`{kind}` isn't a link type ({})",
                LINK_TYPES.join(", ")
            ));
        }
        // A bare task id is another task; anything with a scheme is a URI.
        let to = if target.contains(':') {
            target.to_string()
        } else {
            uri_of(&task_id(target)?)
        };
        let statement = if add {
            self.exists(id)?;
            stmt(
                "INSERT INTO task_link (task_id, to_uri, type) VALUES (?1, ?2, ?3)
                 ON CONFLICT(task_id, to_uri, type) DO UPDATE SET deleted = 0",
                vec![json!(id), json!(to), json!(kind)],
                Some(1),
            )
        } else {
            stmt(
                "UPDATE task_link SET deleted = 1
                  WHERE task_id = ?1 AND to_uri = ?2 AND type = ?3 AND deleted = 0",
                vec![json!(id), json!(to), json!(kind)],
                Some(1),
            )
        };
        self.store.write(&[statement]).map_err(err)?;
        pretty(json!({ "uri": uri_of(id), "id": id, "to": to, "type": kind }))
    }
}

fn pretty(v: Value) -> Result<String, String> {
    serde_json::to_string_pretty(&v).map_err(err)
}
