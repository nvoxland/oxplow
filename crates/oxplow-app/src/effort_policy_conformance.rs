//! The effort-policy conformance suite (`.context/work-tracking.md`): what
//! every effort policy must do, checked through what a person and an agent
//! see — the `work_item.*` commands in, `v_effort` and the event log out.
//! It runs in-tree against the built-in (`oxplow:commit-or-switch`) and
//! against a provider process through the conformance kit
//! (`oxplow extension test`), the same way: core never special-cases its
//! own implementation.
//!
//! The checks are rule 1's floor, which any policy keeps: an agent
//! starting an item on its thread opens an effort there linked to the
//! item, and finishing it closes that effort — each by the policy's own
//! effect actor, never by a person's hand.

use std::time::Duration;

use oxplow_db::SqlCell;
use oxplow_domain::refs::build::{effort_ref, thread_ref};
use oxplow_domain::{Actor, EffortId, StreamId, ThreadId};
use serde_json::json;

pub use crate::work_items_conformance::{Finding, SuiteRun};

/// Run every check against `policy`, which the caller made the project's
/// active effort policy, filing its items on the active work list.
pub async fn suite(svc: &crate::Services, policy: &str) -> SuiteRun {
    let mut findings = Vec::new();
    if let Err(message) = run(svc, policy, &mut findings).await {
        findings.push(Finding {
            check: "setup",
            message,
        });
    }
    SuiteRun {
        findings,
        left: Vec::new(),
    }
}

async fn run(
    svc: &crate::Services,
    policy: &str,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    let config = crate::config_service::read_config(&svc.config);
    let active = svc
        .capabilities
        .active(&config, crate::effort_policy::CAPABILITY);
    if active != policy {
        return Err(format!(
            "the active effort policy is `{active}`, not `{policy}`"
        ));
    }
    svc.streams
        .ensure_primary()
        .await
        .map_err(|e| e.to_string())?;
    let (thread, stream) = first_thread(svc).await?;
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(thread),
        stream_id: Some(stream),
    };
    let actor = format!("effect:{}:{policy}", crate::effort_policy::CAPABILITY);
    // A clean thread: what the policy opens is all that's open.
    for effort in open_efforts(svc, thread, None).await? {
        svc.commands
            .run(
                &Actor::Human,
                crate::commands::effort::CLOSE,
                json!({ "effort": effort_ref(effort) }),
                false,
            )
            .await
            .map_err(|e| format!("closing effort {effort}: {e}"))?;
    }
    settle(svc).await;
    let item = svc
        .commands
        .run(
            &Actor::Human,
            crate::commands::work_item::CREATE,
            json!({ "title": "conformance: an item to start", "thread": thread.to_string() }),
            false,
        )
        .await
        .map_err(|e| format!("filing an item: {e}"))?
        .result["ref"]
        .as_str()
        .ok_or("the work list filed no item")?
        .to_string();

    let before = head(svc).await?;
    transition(svc, &agent, &item, "in_progress").await?;
    let open = open_efforts(svc, thread, Some(&item)).await?;
    if open.len() != 1 {
        findings.push(Finding {
            check: "a_started_item_opens_a_linked_effort",
            message: format!(
                "an agent starting {item} on {} left {} open efforts linked to it, not one",
                thread_ref(thread),
                open.len()
            ),
        });
    }
    check_sources(svc, "effort.opened", before, &actor, findings).await?;

    let before = head(svc).await?;
    transition(svc, &agent, &item, "done").await?;
    let still = open_efforts(svc, thread, Some(&item)).await?;
    if !still.is_empty() {
        findings.push(Finding {
            check: "a_finished_item_closes_its_effort",
            message: format!("{item} is done and its effort is still open"),
        });
    }
    check_sources(svc, "effort.closed", before, &actor, findings).await?;
    Ok(())
}

/// Every `event_type` logged since `seq` was the policy's doing, and there
/// was at least one.
async fn check_sources(
    svc: &crate::Services,
    event_type: &str,
    since: i64,
    actor: &str,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    let sources: Vec<String> = rows(
        svc,
        "SELECT source FROM v_event WHERE type = ?1 AND seq > ?2 ORDER BY seq",
        vec![SqlCell::Text(event_type.into()), SqlCell::Int(since)],
    )
    .await?
    .into_iter()
    .filter_map(|r| text(&r[0]))
    .collect();
    if sources.is_empty() || sources.iter().any(|s| s != actor) {
        findings.push(Finding {
            check: "the_policy_runs_as_its_own_effect",
            message: format!("`{event_type}` was logged by {sources:?}, not only by `{actor}`"),
        });
    }
    Ok(())
}

async fn transition(
    svc: &crate::Services,
    actor: &Actor,
    item: &str,
    to: &str,
) -> Result<(), String> {
    svc.commands
        .run(
            actor,
            crate::commands::work_item::NAME,
            json!({ "ref": item, "to": to }),
            false,
        )
        .await
        .map_err(|e| format!("moving {item} to {to}: {e}"))?;
    settle(svc).await;
    Ok(())
}

async fn settle(svc: &crate::Services) {
    svc.event_pump
        .settle(&[crate::effort_policy::NAME], Duration::from_secs(10))
        .await;
}

async fn first_thread(svc: &crate::Services) -> Result<(ThreadId, StreamId), String> {
    let row = rows(
        svc,
        "SELECT id, stream_id FROM v_thread ORDER BY id LIMIT 1",
        Vec::new(),
    )
    .await?
    .into_iter()
    .next()
    .ok_or("the project has no thread")?;
    match (&row[0], &row[1]) {
        (SqlCell::Int(t), SqlCell::Int(s)) => Ok((ThreadId::new(*t), StreamId::new(*s))),
        other => Err(format!("a thread row reads {other:?}")),
    }
}

/// `thread`'s open efforts, those linked to `item` when it's given.
async fn open_efforts(
    svc: &crate::Services,
    thread: ThreadId,
    item: Option<&str>,
) -> Result<Vec<EffortId>, String> {
    let (sql, params) = match item {
        Some(item) => (
            "SELECT id FROM v_effort WHERE thread_id = ?1 AND ended_at IS NULL AND work_item = ?2",
            vec![SqlCell::Int(thread.value()), SqlCell::Text(item.into())],
        ),
        None => (
            "SELECT id FROM v_effort WHERE thread_id = ?1 AND ended_at IS NULL",
            vec![SqlCell::Int(thread.value())],
        ),
    };
    Ok(rows(svc, sql, params)
        .await?
        .into_iter()
        .filter_map(|r| match r[0] {
            SqlCell::Int(id) => Some(EffortId::new(id)),
            _ => None,
        })
        .collect())
}

async fn head(svc: &crate::Services) -> Result<i64, String> {
    let rows = rows(svc, "SELECT coalesce(max(seq), 0) FROM v_event", Vec::new()).await?;
    match rows.first().map(|r| &r[0]) {
        Some(SqlCell::Int(seq)) => Ok(*seq),
        other => Err(format!("the log's head reads {other:?}")),
    }
}

async fn rows(
    svc: &crate::Services,
    sql: &str,
    params: Vec<SqlCell>,
) -> Result<Vec<Vec<SqlCell>>, String> {
    svc.sql
        .query_sql(sql, params, None)
        .await
        .map(|r| r.rows)
        .map_err(|e| e.to_string())
}

fn text(cell: &SqlCell) -> Option<String> {
    match cell {
        SqlCell::Text(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;

    /// The built-in passes the suite every provider must: core calls its
    /// own policy the way it calls any.
    #[tokio::test]
    async fn the_built_in_policy_passes_the_suite() {
        let fx = services_with_effort().await;
        let config = crate::config_service::read_config(&fx.svc.config);
        let active = fx
            .svc
            .capabilities
            .active(&config, crate::effort_policy::CAPABILITY);
        let run = suite(&fx.svc, &active).await;
        assert_eq!(run.findings, Vec::new());
    }

    /// With none active, nothing opens: the suite finds it.
    #[tokio::test]
    async fn a_policy_that_does_nothing_fails_the_suite() {
        let fx = services_with_effort().await;
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(crate::effort_policy::CAPABILITY.into(), "none".into());
        let run = suite(&fx.svc, "none").await;
        let checks: Vec<_> = run.findings.iter().map(|f| f.check).collect();
        assert!(
            checks.contains(&"a_started_item_opens_a_linked_effort"),
            "{:?}",
            run.findings
        );
    }
}
