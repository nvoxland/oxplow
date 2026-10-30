//! An extension's effects (`effects:` in its manifest, P8.D9–D11; **stable
//! since P11**, tsk956; `.context/extensions-events.md` → "Effects"): a Starlark
//! script that reacts to a logged event by composing commands, run as
//! `Actor::Effect` with an agent's rights.
//!
//! ```yaml
//! effects:
//!   - id: announce-done          # [a-z0-9-]+, unique in the extension
//!     summary: Note a finished item on its thread.
//!     on: [work_item.state_changed]
//!     where: { to: done }        # optional: payload fields equal to these
//!     needs: [sql.read]              # the scopes its script calls
//!     entry: effects/announce.star   # transform({event}) → {commands, events?} | {skip}
//!     after: [page_ref.work_item]    # optional: what it runs after on one event
//! ```
//!
//! It starts when a generation that runs it first publishes — its
//! extension deployed (or a bundled one shipping it) — at the log's head
//! (`effect_state`): an effect never reacts to what happened before it
//! started. What it may compose and read is its extension's `permissions:`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::extension::extensions::manifest_v2::{at, item_lines, key_line};

/// An effect as loaded (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectDecl {
    pub id: String,
    pub extension: String,
    pub summary: String,
    /// The event types it reacts to.
    pub on: Vec<String>,
    /// Payload fields that must equal these values (the collectors'
    /// `where`).
    pub filter: BTreeMap<String, String>,
    /// The scopes its script calls (`scope(id, args)`,
    /// `.context/commands.md` "Scopes"): `sql.read`.
    pub needs: Vec<String>,
    /// The script's path in the folder.
    pub entry: String,
    /// Its source.
    pub script: String,
    /// What it runs after on one event, as written (resolved to refs on
    /// its descriptor; `extensions-events.md` "Ordering").
    pub after: Vec<String>,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

impl EffectDecl {
    /// `<extension>/<id>`: its actor (`Actor::Effect`), approval and state.
    pub fn name(&self) -> String {
        format!("{}/{}", self.extension, self.id)
    }
}

/// An `effects:` entry as the manifest holds it; `on` and `where` are read
/// by the one trigger grammar (`collectors::parse_trigger`), as a
/// collector's are.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectFile {
    id: String,
    summary: String,
    /// The event types it reacts to.
    #[schemars(with = "oxplow_config::collectors::EventTypeList")]
    on: serde_yaml::Value,
    /// Payload fields and the values they must equal.
    #[serde(default, rename = "where")]
    #[schemars(with = "Option<oxplow_config::collectors::PayloadFilter>")]
    filter: Option<serde_yaml::Value>,
    #[serde(default)]
    needs: Vec<String>,
    entry: String,
    #[serde(default)]
    after: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    oxplow_domain::contribution::valid_local(
        oxplow_domain::contribution::ContributionKind::Effect,
        id,
    )
}

/// Parse an `effects:` block: each effect's trigger (`on`, `where`, by the
/// collectors' rules, against `knows_event`), its script (in the folder,
/// defining `transform`); what's wrong is `file:line`.
pub fn parse_effects(
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
    knows_event: &dyn Fn(&str) -> bool,
) -> (Vec<EffectDecl>, Vec<String>) {
    let block = key_line(manifest, "effects");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`effects` must be a list")],
        );
    };
    let mut out: Vec<EffectDecl> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "effects");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let f: EffectFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, item_line, format!("effect: {e}")));
                continue;
            }
        };
        let line = item_line;
        let declared_at = at(file, line, "").trim_end_matches(": ").to_string();
        match decl_of(extension, f, read, knows_event, declared_at) {
            Ok(d) if out.iter().any(|o| o.id == d.id) => errors.push(at(
                file,
                line,
                format!("effect `{}` is declared twice", d.id),
            )),
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn decl_of(
    extension: &str,
    f: EffectFile,
    read: &dyn Fn(&str) -> Option<String>,
    knows_event: &dyn Fn(&str) -> bool,
    declared_at: String,
) -> Result<EffectDecl, String> {
    if !valid_id(&f.id) {
        return Err(format!(
            "effect id `{}` must be lowercase letters, digits and dashes",
            f.id
        ));
    }
    let named = |m: String| format!("effect `{}`: {m}", f.id);
    // The collectors' grammar for `on`/`where`; an effect reacts to each
    // event, so it has no clock, no report run and no pacing.
    let (on, filter) = match oxplow_config::collectors::parse_trigger(
        oxplow_config::collectors::TriggerKeys {
            on: Some(&f.on),
            filter: f.filter.as_ref(),
            ..Default::default()
        },
        knows_event,
    )
    .map_err(named)?
    {
        oxplow_config::collectors::Trigger::On { events, filter, .. } => (events, filter),
        _ => return Err(named("`on` lists the event types it reacts to".into())),
    };
    for need in &f.needs {
        crate::scope_calls::check_script_need(need).map_err(&named)?;
    }
    let script = read(&f.entry)
        .ok_or_else(|| named(format!("entry `{}` isn't in the extension", f.entry)))?;
    oxplow_script::runtime::check_starlark(&f.entry, &script)
        .map_err(|e| named(format!("`{}` {e}", f.entry)))?;
    crate::services::check_uses(&script, &f.needs, crate::scope_calls::check_script_need)
        .map_err(|e| named(format!("`{}` {e}", f.entry)))?;
    Ok(EffectDecl {
        id: f.id,
        extension: extension.to_string(),
        summary: f.summary,
        on,
        filter,
        needs: f.needs,
        entry: f.entry,
        script,
        after: f.after,
        declared_at,
    })
}

/// Whether an effect may react to the event at `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// It hasn't started: no deployment of it has run yet.
    NotStarted,
    /// The event was logged before (or at) it started.
    BeforeStart,
    Runs,
}

/// Decide [`Gate`] at event `seq`, given where the effect starts
/// (`effect_state`: set when a generation with it first publishes).
pub fn gate(start_after: Option<i64>, seq: i64) -> Gate {
    match start_after {
        None => Gate::NotStarted,
        Some(start) if seq <= start => Gate::BeforeStart,
        Some(_) => Gate::Runs,
    }
}

/// Start effect `name` after the log's head now, dropping its scheduled
/// automatic retries (tsk990): what a test does to have it react to what
/// it logs next. A generation that first runs an effect starts it in its
/// publication ([`oxplow_db::effect_state_store::start_if_new_tx`]).
pub async fn start_at_head(
    db: &oxplow_db::Database,
    name: &str,
) -> Result<i64, oxplow_domain::DomainError> {
    let (name, now) = (
        name.to_string(),
        oxplow_domain::Timestamp::now().to_string(),
    );
    db.transaction(move |tx| {
        oxplow_db::effect_run_store::drop_retries_of_tx(tx, &name)?;
        oxplow_db::effect_state_store::start_at_head_tx(tx, &name, &now)
    })
    .await
}

/// Where effect `name` starts reading the log; `None` before it started.
pub async fn start_after(
    db: &oxplow_db::Database,
    name: &str,
) -> Result<Option<i64>, oxplow_domain::DomainError> {
    let name = name.to_string();
    db.read(move |tx| oxplow_db::effect_state_store::start_after_tx(tx, &name))
        .await
}

/// What an effect's script decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Reaction {
    /// Nothing to do, and why.
    Skip(String),
    /// Run these commands, and append these of its extension's events.
    Run {
        calls: Vec<oxplow_domain::CommandCall>,
        events: Vec<crate::extension::extension_commands::ComposedEvent>,
    },
}

impl Reaction {
    /// The names of the commands it runs.
    pub fn command_names(&self) -> Vec<String> {
        match self {
            Reaction::Skip(_) => Vec::new(),
            Reaction::Run { calls, .. } => calls.iter().map(|c| c.name.clone()).collect(),
        }
    }
}

/// `{ skip: "why" }` or `{ commands: [{ name, input }], events?: [...] }`.
pub fn reaction(value: serde_json::Value) -> Result<Reaction, String> {
    use serde_json::{json, Value};
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Out {
        #[serde(default)]
        commands: Option<Vec<Call>>,
        #[serde(default)]
        events: Vec<crate::extension::extension_commands::ComposedEvent>,
        #[serde(default)]
        skip: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Call {
        name: String,
        #[serde(default = "empty_object")]
        input: Value,
    }
    fn empty_object() -> Value {
        json!({})
    }
    const SHAPE: &str = "an effect's script returns `{ commands: [{ name, input }], events? }` \
                         or `{ skip: \"why\" }`";
    let out: Out = serde_json::from_value(value).map_err(|e| format!("{SHAPE}: {e}"))?;
    match (out.skip, out.commands) {
        (Some(why), None) if out.events.is_empty() => Ok(Reaction::Skip(why)),
        (Some(_), _) => Err(format!("{SHAPE}: a skip composes nothing")),
        (None, None) => Err(format!("{SHAPE}: it returned neither")),
        (None, Some(commands)) => Ok(Reaction::Run {
            calls: commands
                .into_iter()
                .map(|c| oxplow_domain::CommandCall {
                    name: c.name,
                    input: c.input,
                })
                .collect(),
            events: out.events,
        }),
    }
}

/// Whether `decl` reacts to an event of `event_type` with `payload`: its
/// `on` names the type and its `where` matches.
pub fn reacts_to(decl: &EffectDecl, event_type: &str, payload: &serde_json::Value) -> bool {
    oxplow_config::collectors::fires(&decl.on, &decl.filter, event_type, payload)
}

/// Run `script` over `{ event }`, sandboxed with a command script's budget,
/// with the services it was `given` (`services::prepare`: it composes, so
/// never a model), its `scope` calls answered by `calls`. Blocks: call it
/// off the async runtime.
pub fn run_script(
    script: &str,
    event: serde_json::Value,
    given: crate::services::Given,
    calls: &mut crate::scope_calls::Calls<'_>,
) -> Result<Reaction, String> {
    let run = calls.run().clone();
    let mut serve = |id: &str, args: serde_json::Value| calls.serve(id, args);
    let out = crate::extension::extension_commands::run_command_script(
        script,
        serde_json::json!({ "event": event }),
        given,
        &mut serve,
        &run,
    )
    .map_err(|e| format!("the script failed: {e}"))?;
    reaction(out)
}

/// What `decl` would do with `event` (the event envelope, a fixture's
/// or a logged one's) — its script, its scope calls answered from
/// `answers` or for real (reads through `layer`), the commands it composes
/// checked against `registry` — running nothing: `extension test` and a
/// change's review.
pub async fn dry_run(
    layer: &crate::sql_gateway::SqlGateway,
    decl: &EffectDecl,
    script: &str,
    event: serde_json::Value,
    answers: &BTreeMap<String, Vec<serde_json::Value>>,
    registry: Option<crate::extension::extensions::CommandSchemas<'_>>,
) -> Result<Reaction, String> {
    let run = oxplow_domain::run::RunContext::core(
        oxplow_domain::Actor::System,
        oxplow_domain::run::Mode::Pure,
    );
    let reaction = run_over(
        layer,
        &run,
        &decl.needs,
        script,
        event,
        Some(answers),
        &crate::services::Sources::default(),
    )
    .await?;
    if let (Some(registry), Reaction::Run { calls, .. }) = (registry, &reaction) {
        crate::extension::extension_commands::check_calls(registry, calls)?;
    }
    Ok(reaction)
}

/// `script` (an effects-shaped `transform`) over `event`, its scope calls
/// limited to `needs` and answered from `answers` (a fixture's, in call
/// order) or for real (reads through `layer`), the other services it may
/// use built from `sources`: what it composes, nothing run. Off the async
/// runtime, under the effects' budget. An effect's dry run and an effort
/// policy written as a script both run through it.
pub async fn run_over(
    layer: &crate::sql_gateway::SqlGateway,
    run: &oxplow_domain::run::RunContext,
    needs: &[String],
    script: &str,
    event: serde_json::Value,
    answers: Option<&BTreeMap<String, Vec<serde_json::Value>>>,
    sources: &crate::services::Sources,
) -> Result<Reaction, String> {
    let given = crate::services::prepare(run, script, needs, sources).await?;
    let (script, needs) = (script.to_string(), needs.to_vec());
    let (layer, answers, run) = (layer.clone(), answers.cloned(), run.clone());
    // A revocation reaches a script that is computing (no host call to notice by).
    let _watch = crate::run_context::watch_authority(&run);
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(layer.run(q)));
        let calls =
            crate::scope_calls::Calls::new(&run, &needs, read).with_host(Some(&layer), &runtime);
        let mut calls = match &answers {
            Some(answers) => calls.with_answers(answers),
            None => calls,
        };
        run_script(&script, event, given, &mut calls)
    })
    .await
    .map_err(|e| format!("the script's worker failed: {e}"))?
}

/// What a reaction that another delivery already recorded answers (the
/// bus's lost race, `RunOrigin::Effect`): not a failure of the effect.
pub const ALREADY_REACTED: &str = "already reacted to event";

/// How a reaction whose commands ran ended: `ok`, or `failed` with the
/// step error when a step outside the transaction failed partway.
pub fn ran(audit_id: i64, failed: Option<String>) -> oxplow_db::effect_run_store::Finished {
    use oxplow_db::effect_run_store::{Finished, RunState};
    Finished {
        state: if failed.is_some() {
            RunState::Failed
        } else {
            RunState::Ok
        },
        reason: failed,
        audit_id: Some(audit_id),
        proposal_id: None,
    }
}

/// An attempt at an effect's reaction to one event ended: its
/// `effect_run` row (its `started` claim finished) and `effect.result@4`
/// (which attempt, started by what), from the effect,
/// about the event and the extension, caused by `cause` (its run's
/// `command.executed`, its proposal's `command.proposed`; none when no
/// command ran). `Invalid` when the reaction already ended.
pub fn finished_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    key: &oxplow_db::effect_run_store::EffectRunKey,
    done: &oxplow_db::effect_run_store::Finished,
    cause: Option<oxplow_domain::EventId>,
) -> Result<(), oxplow_domain::DomainError> {
    use oxplow_db::effect_run_store::ReactionOrigin;
    use oxplow_db::effect_run_store::RunState;
    use oxplow_domain::events::schema::{
        EffectOrigin, EffectOutcome, EffectResult, EffectResultV5,
    };
    let now = oxplow_domain::Timestamp::now().to_string();
    oxplow_db::effect_run_store::finish_tx(tx, key, done, &now)?;
    let outcome = match done.state {
        RunState::Ok => EffectOutcome::Ok,
        RunState::Skipped => EffectOutcome::Skipped,
        RunState::Proposed => EffectOutcome::Proposed,
        RunState::Started | RunState::Failed => EffectOutcome::Failed,
    };
    let extension = key.effect.split('/').next().unwrap_or_default();
    let event_ref = oxplow_domain::uri::build::event_uri(&key.event_id.to_string());
    let mut result = oxplow_domain::Envelope::typed::<EffectResult>(
        oxplow_domain::uri::build::effect_source(&key.effect),
        &EffectResultV5 {
            effect: key.effect.clone(),
            event: Some(key.event_id.to_string()),
            outcome,
            reason: done.reason.clone(),
            proposal: done.proposal_id,
            detail: serde_json::Value::Null,
            attempt: key.attempt,
            origin: match key.origin {
                ReactionOrigin::Live => EffectOrigin::Live,
                ReactionOrigin::Retry => EffectOrigin::Retry,
                ReactionOrigin::Backfill => EffectOrigin::Backfill,
                ReactionOrigin::Auto => EffectOrigin::Auto,
            },
        },
    )
    .with_subject([
        event_ref,
        oxplow_domain::uri::build::extension_uri(extension),
    ]);
    result.cause = cause;
    oxplow_db::event_log_store::append_tx(tx, vocabulary, &result)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {

    /// What a scheduled retry would send was composed by the script as it
    /// was when it failed. Starting an effect drops its scheduled retries —
    /// the reactions stay failed, for a person's Retry, which composes
    /// afresh.
    #[tokio::test]
    async fn starting_an_effect_drops_its_scheduled_retries() {
        use oxplow_db::effect_run_store as runs;
        let db = oxplow_db::Database::in_memory();
        let key = runs::EffectRunKey::first("acme/notify", "e1", 3, runs::ReactionOrigin::Live);
        let other = runs::EffectRunKey::first("acme/other", "e1", 3, runs::ReactionOrigin::Live);
        db.transaction({
            let (key, other) = (key.clone(), other.clone());
            move |tx| {
                for k in [&key, &other] {
                    runs::claim_tx(tx, k, "2026-10-01T00:00:00Z", Some("{}"))?;
                    runs::finish_tx(
                        tx,
                        k,
                        &runs::Finished {
                            state: runs::RunState::Failed,
                            reason: Some("unavailable".into()),
                            audit_id: None,
                            proposal_id: None,
                        },
                        "2026-10-01T00:00:01Z",
                    )?;
                    runs::schedule_retry_tx(tx, k, "2026-10-01T00:00:10Z", "{}")?;
                }
                Ok(())
            }
        })
        .await
        .unwrap();
        start_at_head(&db, "acme/notify").await.unwrap();
        let due = db
            .read(|tx| runs::due_retries_tx(tx, "2026-10-02T00:00:00Z"))
            .await
            .unwrap();
        assert_eq!(
            due.into_iter().map(|(k, _)| k.effect).collect::<Vec<_>>(),
            vec!["acme/other".to_string()],
            "only the started effect's retry is dropped"
        );
    }

    use super::*;
    use crate::extension::extensions::{load_extensions, Extension};
    use std::path::Path;

    pub(crate) const MANIFEST: &str = "manifest: 2
name: acme
intent: { purpose: Effects., origin: null, examples: [] }
effects:
  - id: announce-done
    summary: Note a finished item.
    on: [work_item.state_changed]
    where: { to: done }
    entry: effects/announce.star
";

    pub(crate) const SCRIPT: &str = "def transform(x):\n    return {\"skip\": \"nothing to do\"}\n";

    pub(crate) fn write_acme(root: &Path, manifest: &str, script: &str) {
        let dir = root.join(".oxplow/extensions/acme/effects");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            root.join(".oxplow/extensions/acme/extension.yaml"),
            manifest,
        )
        .unwrap();
        std::fs::write(dir.join("announce.star"), script).unwrap();
    }

    fn acme(root: &Path) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap()
    }

    #[test]
    fn an_effect_loads_with_its_trigger_and_script() {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), MANIFEST, SCRIPT);
        let ext = acme(dir.path());
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let e = &ext.effects[0];
        assert_eq!(e.name(), "acme/announce-done");
        assert_eq!(e.on, vec!["work_item.state_changed".to_string()]);
        assert_eq!(e.filter.get("to").map(String::as_str), Some("done"));
        assert_eq!(e.declared_at, ".oxplow/extensions/acme/extension.yaml:5");
    }

    #[test]
    fn a_broken_effect_is_an_error_at_its_line() {
        for (from, to, says) in [
            (
                "on: [work_item.state_changed]",
                // A core namespace's type that doesn't exist (another
                // extension's would be a subscription: P9.D1).
                "on: [work_item.never]",
                "isn't a registered event type",
            ),
            (
                "entry: effects/announce.star",
                "entry: effects/gone.star",
                "isn't in the extension",
            ),
            ("id: announce-done", "id: Announce", "lowercase"),
            // A scope a script can't call is refused at load, not when the
            // call fails.
            (
                "entry: effects/announce.star",
                "entry: effects/announce.star\n    needs: [threads.write]",
                "`threads.write` is a scope a script can't use",
            ),
            // An effect's script composes: it may read, never ask a model.
            (
                "entry: effects/announce.star",
                "entry: effects/announce.star\n    needs: [ai.compute]",
                "this script composes, so it runs pure",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_acme(dir.path(), &MANIFEST.replace(from, to), SCRIPT);
            let ext = acme(dir.path());
            assert!(ext.effects.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains("extension.yaml:5:") && errors.contains(says),
                "{to}: {errors}"
            );
        }
    }

    async fn gate_at(svc: &crate::Services, seq: i64) -> Gate {
        let start = start_after(&svc.db, "acme/announce-done").await.unwrap();
        gate(start, seq)
    }

    /// An effect starts when the generation that runs it first publishes —
    /// its extension's deploy — at the log's head: it never reacts to what
    /// happened before. A later deploy of it keeps where it started.
    #[tokio::test]
    async fn an_effect_reacts_only_to_events_after_its_deployment() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write_acme(&root, MANIFEST, SCRIPT);
        let log = || async {
            svc.event_log_store
                .append(
                    oxplow_domain::Envelope::new(
                        "config.changed",
                        2,
                        "test",
                        serde_json::json!({ "key": "zones", "before": null, "after": [], "layer": "project" }),
                    )
                    .unwrap(),
                )
                .await
                .unwrap()
        };
        let before = log().await;
        assert_eq!(gate_at(svc, before).await, Gate::NotStarted);

        crate::extension::extension_deploy::deploy(svc, "acme", true)
            .await
            .unwrap();
        assert_eq!(gate_at(svc, before).await, Gate::BeforeStart);
        let after = log().await;
        assert_eq!(gate_at(svc, after).await, Gate::Runs);

        write_acme(&root, MANIFEST, &format!("{SCRIPT}# edited\n"));
        crate::extension::extension_deploy::deploy(svc, "acme", true)
            .await
            .unwrap();
        assert_eq!(gate_at(svc, after).await, Gate::Runs);
    }
}
