//! The commands a person runs an extension's effects with (P9.D4–D5,
//! `.context/extensions.md` → "Effects"). The live consumer
//! (`effect.triggers`) reacts to each event once, and only to events
//! logged after the effect's approval; everything else is a person's:
//!
//! - **`effect.retry`** — have an effect react again to an event its
//!   reaction to **failed**: the one way a failed reaction is ever
//!   attempted again;
//! - **`effect.backfill`** — have it react to matching events it never
//!   reacted to, whenever they were logged (before its approval, or while
//!   it waited to be re-approved): the one way the past is reacted to;
//! - **`effect.backfill_plan`** — a read: what a backfill would do.
//!
//! Both writes are a person's, asked first every time
//! (`Confirm::Always`), because of what they can do outside oxplow. A
//! failed attempt interrupted with an external step under way may have
//! landed that step, and a retry sends it again; a backfill runs an
//! effect that may call outside oxplow once per past event. No provider
//! promises a write is safe to repeat, so nothing retries by itself
//! (`.context/providers.md` "Idempotency").
//!
//! Each is an attempt recorded like any (`effect_run`, `effect.result@3 {
//! attempt, origin }`), run by [`run_reaction`] as the effect is **now**:
//! it must be enabled and approved as it is, it composes afresh from the
//! event, it passes the loop guard, and it counts toward the effect's
//! health — three failures in a row disable it, and stop a backfill.
//!
//! There is no consumer-level replay: core's consumers are re-derivable
//! (a projection is rebuilt, not replayed), and replaying the log through
//! every consumer would re-fire collectors noisily and effects without
//! consent. Reacting to the past exists only as `effect.backfill`.

use std::sync::{Arc, Weak};

use oxplow_db::effect_run_store::ReactionOrigin;
use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, DomainError, Invokers, Lifecycle,
    StoredEvent,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::{Command, Handler, HandlerOutput, Invocation};
use crate::effect_triggers::{self, run_reaction, Reacted};
use crate::effects::EffectDecl;
use crate::extensions::Extension;
use crate::plugin_health::PluginHealth;
use crate::Services;

pub const RETRY: &str = "effect.retry";
pub const BACKFILL: &str = "effect.backfill";
pub const BACKFILL_PLAN: &str = "effect.backfill_plan";

/// The most reactions one `effect.backfill` run makes; the rest are
/// `remaining`, for another run.
pub const BACKFILL_BATCH: usize = 200;
/// How many candidate events a backfill's plan reads at a time.
const SCAN_PAGE: usize = 1_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryInput {
    /// The effect: `<extension>/<id>`.
    pub effect: String,
    /// The event whose reaction failed: `event:<id>` (`v_effect_run`).
    pub event: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackfillInput {
    /// The effect: `<extension>/<id>`.
    pub effect: String,
    /// Only events at or after this log position — or `since`, not both.
    #[serde(default)]
    pub from_seq: Option<i64>,
    /// Only events logged at or after this time (RFC 3339).
    #[serde(default)]
    pub since: Option<String>,
    /// Only events at or before this log position.
    #[serde(default)]
    pub to_seq: Option<i64>,
}

/// Which of the log a backfill covers; everything, by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Range {
    pub from_seq: Option<i64>,
    /// RFC 3339, as the log stores times.
    pub since: Option<String>,
    pub to_seq: Option<i64>,
}

impl BackfillInput {
    fn range(&self) -> Result<Range, CommandError> {
        if self.from_seq.is_some() && self.since.is_some() {
            return Err(invalid(
                "/since",
                "say where it starts one way: `from_seq` or `since`, not both".into(),
            ));
        }
        let since = self
            .since
            .as_deref()
            .map(|s| {
                oxplow_domain::Timestamp::parse(s)
                    .map(|t| t.to_string())
                    .map_err(|e| invalid("/since", format!("`{s}` isn't an RFC 3339 time: {e}")))
            })
            .transpose()?;
        Ok(Range {
            from_seq: self.from_seq,
            since,
            to_seq: self.to_seq,
        })
    }
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

/// The enabled effect `name`, ready to run for a person: not disabled,
/// and approved as it is now.
async fn ready(
    svc: &Arc<Services>,
    name: &str,
) -> Result<(Extension, EffectDecl, PluginHealth), CommandError> {
    let (ext, decl) = effect_triggers::find_effect(svc, name).ok_or_else(|| {
        invalid(
            "/effect",
            format!("no enabled extension declares an effect `{name}`"),
        )
    })?;
    let health = PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
    if let Some(why) = health
        .disabled_reason(&effect_triggers::plugin_key(&decl))
        .await?
    {
        return Err(invalid(
            "/effect",
            format!(
                "effect `{name}` is disabled ({why}): enable it first (`plugin.enable`, \
                 Settings → Extensions)"
            ),
        ));
    }
    let program = crate::effects::effect_program(&ext, &decl);
    if !effect_triggers::approved_now(svc, &program) {
        return Err(invalid(
            "/effect",
            format!(
                "effect `{name}` isn't approved as it is now: a person approves it first \
                 (Settings → Data → Programs)"
            ),
        ));
    }
    Ok((ext, decl, health))
}

/// What a backfill would react to in a range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unreacted {
    /// How many there are.
    pub count: usize,
    /// The first few, oldest first: as many as were asked for.
    pub first: Vec<StoredEvent>,
    /// The log positions of the first and the last.
    pub from_seq: Option<i64>,
    pub to_seq: Option<i64>,
}

/// The events in `range` that `decl` reacts to (`on`, `where`, each read
/// at its type's newest version, as the pump delivers it) and has never
/// reacted to — counted, with the first `keep` of them, oldest first. One
/// its own run led to is never its trigger (the loop guard), so it is
/// never planned either (tsk846): an effect that changes what it reacts
/// to would otherwise meet its own changes on every backfill. The
/// candidates are read `page` at a time by log position, so `where` is
/// applied to every one in the range, not to a first window of them
/// (tsk848).
pub(crate) fn unreacted_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    decl: &EffectDecl,
    range: &Range,
    keep: usize,
    page: usize,
) -> Result<Unreacted, DomainError> {
    let types = vec!["?"; decl.on.len()].join(", ");
    let sql = format!(
        "SELECT e.id, e.seq FROM event_log e
          WHERE e.type IN ({types})
            AND e.seq >= ?{n1} AND e.seq <= ?{n2} AND e.at >= ?{n3}
            AND NOT EXISTS (SELECT 1 FROM effect_run r
                             WHERE r.effect = ?{n4} AND r.event_id = e.id)
          ORDER BY e.seq LIMIT {page}",
        n1 = decl.on.len() + 1,
        n2 = decl.on.len() + 2,
        n3 = decl.on.len() + 3,
        n4 = decl.on.len() + 4,
    );
    // What was logged after its approval is the live consumer's: a
    // backfill stops there, so the two never attempt one event at once
    // (tsk847). Never approved, it has nothing to backfill.
    let live_from = oxplow_db::effect_state_store::start_after_tx(tx, &decl.name())?.unwrap_or(-1);
    let to = range.to_seq.unwrap_or(i64::MAX).min(live_from);
    let own = format!("effect:{}", decl.name());
    let mut st = tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
    let mut out = Unreacted::default();
    let mut from = range.from_seq.unwrap_or(0);
    loop {
        let mut params: Vec<rusqlite::types::Value> = decl
            .on
            .iter()
            .map(|t| rusqlite::types::Value::Text(t.clone()))
            .collect();
        params.push(from.into());
        params.push(to.into());
        params.push(range.since.clone().unwrap_or_default().into());
        params.push(decl.name().into());
        let candidates: Vec<(String, i64)> = st
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(oxplow_db::map_sql_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(oxplow_db::map_sql_err)?;
        let Some(&(_, last)) = candidates.last() else {
            break;
        };
        for (id, seq) in &candidates {
            let Some(stored) =
                oxplow_db::event_log_store::get_tx(tx, &oxplow_domain::EventId(id.clone()))?
            else {
                continue;
            };
            // One that can't be carried to today's shape isn't reacted to.
            let Ok(event) = crate::event_pump::at_latest(vocabulary, &stored) else {
                continue;
            };
            if !crate::effects::reacts_to(decl, &event.envelope.event_type, &event.envelope.payload)
            {
                continue;
            }
            if crate::event_lineage::lineage_tx(tx, stored, &own)?.own {
                continue;
            }
            out.count += 1;
            out.from_seq.get_or_insert(*seq);
            out.to_seq = Some(*seq);
            if out.first.len() < keep {
                out.first.push(event);
            }
        }
        if candidates.len() < page {
            break;
        }
        from = last + 1;
    }
    Ok(out)
}

/// What a backfill run came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backfilled {
    /// The matching events the effect had never reacted to.
    pub planned: usize,
    pub ran: usize,
    pub skipped: usize,
    pub proposed: usize,
    pub failed: usize,
    /// Planned and not attempted: past this run's batch, or after it
    /// stopped.
    pub remaining: usize,
    /// Why it stopped before its batch was done.
    pub stopped: Option<String>,
}

/// Have `decl` react to the events in `range` it never reacted to, oldest
/// first, at most `batch` of them — each an attempt like a live one
/// (deduped, loop-guarded, counted toward its health). It stops when the
/// effect is disabled (three failures in a row).
pub async fn backfill(
    svc: &Arc<Services>,
    ext: &Extension,
    decl: &EffectDecl,
    range: &Range,
    batch: usize,
) -> Result<Backfilled, CommandError> {
    let health = PluginHealth::new(svc.db.clone(), svc.vocabulary.clone());
    let key = effect_triggers::plugin_key(decl);
    let planned = {
        let (vocabulary, decl, range) = (svc.vocabulary.current(), decl.clone(), range.clone());
        svc.db
            .read(move |tx| unreacted_tx(tx, &vocabulary, &decl, &range, batch, SCAN_PAGE))
            .await?
    };
    let mut out = Backfilled {
        planned: planned.count,
        ..Backfilled::default()
    };
    let mut attempted = 0;
    for event in &planned.first {
        if let Some(why) = health.disabled_reason(&key).await? {
            out.stopped = Some(format!("the effect was disabled: {why}"));
            break;
        }
        let started = std::time::Instant::now();
        let reacted = run_reaction(svc, ext, decl, true, event, ReactionOrigin::Backfill).await?;
        effect_triggers::count(&health, decl, &reacted, started.elapsed()).await;
        attempted += 1;
        match reacted {
            Reacted::Ran => out.ran += 1,
            Reacted::Failed(_) => out.failed += 1,
            Reacted::Proposed => out.proposed += 1,
            // Its own run's event, or one another delivery got to first.
            Reacted::Skipped(_) | Reacted::Nothing => out.skipped += 1,
        }
    }
    out.remaining = out.planned - attempted;
    Ok(out)
}

const BACKFILL_SUMMARY: &str = "Have an effect react to the matching events it never reacted to \
     — those logged before its approval, or while it waited to be approved again — oldest \
     first, once each. The effect may call outside oxplow for every one of them. It runs as it \
     is now; `effect.backfill_plan` says how many events that is.";

/// `effect.backfill { effect, from_seq? | since?, to_seq? }`.
pub fn backfill_command(services: Weak<Services>) -> Command {
    Command::new(
        CommandSpec {
            name: BACKFILL.into(),
            summary: BACKFILL_SUMMARY.into(),
            input_schema: serde_json::to_value(schemars::schema_for!(BackfillInput))
                .expect("schema serializes"),
            invokers: Invokers::HUMAN_ONLY,
            confirm: Confirm::Always,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::External,
            effect: CommandEffect::Write,
        },
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: BackfillInput =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let range = input.range()?;
                let svc = services.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "oxplow is shutting down".into(),
                })?;
                let (ext, decl, _) = ready(&svc, &input.effect).await?;
                let done = backfill(&svc, &ext, &decl, &range, BACKFILL_BATCH).await?;
                let mut result = json!({
                    "effect": input.effect,
                    "planned": done.planned,
                    "ran": done.ran,
                    "skipped": done.skipped,
                    "proposed": done.proposed,
                    "failed": done.failed,
                    "remaining": done.remaining,
                });
                if let Some(why) = done.stopped {
                    result["stopped"] = json!(why);
                }
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("effect.backfill is a valid command")
}

/// `effect.backfill_plan { effect, from_seq? | since?, to_seq? }`: what
/// `effect.backfill` would react to — a read.
pub fn backfill_plan_command(services: Weak<Services>) -> Command {
    Command::new(
        CommandSpec {
            name: BACKFILL_PLAN.into(),
            summary: "How many events an `effect.backfill` would have an effect react to: the \
                      matching ones it never reacted to, the log positions they span (pass \
                      `to_seq` to the backfill to run on just these), and how many one run \
                      reacts to (`batch`)."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(BackfillInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Read,
        },
        Handler::Tx(Arc::new(move |ctx, input| {
            let input: BackfillInput =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            let range = input.range()?;
            let svc = services.upgrade().ok_or_else(|| CommandError::Failed {
                message: "oxplow is shutting down".into(),
            })?;
            let (_, decl) = effect_triggers::find_effect(&svc, &input.effect).ok_or_else(|| {
                invalid(
                    "/effect",
                    format!("no enabled extension declares an effect `{}`", input.effect),
                )
            })?;
            let planned =
                unreacted_tx(ctx.conn, ctx.events.vocabulary, &decl, &range, 0, SCAN_PAGE)?;
            Ok(HandlerOutput {
                // `batch`: one `effect.backfill` reacts to at most that
                // many; pass `to_seq` to it to run on just this range.
                result: json!({
                    "effect": input.effect,
                    "planned": planned.count,
                    "from_seq": planned.from_seq,
                    "to_seq": planned.to_seq,
                    "batch": BACKFILL_BATCH,
                }),
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("effect.backfill_plan is a valid command")
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
        Handler::External(Arc::new(move |_: Invocation, input| {
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
                let (ext, decl, health) = ready(&svc, &effect).await?;
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
                // As the pump would hand it over: at its type's newest version.
                let stored = crate::event_pump::at_latest(&svc.vocabulary.current(), &stored)?;
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
                        .map_or(0, |latest| i64::from(latest.attempt))
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
