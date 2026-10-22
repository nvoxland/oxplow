//! `v_subagent` and a subagent's `#result` (`.context/agent-model.md`
//! "Subagents"): who in a session acted, bracketed or only named by calls.

use oxplow_domain::agent::tool::Subagent;
use oxplow_domain::HookKind;
use serde_json::json;

use crate::test_fixtures::{services_with_effort, EffortFixture};
use crate::{HookEnvelope, ToolDecision};

fn envelope(
    fx: &EffortFixture,
    kind: HookKind,
    body: serde_json::Value,
    subagent: Option<&str>,
) -> HookEnvelope {
    HookEnvelope {
        session_title: None,
        turn_run: None,
        kind,
        thread_id: Some(fx.thread),
        stream_id: None,
        agent_session_id: Some(fx.session),
        session_id: Some("native".into()),
        payload_json: body.to_string(),
        prompt: None,
        decision: None::<ToolDecision>,
        tool: crate::test_fixtures::claude_tool(kind, &body),
        subagent: subagent.map(|id| Subagent {
            id: id.into(),
            kind: Some("Explore".into()),
        }),
        peer: None,
    }
}

async fn subagents(fx: &EffortFixture) -> Vec<Vec<oxplow_db::SqlCell>> {
    fx.svc
        .sql
        .query_sql(
            "SELECT uri, id, kind, state, calls, finished_event_id IS NOT NULL
               FROM v_subagent ORDER BY id",
            vec![],
            None,
        )
        .await
        .unwrap()
        .rows
}

fn text(c: &oxplow_db::SqlCell) -> String {
    match c {
        oxplow_db::SqlCell::Text(t) => t.clone(),
        other => panic!("not text: {other:?}"),
    }
}

/// A subagent Claude brackets is `running`, then `finished`, with the
/// calls made as it; one only a tool call names is `unknown`.
#[tokio::test]
async fn a_subagent_is_running_finished_or_only_named_by_its_calls() {
    let fx = services_with_effort().await;
    let ingest = |e: HookEnvelope| {
        let svc = fx.svc.clone();
        async move { svc.hook_ingest.ingest(e).await.unwrap() }
    };
    ingest(envelope(&fx, HookKind::UserPromptSubmit, json!({}), None)).await;
    // The prompt's text rides the envelope's `prompt`.
    ingest(envelope(
        &fx,
        HookKind::SubagentStart,
        json!({}),
        Some("a1"),
    ))
    .await;
    // The calls' rows are projected by a consumer the test doesn't run.
    let (thread, session) = (fx.thread.value(), fx.session.value());
    fx.svc
        .db
        .transaction(move |tx| {
            for child in ["a1", "codex-child"] {
                tx.execute(
                    "INSERT INTO agent_tool_call
                       (thread_id, tool, kind, path, at, agent_session_id, subagent_id, subagent_kind)
                     VALUES (?1, 'Read', 'read', 'a.txt', '2026-10-01T00:00:00.000000Z', ?2, ?3, 'Explore')",
                    rusqlite::params![thread, session, child],
                )
                .map_err(oxplow_db::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let rows = subagents(&fx).await;
    let states: Vec<(String, String)> = rows.iter().map(|r| (text(&r[1]), text(&r[3]))).collect();
    assert_eq!(
        states,
        [
            ("a1".to_string(), "running".to_string()),
            ("codex-child".to_string(), "unknown".to_string())
        ],
        "{rows:?}"
    );
    assert_eq!(
        text(&rows[0][0]),
        oxplow_domain::uri::build::of(
            oxplow_domain::uri::resources::SUBAGENT,
            &[
                ("session", &format!("ses{}", fx.session.value())),
                ("agent", "a1")
            ]
        )
    );

    ingest(envelope(
        &fx,
        HookKind::SubagentStop,
        json!({ "last_assistant_message": "the leak is in the gasket" }),
        Some("a1"),
    ))
    .await;
    let rows = subagents(&fx).await;
    assert_eq!(text(&rows[0][3]), "finished");
    assert_eq!(rows[0][4], oxplow_db::SqlCell::Int(1), "its one call");
    assert_eq!(
        rows[0][5],
        oxplow_db::SqlCell::Int(1),
        "the event that finished it"
    );

    // Its last message is `#result`, to the person and to an agent of the stream.
    let reference = format!(
        "{}#part=result",
        oxplow_domain::uri::build::of(
            oxplow_domain::uri::resources::SUBAGENT,
            &[
                ("session", &format!("ses{}", fx.session.value())),
                ("agent", "a1")
            ]
        )
    );
    let read = crate::ref_read::read(
        &fx.svc,
        &reference,
        &crate::ref_read::Reader {
            thread: None,
            stream: Some(oxplow_domain::StreamId::new(1)),
        },
        None,
    )
    .await
    .unwrap();
    // As an event's body reads: the text with its size.
    let body: serde_json::Value = serde_json::from_str(read.content.as_deref().unwrap()).unwrap();
    assert_eq!(body["text"], "the leak is in the gasket");
}
