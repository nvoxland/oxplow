//! Language servers (P8.A9): `oxplow.lsp.install_server` downloads a Mason package
//! and registers its binary; `oxplow.lsp.remove_server` deletes it. `External`
//! (the network and `.oxplow/lsp/`) and `Confirm::Always`: what binaries
//! oxplow downloads and runs is a person's call — an agent's run becomes a
//! proposal. Each pushes `LspServersChanged` so the settings list
//! refreshes (the server list isn't a model).

use oxplow_domain::Confirm;
use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::{Handler, HandlerOutput, Invocation};
use crate::background_task::{BackgroundTaskKind, BackgroundTaskStore, StartInput};
use crate::events::{EventBus, OxplowEvent};
use crate::lsp_installer::LspInstallerService;

/// The commands its operations are declared as.
pub const INSTALL_SERVER: &str = "oxplow.lsp.install_server";
pub const REMOVE_SERVER: &str = "oxplow.lsp.remove_server";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageInput {
    /// The Mason package (`rust-analyzer`, `typescript-language-server`).
    pub package: String,
}

/// What installing touches.
#[derive(Clone)]
pub struct LspDeps {
    pub installer: LspInstallerService,
    pub background: BackgroundTaskStore,
    pub events: EventBus,
}

fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(PackageInput)).expect("schema serializes")
}

fn parse(input: Value) -> Result<PackageInput, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed {
        message: e.to_string(),
    }
}

/// `lsp.install_server { package }`.
pub fn install_op(deps: LspDeps) -> Op {
    Op::new(
        "lsp.install",
        "install_server",
        schema(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input = parse(input)?;
                let task = deps.background.start(StartInput {
                    kind: BackgroundTaskKind::Lsp,
                    label: format!("Install language server: {}", input.package),
                    detail: Some("downloading from mason-registry".into()),
                    progress: None,
                });
                match deps.installer.install(&input.package).await {
                    Ok(entry) => {
                        deps.background.complete(&task.id, None);
                        deps.events.emit(OxplowEvent::LspServersChanged);
                        Ok(HandlerOutput {
                            result: json!({
                                "name": entry.name,
                                "version": entry.version,
                                "language_ids": entry.language_ids,
                                "binary": entry.binary.to_string_lossy(),
                            }),
                            ..HandlerOutput::default()
                        })
                    }
                    Err(e) => {
                        deps.background.fail(&task.id, e.to_string(), None);
                        Err(failed(e))
                    }
                }
            })
        })),
    )
    .confirm_at_least(Confirm::Always)
}

/// `lsp.remove_server { package }`.
pub fn remove_op(deps: LspDeps) -> Op {
    Op::new(
        "lsp.install",
        "remove_server",
        schema(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input = parse(input)?;
                deps.installer
                    .remove(&input.package)
                    .await
                    .map_err(failed)?;
                deps.events.emit(OxplowEvent::LspServersChanged);
                Ok(HandlerOutput::default())
            })
        })),
    )
    .confirm_at_least(Confirm::Always)
}

pub fn ops(deps: LspDeps) -> Vec<Op> {
    vec![install_op(deps.clone()), remove_op(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;

    /// What oxplow downloads and runs is a person's call.
    #[tokio::test]
    async fn an_agents_install_is_a_proposal() {
        let fx = services_with_effort().await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                INSTALL_SERVER,
                json!({ "package": "rust-analyzer" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    }

    /// Removing a server that isn't installed is a no-op, not an error.
    #[tokio::test]
    async fn removing_an_absent_server_is_idempotent() {
        let fx = services_with_effort().await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REMOVE_SERVER,
                json!({ "package": "not-installed" }),
                true,
            )
            .await
            .unwrap();
    }
}
