//! The `effect.triggers` pump consumer (P8.D10, `.context/extensions.md` →
//! "Effects"): runs each extension effect whose `on:` names a logged event.
//!
//! One async consumer for every effect, like `collector.triggers`. For
//! each event, each effect of an enabled extension in the primary worktree
//! whose `on` names the event's type and whose `where` matches its payload
//! **reacts** at most once:
//!
//! 1. Already reacted (`effect_run`): nothing — a redelivery writes
//!    nothing. A `started` row is a run interrupted after it claimed a
//!    step outside the transaction: recorded `failed` and **never sent
//!    again**.
//! 2. Not approved as it is now, or the event is from before its approval
//!    (`effects::gate`): nothing — it never reacts to the past.
//! 3. The loop guard: an event its own run caused is never its trigger,
//!    and a chain of more than [`MAX_CHAIN`] effect runs stops (`skipped`).
//! 4. Its script composes, over the event and its `input` rows:
//!    `{ skip: "why" }` is `skipped`; `{ commands, events? }` runs as
//!    `command.sequence` by `Actor::Effect` (`CommandBus::run_effect`),
//!    whose `effect_run` row and `effect.result@2` land with the run, its
//!    proposal, or — when it failed before anything ran — here.
//!
//! An effect that fails is recorded, never dead-letters the event: one
//! broken effect doesn't hold up the others. It holds `Services` weakly:
//! the pump that runs it is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_db::effect_run_store::{EffectRunKey, Finished, RunState};
use oxplow_domain::{CommandCall, CommandError, DomainError, StoredEvent};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::effects::{self, EffectDecl, Gate};
use crate::event_pump::AsyncEventConsumer;
use crate::extension_commands::{own_events, ComposedEvent};
use crate::extensions::Extension;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "effect.triggers";

/// The most effect runs a chain of events may pass through before an
/// effect stops reacting to it.
pub const MAX_CHAIN: usize = 4;

pub struct EffectTriggers {
    services: Weak<Services>,
}

impl EffectTriggers {
    pub fn new(services: Weak<Services>) -> Self {
        Self { services }
    }
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(EffectTriggers::new(Arc::downgrade(svc))));
}

/// The enabled extensions' effects, in the primary worktree (where, like
/// providers and commands, they run from).
fn effects(svc: &Services) -> Vec<(Extension, EffectDecl)> {
    svc.extension_catalog
        .get(&svc.layout.project_dir)
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.effects.iter().map(move |d| (e.clone(), d.clone())))
        .collect()
}

fn reacts_to(decl: &EffectDecl, event: &StoredEvent) -> bool {
    decl.on.contains(&event.envelope.event_type)
        && crate::collector_triggers::payload_matches(&decl.filter, &event.envelope.payload)
}

impl EffectTriggers {
    /// The `after:` consumers of the effects `event_type` triggers (all of
    /// them for `None`), limited to consumers the pump has.
    fn predecessors(&self, event_type: Option<&str>) -> Vec<String> {
        let Some(svc) = self.services.upgrade() else {
            return Vec::new();
        };
        let known = svc.event_pump.consumer_names();
        let mut after: Vec<String> = Vec::new();
        for (_, decl) in effects(&svc) {
            if event_type.is_some_and(|t| !decl.on.iter().any(|o| o == t)) {
                continue;
            }
            for name in &decl.after {
                if !known.contains(&name.as_str()) {
                    tracing::warn!(effect = %decl.name(), consumer = %name, "`after` names no consumer; ignored");
                } else if !after.contains(name) {
                    after.push(name.clone());
                }
            }
        }
        after
    }
}

#[async_trait]
impl AsyncEventConsumer for EffectTriggers {
    fn name(&self) -> &'static str {
        NAME
    }

    fn after(&self) -> Vec<String> {
        self.predecessors(None)
    }

    fn after_for(&self, event_type: &str) -> Vec<String> {
        self.predecessors(Some(event_type))
    }

    fn handles(&self, event_type: &str) -> bool {
        let Some(svc) = self.services.upgrade() else {
            return false;
        };
        effects(&svc)
            .iter()
            .any(|(_, d)| d.on.iter().any(|t| t == event_type))
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // Shutting down: the checkpoint stays, a restart delivers it.
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        for (ext, decl) in effects(&svc) {
            if !reacts_to(&decl, event) {
                continue;
            }
            if let Err(error) = react(&svc, &ext, &decl, event).await {
                if matches!(error, DomainError::Busy(_)) {
                    return Err(error);
                }
                tracing::warn!(effect = %decl.name(), %error, "effect failed");
            }
        }
        Ok(())
    }
}

/// Record how `key`'s reaction ended outside a run (skipped, failed
/// before any command ran, interrupted). A reaction already recorded —
/// by its run, its proposal or a concurrent delivery — stays as it is.
async fn finish(svc: &Services, key: &EffectRunKey, done: Finished) -> Result<(), DomainError> {
    let (key, vocabulary) = (key.clone(), svc.vocabulary.clone());
    let recorded = svc
        .db
        .transaction(move |tx| effects::finished_tx(tx, &vocabulary.current(), &key, &done, None))
        .await;
    svc.event_pump.wake();
    match recorded {
        Err(DomainError::Invalid(_)) => Ok(()),
        other => other,
    }
}

fn ended(state: RunState, reason: impl Into<String>) -> Finished {
    Finished {
        state,
        reason: Some(reason.into()),
        audit_id: None,
        proposal_id: None,
    }
}

/// `decl`'s reaction to `event`.
async fn react(
    svc: &Arc<Services>,
    ext: &Extension,
    decl: &EffectDecl,
    event: &StoredEvent,
) -> Result<(), DomainError> {
    let key = EffectRunKey {
        effect: decl.name(),
        event_id: event.envelope.id.to_string(),
        event_seq: event.seq,
    };
    let state = {
        let key = key.clone();
        svc.db
            .read(move |tx| oxplow_db::effect_run_store::state_tx(tx, &key))
            .await?
    };
    match state {
        None => {}
        Some(RunState::Started) => {
            return finish(
                svc,
                &key,
                ended(
                    RunState::Failed,
                    "interrupted: a step outside oxplow may have run, so it isn't sent again",
                ),
            )
            .await;
        }
        Some(_) => return Ok(()),
    }
    let start = effects::start_after(&svc.db, &key.effect).await?;
    let project_dir = &svc.layout.project_dir;
    if effects::gate(&svc.approvals, project_dir, ext, decl, start, event.seq) != Gate::Runs {
        return Ok(());
    }
    let (own, depth) = lineage(svc, event, &format!("effect:{}", key.effect)).await?;
    if own {
        return Ok(());
    }
    if depth >= MAX_CHAIN {
        return finish(
            svc,
            &key,
            ended(
                RunState::Skipped,
                format!("loop guard: {depth} effect runs already led to this event"),
            ),
        )
        .await;
    }
    let composed = match run_script(svc, decl, event).await {
        Ok(c) => c,
        Err(reason) => return finish(svc, &key, ended(RunState::Failed, reason)).await,
    };
    let (calls, events) = match composed {
        Reaction::Skip(why) => return finish(svc, &key, ended(RunState::Skipped, why)).await,
        Reaction::Run { calls, events } => (calls, events),
    };
    let events = match own_events(
        &svc.vocabulary.current(),
        &ext.name,
        &format!("effect:{}", key.effect),
        events,
    ) {
        Ok(events) => events,
        Err(e) => return finish(svc, &key, ended(RunState::Failed, e.to_string())).await,
    };
    let input = json!({ "calls": calls });
    let command = effect_command(&svc.commands, calls, events)
        .map_err(|e| DomainError::Invariant(e.to_string()))?;
    match svc.commands.run_effect(key.clone(), command, input).await {
        // Recorded with the run, or with its proposal.
        Ok(_) | Err(CommandError::Proposed { .. }) => Ok(()),
        Err(CommandError::Busy { message }) => Err(DomainError::Busy(message)),
        Err(e) => finish(svc, &key, ended(RunState::Failed, e.to_string())).await,
    }
}

/// The run of an effect's reaction: `command.sequence`'s spec over what
/// its script composed — `calls`, and its own `events` beside them.
fn effect_command(
    bus: &Arc<crate::commands::CommandBus>,
    calls: Vec<CommandCall>,
    events: Vec<oxplow_domain::Envelope>,
) -> Result<crate::commands::Command, CommandError> {
    use crate::commands::compose::{sequence_spec, Compose, Composer, Composition};
    let spec = sequence_spec();
    let composer: Arc<Composer> = Arc::new(move |_conn, _input| {
        Ok(Composition {
            calls: calls.clone(),
            result: None,
            // Fresh ids each time the bus composes (it may, more than once).
            events: events
                .iter()
                .map(|e| oxplow_domain::Envelope {
                    id: oxplow_domain::EventId::generate(),
                    ..e.clone()
                })
                .collect(),
        })
    });
    crate::commands::Command::new(spec.clone(), Compose::handler(bus, spec, composer))
}

/// What an effect's script decided.
enum Reaction {
    Skip(String),
    Run {
        calls: Vec<CommandCall>,
        events: Vec<ComposedEvent>,
    },
}

/// Read the effect's `input` rows (the event's payload fields bound) and
/// run its script over `{ event, rows }`, sandboxed. `Err` is why it
/// failed.
async fn run_script(
    svc: &Services,
    decl: &EffectDecl,
    event: &StoredEvent,
) -> Result<Reaction, String> {
    let env = &event.envelope;
    let rows = match &decl.input {
        Some(sql) => {
            let query = crate::extension_commands::input_query(sql, &env.payload);
            let result = svc
                .db
                .read(move |tx| oxplow_db::semantic_layer::read_on(tx, &query))
                .await
                .map_err(|e| format!("the `input` query failed: {e}"))?;
            crate::extension_commands::rows_json(&result)
        }
        None => Vec::new(),
    };
    let input = json!({
        "event": {
            "id": env.id.to_string(),
            "type": env.event_type,
            "v": env.v,
            "seq": event.seq,
            "source": env.source,
            "subject": env.subject,
            "payload": env.payload,
        },
        "rows": rows,
    });
    let script = decl.script.clone();
    let out = tokio::task::spawn_blocking(move || {
        use oxplow_collect_plugin::runtime::{run_sandboxed, run_starlark};
        run_sandboxed(
            &crate::extension_commands::COMMAND_SCRIPT_BUDGET,
            move || run_starlark(&script, &input),
        )
    })
    .await
    .map_err(|e| format!("the script panicked: {e}"))?
    .map_err(|e| format!("the script failed: {e}"))?;
    reaction(out)
}

/// `{ skip: "why" }` or `{ commands: [{ name, input }], events?: [...] }`.
fn reaction(value: Value) -> Result<Reaction, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Out {
        #[serde(default)]
        commands: Option<Vec<Call>>,
        #[serde(default)]
        events: Vec<ComposedEvent>,
        #[serde(default)]
        skip: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Call {
        name: String,
        #[serde(default = "empty_object")]
        input: Value,
    }
    fn empty_object() -> Value {
        json!({})
    }
    const SHAPE: &str = "an effect's script returns `{ commands: [{ name, input }], events? }` \
                         or `{ skip: \"why\" }`";
    let out: Out = serde_json::from_value(value).map_err(|e| format!("{SHAPE}: {e}"))?;
    match (out.skip, out.commands) {
        (Some(why), None) if out.events.is_empty() => Ok(Reaction::Skip(why)),
        (Some(_), _) => Err(format!("{SHAPE}: a skip composes nothing")),
        (None, None) => Err(format!("{SHAPE}: it returned neither")),
        (None, Some(commands)) => Ok(Reaction::Run {
            calls: commands
                .into_iter()
                .map(|c| CommandCall {
                    name: c.name,
                    input: c.input,
                })
                .collect(),
            events: out.events,
        }),
    }
}

/// Walk `event`'s causes: whether `source` (an effect's) caused it — its
/// own run's events, which never trigger it — and how many effect runs
/// (`command.executed` from an effect) led to it. Bounded: a chain longer
/// than the guard needs isn't followed.
async fn lineage(
    svc: &Services,
    event: &StoredEvent,
    source: &str,
) -> Result<(bool, usize), DomainError> {
    const MAX_WALK: usize = 64;
    let (first, source) = (event.clone(), source.to_string());
    svc.db
        .read(move |tx| {
            let (mut own, mut depth) = (false, 0usize);
            let mut current = Some(first);
            for _ in 0..MAX_WALK {
                let Some(e) = current.take() else { break };
                let env = &e.envelope;
                if env.source == source {
                    own = true;
                    break;
                }
                if env.event_type == "command.executed" && env.source.starts_with("effect:") {
                    depth += 1;
                }
                current = match &env.cause {
                    Some(cause) => oxplow_db::event_log_store::get_tx(tx, cause)?,
                    None => None,
                };
            }
            Ok((own, depth))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_consent::ProgramKind;
    use oxplow_domain::events::schema::EventType as _;
    use oxplow_domain::events::schema::{
        ActorKind, CommandExecuted, CommandExecutedV2, CommandOutcome, WorkItemEdited,
        WorkItemTransitioned, WorkItemTransitionedV1,
    };
    use oxplow_domain::{Envelope, TaskStatus};
    use std::path::Path;

    const HEAD: &str = "manifest: 2\nname: acme\nsharing: private\nintent: { purpose: Effects., origin: null, examples: [] }\neffects:\n";

    /// Marks a finished item's title.
    const MARK_DONE: &str = "  - id: mark-done\n    summary: Mark a finished item.\n    on: [work_item.transitioned]\n    where: { to: done }\n    input: \"SELECT title FROM v_work_item WHERE ref = :work_item\"\n    entry: mark.star\n";
    const MARK: &str = "def transform(x):\n    ref = x[\"event\"][\"payload\"][\"work_item\"]\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": ref, \"title\": x[\"rows\"][0][\"title\"] + \" (done)\"}}]}\n";

    fn extension(root: &Path, effects: &str, files: &[(&str, &str)]) {
        let dir = root.join("oxplow/extensions/acme");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("extension.yaml"), format!("{HEAD}{effects}")).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
    }

    /// A person approves every effect of `acme` as it is now.
    async fn approve(svc: &Services) {
        let root = &svc.layout.project_dir;
        let config = svc.config.read().unwrap().clone();
        let ext = effects(svc).into_iter().next().unwrap().0;
        for decl in &ext.effects {
            let program = effects::effect_program(&ext, decl);
            crate::exec_consent::approve_program(
                &svc.approvals,
                root,
                &config,
                std::slice::from_ref(&ext),
                ProgramKind::Effect,
                &program.name,
                &program.hash(root).unwrap(),
            )
            .unwrap();
            effects::approved(&svc.db, &decl.name()).await.unwrap();
        }
    }

    async fn log(svc: &Services, env: Envelope) -> StoredEvent {
        let id = env.id.clone();
        svc.event_log_store.append(env).await.unwrap();
        svc.event_log_store.get(id).await.unwrap().unwrap()
    }

    fn transitioned(task: oxplow_domain::TaskId, to: TaskStatus) -> Envelope {
        Envelope::typed::<WorkItemTransitioned>(
            "human",
            &WorkItemTransitionedV1 {
                work_item: oxplow_domain::refs::build::work_item_ref(task),
                from: TaskStatus::InProgress,
                to,
                effort: None,
            },
        )
    }

    async fn title(fx: &crate::test_fixtures::EffortFixture) -> String {
        use oxplow_domain::stores::TaskStore as _;
        fx.svc.task_store.get(fx.task).await.unwrap().unwrap().title
    }

    async fn rows(svc: &Services, sql: &str) -> Value {
        serde_json::to_value(svc.sql.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
    }

    /// An approved effect reacts once per matching event: its command runs
    /// as the effect, caused by the event; its `effect.result` is caused by
    /// that run; a redelivery writes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_approved_effect_reacts_once_with_its_run_caused_by_the_event() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        let before = title(&fx).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        assert!(consumer.handles("work_item.transitioned"));
        consumer.handle(&ev).await.unwrap();
        consumer.handle(&ev).await.unwrap();
        assert_eq!(title(&fx).await, format!("{before} (done)"));
        assert_eq!(
            rows(svc, "SELECT effect, state FROM v_effect_run").await,
            json!([["acme/mark-done", "ok"]])
        );
        let executed = rows(
            svc,
            "SELECT id, cause, source FROM v_event WHERE type = 'command.executed' AND source LIKE 'effect:%'",
        )
        .await;
        assert_eq!(executed[0][1], json!(ev.envelope.id.to_string()));
        assert_eq!(executed[0][2], json!("effect:acme/mark-done"));
        assert_eq!(
            rows(
                svc,
                "SELECT json_extract(payload, '$.outcome'), json_extract(payload, '$.event'), cause FROM v_event WHERE type = 'effect.result'",
            )
            .await,
            json!([["ok", format!("event:{}", ev.envelope.id), executed[0][0]]])
        );
    }

    /// Nothing before approval, and a `where` that doesn't match skips.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_effect_never_reacts_to_the_past_or_to_what_where_excludes() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        let early = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        consumer.handle(&early).await.unwrap();
        approve(svc).await;
        consumer.handle(&early).await.unwrap();
        let blocked = log(svc, transitioned(fx.task, TaskStatus::Blocked)).await;
        consumer.handle(&blocked).await.unwrap();
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_effect_run").await,
            json!([[0]])
        );
    }

    /// A reaction claimed `started` (a step outside oxplow may have run)
    /// and then interrupted is recorded failed and never sent again.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_interrupted_reaction_is_failed_not_sent_again() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        let before = title(&fx).await;
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        let key = EffectRunKey {
            effect: "acme/mark-done".into(),
            event_id: ev.envelope.id.to_string(),
            event_seq: ev.seq,
        };
        svc.db
            .transaction(move |tx| oxplow_db::effect_run_store::claim_tx(tx, &key, "t"))
            .await
            .unwrap();
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&ev)
            .await
            .unwrap();
        assert_eq!(title(&fx).await, before, "not sent again");
        let run = rows(svc, "SELECT state, reason FROM v_effect_run").await;
        assert_eq!(run[0][0], json!("failed"));
        assert!(run[0][1].as_str().unwrap().contains("interrupted"), "{run}");
    }

    /// The loop guard: an effect never reacts to what its own run caused,
    /// and stops after a chain of [`MAX_CHAIN`] effect runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_loop_guard_holds() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let bang = "  - id: bang\n    summary: Add a bang.\n    on: [work_item.edited]\n    input: \"SELECT title FROM v_work_item WHERE ref = :work_item\"\n    entry: bang.star\n";
        let script = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": x[\"event\"][\"payload\"][\"work_item\"], \"title\": x[\"rows\"][0][\"title\"] + \"!\"}}]}\n";
        extension(&svc.layout.project_dir, bang, &[("bang.star", script)]);
        approve(svc).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                "work_item.update",
                json!({ "ref": r, "title": "renamed" }),
                false,
            )
            .await
            .unwrap();
        // Deliver every edit, the effect's own included, until none is new.
        for _ in 0..3 {
            let edits = svc.event_log_store.read_after(0, 10_000).await.unwrap();
            for e in edits
                .iter()
                .filter(|e| e.envelope.event_type == WorkItemEdited::TYPE)
            {
                consumer.handle(e).await.unwrap();
            }
        }
        assert_eq!(title(&fx).await, "renamed!");

        // A chain of four effect runs: the fifth doesn't react.
        let mut cause: Option<oxplow_domain::EventId> = None;
        for i in 0..MAX_CHAIN {
            let mut executed = Envelope::typed::<CommandExecuted>(
                format!("effect:other/e{i}"),
                &CommandExecutedV2 {
                    command: "command.sequence".into(),
                    actor_kind: ActorKind::Effect,
                    actor_id: Some(format!("other/e{i}")),
                    outcome: CommandOutcome::Ok,
                    audit_id: 1,
                    undoable: false,
                },
            );
            executed.cause = cause.clone();
            cause = Some(executed.id.clone());
            log(svc, executed).await;
        }
        let mut chained = Envelope::typed::<WorkItemEdited>(
            "effect:other/e3",
            &serde_json::from_value(json!({ "work_item": r, "fields": ["title"] })).unwrap(),
        );
        chained.cause = cause;
        let chained = log(svc, chained).await;
        consumer.handle(&chained).await.unwrap();
        let guarded = rows(
            svc,
            &format!(
                "SELECT state, reason FROM v_effect_run WHERE event_id = '{}'",
                chained.envelope.id
            ),
        )
        .await;
        assert_eq!(guarded[0][0], json!("skipped"));
        assert!(
            guarded[0][1].as_str().unwrap().contains("loop guard"),
            "{guarded}"
        );
    }

    /// A composed command that asks a person becomes a proposal; the
    /// reaction is recorded `proposed`, with the proposal.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_that_asks_leaves_a_proposal() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let script = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.delete\", \"input\": {\"ref\": x[\"event\"][\"payload\"][\"work_item\"]}}]}\n";
        extension(
            &svc.layout.project_dir,
            &MARK_DONE.replace("entry: mark.star", "entry: drop.star"),
            &[("drop.star", script)],
        );
        approve(svc).await;
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&ev)
            .await
            .unwrap();
        let run = rows(svc, "SELECT state, proposal_id FROM v_effect_run").await;
        assert_eq!(run[0][0], json!("proposed"));
        assert!(run[0][1].is_number(), "{run}");
        assert_eq!(
            rows(svc, "SELECT json_extract(payload, '$.outcome') FROM v_event WHERE type = 'effect.result'").await,
            json!([["proposed"]])
        );
        use oxplow_domain::stores::TaskStore as _;
        assert!(
            svc.task_store.get(fx.task).await.unwrap().is_some(),
            "nothing deleted"
        );
    }
}
