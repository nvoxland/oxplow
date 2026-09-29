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
use oxplow_domain::{DomainError, StoredEvent};
use tokio::sync::Notify;

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
}

pub struct EventPump {
    db: Database,
    log: SqliteEventLogStore,
    consumers: Vec<Arc<dyn EventConsumer>>,
    notify: Notify,
    batch: usize,
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
            notify: Notify::new(),
            batch: 64,
        }
    }

    pub fn consumers(&self) -> &[Arc<dyn EventConsumer>] {
        &self.consumers
    }

    /// Tell the loop there is something new. Cheap; call it after a
    /// producer's transaction commits.
    pub fn wake(&self) {
        self.notify.notify_one();
    }

    /// Run every consumer until each has caught up with the log.
    pub async fn run_once(&self) -> Result<PumpReport, DomainError> {
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
                    }
                }
            }
        }
        Ok(report)
    }

    /// One event to one consumer: handler (under a savepoint) and
    /// checkpoint in one transaction; a failure becomes a dead letter in
    /// that same transaction.
    async fn deliver(
        &self,
        consumer: Arc<dyn EventConsumer>,
        event: Arc<StoredEvent>,
    ) -> Result<Delivery, DomainError> {
        self.db
            .transaction(move |tx| {
                let outcome = if consumer.handles(&event.envelope.event_type) {
                    match run_handler(consumer.as_ref(), tx, &event) {
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
        self.db
            .transaction(move |tx| {
                let Some(event) = event_by_seq_tx(tx, seq)? else {
                    return Err(DomainError::NotFound);
                };
                match run_handler(consumer.as_ref(), tx, &event) {
                    Ok(()) => set_dead_letter_state_tx(tx, id, "retried"),
                    Err(err) => dead_letter_tx(tx, consumer.name(), seq, &err.to_string()),
                }
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

    /// The delivery loop: run, then wait for a wake or the idle interval.
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match self.run_once().await {
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
}

/// The handler under a savepoint, so a failing handler's partial writes
/// roll back while the dead letter and checkpoint written afterwards in
/// the same transaction commit. A panicking handler is a failure too:
/// the pump must never stall on one.
fn run_handler(
    consumer: &dyn EventConsumer,
    conn: &rusqlite::Connection,
    event: &StoredEvent,
) -> Result<(), DomainError> {
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
    use oxplow_domain::{Envelope, EventSchemaRegistry};
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
                dead_lettered: 1
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
