//! Lenses for an agent: `lenses.read/run` reads what a lens shows, as the
//! person sees it (its text rendering, or the run's rows), and
//! `lenses.write/run_action` presses one of its actions — the command it
//! declares, run through the bus as the lens acting for the caller
//! (`lens_actions`), so the caller's own policy bounds it.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::{CommandError, DomainError, StreamId, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use super::{check_stream_id, stream_or_actors};
use crate::lens_actions::LensActionCall;
use crate::Services;

/// The commands its operations are declared as.
pub const LENS_RUN: &str = "oxplow.lens.run";
pub const LENS_RUN_ACTION: &str = "oxplow.lens.run_action";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunLensInput {
    /// Lens id: `<extension>/<slug>`.
    pub id: String,
    /// Param overrides by name (see the lens's `params`); the rest use
    /// their defaults. Unknown names are rejected.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// Stream whose worktree to read from; omit for your own.
    #[serde(default)]
    pub stream_id: Option<String>,
    /// Your thread (`thr…`): bound into a lens's `thread_id` param (and its
    /// stream into `stream_id`) unless `params` sets them. Omit to use the
    /// stream's selected thread — what the person sees.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// `text` (default): the lens's text rendering (what it shows — a
    /// chart's series, a grid's children) with the run's columns, row
    /// count, params and alert, but not the rows. `json`: the full run,
    /// rows included, for when you need to parse the values.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunLensActionInput {
    /// Lens id: `<extension>/<slug>`.
    pub id: String,
    /// The action's id, as the lens declares it (see `read` of `lens:<ext>/<slug>` → `actions`).
    pub action: String,
    /// Param overrides by name, as for `oxplow.lens.run`.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// Stream whose worktree to read the lens from; omit for your own.
    #[serde(default)]
    pub stream_id: Option<String>,
    /// Your thread, as for `oxplow.lens.run`.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// For a row action (`row: true` in `read` of `lens:<ext>/<slug>` → `actions`): the row
    /// it runs on, column → value, as `oxplow.lens.run` returned it.
    #[serde(default)]
    pub row: Option<BTreeMap<String, Value>>,
}

/// Param values as the lens runner binds them.
fn cells(values: Option<BTreeMap<String, Value>>) -> BTreeMap<String, SqlCell> {
    values
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, SqlCell::from(v)))
        .collect()
}

/// The stream a lens runs against and the context it runs in: the stream
/// the call names, else the actor's own, and the thread it names (checked).
async fn lens_run_target(
    svc: &Services,
    invocation: &Invocation,
    stream_id: Option<String>,
    thread_id: Option<String>,
) -> Result<
    (
        Option<String>,
        std::path::PathBuf,
        crate::extensions::LensContext,
    ),
    CommandError,
> {
    if let Some(stream) = &stream_id {
        check_stream_id("/stream_id", stream)?;
    }
    let stream = stream_or_actors(svc, invocation.actor(), stream_id).await?;
    let root = svc
        .worktrees
        .resolve(stream.as_deref())
        .await
        .into_local_path();
    let thread = thread_id
        .map(|t| {
            ThreadId::try_from_str(&t).ok_or_else(|| {
                invalid(
                    "/thread_id",
                    format!("expected a thread id (thr…), got `{t}`"),
                )
            })
        })
        .transpose()?;
    let ctx = crate::extensions::lens_context(
        svc,
        stream.as_deref().and_then(StreamId::try_from_str),
        thread,
    )
    .await;
    Ok((stream, root, ctx))
}

/// A lens lookup/run error as the bus's.
fn lens_error(id: &str, e: DomainError) -> CommandError {
    match e {
        DomainError::NotFound => invalid(
            "/id",
            format!("no lens `{id}` (ids are `<extension>/<slug>`; see `v_lens`)"),
        ),
        DomainError::Invalid(message) => CommandError::Invalid {
            field: None,
            message,
        },
        other => failed(other),
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![run_op(services.clone()), run_action_op(services)]
}

fn run_op(services: ServicesSlot) -> Op {
    Op::new(
        "lenses.read",
        "run",
        schema::<RunLensInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: RunLensInput = parse(input)?;
                let format = p.format.as_deref().unwrap_or("text");
                if !matches!(format, "text" | "json") {
                    return Err(invalid(
                        "/format",
                        format!("format `{format}` isn't text or json"),
                    ));
                }
                let svc = services.get()?;
                let (_, root, ctx) =
                    lens_run_target(&svc, &invocation, p.stream_id.clone(), p.thread_id.clone())
                        .await?;
                // Through its extension's models as that stream has them,
                // compiled and published nowhere: a lens over a model the
                // author hasn't published yet answers.
                let extension = p.id.split_once('/').map_or(p.id.as_str(), |(ext, _)| ext);
                let overlay = crate::extensions::lens_overlay(
                    &svc.sql,
                    &svc.extension_catalog,
                    &root,
                    extension,
                )
                .await;
                let run = crate::extensions::run_lens(
                    &svc.sql.with_overlay(overlay),
                    &svc.extension_catalog,
                    &root,
                    &p.id,
                    cells(p.params),
                    &ctx,
                )
                .await
                .map_err(|e| lens_error(&p.id, e))?;
                let result = match format {
                    "json" => serde_json::to_value(&run).map_err(failed)?,
                    _ => serde_json::to_value(
                        crate::lens_text::text_run(&svc, &root, &run, &ctx)
                            .await
                            .map_err(|e| lens_error(&p.id, e))?,
                    )
                    .map_err(failed)?,
                };
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

fn run_action_op(services: ServicesSlot) -> Op {
    Op::new(
        "lenses.write",
        "run_action",
        schema::<RunLensActionInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let p: RunLensActionInput = parse(input)?;
                let svc = services.get()?;
                let (_, root, ctx) =
                    lens_run_target(&svc, &invocation, p.stream_id.clone(), p.thread_id.clone())
                        .await?;
                let outcome = crate::lens_actions::run_lens_action(
                    &svc,
                    &root,
                    LensActionCall {
                        lens_id: p.id,
                        action_id: p.action,
                        params: cells(p.params),
                        row: p.row.map(|r| cells(Some(r))),
                        input: None,
                        on_behalf_of: invocation.actor().clone(),
                        // An agent never confirms.
                        confirmed: false,
                    },
                    &ctx,
                )
                .await?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(outcome).map_err(failed)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_task_effort;
    use oxplow_domain::Actor;
    use serde_json::json;

    /// A task, and a lens over it with an action that finishes it.
    async fn fixture() -> (crate::test_fixtures::TaskEffortFixture, Actor) {
        let fx = services_with_task_effort().await;
        let ext = fx.svc.layout.project_dir.join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/tasks.yaml"),
            r#"title: Tasks
query: "SELECT kind FROM v_stream WHERE kind = :kind"
params: [{ name: kind, default: primary }, { name: item, default: "" }]
actions:
  - { id: finish, label: Finish, command: oxplow.work_item.transition, input: { ref: "{{param.item}}", to: done } }
"#,
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        (fx, actor)
    }

    /// A lens reads as text by default and as rows with `format: json`,
    /// with no record; an unknown lens or format is refused.
    #[tokio::test]
    async fn a_lens_runs_through_the_bus_as_text_or_rows() {
        let (fx, actor) = fixture().await;
        let run = |input: Value| {
            let svc = fx.svc.clone();
            let actor = actor.clone();
            async move { svc.commands.run(&actor, LENS_RUN, input, false).await }
        };
        let text = run(json!({ "id": "acme/tasks" })).await.unwrap();
        assert!(text.audit_id.is_none(), "a read leaves no record");
        assert!(text.result.get("result").is_none(), "{}", text.result);
        assert_eq!(text.result["rowCount"], 1);
        assert!(text.result["text"]
            .as_str()
            .unwrap()
            .contains("| primary |"));
        let rows = run(json!({ "id": "acme/tasks", "format": "json" }))
            .await
            .unwrap();
        assert_eq!(rows.result["result"]["rows"], json!([["primary"]]));
        for bad in [
            json!({ "id": "acme/nope" }),
            json!({ "id": "acme/tasks", "format": "xml" }),
            json!({ "id": "acme/tasks", "stream_id": "thr1" }),
        ] {
            let err = run(bad.clone()).await.unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{bad}: {err:?}"
            );
        }
    }

    /// An action runs its command as the lens acting for the calling
    /// agent: the task finishes and the run is audited as the lens, under
    /// the call's own record.
    #[tokio::test]
    async fn an_action_runs_as_the_lens_for_the_agent() {
        use oxplow_tasks::TaskStore as _;
        let (fx, actor) = fixture().await;
        let out = fx
            .svc
            .commands
            .run(
                &actor,
                LENS_RUN_ACTION,
                json!({
                    "id": "acme/tasks",
                    "action": "finish",
                    "params": { "item": oxplow_tasks::work_item_ref(fx.task) },
                }),
                false,
            )
            .await
            .unwrap();
        assert!(out.audit_id.is_some(), "an action is a recorded run");
        let task = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(task.status).unwrap().as_str(),
            Some("done")
        );
    }
}
