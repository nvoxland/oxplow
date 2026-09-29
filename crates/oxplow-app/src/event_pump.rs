//! Delivery from the event log to its consumers (`.context/data-model.md`
//! "event_log"; `.context/target-architecture.md` §5.4).
//!
//! **At least once, forward only.** Each consumer has a checkpoint (the
//! last `seq` it handled). The pump reads `seq > checkpoint` in batches
//! and, for every event, runs the handler and advances the checkpoint in
//! **one transaction**, so a consumer's writes and its position can never
//! disagree. A handler that fails has its writes rolled back to a
//! savepoint, the event is parked in the dead-letter table with the
//! error, and the checkpoint still advances: one poison event never
//! stalls the pump, and nothing is skipped silently — a person retries
//! or discards the letter (`retry_dead_letter` / `discard_dead_letter`).
//!
//! **Async consumers** (P2.6.2, tsk454) do work that can't live in a
//! SQLite transaction — take a snapshot, call a service. The pump runs the
//! handler outside any transaction and checkpoints only after it returns,
//! so a crash mid-work re-delivers the event on the next run: an async
//! handler must be idempotent. A `Busy` failure leaves the checkpoint
//! where it is (the event is retried on the next run, and later events for
//! that consumer wait behind it); any other failure or a panic parks the
//! event as a dead letter and moves on, like the sync kind.
//!
//! [`EventPump::settle`] runs the pump now and waits (bounded) for it — a
//! caller whose answer depends on a consumer's effect (`complete_task`'s
//! file review needs the effort's end snapshot) settles instead of
//! re-implementing the effect inline. One run at a time: the loop and a
//! settle never deliver the same event concurrently.
//!
//! **Waking.** Producers call [`EventPump::wake`] after their transaction
//! commits (the in-memory `OxplowEvent` broadcast is the UI's wake-up;
//! this is the pump's). The loop also runs on boot and on a slow timer,
//! so a producer without a wake still delivers.

use std::sync::Arc;
use std::time::Duration;

use oxplow_db::event_log_store::{
    dead_letter_tx, read_after_tx, set_checkpoint_tx, set_dead_letter_state_tx, DeadLetter,
    SqliteEventLogStore,
};
use oxplow_db::Database;
use oxplow_domain::{DomainError, EventSchemaRegistry, StoredEvent};
use tokio::sync::Notify;

/// A consumer whose work runs outside the pump's transaction (see the
/// module docs). `handle` must be idempotent: it is re-run after a crash
/// between the work and the checkpoint.
#[async_trait::async_trait]
pub trait AsyncEventConsumer: Send + Sync {
    /// Stable name, keying the checkpoint and dead letters.
    fn name(&self) -> &'static str;
    fn handles(&self, event_type: &str) -> bool;
    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError>;
}

/// Something that reacts to events: a projection, an ingester, an effect.
pub trait EventConsumer: Send + Sync {
    /// Stable name; the checkpoint and dead letters are keyed by it, so
    /// renaming a consumer restarts it from the beginning of the log.
    fn name(&self) -> &'static str;

    /// Whether this consumer wants `event_type`. Events it doesn't want
    /// are skipped (the checkpoint still advances past them).
    fn handles(&self, event_type: &str) -> bool;

    /// Handle one event inside the pump's transaction. Any write made
    /// through `conn` commits with the checkpoint, or rolls back with the
    /// dead letter if this returns `Err`.
    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError>;
}

/// What one [`EventPump::run_once`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PumpReport {
    pub handled: usize,
    pub skipped: usize,
    pub dead_lettered: usize,
    /// Async deliveries that failed transiently and wait for the next run.
    pub deferred: usize,
}

impl PumpReport {
    fn add(&mut self, other: PumpReport) {
        self.handled += other.handled;
        self.skipped += other.skipped;
        self.dead_lettered += other.dead_lettered;
        self.deferred += other.deferred;
    }
}

pub struct EventPump {
    db: Database,
    log: SqliteEventLogStore,
    consumers: Vec<Arc<dyn EventConsumer>>,
    async_consumers: parking_lot::RwLock<Vec<Arc<AsyncSlot>>>,
    /// One sync-consumer run at a time.
    run_lock: tokio::sync::Mutex<()>,
    notify: Notify,
    batch: usize,
    /// Set by [`Self::spawn`], so a consumer registered later gets its loop.
    spawned: std::sync::OnceLock<std::sync::Weak<EventPump>>,
}

/// One async consumer and what keeps it independent of the others: its
/// own run lock (the loop, a settle and a manual run never deliver the same
/// event twice) and its own wake-up, so a slow consumer (a model call)
/// delays only itself.
struct AsyncSlot {
    consumer: Arc<dyn AsyncEventConsumer>,
    lock: tokio::sync::Mutex<()>,
    notify: Notify,
}

/// How long the loop sleeps between unprompted runs.
const IDLE_INTERVAL: Duration = Duration::from_secs(5);

impl EventPump {
    pub fn new(
        db: Database,
        log: SqliteEventLogStore,
        consumers: Vec<Arc<dyn EventConsumer>>,
    ) -> Self {
        Self {
            db,
            log,
            consumers,
            async_consumers: parking_lot::RwLock::new(Vec::new()),
            run_lock: tokio::sync::Mutex::new(()),
            notify: Notify::new(),
            batch: 64,
            spawned: std::sync::OnceLock::new(),
        }
    }

    /// Add an async consumer. Services and boot register them once the
    /// services they drive exist; one registered after [`Self::spawn`]
    /// gets its loop at once.
    pub fn register_async(&self, consumer: Arc<dyn AsyncEventConsumer>) {
        let slot = Arc::new(AsyncSlot {
            consumer,
            lock: tokio::sync::Mutex::new(()),
            notify: Notify::new(),
        });
        self.async_consumers.write().push(slot.clone());
        if let Some(pump) = self.spawned.get().and_then(std::sync::Weak::upgrade) {
            pump.spawn_slot(slot);
        }
    }

    fn async_slots(&self) -> Vec<Arc<AsyncSlot>> {
        self.async_consumers.read().clone()
    }

    /// Run the named async consumers now and wait up to `timeout` for them
    /// to catch up — for a caller whose answer needs their effect.
    /// `true` when everything outstanding for them was delivered (handled
    /// or parked); `false` on timeout, a run error, or a deferred delivery —
    /// the work then finishes on a later run. The run is spawned, so a
    /// timeout abandons the wait, never the work. Only the named consumers
    /// run: a slow one elsewhere (a model call) never holds a settle up.
    pub async fn settle(self: &Arc<Self>, consumers: &[&str], timeout: Duration) -> bool {
        let pump = self.clone();
        let slots: Vec<Arc<AsyncSlot>> = self
            .async_slots()
            .into_iter()
            .filter(|s| consumers.contains(&s.consumer.name()))
            .collect();
        let run = tokio::spawn(async move {
            let mut report = PumpReport::default();
            for slot in slots {
                report.add(pump.run_slot(&slot).await?);
            }
            Ok::<_, DomainError>(report)
        });
        match tokio::time::timeout(timeout, run).await {
            Ok(Ok(Ok(report))) => report.deferred == 0,
            Ok(Ok(Err(err))) => {
                tracing::warn!(error = %err, "event pump: settle run failed");
                false
            }
            Ok(Err(join)) => {
                tracing::warn!(error = %join, "event pump: settle run panicked");
                false
            }
            Err(_) => false,
        }
    }

    pub fn consumers(&self) -> &[Arc<dyn EventConsumer>] {
        &self.consumers
    }

    fn wake_others(&self, except: &AsyncSlot) {
        self.notify.notify_one();
        for slot in self.async_slots() {
            if !std::ptr::eq(slot.as_ref(), except) {
                slot.notify.notify_one();
            }
        }
    }

    /// Tell the loops there is something new. Cheap; call it after a
    /// producer's transaction commits.
    pub fn wake(&self) {
        self.notify.notify_one();
        for slot in self.async_slots() {
            slot.notify.notify_one();
        }
    }

    /// Run every consumer until each has caught up with the log.
    pub async fn run_once(&self) -> Result<PumpReport, DomainError> {
        let mut report = self.run_sync().await?;
        for slot in self.async_slots() {
            report.add(self.run_slot(&slot).await?);
        }
        Ok(report)
    }

    /// The sync consumers, one run at a time.
    async fn run_sync(&self) -> Result<PumpReport, DomainError> {
        let _one_run = self.run_lock.lock().await;
        let mut report = PumpReport::default();
        for consumer in &self.consumers {
            loop {
                let cp = self.log.checkpoint(consumer.name().to_string()).await?;
                let events = self.log.read_after(cp, self.batch).await?;
                if events.is_empty() {
                    break;
                }
                for event in events {
                    match self.deliver(consumer.clone(), Arc::new(event)).await? {
                        Delivery::Handled => report.handled += 1,
                        Delivery::Skipped => report.skipped += 1,
                        Delivery::DeadLettered => report.dead_lettered += 1,
                        Delivery::Deferred => report.deferred += 1,
                    }
                }
            }
        }
        Ok(report)
    }

    /// One async consumer until it has caught up (or deferred), under its
    /// own lock.
    async fn run_slot(&self, slot: &AsyncSlot) -> Result<PumpReport, DomainError> {
        let _one_run = slot.lock.lock().await;
        let consumer = &slot.consumer;
        let mut report = PumpReport::default();
        {
            'consumer: loop {
                let cp = self.log.checkpoint(consumer.name().to_string()).await?;
                let events = self.log.read_after(cp, self.batch).await?;
                if events.is_empty() {
                    break;
                }
                for event in events {
                    match self
                        .deliver_async(consumer.clone(), Arc::new(event))
                        .await?
                    {
                        Delivery::Handled => report.handled += 1,
                        Delivery::Skipped => report.skipped += 1,
                        Delivery::DeadLettered => report.dead_lettered += 1,
                        Delivery::Deferred => {
                            // Later events wait behind this one (per-consumer
                            // order); the next run tries it again.
                            report.deferred += 1;
                            break 'consumer;
                        }
                    }
                }
            }
        }
        Ok(report)
    }

    /// One event to one async consumer: the handler outside any
    /// transaction, then the checkpoint (and, on a permanent failure, the
    /// dead letter) in one transaction after it returns.
    async fn deliver_async(
        &self,
        consumer: Arc<dyn AsyncEventConsumer>,
        event: Arc<StoredEvent>,
    ) -> Result<Delivery, DomainError> {
        let name = consumer.name();
        let seq = event.seq;
        let outcome = if consumer.handles(&event.envelope.event_type) {
            match run_async_handler(self.log.schemas().clone(), consumer, event).await {
                Ok(()) => Delivery::Handled,
                Err(err) if err.is_retryable() => return Ok(Delivery::Deferred),
                Err(err) => {
                    let message = err.to_string();
                    self.db
                        .transaction(move |tx| {
                            dead_letter_tx(tx, name, seq, &message)?;
                            set_checkpoint_tx(tx, name, seq)
                        })
                        .await?;
                    return Ok(Delivery::DeadLettered);
                }
            }
        } else {
            Delivery::Skipped
        };
        self.db
            .transaction(move |tx| set_checkpoint_tx(tx, name, seq))
            .await?;
        Ok(outcome)
    }

    /// One event to one consumer: handler (under a savepoint) and
    /// checkpoint in one transaction; a failure becomes a dead letter in
    /// that same transaction.
    async fn deliver(
        &self,
        consumer: Arc<dyn EventConsumer>,
        event: Arc<StoredEvent>,
    ) -> Result<Delivery, DomainError> {
        let schemas = self.log.schemas().clone();
        self.db
            .transaction(move |tx| {
                let outcome = if consumer.handles(&event.envelope.event_type) {
                    match run_handler(&schemas, consumer.as_ref(), tx, &event) {
                        Ok(()) => Delivery::Handled,
                        // A lock blip isn't a poison event: fail the whole
                        // delivery so the transaction retries it, and if it
                        // stays busy the event waits (checkpoint unmoved).
                        Err(err) if err.is_retryable() => return Err(err),
                        Err(err) => {
                            dead_letter_tx(tx, consumer.name(), event.seq, &err.to_string())?;
                            Delivery::DeadLettered
                        }
                    }
                } else {
                    Delivery::Skipped
                };
                set_checkpoint_tx(tx, consumer.name(), event.seq)?;
                Ok(outcome)
            })
            .await
    }

    /// Run the parked event through its consumer again, now. On success
    /// the letter is `retried`; on failure it stays `pending` with the new
    /// error and one more attempt. Returns the letter's new state.
    pub async fn retry_dead_letter(&self, id: i64) -> Result<DeadLetter, DomainError> {
        let letter = self.find_letter(id).await?;
        if letter.state != "pending" {
            // Re-running a resolved or discarded event would run a possibly
            // non-idempotent handler a second time.
            return Err(DomainError::Invalid(format!(
                "dead letter {id} is `{}`, not `pending`; only a pending letter can be retried",
                letter.state
            )));
        }
        if let Some(slot) = self
            .async_slots()
            .into_iter()
            .find(|s| s.consumer.name() == letter.consumer)
        {
            // Under the consumer's lock: a retry never races its loop.
            let _one_run = slot.lock.lock().await;
            return self
                .retry_async_letter(slot.consumer.clone(), id, letter.event_seq)
                .await;
        }
        let consumer = self
            .consumers
            .iter()
            .find(|c| c.name() == letter.consumer)
            .cloned()
            .ok_or_else(|| {
                DomainError::Invalid(format!(
                    "no consumer named `{}` is registered; discard the letter instead",
                    letter.consumer
                ))
            })?;
        let seq = letter.event_seq;
        let schemas = self.log.schemas().clone();
        self.db
            .transaction(move |tx| {
                let Some(event) = event_by_seq_tx(tx, seq)? else {
                    return Err(DomainError::NotFound);
                };
                match run_handler(&schemas, consumer.as_ref(), tx, &event) {
                    Ok(()) => set_dead_letter_state_tx(tx, id, "retried"),
                    Err(err) => dead_letter_tx(tx, consumer.name(), seq, &err.to_string()),
                }
            })
            .await?;
        self.find_letter(id).await
    }

    async fn retry_async_letter(
        &self,
        consumer: Arc<dyn AsyncEventConsumer>,
        id: i64,
        seq: i64,
    ) -> Result<DeadLetter, DomainError> {
        let event = self
            .db
            .transaction(move |tx| event_by_seq_tx(tx, seq))
            .await?
            .ok_or(DomainError::NotFound)?;
        let name = consumer.name();
        let result = run_async_handler(self.log.schemas().clone(), consumer, Arc::new(event)).await;
        self.db
            .transaction(move |tx| match &result {
                Ok(()) => set_dead_letter_state_tx(tx, id, "retried"),
                Err(err) => dead_letter_tx(tx, name, seq, &err.to_string()),
            })
            .await?;
        self.find_letter(id).await
    }

    /// Give up on the parked event; it stays visible as `discarded`.
    pub async fn discard_dead_letter(&self, id: i64) -> Result<DeadLetter, DomainError> {
        self.log
            .set_dead_letter_state(id, "discarded".into())
            .await?;
        self.find_letter(id).await
    }

    pub async fn list_dead_letters(&self, all: bool) -> Result<Vec<DeadLetter>, DomainError> {
        self.log.list_dead_letters(all).await
    }

    async fn find_letter(&self, id: i64) -> Result<DeadLetter, DomainError> {
        self.list_dead_letters(true)
            .await?
            .into_iter()
            .find(|l| l.id == id)
            .ok_or(DomainError::NotFound)
    }

    /// One async consumer's own loop: catch up, then wait for a wake or
    /// the idle interval.
    fn spawn_slot(self: &Arc<Self>, slot: Arc<AsyncSlot>) {
        let pump = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(p) = pump.upgrade() else { return };
                let ran = p.run_slot(&slot).await;
                // A handler may have logged events (`effort.finished`) that
                // other consumers wait on.
                if matches!(&ran, Ok(report) if report.handled > 0) {
                    p.wake_others(&slot);
                }
                match ran {
                    Ok(report) if report.dead_lettered > 0 => tracing::warn!(
                        consumer = slot.consumer.name(),
                        dead_lettered = report.dead_lettered,
                        "event pump: events parked in the dead-letter queue"
                    ),
                    Ok(_) => {}
                    Err(err) => tracing::warn!(
                        consumer = slot.consumer.name(),
                        error = %err,
                        "event pump: run failed; retrying after the idle interval"
                    ),
                }
                drop(p);
                tokio::select! {
                    _ = slot.notify.notified() => {}
                    _ = tokio::time::sleep(IDLE_INTERVAL) => {}
                }
            }
        });
    }

    /// The delivery loops: one for the sync consumers and one per async
    /// consumer, each running, then waiting for a wake or the idle
    /// interval.
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let _ = self.spawned.set(Arc::downgrade(&self));
        for slot in self.async_slots() {
            self.spawn_slot(slot);
        }
        tokio::spawn(async move {
            loop {
                match self.run_sync().await {
                    Ok(report) if report.dead_lettered > 0 => {
                        tracing::warn!(
                            dead_lettered = report.dead_lettered,
                            handled = report.handled,
                            "event pump: events parked in the dead-letter queue"
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(error = %err, "event pump: run failed; retrying after the idle interval");
                    }
                }
                tokio::select! {
                    _ = self.notify.notified() => {}
                    _ = tokio::time::sleep(IDLE_INTERVAL) => {}
                }
            }
        })
    }
}

enum Delivery {
    Handled,
    Skipped,
    DeadLettered,
    /// Async only: a transient failure; the checkpoint didn't move.
    Deferred,
}

/// An async handler on its own task, so a panic is a failure the pump
/// parks rather than one that takes the loop down.
/// The event as a consumer reads it: carried to the newest registered
/// version of its type (P3.1), so a consumer is written against one shape
/// and rows logged at an older version still reach it. A row that can't
/// be upcast fails the delivery, which parks it as a dead letter.
fn at_latest(
    schemas: &EventSchemaRegistry,
    event: &StoredEvent,
) -> Result<StoredEvent, DomainError> {
    let env = &event.envelope;
    if schemas.latest(&env.event_type) == Some(env.v) {
        return Ok(event.clone());
    }
    let (v, payload) = schemas.upcast_to_latest(&env.event_type, env.v, env.payload.clone())?;
    let mut out = event.clone();
    out.envelope.v = v;
    out.envelope.payload = payload;
    Ok(out)
}

async fn run_async_handler(
    schemas: Arc<EventSchemaRegistry>,
    consumer: Arc<dyn AsyncEventConsumer>,
    event: Arc<StoredEvent>,
) -> Result<(), DomainError> {
    let event = at_latest(&schemas, &event)?;
    match tokio::spawn(async move { consumer.handle(&event).await }).await {
        Ok(result) => result,
        Err(join) => {
            let msg = match join.try_into_panic() {
                Ok(panic) => panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "handler panicked".to_string()),
                Err(join) => join.to_string(),
            };
            Err(DomainError::Invariant(format!("handler panicked: {msg}")))
        }
    }
}

/// The handler under a savepoint, so a failing handler's partial writes
/// roll back while the dead letter and checkpoint written afterwards in
/// the same transaction commit. A panicking handler is a failure too:
/// the pump must never stall on one.
fn run_handler(
    schemas: &EventSchemaRegistry,
    consumer: &dyn EventConsumer,
    conn: &rusqlite::Connection,
    event: &StoredEvent,
) -> Result<(), DomainError> {
    let event = &at_latest(schemas, event)?;
    conn.execute_batch("SAVEPOINT handler")
        .map_err(|e| DomainError::Storage(format!("savepoint: {e}")))?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consumer.handle(conn, event)
    }))
    .unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "handler panicked".to_string());
        Err(DomainError::Invariant(format!("handler panicked: {msg}")))
    });
    match result {
        Ok(()) => conn
            .execute_batch("RELEASE handler")
            .map_err(|e| DomainError::Storage(format!("release savepoint: {e}"))),
        Err(err) => {
            conn.execute_batch("ROLLBACK TO handler; RELEASE handler")
                .map_err(|e| DomainError::Storage(format!("rollback savepoint: {e}")))?;
            Err(err)
        }
    }
}

fn event_by_seq_tx(
    conn: &rusqlite::Connection,
    seq: i64,
) -> Result<Option<StoredEvent>, DomainError> {
    // `read_after_tx(seq - 1, 1)` is exactly the row at `seq` when it exists.
    let mut rows = read_after_tx(conn, seq - 1, 1)?;
    Ok(rows.pop().filter(|e| e.seq == seq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::event_log_store::SqliteEventLogStore;
    use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV1};
    use oxplow_domain::Envelope;
    use parking_lot::Mutex;
    use serde_json::{json, Value};

    /// Records the keys it saw into a table, and fails on a chosen key.
    struct Recorder {
        name: &'static str,
        poison: Mutex<Option<String>>,
    }

    impl Recorder {
        fn new(name: &'static str, poison: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                name,
                poison: Mutex::new(poison.map(str::to_string)),
            })
        }
    }

    impl EventConsumer for Recorder {
        fn name(&self) -> &'static str {
            self.name
        }
        fn handles(&self, event_type: &str) -> bool {
            event_type == "config.changed"
        }
        fn handle(
            &self,
            conn: &rusqlite::Connection,
            event: &StoredEvent,
        ) -> Result<(), DomainError> {
            let key = event.envelope.payload["key"]
                .as_str()
                .unwrap_or("")
                .to_string();
            conn.execute(
                "INSERT INTO seen (consumer, key) VALUES (?1, ?2)",
                rusqlite::params![self.name, key],
            )
            .map_err(|e| DomainError::Storage(e.to_string()))?;
            if self.poison.lock().as_deref() == Some(key.as_str()) {
                return Err(DomainError::Invariant(format!("cannot handle `{key}`")));
            }
            Ok(())
        }
    }

    struct Panicker;
    impl EventConsumer for Panicker {
        fn name(&self) -> &'static str {
            "panicker"
        }
        fn handles(&self, _: &str) -> bool {
            true
        }
        fn handle(&self, _: &rusqlite::Connection, _: &StoredEvent) -> Result<(), DomainError> {
            panic!("boom");
        }
    }

    async fn setup() -> (Database, SqliteEventLogStore) {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE seen (consumer TEXT NOT NULL, key TEXT NOT NULL)")
                .map_err(|e| DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
        let store = SqliteEventLogStore::new(db.clone(), Arc::new(EventSchemaRegistry::core()));
        (db, store)
    }

    fn pump(
        db: &Database,
        store: &SqliteEventLogStore,
        consumers: Vec<Arc<dyn EventConsumer>>,
    ) -> EventPump {
        EventPump::new(db.clone(), store.clone(), consumers)
    }

    fn config_changed(key: &str) -> Envelope {
        Envelope::typed::<ConfigChanged>(
            "test",
            &ConfigChangedV1 {
                key: key.into(),
                before: Value::Null,
                after: json!(1),
            },
        )
    }

    async fn seen(db: &Database, consumer: &str) -> Vec<String> {
        let consumer = consumer.to_string();
        db.transaction(move |tx| {
            let mut stmt = tx
                .prepare("SELECT key FROM seen WHERE consumer = ?1 ORDER BY rowid")
                .map_err(|e| DomainError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map([consumer.clone()], |r| r.get::<_, String>(0))
                .map_err(|e| DomainError::Storage(e.to_string()))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap()
    }

    /// An async consumer (P2.6.2, tsk454): work outside the pump's
    /// transaction, checkpoint after it completes. Scripted outcomes per
    /// call; `Ok` once the script runs out.
    struct AsyncRecorder {
        seen: Mutex<Vec<String>>,
        script: Mutex<std::collections::VecDeque<Result<(), DomainError>>>,
    }

    impl AsyncRecorder {
        fn new(script: Vec<Result<(), DomainError>>) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                script: Mutex::new(script.into()),
            })
        }
        fn seen(&self) -> Vec<String> {
            self.seen.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl AsyncEventConsumer for AsyncRecorder {
        fn name(&self) -> &'static str {
            "async_rec"
        }
        fn handles(&self, event_type: &str) -> bool {
            event_type == "config.changed"
        }
        async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
            let key = event.envelope.payload["key"]
                .as_str()
                .unwrap_or("")
                .to_string();
            self.seen.lock().push(key.clone());
            if key == "panic" {
                panic!("async boom");
            }
            self.script.lock().pop_front().unwrap_or(Ok(()))
        }
    }

    fn async_pump(
        db: &Database,
        store: &SqliteEventLogStore,
        consumer: Arc<AsyncRecorder>,
    ) -> EventPump {
        let pump = EventPump::new(db.clone(), store.clone(), vec![]);
        pump.register_async(consumer);
        pump
    }

    #[tokio::test]
    async fn an_async_consumer_checkpoints_after_its_work() {
        let (db, store) = setup().await;
        for key in ["a", "b"] {
            store.append(config_changed(key)).await.unwrap();
        }
        let rec = AsyncRecorder::new(vec![]);
        let pump = async_pump(&db, &store, rec.clone());
        let report = pump.run_once().await.unwrap();
        assert_eq!(report.handled, 2);
        assert_eq!(rec.seen(), vec!["a", "b"]);
        assert_eq!(store.checkpoint("async_rec".into()).await.unwrap(), 2);
        // A fresh pump over the same log (a restart) has nothing to redo.
        let again = AsyncRecorder::new(vec![]);
        let restarted = async_pump(&db, &store, again.clone());
        restarted.run_once().await.unwrap();
        assert!(again.seen().is_empty());
    }

    #[tokio::test]
    async fn an_async_transient_failure_waits_and_retries_instead_of_parking() {
        let (db, store) = setup().await;
        for key in ["a", "b"] {
            store.append(config_changed(key)).await.unwrap();
        }
        let rec = AsyncRecorder::new(vec![Err(DomainError::Busy("locked".into()))]);
        let pump = async_pump(&db, &store, rec.clone());
        let first = pump.run_once().await.unwrap();
        assert_eq!(
            (first.handled, first.deferred, first.dead_lettered),
            (0, 1, 0)
        );
        assert_eq!(store.checkpoint("async_rec".into()).await.unwrap(), 0);
        assert!(pump.list_dead_letters(false).await.unwrap().is_empty());
        // The next run delivers it again, then moves on.
        let second = pump.run_once().await.unwrap();
        assert_eq!(second.handled, 2);
        assert_eq!(rec.seen(), vec!["a", "a", "b"]);
        assert_eq!(store.checkpoint("async_rec".into()).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn an_async_permanent_failure_or_panic_is_parked_and_retryable() {
        let (db, store) = setup().await;
        for key in ["bad", "panic", "c"] {
            store.append(config_changed(key)).await.unwrap();
        }
        let rec = AsyncRecorder::new(vec![Err(DomainError::Invariant("nope".into()))]);
        let pump = async_pump(&db, &store, rec.clone());
        let report = pump.run_once().await.unwrap();
        assert_eq!((report.handled, report.dead_lettered), (1, 2));
        assert_eq!(store.checkpoint("async_rec".into()).await.unwrap(), 3);
        let letters = pump.list_dead_letters(false).await.unwrap();
        assert_eq!(letters.len(), 2);
        assert!(letters[0].error.contains("nope"), "{}", letters[0].error);
        assert!(
            letters[1].error.contains("async boom"),
            "{}",
            letters[1].error
        );
        // Retrying the first runs the async handler again; it succeeds now.
        let retried = pump.retry_dead_letter(letters[0].id).await.unwrap();
        assert_eq!(retried.state, "retried");
        assert_eq!(rec.seen(), vec!["bad", "panic", "c", "bad"]);
    }

    #[tokio::test]
    async fn settle_delivers_what_is_outstanding_before_returning() {
        let (db, store) = setup().await;
        let rec = AsyncRecorder::new(vec![]);
        let pump = Arc::new(async_pump(&db, &store, rec.clone()));
        store.append(config_changed("a")).await.unwrap();
        assert!(
            pump.settle(&["async_rec"], std::time::Duration::from_secs(5))
                .await
        );
        assert_eq!(rec.seen(), vec!["a"]);
    }

    #[tokio::test]
    async fn a_poison_event_is_dead_lettered_and_the_pump_moves_on() {
        let (db, store) = setup().await;
        for key in ["a", "bad", "c"] {
            store.append(config_changed(key)).await.unwrap();
        }
        let recorder = Recorder::new("rec", Some("bad"));
        let pump = pump(&db, &store, vec![recorder.clone()]);
        let report = pump.run_once().await.unwrap();
        assert_eq!(
            report,
            PumpReport {
                handled: 2,
                skipped: 0,
                dead_lettered: 1,
                deferred: 0,
            }
        );
        // The failing handler's own write rolled back; the others stuck.
        assert_eq!(seen(&db, "rec").await, vec!["a", "c"]);
        let letters = pump.list_dead_letters(false).await.unwrap();
        assert_eq!(letters.len(), 1);
        assert_eq!(letters[0].consumer, "rec");
        assert_eq!(letters[0].event_seq, 2);
        assert!(letters[0].error.contains("cannot handle `bad`"));
        // The checkpoint is past the poison event.
        assert_eq!(store.checkpoint("rec".into()).await.unwrap(), 3);
        // Nothing left to do.
        assert_eq!(pump.run_once().await.unwrap(), PumpReport::default());
    }

    /// Records the schema version each event was delivered at.
    struct VersionRecorder;
    impl EventConsumer for VersionRecorder {
        fn name(&self) -> &'static str {
            "versions"
        }
        fn handles(&self, event_type: &str) -> bool {
            event_type == "agent.turn.ended"
        }
        fn handle(
            &self,
            conn: &rusqlite::Connection,
            event: &StoredEvent,
        ) -> Result<(), DomainError> {
            conn.execute(
                "INSERT INTO seen (consumer, key) VALUES ('versions', ?1)",
                [format!(
                    "v{} transcript={}",
                    event.envelope.v,
                    event.envelope.payload.get("transcript_path").is_some()
                )],
            )
            .map_err(|e| DomainError::Storage(e.to_string()))?;
            Ok(())
        }
    }

    /// P3.1 (tsk471): a consumer reads only the newest shape of a type. A
    /// row written at `agent.turn.ended@1` is delivered upcast to v2.
    #[tokio::test]
    async fn consumers_receive_the_newest_version_of_a_type() {
        let (db, store) = setup().await;
        let v1 = Envelope::new(
            "agent.turn.ended",
            1,
            "test",
            json!({ "turn": "turn:trn1", "thread": "thread:thr1", "outcome": "completed" }),
        )
        .unwrap();
        store.append(v1).await.unwrap();
        pump(&db, &store, vec![Arc::new(VersionRecorder)])
            .run_once()
            .await
            .unwrap();
        assert_eq!(seen(&db, "versions").await, vec!["v2 transcript=false"]);
    }

    #[tokio::test]
    async fn a_new_pump_over_the_same_db_resumes_from_the_checkpoint() {
        let (db, store) = setup().await;
        store.append(config_changed("a")).await.unwrap();
        pump(&db, &store, vec![Recorder::new("rec", None)])
            .run_once()
            .await
            .unwrap();
        store.append(config_changed("b")).await.unwrap();
        // A restart: a fresh pump, same consumer name, same database.
        let report = pump(&db, &store, vec![Recorder::new("rec", None)])
            .run_once()
            .await
            .unwrap();
        assert_eq!(report.handled, 1);
        assert_eq!(seen(&db, "rec").await, vec!["a", "b"]);
        // A consumer under a new name starts from the beginning.
        pump(&db, &store, vec![Recorder::new("other", None)])
            .run_once()
            .await
            .unwrap();
        assert_eq!(seen(&db, "other").await, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn events_a_consumer_does_not_handle_are_skipped_but_checkpointed() {
        let (db, store) = setup().await;
        store.append(config_changed("a")).await.unwrap();
        let mut other = config_changed("x");
        other.event_type = "effect.result".into();
        other.payload = json!({"effect": "notify.desktop", "ok": true, "detail": null});
        store.append(other).await.unwrap();
        let pump = pump(&db, &store, vec![Recorder::new("rec", None)]);
        let report = pump.run_once().await.unwrap();
        assert_eq!((report.handled, report.skipped), (1, 1));
        assert_eq!(store.checkpoint("rec".into()).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn a_panicking_handler_is_a_dead_letter_not_a_stall() {
        let (db, store) = setup().await;
        store.append(config_changed("a")).await.unwrap();
        let pump = pump(&db, &store, vec![Arc::new(Panicker)]);
        let report = pump.run_once().await.unwrap();
        assert_eq!(report.dead_lettered, 1);
        let letters = pump.list_dead_letters(false).await.unwrap();
        assert!(letters[0].error.contains("boom"), "{}", letters[0].error);
        assert_eq!(store.checkpoint("panicker".into()).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn retry_reruns_the_consumer_and_discard_parks_for_good() {
        let (db, store) = setup().await;
        store.append(config_changed("bad")).await.unwrap();
        store.append(config_changed("worse")).await.unwrap();
        let recorder = Recorder::new("rec", Some("bad"));
        let pump = pump(&db, &store, vec![recorder.clone()]);
        pump.run_once().await.unwrap();
        let letters = pump.list_dead_letters(false).await.unwrap();
        assert_eq!(letters.len(), 1);
        let id = letters[0].id;
        // Still broken: the retry fails, attempts climb, still pending.
        let again = pump.retry_dead_letter(id).await.unwrap();
        assert_eq!((again.state.as_str(), again.attempts), ("pending", 2));
        assert!(seen(&db, "rec").await.iter().all(|k| k != "bad"));
        // Fixed (the consumer no longer chokes): the retry lands the
        // handler's write and resolves the letter.
        *recorder.poison.lock() = None;
        let fixed = pump.retry_dead_letter(id).await.unwrap();
        assert_eq!(fixed.state, "retried");
        // `worse` landed in the first run; `bad` only on the retry.
        assert_eq!(seen(&db, "rec").await, vec!["worse", "bad"]);
        assert!(pump.list_dead_letters(false).await.unwrap().is_empty());
        // Discard is explicit and visible.
        *recorder.poison.lock() = Some("bad".into());
        store.append(config_changed("bad")).await.unwrap();
        pump.run_once().await.unwrap();
        let id2 = pump.list_dead_letters(false).await.unwrap()[0].id;
        assert_eq!(
            pump.discard_dead_letter(id2).await.unwrap().state,
            "discarded"
        );
        assert!(pump.list_dead_letters(false).await.unwrap().is_empty());
        assert_eq!(pump.list_dead_letters(true).await.unwrap().len(), 2);
        assert!(matches!(
            pump.retry_dead_letter(9999).await.unwrap_err(),
            DomainError::NotFound
        ));
        // Only a pending letter can be retried: re-running a resolved or
        // discarded one would run a possibly non-idempotent handler again.
        *recorder.poison.lock() = None;
        let err = pump.retry_dead_letter(id).await.unwrap_err();
        assert!(err.to_string().contains("retried"), "{err}");
        let err = pump.retry_dead_letter(id2).await.unwrap_err();
        assert!(err.to_string().contains("discarded"), "{err}");
    }

    /// Fails with a retryable `Busy` a set number of times, then handles.
    struct Flaky {
        busy_left: parking_lot::Mutex<u32>,
    }
    impl EventConsumer for Flaky {
        fn name(&self) -> &'static str {
            "flaky"
        }
        fn handles(&self, _event_type: &str) -> bool {
            true
        }
        fn handle(
            &self,
            _conn: &rusqlite::Connection,
            _event: &StoredEvent,
        ) -> Result<(), DomainError> {
            let mut left = self.busy_left.lock();
            if *left > 0 {
                *left -= 1;
                return Err(DomainError::Busy("database is locked".into()));
            }
            Ok(())
        }
    }

    /// A lock blip inside a handler is not a poison event: the delivery
    /// is retried, and if the database stays busy the event stays
    /// undelivered (checkpoint unmoved) for the next run — never parked.
    #[tokio::test]
    async fn a_transient_busy_error_is_retried_not_dead_lettered() {
        let (db, store) = setup().await;
        store.append(config_changed("a")).await.unwrap();
        let flaky = Arc::new(Flaky {
            busy_left: parking_lot::Mutex::new(1),
        });
        let pump = pump(&db, &store, vec![flaky.clone()]);
        let report = pump.run_once().await.unwrap();
        assert_eq!(report.handled, 1);
        assert!(pump.list_dead_letters(true).await.unwrap().is_empty());
        // Busy for longer than the transaction's own retries: the run
        // errors, nothing is parked, and the checkpoint stays put.
        store.append(config_changed("b")).await.unwrap();
        *flaky.busy_left.lock() = 10;
        assert!(pump.run_once().await.is_err());
        assert!(pump.list_dead_letters(true).await.unwrap().is_empty());
        assert_eq!(store.checkpoint("flaky".into()).await.unwrap(), 1);
        *flaky.busy_left.lock() = 0;
        assert_eq!(pump.run_once().await.unwrap().handled, 1);
        assert_eq!(store.checkpoint("flaky".into()).await.unwrap(), 2);
    }
}
