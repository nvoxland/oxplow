//! Inferred decisions: when an effort closes, oxplow's `main` model reads
//! what the agent did (its turns and tool calls) and proposes the
//! decisions it made without recording them — a recorded `extract`
//! (`ai_compute`), so an unchanged effort isn't asked about twice. They're
//! stored with provenance `inferred` for the review packet, never fed back
//! to the agent. Off until a `main` model is assigned. See
//! `.context/ai-providers.md` → "Inferred decisions".

use oxplow_db::{NewDecision, SqlCell};
use serde::Deserialize;

use crate::ai_compute::AiComputeError;
use crate::ai_service::{AiServiceError, Role};

/// Caller name on the `v_ai_call` rows this makes.
pub const CALLER: &str = "inferred-decisions";
/// Most proposals kept per effort.
pub const MAX_PROPOSALS: usize = 8;
/// Longest activity digest sent to the model, in characters.
pub const MAX_DIGEST_CHARS: usize = 40_000;
/// Longest single prompt or answer kept in the digest.
const MAX_TURN_CHARS: usize = 2_000;

/// One tool call, as the digest shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCallLine {
    pub tool: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ok: Option<bool>,
}

/// What the agent did during one effort.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffortActivity {
    pub thread_id: i64,
    pub task_id: Option<i64>,
    pub task_title: String,
    /// (what the human asked, what the agent answered).
    pub turns: Vec<(String, String)>,
    pub tool_calls: Vec<ToolCallLine>,
    /// Questions of decisions the agent already recorded.
    pub recorded: Vec<String>,
}

pub const SYSTEM_PROMPT: &str = "You review a coding agent's work session. Find the \
decisions it made without asking: forks where it picked one approach over another \
(where to put code, which library, what to change or leave, how to handle a case). \
Only real forks with a plausible alternative; skip routine steps. Don't repeat decisions \
it already recorded. For each: `question` is what had to be decided, `choice` what it \
chose, `alternatives` the options not taken, `why` its reasoning if visible, and \
`confidence` (low, medium or high) how sure you are it was a real decision. At most 8; \
an empty list is fine.";

/// The shape `extract` asks the model for.
pub fn reply_schema() -> serde_json::Value {
    let text = serde_json::json!({ "type": "string" });
    serde_json::json!({
        "type": "object",
        "required": ["decisions"],
        "properties": {
            "decisions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "question": text,
                        "choice": text,
                        "alternatives": { "type": "array", "items": text },
                        "why": text,
                        "confidence": text
                    }
                }
            }
        }
    })
}

/// The user prompt for the model, or `None` when there's nothing to read.
pub fn build_prompt(activity: &EffortActivity) -> Option<String> {
    if activity.turns.is_empty() && activity.tool_calls.is_empty() {
        return None;
    }
    let mut head = format!("Task: {}\n", activity.task_title);
    if !activity.recorded.is_empty() {
        head.push_str("\nDecisions it already recorded (don't repeat these):\n");
        for q in &activity.recorded {
            head.push_str(&format!("- {q}\n"));
        }
    }
    let tool_lines: Vec<String> = activity
        .tool_calls
        .iter()
        .map(|c| {
            let mut line = c.tool.clone();
            if let Some(p) = &c.path {
                line.push(' ');
                line.push_str(p);
            }
            if let Some(d) = &c.detail {
                line.push_str(": ");
                line.push_str(&clip(d, 200));
            }
            if c.ok == Some(false) {
                line.push_str(" (failed)");
            }
            line
        })
        .collect();
    let tools = newest_that_fit(&tool_lines, MAX_DIGEST_CHARS / 4);
    let turn_blocks: Vec<String> = activity
        .turns
        .iter()
        .map(|(ask, answer)| {
            format!(
                "Human: {}\nAgent: {}\n",
                clip(ask, MAX_TURN_CHARS),
                clip(answer, MAX_TURN_CHARS)
            )
        })
        .collect();
    let room = MAX_DIGEST_CHARS
        .saturating_sub(head.chars().count())
        .saturating_sub(tools.iter().map(|l| l.chars().count() + 1).sum());
    let turns = newest_that_fit(&turn_blocks, room);
    let mut out = head;
    out.push_str("\nConversation (oldest first):\n");
    out.push_str(&turns.join("\n"));
    out.push_str("\nTool calls (oldest first):\n");
    out.push_str(&tools.join("\n"));
    Some(out)
}

/// The most recent `items` whose total length fits `budget`, oldest first.
fn newest_that_fit(items: &[String], budget: usize) -> Vec<String> {
    let mut used = 0;
    let mut kept: Vec<String> = Vec::new();
    for item in items.iter().rev() {
        let len = item.chars().count() + 1;
        if used + len > budget {
            break;
        }
        used += len;
        kept.push(item.clone());
    }
    kept.reverse();
    kept
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// The model's reply as decisions for `thread_id` / `task_id`: entries
/// without a question or choice are skipped, a missing or unknown
/// confidence is `low`, and at most [`MAX_PROPOSALS`] are kept.
pub fn parse_proposals(
    reply: &serde_json::Value,
    thread_id: i64,
    task_id: Option<i64>,
) -> Result<Vec<NewDecision>, String> {
    let reply: Reply = serde_json::from_value(reply.clone())
        .map_err(|e| format!("the model's reply wasn't the expected JSON ({e})"))?;
    let str_of =
        |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().trim().to_string();
    Ok(reply
        .decisions
        .iter()
        .filter(|d| !str_of(d, "question").is_empty() && !str_of(d, "choice").is_empty())
        .take(MAX_PROPOSALS)
        .map(|d| NewDecision {
            thread_id,
            task_id,
            effort_id: None,
            question: str_of(d, "question"),
            choice: str_of(d, "choice"),
            alternatives: d["alternatives"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            confidence: match str_of(d, "confidence").as_str() {
                c @ ("low" | "medium" | "high") => c.to_string(),
                _ => "low".to_string(),
            },
            why: str_of(d, "why"),
        })
        .collect())
}

/// What an inference pass did.
#[derive(Debug, Clone, PartialEq)]
pub enum InferOutcome {
    /// No `main` model is assigned; nothing ran.
    Off,
    /// Nothing to read for this effort.
    NoActivity,
    /// Stored this many proposals (replacing earlier ones).
    Stored(usize),
}

/// Gather `effort_id`'s activity, extract its decisions with the `main`
/// model (recorded: the same activity isn't asked about twice), and
/// replace the effort's inferred decisions.
pub async fn infer_for_effort(
    svc: &crate::Services,
    effort_id: i64,
) -> Result<InferOutcome, String> {
    let configured = svc
        .ai
        .config()
        .map_err(|e| e.to_string())?
        .resolve(Role::Main)
        .is_some();
    if !configured {
        return Ok(InferOutcome::Off);
    }
    let layer = svc.sql.clone();
    let activity = gather(&layer, effort_id).await?;
    let Some(prompt) = build_prompt(&activity) else {
        return Ok(InferOutcome::NoActivity);
    };
    let reply = match svc
        .ai_compute
        .extract(CALLER, SYSTEM_PROMPT, &prompt, &reply_schema())
        .await
    {
        Ok(recorded) => recorded.value,
        // Unassigned between the check and the call.
        Err(AiComputeError::Ai(AiServiceError::NotConfigured(_))) => return Ok(InferOutcome::Off),
        Err(e) => return Err(e.to_string()),
    };
    let proposals = parse_proposals(&reply, activity.thread_id, activity.task_id)?;
    let stored = svc
        .reasoning_store
        .replace_inferred(effort_id, proposals)
        .await
        .map_err(|e| e.to_string())?;
    Ok(InferOutcome::Stored(stored))
}

/// Read what the agent did during `effort_id`.
pub async fn gather(
    layer: &crate::sql_gateway::SqlGateway,
    effort_id: i64,
) -> Result<EffortActivity, String> {
    let q = |sql: &'static str| async move {
        layer
            .query_sql(sql, vec![SqlCell::Int(effort_id)], Some(2_000))
            .await
            .map(|r| r.rows)
            .map_err(|e| e.to_string())
    };
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => t.clone(),
        _ => String::new(),
    };
    let int = |c: &SqlCell| match c {
        SqlCell::Int(i) => Some(*i),
        _ => None,
    };
    let effort = q(
        "SELECT e.thread_id, e.task_id, coalesce(t.title, '') FROM v_effort e
                    LEFT JOIN v_task t ON t.id = e.task_id WHERE e.id = ?1",
    )
    .await?;
    let Some(effort) = effort.first() else {
        return Err(format!("no effort {effort_id}"));
    };
    // Newest first under the row cap, so a long effort keeps its latest
    // activity (tsk369); reversed back to chronological order below.
    // The effort's thread's turns that overlap its time window.
    let mut turns = q(
        "SELECT a.prompt, coalesce(a.answer, '') FROM v_agent_turn a, v_effort e
                   WHERE e.id = ?1 AND a.thread_id = e.thread_id
                     AND a.started_at <= coalesce(e.ended_at, '9999')
                     AND coalesce(a.ended_at, '9999') >= e.started_at
                   ORDER BY a.started_at DESC, a.id DESC",
    )
    .await?;
    let mut tools =
        q("SELECT tool, path, detail, ok FROM v_tool_call WHERE effort_id = ?1 ORDER BY id DESC")
            .await?;
    turns.reverse();
    tools.reverse();
    let recorded = q("SELECT question FROM v_decision
                      WHERE effort_id = ?1 AND provenance = 'recorded' ORDER BY id")
    .await?;
    let opt_text = |c: &SqlCell| match c {
        SqlCell::Text(t) => Some(t.clone()),
        _ => None,
    };
    Ok(EffortActivity {
        thread_id: int(&effort[0]).unwrap_or_default(),
        task_id: int(&effort[1]),
        task_title: text(&effort[2]),
        turns: turns.iter().map(|r| (text(&r[0]), text(&r[1]))).collect(),
        tool_calls: tools
            .iter()
            .map(|r| ToolCallLine {
                tool: text(&r[0]),
                path: opt_text(&r[1]),
                detail: opt_text(&r[2]),
                ok: int(&r[3]).map(|v| v != 0),
            })
            .collect(),
        recorded: recorded.iter().map(|r| text(&r[0])).collect(),
    })
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    decisions: Vec<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::stores::AgentTurnStore as _;

    fn activity() -> EffortActivity {
        EffortActivity {
            task_title: "Add CSV export".into(),
            turns: vec![(
                "Add CSV export to reports".into(),
                "Done; used the csv crate.".into(),
            )],
            tool_calls: vec![
                ToolCallLine {
                    tool: "Edit".into(),
                    path: Some("src/report.rs".into()),
                    detail: None,
                    ok: Some(true),
                },
                ToolCallLine {
                    tool: "Bash".into(),
                    path: None,
                    detail: Some("cargo add csv".into()),
                    ok: Some(true),
                },
            ],
            recorded: vec!["Where does export live?".into()],
            ..Default::default()
        }
    }

    #[test]
    fn the_prompt_carries_the_task_turns_tools_and_recorded_decisions() {
        let p = build_prompt(&activity()).unwrap();
        for needle in [
            "Add CSV export",
            "used the csv crate",
            "Edit src/report.rs",
            "cargo add csv",
            "Where does export live?",
        ] {
            assert!(p.contains(needle), "missing {needle}: {p}");
        }
    }

    #[test]
    fn nothing_to_read_means_no_prompt() {
        assert_eq!(build_prompt(&EffortActivity::default()), None);
    }

    #[test]
    fn long_activity_is_capped() {
        let mut a = activity();
        a.turns = (0..200)
            .map(|i| (format!("ask {i} {}", "x".repeat(3000)), "y".repeat(3000)))
            .collect();
        let p = build_prompt(&a).unwrap();
        assert!(p.chars().count() <= MAX_DIGEST_CHARS + 200, "{}", p.len());
        assert!(p.contains("ask 199"), "keeps the most recent turns");
    }

    #[test]
    fn proposals_parse_skip_junk_and_are_capped() {
        let mut items: Vec<serde_json::Value> = vec![
            serde_json::json!({"question": "Which CSV library?", "choice": "csv crate", "alternatives": ["hand-rolled"], "why": "robust quoting", "confidence": "high"}),
            serde_json::json!({"question": "", "choice": "x"}),
            serde_json::json!("nonsense"),
        ];
        for i in 0..20 {
            items.push(serde_json::json!({"question": format!("q{i}"), "choice": "c"}));
        }
        let got = parse_proposals(&serde_json::json!({"decisions": items}), 3, Some(9)).unwrap();
        assert_eq!(got.len(), MAX_PROPOSALS);
        assert_eq!(got[0].question, "Which CSV library?");
        assert_eq!(got[0].alternatives, vec!["hand-rolled"]);
        assert_eq!(got[0].confidence, "high");
        assert_eq!((got[0].thread_id, got[0].task_id), (3, Some(9)));
        assert_eq!(got[1].question, "q0");
        assert_eq!(got[1].confidence, "low", "missing confidence defaults low");
        assert!(parse_proposals(&serde_json::json!([1]), 1, None).is_err());
    }

    /// A long effort keeps its newest activity, in order (tsk369).
    #[tokio::test]
    async fn gather_keeps_the_newest_tool_calls() {
        let f = crate::test_fixtures::services_with_effort().await;
        let (thread, effort) = (f.thread.value(), f.effort.value());
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 2499)
                     INSERT INTO agent_tool_call (thread_id, effort_id, tool, detail, ok, at)
                     SELECT ?1, ?2, 'Bash', 'call ' || i, 1, '2026-09-28T00:00:00Z' FROM n",
                    [thread, effort],
                )
                .map_err(|e| oxplow_domain::DomainError::Invalid(e.to_string()))?;
                Ok(())
            })
            .await
            .unwrap();
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let got = gather(&layer, effort).await.unwrap();
        let detail = |i: usize| got.tool_calls[i].detail.clone().unwrap();
        assert_eq!(detail(got.tool_calls.len() - 1), "call 2499");
        assert_eq!(detail(0), format!("call {}", 2500 - got.tool_calls.len()));
    }

    #[tokio::test]
    async fn an_effort_is_inferred_stored_and_replaced_when_main_is_assigned() {
        use crate::ai_service::{ProviderConfig, ProviderKind, RoleBinding};
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();

        // Off until a main model is assigned.
        assert_eq!(
            infer_for_effort(&f.svc, effort).await.unwrap(),
            InferOutcome::Off
        );

        let reply = serde_json::json!({"decisions": [{"question": "Which CSV library?", "choice": "csv crate", "confidence": "medium"}]});
        let (base, seen) = oxplow_ai::testing::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 5, "completion_tokens": 5}}),
        )
        .await;
        f.svc
            .ai
            .save_provider(
                ProviderConfig {
                    id: "m".into(),
                    kind: ProviderKind::OpenaiCompatible,
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        f.svc
            .ai
            .set_role(
                Role::Main,
                Some(RoleBinding {
                    provider: "m".into(),
                    model: "x".into(),
                }),
            )
            .unwrap();

        // No activity yet: nothing to ask about.
        assert_eq!(
            infer_for_effort(&f.svc, effort).await.unwrap(),
            InferOutcome::NoActivity
        );

        let turn = f
            .svc
            .agent_turn_store
            .open(&oxplow_domain::AgentTurn {
                id: oxplow_domain::AgentTurnId::placeholder(),
                thread_id: f.thread,
                prompt: "Add CSV export".into(),
                answer: None,
                session_id: None,
                started_at: oxplow_domain::Timestamp::now(),
                ended_at: None,
                start_snapshot_id: None,
                snapshot_id: None,
            })
            .await
            .unwrap();
        f.svc
            .agent_turn_store
            .close(
                &turn,
                Some("Used the csv crate.".into()),
                oxplow_domain::hook::TurnOutcome::Completed,
            )
            .await
            .unwrap();
        f.svc
            .tool_call_store
            .record(oxplow_db::NewToolCall {
                thread_id: f.thread.value(),
                effort_id: Some(effort),
                tool: "Edit".into(),
                path: Some("src/report.rs".into()),
                detail: None,
                ok: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(
            infer_for_effort(&f.svc, effort).await.unwrap(),
            InferOutcome::Stored(1)
        );
        assert_eq!(
            infer_for_effort(&f.svc, effort).await.unwrap(),
            InferOutcome::Stored(1)
        );
        let prompt = seen.lock().unwrap()[0].2["messages"].to_string();
        assert!(
            prompt.contains("Used the csv crate.") && prompt.contains("src/report.rs"),
            "{prompt}"
        );

        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql(
                "SELECT question, provenance FROM v_decision WHERE effort_id = ?1",
                vec![SqlCell::Int(effort)],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["Which CSV library?", "inferred"]]),
            "a second pass replaced the first"
        );
        // The second pass read the recorded extraction: one call.
        let calls = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql("SELECT caller, role FROM v_ai_call", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&calls.rows).unwrap(),
            serde_json::json!([[CALLER, "main"]])
        );
    }

    #[tokio::test]
    async fn closing_a_task_infers_its_efforts_decisions_in_the_background() {
        use crate::ai_service::{ProviderConfig, ProviderKind, RoleBinding};
        let f = crate::test_fixtures::services_with_effort().await;
        let reply = serde_json::json!({"decisions": [{"question": "Q", "choice": "C"}]});
        let (base, _) = oxplow_ai::testing::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        f.svc
            .ai
            .save_provider(
                ProviderConfig {
                    id: "m".into(),
                    kind: ProviderKind::OpenaiCompatible,
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        f.svc
            .ai
            .set_role(
                Role::Main,
                Some(RoleBinding {
                    provider: "m".into(),
                    model: "x".into(),
                }),
            )
            .unwrap();
        f.svc
            .tool_call_store
            .record(oxplow_db::NewToolCall {
                thread_id: f.thread.value(),
                effort_id: Some(f.effort.value()),
                tool: "Edit".into(),
                path: Some("a.rs".into()),
                detail: None,
                ok: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        crate::effort_reactors::register(&f.svc);
        f.svc
            .tasks
            .update(
                f.task,
                crate::task_service::UpdateTaskChanges {
                    status: Some(oxplow_domain::TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // The close logged `effort.finished`; the pump hands it to the
        // `effort.decisions` reactor.
        f.svc.event_pump.run_once().await.unwrap();
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        for _ in 0..100 {
            let out = layer
                .query_sql(
                    "SELECT count(*) FROM v_decision WHERE provenance = 'inferred'",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            if out.rows[0][0] == SqlCell::Int(1) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("no inferred decision appeared after the effort closed");
    }
}
