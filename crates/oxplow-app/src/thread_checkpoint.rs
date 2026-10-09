//! The `thread.checkpoint` consumer (`.context/work-tracking.md`): once a
//! turn's end take lands (`snapshot.taken` with `trigger: turn_end`,
//! logged even when the take outruns the Stop hook's budget), log
//! `thread.checkpoint` with what an effort policy needs and nothing it
//! would have to dig for: whether the worktree differs from where the turn
//! began, and how many of the turn's tool calls could have changed it.
//!
//! "Changed" compares the turn's start snapshot with the take's: snapshots
//! are content-addressed, so an unchanged tree is the same id. Which tools
//! can change the worktree is harness knowledge kept here, so a policy
//! never reads tool names.

use async_trait::async_trait;
use oxplow_db::{SqlCell, SqliteEventLogStore};
use oxplow_domain::events::schema::{
    CheckpointReason, EventType as _, SnapshotTaken, SnapshotTakenV2, ThreadCheckpoint,
    ThreadCheckpointV1,
};
use oxplow_domain::refs::build::{snapshot_ref, system_source, thread_ref, turn_ref};
use oxplow_domain::snapshot::SnapshotTrigger;
use oxplow_domain::{AgentTurnId, DomainError, Envelope, StoredEvent, ThreadId};

use oxplow_domain::agent::tool::ToolKind;

use crate::event_pump::AsyncEventConsumer;
use crate::sql_gateway::SqlGateway;

/// The consumer's name (its checkpoint key; what callers settle on).
pub const NAME: &str = "thread.checkpoint";

/// Whether a call could have changed the worktree: an edit, a shell
/// command or a subagent (`ToolKind::changes_worktree`), whatever its
/// harness calls it, or oxplow's own `run_command` (a command may write).
pub fn can_write(kind: &str, tool: &str) -> bool {
    match serde_json::from_value::<ToolKind>(serde_json::Value::String(kind.to_string())) {
        Ok(ToolKind::Mcp) => tool.ends_with("run_command"),
        Ok(k) => k.changes_worktree(),
        Err(_) => false,
    }
}

pub struct ThreadCheckpointConsumer {
    pub log: SqliteEventLogStore,
    pub sql: SqlGateway,
}

impl ThreadCheckpointConsumer {
    /// The turn's thread and start snapshot, if the turn exists.
    async fn turn(
        &self,
        turn: AgentTurnId,
    ) -> Result<Option<(ThreadId, Option<i64>)>, DomainError> {
        let rows = self
            .sql
            .query_sql(
                "SELECT thread_id, start_snapshot_id FROM v_agent_turn WHERE id = ?1",
                vec![SqlCell::Int(turn.value())],
                None,
            )
            .await?
            .rows;
        Ok(rows.into_iter().next().and_then(|row| match &row[..] {
            [SqlCell::Int(thread), start] => Some((
                ThreadId::new(*thread),
                match start {
                    SqlCell::Int(s) => Some(*s),
                    _ => None,
                },
            )),
            _ => None,
        }))
    }

    async fn writing_tools(&self, turn: AgentTurnId) -> Result<u32, DomainError> {
        let rows = self
            .sql
            .query_sql(
                "SELECT kind, tool FROM v_tool_call WHERE turn_id = ?1",
                vec![SqlCell::Int(turn.value())],
                None,
            )
            .await?
            .rows;
        let n = rows
            .iter()
            .filter(|row| matches!(&row[..], [SqlCell::Text(kind), SqlCell::Text(tool)] if can_write(kind, tool)))
            .count();
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

#[async_trait]
impl AsyncEventConsumer for ThreadCheckpointConsumer {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == SnapshotTaken::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let anchors = &event.envelope.anchors;
        let (Some(turn), Some(snapshot)) = (anchors.turn_id, anchors.snapshot_id) else {
            return Ok(());
        };
        let take: SnapshotTakenV2 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("snapshot.taken seq {}: {e}", event.seq)))?;
        if take.trigger != SnapshotTrigger::TurnEnd {
            return Ok(());
        }
        let turn = AgentTurnId::new(turn);
        let Some((thread, start)) = self.turn(turn).await? else {
            return Ok(());
        };
        let env = Envelope::typed::<ThreadCheckpoint>(
            system_source(NAME),
            &ThreadCheckpointV1 {
                thread: thread_ref(thread),
                turn: turn_ref(turn),
                reason: CheckpointReason::TurnEnd,
                snapshot: snapshot_ref(snapshot),
                changed: start != Some(snapshot),
                writing_tools: self.writing_tools(turn).await?,
            },
        )
        .with_anchors(oxplow_domain::Anchors {
            thread_id: Some(thread),
            turn_id: Some(turn.value()),
            effort_id: None,
            ..anchors.clone()
        })
        .with_subject([thread_ref(thread), turn_ref(turn)])
        .with_cause(event.envelope.id.clone())
        .with_dedupe_key(format!("thread.checkpoint:{turn}"));
        // A re-delivered take logs the same checkpoint: once.
        match self.log.append(env).await {
            Ok(_) | Err(DomainError::Constraint(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hook_ingest::HookEnvelope;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::hook::HookKind;
    use oxplow_domain::stores::AgentTurnStore as _;

    fn envelope(kind: HookKind, thread: ThreadId) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
            decision: None,
            tool: None,
            subagent: None,
        }
    }

    /// Run one turn on the fixture's thread: `edit` writes a file before it
    /// ends, `tools` are the kinds of the calls it made (`edit`, `shell`,
    /// …). The checkpoint it logged.
    pub(crate) async fn turn(
        f: &EffortFixture,
        edit: Option<(&str, &str)>,
        tools: &[&str],
    ) -> ThreadCheckpointV1 {
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(envelope(HookKind::UserPromptSubmit, f.thread))
            .await
            .unwrap();
        let turn = svc.agent_turn_store.list_open(&f.thread).await.unwrap()[0].id;
        for tool in tools {
            svc.tool_call_store
                .record(oxplow_db::NewToolCall {
                    thread_id: f.thread.value(),
                    turn_id: Some(turn.value()),
                    tool: tool.to_string(),
                    kind: tool.to_string(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        if let Some((path, body)) = edit {
            std::fs::write(svc.layout.project_dir.join(path), body).unwrap();
            let stream = svc.streams.list_streams().await.unwrap()[0].id;
            svc.snapshot_captures
                .get(&stream)
                .unwrap()
                .enqueue_startup_diff()
                .await
                .unwrap();
        }
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();
        svc.event_pump
            .settle(&[NAME], std::time::Duration::from_secs(10))
            .await;
        let events = svc.event_log_store.read_after(0, 10_000).await.unwrap();
        let logged = events
            .iter()
            .rev()
            .find(|e| {
                e.envelope.event_type == ThreadCheckpoint::TYPE
                    && e.envelope.anchors.turn_id == Some(turn.value())
            })
            .expect("a checkpoint for the turn");
        serde_json::from_value(logged.envelope.payload.clone()).unwrap()
    }

    /// The fixture with a baseline snapshot of its worktree, as a running
    /// app has from its startup sweep.
    pub(crate) async fn with_baseline() -> EffortFixture {
        let f = services_with_effort().await;
        let stream = f.svc.streams.list_streams().await.unwrap()[0].id;
        let capture = f.svc.snapshot_captures.get(&stream).unwrap();
        std::fs::write(f.svc.layout.project_dir.join("seed.txt"), "seed").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
            .await
            .unwrap();
        f
    }

    /// A turn that edited logs a changed checkpoint counting its writing
    /// calls; one that only read changes nothing and counts none.
    #[tokio::test]
    async fn a_turn_end_logs_what_changed_and_what_could_have() {
        let f = with_baseline().await;
        let edited = turn(
            &f,
            Some(("made.txt", "by the agent")),
            &["read", "edit", "shell"],
        )
        .await;
        assert!(edited.changed);
        assert_eq!(edited.writing_tools, 2);
        assert_eq!(edited.reason, CheckpointReason::TurnEnd);
        assert_eq!(edited.thread, thread_ref(f.thread));

        let asked = turn(&f, None, &["read", "fetch"]).await;
        assert!(!asked.changed);
        assert_eq!(asked.writing_tools, 0);
    }

    /// A call counts as writing by its kind, whatever its harness calls
    /// it; of the MCP calls, oxplow's own `run_command`.
    #[test]
    fn writing_calls_are_read_by_kind() {
        for (kind, tool) in [
            ("edit", "apply_patch"),
            ("shell", "Bash"),
            ("subagent", "Task"),
            ("mcp", "mcp__oxplow__run_command"),
        ] {
            assert!(can_write(kind, tool), "{kind} {tool}");
        }
        for (kind, tool) in [
            ("read", "Read"),
            ("search", "Grep"),
            ("fetch", "WebFetch"),
            ("mcp", "mcp__oxplow__query_sql"),
            ("other", "TodoWrite"),
            ("nonsense", "Edit"),
        ] {
            assert!(!can_write(kind, tool), "{kind} {tool}");
        }
    }
}
