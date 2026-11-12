//! The writes of an extension's own store and folders
//! (`.context/extensions-permissions.md` "Store services"): commands a
//! script composes, so they run after policy and confirmation, never inside
//! `scope()`. Each runs as the composing extension and refuses any other
//! caller; its private values stay out of the audit.

use std::sync::Arc;

use oxplow_db::extension_store::Statement;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::extension::extension_services::ExtensionServices;

use super::ops::Op;
use super::util::schema;
use super::{Handler, HandlerOutput, Invocation};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatementInput {
    /// One SQL statement; `?1`, `?2`, … bind from `params`.
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StoreWrite {
    /// The statements, in one transaction on the extension's own file.
    batch: Vec<StatementInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileWrite {
    /// Relative to the project root, in a declared folder with `access: write`.
    path: String,
    body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileDelete {
    path: String,
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn refused(message: String) -> CommandError {
    CommandError::Denied { reason: message }
}

/// Run the composer's observers of what it just wrote, as its command, so
/// the model it fills shows the write when the command returns. A failure
/// is logged, not raised: the write already happened.
async fn observe(
    runner: &crate::collector_runner::CollectorRunner,
    services: &ExtensionServices,
    run: &oxplow_domain::run::RunContext,
    command: &str,
    wrote: crate::collector_runner::Wrote,
) {
    let Ok(extension) = services.composer_name(run, command) else {
        return;
    };
    for problem in runner.observe_written(run, &extension, wrote).await {
        tracing::warn!(%extension, %problem, "a store's write wasn't observed at once; its next trigger will");
    }
}

pub fn ops(
    services: Arc<ExtensionServices>,
    runner: crate::collector_runner::CollectorRunner,
) -> Vec<Op> {
    let store = {
        let services = services.clone();
        let runner = runner.clone();
        Op::new(
            "store.write",
            "write",
            schema::<StoreWrite>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                let runner = runner.clone();
                Box::pin(async move {
                    let input: StoreWrite = parse(input)?;
                    let batch: Vec<Statement> = input
                        .batch
                        .into_iter()
                        .map(|s| Statement {
                            sql: s.sql,
                            params: s.params,
                        })
                        .collect();
                    let result =
                        services
                            .write_store(&invocation.run, &batch)
                            .map_err(|message| {
                                if message.contains("can only be composed") {
                                    refused(message)
                                } else {
                                    CommandError::Failed { message }
                                }
                            })?;
                    observe(
                        &runner,
                        &services,
                        &invocation.run,
                        "oxplow.extension_store.write",
                        crate::collector_runner::Wrote::OwnStore,
                    )
                    .await;
                    Ok(HandlerOutput {
                        result,
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .with_unrecorded(&["batch"])
    };
    let write = {
        let services = services.clone();
        let runner = runner.clone();
        Op::new(
            "files.write",
            "extension_write",
            schema::<FileWrite>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                let runner = runner.clone();
                Box::pin(async move {
                    let input: FileWrite = parse(input)?;
                    let result = services
                        .write_file(&invocation.run, &input.path, &input.body)
                        .map_err(refused)?;
                    observe(
                        &runner,
                        &services,
                        &invocation.run,
                        "oxplow.extension_files.write",
                        crate::collector_runner::Wrote::File {
                            path: input.path.clone(),
                            op: crate::collector_runner::FileOp::Modified,
                        },
                    )
                    .await;
                    Ok(HandlerOutput {
                        result,
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .with_unrecorded(&["body"])
    };
    let delete = Op::new(
        "files.write",
        "extension_delete",
        schema::<FileDelete>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            let runner = runner.clone();
            Box::pin(async move {
                let input: FileDelete = parse(input)?;
                let result = services
                    .delete_file(&invocation.run, &input.path)
                    .map_err(refused)?;
                observe(
                    &runner,
                    &services,
                    &invocation.run,
                    "oxplow.extension_files.delete",
                    crate::collector_runner::Wrote::File {
                        path: input.path.clone(),
                        op: crate::collector_runner::FileOp::Removed,
                    },
                )
                .await;
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    );
    vec![store, write, delete]
}
