//! A thread's earlier sessions — what the New Session page lists so a person
//! can bring one back (`new-session-earlier`, oxplow-foundation's lens): its
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

/// What the lens says of one row.
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

/// A session this thread's person closed and its conversation are one row,
/// and taking it reopens the session.
#[tokio::test]
async fn a_session_closed_here_and_its_conversation_are_one_reopen_row() {
    let f = fixture().await;
    let s = session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        Some("c1"),
        Some(SessionCloseReason::Closed),
    )
    .await;
    let rows = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].action, "reopen");
    assert_eq!(
        rows[0].session.as_deref(),
        Some(agent_session_uri(s).as_str())
    );
    assert_eq!(rows[0].note, "closed here");
    assert_eq!(rows[0].conversation.as_deref(), Some("c1"));
}

/// One closed in another thread has its conversation free: resume it here,
/// the row naming the thread it came from.
#[tokio::test]
async fn a_session_closed_in_another_thread_resumes_here() {
    let f = fixture().await;
    session(
        &f.fx.svc,
        f.other,
        "h",
        Some("c1"),
        Some(SessionCloseReason::Closed),
    )
    .await;
    let rows = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
    let r = row(&rows, "c1");
    assert_eq!(r.action, "resume");
    assert_eq!(r.note, "from elsewhere");
}

/// A close the thread or stream made isn't the person's to reopen: the
/// conversation resumes in a new session instead.
#[tokio::test]
async fn a_session_a_thread_close_took_resumes_instead_of_reopening() {
    let f = fixture().await;
    session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        Some("c1"),
        Some(SessionCloseReason::ThreadClosed),
    )
    .await;
    let rows = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
    assert_eq!(row(&rows, "c1").action, "resume");
}

/// A conversation begun outside oxplow has no session: resume it.
#[tokio::test]
async fn an_outside_conversation_resumes() {
    let f = fixture().await;
    let rows = list(&f, vec![harness("h", vec![conversation("x1", 5)])]).await;
    let r = row(&rows, "x1");
    assert_eq!(r.action, "resume");
    assert!(r.session.is_none());
    assert_eq!(r.note, "not started in oxplow");
}

/// A conversation open in this thread's session is a tab to show; one open
/// in another thread's is nothing to take.
#[tokio::test]
async fn a_conversation_that_is_open_is_switched_to_or_left() {
    let f = fixture().await;
    session(&f.fx.svc, f.fx.thread, "h", Some("here"), None).await;
    session(&f.fx.svc, f.other, "h", Some("there"), None).await;
    let rows = list(
        &f,
        vec![harness(
            "h",
            vec![conversation("here", 5), conversation("there", 6)],
        )],
    )
    .await;
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

/// A closed slot that never started anything is not an earlier session;
/// one that ran turns is, whatever its harness lists.
#[tokio::test]
async fn a_never_started_slot_is_left_out_and_a_used_one_counts_its_turns() {
    let f = fixture().await;
    session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        None,
        Some(SessionCloseReason::Closed),
    )
    .await;
    let used = session(
        &f.fx.svc,
        f.fx.thread,
        "h",
        None,
        Some(SessionCloseReason::Closed),
    )
    .await;
    let id = used.value();
    let thread = f.fx.thread.value();
    f.fx.svc
        .db
        .transaction(move |tx| {
            for prompt in ["first", "second"] {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, started_at)
                     VALUES (?1, ?2, ?3, '2026-10-10T10:00:00Z')",
                    rusqlite::params![thread, id, prompt],
                )
                .map_err(oxplow_db::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let rows = list(&f, vec![harness("h", vec![])]).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].session.as_deref(),
        Some(agent_session_uri(used).as_str())
    );
    assert_eq!(rows[0].turns, Some(2));
    assert_eq!(rows[0].label, "second", "named by its last prompt");
    assert_eq!(rows[0].action, "reopen");
}

/// A closed session whose conversation its harness didn't list (it is
/// beyond the limit, or gone) still carries its native id for a resume, and
/// the person's title, if any, names it.
#[tokio::test]
async fn an_unlisted_conversation_keeps_its_native_id_and_the_sessions_name() {
    let f = fixture().await;
    let other = session(
        &f.fx.svc,
        f.other,
        "h",
        Some("native-9"),
        Some(SessionCloseReason::Closed),
    )
    .await;
    let id = other.value();
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
    let rows = list(&f, vec![harness("h", vec![])]).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].action, "resume");
    assert_eq!(rows[0].conversation.as_deref(), Some("native-9"));
    assert_eq!(rows[0].label, "The parser");
}

/// A closed session no one can take — another thread's, with turns but no
/// conversation to resume — is left out; only a conversation open in another
/// thread's session is listed with nothing to take.
#[tokio::test]
async fn a_closed_session_with_nothing_to_take_is_left_out() {
    let f = fixture().await;
    let s = session(
        &f.fx.svc,
        f.other,
        "h",
        None,
        Some(SessionCloseReason::Closed),
    )
    .await;
    let id = s.value();
    let thread = f.other.value();
    f.fx.svc
        .db
        .transaction(move |tx| {
            tx.execute(
                "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, started_at)
                 VALUES (?1, ?2, 'hello', '2026-10-10T10:00:00Z')",
                rusqlite::params![thread, id],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
    session(&f.fx.svc, f.other, "h", Some("busy"), None).await;
    let rows = list(&f, vec![harness("h", vec![conversation("busy", 5)])]).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].action, "none");
}

/// A session open on this thread is a row to show even when its harness
/// lists no conversation for it; one open elsewhere is listed only beside a
/// conversation.
#[tokio::test]
async fn a_session_open_here_is_listed_without_a_conversation() {
    let f = fixture().await;
    let here = session(&f.fx.svc, f.fx.thread, "h", None, None).await;
    session(&f.fx.svc, f.other, "h", Some("there"), None).await;
    let rows = list(&f, vec![harness("h", vec![])]).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].action, "show");
    assert_eq!(
        rows[0].session.as_deref(),
        Some(agent_session_uri(here).as_str())
    );
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
