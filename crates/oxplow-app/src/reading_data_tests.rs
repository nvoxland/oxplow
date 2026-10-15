//! The example queries in the `oxplow-extension` skill's `reading-data`
//! reference, run as written over a seeded project. The SQL is read out of
//! the reference file, so the text and these checks cannot drift: change a
//! query there and the aggregate here must still hold.

use oxplow_db::SqlCell;
use serde_json::json;

use crate::test_fixtures::{services_with_effort, EffortFixture};

const REFERENCE: &str = include_str!(
    "../../oxplow-agent-text/assets/skills/oxplow-extension/references/reading-data.md"
);

/// The fenced SQL under the reference's `### <title>` heading.
fn example(title: &str) -> String {
    let (_, rest) = REFERENCE
        .split_once(&format!("### {title}\n"))
        .unwrap_or_else(|| panic!("the reference has an example `{title}`"));
    let (_, rest) = rest.split_once("```sql\n").expect("it shows a sql block");
    let (sql, _) = rest.split_once("```").expect("the block closes");
    sql.trim().to_string()
}

async fn rows(fx: &EffortFixture, title: &str) -> Vec<Vec<SqlCell>> {
    let out = fx
        .svc
        .sql
        .query_sql(&example(title), vec![], None)
        .await
        .unwrap_or_else(|e| panic!("`{title}` runs: {e}"));
    assert!(!out.truncated, "{title}");
    out.rows
}

fn int(c: &SqlCell) -> i64 {
    match c {
        SqlCell::Int(i) => *i,
        SqlCell::Real(r) => *r as i64,
        other => panic!("not a number: {other:?}"),
    }
}

fn text(c: &SqlCell) -> String {
    match c {
        SqlCell::Text(t) => t.clone(),
        other => panic!("not text: {other:?}"),
    }
}

/// Three coverage reports in three different weeks read back as three
/// weekly points, in order, each the value reported that week.
#[tokio::test]
async fn coverage_trend_by_week() {
    let fx = services_with_effort().await;
    for value in [70.0, 80.0, 90.0] {
        fx.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::metric::RECORD,
                json!({ "key": "oxplow.coverage.abs_pct", "value": value }),
                false,
            )
            .await
            .unwrap();
    }
    // Weeks starting Monday 2026-09-07, 09-14 and 09-21 (UTC).
    for (i, day) in ["2026-09-08", "2026-09-15", "2026-09-22"]
        .iter()
        .enumerate()
    {
        let at = format!("{day}T10:00:00.000000Z");
        let nth = i as i64;
        fx.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE metric_capture SET captured_at = ?1
                     WHERE id = (SELECT id FROM metric_capture ORDER BY id LIMIT 1 OFFSET ?2)",
                    rusqlite::params![at, nth],
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
    }
    let got = rows(&fx, "Coverage trend by week").await;
    let values: Vec<i64> = got.iter().map(|r| int(&r[1])).collect();
    assert_eq!(values, [70, 80, 90], "{got:?}");
    let weeks: Vec<String> = got.iter().map(|r| text(&r[0])).collect();
    assert!(weeks.windows(2).all(|w| w[0] < w[1]), "{weeks:?}");
}

/// A test that is red now is failing (most flips first among the failing),
/// one that went red and green again is flaky, a steady green is neither.
#[tokio::test]
async fn failing_or_flaky_tests() {
    let fx = services_with_effort().await;
    fx.svc
        .db
        .transaction(|tx| {
            for (subject, status, runs, failures, flips) in [
                ("test:always_red", "failed", 5, 5, 0),
                ("test:just_broke", "failed", 4, 1, 1),
                ("test:flaky", "passed", 9, 4, 5),
                ("test:steady", "passed", 9, 0, 0),
                ("test:fixed_once", "passed", 3, 1, 1),
            ] {
                tx.execute(
                    "INSERT INTO test_case_stat
                       (stream_id, branch, producer, subject, last_status, runs, failures, flips,
                        first_seen_at, last_seen_at)
                     VALUES (1, 'main', 'tests', ?1, ?2, ?3, ?4, ?5,
                             '2026-10-01T00:00:00.000000Z', '2026-10-02T00:00:00.000000Z')",
                    rusqlite::params![subject, status, runs, failures, flips],
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let got = rows(&fx, "Failing or flaky tests").await;
    let subjects: Vec<String> = got.iter().map(|r| text(&r[0])).collect();
    assert_eq!(
        subjects,
        ["test:just_broke", "test:always_red", "test:flaky"],
        "{got:?}"
    );
}

/// Efforts count per thread, an open one counted apart, and a thread with
/// none stays as a row of zeros.
#[tokio::test]
async fn efforts_per_thread() {
    let fx = services_with_effort().await;
    let stream = oxplow_domain::StreamId::new(1);
    let second = crate::test_fixtures::new_thread(&fx.svc, stream, "Second").await;
    let third = crate::test_fixtures::new_thread(&fx.svc, stream, "Third").await;
    let run = |name: &'static str, input: serde_json::Value| {
        let svc = fx.svc.clone();
        async move {
            svc.commands
                .run(&oxplow_domain::Actor::Human, name, input, false)
                .await
                .unwrap()
        }
    };
    let thread_ref = oxplow_domain::refs::build::thread_ref;
    // thr1: its fixture effort is closed and a second one left open.
    run(
        crate::commands::effort::CLOSE,
        json!({ "effort": oxplow_domain::refs::build::effort_ref(fx.effort) }),
    )
    .await;
    run(
        crate::commands::effort::OPEN,
        json!({ "thread": thread_ref(fx.thread) }),
    )
    .await;
    // thr2: one effort, closed.
    let opened = run(
        crate::commands::effort::OPEN,
        json!({ "thread": thread_ref(second.id) }),
    )
    .await;
    run(
        crate::commands::effort::CLOSE,
        json!({ "effort": opened.result["effort"] }),
    )
    .await;
    let got = rows(&fx, "Efforts per thread").await;
    let by_thread: Vec<(i64, i64, i64)> = got
        .iter()
        .map(|r| (int(&r[0]), int(&r[2]), int(&r[3])))
        .collect();
    assert_eq!(
        by_thread,
        [
            (fx.thread.value(), 2, 1),
            (second.id.value(), 1, 0),
            (third.id.value(), 0, 0)
        ],
        "{got:?}"
    );
}

/// Usage reports sum per thread and agent, cache reads apart.
#[tokio::test]
async fn token_usage_per_thread_and_agent() {
    let fx = services_with_effort().await;
    let stream = oxplow_domain::StreamId::new(1);
    let second = crate::test_fixtures::new_thread(&fx.svc, stream, "Second").await;
    let usage = |thread: oxplow_domain::ThreadId, kind: &str, input, output, cache_read| {
        oxplow_db::NewAgentTokenUsage {
            stream_id: stream.to_string(),
            thread_id: thread.to_string(),
            effort_id: None,
            session_id: "s".into(),
            agent_kind: kind.into(),
            model: None,
            prompt: None,
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: cache_read,
            message_count: 1,
            turn_id: None,
            cause: None,
        }
    };
    for u in [
        usage(fx.thread, "claude", 100, 50, 1000),
        usage(fx.thread, "claude", 10, 5, 0),
        usage(fx.thread, "codex", 20, 20, 0),
        usage(second.id, "claude", 400, 100, 7),
    ] {
        fx.svc.token_usage_store.record(u).await.unwrap();
    }
    let got = rows(&fx, "Token usage per thread and agent").await;
    let shaped: Vec<(i64, String, i64, i64, i64)> = got
        .iter()
        .map(|r| (int(&r[0]), text(&r[1]), int(&r[2]), int(&r[3]), int(&r[4])))
        .collect();
    assert_eq!(
        shaped,
        [
            (second.id.value(), "claude".into(), 500, 7, 1),
            (fx.thread.value(), "claude".into(), 165, 1000, 2),
            (fx.thread.value(), "codex".into(), 40, 0, 1),
        ],
        "{got:?}"
    );
}
