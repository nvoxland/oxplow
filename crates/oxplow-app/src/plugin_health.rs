//! One failure policy for every plugin contribution (P7.C1): a provider
//! instance, a collector, an effect (P8.D11).
//!
//! [`PluginHealth`] keeps each contribution's `plugin_health` row (V135,
//! read as `v_plugin_health`). A failure counts; [`FAILURES_TO_DISABLE`]
//! in a row disable it — the row and `plugin.disabled@1` commit together
//! — and it stays off, across restarts, until a person runs
//! `plugin.enable` (`plugin.enabled@1`). A success starts the count over.
//! What "off" means is the contribution's: the provider registry stops
//! the instance; a collector's scheduler skips it; an effect stops
//! reacting (`effect_triggers`).

use oxplow_domain::vocabulary::VocabularyHandle;
use std::sync::{Arc, Weak};
use std::time::Duration;

use oxplow_db::plugin_health_store::{self as store, PluginHealthRow, PluginKey};
use oxplow_db::Database;
use oxplow_domain::events::schema::{
    PluginDisabled, PluginDisabledV1, PluginEnabled, PluginEnabledV1,
};
use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, DomainError, Envelope, Invokers,
    Lifecycle,
};
use serde::Deserialize;

use crate::commands::{Command, Handler, HandlerOutput, Invocation};

/// Failures in a row that disable a contribution.
pub const FAILURES_TO_DISABLE: i64 = 3;
/// The command a person runs to enable a disabled contribution again.
pub const ENABLE: &str = "plugin.enable";
/// What a disable is logged as.
const SOURCE: &str = "system:plugins";

/// What a counted failure means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Try again later: `failures` in a row so far.
    Backoff { failures: i64 },
    /// It's off now, for this reason (logged as `plugin.disabled@1`).
    Disabled { reason: String },
}

/// The policy over `plugin_health`. A cheap handle: clone it.
#[derive(Clone)]
pub struct PluginHealth {
    db: Database,
    vocabulary: VocabularyHandle,
}

/// How late a scheduled run may start: the collector and provider
/// schedulers each tick once a minute.
pub const SCHEDULER_TICK_MS: i64 = 60_000;

/// When a scheduled contribution should have run again by: its last run —
/// or now, when it's due and runs now — plus its interval, plus one
/// scheduler tick. Past it, `v_plugin_health.fresh` says it missed its
/// schedule.
pub fn next_due_ms(last_ms: Option<i64>, every_ms: i64, now_ms: i64) -> i64 {
    let ran = match last_ms {
        Some(last) if now_ms - last < every_ms => last,
        _ => now_ms,
    };
    ran + every_ms + SCHEDULER_TICK_MS
}

fn now() -> String {
    oxplow_domain::Timestamp::now().to_string()
}

fn disabled_event(key: &PluginKey, reason: &str) -> Envelope {
    Envelope::typed::<PluginDisabled>(
        SOURCE,
        &PluginDisabledV1 {
            plugin: plugin_ref(&key.plugin),
            contribution: key.contribution.clone(),
            kind: key.kind.to_string(),
            reason: reason.to_string(),
        },
    )
    .with_subject([plugin_ref(&key.plugin)])
}

/// `plugin:<extension>`.
pub fn plugin_ref(plugin: &str) -> String {
    format!("plugin:{plugin}")
}

impl PluginHealth {
    pub fn new(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self { db, vocabulary }
    }

    /// Count a failure. The [`FAILURES_TO_DISABLE`]th in a row disables it
    /// (unless it already is), with the row and `plugin.disabled@1` in one
    /// transaction.
    pub async fn failed(&self, key: &PluginKey, error: &str) -> Result<Verdict, DomainError> {
        let (key, error, vocabulary) = (key.clone(), error.to_string(), self.vocabulary.clone());
        self.db
            .transaction(move |tx| {
                let at = now();
                let failures = store::failed_tx(tx, &key, &error, &at)?;
                let already = store::get_tx(tx, &key)?.is_some_and(|r| r.state == "disabled");
                if already {
                    return Ok(Verdict::Disabled {
                        reason: store::get_tx(tx, &key)?
                            .and_then(|r| r.reason)
                            .unwrap_or_default(),
                    });
                }
                if failures < FAILURES_TO_DISABLE {
                    return Ok(Verdict::Backoff { failures });
                }
                let reason = format!("{failures} failures in a row; the last: {error}");
                store::disable_tx(tx, &key, &reason, &at)?;
                oxplow_db::event_log_store::append_tx(
                    tx,
                    &vocabulary.current(),
                    &disabled_event(&key, &reason),
                )?;
                Ok(Verdict::Disabled { reason })
            })
            .await
    }

    /// A success: the count starts over; `took` (a timed run or call)
    /// joins its average.
    pub async fn succeeded(
        &self,
        key: &PluginKey,
        took: Option<Duration>,
    ) -> Result<(), DomainError> {
        let key = key.clone();
        let ms = took.map(|t| t.as_secs_f64() * 1000.0);
        self.db
            .transaction(move |tx| store::succeeded_tx(tx, &key, ms, &now()))
            .await
    }

    /// Disable it now, for `reason` — something other than a count of
    /// failures (a provider that no longer answers with its approved
    /// declarations). Logged once: already disabled, nothing changes.
    /// Forget `key`: its contribution is gone (a removed provider instance).
    pub async fn forget(&self, key: &PluginKey) -> Result<(), DomainError> {
        store::SqlitePluginHealthStore::new(self.db.clone())
            .remove(key)
            .await
    }

    pub async fn disable(&self, key: &PluginKey, reason: &str) -> Result<(), DomainError> {
        let (key, reason, vocabulary) = (key.clone(), reason.to_string(), self.vocabulary.clone());
        self.db
            .transaction(move |tx| {
                if store::get_tx(tx, &key)?.is_some_and(|r| r.state == "disabled") {
                    return Ok(());
                }
                store::disable_tx(tx, &key, &reason, &now())?;
                oxplow_db::event_log_store::append_tx(
                    tx,
                    &vocabulary.current(),
                    &disabled_event(&key, &reason),
                )
                .map(|_| ())
            })
            .await
    }

    /// A person enabled it again (`plugin.enable`, logged as `source`):
    /// `ok`, its count starting over, and `plugin.enabled@1`.
    pub async fn enable(&self, key: &PluginKey, source: &str) -> Result<(), DomainError> {
        let (key, source, vocabulary) = (key.clone(), source.to_string(), self.vocabulary.clone());
        self.db
            .transaction(move |tx| {
                store::enable_tx(tx, &key, &now())?;
                let event = Envelope::typed::<PluginEnabled>(
                    source.clone(),
                    &PluginEnabledV1 {
                        plugin: plugin_ref(&key.plugin),
                        contribution: key.contribution.clone(),
                        kind: key.kind.to_string(),
                    },
                )
                .with_subject([plugin_ref(&key.plugin)]);
                oxplow_db::event_log_store::append_tx(tx, &vocabulary.current(), &event).map(|_| ())
            })
            .await
    }

    /// What a scheduler planned, in one transaction: each contribution's
    /// [`next_due_ms`], `None` for one that doesn't run on a schedule now
    /// (manual, not approved, disabled).
    pub async fn set_next_due(
        &self,
        plans: Vec<(PluginKey, Option<i64>)>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                let now = now();
                for (key, due) in &plans {
                    let due = due.map(|ms| oxplow_domain::Timestamp::from_unix_ms(ms).to_string());
                    store::set_next_due_tx(tx, key, due.as_deref(), &now)?;
                }
                Ok(())
            })
            .await
    }

    /// Why it's disabled, when it is.
    pub async fn disabled_reason(&self, key: &PluginKey) -> Result<Option<String>, DomainError> {
        Ok(self
            .get(key)
            .await?
            .filter(|r| r.state == "disabled")
            .map(|r| r.reason.unwrap_or_default()))
    }

    pub async fn get(&self, key: &PluginKey) -> Result<Option<PluginHealthRow>, DomainError> {
        let key = key.clone();
        self.db.read(move |c| store::get_tx(c, &key)).await
    }
}

/// Which kind of contribution `plugin.enable` names.
#[derive(Deserialize, schemars::JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Provider,
    Collector,
    Effect,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Provider => "provider",
            Kind::Collector => "collector",
            Kind::Effect => "effect",
        }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnableInput {
    /// The extension.
    plugin: String,
    /// `provider`, `collector` or `effect`: two kinds may share an id.
    kind: Kind,
    /// Its provider's, collector's or effect's id.
    contribution: String,
}

/// `plugin.enable { plugin, kind, contribution }`: a person turns a contribution
/// back on on this machine, clearing an automatic disable. A provider
/// instance starts again when the project's config enables it; a
/// collector runs at its next trigger. Human
/// only (an agent can't undo what stopped a failing plugin); logs
/// `plugin.enabled@1`.
pub fn enable_command(
    health: PluginHealth,
    providers: Weak<crate::providers::ProviderRegistry>,
) -> Command {
    Command::new(
        CommandSpec {
            name: ENABLE.into(),
            summary: "Enable a disabled extension provider, collector or effect on this machine \
                      again, clearing an automatic disable (a provider's process restarts, a \
                      system the bus doesn't own)."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(EnableInput))
                .expect("schema serializes"),
            invokers: Invokers::HUMAN_ONLY,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::External,
            effect: CommandEffect::Write,
        },
        Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
            let (health, providers) = (health.clone(), providers.clone());
            Box::pin(async move {
                let EnableInput {
                    plugin,
                    kind,
                    contribution,
                } = serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
                let providers = providers.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "the provider registry is gone".into(),
                })?;
                let instance = format!("{plugin}/{contribution}");
                let key = PluginKey {
                    plugin,
                    contribution,
                    kind: kind.as_str(),
                };
                // Something to enable: a contribution with health (a
                // collector has it once it failed), or a provider instance
                // the registry knows (enabled from Settings before it ever
                // failed).
                if health.get(&key).await?.is_none()
                    && (kind != Kind::Provider || providers.find(&instance).is_none())
                {
                    return Err(CommandError::Invalid {
                        field: Some("/contribution".into()),
                        message: format!(
                            "no {} `{instance}` has failed or is configured; there's nothing \
                             to enable",
                            kind.as_str()
                        ),
                    });
                }
                // Recorded first, so the reconcile below sees it cleared.
                health.enable(&key, &actor.source()).await?;
                if kind == Kind::Provider {
                    providers.reset(&instance).await;
                    providers.reconcile().await;
                }
                let row = health.get(&key).await?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(row).expect("row serializes"),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("plugin.enable is a valid command")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> PluginKey {
        PluginKey {
            plugin: "tracker".into(),
            contribution: "fake".into(),
            kind: "provider",
        }
    }

    async fn events(db: &Database, kind: &'static str) -> Vec<serde_json::Value> {
        db.read(move |c| {
            let mut st = c
                .prepare("SELECT payload FROM event_log WHERE type = ?1 ORDER BY seq")
                .map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([kind], |r| r.get::<_, String>(0))
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter_map(|p| serde_json::from_str(&p).ok())
                .collect())
        })
        .await
        .unwrap()
    }

    /// Three failures in a row disable it, logged once with the reason; a
    /// success in between starts the count over; an enable clears it.
    #[tokio::test]
    async fn three_failures_in_a_row_disable_until_enabled() {
        let db = Database::in_memory();
        let h = PluginHealth::new(db.clone(), VocabularyHandle::core());
        let k = key();
        assert_eq!(
            h.failed(&k, "a").await.unwrap(),
            Verdict::Backoff { failures: 1 }
        );
        h.succeeded(&k, Some(Duration::from_millis(5)))
            .await
            .unwrap();
        assert_eq!(
            h.failed(&k, "b").await.unwrap(),
            Verdict::Backoff { failures: 1 }
        );
        assert_eq!(
            h.failed(&k, "c").await.unwrap(),
            Verdict::Backoff { failures: 2 }
        );
        let Verdict::Disabled { reason } = h.failed(&k, "d").await.unwrap() else {
            panic!("disabled");
        };
        assert!(
            reason.contains("3 failures in a row") && reason.contains("d"),
            "{reason}"
        );
        // Already off: another failure doesn't log again.
        assert!(matches!(
            h.failed(&k, "e").await.unwrap(),
            Verdict::Disabled { .. }
        ));
        let disabled = events(&db, "plugin.disabled").await;
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0]["plugin"], "plugin:tracker");
        assert_eq!(disabled[0]["contribution"], "fake");
        assert_eq!(h.disabled_reason(&k).await.unwrap(), Some(reason));

        h.enable(&k, "human").await.unwrap();
        assert_eq!(h.disabled_reason(&k).await.unwrap(), None);
        assert_eq!(events(&db, "plugin.enabled").await.len(), 1);
        assert_eq!(
            h.failed(&k, "f").await.unwrap(),
            Verdict::Backoff { failures: 1 }
        );
    }

    /// P7 review (tsk721): a provider and a collector with the same
    /// `<plugin>/<id>` are two contributions with two health rows.
    #[tokio::test]
    async fn a_provider_and_a_collector_named_alike_have_their_own_health() {
        let db = Database::in_memory();
        let h = PluginHealth::new(db.clone(), VocabularyHandle::core());
        let provider = key();
        let collector = PluginKey {
            kind: "collector",
            ..key()
        };
        h.failed(&collector, "a").await.unwrap();
        h.disable(&provider, "broken").await.unwrap();
        let p = h.get(&provider).await.unwrap().unwrap();
        let c = h.get(&collector).await.unwrap().unwrap();
        assert_eq!(
            (p.kind.as_str(), p.state.as_str()),
            ("provider", "disabled")
        );
        assert_eq!(
            (c.kind.as_str(), c.state.as_str(), c.consecutive_failures),
            ("collector", "failing", 1)
        );
        assert_eq!(h.disabled_reason(&collector).await.unwrap(), None);
    }

    /// A run due now is planned from now; one not yet due from its last
    /// run — either way plus its interval and one scheduler tick.
    #[test]
    fn next_due_is_the_run_plus_its_interval_and_a_tick() {
        let (every, now) = (600_000, 10_000_000);
        assert_eq!(
            next_due_ms(None, every, now),
            now + every + SCHEDULER_TICK_MS
        );
        assert_eq!(
            next_due_ms(Some(now - every), every, now),
            now + every + SCHEDULER_TICK_MS
        );
        assert_eq!(
            next_due_ms(Some(now - 1_000), every, now),
            now - 1_000 + every + SCHEDULER_TICK_MS
        );
    }
}
