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
//!    whose `effect_run` row and `effect.result@3` land with the run, its
//!    proposal, or — when it failed before anything ran — here.
//!
//! An effect that fails is recorded, never dead-letters the event: one
//! broken effect doesn't hold up the others. It holds `Services` weakly:
//! the pump that runs it is part of it.
//!
//! [`run_reaction`] is that reaction, and it is also what a person's
//! `effect.retry` of a failed one runs (P9.D4): the next **attempt**, the
//! same steps from 3 on, with what started it (`ReactionOrigin`) recorded.
//!
//! **Sent again by itself** (P10, [`auto_retry_due`]): an attempt that
//! failed while every step it composed was a write to a provider keeping
//! `idempotent_writes` ([`safe_to_resend`]) — each step's key is the same
//! on every attempt, so a write that landed lands once — is retried at
//! most twice, 10 s then 60 s after the failure before
//! ([`RETRY_DELAYS`]); waiting, it doesn't count against the effect's
//! health, and the attempt that exhausts the retries counts once.
//! Anything else — a step inside oxplow, another provider, an attempt
//! cut off by oxplow stopping (what it composed isn't kept) — waits for
//! a person's retry, asked first (`.context/providers.md`
//! "Idempotency").

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_db::effect_run_store::{EffectRunKey, Finished, ReactionOrigin, RunState};
use oxplow_domain::{CommandCall, CommandError, DomainError, StoredEvent};
use serde_json::json;
use std::time::Duration;

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

/// Register the consumer on `svc`'s pump (boot, before it spawns), and
/// the commands a person runs effects with (`effect.retry`,
/// `effect.backfill` and its plan): they hold `Services` as the consumer
/// does.
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(EffectTriggers::new(Arc::downgrade(svc))));
    use crate::commands::effect;
    for command in [
        effect::retry_command(Arc::downgrade(svc)),
        effect::backfill_command(Arc::downgrade(svc)),
        effect::backfill_plan_command(Arc::downgrade(svc)),
    ] {
        svc.commands
            .register(command)
            .expect("the effect commands register");
    }
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
            let reacted =
                run_reaction(&svc, &ext, &decl, approved, event, ReactionOrigin::Live).await;
            match reacted {
                Ok(reacted) => count(&health, &decl, &reacted, started.elapsed()).await,
                Err(error) if matches!(error, DomainError::Busy(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(effect = %decl.name(), %error, "effect failed");
                }
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

/// How an attempt at a reaction went, as health counts it: only `Failed`
/// is a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reacted {
    /// Its commands ran.
    Ran,
    Failed(String),
    /// Its script decided there was nothing to do, or the loop guard
    /// stopped it: recorded `skipped`, with why.
    Skipped(String),
    /// A command it composed asks a person: recorded with its proposal.
    Proposed,
    /// It failed, and will be sent again by itself: not counted yet.
    Retrying(String),
    /// A scheduled automatic retry that wasn't sent (tsk887): the failed
    /// attempt's failure stands, a person's to retry, and counts now.
    NotResent(String),
    /// Nothing was attempted: not its to run (unapproved, before its
    /// approval, its own event) or already recorded by another delivery.
    Nothing,
}

/// Count an attempt toward the effect's health: a run is a success, a
/// failure a failure (three in a row disable it); nothing else counts.
pub(crate) async fn count(
    health: &crate::plugin_health::PluginHealth,
    decl: &EffectDecl,
    reacted: &Reacted,
    took: std::time::Duration,
) {
    let key = plugin_key(decl);
    let counted = match reacted {
        Reacted::Ran => health.succeeded(&key, Some(took)).await,
        Reacted::Failed(reason) | Reacted::NotResent(reason) => {
            health.failed(&key, reason).await.map(|_| ())
        }
        Reacted::Skipped(_) | Reacted::Proposed | Reacted::Retrying(_) | Reacted::Nothing => Ok(()),
    };
    if let Err(error) = counted {
        tracing::warn!(effect = %decl.name(), %error, "recording the effect's health failed");
    }
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

/// Why an attempt claimed `started` and never finished is failed.
pub(crate) const INTERRUPTED: &str =
    "interrupted: a step outside oxplow may have run, so it isn't sent again";

/// At start: every attempt a person started (a retry, a backfill) that is
/// still `started` was cut off — the app stopped between its claim and
/// its record — and no pump delivery will find it, so record each failed,
/// interrupted, with what started it (tsk845): Delivery lists it, and a
/// person may retry it. A live one is the pump's: its redelivery finds
/// it. How many were recovered.
pub(crate) async fn recover_interrupted(svc: &Arc<Services>) -> Result<usize, DomainError> {
    let cut_off = svc
        .db
        .read(|tx| oxplow_db::effect_run_store::person_started_tx(tx))
        .await?;
    let mut recovered = 0;
    for key in cut_off {
        if finish(svc, &key, ended(RunState::Failed, INTERRUPTED)).await? {
            recovered += 1;
        }
    }
    Ok(recovered)
}

/// Why [`run_reaction`] made no attempt for a person's `origin` (a
/// retry, a backfill): what they asked for isn't there to do.
pub(crate) const NOT_FAILED: &str = "didn't fail";
pub(crate) const NOT_REACTED: &str = "hasn't reacted";

/// An attempt at `decl`'s reaction to `event`, started by `origin`:
///
/// - `Live` (the pump): the first attempt, made only if there was none —
///   a redelivery finds the reaction and runs nothing (an attempt left
///   `started` is recorded `failed`: interrupted) — and only for an event
///   logged after the effect's approval;
/// - `Retry` (a person's `effect.retry`): the next attempt at a reaction
///   whose latest **failed**; `Invalid` otherwise. The person named the
///   event, so when it was logged doesn't matter;
/// - `Backfill`: the first attempt at an event the effect never reacted
///   to, whenever it was logged;
/// - `Auto`: the next attempt at a failed one scheduled to be sent again,
///   sending exactly what that one composed (tsk887) — a person's retry
///   composes afresh, from what the effect reads now.
///
/// Every origin needs the effect `approved` as it is now, and passes the
/// loop guard.
pub(crate) async fn run_reaction(
    svc: &Arc<Services>,
    ext: &Extension,
    decl: &EffectDecl,
    approved: bool,
    event: &StoredEvent,
    origin: ReactionOrigin,
) -> Result<Reacted, DomainError> {
    let (effect, event_id) = (decl.name(), event.envelope.id.to_string());
    let latest = {
        let (effect, event_id) = (effect.clone(), event_id.clone());
        svc.db
            .read(move |tx| oxplow_db::effect_run_store::latest_tx(tx, &effect, &event_id))
            .await?
    };
    let scheduled = latest.as_ref().is_some_and(|l| l.retry_at.is_some());
    let resend = latest.as_ref().and_then(|l| l.resend.clone());
    let attempt = match (origin, latest.map(|l| (l.attempt, l.state, l.origin))) {
        (ReactionOrigin::Live | ReactionOrigin::Backfill, None) => 1,
        // A live attempt left `started`: the run that claimed it was cut
        // off. A person's (a retry, a backfill) under way is theirs to
        // record — or the next start's, when it was cut off (tsk847).
        (ReactionOrigin::Live, Some((attempt, RunState::Started, ReactionOrigin::Live))) => {
            let key = EffectRunKey {
                effect,
                event_id,
                event_seq: event.seq,
                attempt,
                origin,
            };
            return Ok(
                if finish(svc, &key, ended(RunState::Failed, INTERRUPTED)).await? {
                    Reacted::Failed(INTERRUPTED.into())
                } else {
                    Reacted::Nothing
                },
            );
        }
        (ReactionOrigin::Live | ReactionOrigin::Backfill, Some(_)) => return Ok(Reacted::Nothing),
        (ReactionOrigin::Retry, Some((attempt, RunState::Failed, _))) => attempt + 1,
        // Sent again by itself: only the failed attempt it was scheduled
        // for (a person's retry since is the latest, and isn't).
        (ReactionOrigin::Auto, Some((attempt, RunState::Failed, _))) if scheduled => attempt + 1,
        (ReactionOrigin::Auto, _) => return Ok(Reacted::Nothing),
        (ReactionOrigin::Retry, Some((_, state, _))) => {
            return Err(DomainError::Invalid(format!(
                "effect `{effect}`'s reaction to event {event_id} {NOT_FAILED} (it is `{}`): \
                 only a failed reaction is retried",
                state.as_str()
            )))
        }
        (ReactionOrigin::Retry, None) => {
            return Err(DomainError::Invalid(format!(
                "effect `{effect}` {NOT_REACTED} to event {event_id}: there is nothing to retry"
            )))
        }
    };
    let key = EffectRunKey {
        effect,
        event_id,
        event_seq: event.seq,
        attempt,
        origin,
    };
    // The live consumer never reacts to the past; a person's retry or
    // backfill names what to react to, and needs only the approval.
    let runs = match origin {
        ReactionOrigin::Live => {
            let start = effects::start_after(&svc.db, &key.effect).await?;
            effects::gate(approved, start, event.seq) == Gate::Runs
        }
        ReactionOrigin::Retry | ReactionOrigin::Backfill | ReactionOrigin::Auto => approved,
    };
    if !runs {
        return Ok(Reacted::Nothing);
    }
    let lineage =
        crate::event_lineage::lineage(&svc.db, event, &format!("effect:{}", key.effect)).await?;
    // Its own run's events never trigger it, and aren't worth a record.
    if lineage.own {
        return Ok(Reacted::Nothing);
    }
    let skipped = |why: String| async {
        finish(svc, &key, ended(RunState::Skipped, why.clone())).await?;
        Ok(Reacted::Skipped(why))
    };
    if let Some(why) = lineage.refusal() {
        return skipped(why).await;
    }
    // A failure counts once: not when the attempt was already recorded
    // (its run lost a race to another delivery, or finished it failed).
    let failed = |reason: String| async {
        Ok(
            if finish(svc, &key, ended(RunState::Failed, reason.clone())).await? {
                Reacted::Failed(reason)
            } else {
                Reacted::Nothing
            },
        )
    };
    // An automatic attempt sends exactly what the failed one composed
    // (tsk887): composing afresh could change a step's input — so its
    // key — and make a write that landed again. Unless every step still
    // goes to a provider that keeps `idempotent_writes`, it isn't sent:
    // the failure is a person's.
    let (calls, events) = if origin == ReactionOrigin::Auto {
        let failed_attempt = EffectRunKey {
            attempt: key.attempt - 1,
            ..key.clone()
        };
        let resend = resend
            .and_then(|json| serde_json::from_str::<Resend>(&json).ok())
            .filter(|r| safe_to_resend(svc, &r.calls));
        let Some(Resend { calls, events }) = resend else {
            drop_retry(svc, &failed_attempt).await?;
            return Ok(Reacted::NotResent(NO_LONGER_RESENT.into()));
        };
        (calls, events)
    } else {
        let composed = match run_script(svc, decl, event).await {
            Ok(c) => c,
            Err(reason) => return failed(reason).await,
        };
        match composed {
            Reaction::Skip(why) => return skipped(why).await,
            Reaction::Run { calls, events } => (calls, events),
        }
    };
    let resend = serde_json::to_string(&Resend {
        calls: calls.clone(),
        events: events.clone(),
    })
    .map_err(|e| DomainError::Invariant(e.to_string()))?;
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
    let safe = safe_to_resend(svc, &calls);
    let command = effect_command(&svc.commands, calls, events)
        .map_err(|e| DomainError::Invariant(e.to_string()))?;
    match svc.commands.run_effect(key.clone(), command, input).await {
        Ok(_) => Ok(Reacted::Ran),
        // Recorded with its proposal: a person decides.
        Err(CommandError::Proposed { .. }) => Ok(Reacted::Proposed),
        Err(CommandError::Busy { message }) => Err(DomainError::Busy(message)),
        // Another delivery recorded it: not this effect's failure.
        Err(CommandError::Invalid { message, .. })
            if message.contains(effects::ALREADY_REACTED) =>
        {
            Ok(Reacted::Nothing)
        }
        // Recorded here, or already by the bus (a step failed partway):
        // a failure either way — sent again by itself when it may have
        // been passing (`Unavailable`, tsk914), that is safe, and it has
        // retries left.
        Err(e) => {
            let reason = e.to_string();
            finish(svc, &key, ended(RunState::Failed, reason.clone())).await?;
            if let CommandError::Unavailable { retry_after_ms, .. } = e {
                if safe && schedule_retry(svc, &key, resend, retry_after_ms).await? {
                    return Ok(Reacted::Retrying(reason));
                }
            }
            Ok(Reacted::Failed(reason))
        }
    }
}

/// What a failed attempt composed, kept for its automatic retry to send
/// exactly (tsk887).
#[derive(serde::Serialize, serde::Deserialize)]
struct Resend {
    calls: Vec<CommandCall>,
    events: Vec<crate::extension_commands::ComposedEvent>,
}

/// Why a scheduled automatic retry wasn't sent: what it would send no
/// longer goes only to providers that keep `idempotent_writes`.
pub(crate) const NO_LONGER_RESENT: &str =
    "not sent again by itself: a step's provider no longer keeps idempotent writes";

/// How long after a failed attempt each automatic retry waits: at most
/// two in a row (P10).
pub(crate) const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(10), Duration::from_secs(60)];

/// Whether sending a reaction's `calls` again by itself is safe (P10):
/// every one is a write to a provider outside oxplow that keeps
/// `idempotent_writes`, so — its key the same on every attempt — a write
/// that landed lands once. A step inside oxplow, or toward any other
/// provider, waits for a person.
fn safe_to_resend(svc: &Services, calls: &[CommandCall]) -> bool {
    !calls.is_empty()
        && calls.iter().all(|call| {
            crate::commands::work_item::provider_for(&svc.work_items, &call.name, &call.input)
                .is_some_and(|p| p.external.is_some() && p.features.idempotent_writes)
        })
}

/// The longest a service's own "try again in" is waited for by itself: a
/// longer wait is a person's to retry after (tsk914).
pub(crate) const MAX_ASKED_WAIT: Duration = Duration::from_secs(15 * 60);

/// Schedule `key`'s failed attempt to be sent again — no sooner than its
/// service asked (`retry_after_ms`) — unless the reaction already had its
/// automatic retries, or the service asked for longer than
/// [`MAX_ASKED_WAIT`]: whether it was.
async fn schedule_retry(
    svc: &Services,
    key: &EffectRunKey,
    resend: String,
    retry_after_ms: Option<u64>,
) -> Result<bool, DomainError> {
    let asked = retry_after_ms.map(Duration::from_millis);
    if asked.is_some_and(|a| a > MAX_ASKED_WAIT) {
        return Ok(false);
    }
    let key = key.clone();
    svc.db
        .transaction(move |tx| {
            let made =
                oxplow_db::effect_run_store::automatic_in_a_row_tx(tx, &key.effect, &key.event_id)?;
            let Some(delay) = RETRY_DELAYS.get(made as usize) else {
                return Ok(false);
            };
            let delay = asked.map_or(*delay, |a| a.max(*delay));
            let at = oxplow_domain::Timestamp::from_unix_ms(
                oxplow_domain::Timestamp::now().unix_ms() + delay.as_millis() as i64,
            );
            oxplow_db::effect_run_store::schedule_retry_tx(tx, &key, &at.to_string(), &resend)?;
            Ok(true)
        })
        .await
}

/// Send again every failed attempt whose automatic retry is due by `now`
/// (P10): the next attempt, `auto`, made when the effect is still there,
/// enabled and approved as it is now — otherwise the retry is dropped and
/// the failure counted, a person's to retry. Its health is counted as a
/// live attempt's. How many were sent.
pub async fn auto_retry_due(
    svc: &Arc<Services>,
    now: oxplow_domain::Timestamp,
) -> Result<usize, DomainError> {
    let now = now.to_string();
    let due = svc
        .db
        .read(move |tx| oxplow_db::effect_run_store::due_retries_tx(tx, &now))
        .await?;
    let health = crate::plugin_health::PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
    let mut sent = 0;
    for key in due {
        // At its newest version, as the pump, `effect.retry` and backfill
        // hand it over (tsk911). The attempt sends what the failed one
        // composed (tsk887), so an expired payload doesn't stop it.
        let event = match svc
            .event_log_store
            .get(oxplow_domain::EventId(key.event_id.clone()))
            .await?
        {
            // One that no longer upcasts can't be reacted to: its retry is
            // dropped below, a person's.
            Some(stored) => crate::event_pump::at_latest(&svc.vocabulary.current(), &stored).ok(),
            None => None,
        };
        let effect = find_effect(svc, &key.effect);
        let runnable = match (&effect, &event) {
            (Some((ext, decl)), Some(_)) => {
                matches!(health.disabled_reason(&plugin_key(decl)).await, Ok(None))
                    && approved_now(svc, &effects::effect_program(ext, decl))
            }
            _ => false,
        };
        let (Some((ext, decl)), Some(event), true) = (effect, event, runnable) else {
            drop_retry(svc, &key).await?;
            continue;
        };
        let started = std::time::Instant::now();
        let reacted = run_reaction(svc, &ext, &decl, true, &event, ReactionOrigin::Auto).await?;
        if !matches!(reacted, Reacted::Nothing | Reacted::NotResent(_)) {
            sent += 1;
        }
        count(&health, &decl, &reacted, started.elapsed()).await;
    }
    Ok(sent)
}

/// A scheduled retry that can't run (the effect gone, disabled or no
/// longer approved as it is): dropped, so the failure is a person's.
async fn drop_retry(svc: &Services, key: &EffectRunKey) -> Result<(), DomainError> {
    let key = key.clone();
    svc.db
        .transaction(move |tx| oxplow_db::effect_run_store::drop_retry_tx(tx, &key))
        .await
}

/// Check for due automatic retries every few seconds, for as long as the
/// services live.
pub fn spawn_auto_retry(state: &Arc<Services>) {
    let services = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(svc) = services.upgrade() else {
                return;
            };
            if let Err(error) = auto_retry_due(&svc, oxplow_domain::Timestamp::now()).await {
                tracing::warn!(%error, "sending effect attempts again failed");
            }
        }
    });
}

/// The approval hash of `program`'s folder as it is now.
pub(crate) fn approved_now(svc: &Services, program: &crate::exec_consent::ProjectProgram) -> bool {
    let hash = program.hash(&svc.layout.project_dir).ok();
    effects::is_approved(&svc.approvals, program, hash.as_deref())
}

/// The enabled effect `<extension>/<id>` in the primary worktree, with
/// its extension.
pub(crate) fn find_effect(svc: &Services, name: &str) -> Option<(Extension, EffectDecl)> {
    effects(svc).into_iter().find(|(_, d)| d.name() == name)
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
        interrupt(svc, &ev).await;
        assert_eq!(title(&fx).await, before, "not sent again");
        let run = rows(svc, "SELECT state, reason FROM v_effect_run").await;
        assert_eq!(run[0][0], json!("failed"));
        assert!(run[0][1].as_str().unwrap().contains("interrupted"), "{run}");
    }

    /// tsk845: a person's retry or backfill claimed `started` and then cut
    /// off (the app quit mid-step) is no pump delivery, so no redelivery
    /// ever finds it. Starting up does: each such attempt is recorded
    /// failed — interrupted — with what started it, so Delivery lists it
    /// and a person may retry it. A live one is the pump's to find.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cut_off_backfill_attempt_is_recovered_at_start() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        register(svc);
        let claim = |ev: &StoredEvent, origin| {
            let key =
                EffectRunKey::first("acme/mark-done", ev.envelope.id.to_string(), ev.seq, origin);
            async move {
                svc.db
                    .transaction(move |tx| oxplow_db::effect_run_store::claim_tx(tx, &key, "t"))
                    .await
                    .unwrap()
            }
        };
        let cut_off = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        claim(&cut_off, ReactionOrigin::Backfill).await;
        let live = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        claim(&live, ReactionOrigin::Live).await;

        assert_eq!(recover_interrupted(svc).await.unwrap(), 1);
        assert_eq!(
            rows(
                svc,
                "SELECT origin, state, reason LIKE 'interrupted%' FROM v_effect_run ORDER BY event_seq"
            )
            .await,
            json!([["backfill", "failed", 1], ["live", "started", null]])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT json_extract(payload, '$.origin'), json_extract(payload, '$.outcome') FROM v_event WHERE type = 'effect.result'"
            )
            .await,
            json!([["backfill", "failed"]])
        );
        let human = oxplow_domain::Actor::Human;
        let retried = retry(svc, &human, &cut_off, true).await.unwrap();
        assert_eq!(retried.result["attempt"], json!(2));
        assert_eq!(retried.result["outcome"], json!("ok"));
    }

    /// `acme/mark-done`'s reaction to `ev`, claimed `started` (a step
    /// outside oxplow under way) and then found by a redelivery: failed.
    async fn interrupt(svc: &Arc<Services>, ev: &StoredEvent) {
        let key = EffectRunKey::first(
            "acme/mark-done",
            ev.envelope.id.to_string(),
            ev.seq,
            ReactionOrigin::Live,
        );
        svc.db
            .transaction(move |tx| oxplow_db::effect_run_store::claim_tx(tx, &key, "t"))
            .await
            .unwrap();
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(ev)
            .await
            .unwrap();
    }

    /// A reaction of two steps: the item marked inside oxplow, then a
    /// write outside it (`probe.write`).
    const PROBE: &str = "def transform(x):\n    ref = x[\"event\"][\"payload\"][\"work_item\"]\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": ref, \"title\": \"probed\"}}, {\"name\": \"probe.write\", \"input\": {\"n\": 1}}]}\n";

    /// `probe.write`: a write outside oxplow that keeps each idempotency
    /// key it is sent, and fails while `fail` is set.
    fn register_probe(
        svc: &Services,
        keys: Arc<parking_lot::Mutex<Vec<Option<String>>>>,
        fail: Arc<std::sync::atomic::AtomicBool>,
    ) {
        use crate::commands::{Command, Handler, HandlerOutput, Invocation};
        use oxplow_domain::{Atomicity, CommandEffect, CommandSpec, Confirm, Invokers, Lifecycle};
        let command = Command::new(
            CommandSpec {
                name: "probe.write".into(),
                summary: "Write outside oxplow (a test probe).".into(),
                input_schema: json!({ "type": "object" }),
                invokers: Invokers::ALL,
                confirm: Confirm::Never,
                undoable: false,
                lifecycle: Lifecycle::Stable,
                atomicity: Atomicity::External,
                effect: CommandEffect::Write,
            },
            Handler::External(Arc::new(move |invocation: Invocation, _input| {
                keys.lock().push(invocation.idempotency_key);
                let failing = fail.load(std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move {
                    if failing {
                        return Err(CommandError::Failed {
                            message: "the probe's system is down".into(),
                        });
                    }
                    Ok(HandlerOutput::default())
                })
            })),
        )
        .unwrap();
        svc.commands.register(command).unwrap();
    }

    /// P10: a step outside oxplow carries an idempotency key derived from
    /// the reaction — its effect, its event, the step's position and what
    /// it calls — so every attempt at it sends the same one, and a
    /// provider that keeps `idempotent_writes` does it once. A person's
    /// own run of the command carries none.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_effect_step_sends_the_same_key_on_every_attempt() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", PROBE)]);
        approve(svc).await;
        register(svc);
        let keys: Arc<parking_lot::Mutex<Vec<Option<String>>>> = Arc::default();
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        register_probe(svc, keys.clone(), fail.clone());
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&ev)
            .await
            .unwrap();
        assert_eq!(
            rows(svc, "SELECT state FROM v_effect_run").await,
            json!([["failed"]])
        );
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let human = oxplow_domain::Actor::Human;
        let retried = retry(svc, &human, &ev, true).await.unwrap();
        assert_eq!(retried.result["outcome"], json!("ok"));
        let sent = keys.lock().clone();
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(sent[0], sent[1], "both attempts sent one key");
        let key = sent[0].clone().unwrap_or_default();
        assert!(
            key.starts_with(&format!("effect:acme/mark-done:{}:1:", ev.envelope.id)),
            "{key}"
        );
        svc.commands
            .run(&human, "probe.write", json!({ "n": 1 }), false)
            .await
            .unwrap();
        assert_eq!(keys.lock().last(), Some(&None));
    }

    /// tsk912: a step that landed before a later one failed is sent again
    /// on the retry (its key the same), and its system answers it again —
    /// but its events are logged once, not again under new ids.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_landed_steps_events_are_logged_once_across_a_retry() {
        use crate::commands::{Command, Handler, HandlerOutput, Invocation};
        use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV1};
        use oxplow_domain::{Atomicity, CommandEffect, CommandSpec, Confirm, Invokers, Lifecycle};
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        const TWO_STEPS: &str = "def transform(x):\n    return {\"commands\": [{\"name\": \"probe.note\", \"input\": {\"n\": 1}}, {\"name\": \"probe.write\", \"input\": {\"n\": 2}}]}\n";
        extension(
            &svc.layout.project_dir,
            MARK_DONE,
            &[("mark.star", TWO_STEPS)],
        );
        approve(svc).await;
        register(svc);
        let keys: Arc<parking_lot::Mutex<Vec<Option<String>>>> = Arc::default();
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        register_probe(svc, keys.clone(), fail.clone());
        // A write outside oxplow that always lands, answering with an event.
        let note = Command::new(
            CommandSpec {
                name: "probe.note".into(),
                summary: "Note something outside oxplow (a test probe).".into(),
                input_schema: json!({ "type": "object" }),
                invokers: Invokers::ALL,
                confirm: Confirm::Never,
                undoable: false,
                lifecycle: Lifecycle::Stable,
                atomicity: Atomicity::External,
                effect: CommandEffect::Write,
            },
            Handler::External(Arc::new(move |_invocation: Invocation, _input| {
                Box::pin(async move {
                    Ok(HandlerOutput {
                        events: vec![Envelope::typed::<ConfigChanged>(
                            "probe",
                            &ConfigChangedV1 {
                                key: "probe.noted".into(),
                                before: Value::Null,
                                after: json!(1),
                            },
                        )],
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .unwrap();
        svc.commands.register(note).unwrap();
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&ev)
            .await
            .unwrap();
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let human = oxplow_domain::Actor::Human;
        let retried = retry(svc, &human, &ev, true).await.unwrap();
        assert_eq!(retried.result["outcome"], json!("ok"));
        assert_eq!(
            rows(
                svc,
                "SELECT count(*) FROM v_event WHERE type = 'config.changed' \
                 AND json_extract(payload, '$.key') = 'probe.noted'"
            )
            .await,
            json!([[1]])
        );
    }

    async fn retry(
        svc: &Services,
        actor: &oxplow_domain::Actor,
        ev: &StoredEvent,
        confirmed: bool,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        svc.commands
            .run(
                actor,
                crate::commands::effect::RETRY,
                json!({ "effect": "acme/mark-done", "event": format!("event:{}", ev.envelope.id) }),
                confirmed,
            )
            .await
    }

    /// P9.D4: a person retries a reaction that failed — here one
    /// interrupted with a step outside oxplow under way. It is asked
    /// first, runs the effect once more as the next attempt, and is
    /// recorded with which attempt it was and what started it. An agent
    /// can't; a reaction that didn't fail isn't retried.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_retries_an_interrupted_reaction() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        register(svc);
        let before = title(&fx).await;
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        interrupt(svc, &ev).await;
        assert_eq!(title(&fx).await, before);

        // An agent may not; a person is asked, and told why to think.
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        assert!(matches!(
            retry(svc, &agent, &ev, true).await,
            Err(CommandError::Denied { .. })
        ));
        let human = oxplow_domain::Actor::Human;
        match retry(svc, &human, &ev, false).await {
            Err(CommandError::NeedsConfirmation { preview }) => {
                assert!(
                    preview.summary.contains("may already have run"),
                    "{}",
                    preview.summary
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(title(&fx).await, before, "asking ran nothing");

        let out = retry(svc, &human, &ev, true).await.unwrap();
        assert_eq!(
            out.result,
            json!({ "effect": "acme/mark-done", "event": format!("event:{}", ev.envelope.id), "attempt": 2, "outcome": "ok" })
        );
        assert_eq!(
            title(&fx).await,
            format!("{before} (done)"),
            "ran once more"
        );
        assert_eq!(
            rows(
                svc,
                "SELECT attempt, origin, state, latest FROM v_effect_run ORDER BY attempt"
            )
            .await,
            json!([[1, "live", "failed", 0], [2, "retry", "ok", 1]])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT v, json_extract(payload, '$.attempt'), json_extract(payload, '$.origin'), json_extract(payload, '$.outcome') FROM v_event WHERE type = 'effect.result' ORDER BY seq"
            )
            .await,
            json!([[4, 1, "live", "failed"], [4, 2, "retry", "ok"]])
        );
        // It succeeded: nothing left to retry, and a redelivery of the
        // event still runs nothing.
        let refused = retry(svc, &human, &ev, true).await.unwrap_err();
        assert!(refused.to_string().contains("didn't fail"), "{refused}");
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&ev)
            .await
            .unwrap();
        assert_eq!(title(&fx).await, format!("{before} (done)"));
        // A reaction never made isn't one to retry either.
        let other = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        let refused = retry(svc, &human, &other, true).await.unwrap_err();
        assert!(refused.to_string().contains("hasn't reacted"), "{refused}");
    }

    /// P9.D4: a retry runs the effect as it is now, so it needs what a
    /// live reaction needs — the effect enabled, and approved as it is.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_retry_needs_a_current_approval_and_an_enabled_effect() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        approve(svc).await;
        register(svc);
        let ev = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        interrupt(svc, &ev).await;
        let human = oxplow_domain::Actor::Human;

        // Disabled (three failures would; here a person's record of one).
        let health =
            crate::plugin_health::PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
        let decl = effects(svc).into_iter().next().unwrap().1;
        health
            .disable(&plugin_key(&decl), "off for the test")
            .await
            .unwrap();
        let refused = retry(svc, &human, &ev, true).await.unwrap_err();
        assert!(refused.to_string().contains("is disabled"), "{refused}");
        health.enable(&plugin_key(&decl), "human").await.unwrap();

        // Edited since it was approved: a person approves it first.
        extension(
            &svc.layout.project_dir,
            MARK_DONE,
            &[("mark.star", &format!("{MARK}# edited\n"))],
        );
        let refused = retry(svc, &human, &ev, true).await.unwrap_err();
        assert!(refused.to_string().contains("approv"), "{refused}");
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_effect_run").await,
            json!([[1]]),
            "no attempt was made"
        );
        // No such effect, no such event.
        let unknown = svc
            .commands
            .run(
                &human,
                crate::commands::effect::RETRY,
                json!({ "effect": "acme/nope", "event": format!("event:{}", ev.envelope.id) }),
                true,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(unknown, CommandError::Invalid { .. }),
            "{unknown:?}"
        );
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

    async fn run_as_person(
        svc: &Services,
        name: &str,
        input: Value,
    ) -> Result<Value, CommandError> {
        svc.commands
            .run(&oxplow_domain::Actor::Human, name, input, true)
            .await
            .map(|o| o.result)
    }

    /// tsk847: what was logged after the effect's approval is the live
    /// consumer's — a backfill stops at `start_after_seq`, so the two
    /// never attempt one event at once — and a person's attempt under way
    /// (`started` by a retry or a backfill) isn't the live consumer's to
    /// call interrupted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_and_the_live_consumer_keep_off_each_others_events() {
        use crate::commands::effect::BACKFILL_PLAN;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        register(svc);
        let past = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        approve(svc).await;
        // The live consumer hasn't reached this one yet.
        let after = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        let plan = run_as_person(svc, BACKFILL_PLAN, json!({ "effect": "acme/mark-done" }))
            .await
            .unwrap();
        assert_eq!(
            (plan["planned"].clone(), plan["to_seq"].clone()),
            (json!(1), json!(past.seq)),
            "{plan}"
        );
        let plan = run_as_person(
            svc,
            BACKFILL_PLAN,
            json!({ "effect": "acme/mark-done", "to_seq": after.seq }),
        )
        .await
        .unwrap();
        assert_eq!(
            plan["planned"],
            json!(1),
            "a range past it is capped: {plan}"
        );

        // A person's attempt at `after` under way: the pump leaves it be.
        let key = EffectRunKey::first(
            "acme/mark-done",
            after.envelope.id.to_string(),
            after.seq,
            ReactionOrigin::Backfill,
        );
        svc.db
            .transaction(move |tx| oxplow_db::effect_run_store::claim_tx(tx, &key, "t"))
            .await
            .unwrap();
        EffectTriggers::new(Arc::downgrade(svc))
            .handle(&after)
            .await
            .unwrap();
        assert_eq!(
            rows(svc, "SELECT origin, state FROM v_effect_run").await,
            json!([["backfill", "started"]])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT count(*) FROM v_event WHERE type = 'effect.result'"
            )
            .await,
            json!([[0]])
        );
    }

    /// tsk848: a backfill's plan applies `where` to every candidate in its
    /// range, reading them a page at a time: events its `where` excludes
    /// filling the first pages never hide the matches after them.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_plan_reads_past_what_where_excludes() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        for _ in 0..5 {
            log(svc, transitioned(fx.task, TaskStatus::Blocked)).await;
        }
        let first = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        let last = log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        approve(svc).await;
        let (_, decl) = find_effect(svc, "acme/mark-done").unwrap();
        let vocabulary = svc.vocabulary.current();
        let planned = svc
            .db
            .read(move |tx| {
                crate::commands::effect::unreacted_tx(
                    tx,
                    &vocabulary,
                    &decl,
                    &crate::commands::effect::Range::default(),
                    1,
                    2,
                )
            })
            .await
            .unwrap();
        assert_eq!(planned.count, 2);
        assert_eq!(
            (planned.from_seq, planned.to_seq),
            (Some(first.seq), Some(last.seq))
        );
        assert_eq!(
            planned.first.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![first.seq],
            "only as many as asked for"
        );
    }

    /// tsk846: an event the effect's own run led to is never its trigger
    /// (the loop guard), so a backfill never plans it either: otherwise
    /// an effect that edits what it reacts to would find its own edits
    /// "unreacted" on every backfill, and never get past them.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_never_plans_the_effects_own_events() {
        use crate::commands::effect::BACKFILL_PLAN;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let bang = "  - id: bang\n    summary: Add a bang.\n    on: [work_item.edited]\n    input: \"SELECT title FROM v_work_item WHERE ref = :work_item\"\n    entry: bang.star\n";
        let script = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": x[\"event\"][\"payload\"][\"work_item\"], \"title\": x[\"rows\"][0][\"title\"] + \"!\"}}]}\n";
        extension(&svc.layout.project_dir, bang, &[("bang.star", script)]);
        register(svc);
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let edited = |source: &str| {
            Envelope::typed::<WorkItemEdited>(
                source,
                &serde_json::from_value(json!({ "work_item": r, "fields": ["title"] })).unwrap(),
            )
        };
        // A person's edit, and one the effect's own run made (its source).
        let theirs = log(svc, edited("human")).await;
        log(svc, edited("effect:acme/bang")).await;
        approve(svc).await;
        let plan = run_as_person(svc, BACKFILL_PLAN, json!({ "effect": "acme/bang" }))
            .await
            .unwrap();
        assert_eq!(plan["planned"], json!(1), "{plan}");
        assert_eq!(plan["from_seq"], json!(theirs.seq));
    }

    /// P9.D5: an effect never reacts to what was logged before its
    /// approval — until a person backfills it. A backfill reacts, once
    /// each and oldest first, to the matching events the effect never
    /// reacted to; a second one has nothing left.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_reacts_to_what_the_effect_never_saw() {
        use crate::commands::effect::{BACKFILL, BACKFILL_PLAN};
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        register(svc);
        let before = title(&fx).await;
        // Before its approval: three it would react to, one its `where`
        // excludes.
        let mut past = Vec::new();
        for _ in 0..3 {
            past.push(log(svc, transitioned(fx.task, TaskStatus::Done)).await);
        }
        log(svc, transitioned(fx.task, TaskStatus::Blocked)).await;
        approve(svc).await;
        let consumer = EffectTriggers::new(Arc::downgrade(svc));
        for ev in &past {
            consumer.handle(ev).await.unwrap();
        }
        assert_eq!(title(&fx).await, before, "the past isn't reacted to");
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_effect_run").await,
            json!([[0]])
        );

        // What a backfill would do: a read, an agent's too.
        let input = json!({ "effect": "acme/mark-done" });
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let plan = svc
            .commands
            .run(&agent, BACKFILL_PLAN, input.clone(), false)
            .await
            .unwrap()
            .result;
        assert_eq!(
            plan,
            json!({ "effect": "acme/mark-done", "planned": 3, "from_seq": past[0].seq, "to_seq": past[2].seq, "batch": crate::commands::effect::BACKFILL_BATCH })
        );
        // Running it is a person's, asked first.
        assert!(matches!(
            svc.commands
                .run(&agent, BACKFILL, input.clone(), true)
                .await,
            Err(CommandError::Denied { .. })
        ));
        match svc
            .commands
            .run(&oxplow_domain::Actor::Human, BACKFILL, input.clone(), false)
            .await
        {
            Err(CommandError::NeedsConfirmation { preview }) => {
                assert!(
                    preview.summary.contains("outside oxplow"),
                    "{}",
                    preview.summary
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(title(&fx).await, before, "asking ran nothing");

        let ran = run_as_person(svc, BACKFILL, input.clone()).await.unwrap();
        assert_eq!(
            ran,
            json!({ "effect": "acme/mark-done", "planned": 3, "ran": 3, "skipped": 0, "proposed": 0, "failed": 0, "remaining": 0 })
        );
        assert_eq!(title(&fx).await, format!("{before} (done) (done) (done)"));
        assert_eq!(
            rows(
                svc,
                "SELECT event_seq, attempt, origin, state FROM v_effect_run ORDER BY event_seq"
            )
            .await,
            json!(past
                .iter()
                .map(|e| json!([e.seq, 1, "backfill", "ok"]))
                .collect::<Vec<_>>())
        );
        assert_eq!(
            rows(svc, "SELECT DISTINCT json_extract(payload, '$.origin') FROM v_event WHERE type = 'effect.result'").await,
            json!([["backfill"]])
        );
        // Nothing left, and the live consumer doesn't react to them again.
        let again = run_as_person(svc, BACKFILL, input.clone()).await.unwrap();
        assert_eq!(
            (again["planned"].clone(), again["ran"].clone()),
            (json!(0), json!(0))
        );
        for ev in &past {
            consumer.handle(ev).await.unwrap();
        }
        assert_eq!(title(&fx).await, format!("{before} (done) (done) (done)"));
    }

    /// P9.D5: a backfill is bounded — by the range a person gives, and by
    /// a batch a run (the rest is `remaining`, for another run).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_keeps_to_its_range_and_its_batch() {
        use crate::commands::effect::{backfill, Range, BACKFILL, BACKFILL_PLAN};
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", MARK)]);
        register(svc);
        let mut past = Vec::new();
        for _ in 0..3 {
            past.push(log(svc, transitioned(fx.task, TaskStatus::Done)).await);
        }
        approve(svc).await;
        let planned = |input: Value| async move {
            run_as_person(svc, BACKFILL_PLAN, input).await.unwrap()["planned"].clone()
        };
        assert_eq!(
            planned(json!({ "effect": "acme/mark-done", "from_seq": past[1].seq })).await,
            json!(2)
        );
        assert_eq!(
            planned(json!({ "effect": "acme/mark-done", "to_seq": past[0].seq })).await,
            json!(1)
        );
        assert_eq!(
            planned(json!({ "effect": "acme/mark-done", "since": "2999-01-01T00:00:00Z" })).await,
            json!(0)
        );
        // One way to say where it starts.
        let both = run_as_person(
            svc,
            BACKFILL,
            json!({ "effect": "acme/mark-done", "from_seq": 1, "since": "2020-01-01T00:00:00Z" }),
        )
        .await
        .unwrap_err();
        assert!(both.to_string().contains("`from_seq` or `since`"), "{both}");

        // Two a run: the third waits for the next.
        let (ext, decl) = find_effect(svc, "acme/mark-done").unwrap();
        let first = backfill(svc, &ext, &decl, &Range::default(), 2)
            .await
            .unwrap();
        assert_eq!((first.planned, first.ran, first.remaining), (3, 2, 1));
        let second = backfill(svc, &ext, &decl, &Range::default(), 2)
            .await
            .unwrap();
        assert_eq!((second.planned, second.ran, second.remaining), (1, 1, 0));
    }

    /// P9.D5: a backfill runs the effect as it is now, under the same
    /// health: it needs the effect enabled and approved, and three
    /// failures in a row stop it — and disable the effect, as live.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_backfill_needs_approval_and_stops_when_the_effect_is_disabled() {
        use crate::commands::effect::BACKFILL;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let broken = "def transform(x):\n    return 1 // 0\n";
        extension(&svc.layout.project_dir, MARK_DONE, &[("mark.star", broken)]);
        register(svc);
        for _ in 0..5 {
            log(svc, transitioned(fx.task, TaskStatus::Done)).await;
        }
        let input = json!({ "effect": "acme/mark-done" });
        let unapproved = run_as_person(svc, BACKFILL, input.clone())
            .await
            .unwrap_err();
        assert!(unapproved.to_string().contains("approv"), "{unapproved}");
        approve(svc).await;
        let out = run_as_person(svc, BACKFILL, input.clone()).await.unwrap();
        assert_eq!(
            (
                out["planned"].clone(),
                out["failed"].clone(),
                out["remaining"].clone()
            ),
            (json!(5), json!(3), json!(2))
        );
        assert!(
            out["stopped"].as_str().unwrap().contains("disabled"),
            "{out}"
        );
        let disabled = run_as_person(svc, BACKFILL, input).await.unwrap_err();
        assert!(disabled.to_string().contains("is disabled"), "{disabled}");
    }
}
