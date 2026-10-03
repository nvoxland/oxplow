//! `effect.retry` (P9.D4, `.context/extensions.md` → "Effects"): a
//! person has an effect react again to an event its reaction to **failed**
//! — the one way a failed reaction is ever attempted again.
//!
//! It is a person's, asked first every time (`Confirm::Always`), because
//! of what a failure can hide: a reaction interrupted with a step outside
//! oxplow under way may have landed that step, and running it again sends
//! it twice. No provider promises a write is safe to repeat, so nothing
//! retries by itself (`.context/providers.md` "Idempotency").
//!
//! The retry is the reaction's next attempt (`effect_run.attempt`,
//! `effect.result@3 { attempt, origin: retry }`), run by
//! [`run_reaction`] as the effect is **now**: it must be enabled and
//! approved as it is, and it composes afresh from the event — what it
//! runs is what its script says today, not what the failed attempt
//! composed.

use std::sync::{Arc, Weak};

use oxplow_db::effect_run_store::ReactionOrigin;
use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, DomainError, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::{Command, Handler, HandlerOutput};
use crate::effect_triggers::{self, run_reaction, Reacted};
use crate::Services;

pub const RETRY: &str = "effect.retry";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryInput {
    /// The effect: `<extension>/<id>`.
    pub effect: String,
    /// The event whose reaction failed: `event:<id>` (`v_effect_run`).
    pub event: String,
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

/// What an attempt came to, as a command's result.
fn outcome(effect: &str, event: &str, attempt: i64, reacted: &Reacted) -> serde_json::Value {
    let mut out = json!({ "effect": effect, "event": event, "attempt": attempt });
    let (outcome, reason) = match reacted {
        Reacted::Ran => ("ok", None),
        Reacted::Failed(why) => ("failed", Some(why.clone())),
        Reacted::Skipped(why) => ("skipped", Some(why.clone())),
        Reacted::Proposed => ("proposed", None),
        Reacted::Nothing => ("nothing", None),
    };
    out["outcome"] = json!(outcome);
    if let Some(reason) = reason {
        out["reason"] = json!(reason);
    }
    out
}

/// `effect.retry { effect, event }`.
pub fn retry_command(services: Weak<Services>) -> Command {
    Command::new(
        CommandSpec {
            name: RETRY.into(),
            summary: "Have an effect react again to an event its reaction to failed, as its \
                      next attempt. If the failed attempt was interrupted with a step outside \
                      oxplow under way, that step may already have run: retrying sends it \
                      again. It runs the effect as it is now, composing afresh from the event."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(RetryInput))
                .expect("schema serializes"),
            invokers: Invokers::HUMAN_ONLY,
            confirm: Confirm::Always,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::External,
            effect: CommandEffect::Write,
        },
        Handler::External(Arc::new(move |_actor, input| {
            let services = services.clone();
            Box::pin(async move {
                let RetryInput { effect, event } =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let svc = services.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "oxplow is shutting down".into(),
                })?;
                let (ext, decl) = effect_triggers::find_effect(&svc, &effect).ok_or_else(|| {
                    invalid(
                        "/effect",
                        format!("no enabled extension declares an effect `{effect}`"),
                    )
                })?;
                let id = event.strip_prefix("event:").ok_or_else(|| {
                    invalid(
                        "/event",
                        format!("`{event}` isn't an event ref (`event:<id>`)"),
                    )
                })?;
                let stored = svc
                    .event_log_store
                    .get(oxplow_domain::EventId(id.to_string()))
                    .await?
                    .ok_or_else(|| invalid("/event", format!("no event `{event}` in the log")))?;
                let health =
                    crate::plugin_health::PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
                if let Some(why) = health
                    .disabled_reason(&effect_triggers::plugin_key(&decl))
                    .await?
                {
                    return Err(invalid(
                        "/effect",
                        format!(
                            "effect `{effect}` is disabled ({why}): enable it first \
                             (`plugin.enable`, Settings → Extensions)"
                        ),
                    ));
                }
                let program = crate::effects::effect_program(&ext, &decl);
                if !effect_triggers::approved_now(&svc, &program) {
                    return Err(invalid(
                        "/effect",
                        format!(
                            "effect `{effect}` isn't approved as it is now: a person approves \
                             it first (Settings → Data → Programs)"
                        ),
                    ));
                }
                let started = std::time::Instant::now();
                let reacted = run_reaction(&svc, &ext, &decl, true, &stored, ReactionOrigin::Retry)
                    .await
                    .map_err(|e| match e {
                        DomainError::Invalid(message) => invalid("/event", message),
                        other => CommandError::from(other),
                    })?;
                effect_triggers::count(&health, &decl, &reacted, started.elapsed()).await;
                let attempt = {
                    let (effect, id) = (effect.clone(), id.to_string());
                    svc.db
                        .read(move |tx| oxplow_db::effect_run_store::latest_tx(tx, &effect, &id))
                        .await?
                        .map_or(0, |(attempt, _)| i64::from(attempt))
                };
                Ok(HandlerOutput {
                    result: outcome(&effect, &event, attempt, &reacted),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("effect.retry is a valid command")
}
