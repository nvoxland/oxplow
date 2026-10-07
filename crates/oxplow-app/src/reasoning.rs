//! Feeding recorded decisions back to the agent: a context block with the
//! open effort's decisions (so they survive compaction / resume). See
//! `.context/semantic-layer.md` (`v_decision`).

use oxplow_db::SqlCell;

/// Decisions shown in the context block (most recent last).
pub const MAX_DECISIONS_IN_CONTEXT: usize = 15;

/// One decision as shown back to the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionLine {
    pub question: String,
    pub choice: String,
    pub alternatives: Vec<String>,
    pub why: String,
}

/// Markdown block for the given decisions; `None` when empty.
pub fn format_decisions_block(decisions: &[DecisionLine]) -> Option<String> {
    if decisions.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Decisions you've recorded for this effort\n\n\
         Keep to these unless the user changes them (record a new decision if you do).\n",
    );
    for d in decisions {
        out.push_str(&format!("\n- {} → {}", d.question, d.choice));
        if !d.alternatives.is_empty() {
            out.push_str(&format!(" (over: {})", d.alternatives.join("; ")));
        }
        if !d.why.trim().is_empty() {
            out.push_str(&format!(". {}", d.why.trim()));
        }
    }
    Some(out)
}

/// `effort_id`'s decisions as a context block (see
/// [`format_decisions_block`]).
pub async fn effort_decisions_block(
    layer: &crate::sql_gateway::SqlGateway,
    effort_id: i64,
) -> Option<String> {
    let out = layer
        .query_sql(
            // Only what the agent recorded: inferred decisions are
            // oxplow's guesses, not the agent's own commitments.
            "SELECT question, choice, alternatives, why FROM v_decision
             WHERE effort_id = ?1 AND provenance = 'recorded' ORDER BY id DESC LIMIT ?2",
            vec![
                SqlCell::Int(effort_id),
                SqlCell::Int(MAX_DECISIONS_IN_CONTEXT as i64),
            ],
            None,
        )
        .await
        .ok()?;
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => t.clone(),
        _ => String::new(),
    };
    let mut lines: Vec<DecisionLine> = out
        .rows
        .iter()
        .map(|r| DecisionLine {
            question: text(&r[0]),
            choice: text(&r[1]),
            alternatives: serde_json::from_str(&text(&r[2])).unwrap_or_default(),
            why: text(&r[3]),
        })
        .collect();
    lines.reverse();
    format_decisions_block(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_lists_decisions() {
        assert_eq!(format_decisions_block(&[]), None);
        let block = format_decisions_block(&[DecisionLine {
            question: "Where?".into(),
            choice: "A".into(),
            alternatives: vec!["B".into(), "C".into()],
            why: "because".into(),
        }])
        .unwrap();
        assert!(block.contains("Decisions you've recorded"), "{block}");
        assert!(
            block.contains("- Where? → A (over: B; C). because"),
            "{block}"
        );
        let bare = format_decisions_block(&[DecisionLine {
            question: "Q".into(),
            choice: "X".into(),
            alternatives: vec![],
            why: String::new(),
        }])
        .unwrap();
        assert!(
            bare.contains("- Q → X\n") || bare.ends_with("- Q → X"),
            "{bare}"
        );
    }

    #[tokio::test]
    async fn inferred_decisions_are_never_fed_back_to_the_agent() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let effort = f.effort.value();
        let d = |q: &str| oxplow_db::NewDecision {
            thread_id: f.thread.value(),
            work_item: Some(oxplow_domain::refs::build::work_item_ref(f.task)),
            effort_id: Some(effort),
            question: q.into(),
            choice: "c".into(),
            alternatives: vec![],
            confidence: "medium".into(),
            why: String::new(),
        };
        f.svc
            .reasoning_store
            .replace_inferred(effort, vec![d("guessed")])
            .await
            .unwrap();
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        assert_eq!(effort_decisions_block(&layer, effort).await, None);
        f.svc
            .db
            .transaction({
                let recorded = d("recorded");
                move |tx| oxplow_db::record_decision_tx(tx, &recorded)
            })
            .await
            .unwrap();
        let block = effort_decisions_block(&layer, effort).await.unwrap();
        assert!(
            block.contains("recorded") && !block.contains("guessed"),
            "{block}"
        );
    }
}
