//! A thread's earlier sessions: what the New Session page lists so a person
//! can bring one back (`.context/terminal.md` "Choosing another
//! conversation", `.context/pages-and-tabs.md`). oxplow's own closed
//! sessions in the stream, and every harness's saved conversations for the
//! stream's worktree (started in oxplow or outside it), merged on
//! (harness, native id). Each row says what taking it does — decided here,
//! so the page never re-derives the rules. Read-only: taking a row is
//! `oxplow.agent_session.reopen` / `.open { resume }`.

use std::collections::HashMap;
use std::sync::Arc;

use oxplow_domain::agent::harness::{
    AgentHarness, ConversationQuery, HarnessError, NativeConversation,
};
use oxplow_domain::agent_session::{AgentSession, SessionCloseReason};
use oxplow_domain::refs::build::{agent_session_ref, thread_ref};
use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
use oxplow_domain::{DomainError, Stream, ThreadId, Timestamp};
use serde::Serialize;

use crate::Services;

/// The most conversations listed per harness.
const LIMIT: usize = 50;

/// What taking a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EarlierAction {
    /// This thread's session a person closed: `oxplow.agent_session.reopen`.
    Reopen,
    /// A free conversation: a new session on it here
    /// (`oxplow.agent_session.open { resume }`).
    Resume,
    /// Open in a session on this thread: show its tab.
    Switch,
    /// Open in another thread's session: nothing to take.
    None,
}

/// An oxplow session a row is about.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct PastSession {
    /// `agent_session:ses3`.
    pub session: String,
    pub display_name: String,
    /// `thread:thr1` and its title.
    pub thread: String,
    pub thread_title: String,
    /// It is on this thread.
    pub here: bool,
    /// Still open (a row noting where a conversation is open).
    pub open: bool,
    pub closed_at: Option<Timestamp>,
    pub turns: u32,
    pub last_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct EarlierSession {
    pub harness: String,
    pub session: Option<PastSession>,
    pub conversation: Option<NativeConversation>,
    pub action: EarlierAction,
}

/// A harness that couldn't list its conversations now.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct HarnessUnavailable {
    pub harness: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct EarlierSessions {
    /// Most recently active first.
    pub rows: Vec<EarlierSession>,
    pub unavailable: Vec<HarnessUnavailable>,
}

/// The saved conversations of `harness` begun in `stream`'s worktree.
pub async fn harness_conversations(
    harness: &dyn AgentHarness,
    stream: &Stream,
) -> Result<Vec<NativeConversation>, HarnessError> {
    let query = ConversationQuery {
        cwd: std::path::PathBuf::from(&stream.worktree_path),
        // Its saved conversations are on the machine it runs on.
        home: (stream.host == oxplow_domain::HostId::LOCAL)
            .then(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
            .flatten(),
        search_path: crate::agent_path::search_path(),
        limit: LIMIT,
    };
    harness.conversations(&query).await
}

/// `thread`'s earlier sessions: see the module docs.
pub async fn earlier_sessions(
    svc: &Services,
    thread: ThreadId,
) -> Result<EarlierSessions, DomainError> {
    earlier_sessions_of(svc, thread, svc.harnesses.all()).await
}

/// [`earlier_sessions`] over `harnesses`.
pub async fn earlier_sessions_of(
    svc: &Services,
    thread: ThreadId,
    harnesses: Vec<Arc<dyn AgentHarness>>,
) -> Result<EarlierSessions, DomainError> {
    let this = svc
        .thread_store
        .get(&thread)
        .await?
        .ok_or(DomainError::NotFound)?;
    let stream = svc
        .stream_store
        .get(&this.stream_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    let titles: HashMap<ThreadId, String> = svc
        .thread_store
        .list_for_stream(&stream.id)
        .await?
        .into_iter()
        .map(|t| (t.id, t.title))
        .collect();

    let stream_id = stream.id;
    let (sessions, turns) = svc
        .db
        .read(move |c| {
            let sessions = oxplow_db::agent_session_store::list_for_stream_tx(c, stream_id)?;
            let ids: Vec<_> = sessions.iter().map(|s| s.id).collect();
            let turns = oxplow_db::agent_session_store::turn_summaries_tx(c, &ids)?;
            Ok((sessions, turns))
        })
        .await?;

    // Every harness answers at once; a failure is a line, not a failed list.
    let answers = futures::future::join_all(harnesses.iter().map(|h| {
        let stream = &stream;
        async move {
            (
                h.id().to_string(),
                harness_conversations(h.as_ref(), stream).await,
            )
        }
    }))
    .await;
    let mut unavailable = Vec::new();
    let mut conversations: Vec<(String, NativeConversation)> = Vec::new();
    for (harness, answer) in answers {
        match answer {
            Ok(found) => conversations.extend(found.into_iter().map(|c| (harness.clone(), c))),
            // It keeps no list: nothing to say.
            Err(HarnessError::Unsupported { .. }) => {}
            Err(other) => unavailable.push(HarnessUnavailable {
                harness,
                reason: other.to_string(),
            }),
        }
    }

    let past = |s: &AgentSession| PastSession {
        session: agent_session_ref(s.id),
        display_name: if !s.title.is_empty() {
            s.title.clone()
        } else if !s.native_title.is_empty() {
            s.native_title.clone()
        } else {
            format!("{} {}", s.harness, agent_session_ref(s.id))
        },
        thread: thread_ref(s.thread_id),
        thread_title: titles.get(&s.thread_id).cloned().unwrap_or_default(),
        here: s.thread_id == thread,
        open: s.is_open(),
        closed_at: s.closed_at,
        turns: turns.get(&s.id).map_or(0, |t| t.0 as u32),
        last_prompt: turns.get(&s.id).and_then(|t| t.1.clone()),
    };
    // What taking a session does: the rules of `reopen`, then of `open`.
    let action_for = |s: &AgentSession, conversation: bool| {
        if s.is_open() {
            if s.thread_id == thread {
                EarlierAction::Switch
            } else {
                EarlierAction::None
            }
        } else if s.thread_id == thread && s.closed_reason == Some(SessionCloseReason::Closed) {
            EarlierAction::Reopen
        } else if conversation || !s.resume_session_id.is_empty() {
            EarlierAction::Resume
        } else {
            EarlierAction::None
        }
    };
    // The session to show for a conversation: an open one, else one closed
    // here by the person, else the newest.
    let rank = |s: &AgentSession| {
        (
            s.is_open(),
            s.thread_id == thread && s.closed_reason == Some(SessionCloseReason::Closed),
            s.closed_at.or(Some(s.updated_at)),
        )
    };

    let mut used = std::collections::HashSet::new();
    let mut rows: Vec<(Option<Timestamp>, EarlierSession)> = Vec::new();
    for (harness, conversation) in conversations {
        let matched = sessions
            .iter()
            .filter(|s| s.harness == harness && s.resume_session_id == conversation.id)
            .max_by_key(|s| rank(s));
        if let Some(s) = matched {
            used.insert(s.id);
        }
        let action = match matched {
            Some(s) => action_for(s, true),
            None => EarlierAction::Resume,
        };
        let at = conversation
            .updated_at
            .or(matched.and_then(|s| s.closed_at.or(Some(s.updated_at))));
        rows.push((
            at,
            EarlierSession {
                harness,
                session: matched.map(past),
                conversation: Some(conversation),
                action,
            },
        ));
    }
    // Closed sessions no conversation listing carried (its harness keeps no
    // list, or the conversation is gone): left out when the slot never
    // started — no native id, no turns.
    for s in &sessions {
        if s.is_open() || used.contains(&s.id) {
            continue;
        }
        let started = !s.resume_session_id.is_empty() || turns.contains_key(&s.id);
        if !started {
            continue;
        }
        rows.push((
            s.closed_at.or(Some(s.updated_at)),
            EarlierSession {
                harness: s.harness.clone(),
                session: Some(past(s)),
                conversation: None,
                action: action_for(s, false),
            },
        ));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    Ok(EarlierSessions {
        rows: rows.into_iter().map(|(_, r)| r).collect(),
        unavailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use oxplow_domain::agent::harness::{Launch, LaunchInput};
    use oxplow_domain::agent::observe::HookAnswer;
    use oxplow_domain::agent::tool::ToolUse;
    use oxplow_domain::agent_session::BindingOrigin;
    use oxplow_domain::AgentSessionId;

    /// A harness that lists what it is told to, or fails.
    struct Listing {
        id: &'static str,
        answer: Result<Vec<NativeConversation>, String>,
    }

    #[async_trait]
    impl AgentHarness for Listing {
        fn id(&self) -> &str {
            self.id
        }
        fn title(&self) -> &str {
            self.id
        }
        async fn launch(&self, _input: &LaunchInput) -> Result<Launch, HarnessError> {
            Err(HarnessError::Unsupported {
                harness: self.id.into(),
                what: "launch".into(),
            })
        }
        async fn tool_use(&self, _body: &serde_json::Value) -> Option<ToolUse> {
            None
        }
        async fn render(&self, _answer: &HookAnswer) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn conversations(
            &self,
            _query: &ConversationQuery,
        ) -> Result<Vec<NativeConversation>, HarnessError> {
            self.answer.clone().map_err(HarnessError::Unavailable)
        }
    }

    fn conversation(id: &str, minute: u32) -> NativeConversation {
        NativeConversation {
            id: id.into(),
            title: Some(format!("about {id}")),
            first_prompt: None,
            started_at: None,
            updated_at: Some(
                oxplow_domain::Timestamp::parse(&format!("2026-10-10T10:{minute:02}:00Z")).unwrap(),
            ),
            size_bytes: None,
            parent: None,
        }
    }

    fn harness(id: &'static str, found: Vec<NativeConversation>) -> Arc<dyn AgentHarness> {
        Arc::new(Listing {
            id,
            answer: Ok(found),
        })
    }

    /// A session of `harness` on `thread`, on `native` when given, closed
    /// for `reason` when given.
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
                    s::bind_tx(tx, row.id, native, BindingOrigin::Started, now)?;
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
        let stream = fx.svc.streams.ensure_primary().await.unwrap().id;
        let other = crate::test_fixtures::new_thread(&fx.svc, stream, "elsewhere").await;
        Fixture {
            other: other.id,
            fx,
        }
    }

    async fn list(f: &Fixture, harnesses: Vec<Arc<dyn AgentHarness>>) -> EarlierSessions {
        earlier_sessions_of(&f.fx.svc, f.fx.thread, harnesses)
            .await
            .unwrap()
    }

    fn row<'a>(out: &'a EarlierSessions, native: &str) -> &'a EarlierSession {
        out.rows
            .iter()
            .find(|r| r.conversation.as_ref().is_some_and(|c| c.id == native))
            .unwrap_or_else(|| panic!("no row for {native}: {:?}", out.rows))
    }

    /// A session this thread's person closed and its conversation are one
    /// row, and taking it reopens the session.
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
        let out = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
        assert_eq!(out.rows.len(), 1, "{:?}", out.rows);
        let r = &out.rows[0];
        assert_eq!(r.action, EarlierAction::Reopen);
        assert_eq!(r.session.as_ref().unwrap().session, agent_session_ref(s));
        assert!(r.session.as_ref().unwrap().here);
        assert_eq!(r.conversation.as_ref().unwrap().id, "c1");
    }

    /// One closed in another thread has its conversation free: resume it
    /// here, the row naming the thread it came from.
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
        let out = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
        let r = row(&out, "c1");
        assert_eq!(r.action, EarlierAction::Resume);
        let past = r.session.as_ref().unwrap();
        assert!(!past.here);
        assert_eq!(past.thread_title, "elsewhere");
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
        let out = list(&f, vec![harness("h", vec![conversation("c1", 5)])]).await;
        assert_eq!(row(&out, "c1").action, EarlierAction::Resume);
    }

    /// A conversation begun outside oxplow has no session: resume it.
    #[tokio::test]
    async fn an_outside_conversation_resumes() {
        let f = fixture().await;
        let out = list(&f, vec![harness("h", vec![conversation("x1", 5)])]).await;
        let r = row(&out, "x1");
        assert_eq!(r.action, EarlierAction::Resume);
        assert!(r.session.is_none());
    }

    /// A conversation open in this thread's session is a tab to show; one
    /// open in another thread's is nothing to take.
    #[tokio::test]
    async fn a_conversation_that_is_open_is_switched_to_or_left() {
        let f = fixture().await;
        session(&f.fx.svc, f.fx.thread, "h", Some("here"), None).await;
        session(&f.fx.svc, f.other, "h", Some("there"), None).await;
        let out = list(
            &f,
            vec![harness(
                "h",
                vec![conversation("here", 5), conversation("there", 6)],
            )],
        )
        .await;
        assert_eq!(row(&out, "here").action, EarlierAction::Switch);
        let there = row(&out, "there");
        assert_eq!(there.action, EarlierAction::None);
        assert!(there.session.as_ref().unwrap().open);
        assert_eq!(there.session.as_ref().unwrap().thread_title, "elsewhere");
    }

    /// A harness that can't answer is a line; the others' rows still come.
    #[tokio::test]
    async fn a_failing_harness_is_unavailable_and_the_rest_still_list() {
        let f = fixture().await;
        let broken: Arc<dyn AgentHarness> = Arc::new(Listing {
            id: "down",
            answer: Err("the daemon isn't running".into()),
        });
        let out = list(&f, vec![broken, harness("h", vec![conversation("c1", 5)])]).await;
        assert_eq!(out.rows.len(), 1);
        assert_eq!(
            out.unavailable,
            [HarnessUnavailable {
                harness: "down".into(),
                reason: "the daemon isn't running".into()
            }]
        );
    }

    /// A closed slot that never started anything is not an earlier
    /// session; one that ran turns is, whatever its harness lists.
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
        let out = list(&f, vec![harness("h", vec![])]).await;
        assert_eq!(out.rows.len(), 1, "{:?}", out.rows);
        let past = out.rows[0].session.as_ref().unwrap();
        assert_eq!(past.session, agent_session_ref(used));
        assert_eq!(
            (past.turns, past.last_prompt.as_deref()),
            (2, Some("second"))
        );
        assert_eq!(out.rows[0].action, EarlierAction::Reopen);
    }

    /// Most recently active first.
    #[tokio::test]
    async fn rows_come_newest_first() {
        let f = fixture().await;
        let out = list(
            &f,
            vec![harness(
                "h",
                vec![conversation("old", 1), conversation("new", 9)],
            )],
        )
        .await;
        let ids: Vec<_> = out
            .rows
            .iter()
            .map(|r| r.conversation.as_ref().unwrap().id.clone())
            .collect();
        assert_eq!(ids, ["new", "old"]);
    }
}
