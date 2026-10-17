//! `v_agent_plan` (`.context/agent-model.md` "Plans"): the plans an agent
//! put to the person, read from `agent.plan.proposed`.

use oxplow_domain::HookKind;
use serde_json::json;

use crate::test_fixtures::{services_with_effort, EffortFixture};
use crate::{HookEnvelope, ToolDecision};

fn envelope(fx: &EffortFixture, kind: HookKind, body: serde_json::Value) -> HookEnvelope {
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
        subagent: None,
        peer: None,
    }
}

/// A plan Claude proposes is a row with its thread, effort, turn, text and
/// file; the row's event is the plan's ref.
#[tokio::test]
async fn a_proposed_plan_is_a_row_with_its_effort_and_turn() {
    let fx = services_with_effort().await;
    let ingest = |e: HookEnvelope| {
        let svc = fx.svc.clone();
        async move { svc.hook_ingest.ingest(e).await.unwrap() }
    };
    ingest(envelope(&fx, HookKind::UserPromptSubmit, json!({}))).await;
    ingest(envelope(
        &fx,
        HookKind::PreToolUse,
        json!({ "tool_name": "ExitPlanMode", "tool_use_id": "t1",
                "tool_input": { "plan": "1. Model\n2. Lens", "planFilePath": "/h/p.md" } }),
    ))
    .await;
    let rows = fx
        .svc
        .sql
        .query_sql(
            "SELECT thread_id, effort_id IS NOT NULL, turn_id IS NOT NULL, text, path,
                    event_id, proposed_at IS NOT NULL
               FROM v_agent_plan",
            vec![],
            None,
        )
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1, "{rows:?}");
    use oxplow_db::SqlCell::{Int, Text};
    let row = &rows[0];
    assert_eq!(row[1], Int(1), "an open effort");
    assert_eq!(row[2], Int(1), "an open turn");
    assert_eq!(row[3], Text("1. Model\n2. Lens".into()));
    assert_eq!(row[4], Text("/h/p.md".into()));
    assert_eq!(row[6], Int(1));
    // The row's event is the proposal itself.
    let Text(event) = &row[5] else {
        panic!("{row:?}")
    };
    let kinds = fx
        .svc
        .sql
        .query_sql(
            "SELECT type FROM v_event WHERE id = ?1",
            vec![Text(event.clone())],
            None,
        )
        .await
        .unwrap()
        .rows;
    assert_eq!(kinds, [vec![Text("agent.plan.proposed".into())]]);
}
