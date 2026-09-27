//! Feeding recorded decisions back to the agent: a context block with the
//! open effort's decisions (so they survive compaction / resume), and a
//! nudge when a big effort closes with none recorded. See
//! `.context/semantic-layer.md` (`v_decision`).

use oxplow_db::{SemanticLayer, SqlCell};

/// Decisions shown in the context block (most recent last).
pub const MAX_DECISIONS_IN_CONTEXT: usize = 15;
/// An effort touching at least this many files with no decisions gets a nudge.
pub const NUDGE_FILE_THRESHOLD: i64 = 8;

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

/// The `complete_task` nudge for an effort that touched `files` files and
/// recorded `decisions` decisions; `None` when it isn't warranted.
pub fn format_missing_decisions_hint(files: i64, decisions: i64) -> Option<String> {
    (files >= NUDGE_FILE_THRESHOLD && decisions == 0).then(|| {
        format!(
            "This effort touched {files} files but recorded no decisions. If you resolved any \
             forks without asking (where something lives, which approach, what you left out), \
             call record_decision for each now: the reviewer checks those first."
        )
    })
}

/// `effort_id`'s decisions as a context block (see
/// [`format_decisions_block`]).
pub async fn effort_decisions_block(layer: &SemanticLayer, effort_id: i64) -> Option<String> {
    let out = layer
        .query_sql(
            "SELECT question, choice, alternatives, why FROM v_decision
             WHERE effort_id = ?1 ORDER BY id DESC LIMIT ?2",
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

/// The `complete_task` nudge for `effort_id` (see
/// [`format_missing_decisions_hint`]).
pub async fn missing_decisions_hint(layer: &SemanticLayer, effort_id: i64) -> Option<String> {
    let out = layer
        .query_sql(
            "SELECT (SELECT count(*) FROM v_effort_file WHERE effort_id = ?1),
                    (SELECT count(*) FROM v_decision WHERE effort_id = ?1)",
            vec![SqlCell::Int(effort_id)],
            None,
        )
        .await
        .ok()?;
    match out.rows.first().map(|r| (&r[0], &r[1])) {
        Some((SqlCell::Int(files), SqlCell::Int(decisions))) => {
            format_missing_decisions_hint(*files, *decisions)
        }
        _ => None,
    }
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

    #[test]
    fn nudges_only_big_efforts_without_decisions() {
        assert_eq!(format_missing_decisions_hint(3, 0), None);
        assert_eq!(format_missing_decisions_hint(8, 1), None);
        let hint = format_missing_decisions_hint(8, 0).unwrap();
        assert!(
            hint.contains("8 files") && hint.contains("record_decision"),
            "{hint}"
        );
    }
}
