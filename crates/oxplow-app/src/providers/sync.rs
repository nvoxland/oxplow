//! Reading a provider's collectors (P7.A3, `.context/providers.md`): how
//! another tracker's items reach `v_work_item` when oxplow didn't write
//! them.
//!
//! [`Instance::read`] runs one collector's `read`, resuming from the
//! checkpoint it last stored. What the provider streams about the request
//! (`Peer::start_streaming`) is checked as it arrives — a `$/record` must
//! be the collector's entity and, for a work-items provider, a
//! `WorkItemRecord` of its own item — and kept as `work_item.recorded@1`
//! envelopes until the next `$/state`, which commits them **with** the
//! checkpoint in one transaction (`provider_collector_state`). So a read
//! that fails midway keeps exactly what its last checkpoint covered. The
//! read's result must count what was streamed; a read that sends nothing
//! for `call_timeout` is cancelled. A failure counts toward the instance's
//! health like a failed call.
//!
//! `provider.sync { instance, collector? }` is the one way to read: a
//! person's "Sync now", an agent's, the schedule ([`ProviderRegistry::sync_due`],
//! as the system, every instance's `syncMinutes`) and the read an
//! instance gets once it starts.

use std::sync::Arc;
use std::time::Instant;

use oxplow_domain::events::schema::{WorkItemRecorded, WorkItemRecordedV1};
use oxplow_domain::work_items::{provider_of, WorkItemRecord};
use oxplow_domain::{
    Actor, Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Envelope, Invokers,
    Lifecycle,
};
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::{method, CollectorDecl, ReadParams, ReadResult};
use oxplow_provider_protocol::{Incoming, ProtocolError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::registry::{Instance, ProviderRegistry, Refusal};
use super::spec;
use crate::commands::{Command, Handler, HandlerOutput, Invocation};

/// The command that reads a provider's collectors.
pub const SYNC: &str = "provider.sync";

/// What one collector's read delivered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct CollectorRead {
    pub collector: String,
    pub records: u64,
}

/// Why a read stopped, and whether it counts against the instance.
struct ReadFailure {
    error: CommandError,
    counts: bool,
    /// Whether, and how, it may be tried again.
    retry: Option<Retry>,
}

/// How a stopped read may be tried again.
enum Retry {
    /// The provider's service said to wait: its message and how long.
    RateLimited(String, Option<u64>),
    /// On a renewed sign-in.
    Refused(Refusal),
}

impl ReadFailure {
    fn counted(message: String) -> Self {
        Self {
            error: CommandError::Failed { message },
            counts: true,
            retry: None,
        }
    }

    fn uncounted(error: CommandError) -> Self {
        Self {
            error,
            counts: false,
            retry: None,
        }
    }
}

fn now() -> String {
    oxplow_domain::Timestamp::now().to_string()
}

impl Instance {
    /// Run `collector`'s `read` from its last checkpoint, committing each
    /// checkpoint with the records it covers; record the outcome.
    pub async fn read(
        &self,
        actor: &Actor,
        collector: &str,
    ) -> Result<CollectorRead, CommandError> {
        let decl = self
            .declared
            .collectors
            .iter()
            .find(|c| c.name == collector)
            .cloned()
            .ok_or_else(|| CommandError::Invalid {
                field: Some("/collector".into()),
                message: format!(
                    "provider `{}` declares no collector `{collector}`; it declares: {}",
                    self.name,
                    self.declared
                        .collectors
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })?;
        // One read of a collector at a time (tsk715): two from the same
        // checkpoint would record its items twice and race the cursor.
        let lock = self
            .reading
            .lock()
            .entry(collector.to_string())
            .or_default()
            .clone();
        let _reading = lock.lock().await;
        let store = oxplow_db::SqliteProviderCollectorStore::new(self.deps.db.clone());
        let started = Instant::now();
        let mut retried = false;
        let mut reauthorized = false;
        let outcome = loop {
            // From the last checkpoint — a retry resumes where the
            // rate-limited read's last batch landed.
            let resume = store
                .get(&self.name, collector)
                .await?
                .and_then(|s| s.state);
            // When the read began, for "renewed since": taken once it has
            // its process — one this read started began after it, which
            // isn't a renewal under it (tsk907).
            let mut called = Instant::now();
            let outcome = match self.connection().await {
                Ok((peer, handle)) => {
                    called = Instant::now();
                    let mut outcome = self.stream(actor, &peer, handle, &decl, resume).await;
                    // Ended under it to renew the sign-in for another
                    // caller: as refused, so it tries again on the renewal.
                    if let Err(failure) = &mut outcome {
                        if failure.retry.is_none() && peer.is_closed() && self.renewed_since(called)
                        {
                            failure.retry = Some(Retry::Refused(Refusal::RenewedUnder));
                        }
                    }
                    self.forget_if_closed(&peer).await;
                    outcome
                }
                // A start that failed was counted as it failed.
                Err(error) => Err(ReadFailure::uncounted(error)),
            };
            match &outcome {
                Err(ReadFailure {
                    retry: Some(Retry::RateLimited(message, retry_after_ms)),
                    ..
                }) => match self.rate_limited(message, *retry_after_ms, retried).await {
                    None => {
                        retried = true;
                        continue;
                    }
                    Some(error) => break Err(ReadFailure::uncounted(error)),
                },
                // Its service refused its credentials: once, on renewed ones.
                Err(ReadFailure {
                    retry: Some(Retry::Refused(refusal)),
                    ..
                }) if !reauthorized && self.reauthorize(called, refusal).await => {
                    reauthorized = true;
                    continue;
                }
                _ => {}
            }
            break outcome;
        };
        if let Some(r) = self.registry.upgrade() {
            r.set_activity(self, None).await;
        }
        let registry = self.registry.upgrade();
        let (finished, result) = match outcome {
            Ok(records) => {
                if let Some(r) = &registry {
                    r.call_succeeded(self, started.elapsed()).await;
                }
                (
                    None,
                    Ok(CollectorRead {
                        collector: collector.to_string(),
                        records,
                    }),
                )
            }
            Err(ReadFailure { error, counts, .. }) => {
                if let (true, Some(r)) = (counts, &registry) {
                    r.call_failed(self, error.to_string()).await;
                }
                (Some(error.to_string()), Err(error))
            }
        };
        if let Err(e) = store.finish(&self.name, collector, finished, now()).await {
            tracing::warn!(instance = %self.name, collector, error = %e, "recording a read's outcome failed");
        }
        result
    }

    /// The read itself: how many records it delivered.
    async fn stream(
        &self,
        actor: &Actor,
        peer: &oxplow_provider_protocol::Peer,
        handle: oxplow_provider_protocol::model::Handle,
        decl: &CollectorDecl,
        resume: Option<Value>,
    ) -> Result<u64, ReadFailure> {
        let params = serde_json::to_value(ReadParams {
            handle,
            collector: decl.name.clone(),
            state: resume,
        })
        .expect("read params serialize");
        let (call, mut stream) = peer
            .start_streaming(method::READ, params)
            .await
            .map_err(|e| ReadFailure::counted(format!("provider `{}`: read: {e}", self.name)))?;
        let id = call.id;
        let mut batch: Vec<Envelope> = Vec::new();
        let mut streamed: u64 = 0;
        let failed = |message: String| async move {
            let _ = peer.cancel(id).await;
            ReadFailure::counted(format!("provider `{}`: {message}", self.name))
        };
        loop {
            let next = match tokio::time::timeout(self.deps.call_timeout, stream.recv()).await {
                Ok(next) => next,
                Err(_) => {
                    return Err(failed(format!(
                        "its read sent nothing for {}s",
                        self.deps.call_timeout.as_secs_f32()
                    ))
                    .await)
                }
            };
            let Some(Incoming::Notification { method, params }) = next else {
                break;
            };
            match method.as_str() {
                notify::RECORD => match self.record(actor, decl, params) {
                    Ok(envelope) => {
                        batch.push(envelope);
                        streamed += 1;
                    }
                    Err(message) => return Err(failed(message).await),
                },
                notify::STATE => {
                    let state: notify::State = match serde_json::from_value(params) {
                        Ok(state) => state,
                        Err(e) => return Err(failed(format!("a `$/state`: {e}")).await),
                    };
                    if let Err(e) = self
                        .commit(&decl.name, std::mem::take(&mut batch), Some(state.state))
                        .await
                    {
                        return Err(failed(e.to_string()).await);
                    }
                }
                // What it's doing, shown while the read runs (P7.A4).
                notify::PROGRESS => {
                    if let (Ok(p), Some(r)) = (
                        serde_json::from_value::<notify::Progress>(params),
                        self.registry.upgrade(),
                    ) {
                        r.set_activity(self, Some(activity_of(&decl.name, &p)))
                            .await;
                    }
                }
                _ => {}
            }
        }
        let result: ReadResult = match call.reply().await {
            Ok(v) => serde_json::from_value(v)
                .map_err(|e| ReadFailure::counted(format!("its read result: {e}")))?,
            Err(ProtocolError::RateLimited {
                message,
                retry_after_ms,
            }) => {
                return Err(ReadFailure {
                    error: CommandError::Failed {
                        message: message.clone(),
                    },
                    counts: false,
                    retry: Some(Retry::RateLimited(message, retry_after_ms)),
                });
            }
            Err(e) => {
                let counts = !matches!(
                    e,
                    ProtocolError::InvalidInput { .. } | ProtocolError::Cancelled
                );
                let retry = match &e {
                    ProtocolError::Auth { credential, .. } => {
                        Some(Retry::Refused(Refusal::Auth(credential.clone())))
                    }
                    _ => None,
                };
                return Err(ReadFailure {
                    error: self.command_error(e),
                    counts,
                    retry,
                });
            }
        };
        if result.records != streamed {
            return Err(ReadFailure::counted(format!(
                "provider `{}`: its read reported {} records but streamed {streamed}",
                self.name, result.records
            )));
        }
        // Records after its last checkpoint: they landed, so keep them.
        if !batch.is_empty() {
            self.commit(&decl.name, batch, None)
                .await
                .map_err(|e| ReadFailure::counted(e.to_string()))?;
        }
        Ok(streamed)
    }

    /// A streamed `$/record` as the envelope it is logged as: the
    /// collector's entity, and — for a work-items provider — a record of
    /// one of its own items.
    fn record(
        &self,
        actor: &Actor,
        decl: &CollectorDecl,
        params: Value,
    ) -> Result<Envelope, String> {
        let record: notify::Record =
            serde_json::from_value(params).map_err(|e| format!("a `$/record`: {e}"))?;
        if record.entity != decl.entity {
            return Err(format!(
                "collector `{}` streamed a `{}` record; it declares `{}`",
                decl.name, record.entity, decl.entity
            ));
        }
        if self.spec.capability != spec::WORK_ITEMS || decl.entity != "work_item" {
            return Err(format!(
                "collector `{}`: a {} provider's records are `work_item`s",
                decl.name, self.spec.capability
            ));
        }
        let item: WorkItemRecord = serde_json::from_value(record.row).map_err(|e| {
            format!(
                "collector `{}` streamed a record that isn't a work item: {e}",
                decl.name
            )
        })?;
        if provider_of(&item.item_ref).ok() != Some(self.id.as_str()) {
            return Err(format!(
                "collector `{}` streamed `{}`, which isn't one of its items",
                decl.name, item.item_ref
            ));
        }
        let subject = item.item_ref.clone();
        Ok(
            Envelope::typed::<WorkItemRecorded>(actor.source(), &WorkItemRecordedV1 { item })
                .with_subject([subject]),
        )
    }

    /// Append `batch` and the checkpoint covering it, in one transaction —
    /// each record only when it changes its item: one equal to the item's
    /// last `work_item.recorded` (a write's, or an earlier read's) restates
    /// nothing, and logging it would echo a write back to whatever reacted
    /// to it (an effect writing the item, tsk799). The checkpoint counts
    /// what was read.
    async fn commit(
        &self,
        collector: &str,
        batch: Vec<Envelope>,
        state: Option<Value>,
    ) -> Result<(), oxplow_domain::DomainError> {
        let (instance, collector) = (self.name.clone(), collector.to_string());
        let vocabulary = self.deps.log.vocabulary().clone();
        let at = now();
        self.deps
            .db
            .transaction(move |tx| {
                for envelope in &batch {
                    if restates(tx, envelope)? {
                        continue;
                    }
                    oxplow_db::event_log_store::append_tx(tx, &vocabulary.current(), envelope)?;
                }
                oxplow_db::provider_collector_store::checkpoint_tx(
                    tx,
                    &instance,
                    &collector,
                    state.as_ref(),
                    batch.len() as i64,
                    &at,
                )
            })
            .await
    }
}

/// Whether `record` (a `work_item.recorded`) says what its item's last
/// record already said.
fn restates(
    tx: &rusqlite::Connection,
    record: &Envelope,
) -> Result<bool, oxplow_domain::DomainError> {
    use rusqlite::OptionalExtension;
    let Some(item) = record.payload["item"]["ref"].as_str() else {
        return Ok(false);
    };
    let last: Option<String> = tx
        .query_row(
            "SELECT payload FROM event_log
              WHERE type = 'work_item.recorded'
                AND json_extract(payload, '$.item.ref') = ?1
              ORDER BY seq DESC LIMIT 1",
            [item],
            |r| r.get(0),
        )
        .optional()
        .map_err(oxplow_db::map_sql_err)?;
    Ok(last
        .and_then(|p| serde_json::from_str::<Value>(&p).ok())
        .is_some_and(|p| p == record.payload))
}

/// A `$/progress` as a line on the instance: `issues: page 2 (40%)`.
fn activity_of(collector: &str, p: &notify::Progress) -> String {
    let percent = p.fraction.map(|f| format!(" ({:.0}%)", f * 100.0));
    match (&p.message, percent) {
        (Some(m), Some(pc)) => format!("{collector}: {m}{pc}"),
        (Some(m), None) => format!("{collector}: {m}"),
        (None, Some(pc)) => format!("{collector}: reading{pc}"),
        (None, None) => format!("{collector}: reading"),
    }
}

impl ProviderRegistry {
    /// Read every collector of `instance` that is due on its schedule
    /// (`syncMinutes`), as the system, through `provider.sync` so each is
    /// audited. Called on a timer; returns how many reads it started.
    pub async fn sync_due(&self) -> usize {
        let Some(bus) = self.bus.upgrade() else {
            return 0;
        };
        let configured = self.instances_config();
        let running: Vec<Arc<Instance>> = self.running.lock().await.values().cloned().collect();
        let store = oxplow_db::SqliteProviderCollectorStore::new(self.deps.db.clone());
        let now = oxplow_domain::Timestamp::now();
        let mut started = 0;
        let mut plans = Vec::new();
        for instance in running {
            // Its service asked it to wait: the schedule does (and its
            // next due time stays what it was).
            if self.is_rate_limited(&instance.name) {
                continue;
            }
            let Some(every) = configured.get(&instance.name).and_then(|c| c.sync_every()) else {
                plans.push((super::registry::plugin_key(&instance.name), None));
                continue;
            };
            let every_ms = every.as_millis() as i64;
            let mut next_due: Option<i64> = None;
            for c in &instance.declared.collectors {
                let last = store
                    .get(&instance.name, &c.name)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|s| s.last_read_at)
                    .and_then(|t| oxplow_domain::Timestamp::parse(&t).ok());
                let planned = crate::plugin_health::next_due_ms(
                    last.as_ref().map(|t| t.unix_ms()),
                    every_ms,
                    now.unix_ms(),
                );
                next_due = Some(next_due.map_or(planned, |d| d.min(planned)));
                let due = last.is_none_or(|t| {
                    now.unix_ms().saturating_sub(t.unix_ms()) >= every.as_millis() as i64
                });
                if !due {
                    continue;
                }
                started += 1;
                if let Err(e) = bus
                    .run(
                        &Actor::System,
                        SYNC,
                        json!({ "instance": instance.name, "collector": c.name }),
                        false,
                    )
                    .await
                {
                    tracing::warn!(instance = %instance.name, collector = %c.name, error = %e, "a scheduled sync failed");
                }
            }
            plans.push((super::registry::plugin_key(&instance.name), next_due));
        }
        // When each instance is next due, so one that misses its read
        // reads unfresh.
        if let Err(e) = self.plugins.set_next_due(plans).await {
            tracing::warn!(error = ?e, "recording the instances' next due times failed");
        }
        started
    }

    /// Read every collector of a just-started `instance` once, as the
    /// system, so its items are there before the first scheduled read.
    pub(super) async fn sync_started(&self, instance: &str) {
        let Some(bus) = self.bus.upgrade() else {
            return;
        };
        if let Err(e) = bus
            .run(&Actor::System, SYNC, json!({ "instance": instance }), false)
            .await
        {
            tracing::warn!(instance, error = %e, "the first sync of a started provider failed");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SyncInput {
    /// The instance, `<extension>/<instance id>` (a provider's default
    /// instance has the provider's id).
    instance: String,
    /// One collector; absent, every collector it declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collector: Option<String>,
}

/// `provider.sync { instance, collector? }`: read a running instance's
/// collectors now. Any invoker; it changes oxplow's records (the items it
/// restates), not the worktree, so any thread may run it.
pub fn sync_command(registry: &Arc<ProviderRegistry>) -> Command {
    let registry = Arc::downgrade(registry);
    Command::new(
        CommandSpec {
            name: SYNC.into(),
            summary: "Read a provider instance's collectors now (all, or one), restating the \
                      items it tracks (runs the provider process, a system the bus doesn't own)."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(SyncInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::External,
            effect: CommandEffect::Record,
            needs: Vec::new(),
        },
        Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
            let registry = registry.clone();
            Box::pin(async move {
                let SyncInput {
                    instance,
                    collector,
                } = serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
                let registry = registry.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "the provider registry is gone".into(),
                })?;
                let running =
                    registry
                        .get(&instance)
                        .await
                        .ok_or_else(|| CommandError::Invalid {
                            field: Some("/instance".into()),
                            message: format!("provider instance `{instance}` isn't running"),
                        })?;
                let collectors: Vec<String> = match collector {
                    Some(c) => vec![c],
                    None => running
                        .declared
                        .collectors
                        .iter()
                        .map(|c| c.name.clone())
                        .collect(),
                };
                let mut reads = Vec::new();
                for c in collectors {
                    reads.push(running.read(&actor, &c).await?);
                }
                Ok(HandlerOutput {
                    result: json!({ "reads": reads }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("provider.sync is a valid command")
}

/// Run [`ProviderRegistry::sync_due`] every minute for the app's life.
pub fn spawn_sync_scheduler(state: Arc<crate::Services>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            // Another project's oxplow may have changed this machine's
            // global instances.
            state.providers.reconcile_if_global_changed().await;
            state.providers.sync_due().await;
        }
    });
}
