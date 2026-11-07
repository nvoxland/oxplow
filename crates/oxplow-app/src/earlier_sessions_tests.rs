//! A thread's earlier sessions — what the New Session page lists so a person
//! can bring one back (`new-session-earlier`, oxplow-foundation's page): its
//! closed sessions in the stream, and every harness's saved conversations for
//! the stream's worktree (`v_agent_conversation`), merged on (harness, native
//! id). Each row says what taking it does: reopen (a session the person closed
//! here), resume (a free conversation), show (open on this thread) or nothing
//! (open on another).

use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent_session::SessionCloseReason;
use oxplow_domain::uri::build::agent_session_uri;
use oxplow_domain::{AgentSessionId, ThreadId, Timestamp};

use crate::test_fixtures::{conversation, listing_harness as harness, Listing};
use crate::Services;

/// A session of `harness` on `thread`, on `native` when given, closed for
/// `reason` when given.
async fn session(
    svc: &Services,
    thread: ThreadId,
    harness: &str,
    native: Option<&str>,
    closed: Option<SessionCloseReason>,
) -> AgentSessionId {
    let harness = harness.to_string();
    let native = native.map(str::to_string);
    svc.db
        .transaction(move |tx| {
            use oxplow_db::agent_session_store as s;
            let now = Timestamp::now();
            let row = s::insert_tx(
                tx,
                &oxplow_domain::agent_session::NewAgentSession::terminal(thread, &harness),
                now,
            )?;
            if let Some(native) = &native {
                s::bind_tx(
                    tx,
                    row.id,
                    native,
                    oxplow_domain::agent_session::BindingOrigin::Started,
                    now,
                )?;
            }
            if let Some(reason) = closed {
                s::close_tx(tx, row.id, reason, now)?;
            }
            Ok(row.id)
        })
        .await
        .unwrap()
}

struct Fixture {
    fx: crate::test_fixtures::EffortFixture,
    other: ThreadId,
}

async fn fixture() -> Fixture {
    let fx = crate::test_fixtures::services_with_effort().await;
    // The setup's own session never ran: closed, it lists as nothing, so
    // each test's rows are its own.
    let setup = fx.session;
    fx.svc
        .db
        .transaction(move |tx| {
            oxplow_db::agent_session_store::close_tx(
                tx,
                setup,
                SessionCloseReason::Closed,
                Timestamp::now(),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Only the harnesses a test registers answer: the shipped ones run the
    // machine's own `codex` and `opencode`.
    for h in fx.svc.harnesses.all() {
        fx.svc.harnesses.unregister(h.id());
    }
    let stream = fx.svc.streams.ensure_primary().await.unwrap().id;
    let other = crate::test_fixtures::new_thread(&fx.svc, stream, "elsewhere").await;
    Fixture {
        other: other.id,
        fx,
    }
}

/// What the page says of one row.
#[derive(Debug)]
struct Row {
    label: String,
    note: String,
    harness: String,
    /// `show`, `reopen`, `resume` or `none`, from the row's link and actions.
    action: &'static str,
    session: Option<String>,
    conversation: Option<String>,
    turns: Option<i64>,
}

async fn list(f: &Fixture, harnesses: Vec<Arc<dyn AgentHarness>>) -> Vec<Row> {
    for h in harnesses {
        f.fx.svc.harnesses.register(h);
    }
    let svc = &f.fx.svc;
    let run = crate::test_fixtures::run_as_page(
        svc,
        "oxplow-foundation/new-session-earlier",
        &[
            ("stream_id".to_string(), SqlCell::Int(1)),
            ("thread_id".to_string(), SqlCell::Int(f.fx.thread.value())),
        ],
    )
    .await;
    let col = |name: &str| run.columns.iter().position(|c| c == name).unwrap();
    let text = |row: &Vec<SqlCell>, name: &str| match &row[col(name)] {
        SqlCell::Text(t) => Some(t.clone()),
        _ => None,
    };
    let flag = |row: &Vec<SqlCell>, name: &str| matches!(&row[col(name)], SqlCell::Int(1));
    run.rows
        .iter()
        .map(|row| Row {
            label: text(row, "label").unwrap_or_default(),
            note: text(row, "note").unwrap_or_default(),
            harness: text(row, "harness").unwrap_or_default(),
            action: if text(row, "session_uri").is_some() {
                "show"
            } else if flag(row, "can_reopen") {
                "reopen"
            } else if flag(row, "can_resume") {
                "resume"
            } else {
                "none"
            },
            session: text(row, "session"),
            conversation: text(row, "conversation"),
            turns: match &row[col("turns")] {
                SqlCell::Int(n) => Some(*n),
                _ => None,
            },
        })
        .collect()
}

fn row<'a>(rows: &'a [Row], native: &str) -> &'a Row {
    rows.iter()
        .find(|r| r.conversation.as_deref() == Some(native))
        .unwrap_or_else(|| panic!("no row for {native}: {rows:?}"))
}

/// What taking a row does, by how it came to be: a session the person closed
/// here and its conversation are one row that reopens it; one closed in
/// another thread, or by a thread close, or begun outside oxplow, has a free
/// conversation to resume; one open here is a tab to show; one open in
/// another thread is nothing to take.
#[tokio::test]
async fn each_row_says_what_taking_it_does() {
    let f = fixture().await;
    let closed_here = session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        Some("c1"),
        Some(SessionCloseReason::Closed),
    )
    .await;
    session(
        &f.fx.svc,
        f.other,
        "h",
        Some("c2"),
        Some(SessionCloseReason::Closed),
    )
    .await;
    // A close the thread or stream made isn't the person's to reopen.
    session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        Some("c3"),
        Some(SessionCloseReason::ThreadClosed),
    )
    .await;
    session(&f.fx.svc, f.fx.thread, "h", Some("here"), None).await;
    session(&f.fx.svc, f.other, "h", Some("there"), None).await;
    let conversations = ["c1", "c2", "c3", "x1", "here", "there"]
        .map(|id| conversation(id, 5))
        .to_vec();
    let rows = list(&f, vec![harness("h", conversations)]).await;
    assert_eq!(
        rows.len(),
        6,
        "a session and its conversation are one row: {rows:?}"
    );

    let c1 = row(&rows, "c1");
    assert_eq!(c1.action, "reopen", "{c1:?}");
    assert_eq!(
        c1.session.as_deref(),
        Some(agent_session_uri(closed_here).as_str())
    );
    assert_eq!(c1.note, "closed here");

    let c2 = row(&rows, "c2");
    assert_eq!(c2.action, "resume", "closed in another thread: {c2:?}");
    assert_eq!(c2.note, "from elsewhere");

    assert_eq!(
        row(&rows, "c3").action,
        "resume",
        "a thread close is resumed, not reopened"
    );

    let x1 = row(&rows, "x1");
    assert_eq!(x1.action, "resume", "{x1:?}");
    assert!(x1.session.is_none());
    assert_eq!(x1.note, "not started in oxplow");

    assert_eq!(row(&rows, "here").action, "show");
    let there = row(&rows, "there");
    assert_eq!(there.action, "none");
    assert!(there.note.ends_with("(elsewhere)"), "{}", there.note);
}

/// A harness that can't answer is a line; the others' rows still come.
#[tokio::test]
async fn a_failing_harness_is_unavailable_and_the_rest_still_list() {
    let f = fixture().await;
    let broken: Arc<dyn AgentHarness> = Arc::new(Listing {
        id: "down",
        answer: Err("the daemon isn't running".into()),
    });
    let rows = list(&f, vec![broken, harness("h", vec![conversation("c1", 5)])]).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    let down = rows.iter().find(|r| r.harness == "down").unwrap();
    assert_eq!(down.action, "none");
    assert!(
        down.label.contains("the daemon isn't running"),
        "{}",
        down.label
    );
    // The rows that can be taken come first.
    assert_eq!(rows[0].harness, "h");
}

/// A turn of `session` on `thread`, asked `prompt`.
async fn turn(f: &Fixture, thread: ThreadId, session: AgentSessionId, prompt: &'static str) {
    let (thread, session) = (thread.value(), session.value());
    f.fx.svc
        .db
        .transaction(move |tx| {
            tx.execute(
                "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, started_at)
                 VALUES (?1, ?2, ?3, '2026-10-10T10:00:00Z')",
                rusqlite::params![thread, session, prompt],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
}

/// A session's own row, when its harness lists no conversation for it (or
/// none that is free): a closed slot that never started anything is not an
/// earlier session, one that ran turns is (named by its last prompt); one
/// whose conversation the harness didn't list keeps its native id for a
/// resume and the person's title; a closed one no one can take (another
/// thread's, with turns but no conversation) is left out; one open here is a
/// row to show; one open elsewhere is listed only beside a conversation.
#[tokio::test]
async fn a_session_without_a_listed_conversation_is_a_row_only_when_it_can_be_taken() {
    let f = fixture().await;
    let closed = Some(SessionCloseReason::Closed);
    session(&f.fx.svc, f.fx.thread, "h", None, closed).await;
    let used = session(&f.fx.svc, f.fx.thread, "h", None, closed).await;
    turn(&f, f.fx.thread, used, "first").await;
    turn(&f, f.fx.thread, used, "second").await;

    let unlisted = session(&f.fx.svc, f.other, "h", Some("native-9"), closed).await;
    let id = unlisted.value();
    f.fx.svc
        .db
        .transaction(move |tx| {
            tx.execute(
                "UPDATE agent_session SET title = 'The parser' WHERE id = ?1",
                [id],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();

    let nothing_to_take = session(&f.fx.svc, f.other, "h", None, closed).await;
    turn(&f, f.other, nothing_to_take, "hello").await;

    let here = session(&f.fx.svc, f.fx.thread, "h", None, None).await;
    session(&f.fx.svc, f.other, "h", Some("there"), None).await;
    session(&f.fx.svc, f.other, "h", Some("busy"), None).await;

    let rows = list(&f, vec![harness("h", vec![conversation("busy", 5)])]).await;
    assert_eq!(
        rows.len(),
        4,
        "only the ones that can be taken or shown: {rows:?}"
    );
    let by_session = |s: AgentSessionId| {
        let uri = agent_session_uri(s);
        rows.iter()
            .find(|r| r.session.as_deref() == Some(uri.as_str()))
            .unwrap_or_else(|| panic!("no row for session {uri}: {rows:?}"))
    };

    let used = by_session(used);
    assert_eq!(used.turns, Some(2));
    assert_eq!(used.label, "second", "named by its last prompt");
    assert_eq!(used.action, "reopen");

    let unlisted = row(&rows, "native-9");
    assert_eq!(unlisted.action, "resume");
    assert_eq!(unlisted.label, "The parser");

    assert_eq!(row(&rows, "busy").action, "none");
    assert_eq!(by_session(here).action, "show");
}

/// Most recently active first.
#[tokio::test]
async fn rows_come_newest_first() {
    let f = fixture().await;
    let rows = list(
        &f,
        vec![harness(
            "h",
            vec![conversation("old", 1), conversation("new", 9)],
        )],
    )
    .await;
    let ids: Vec<_> = rows
        .iter()
        .map(|r| r.conversation.clone().unwrap())
        .collect();
    assert_eq!(ids, ["new", "old"]);
}

/// The listing asks the harnesses a test registers and no others: the
/// shipped ones would run the machine's `codex` and `opencode`, whose answer
/// (or lateness under load) would be rows the test didn't ask for.
#[tokio::test]
async fn only_the_registered_test_harnesses_are_asked() {
    let f = fixture().await;
    list(&f, vec![harness("h", vec![])]).await;
    let ids: Vec<_> =
        f.fx.svc
            .harnesses
            .all()
            .iter()
            .map(|h| h.id().to_string())
            .collect();
    assert_eq!(ids, ["h"]);
}
