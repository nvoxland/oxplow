//! Language servers (P8.A9): `lsp.install_server` downloads a Mason package
//! and registers its binary; `lsp.remove_server` deletes it. `External`
//! (the network and `.oxplow/lsp/`) and `Confirm::Always`: what binaries
//! oxplow downloads and runs is a person's call — an agent's run becomes a
//! proposal. Each pushes `LspServersChanged` so the settings list
//! refreshes (the server list isn't a model).

use std::sync::Arc;

use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Command, Handler, HandlerOutput, Invocation};
use crate::background_task::{BackgroundTaskKind, BackgroundTaskStore, StartInput};
use crate::events::{EventBus, OxplowEvent};
use crate::lsp_installer::LspInstallerService;

pub const INSTALL_SERVER: &str = "lsp.install_server";
pub const REMOVE_SERVER: &str = "lsp.remove_server";

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

fn spec(name: &str, summary: &str) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: serde_json::to_value(schemars::schema_for!(PackageInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        confirm: Confirm::Always,
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::External,
        effect: CommandEffect::Write,
    }
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
pub fn install_command(deps: LspDeps) -> Command {
    Command::new(
        spec(
            INSTALL_SERVER,
            "Download and install a language server (a Mason package) and register its \
             binary. A person's decision: an agent's run becomes a proposal.",
        ),
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
    .expect("lsp.install_server is a valid command")
}

/// `lsp.remove_server { package }`.
pub fn remove_command(deps: LspDeps) -> Command {
    Command::new(
        spec(
            REMOVE_SERVER,
            "Uninstall a language server: delete its files, manifest entry and registrations. \
             A person's decision.",
        ),
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
    .expect("lsp.remove_server is a valid command")
}

pub fn commands(deps: LspDeps) -> Vec<Command> {
    vec![install_command(deps.clone()), remove_command(deps)]
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
