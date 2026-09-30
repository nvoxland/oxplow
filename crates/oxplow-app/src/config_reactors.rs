//! Reactions to `config.changed` (`.context/commands.md`): a config key
//! whose new value has to reach something running. Driven by the event
//! log, so a change made by anyone — the person's Settings page, an
//! agent's `config.set`, an undo — takes effect the same way.
//!
//! The new value comes from the event's own `after`, never from the
//! in-memory config: the pump can see the committed event before the
//! command's after-commit swap lands.

use std::path::PathBuf;

use oxplow_config::GeneratedConfig;
use oxplow_domain::events::schema::ConfigChangedV1;
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;

pub const WORKSPACE_FILTER: &str = "config.workspace_filter";

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
        let change: ConfigChangedV1 = serde_json::from_value(event.envelope.payload.clone())
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

    /// tsk515: an agent's `config.set` on `generated` (not only the
    /// Settings page) reaches the snapshot captures; unsetting it lifts
    /// the exclusion again.
    #[tokio::test]
    async fn a_generated_change_reaches_the_captures() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let agent = Actor::Agent {
            thread_id: Some(ThreadId::new(1)),
            stream_id: Some(StreamId::new(1)),
        };
        assert!(!ignores(svc, "out/bundle.js"));
        svc.commands
            .run(
                &agent,
                "config.set",
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
            .run(&agent, "config.unset", json!({ "key": "generated" }), false)
            .await
            .unwrap();
        assert!(
            svc.event_pump
                .settle(&[WORKSPACE_FILTER], Duration::from_secs(5))
                .await
        );
        assert!(!ignores(svc, "out/bundle.js"));
    }
}
