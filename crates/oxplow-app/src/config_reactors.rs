//! Reactions to `config.changed` (`.context/commands.md`): a config key
//! whose new value has to reach something running. Driven by the event
//! log, so a change made by anyone — the person's Settings page, an
//! agent's `oxplow.config.set`, an undo — takes effect the same way.
//!
//! The new value comes from the event's own `after`, never from the
//! in-memory config: the pump can see the committed event before the
//! command's after-commit swap lands. A reactor that reads the whole
//! config (a reconcile, a reseed) first waits for that swap
//! ([`applied`]).
//!
//! These are how config reaches the backend (P7.B6): nothing there
//! listens to the in-memory `ConfigChanged`, which is the renderer's.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use oxplow_config::GeneratedConfig;
use oxplow_domain::events::schema::ConfigChangedV2;
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;

pub const WORKSPACE_FILTER: &str = "config.workspace_filter";
/// `extensions` (which are on): the catalog's change signal.
pub const EXTENSIONS: &str = "config.extensions";
/// `extensionInstances`, `activeProviders`: the provider registry reconciles.
pub const PROVIDERS: &str = "config.providers";
/// Any key: the metric catalog reseeds (`measures`, `metrics`,
/// `dimensions`, `collectors`, the retention and visibility keys).
pub const METRICS: &str = "config.metrics";

/// How long a reactor waits for `oxplow.config.set`'s after-commit swap.
const APPLY_WAIT: Duration = Duration::from_secs(5);

/// Wait until the in-memory config has `change` applied — its key no
/// longer reads `before` (it reads `after`, or a later change's value) —
/// or [`APPLY_WAIT`] passes. `oxplow.config.set` swaps it right after the commit
/// the pump may already have seen.
pub async fn applied(svc: &crate::Services, change: &ConfigChangedV2) {
    let deadline = tokio::time::Instant::now() + APPLY_WAIT;
    loop {
        // Registered before the check, so a swap between the two wakes it.
        let swapped = svc.config_applied.notified();
        let now = oxplow_config::keys::key_value(
            &crate::config_service::read_config(&svc.config),
            &svc.layout.project_dir,
            &change.key,
        )
        .unwrap_or(serde_json::Value::Null);
        if now != change.before || change.before == change.after {
            return;
        }
        if tokio::time::timeout_at(deadline, swapped).await.is_err() {
            tracing::warn!(key = %change.key, "config.changed: the in-memory config never applied it");
            return;
        }
    }
}

type Act = Box<
    dyn Fn(Arc<crate::Services>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// A reaction to some config keys (`keys`; empty: every key), once the
/// change has applied.
pub struct KeyReactor {
    name: &'static str,
    keys: &'static [&'static str],
    svc: Weak<crate::Services>,
    act: Act,
}

#[async_trait::async_trait]
impl AsyncEventConsumer for KeyReactor {
    fn name(&self) -> &'static str {
        self.name
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "config.changed"
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let change: ConfigChangedV2 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("config.changed: {e}")))?;
        if !self.keys.is_empty() && !self.keys.contains(&change.key.as_str()) {
            return Ok(());
        }
        let Some(svc) = self.svc.upgrade() else {
            return Ok(());
        };
        applied(&svc, &change).await;
        (self.act)(svc).await;
        Ok(())
    }
}

/// Register the reactors that need the whole of `Services`.
pub fn register(state: &Arc<crate::Services>) {
    let svc = Arc::downgrade(state);
    let reactor = |name, keys, act: Act| {
        Arc::new(KeyReactor {
            name,
            keys,
            svc: svc.clone(),
            act,
        })
    };
    state.event_pump.register_async(reactor(
        EXTENSIONS,
        &["extensions"],
        Box::new(|svc| Box::pin(async move { svc.extension_catalog.changed() })),
    ));
    state.event_pump.register_async(reactor(
        PROVIDERS,
        &["extensionInstances", "activeProviders"],
        Box::new(|svc| Box::pin(async move { svc.providers.reconcile().await })),
    ));
    state.event_pump.register_async(reactor(
        METRICS,
        &[],
        Box::new(|svc| Box::pin(async move { svc.metrics.reseed().await })),
    ));
}

/// `generated` changed: the snapshot captures take the new filter, so an
/// include/exclude change applies without a restart.
pub struct WorkspaceFilterConsumer {
    pub captures: SnapshotCaptureRegistry,
    pub project_dir: PathBuf,
}

#[async_trait::async_trait]
impl AsyncEventConsumer for WorkspaceFilterConsumer {
    fn name(&self) -> &'static str {
        WORKSPACE_FILTER
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "config.changed"
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let change: ConfigChangedV2 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("config.changed: {e}")))?;
        if change.key != "generated" {
            return Ok(());
        }
        let generated: GeneratedConfig = if change.after.is_null() {
            GeneratedConfig::default()
        } else {
            serde_json::from_value(change.after)
                .map_err(|e| DomainError::Invalid(format!("config.changed generated: {e}")))?
        };
        self.captures
            .set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::for_project(
                &self.project_dir,
                &generated.exclude,
                &generated.include,
            ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use oxplow_domain::{Actor, StreamId, ThreadId};
    use serde_json::json;

    use super::*;

    fn ignores(svc: &crate::Services, path: &str) -> bool {
        svc.snapshot_captures
            .workspace_filter()
            .ignore(&svc.layout.project_dir.join(Path::new(path)), false)
    }

    /// tsk515: an agent's `oxplow.config.set` on `generated` (not only the
    /// Settings page) reaches the snapshot captures; unsetting it lifts
    /// the exclusion again.
    #[tokio::test]
    async fn a_generated_change_reaches_the_captures() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(ThreadId::new(1)),
            stream_id: Some(StreamId::new(1)),
        };
        assert!(!ignores(svc, "out/bundle.js"));
        svc.commands
            .run(
                &agent,
                "oxplow.config.set",
                json!({ "key": "generated", "value": { "exclude": ["out"] } }),
                false,
            )
            .await
            .unwrap();
        assert!(
            svc.event_pump
                .settle(&[WORKSPACE_FILTER], Duration::from_secs(5))
                .await
        );
        assert!(ignores(svc, "out/bundle.js"));
        svc.commands
            .run(
                &agent,
                "oxplow.config.unset",
                json!({ "key": "generated" }),
                false,
            )
            .await
            .unwrap();
        assert!(
            svc.event_pump
                .settle(&[WORKSPACE_FILTER], Duration::from_secs(5))
                .await
        );
        assert!(!ignores(svc, "out/bundle.js"));
    }

    /// P7.B6: a config change reaches the metric catalog through the pump
    /// (`config.metrics` on `config.changed`), not the in-memory bus.
    #[tokio::test]
    async fn a_metrics_change_reseeds_the_catalog_through_the_pump() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        crate::config_reactors::register(&svc_arc(&f));
        svc.commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({ "key": "metrics", "value": [
                    { "key": "work.ready_now", "entity": "v_work_item", "where": "state = 'todo'" }
                ] }),
                false,
            )
            .await
            .unwrap();
        assert!(
            svc.event_pump
                .settle(&[METRICS], Duration::from_secs(5))
                .await
        );
        assert!(svc
            .fact_store
            .get_spec("work.ready_now")
            .await
            .unwrap()
            .is_some());
    }

    /// P7.B6: turning an extension off reaches whatever follows the
    /// extensions through the catalog's change signal.
    #[tokio::test]
    async fn an_extensions_change_signals_the_catalog() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        crate::config_reactors::register(&svc_arc(&f));
        let mut changes = svc.extension_catalog.changes();
        svc.commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({ "key": "extensions", "value": { "disabled": ["oxplow-bundled"] } }),
                true,
            )
            .await
            .unwrap();
        assert!(
            svc.event_pump
                .settle(&[EXTENSIONS], Duration::from_secs(5))
                .await
        );
        assert!(changes.try_recv().is_ok());
    }

    fn svc_arc(f: &crate::test_fixtures::EffortFixture) -> Arc<crate::Services> {
        f.svc.clone()
    }
}
