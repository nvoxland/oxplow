//! Bounded polling for committed extension-job intent and expired attempts.
//! The dispatcher owns authority, claims and execution; this loop sees only
//! metadata and never retries an interrupted operation automatically.

use std::{sync::Arc, time::Duration};

use oxplow_db::extension_job_store::{RecoveryCursor, SqliteExtensionJobStore};
use oxplow_domain::{extension_job::JobId, run::Cancel};
use tokio::{task::JoinSet, time::Instant};

use crate::extension_job_dispatch::{DispatchError, DispatchOutcome, ExtensionJobDispatcher};

const MAX_DISPATCHES: usize = 4;
const PAGE_SIZE: usize = 64;
const TICK: Duration = Duration::from_secs(1);
const FAILURE_BACKOFF: Duration = Duration::from_secs(5);

/// One project-level scheduler. Construct only after the host has published a
/// coherent active catalog and exact operation registry.
pub struct ExtensionJobScheduler {
    store: SqliteExtensionJobStore,
    dispatcher: Arc<ExtensionJobDispatcher>,
    shutdown: Cancel,
    queued_cursor: Option<RecoveryCursor>,
    recovery_cursor: Option<RecoveryCursor>,
    active: Vec<JobId>,
    workers: JoinSet<(JobId, Result<DispatchOutcome, DispatchError>)>,
    backoff_until: Option<Instant>,
}

impl ExtensionJobScheduler {
    pub fn new(
        store: SqliteExtensionJobStore,
        dispatcher: Arc<ExtensionJobDispatcher>,
        shutdown: Cancel,
    ) -> Self {
        Self {
            store,
            dispatcher,
            shutdown,
            queued_cursor: None,
            recovery_cursor: None,
            active: Vec::new(),
            workers: JoinSet::new(),
            backoff_until: None,
        }
    }

    /// Poll metadata in bounded pages. A failed or unapproved queued owner
    /// remains durable; the cursor advances and wraps on the next sweep.
    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if self.shutdown.is_cancelled() {
                break;
            }
            tokio::select! {
                _ = tick.tick() => {
                    if self.backoff_until.is_none_or(|until| Instant::now() >= until) {
                        // A generation writer may wait behind running job
                        // readers. Keep shutdown responsive while this scan
                        // waits to acquire its own reader.
                        let shutdown = self.shutdown.clone();
                        let failed = tokio::select! {
                            result = self.scan() => result.is_err(),
                            _ = wait_for_cancel(shutdown) => break,
                        };
                        if failed {
                            self.backoff_until = Some(Instant::now() + FAILURE_BACKOFF);
                        } else {
                            self.backoff_until = None;
                        }
                    }
                }
                joined = self.workers.join_next(), if !self.workers.is_empty() => {
                    if let Some(joined) = joined {
                        self.joined(joined).await;
                    }
                }
            }
        }

        // Dropping a dispatcher future asks its run to cancel. It does not
        // prove an external provider stopped; durable leases recover later.
        self.workers.abort_all();
        while self.workers.join_next().await.is_some() {}
    }

    async fn scan(&mut self) -> Result<(), ()> {
        match self
            .dispatcher
            .recover_page(self.recovery_cursor.clone(), PAGE_SIZE)
            .await
        {
            Ok(page) => self.recovery_cursor = page.next,
            Err(_) => {
                tracing::warn!("extension job recovery scan failed");
                return Err(());
            }
        }

        if self.active.len() >= MAX_DISPATCHES {
            return Ok(());
        }

        let page = match self
            .store
            .queued_candidates(self.queued_cursor.clone(), PAGE_SIZE)
            .await
        {
            Ok(page) => page,
            Err(_) => {
                tracing::warn!("extension job queue scan failed");
                return Err(());
            }
        };
        let page_len = page.items.len();
        let mut consumed = 0;
        let mut last_consumed = None;
        for candidate in page.items {
            if self.active.len() >= MAX_DISPATCHES {
                break;
            }
            consumed += 1;
            last_consumed = Some(RecoveryCursor {
                updated_at_ms: candidate.job.updated_at_ms,
                id: candidate.job.id.clone(),
            });
            let id = candidate.job.id;
            if self.active.contains(&id) {
                continue;
            }
            self.active.push(id.clone());
            let dispatcher = self.dispatcher.clone();
            self.workers.spawn(async move {
                let result = dispatcher.run_one(id.clone()).await;
                (id, result)
            });
        }
        self.queued_cursor = if consumed == page_len {
            page.next
        } else {
            last_consumed
        };
        Ok(())
    }

    async fn joined(
        &mut self,
        joined: Result<(JobId, Result<DispatchOutcome, DispatchError>), tokio::task::JoinError>,
    ) {
        match joined {
            Ok((id, result)) => {
                self.active.retain(|active| active != &id);
                if result.is_err() {
                    tracing::warn!("extension job dispatch failed");
                    self.backoff_until = Some(Instant::now() + FAILURE_BACKOFF);
                }
            }
            Err(_) => {
                // A panicked task did not return its ID. Stop and drain all
                // siblings before clearing the in-process active set.
                tracing::warn!("extension job dispatch task ended unexpectedly");
                self.workers.abort_all();
                while self.workers.join_next().await.is_some() {}
                self.active.clear();
                self.backoff_until = Some(Instant::now() + FAILURE_BACKOFF);
            }
        }
    }
}

async fn wait_for_cancel(cancel: Cancel) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(TICK).await;
    }
}
