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
    /// The rows this statement must change (its own, not a trigger's). Fewer
    /// or more fails the command and rolls the batch back, naming the
    /// statement: how a command says "no such row".
    #[serde(default)]
    expect_changes: Option<usize>,
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
                            expect_changes: s.expect_changes,
                        })
                        .collect();
                    let composer = services
                        .composer_name(&invocation.run, "oxplow.extension_store.write")
                        .ok();
                    let result = runner
                        .observe_write(
                            &invocation.run,
                            composer.as_deref(),
                            crate::collector_runner::Wrote::OwnStore,
                            async { services.write_store(&invocation.run, &batch) },
                        )
                        .await
                        .map_err(|message| {
                            if message.contains("can only be composed") {
                                refused(message)
                            } else {
                                CommandError::Failed { message }
                            }
                        })?;
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
                    let composer = services
                        .composer_name(&invocation.run, "oxplow.extension_files.write")
                        .ok();
                    let result = runner
                        .observe_write(
                            &invocation.run,
                            composer.as_deref(),
                            crate::collector_runner::Wrote::File {
                                path: input.path.clone(),
                                op: crate::collector_runner::FileOp::Modified,
                            },
                            async {
                                services.write_file(&invocation.run, &input.path, &input.body)
                            },
                        )
                        .await
                        .map_err(refused)?;
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
                let composer = services
                    .composer_name(&invocation.run, "oxplow.extension_files.delete")
                    .ok();
                let result = runner
                    .observe_write(
                        &invocation.run,
                        composer.as_deref(),
                        crate::collector_runner::Wrote::File {
                            path: input.path.clone(),
                            op: crate::collector_runner::FileOp::Removed,
                        },
                        async { services.delete_file(&invocation.run, &input.path) },
                    )
                    .await
                    .map_err(refused)?;
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    );
    vec![store, write, delete]
}
