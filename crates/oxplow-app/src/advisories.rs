//! Advisories: extension-declared guidance for the coding agent — hints
//! (see `extensions::Advisory`). Core runs each advisory's query at
//! `post-tool-use`, `prompt` or `turn-end` with the thread, stream, turn
//! and request bound, applies its `once_per` rule, and records every hit
//! as a nudge (`v_agent_nudge`): the thread's undelivered nudges go out on
//! its next prompt or tool call, stamped when they do. See
//! `.context/extensions-commands.md` → "Advisories".

use async_trait::async_trait;
use oxplow_db::{OnceScope, SqlCell, SqliteAgentNudgeStore};
use oxplow_domain::events::schema::{EventType as _, ThreadCheckpoint};
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::extension::extensions::{
    Advisory, AdvisoryAudience, AdvisoryOn, AdvisoryOncePer, Extension,
};

/// The params every advisory query may use.
pub const PARAMS: &[&str] = &["thread_id", "stream_id", "turn_id", "request_id"];

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
    /// The request of the turn the trigger belongs to (a prompt's: the turn
    /// it opened or went into).
    pub request: Option<i64>,
    pub session: Option<String>,
}

impl AdvisoryScope {
    fn params(&self) -> Vec<(String, SqlCell)> {
        let cell = |v: Option<i64>| v.map_or(SqlCell::Null(()), SqlCell::Int);
        vec![
            ("thread_id".into(), SqlCell::Int(self.thread)),
            ("stream_id".into(), cell(self.stream)),
            ("turn_id".into(), cell(self.turn)),
            ("request_id".into(), cell(self.request)),
        ]
    }

    fn thread_scope(&self) -> OnceScope {
        OnceScope::thread(self.thread)
    }

    /// The request, else the thread: where deliveries are counted for
    /// muting, and a muted hint stays muted. A new request starts fresh.
    fn work_scope(&self) -> OnceScope {
        match self.request {
            Some(r) => OnceScope::request(self.thread, r),
            None => self.thread_scope(),
        }
    }

    /// The one-shot mark `id` claims under `once_per`, and where: `None`
    /// for the rules that repeat (`row` marks each key, `turn` nothing);
    /// `Err` when the rule can't hold here (once per request with no request,
    /// once per session with none), so the advisory doesn't run.
    fn once_mark(
        &self,
        id: &str,
        once_per: AdvisoryOncePer,
    ) -> Result<Option<(OnceScope, String)>, ()> {
        Ok(match once_per {
            AdvisoryOncePer::Request => match self.request {
                Some(r) => Some((OnceScope::request(self.thread, r), id.to_string())),
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
        match layer
            .read_generation(self.run_in_generation(layer, extensions, on, scope))
            .await
        {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!(%error, "advisory generation read failed");
                Vec::new()
            }
        }
    }

    async fn run_in_generation(
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
                let id = format!("{}/{}", ext.namespace, a.id);
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
                let query = match layer.with_extension_config(ext, &a.query).await {
                    Ok(query) => query,
                    Err(error) => {
                        tracing::warn!(advisory = %id, %error, "advisory configuration failed");
                        continue;
                    }
                };
                let result = match layer
                    .run(
                        oxplow_db::SqlQuery::new(&query)
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
                        let place = if scope.request.is_some() {
                            "this request"
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
    pub thread_store: std::sync::Arc<oxplow_db::SqliteThreadStore>,
    pub worktrees: std::sync::Arc<crate::worktrees::WorktreeRouter>,
    pub extension_catalog: std::sync::Arc<crate::extension::extension_catalog::ExtensionCatalog>,
    pub db: oxplow_db::Database,
    pub sql: crate::sql_gateway::SqlGateway,
    pub collection: crate::collection::CollectionService,
    /// What's active, for an advisory's needs.
    pub capabilities: std::sync::Arc<crate::capabilities::CapabilityRegistry>,
    pub config: std::sync::Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    pub global_config_dir: std::path::PathBuf,
}

pub(crate) fn visit_eligible(
    svc: &AdvisoryDeps,
    on: AdvisoryOn,
    accept: impl Fn(&Advisory) -> bool,
    mut visit: impl FnMut(&Extension, Vec<&Advisory>),
) {
    let root = svc.worktrees.project_dir();
    let catalog = svc.extension_catalog.get(root);
    let mut active = None;
    for ext in catalog.iter().filter(|ext| ext.enabled) {
        let candidates = ext.advisories.iter().filter(|a| a.on == on && accept(a));
        let mut settings = None;
        let selected: Vec<_> = candidates
            .filter(|a| {
                let meets_needs = a.needs.is_empty()
                    || active
                        .get_or_insert_with(|| {
                            svc.capabilities
                                .snapshot(&crate::config_service::read_config(&svc.config))
                        })
                        .unmet(&a.needs)
                        .is_empty();
                meets_needs
                    && a.when.as_ref().is_none_or(|when| {
                        settings
                            .get_or_insert_with(|| {
                                ext.configuration.as_ref().and_then(|decl| {
                                    crate::extension::extension_configuration::effective_values(
                                        root,
                                        &svc.global_config_dir,
                                        &ext.namespace,
                                        decl,
                                    )
                                    .ok()
                                })
                            })
                            .as_ref()
                            .is_some_and(|values| when.holds(values))
                    })
            })
            .collect();
        if !selected.is_empty() {
            visit(ext, selected);
        }
    }
}

/// Run the `on` advisories for `thread`, reading extensions from the main
/// worktree (whatever the thread's stream), and record each hit as an undelivered nudge.
/// For an event (`cause`), the turn is the event's anchor; a prompt has
/// none, and its request is that of the thread's newest turn.
pub async fn for_thread(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    on: AdvisoryOn,
    cause: Option<&crate::collection::RunCause>,
) -> Vec<AdvisoryHit> {
    for_thread_matching(svc, thread, on, AdvisoryOrigin::Context(cause), |_| true)
        .await
        .unwrap_or_default()
}

enum AdvisoryOrigin<'a> {
    Context(Option<&'a crate::collection::RunCause>),
    Tool(&'a StoredEvent),
}

/// The consumers the ordered post-tool advisories wait on, across the
/// running extensions: a core consumer by name, the effect or collector
/// consumer for an effect or collector target.
pub(crate) fn ordered_gates(catalog: &[Extension]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for ext in catalog.iter().filter(|e| e.enabled) {
        for a in ext
            .advisories
            .iter()
            .filter(|a| a.on == AdvisoryOn::PostToolUse)
        {
            let after: Vec<_> = a
                .after
                .iter()
                .filter_map(|t| oxplow_domain::ordering::After::parse(t).ok())
                .collect();
            for gate in crate::extension::extensions::descriptors::gates(
                oxplow_domain::contribution::ContributionKind::Advisory,
                &after,
            ) {
                if !out.contains(&gate) {
                    out.push(gate);
                }
            }
        }
    }
    out
}

pub(crate) async fn for_tool_event(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    event: &StoredEvent,
    ordered: bool,
) -> Result<(), DomainError> {
    for_thread_matching(
        svc,
        thread,
        AdvisoryOn::PostToolUse,
        AdvisoryOrigin::Tool(event),
        |a| a.after.is_empty() != ordered,
    )
    .await
    .map(|_| ())
}

async fn for_thread_matching(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    on: AdvisoryOn,
    origin: AdvisoryOrigin<'_>,
    accept: impl Fn(&Advisory) -> bool,
) -> Result<Vec<AdvisoryHit>, DomainError> {
    match svc
        .sql
        .read_generation(for_thread_in_generation(svc, thread, on, origin, accept))
        .await
    {
        Ok(hits) => hits,
        Err(error) => {
            tracing::warn!(%error, "thread advisory generation read failed");
            Ok(Vec::new())
        }
    }
}

async fn for_thread_in_generation(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    on: AdvisoryOn,
    origin: AdvisoryOrigin<'_>,
    accept: impl Fn(&Advisory) -> bool,
) -> Result<Vec<AdvisoryHit>, DomainError> {
    use oxplow_domain::stores::ThreadStore as _;
    let mut extensions = Vec::new();
    visit_eligible(svc, on, accept, |ext, advisories| {
        let mut ext = ext.clone();
        ext.advisories = advisories.into_iter().cloned().collect();
        extensions.push(ext);
    });
    if extensions.is_empty() {
        return Ok(Vec::new());
    }
    let tool_cause;
    let cause = match origin {
        AdvisoryOrigin::Context(cause) => cause,
        AdvisoryOrigin::Tool(event) => {
            tool_cause = crate::post_tool_reactors::cause_of(&svc.db, event).await?;
            Some(&tool_cause)
        }
    };
    let stream = match svc.thread_store.get(thread).await {
        Ok(Some(t)) => Some(t.stream_id),
        _ => None,
    };
    let scope = AdvisoryScope {
        thread: thread.value(),
        stream: stream.map(|s| s.value()),
        turn: cause.and_then(|c| c.anchors.turn_id),
        request: request_of(&svc.sql, thread, cause.and_then(|c| c.anchors.turn_id)).await,
        session: session_of(&svc.sql, thread).await,
    };
    let hits = svc.advisories.run(&svc.sql, &extensions, on, &scope).await;
    for hit in &hits {
        svc.collection
            .persist_nudge(
                thread,
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
    Ok(hits)
}

/// The request a trigger belongs to: that of `turn` when the trigger names
/// one (a tool call's, a turn end's), else of the thread's newest turn (a
/// prompt's: the turn it opened or went into).
pub(crate) async fn request_of(
    sql: &crate::sql_gateway::SqlGateway,
    thread: &oxplow_domain::ThreadId,
    turn: Option<i64>,
) -> Option<i64> {
    let (query, arg) = match turn {
        Some(t) => ("SELECT request_id FROM v_agent_turn WHERE id = ?1", t),
        None => (
            "SELECT request_id FROM v_agent_turn WHERE thread_id = ?1 ORDER BY id DESC LIMIT 1",
            thread.value(),
        ),
    };
    let rows = sql
        .query_sql(query, vec![SqlCell::Int(arg)], None)
        .await
        .ok()?
        .rows;
    match rows.first().and_then(|r| r.first()) {
        Some(SqlCell::Int(id)) => Some(*id),
        _ => None,
    }
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

/// Runs the `turn-end` advisories on each `thread.checkpoint`, for the
/// turn the checkpoint ends (its request is the turn's).
pub struct TurnEndAdvisories {
    pub deps: AdvisoryDeps,
}

#[async_trait]
impl AsyncEventConsumer for TurnEndAdvisories {
    fn name(&self) -> &'static str {
        TURN_END
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == ThreadCheckpoint::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let (Some(thread), Some(_)) = (
            event.envelope.anchors.thread_id,
            event.envelope.anchors.turn_id,
        ) else {
            return Ok(());
        };
        let cause = crate::collection::RunCause {
            event_id: event.envelope.id.as_str().to_string(),
            seq: event.seq,
            anchors: event.envelope.anchors.clone(),
            at: event.envelope.at,
            started: None,
        };
        for_thread(&self.deps, &thread, AdvisoryOn::TurnEnd, Some(&cause)).await;
        Ok(())
    }
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
    use crate::extension::extensions::Advisory;

    fn ext(advisories: Vec<Advisory>) -> Extension {
        let mut e =
            crate::extension::extensions::load_extensions(tempfile::tempdir().unwrap().path())
                .into_iter()
                .find(|e| e.name == "oxplow-foundation")
                .unwrap();
        e.name = "x".into();
        e.namespace = "x".into();
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
            audience: crate::extension::extensions::AdvisoryAudience::Agent,
            needs: Vec::new(),
            when: None,
            after: Vec::new(),
        }
    }

    /// The scope of thread 1, outside any request.
    fn on_thread() -> AdvisoryScope {
        AdvisoryScope {
            thread: 1,
            stream: Some(1),
            turn: None,
            request: None,
            session: None,
        }
    }

    /// The scope of request `id` on thread 1.
    fn on_request(request: i64) -> AdvisoryScope {
        AdvisoryScope {
            thread: 1,
            stream: Some(1),
            turn: None,
            request: Some(request),
            session: None,
        }
    }

    /// A layer and a runner over one database holding requests 1–5 (a
    /// once-mark references its request).
    async fn setup() -> (crate::sql_gateway::SqlGateway, AdvisoryRunner) {
        let db = oxplow_db::Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                   VALUES (1, 1, 't', '2026-01-01', '2026-01-01');
                 INSERT INTO request (id, thread_id) VALUES (1, 1), (2, 1), (3, 1), (4, 1), (5, 1);",
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
    async fn once_per_request_fires_only_with_rows_and_durably() {
        let (l, runner) = setup().await;
        let e = vec![ext(vec![adv(
            "low",
            AdvisoryOn::PostToolUse,
            AdvisoryOncePer::Request,
            "SELECT 'add tests (' || :request_id || ')' AS message WHERE :request_id <> 3",
        )])];
        let (l, e, runner) = (&l, &e, &runner);
        let run = |id: i64| async move {
            runner
                .run(l, e, AdvisoryOn::PostToolUse, &on_request(id))
                .await
        };
        assert_eq!(
            run(1).await,
            vec![AdvisoryHit {
                id: "x/low".into(),
                text: "add tests (1)".into(),
                audience: AdvisoryAudience::Agent,
            }]
        );
        assert!(run(3).await.is_empty(), "no rows, no hit");
        assert!(
            runner
                .run(l, e, AdvisoryOn::Prompt, &on_request(5))
                .await
                .is_empty(),
            "only advisories for this hook point run"
        );
        // Durable: a runner over the same database after a restart agrees.
        let again = AdvisoryRunner::new(runner.marks.clone());
        assert!(again
            .run(l, e, AdvisoryOn::PostToolUse, &on_request(1))
            .await
            .is_empty());
    }

    /// A once-per-request hint fires once in a request and again in the next
    /// (`:request_id` is bound); a hint that keeps firing is muted in the
    /// request it reached the agent three times in, and a new request
    /// starts fresh.
    #[tokio::test]
    async fn once_per_request_fires_once_per_request_and_mutes_per_request() {
        let (l, runner) = setup().await;
        let e = vec![ext(vec![adv(
            "per-request",
            AdvisoryOn::PostToolUse,
            AdvisoryOncePer::Request,
            "SELECT 'request ' || :request_id AS message",
        )])];
        let (l, e, runner) = (&l, &e, &runner);
        let run = |r: i64| async move {
            runner
                .run(l, e, AdvisoryOn::PostToolUse, &on_request(r))
                .await
        };
        assert_eq!(run(1).await[0].text, "request 1");
        assert!(run(1).await.is_empty(), "once per request");
        assert_eq!(
            run(2).await[0].text,
            "request 2",
            "the next request is fresh"
        );
        // With no request to scope it, a once-per-request hint doesn't run.
        assert!(runner
            .run(l, e, AdvisoryOn::PostToolUse, &on_thread())
            .await
            .is_empty());

        // Three deliveries in request 1 mute a hint that still fires there;
        // request 2 is unaffected.
        let every = vec![ext(vec![adv(
            "again",
            AdvisoryOn::PostToolUse,
            AdvisoryOncePer::Turn,
            "SELECT 'm' AS message",
        )])];
        runner
            .marks
            .database()
            .transaction(|tx| {
                tx.execute_batch(
                    "INSERT INTO agent_nudge (thread_id, kind, message, created_at, audience, delivered_at, request_id) VALUES
                       (1, 'x/again', 'm', 'a', 'agent', 'b', 1), (1, 'x/again', 'm', 'a', 'agent', 'b', 1),
                       (1, 'x/again', 'm', 'a', 'agent', 'b', 1);",
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let first = runner
            .run(l, &every, AdvisoryOn::PostToolUse, &on_request(1))
            .await;
        assert_eq!(first[0].id, MUTED_KIND);
        let second = runner
            .run(l, &every, AdvisoryOn::PostToolUse, &on_request(2))
            .await;
        assert_eq!(second[0].id, "x/again", "a new request starts fresh");
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
        let first = runner.run(&l, &e, AdvisoryOn::Prompt, &on_thread()).await;
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
        let second = runner.run(&l, &e, AdvisoryOn::Prompt, &on_thread()).await;
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
            .run(&l, &[off, bad], AdvisoryOn::Prompt, &on_thread())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn thread_advisories_record_post_tool_use_nudges() {
        let f = crate::test_fixtures::services_with_thread().await;
        let dir = f.svc.layout.project_dir.join(".oxplow/extensions/guide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: guide\nintent:\n  purpose: test\npermissions: [agent.write]\nadvisories:\n  - id: hello\n    on: post-tool-use\n    once_per: thread\n    query: SELECT 'thread ' || :thread_id AS message\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&f.svc).await;
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
                text: format!("thread {}", f.thread.value()),
                audience: AdvisoryAudience::Agent,
            }]
        );
        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql("SELECT kind, message FROM v_agent_nudge", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["guide/hello", format!("thread {}", f.thread.value())]])
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
        let f = crate::test_fixtures::services_with_thread().await;
        approved(
            &f.svc,
            "guide",
            "  - id: hello\n    on: post-tool-use\n    once_per: thread\n    query: SELECT 'hi' AS message\n",
        )
        .await;
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
    async fn approved(svc: &std::sync::Arc<crate::Services>, name: &str, advisories: &str) {
        let dir = svc.layout.project_dir.join(".oxplow/extensions").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!(
                "manifest: 2\nname: {name}\nintent:\n  purpose: test\npermissions: [agent.write]\nadvisories:\n{advisories}"
            ),
        )
        .unwrap();
        crate::test_fixtures::activate(svc).await;
    }

    /// A turn-end hint is evaluated when the turn's checkpoint lands, with
    /// the thread, stream and turn bound;
    /// it reaches the agent once, at the next prompt, stamped delivered;
    /// `once_per: thread` holds it after that.
    #[tokio::test]
    async fn a_turn_end_hint_reaches_the_next_prompt_once_per_thread() {
        let f = crate::thread_checkpoint::tests::with_baseline().await;
        let svc = &f.svc;
        approved(
            svc,
            "guide",
            "  - id: asked\n    on: turn-end\n    once_per: thread\n    query: SELECT 'thread ' || :thread_id || ' stream ' || :stream_id || ' turn ' || :turn_id AS message\n",
        ).await;
        let settle = || {
            svc.event_pump
                .settle(&[TURN_END], std::time::Duration::from_secs(10))
        };
        crate::thread_checkpoint::tests::turn(&f, None, &["read"]).await;
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
            .prompt_context(svc, &f.thread, Some("s"), None, None)
            .await
            .unwrap_or_default();
        assert!(
            context.contains(&format!(
                "thread {} stream {stream} turn {turn}",
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
            .prompt_context(svc, &f.thread, Some("s"), None, None)
            .await
            .unwrap_or_default();
        assert!(
            !again.contains("guide") && !again.contains("turn "),
            "{again}"
        );
        crate::thread_checkpoint::tests::turn(&f, None, &["read"]).await;
        settle().await;
        let third = svc
            .agent_context
            .prompt_context(svc, &f.thread, Some("s"), None, None)
            .await
            .unwrap_or_default();
        assert!(!third.contains("guide"), "once per thread: {third}");
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

    async fn prompt(f: &crate::test_fixtures::ThreadFixture) -> String {
        f.svc
            .agent_context
            .prompt_context(&f.svc, &f.thread, Some("s"), None, None)
            .await
            .unwrap_or_default()
    }

    /// A hint that keeps firing after three deliveries in a scope stops
    /// reaching the agent there and is raised, once, to the person; its
    /// counts say so.
    #[tokio::test]
    async fn a_hint_that_keeps_firing_is_muted_and_raised_to_the_person() {
        let f = crate::test_fixtures::services_with_thread().await;
        approved(
            &f.svc,
            "guide",
            "  - id: nag\n    on: prompt\n    once_per: turn\n    query: SELECT 'please nag' AS message\n",
        ).await;
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
    /// until someone dismisses it (`oxplow.hint.dismiss`); an agent may too.
    #[tokio::test]
    async fn a_person_hint_waits_for_the_person() {
        let f = crate::test_fixtures::services_with_thread().await;
        approved(
            &f.svc,
            "guide",
            "  - id: tell\n    on: prompt\n    audience: person\n    once_per: thread\n    query: SELECT 'look at this' AS message\n",
        ).await;
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
            session_id: None,
            thread_id: Some(f.thread),
            stream_id: Some(f.svc.streams.list_streams().await.unwrap()[0].id),
        };
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
        // Dismissing it again (an agent may dismiss too) changes nothing and
        // leaves no record; a hint that was never raised is still refused.
        let again = f
            .svc
            .commands
            .run(
                &agent,
                crate::commands::hint::DISMISS,
                serde_json::json!({ "nudge": id }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(again.command_run_id, None);
        let err = f
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::hint::DISMISS,
                serde_json::json!({ "nudge": id + 1000 }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/nudge"),
            "{err:?}"
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
            ..on_thread()
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
}
