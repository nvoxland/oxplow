//! `metric.*`: metrics as commands (`.context/commands.md`, P4.7/P4.8).
//!
//! `metric.enable { keys, enabled }` switches metrics on or off in this
//! project's `.oxplow/project.yaml`. Which edit that is depends on the
//! metric — a bundled gauge is off until a `use:` names it, a producer or
//! plugin metric is on until an `enabled: false` marker turns it off — so
//! the command computes the new `metrics:` list with the metrics service's
//! rule and hands it to `config.set`'s core: validated, written after
//! commit, logged as `config.changed`, undone by restoring the old list. The
//! reseed (and `v_metric_catalog`) follows `ConfigChanged` as for any
//! config edit.

use std::sync::Arc;

use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::config_commands::{change, ConfigTarget};
use super::{Command, Handler};
use crate::metrics_service::MetricsService;

pub const ENABLE: &str = "metric.enable";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableInput {
    /// Metric keys (`v_metric_catalog.key`).
    pub keys: Vec<String>,
    /// On (`true`) or off.
    pub enabled: bool,
}

/// The `metric.*` commands.
pub fn commands(target: ConfigTarget, metrics: MetricsService) -> Vec<Command> {
    let enable = Command::new(
        CommandSpec {
            name: ENABLE.into(),
            summary: "Turn metrics on or off in this project (.oxplow/project.yaml `metrics:`); \
                      logged as config.changed, undo restores the previous list."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(EnableInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: true,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Write,
        },
        Handler::Tx(Arc::new(move |ctx: &super::TxCtx<'_>, input| {
            let input: EnableInput =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            for key in &input.keys {
                let known: bool = ctx
                    .conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM metric_catalog WHERE key = ?1)",
                        [key],
                        |r| r.get(0),
                    )
                    .map_err(|e| CommandError::Failed {
                        message: e.to_string(),
                    })?;
                if !known {
                    return Err(CommandError::Invalid {
                        field: Some("/keys".into()),
                        message: format!("no metric `{key}` (see v_metric_catalog)"),
                    });
                }
            }
            let mut list = target
                .config
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .metrics
                .clone();
            for key in &input.keys {
                metrics.apply_metric_enabled(&mut list, key, input.enabled);
            }
            let value = serde_json::to_value(&list).map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
            change(&target, ctx.actor, "metrics", Some(value))
        })),
    )
    .expect("metric.enable registers");
    vec![enable]
}
