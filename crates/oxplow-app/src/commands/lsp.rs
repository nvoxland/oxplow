//! Language servers (P8.A9): `oxplow.lsp.install_server` downloads a Mason package
//! and registers its binary; `oxplow.lsp.remove_server` deletes it. `External`
//! (the network and `.oxplow/lsp/`) and `Confirm::Always`: what binaries
//! oxplow downloads and runs is a person's call — an agent's run becomes a
//! proposal. Each pushes `LspServersChanged` so the settings list
//! refreshes (the server list isn't a dataset).

use oxplow_domain::Confirm;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ops::Op;
use super::util::{bare_id, failed, parse, schema, ServicesSlot};
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

/// `lsp.install_server { package }`.
pub fn install_op(deps: LspDeps) -> Op {
    Op::new(
        "lsp.install",
        "install_server",
        schema::<PackageInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input = parse::<PackageInput>(input)?;
                let task = deps.background.start(StartInput {
                    kind: BackgroundTaskKind::Lsp,
                    label: format!("Install language server: {}", input.package),
                    detail: Some("downloading from mason-registry".into()),
                    progress: None,
                });
                match deps.installer.install(&input.package).await {
                    Ok(entry) => {
                        deps.background.complete(&task.id, None);
                        Ok(HandlerOutput {
                            result: json!({
                                "name": entry.name,
                                "version": entry.version,
                                "language_ids": entry.language_ids,
                                "binary": entry.binary.to_string_lossy(),
                            }),
                            after_commit: Some(servers_changed(&deps)),
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
        schema::<PackageInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input = parse::<PackageInput>(input)?;
                deps.installer
                    .remove(&input.package)
                    .await
                    .map_err(failed)?;
                Ok(HandlerOutput {
                    result: json!({ "removed": input.package }),
                    after_commit: Some(servers_changed(&deps)),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .confirm_at_least(Confirm::Always)
}

/// Once the run is recorded: the server list (not a dataset) re-reads.
fn servers_changed(deps: &LspDeps) -> Box<dyn FnOnce() + Send + Sync> {
    let events = deps.events.clone();
    Box::new(move || events.emit(OxplowEvent::LspServersChanged))
}

/// `lsp.restart_server { stream, language }`: tear down the stream's session
/// for a language and start it again, replaying the open documents. A
/// person's (or a page for one): a server is a program on this machine.
pub fn restart_op(services: ServicesSlot) -> Op {
    Op::new(
        "lsp.control",
        "restart_server",
        schema::<RestartInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input = parse::<RestartInput>(input)?;
                let stream: oxplow_domain::StreamId = bare_id(
                    &input.stream,
                    oxplow_domain::uri::resources::STREAM,
                    "/stream",
                )?;
                let svc = services.get()?;
                svc.code_intel
                    .restart(stream, &input.language)
                    .await
                    .map_err(failed)?;
                Ok(HandlerOutput {
                    result: json!({ "stream": input.stream, "language": input.language }),
                    after_commit: Some({
                        let events = svc.events.clone();
                        Box::new(move || events.emit(OxplowEvent::LspServersChanged))
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestartInput {
    /// The stream whose server it is (`str2`).
    pub stream: String,
    /// The language (`rust`, `typescript`).
    pub language: String,
}

pub fn ops(deps: LspDeps) -> Vec<Op> {
    vec![install_op(deps.clone()), remove_op(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;
    use oxplow_domain::CommandError;

    /// An agent's install runs without asking. The package is one the
    /// registry doesn't have, so nothing is installed: the run reaches the
    /// installer and fails there (one registry lookup), not at a gate.
    #[tokio::test]
    async fn an_agents_install_runs_without_asking() {
        let fx = services_with_effort().await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    session_id: None,
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                INSTALL_SERVER,
                json!({ "package": "no-such-package-for-a-test" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
    }

    /// An agent may restart a server too; a language with no server can't
    /// start one, for anyone.
    #[tokio::test]
    async fn restarting_a_server_needs_a_server() {
        let fx = services_with_effort().await;
        let stream = fx.svc.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let input = json!({ "stream": stream, "language": "no-such-language" });
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(&agent, "oxplow.lsp.restart_server", input.clone(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, "oxplow.lsp.restart_server", input, false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
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
