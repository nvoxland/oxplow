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
use serde_json::json;

use crate::effects::{self, EffectDecl, Gate, Reaction};
use crate::event_pump::AsyncEventConsumer;
use crate::extension_commands::own_events;
use crate::extensions::Extension;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "effect.triggers";

pub use crate::event_lineage::MAX_CHAIN;

pub struct EffectTriggers {
    services: Weak<Services>,
    hashes: std::sync::Mutex<FolderHashes>,
    /// The `(effect, consumer)` pairs whose unknown `after` was warned about.
    warned: std::sync::Mutex<std::collections::BTreeSet<(String, String)>>,
    /// How many such warnings it gave (tests).
    warnings: std::sync::atomic::AtomicUsize,
}

/// The effects' approval hashes for one catalog load: each extension's
/// folder hashed the first time one of its effects is gated, again only
/// when the catalog reloads (a file under it changed).
#[derive(Default)]
struct FolderHashes {
    load: Option<Arc<Vec<Extension>>>,
    /// By program key; `None` when its folder couldn't be read.
    by_program: std::collections::BTreeMap<String, Option<String>>,
    /// How many it has computed (tests).
    computed: usize,
}

impl EffectTriggers {
    pub fn new(services: Weak<Services>) -> Self {
        Self {
            services,
            hashes: std::sync::Mutex::default(),
            warned: std::sync::Mutex::default(),
            warnings: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// `program`'s approval hash as of catalog load `load`.
    fn folder_hash(
        &self,
        load: &Arc<Vec<Extension>>,
        project_dir: &std::path::Path,
        program: &crate::exec_consent::ProjectProgram,
    ) -> Option<String> {
        let mut hashes = self.hashes.lock().unwrap_or_else(|e| e.into_inner());
        if !hashes.load.as_ref().is_some_and(|l| Arc::ptr_eq(l, load)) {
            hashes.load = Some(load.clone());
            hashes.by_program.clear();
        }
        if let Some(hash) = hashes.by_program.get(&program.key()) {
            return hash.clone();
        }
        let hash = program.hash(project_dir).ok();
        hashes.computed += 1;
        hashes.by_program.insert(program.key(), hash.clone());
        hash
    }

    #[cfg(test)]
    fn folder_hashes(&self) -> usize {
        self.hashes.lock().unwrap().computed
    }

    #[cfg(test)]
    fn unknown_after_warnings(&self) -> usize {
        self.warnings.load(std::sync::atomic::Ordering::Relaxed)
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
    effects_of(&svc.extension_catalog.get(&svc.layout.project_dir))
}

fn effects_of(load: &[Extension]) -> Vec<(Extension, EffectDecl)> {
    load.iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.effects.iter().map(move |d| (e.clone(), d.clone())))
        .collect()
}

fn reacts_to(decl: &EffectDecl, event: &StoredEvent) -> bool {
    effects::reacts_to(decl, &event.envelope.event_type, &event.envelope.payload)
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
                    let pair = (decl.name(), name.clone());
                    let first = self
                        .warned
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(pair.clone());
                    if first {
                        self.warnings
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(effect = %pair.0, consumer = %name, "`after` names no consumer; ignored");
                    }
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
        let health =
            crate::plugin_health::PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
        let load = svc.extension_catalog.get(&svc.layout.project_dir);
        for (ext, decl) in effects_of(&load) {
            if !reacts_to(&decl, event) {
                continue;
            }
            // Off after three failures in a row, until a person enables it.
            let key = plugin_key(&decl);
            if !matches!(health.disabled_reason(&key).await, Ok(None)) {
                continue;
            }
            let started = std::time::Instant::now();
            let program = effects::effect_program(&ext, &decl);
            let hash = self.folder_hash(&load, &svc.layout.project_dir, &program);
            let approved = effects::is_approved(&svc.approvals, &program, hash.as_deref());
            let counted = match react(&svc, &ext, &decl, approved, event).await {
                Ok(Reacted::Ran) => health.succeeded(&key, Some(started.elapsed())).await,
                Ok(Reacted::Failed(reason)) => health.failed(&key, &reason).await.map(|_| ()),
                // Skipped, proposed or nothing to do: neither counts.
                Ok(Reacted::Other) => Ok(()),
                Err(error) if matches!(error, DomainError::Busy(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(effect = %decl.name(), %error, "effect failed");
                    Ok(())
                }
            };
            if let Err(error) = counted {
                tracing::warn!(effect = %decl.name(), %error, "recording the effect's health failed");
            }
        }
        Ok(())
    }
}

/// Its health's key: the extension, `effect`, its id.
pub fn plugin_key(decl: &EffectDecl) -> oxplow_db::PluginKey {
    oxplow_db::PluginKey {
        plugin: decl.extension.clone(),
        contribution: decl.id.clone(),
        kind: "effect",
    }
}

/// How a reaction went, as health counts it: only `Failed` is a failure.
enum Reacted {
    /// Its commands ran.
    Ran,
    Failed(String),
    /// Skipped, proposed, not its to run (unapproved, before approval,
    /// its own event), or already recorded.
    Other,
}

/// Record how `key`'s reaction ended outside a run (skipped, failed
/// before any command ran, interrupted); whether this recorded it. A
/// reaction already recorded — by its run, its proposal or a concurrent
/// delivery — stays as it is.
async fn finish(svc: &Services, key: &EffectRunKey, done: Finished) -> Result<bool, DomainError> {
    let (key, vocabulary) = (key.clone(), svc.vocabulary.clone());
    let recorded = svc
        .db
        .transaction(move |tx| effects::finished_tx(tx, &vocabulary.current(), &key, &done, None))
        .await;
    svc.event_pump.wake();
    match recorded {
        Ok(()) => Ok(true),
        Err(DomainError::Invalid(_)) => Ok(false),
        Err(e) => Err(e),
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
    approved: bool,
    event: &StoredEvent,
) -> Result<Reacted, DomainError> {
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
            let reason = "interrupted: a step outside oxplow may have run, so it isn't sent again";
            return Ok(
                if finish(svc, &key, ended(RunState::Failed, reason)).await? {
                    Reacted::Failed(reason.into())
                } else {
                    Reacted::Other
                },
            );
        }
        Some(_) => return Ok(Reacted::Other),
    }
    let start = effects::start_after(&svc.db, &key.effect).await?;
    if effects::gate(approved, start, event.seq) != Gate::Runs {
        return Ok(Reacted::Other);
    }
    let lineage =
        crate::event_lineage::lineage(&svc.db, event, &format!("effect:{}", key.effect)).await?;
    // Its own run's events never trigger it, and aren't worth a record.
    if lineage.own {
        return Ok(Reacted::Other);
    }
    if let Some(why) = lineage.refusal() {
        finish(svc, &key, ended(RunState::Skipped, why)).await?;
        return Ok(Reacted::Other);
    }
    // A failure counts once: not when the reaction was already recorded
    // (its run lost a race to another delivery, or finished it failed).
    let failed = |reason: String| async {
        Ok(
            if finish(svc, &key, ended(RunState::Failed, reason.clone())).await? {
                Reacted::Failed(reason)
            } else {
                Reacted::Other
            },
        )
    };
    let composed = match run_script(svc, decl, event).await {
        Ok(c) => c,
        Err(reason) => return failed(reason).await,
    };
    let (calls, events) = match composed {
        Reaction::Skip(why) => {
            finish(svc, &key, ended(RunState::Skipped, why)).await?;
            return Ok(Reacted::Other);
        }
        Reaction::Run { calls, events } => (calls, events),
    };
    let events = match own_events(
        &svc.vocabulary.current(),
        &ext.name,
        &format!("effect:{}", key.effect),
        events,
    ) {
        Ok(events) => events,
        Err(e) => return failed(e.to_string()).await,
    };
    let input = json!({ "calls": calls });
    let command = effect_command(&svc.commands, calls, events)
        .map_err(|e| DomainError::Invariant(e.to_string()))?;
    match svc.commands.run_effect(key.clone(), command, input).await {
        Ok(_) => Ok(Reacted::Ran),
        // Recorded with its proposal: a person decides.
        Err(CommandError::Proposed { .. }) => Ok(Reacted::Other),
        Err(CommandError::Busy { message }) => Err(DomainError::Busy(message)),
        // Another delivery recorded it: not this effect's failure.
        Err(CommandError::Invalid { message, .. })
            if message.contains(effects::ALREADY_REACTED) =>
        {
            Ok(Reacted::Other)
        }
        // Recorded here, or already by the bus (a step failed partway):
        // a failure either way.
        Err(e) => {
            let reason = e.to_string();
            finish(svc, &key, ended(RunState::Failed, reason.clone())).await?;
            Ok(Reacted::Failed(reason))
        }
    }
}

/// The run of an effect's reaction: the registered `command.sequence` —
/// its spec and compiled schema — over what its script composed: `calls`,
/// and its own `events` beside them.
fn effect_command(
    bus: &Arc<crate::commands::CommandBus>,
    calls: Vec<CommandCall>,
    events: Vec<oxplow_domain::Envelope>,
) -> Result<crate::commands::Command, CommandError> {
    use crate::commands::compose::{Compose, Composer, Composition, SEQUENCE};
    let sequence = bus.command(SEQUENCE).ok_or_else(|| CommandError::Failed {
        message: format!("`{SEQUENCE}` isn't registered"),
    })?;
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
    sequence.with_handler(Compose::handler(bus, sequence.spec.clone(), composer))
}

/// Read the effect's `input` rows (the event's payload fields bound) and
/// run its script over `{ event, rows }`, sandboxed. `Err` is why it
/// failed.
async fn run_script(
    svc: &Services,
    decl: &EffectDecl,
    event: &StoredEvent,
) -> Result<Reaction, String> {
    let rows = match &decl.input {
        Some(sql) => {
            let query = crate::extension_commands::input_query(sql, &event.envelope.payload);
            let result = svc
                .db
                .read(move |tx| oxplow_db::semantic_layer::read_on(tx, &query))
                .await
                .map_err(|e| format!("the `input` query failed: {e}"))?;
            crate::extension_commands::rows_json(&result)
        }
        None => Vec::new(),
    };
    let (script, event) = (decl.script.clone(), effects::event_json(event));
    tokio::task::spawn_blocking(move || effects::run_script(&script, event, rows))
        .await
        .map_err(|e| format!("the script panicked: {e}"))?
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
    use serde_json::Value;
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

    /// tsk798: the extension's folder is hashed once per catalog load, not
    /// per event — and an edit (a new load) is hashed again, so the edited
    /// effect stops until a person approves it again.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_folder_is_hashed_once_per_catalog_load() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        for _ in 0..2 {
            let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
            consumer.handle(&ev).await.unwrap();
        }
        assert_eq!(consumer.folder_hashes(), 1);
        std::fs::write(
            svc.layout
                .project_dir
                .join("oxplow/extensions/acme/mark.star"),
            format!("{MARK}# edited\n"),
        )
        .unwrap();
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        consumer.handle(&ev).await.unwrap();
        assert_eq!(consumer.folder_hashes(), 2);
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_effect_run").await,
            json!([[2]]),
            "the edited effect didn't run"
        );
    }

    /// tsk798: an `after` naming no consumer is warned about once, not on
    /// every ordering question the pump asks.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_after_is_warned_about_once() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(
            &svc.layout.project_dir,
            &format!("{MARK_DONE}    after: [no.such.consumer]\n"),
            &[("mark.star", MARK)],
        );
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        for _ in 0..3 {
            assert!(consumer.after().is_empty());
            assert!(consumer.after_for("work_item.transitioned").is_empty());
        }
        assert_eq!(consumer.unknown_after_warnings(), 1);
    }

    /// tsk798: a reaction's command shares `command.sequence`'s compiled
    /// input schema rather than compiling its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reactions_command_shares_the_sequences_validator() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let (a, b) = (
            effect_command(bus, Vec::new(), Vec::new()).unwrap(),
            effect_command(bus, Vec::new(), Vec::new()).unwrap(),
        );
        assert!(a.shares_validator(&b));
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

    /// P8.D11: three failed reactions in a row disable the effect — it
    /// stops reacting — and the disable files a repair work item naming
    /// its failures.
    #[tokio::test(flavor = "multi_thread")]
    async fn three_failures_disable_an_effect_and_file_a_repair_item() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(
            &svc.layout.project_dir,
            MARK_DONE,
            &[("mark.star", "def transform(x):\n    return 1 // 0\n")],
        );
        approve(svc).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        for _ in 0..3 {
            let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
            consumer.handle(&ev).await.unwrap();
        }
        assert_eq!(
            rows(
                svc,
                "SELECT kind, state FROM v_plugin_health WHERE plugin = 'acme'"
            )
            .await,
            json!([["effect", "disabled"]])
        );
        let fourth = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        consumer.handle(&fourth).await.unwrap();
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_effect_run").await,
            json!([[3]]),
            "disabled: it no longer reacts"
        );
        let disabled = svc
            .event_log_store
            .read_after(0, 10_000)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.envelope.event_type == "plugin.disabled")
            .expect("the disable is logged");
        assert_eq!(disabled.envelope.payload["kind"], json!("effect"));
        crate::plugin_repair::PluginRepair::new(Arc::downgrade(svc))
            .handle(&disabled)
            .await
            .unwrap();
        let repair = rows(
            svc,
            "SELECT repair_item FROM v_plugin_health WHERE plugin = 'acme'",
        )
        .await;
        assert!(repair[0][0].is_string(), "a repair item is filed: {repair}");
    }

    /// Skipped and proposed reactions aren't failures.
    #[tokio::test(flavor = "multi_thread")]
    async fn skipped_and_proposed_reactions_dont_count_as_failures() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let skip = MARK_DONE
            .replace("id: mark-done", "id: skips")
            .replace("entry: mark.star", "entry: skip.star");
        let propose = MARK_DONE
            .replace("id: mark-done", "id: proposes")
            .replace("entry: mark.star", "entry: drop.star");
        extension(
            &svc.layout.project_dir,
            &format!("{skip}{propose}"),
            &[
                ("skip.star", "def transform(x):\n    return {\"skip\": \"not today\"}\n"),
                (
                    "drop.star",
                    "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.delete\", \"input\": {\"ref\": x[\"event\"][\"payload\"][\"work_item\"]}}]}\n",
                ),
            ],
        );
        approve(svc).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        for _ in 0..3 {
            let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
            consumer.handle(&ev).await.unwrap();
        }
        assert_eq!(
            rows(
                svc,
                "SELECT state, count(*) FROM v_effect_run GROUP BY state ORDER BY state"
            )
            .await,
            json!([["proposed", 3], ["skipped", 3]])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT count(*) FROM v_plugin_health WHERE state != 'ok'"
            )
            .await,
            json!([[0]])
        );
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// P9.D1: an effect of one extension reacts to another's event type,
    /// seeing it at the owner's newest version — a row logged at v1
    /// reaches it as v2 through the owner's upcast — and the loop guard
    /// counts the other extension's runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_effect_reacts_to_another_extensions_type_at_its_latest_version() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let owner = |types: &str| {
            format!("manifest: 2\nname: acme-pr\nsharing: private\nintent: {{ purpose: PRs., origin: null, examples: [] }}\nevent_types:\n  types:\n{types}")
        };
        let v1 = "    - { type: acme_pr.merged, v: 1, schema: merged.v1.json, summary: A pull request merged. }\n";
        let v2 = "    - { type: acme_pr.merged, v: 2, schema: merged.v2.json, summary: A pull request merged by someone., upcast: merged.star }\n";
        write(
            &root,
            "oxplow/extensions/acme-pr/extension.yaml",
            &owner(v1),
        );
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.v1.json",
            r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
        );
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.v2.json",
            r#"{"type": "object", "required": ["number", "by"], "properties": {"number": {"type": "integer"}, "by": {"type": "string"}}}"#,
        );
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.star",
            "def transform(x):\n    p = dict(x[\"payload\"])\n    p[\"by\"] = \"unknown\"\n    return p\n",
        );
        // The subscriber: retitles the fixture's task with what it saw.
        let task = oxplow_domain::refs::build::work_item_ref(fx.task);
        write(
            &root,
            "oxplow/extensions/acme/extension.yaml",
            &format!("{HEAD}  - id: note\n    summary: Note a merge.\n    on: [acme_pr.merged]\n    entry: note.star\n"),
        );
        write(
            &root,
            "oxplow/extensions/acme/note.star",
            &format!(
                "def transform(x):\n    e = x[\"event\"]\n    return {{\"commands\": [{{\"name\": \"work_item.update\", \"input\": {{\"ref\": \"{task}\", \"title\": \"#%d v%d by %s\" % (e[\"payload\"][\"number\"], e[\"v\"], e[\"payload\"][\"by\"])}}}}]}}\n"
            ),
        );
        svc.vocabulary_service.sync().await.unwrap();
        // The subscriber is `acme`; `effects()` lists the owner first.
        let config = svc.config.read().unwrap().clone();
        let (ext, decl) = effects(svc)
            .into_iter()
            .find(|(e, _)| e.name == "acme")
            .unwrap();
        let program = effects::effect_program(&ext, &decl);
        crate::exec_consent::approve_program(
            &svc.approvals,
            &root,
            &config,
            std::slice::from_ref(&ext),
            ProgramKind::Effect,
            &program.name,
            &program.hash(&root).unwrap(),
        )
        .unwrap();
        effects::approved(&svc.db, &decl.name()).await.unwrap();

        // Logged at v1, while v1 was the newest.
        let ev = log(
            svc,
            Envelope::new("acme_pr.merged", 1, "test", json!({ "number": 12 })).unwrap(),
        )
        .await;
        // The owner publishes v2; the pump delivers the old row at it.
        write(
            &root,
            "oxplow/extensions/acme-pr/extension.yaml",
            &owner(&format!("{v1}{v2}")),
        );
        svc.vocabulary_service.sync().await.unwrap();
        let consumer = Arc::new(EffectTriggers::new(Arc::downgrade(svc)));
        assert!(consumer.handles("acme_pr.merged"));
        crate::event_pump::run_async_handler(
            svc.vocabulary.clone(),
            consumer.clone(),
            Arc::new(ev),
        )
        .await
        .unwrap();
        assert_eq!(title(&fx).await, "#12 v2 by unknown");

        // The loop guard spans extensions: an event four effect runs of
        // other extensions led to isn't reacted to.
        let mut cause: Option<oxplow_domain::EventId> = None;
        for i in 0..MAX_CHAIN {
            let mut executed = Envelope::typed::<CommandExecuted>(
                format!("effect:acme-pr/e{i}"),
                &CommandExecutedV2 {
                    command: "command.sequence".into(),
                    actor_kind: ActorKind::Effect,
                    actor_id: Some(format!("acme-pr/e{i}")),
                    outcome: CommandOutcome::Ok,
                    audit_id: 1,
                    undoable: false,
                },
            );
            executed.cause = cause.clone();
            cause = Some(executed.id.clone());
            log(svc, executed).await;
        }
        let mut chained = Envelope::new(
            "acme_pr.merged",
            2,
            "effect:acme-pr/e3",
            json!({ "number": 13, "by": "ann" }),
        )
        .unwrap();
        chained.cause = cause;
        let chained = log(svc, chained).await;
        consumer.handle(&chained).await.unwrap();
        assert_eq!(title(&fx).await, "#12 v2 by unknown");
        let guarded = rows(
            svc,
            &format!(
                "SELECT state FROM v_effect_run WHERE event_id = '{}'",
                chained.envelope.id
            ),
        )
        .await;
        assert_eq!(guarded, json!([["skipped"]]));
    }
}
