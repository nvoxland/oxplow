//! Advisories: extension-declared guidance for the coding agent — hints
//! (see `extensions::Advisory`). Core runs each advisory's query at
//! `post-tool-use`, `prompt` or `turn-end` with the thread, stream, turn
//! and effort bound, applies its `once_per` rule, and records every hit
//! as a nudge (`v_agent_nudge`): the thread's undelivered nudges go out on
//! its next prompt or tool call, stamped when they do. See
//! `.context/extensions.md` → "Advisories".

use std::path::Path;

use async_trait::async_trait;
use oxplow_db::{OnceScope, SqlCell, SqliteAgentNudgeStore};
use oxplow_domain::events::schema::{EventType as _, ThreadCheckpoint};
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::extensions::{AdvisoryAudience, AdvisoryOn, AdvisoryOncePer, Extension};

/// The params every advisory query may use.
pub const PARAMS: &[&str] = &["thread_id", "stream_id", "turn_id", "effort_id"];

/// The turn-end consumer's name (its checkpoint key; what callers settle on).
pub const TURN_END: &str = "advisories.turn_end";

/// Deliveries in a scope after which a hint that still fires is muted
/// there and raised to the person: it isn't working.
pub const MUTE_AFTER: i64 = 3;

/// The kind of the person's notice that a hint was muted.
pub const MUTED_KIND: &str = "hint-muted";

/// One advisory that fired: its id (`<extension>/<advisory>`, or
/// [`MUTED_KIND`]), the text (heading, then one message per line) and who
/// it's for.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvisoryHit {
    pub id: String,
    pub text: String,
    pub audience: AdvisoryAudience,
}

/// What an advisory runs for: the values its params take, and the agent
/// session (for `once_per: session`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryScope {
    pub thread: i64,
    pub stream: Option<i64>,
    pub turn: Option<i64>,
    pub effort: Option<i64>,
    pub session: Option<String>,
}

impl AdvisoryScope {
    fn params(&self) -> Vec<(String, SqlCell)> {
        let cell = |v: Option<i64>| v.map_or(SqlCell::Null(()), SqlCell::Int);
        vec![
            ("thread_id".into(), SqlCell::Int(self.thread)),
            ("stream_id".into(), cell(self.stream)),
            ("turn_id".into(), cell(self.turn)),
            ("effort_id".into(), cell(self.effort)),
        ]
    }

    fn thread_scope(&self) -> OnceScope {
        OnceScope::thread(self.thread)
    }

    /// The effort, else the thread: where deliveries are counted for
    /// muting, and a muted hint stays muted.
    fn work_scope(&self) -> OnceScope {
        match self.effort {
            Some(e) => OnceScope::effort(self.thread, e),
            None => self.thread_scope(),
        }
    }

    /// The one-shot mark `id` claims under `once_per`, and where: `None`
    /// for the rules that repeat (`row` marks each key, `turn` nothing);
    /// `Err` when the rule can't hold here (once per effort with no effort,
    /// once per session with none), so the advisory doesn't run.
    fn once_mark(
        &self,
        id: &str,
        once_per: AdvisoryOncePer,
    ) -> Result<Option<(OnceScope, String)>, ()> {
        Ok(match once_per {
            AdvisoryOncePer::Effort => match self.effort {
                Some(e) => Some((OnceScope::effort(self.thread, e), id.to_string())),
                None => return Err(()),
            },
            AdvisoryOncePer::Thread => Some((self.thread_scope(), id.to_string())),
            AdvisoryOncePer::Session => match &self.session {
                Some(s) => Some((self.thread_scope(), format!("{id}@session:{s}"))),
                None => return Err(()),
            },
            AdvisoryOncePer::Day => Some((
                self.thread_scope(),
                format!("{id}@day:{}", time::OffsetDateTime::now_utc().date()),
            )),
            AdvisoryOncePer::Row | AdvisoryOncePer::Turn => None,
        })
    }
}

/// Runs advisories, remembers which have fired (durably, in `once_mark`,
/// so a restart doesn't repeat one), counts their evaluations and mutes
/// the ones that don't work.
pub struct AdvisoryRunner {
    marks: SqliteAgentNudgeStore,
}

impl AdvisoryRunner {
    pub fn new(marks: SqliteAgentNudgeStore) -> Self {
        Self { marks }
    }

    /// Run every `on` advisory of the enabled `extensions` for `scope` and
    /// return the ones that fire. A failing query is logged and skipped.
    pub async fn run(
        &self,
        layer: &crate::sql_gateway::SqlGateway,
        extensions: &[Extension],
        on: AdvisoryOn,
        scope: &AdvisoryScope,
    ) -> Vec<AdvisoryHit> {
        let mut hits = Vec::new();
        let mut evaluated = Vec::new();
        // Marks to record once every query has run, so a failure partway
        // can't consume a one-shot the agent never saw.
        let mut marks: Vec<(OnceScope, String)> = Vec::new();
        for ext in extensions.iter().filter(|e| e.enabled) {
            for a in ext.advisories.iter().filter(|a| a.on == on) {
                let id = format!("{}/{}", ext.name, a.id);
                let Ok(once) = scope.once_mark(&id, a.once_per) else {
                    continue;
                };
                if let Some((at, mark)) = &once {
                    if self.has_fired(*at, mark).await {
                        continue;
                    }
                }
                let mute = format!("mute:{id}");
                if a.audience == AdvisoryAudience::Agent
                    && self.has_fired(scope.work_scope(), &mute).await
                {
                    continue;
                }
                evaluated.push(id.clone());
                let result = match layer
                    .run(
                        oxplow_db::SqlQuery::new(&a.query)
                            .named(scope.params())
                            .limit(None),
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(error) => {
                        tracing::warn!(advisory = %id, %error, "advisory query failed");
                        continue;
                    }
                };
                let col = |name: &str| result.columns.iter().position(|c| c == name);
                let Some(msg_i) = col("message") else {
                    tracing::warn!(advisory = %id, "advisory query returned no `message` column");
                    continue;
                };
                let key_i = col("key");
                let mut lines = Vec::new();
                let mut row_marks = Vec::new();
                for row in &result.rows {
                    let text = cell_text(&row[msg_i]);
                    if text.is_empty() {
                        continue;
                    }
                    if a.once_per == AdvisoryOncePer::Row {
                        let key = key_i.map(|i| cell_text(&row[i])).unwrap_or_default();
                        let mark = format!("{id}#{key}");
                        if self.has_fired(scope.thread_scope(), &mark).await
                            || row_marks.contains(&mark)
                        {
                            continue;
                        }
                        row_marks.push(mark);
                    }
                    lines.push(text);
                }
                if lines.is_empty() {
                    continue;
                }
                let mut text = a.heading.clone().map(|h| vec![h]).unwrap_or_default();
                text.extend(lines);
                let text = text.join("\n");
                if a.audience == AdvisoryAudience::Agent {
                    let delivered = self
                        .marks
                        .delivered(scope.work_scope(), &id)
                        .await
                        .unwrap_or(0);
                    if delivered >= MUTE_AFTER {
                        marks.push((scope.work_scope(), mute));
                        let place = if scope.effort.is_some() {
                            "this effort"
                        } else {
                            "this thread"
                        };
                        hits.push(AdvisoryHit {
                            id: MUTED_KIND.into(),
                            text: format!(
                                "The hint {id} still fired after reaching the agent {delivered} times in {place}, so it's muted there. It said:\n{text}"
                            ),
                            audience: AdvisoryAudience::Person,
                        });
                        continue;
                    }
                }
                marks.extend(once);
                marks.extend(row_marks.into_iter().map(|m| (scope.thread_scope(), m)));
                hits.push(AdvisoryHit {
                    id,
                    text,
                    audience: a.audience,
                });
            }
        }
        for (at, m) in marks {
            if let Err(error) = self.marks.claim_once(at, &m).await {
                tracing::warn!(mark = %m, %error, "recording an advisory mark failed");
            }
        }
        if !evaluated.is_empty() {
            if let Err(error) = self.marks.evaluated(scope.thread, evaluated).await {
                tracing::warn!(%error, "counting advisory evaluations failed");
            }
        }
        hits
    }

    /// A failed read reads as "already fired": suppress rather than nag.
    async fn has_fired(&self, scope: OnceScope, mark: &str) -> bool {
        self.marks.has_fired(scope, mark).await.unwrap_or(true)
    }
}

/// What running a thread's advisories needs (a pump reactor holds its own
/// copy; `Services::advisory_deps` builds one).
#[derive(Clone)]
pub struct AdvisoryDeps {
    pub advisories: std::sync::Arc<AdvisoryRunner>,
    pub effort_store: std::sync::Arc<oxplow_db::SqliteEffortStore>,
    pub thread_store: std::sync::Arc<oxplow_db::SqliteThreadStore>,
    pub worktrees: std::sync::Arc<crate::worktrees::WorktreeRouter>,
    pub approvals: std::sync::Arc<crate::exec_consent::ApprovalStore>,
    pub extension_catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
    pub db: oxplow_db::Database,
    pub sql: crate::sql_gateway::SqlGateway,
    pub collection: crate::collection::CollectionService,
    /// What's active, for an advisory's needs.
    pub capabilities: std::sync::Arc<crate::capabilities::CapabilityRegistry>,
    pub config: std::sync::Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
}

/// Run the `on` advisories for `thread`, reading extensions from the main
/// worktree (whatever the thread's stream), and record each hit as an undelivered nudge.
/// For an event (`cause`), the turn and effort are the event's anchors;
/// for a prompt, the effort is the thread's open one and there's no turn.
pub async fn for_thread(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    on: AdvisoryOn,
    cause: Option<&crate::collection::RunCause>,
) -> Vec<AdvisoryHit> {
    use oxplow_db::EffortStore as _;
    use oxplow_domain::stores::ThreadStore as _;
    let effort = match cause {
        Some(c) => match c.anchors.effort_id {
            Some(id) => svc.effort_store.get_effort(&id).await,
            None => Ok(None),
        },
        None => svc.effort_store.find_open_for_thread(thread).await,
    };
    let Ok(effort) = effort else {
        return Vec::new();
    };
    let stream = match svc.thread_store.get(thread).await {
        Ok(Some(t)) => Some(t.stream_id),
        _ => None,
    };
    // An advisory whose needs aren't met (no work list active) doesn't run.
    let active = svc
        .capabilities
        .snapshot(&crate::config_service::read_config(&svc.config));
    let extensions: Vec<Extension> = consented(
        &svc.approvals,
        &svc.extension_catalog.get(svc.worktrees.project_dir()),
    )
    .into_iter()
    .map(|mut e| {
        e.advisories.retain(|a| active.unmet(&a.needs).is_empty());
        e
    })
    .collect();
    let scope = AdvisoryScope {
        thread: thread.value(),
        stream: stream.map(|s| s.value()),
        turn: cause.and_then(|c| c.anchors.turn_id),
        effort: effort.as_ref().map(|e| e.id.value()),
        session: session_of(&svc.sql, thread).await,
    };
    let hits = svc.advisories.run(&svc.sql, &extensions, on, &scope).await;
    for hit in &hits {
        svc.collection
            .persist_nudge(
                thread,
                effort.as_ref(),
                crate::collection::Raised {
                    kind: &hit.id,
                    message: &hit.text,
                    trigger: "advisory",
                    audience: match hit.audience {
                        AdvisoryAudience::Agent => oxplow_db::Audience::Agent,
                        AdvisoryAudience::Person => oxplow_db::Audience::Person,
                    },
                },
                cause.map_or(
                    crate::collection::RunOrigin::Command { turn: None },
                    crate::collection::RunOrigin::Event,
                ),
            )
            .await;
    }
    hits
}

/// The harness session of the thread's latest turn, if it reported one.
async fn session_of(
    sql: &crate::sql_gateway::SqlGateway,
    thread: &oxplow_domain::ThreadId,
) -> Option<String> {
    let rows = sql
        .query_sql(
            "SELECT session_id FROM v_agent_turn WHERE thread_id = ?1 ORDER BY id DESC LIMIT 1",
            vec![SqlCell::Int(thread.value())],
            None,
        )
        .await
        .ok()?
        .rows;
    match rows.first().and_then(|r| r.first()) {
        Some(SqlCell::Text(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Runs the `turn-end` advisories on each `thread.checkpoint`, after the
/// effort policy and observation (so a turn's new effort and its files are
/// there to read), for the effort holding the turn's end.
pub struct TurnEndAdvisories {
    pub deps: AdvisoryDeps,
}

#[async_trait]
impl AsyncEventConsumer for TurnEndAdvisories {
    fn name(&self) -> &'static str {
        TURN_END
    }

    fn after(&self) -> Vec<String> {
        vec![crate::effort_observation::NAME.to_string()]
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == ThreadCheckpoint::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let (Some(thread), Some(turn)) = (
            event.envelope.anchors.thread_id,
            event.envelope.anchors.turn_id,
        ) else {
            return Ok(());
        };
        let effort = crate::effort_observation::effort_at_turn_end(&self.deps.sql, turn).await?;
        let cause = crate::collection::RunCause {
            event_id: event.envelope.id.as_str().to_string(),
            seq: event.seq,
            anchors: oxplow_domain::Anchors {
                effort_id: effort,
                ..event.envelope.anchors.clone()
            },
            at: event.envelope.at,
            started: None,
        };
        for_thread(&self.deps, &thread, AdvisoryOn::TurnEnd, Some(&cause)).await;
        Ok(())
    }
}

/// The extensions whose advisories may run: bundled ones, and shared ones
/// a person approved as they are now. A teammate's or a
/// git-installed extension can't speak into the agent's context unseen.
pub fn consented(
    approvals: &crate::exec_consent::ApprovalStore,
    extensions: &[Extension],
) -> Vec<Extension> {
    extensions
        .iter()
        .filter(|e| {
            if e.origin == "bundled" || e.advisories.is_empty() {
                return true;
            }
            let p = crate::exec_consent::advisory_program(e);
            p.hash(Path::new(""))
                .is_ok_and(|h| approvals.is_approved(&p.key(), &h))
        })
        .cloned()
        .collect()
}

fn cell_text(c: &SqlCell) -> String {
    match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.trim().to_string(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::Advisory;

    fn ext(advisories: Vec<Advisory>) -> Extension {
        let mut e = crate::extensions::load_extensions(tempfile::tempdir().unwrap().path())
            .into_iter()
            .next()
            .unwrap();
        e.name = "x".into();
        e.advisories = advisories;
        e
    }

    fn adv(id: &str, on: AdvisoryOn, once_per: AdvisoryOncePer, query: &str) -> Advisory {
        Advisory {
            id: id.into(),
            on,
            query: query.into(),
            once_per,
            heading: None,
            audience: crate::extensions::AdvisoryAudience::Agent,
            needs: Vec::new(),
        }
    }

    /// The scope of effort `id` on thread 1.
    fn on(effort: i64) -> AdvisoryScope {
        AdvisoryScope {
            thread: 1,
            stream: Some(1),
            turn: None,
            effort: Some(effort),
            session: None,
        }
    }

    /// A layer and a runner over one database holding efforts 1–5 (a
    /// once-mark references its effort).
    async fn setup() -> (crate::sql_gateway::SqlGateway, AdvisoryRunner) {
        let db = oxplow_db::Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 't', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at, ended_at) VALUES
                   (1, 'work_item:issues:A-1', 1, '2026-01-01', '2026-01-01'),
                   (2, 'work_item:issues:A-2', 1, '2026-01-01', '2026-01-01'),
                   (3, 'work_item:issues:A-3', 1, '2026-01-01', '2026-01-01'),
                   (4, 'work_item:issues:A-4', 1, '2026-01-01', '2026-01-01'),
                   (5, 'work_item:issues:A-5', 1, '2026-01-01', '2026-01-01');",
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        (
            crate::sql_gateway::SqlGateway::new(db.clone()),
            AdvisoryRunner::new(SqliteAgentNudgeStore::new(db)),
        )
    }

    #[tokio::test]
    async fn once_per_effort_fires_once_per_effort_and_only_with_rows() {
        let (l, runner) = setup().await;
        let e = vec![ext(vec![adv(
            "low",
            AdvisoryOn::PostToolUse,
            AdvisoryOncePer::Effort,
            "SELECT 'add tests (' || :effort_id || ')' AS message WHERE :effort_id <> 3",
        )])];
        let (l, e, runner) = (&l, &e, &runner);
        let run = |id: i64| async move { runner.run(l, e, AdvisoryOn::PostToolUse, &on(id)).await };
        assert_eq!(
            run(1).await,
            vec![AdvisoryHit {
                id: "x/low".into(),
                text: "add tests (1)".into(),
                audience: AdvisoryAudience::Agent,
            }]
        );
        assert!(run(1).await.is_empty(), "once per effort");
        assert!(run(3).await.is_empty(), "no rows, no hit");
        assert_eq!(run(2).await.len(), 1, "another effort fires again");
        assert!(
            runner
                .run(l, e, AdvisoryOn::Prompt, &on(5))
                .await
                .is_empty(),
            "only advisories for this hook point run"
        );
        // Durable: a runner over the same database after a restart agrees.
        let again = AdvisoryRunner::new(runner.marks.clone());
        assert!(again
            .run(l, e, AdvisoryOn::PostToolUse, &on(1))
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn once_per_row_fires_each_key_once_and_turn_always() {
        let (l, runner) = setup().await;
        let mut rows = adv(
            "cross",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Row,
            "SELECT 'a crossed' AS message, 'a' AS key UNION ALL SELECT 'b crossed', 'b'",
        );
        rows.heading = Some("# Thresholds".into());
        let every = adv(
            "deltas",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT '- x: 1 → 2' AS message",
        );
        let e = vec![ext(vec![rows, every])];
        let first = runner.run(&l, &e, AdvisoryOn::Prompt, &on(1)).await;
        assert_eq!(
            first,
            vec![
                AdvisoryHit {
                    id: "x/cross".into(),
                    text: "# Thresholds\na crossed\nb crossed".into(),
                    audience: AdvisoryAudience::Agent,
                },
                AdvisoryHit {
                    id: "x/deltas".into(),
                    text: "- x: 1 → 2".into(),
                    audience: AdvisoryAudience::Agent,
                },
            ]
        );
        let second = runner.run(&l, &e, AdvisoryOn::Prompt, &on(1)).await;
        assert_eq!(
            second,
            vec![AdvisoryHit {
                id: "x/deltas".into(),
                text: "- x: 1 → 2".into(),
                audience: AdvisoryAudience::Agent,
            }]
        );
    }

    #[tokio::test]
    async fn disabled_extensions_and_bad_queries_contribute_nothing() {
        let (l, runner) = setup().await;
        let mut off = ext(vec![adv(
            "a",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT 'm' AS message",
        )]);
        off.enabled = false;
        let bad = ext(vec![adv(
            "b",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT nope FROM nowhere",
        )]);
        assert!(runner
            .run(&l, &[off, bad], AdvisoryOn::Prompt, &on(1))
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn thread_advisories_use_the_open_effort_and_record_post_tool_use_nudges() {
        let f = crate::test_fixtures::services_with_effort().await;
        let dir = f.svc.layout.project_dir.join("oxplow/extensions/guide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: guide\nintent:\n  purpose: test\nadvisories:\n  - id: hello\n    on: post-tool-use\n    query: SELECT 'effort ' || :effort_id AS message\n",
        )
        .unwrap();
        // A project extension's advisories are silent until a person
        // approves them.
        assert!(for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None
        )
        .await
        .is_empty());
        let exts = crate::extensions::load_extensions(&f.svc.layout.project_dir);
        let program =
            crate::exec_consent::advisory_program(exts.iter().find(|e| e.name == "guide").unwrap());
        f.svc
            .approvals
            .approve(&program.key(), &program.hash(Path::new("")).unwrap())
            .unwrap();
        let hits = for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None,
        )
        .await;
        assert_eq!(
            hits,
            vec![AdvisoryHit {
                id: "guide/hello".into(),
                text: format!("effort {}", f.effort.value()),
                audience: AdvisoryAudience::Agent,
            }]
        );
        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql("SELECT kind, message FROM v_agent_nudge", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["guide/hello", format!("effort {}", f.effort.value())]])
        );
        assert!(
            for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None)
                .await
                .is_empty()
        );
    }

    /// A thread in a worktree stream runs the main worktree's advisories,
    /// not its own copy's.
    #[tokio::test]
    async fn a_worktree_threads_advisories_come_from_the_main_worktree() {
        let f = crate::test_fixtures::services_with_effort().await;
        approved(
            &f.svc,
            "guide",
            "  - id: hello\n    on: post-tool-use\n    query: SELECT 'hi' AS message\n",
        );
        let worktree = tempfile::tempdir().unwrap();
        let wt = worktree.path().to_string_lossy().to_string();
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE streams SET worktree_path = ?1 WHERE id = 1",
                    [wt.as_str()],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let hits = for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None,
        )
        .await;
        assert_eq!(
            hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            vec!["guide/hello"]
        );
    }

    /// A project extension at `name` with `advisories` (YAML list items),
    /// approved by a person.
    fn approved(svc: &crate::Services, name: &str, advisories: &str) {
        let dir = svc.layout.project_dir.join("oxplow/extensions").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!(
                "manifest: 2\nname: {name}\nintent:\n  purpose: test\nadvisories:\n{advisories}"
            ),
        )
        .unwrap();
        let exts = crate::extensions::load_extensions(&svc.layout.project_dir);
        let program =
            crate::exec_consent::advisory_program(exts.iter().find(|e| e.name == name).unwrap());
        svc.approvals
            .approve(&program.key(), &program.hash(Path::new("")).unwrap())
            .unwrap();
    }

    /// A turn-end hint is evaluated when the turn's checkpoint lands, with
    /// the thread, stream and turn bound and no effort when none is open;
    /// it reaches the agent once, at the next prompt, stamped delivered;
    /// `once_per: thread` holds it after that.
    #[tokio::test]
    async fn a_turn_end_hint_reaches_the_next_prompt_once_per_thread() {
        let f = crate::thread_checkpoint::tests::with_baseline().await;
        let svc = &f.svc;
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::effort::CLOSE,
                serde_json::json!({ "effort": f.effort.to_string() }),
                false,
            )
            .await
            .unwrap();
        approved(
            svc,
            "guide",
            "  - id: asked\n    on: turn-end\n    once_per: thread\n    query: SELECT 'thread ' || :thread_id || ' stream ' || :stream_id || ' turn ' || :turn_id || ' effort ' || coalesce(:effort_id, 'none') AS message\n",
        );
        let settle = || {
            svc.event_pump.settle(
                &[
                    crate::effort_policy::NAME,
                    crate::effort_observation::NAME,
                    TURN_END,
                ],
                std::time::Duration::from_secs(10),
            )
        };
        crate::thread_checkpoint::tests::turn(&f, None, &["Read"]).await;
        settle().await;
        let turn = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql("SELECT max(id) FROM v_agent_turn", vec![], None)
            .await
            .unwrap();
        let turn = serde_json::to_value(&turn.rows).unwrap()[0][0]
            .as_i64()
            .unwrap();
        let stream = svc.streams.list_streams().await.unwrap()[0].id.value();
        let context = svc
            .agent_context
            .prompt_context(svc, &f.thread, Some("s"))
            .await
            .unwrap_or_default();
        assert!(
            context.contains(&format!(
                "thread {} stream {stream} turn {turn} effort none",
                f.thread.value()
            )),
            "{context}"
        );
        let delivered = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql(
                "SELECT kind, turn_id, delivered_at IS NOT NULL FROM v_agent_nudge",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&delivered.rows).unwrap(),
            serde_json::json!([["guide/asked", turn, 1]])
        );
        let again = svc
            .agent_context
            .prompt_context(svc, &f.thread, Some("s"))
            .await
            .unwrap_or_default();
        assert!(
            !again.contains("guide") && !again.contains("turn "),
            "{again}"
        );
        crate::thread_checkpoint::tests::turn(&f, None, &["Read"]).await;
        settle().await;
        let third = svc
            .agent_context
            .prompt_context(svc, &f.thread, Some("s"))
            .await
            .unwrap_or_default();
        assert!(!third.contains("effort none"), "once per thread: {third}");
    }

    /// The bundled `large-uncommitted` hint: at a turn's end, once per
    /// effort, when the open effort holds many files (none committed, or a
    /// commit would have closed it).
    #[tokio::test]
    async fn bundled_large_work_hint_fires_once_at_turn_end() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let cause = crate::collection::RunCause {
            event_id: "evt".into(),
            seq: 1,
            anchors: oxplow_domain::Anchors {
                thread_id: Some(f.thread),
                effort_id: Some(f.effort),
                ..Default::default()
            },
            at: oxplow_domain::Timestamp::now(),
            started: None,
        };
        let hint = || async {
            for_thread(
                &svc.advisory_deps(),
                &f.thread,
                AdvisoryOn::TurnEnd,
                Some(&cause),
            )
            .await
            .into_iter()
            .filter(|h| h.id == "oxplow-bundled/large-uncommitted")
            .collect::<Vec<_>>()
        };
        let record = |n: usize| async move {
            use oxplow_db::EffortStore as _;
            svc.effort_store
                .record_file(
                    &f.effort,
                    &format!("src/f{n}.rs"),
                    oxplow_db::effort_store::EffortFileChange::Updated,
                    oxplow_db::effort_store::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
                    },
                )
                .await
                .unwrap();
        };
        for n in 0..14 {
            record(n).await;
        }
        assert!(hint().await.is_empty(), "14 files is not large");
        record(14).await;
        let fired = hint().await;
        assert_eq!(fired.len(), 1);
        assert!(fired[0].text.contains("15 files"), "{}", fired[0].text);
        assert!(hint().await.is_empty(), "once per effort");
    }

    /// (kind, audience, delivered) of the thread's nudges, oldest first.
    async fn nudges(svc: &crate::Services) -> serde_json::Value {
        let out = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql(
                "SELECT kind, audience, delivered_at IS NOT NULL FROM v_agent_nudge ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&out.rows).unwrap()
    }

    async fn prompt(f: &crate::test_fixtures::EffortFixture) -> String {
        f.svc
            .agent_context
            .prompt_context(&f.svc, &f.thread, Some("s"))
            .await
            .unwrap_or_default()
    }

    /// A hint that keeps firing after three deliveries in a scope stops
    /// reaching the agent there and is raised, once, to the person; its
    /// counts say so.
    #[tokio::test]
    async fn a_hint_that_keeps_firing_is_muted_and_raised_to_the_person() {
        let f = crate::test_fixtures::services_with_effort().await;
        approved(
            &f.svc,
            "guide",
            "  - id: nag\n    on: prompt\n    once_per: turn\n    query: SELECT 'please nag' AS message\n",
        );
        for _ in 0..3 {
            assert!(prompt(&f).await.contains("please nag"));
        }
        let fourth = prompt(&f).await;
        assert!(!fourth.contains("please nag"), "{fourth}");
        assert!(!prompt(&f).await.contains("please nag"));
        assert_eq!(
            nudges(&f.svc).await,
            serde_json::json!([
                ["guide/nag", "agent", 1],
                ["guide/nag", "agent", 1],
                ["guide/nag", "agent", 1],
                ["hint-muted", "person", 0],
            ])
        );
        let stat = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql(
                "SELECT evaluated, fired, delivered, held, muted FROM v_hint_stat WHERE hint = 'guide/nag'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&stat.rows).unwrap(),
            serde_json::json!([[4, 3, 3, 0, 1]])
        );
    }

    /// A person's hint never reaches the agent: it waits in the ledger
    /// until the person dismisses it (`oxplow.hint.dismiss`), which an agent can't.
    #[tokio::test]
    async fn a_person_hint_waits_for_the_person() {
        let f = crate::test_fixtures::services_with_effort().await;
        approved(
            &f.svc,
            "guide",
            "  - id: tell\n    on: prompt\n    audience: person\n    once_per: thread\n    query: SELECT 'look at this' AS message\n",
        );
        assert!(!prompt(&f).await.contains("look at this"));
        assert_eq!(
            nudges(&f.svc).await,
            serde_json::json!([["guide/tell", "person", 0]])
        );
        let id: i64 = {
            let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
                .query_sql("SELECT id FROM v_agent_nudge", vec![], None)
                .await
                .unwrap();
            serde_json::to_value(&out.rows).unwrap()[0][0]
                .as_i64()
                .unwrap()
        };
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(f.thread),
            stream_id: Some(f.svc.streams.list_streams().await.unwrap()[0].id),
        };
        assert!(f
            .svc
            .commands
            .run(
                &agent,
                crate::commands::hint::DISMISS,
                serde_json::json!({ "nudge": id }),
                false
            )
            .await
            .is_err());
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::hint::DISMISS,
                serde_json::json!({ "nudge": id }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            nudges(&f.svc).await,
            serde_json::json!([["guide/tell", "person", 1]])
        );
    }

    /// `once_per: session` fires once per agent session on the thread;
    /// `day` once per day.
    #[tokio::test]
    async fn once_per_session_and_day() {
        let (l, runner) = setup().await;
        let e = vec![ext(vec![
            adv(
                "s",
                AdvisoryOn::Prompt,
                AdvisoryOncePer::Session,
                "SELECT 's' AS message",
            ),
            adv(
                "d",
                AdvisoryOn::Prompt,
                AdvisoryOncePer::Day,
                "SELECT 'd' AS message",
            ),
        ])];
        let in_session = |id: Option<&str>| AdvisoryScope {
            session: id.map(str::to_string),
            ..on(1)
        };
        let ids = |hits: Vec<AdvisoryHit>| hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
        assert_eq!(
            ids(runner
                .run(&l, &e, AdvisoryOn::Prompt, &in_session(Some("a")))
                .await),
            vec!["x/s", "x/d"]
        );
        assert!(runner
            .run(&l, &e, AdvisoryOn::Prompt, &in_session(Some("a")))
            .await
            .is_empty());
        assert_eq!(
            ids(runner
                .run(&l, &e, AdvisoryOn::Prompt, &in_session(Some("b")))
                .await),
            vec!["x/s"],
            "a new session, the same day"
        );
        assert!(
            runner
                .run(&l, &e, AdvisoryOn::Prompt, &in_session(None))
                .await
                .is_empty(),
            "no session: a session hint can't hold"
        );
    }

    /// The bundled `landed-in-progress` hint tells the person, once per
    /// item, that an item a commit landed is still in progress.
    #[tokio::test]
    async fn bundled_landed_hint_tells_the_person_once() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let svc = &f.svc;
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::effort::CLOSE,
                serde_json::json!({ "effort": f.effort.to_string(), "reason": "commit" }),
                false,
            )
            .await
            .unwrap();
        let cause = crate::collection::RunCause {
            event_id: "evt".into(),
            seq: 1,
            anchors: oxplow_domain::Anchors {
                thread_id: Some(f.thread),
                ..Default::default()
            },
            at: oxplow_domain::Timestamp::now(),
            started: None,
        };
        let landed = || async {
            for_thread(
                &svc.advisory_deps(),
                &f.thread,
                AdvisoryOn::TurnEnd,
                Some(&cause),
            )
            .await
            .into_iter()
            .filter(|h| h.id == "oxplow-bundled/landed-in-progress")
            .collect::<Vec<_>>()
        };
        let first = landed().await;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].audience, AdvisoryAudience::Person);
        assert!(
            first[0].text.contains("“t” is still in progress"),
            "{}",
            first[0].text
        );
        assert!(landed().await.is_empty(), "once per item");
    }

    fn delta(
        key: &str,
        title: &str,
        baseline: f64,
        current: f64,
        crossing: Option<&str>,
    ) -> oxplow_db::EffortMetricDelta {
        oxplow_db::EffortMetricDelta {
            key: key.into(),
            title: title.into(),
            unit: None,
            direction: "lower-better".into(),
            kind: "gauge".into(),
            category: None,
            language: None,
            agg: "level".into(),
            baseline: Some(baseline),
            current,
            delta: Some(current - baseline),
            changed: baseline != current,
            attributed_files: None,
            sample_count: 2,
            target: None,
            warn_at: Some(5.0),
            fail_at: Some(10.0),
            crossing: crossing.map(str::to_string),
            latest_run_id: None,
        }
    }

    #[tokio::test]
    async fn bundled_metric_advisories_match_the_old_prompt_text() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        f.svc
            .effort_evidence_store
            .replace(
                effort,
                vec![
                    delta(
                        "test.unsafe_blocks",
                        "unsafe blocks",
                        3.0,
                        12.0,
                        Some("fail"),
                    ),
                    delta("test.flat", "flat", 7.0, 7.0, None),
                    delta("agent.tokens.total", "tokens", 100.0, 5000.0, None),
                    delta("test.ratio", "ratio", 1.5, 1.0, None),
                ],
                Vec::new(),
                String::new(),
            )
            .await
            .unwrap();
        let first = for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None).await;
        assert_eq!(
            first,
            vec![
                AdvisoryHit {
                    id: "oxplow-bundled/metric-deltas".into(),
                    text: "# Metric deltas (this effort)\n- ratio: 1.5 → 1 (Δ -0.5)\n- unsafe blocks: 3 → 12 (Δ +9)\n(Advisory — for awareness, not gating.)".into(),
                    audience: AdvisoryAudience::Agent,
                },
                AdvisoryHit {
                    id: "oxplow-bundled/threshold-crossed".into(),
                    text: "# Metric thresholds\n⚠ unsafe blocks crossed its fail threshold (10)".into(),
                    audience: AdvisoryAudience::Agent,
                },
            ]
        );
        let second = for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None).await;
        assert_eq!(
            second.len(),
            1,
            "the crossing is once per metric: {second:?}"
        );
        assert_eq!(second[0].id, "oxplow-bundled/metric-deltas");
    }

    /// Oxplow sets no coding requirements for a project: a poorly covered
    /// effort draws no bundled nudge to add tests. A project that wants a
    /// coverage target writes its own advisory.
    #[tokio::test]
    async fn bundled_advisories_set_no_coverage_target() {
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .effort_evidence_store
            .replace(
                f.effort.value(),
                Vec::new(),
                vec![oxplow_db::EffortObservation {
                    kind: "diff-coverage".into(),
                    provenance: "observed".into(),
                    source: "post-tool-bash".into(),
                    metric_value: Some(12.0),
                    payload_json: None,
                    local_snapshot_id: None,
                    created_at: oxplow_domain::Timestamp::now(),
                }],
                String::new(),
            )
            .await
            .unwrap();
        let hits = for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None,
        )
        .await;
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_shared_extensions_advisories_speak_only_once_approved() {
        let dir = tempfile::tempdir().unwrap();
        let approvals = crate::exec_consent::ApprovalStore::for_tests(dir.path());
        let mut shared = ext(vec![adv(
            "nag",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Effort,
            "SELECT 'x' AS message",
        )]);
        shared.origin = "project".into();
        shared.name = "team".into();
        let mut bundled = ext(vec![adv(
            "b",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Effort,
            "SELECT 'y' AS message",
        )]);
        bundled.origin = "bundled".into();
        let names = |exts: Vec<Extension>| exts.into_iter().map(|e| e.name).collect::<Vec<_>>();
        let all = vec![shared.clone(), bundled.clone()];

        assert_eq!(
            names(consented(&approvals, &all)),
            vec![bundled.name.clone()]
        );

        let program = crate::exec_consent::advisory_program(&shared);
        let version = program.hash(dir.path()).unwrap();
        approvals.approve(&program.key(), &version).unwrap();
        assert_eq!(
            names(consented(&approvals, &all)),
            vec!["team".to_string(), bundled.name.clone()]
        );

        // Changing what it says needs approving again.
        shared.advisories[0].query = "SELECT 'rm -rf everything' AS message".into();
        assert_eq!(
            names(consented(&approvals, &[shared, bundled.clone()])),
            vec![bundled.name]
        );
    }
}
