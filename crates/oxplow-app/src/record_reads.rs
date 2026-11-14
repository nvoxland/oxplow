//! Reads of the rows of shared models, whatever the model: an agent's read
//! through `oxplow_read` and its own read of the file a store keeps a row in
//! each log `record.read` (`.context/models.md` "Reads"). What keeps reads
//! of a model's rows follows that event, not a capability.

use std::path::{Path, PathBuf};

use oxplow_db::EventCtx;
use oxplow_domain::events::schema::{
    AgentToolFinished, AgentToolFinishedV4, ReadVia, RecordRead, RecordReadV1,
};
use oxplow_domain::DomainError;
use rusqlite::OptionalExtension;

use crate::models::Models;

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}

/// Whether row `uri` of model `model` (its id) is there.
fn row_exists(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    model: &str,
    uri: &str,
) -> Result<bool, DomainError> {
    let Some(spec) = registry.models.get(model) else {
        return Ok(false);
    };
    Ok(conn
        .query_row(
            &format!(
                "SELECT 1 FROM \"{}\" WHERE uri = ?1 AND removed_at IS NULL",
                spec.decl.table()
            ),
            [uri],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?
        .is_some())
}

/// Log an agent's read of the row `uri` through a tool (`oxplow_read`):
/// `record.read { via: tool }`, as the actor, anchored to its open turn. Only
/// a row of a model is one: any other resource is nothing here. A read
/// command runs unrecorded, so it appends this itself.
pub async fn log_read(
    svc: &crate::Services,
    actor: &oxplow_domain::Actor,
    uri: &str,
) -> Result<(), DomainError> {
    let Ok(parsed) = oxplow_domain::uri::OxplowUri::parse(uri) else {
        return Ok(());
    };
    let registry = svc.models.current();
    let resource = parsed.resource().to_string();
    if !registry
        .models
        .values()
        .any(|m| m.decl.resource == resource)
    {
        return Ok(());
    }
    let (vocabulary, source, actor, record) = (
        svc.vocabulary.clone(),
        actor.source(),
        actor.clone(),
        uri.to_string(),
    );
    svc.db
        .transaction(move |tx| {
            let anchors = match &actor {
                oxplow_domain::Actor::Agent {
                    thread_id: Some(thread),
                    session_id,
                    ..
                } => oxplow_db::agent_stores::activity_anchors_tx(tx, *thread, *session_id)?,
                other => other.anchors(),
            };
            let vocabulary = vocabulary.current();
            let env = EventCtx {
                vocabulary: &vocabulary,
                source: source.clone(),
                cause: None,
            }
            .typed::<RecordRead>(&RecordReadV1 {
                record: record.clone(),
                via: ReadVia::Tool,
            })
            .with_anchors(anchors)
            .with_subject([record.clone()]);
            oxplow_db::event_log_store::append_tx(tx, &vocabulary, &env).map(|_| ())
        })
        .await
}

/// An agent reading the file a store keeps a row in, with its own tool, reads
/// the row: each `agent.tool.finished` of a file read whose path a fill's
/// `files` names as a row's file (`ModelRegistry::row_of_file`) logs
/// `record.read { via: file }` for the row, caused by the tool call, with its
/// anchors. A path is the worktree's (relative) or absolute; one that is no
/// row's file, or names a row that isn't there, is nothing.
pub struct RecordFileReads {
    pub models: Models,
    pub project_dir: PathBuf,
    /// Where each fill's `files` folder's root is.
    pub roots: std::sync::Arc<dyn oxplow_domain::folder_root::FolderBases>,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

impl RecordFileReads {
    pub const NAME: &'static str = "record.file_reads";
}

impl crate::event_pump::EventConsumer for RecordFileReads {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <AgentToolFinished as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let Ok(tool) =
            serde_json::from_value::<AgentToolFinishedV4>(event.envelope.payload.clone())
        else {
            return Ok(());
        };
        if tool.kind != oxplow_domain::agent::tool::ToolKind::Read || tool.paths.is_empty() {
            return Ok(());
        }
        let registry = self.models.current();
        if !registry.fills.iter().any(|f| f.files.is_some()) {
            return Ok(());
        }
        // A relative path is the worktree's the agent ran in.
        let worktree: PathBuf = match event.envelope.anchors.stream_id {
            Some(stream) => conn
                .query_row(
                    "SELECT worktree_path FROM streams WHERE id = ?1",
                    [stream.value()],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(sql)?
                .map_or_else(|| self.project_dir.clone(), PathBuf::from),
            None => self.project_dir.clone(),
        };
        let vocabulary = self.vocabulary.current();
        let ev = EventCtx {
            vocabulary: &vocabulary,
            source: event.envelope.source.clone(),
            cause: Some(event.envelope.id.clone()),
        };
        for path in &tool.paths {
            let path = Path::new(path);
            let abs = if path.is_absolute() {
                path.to_path_buf()
            } else {
                worktree.join(path)
            };
            let Some((model, row)) = registry.row_of_file(&*self.roots, &abs) else {
                continue;
            };
            if !row_exists(conn, &registry, &model, &row)? {
                continue;
            }
            let env = ev
                .typed::<RecordRead>(&RecordReadV1 {
                    record: row.clone(),
                    via: ReadVia::File,
                })
                .with_anchors(event.envelope.anchors.clone())
                .with_subject([row]);
            ev.append(conn, &env)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    use crate::extension::extension_private_state_tests::write;
    use crate::test_fixtures::services_with_thread;

    const OWNER: &str = "manifest: 2\nname: shelf\nintent:\n  purpose: p\nresources:\n  - name: book\n    description: A book.\n    label: Book\n    params: { store: { required: true }, id: { required: true } }\nmodels:\n  - name: book\n    resource: book\n    open: true\n    key: id\n    change_event: shelf.book.changed\n    columns: { id: text, title: text }\n";

    const STORE: &str = "manifest: 2\nname: shelves\nintent:\n  purpose: p\nrequires: [shelf]\nfills:\n  - model: shelf/book\n    stores: [shelves]\n    files: { folder: .oxplow/shelves, suffix: .md }\ncollectors:\n  - id: books\n    runtime: starlark\n    entry: books.star\n";

    const DUNE: &str = "oxplow:shelf/book?id=dune&store=shelves";

    async fn reads(fx: &crate::test_fixtures::ThreadFixture) -> Vec<(String, String)> {
        fx.svc
            .db
            .read(|c| {
                let mut st = c
                    .prepare(
                        "SELECT json_extract(payload, '$.record'), json_extract(payload, '$.via')
                           FROM event_log WHERE type = 'record.read' ORDER BY seq",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// An agent reading the file a store keeps a row in reads the row (a
    /// path relative to its worktree, or absolute); a file no fill names, or
    /// a row that isn't there, is no read. `oxplow_read` of a row is a read
    /// too, and of anything that is no model's row is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_reads_of_a_rows_file_and_of_the_row_are_reads_of_it() {
        use oxplow_domain::events::schema::{AgentToolFinished, AgentToolFinishedV4};
        let fx = services_with_thread().await;
        let root = fx._dir.path();
        write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
        write(root, ".oxplow/extensions/shelves/extension.yaml", STORE);
        write(
            root,
            ".oxplow/extensions/shelves/books.star",
            "def transform(x):\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"shelves\", \"snapshot\": [{\"id\": \"dune\", \"title\": \"Dune\"}]}]}\n",
        );
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::collector_runner::SYNC,
                json!({ "collector": "shelves/books" }),
                false,
            )
            .await
            .map_err(|e: CommandError| e)
            .unwrap();
        let vocabulary = fx.svc.vocabulary.current();
        let absolute = root.join(".oxplow/shelves/dune.md").display().to_string();
        for paths in [
            vec![".oxplow/shelves/dune.md".to_string()],
            vec![absolute],
            vec![
                "src/lib.rs".to_string(),
                ".oxplow/shelves/never-there.md".to_string(),
                ".oxplow/shelves/dune.txt".to_string(),
            ],
        ] {
            let env = oxplow_db::EventCtx {
                vocabulary: &vocabulary,
                source: "agent:claude".into(),
                cause: None,
            }
            .typed::<AgentToolFinished>(&AgentToolFinishedV4 {
                tool: "Read".into(),
                kind: oxplow_domain::agent::tool::ToolKind::Read,
                call: None,
                duration_ms: None,
                paths,
                detail: None,
                command: None,
                ok: Some(true),
                exit_code: None,
                input: None,
                output: None,
                subagent: None,
            })
            .with_anchors(oxplow_domain::Anchors {
                thread_id: Some(fx.thread),
                ..Default::default()
            });
            fx.svc.event_log_store.append(env).await.unwrap();
        }
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            reads(&fx).await,
            [
                (DUNE.to_string(), "file".to_string()),
                (DUNE.to_string(), "file".to_string())
            ]
        );
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        super::log_read(&fx.svc, &agent, DUNE).await.unwrap();
        super::log_read(&fx.svc, &agent, "oxplow:oxplow/file?path=src/a.rs")
            .await
            .unwrap();
        let all = reads(&fx).await;
        assert_eq!(all.len(), 3, "{all:?}");
        assert_eq!(all[2], (DUNE.to_string(), "tool".to_string()));
    }

    /// A store whose files are in a folder outside the project (an agent
    /// harness's own) has its rows read when an agent reads one of those
    /// files by its absolute path; the same path under the project is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_of_a_rows_file_outside_the_project_is_a_read_of_it() {
        use oxplow_domain::events::schema::{AgentToolFinished, AgentToolFinishedV4};
        use oxplow_domain::folder_root::FolderRoot;
        let fx = services_with_thread().await;
        let root = fx._dir.path();
        write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
        write(
            root,
            ".oxplow/extensions/memories/extension.yaml",
            "manifest: 2\nname: memories\nintent:\n  purpose: p\nrequires: [shelf]\nfiles:\n  - { root: 'agent:claude', path: memory, access: read }\nfills:\n  - model: shelf/book\n    stores: [memories]\n    files: { root: 'agent:claude', folder: memory, suffix: .md }\ncollectors:\n  - id: books\n    runtime: starlark\n    entry: books.star\n",
        );
        write(
            root,
            ".oxplow/extensions/memories/books.star",
            "def transform(x):\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"memories\", \"snapshot\": [{\"id\": \"m1\", \"title\": \"M\"}]}]}\n",
        );
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::collector_runner::SYNC,
                json!({ "collector": "memories/books" }),
                false,
            )
            .await
            .unwrap();
        let base = fx
            .svc
            .folder_base(&FolderRoot::Agent("claude".into()))
            .expect("a test's agent folder");
        let vocabulary = fx.svc.vocabulary.current();
        for path in [
            base.join("memory/m1.md"),
            root.join("memory/m1.md"),
            base.join("memory/gone.md"),
        ] {
            let env = oxplow_db::EventCtx {
                vocabulary: &vocabulary,
                source: "agent:claude".into(),
                cause: None,
            }
            .typed::<AgentToolFinished>(&AgentToolFinishedV4 {
                tool: "Read".into(),
                kind: oxplow_domain::agent::tool::ToolKind::Read,
                call: None,
                duration_ms: None,
                paths: vec![path.display().to_string()],
                detail: None,
                command: None,
                ok: Some(true),
                exit_code: None,
                input: None,
                output: None,
                subagent: None,
            })
            .with_anchors(oxplow_domain::Anchors {
                thread_id: Some(fx.thread),
                ..Default::default()
            });
            fx.svc.event_log_store.append(env).await.unwrap();
        }
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            reads(&fx).await,
            [(
                "oxplow:shelf/book?id=m1&store=memories".to_string(),
                "file".to_string()
            )]
        );
    }
}
