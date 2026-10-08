//! Forgetting an agent session's stale resume pointer, when its harness's
//! launch found the harness session gone (`Launch::resume_dropped`).
//!
//! `agent_session.resume_session_id` drives a harness's resume. A harness
//! that can tell its session is gone (Claude, by its transcript file) says
//! so from `launch`; without forgetting it here the stale id would linger
//! until the next prompt self-heals it.

/// Forget agent session `session`'s resume pointer when it is still
/// `resume` — the launch found that harness session's transcript gone.
/// Only that column, and only if nothing replaced it meanwhile:
/// agent-session state, written the way hook ingest writes it (off the
/// bus, `ipc-and-stores.md`).
pub async fn forget_missing(
    db: &oxplow_db::Database,
    session: oxplow_domain::AgentSessionId,
    resume: &str,
) -> Result<(), oxplow_domain::DomainError> {
    let resume = resume.to_string();
    db.transaction(move |tx| {
        oxplow_db::agent_session_store::forget_resume_tx(
            tx,
            session,
            &resume,
            oxplow_domain::Timestamp::now(),
        )
        .map(|_| ())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stale pointer is forgotten only while it is still the one the
    /// launch found gone — a harness session that replaced it stays.
    #[tokio::test]
    async fn forgetting_a_missing_session_leaves_a_newer_one() {
        use oxplow_db::agent_session_store::{get_tx, newest_for_thread_tx, set_resume_tx};
        let fx = crate::test_fixtures::services_with_effort().await;
        let (db, thread) = (fx.svc.db.clone(), fx.thread);
        let session = db
            .read(move |c| newest_for_thread_tx(c, thread))
            .await
            .unwrap()
            .expect("the seeded thread has a session")
            .id;
        let set = |resume: &'static str| {
            let db = db.clone();
            async move {
                db.transaction(move |tx| {
                    set_resume_tx(tx, session, resume, oxplow_domain::Timestamp::now())
                })
                .await
                .unwrap()
            }
        };
        let now = || {
            let db = db.clone();
            async move {
                db.read(move |c| get_tx(c, session))
                    .await
                    .unwrap()
                    .unwrap()
                    .resume_session_id
            }
        };
        set("newer").await;
        forget_missing(&db, session, "gone").await.unwrap();
        assert_eq!(now().await, "newer");
        set("gone").await;
        forget_missing(&db, session, "gone").await.unwrap();
        assert_eq!(now().await, "");
    }
}
