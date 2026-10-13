//! `oxplow extension test` (P5.D5, P7.C6; `.context/extensions.md` "The
//! SDK"; `.context/providers.md` "The conformance kit"). Everything runs
//! on a throwaway oxplow — an in-memory one over a copy of the project's
//! `oxplow/extensions/` ([`Host`]) — so it never touches the project's
//! data: `check` (against its real command registry, so `commands:`
//! examples dry-run), then each intent example whose fixture names a lens
//! (`input: { lens, params? }`, `expect: { columns?, rows }`), a derived
//! collector (`input: { collector, rows? }`, `expect: { entities: { <name>:
//! n }, events?: [{ type, payload? }] }`; one that calls a model runs against the fixture model, approved in the throwaway; an exec collector's isn't run — it needs a person's approval),
//! a fact collector (`input: { collector, files: { <path>: <text> } }`,
//! `expect: { facts: [{ measure, value?, path?, … }] }`, tsk1047),
//! or one of its own commands (`input: { command, input, rows? }`,
//! `expect: { commands: [names] }` and/or `{ result }` — only the keys it
//! names — or `{ refuses }`, dry-run),
//! the extension's `questions.yaml`, and for each declared provider —
//!
//! 1. the live `initialize` equals its declarations file;
//! 2. `check` accepts the instance config in
//!    `fixtures/provider-<id>.yaml` (`config: { … }`);
//! 3. each `intent.examples[*]` with a fixture `fixtures/<name>.yaml`
//!    (`input: { command, input }`, `expect`) is invoked and its result
//!    matches `expect` (`$any` matches anything);
//! 4. every message validates against the protocol's schema goldens, and
//!    the session's transcript matches `fixtures/transcripts/<id>.jsonl`
//!    (`--bless` writes it);
//! 5. its capability's conformance suite passes through a throwaway host
//!    (an in-memory oxplow over a copy of the extension).
//!
//! Every finding is a `file:line: what — fix` line.

use oxplow_ai::fixture::FixtureReply;
use oxplow_domain::events::EventView;
use std::path::Path;
use std::sync::Arc;

use oxplow_app::extensions::{Extension, EXTENSIONS_DIR};
use oxplow_app::providers::host::{self, Launch};
use oxplow_app::providers::{self, ProviderSpec};
use oxplow_domain::Actor;
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, InitializeResult, InvokeParams, InvokeResult,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::conformance::{first_mismatch, normalize, ReferenceClient};
use crate::throwaway::Host;
use crate::SdkError;

/// The capability a policy script implements.
const EFFORT_POLICY: &str = providers::effort_policy::EffortPolicyHost::CAPABILITY;

/// What `extension test` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub name: String,
    /// No errors.
    pub ok: bool,
    /// `file:line: what — fix` lines.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Golden transcripts written by `--bless`.
    pub blessed: Vec<String>,
    /// What ran: `check`, `provider <id>`, `<capability> suite`.
    pub ran: Vec<String>,
    /// How each of its `imports:` resolved here (`crate::import_lines`).
    pub imports: Vec<String>,
    /// What serves each of its `ai_profiles:` (`crate::profile_lines`).
    pub profiles: Vec<String>,
    /// The throwaway's activation generation the examples ran against.
    pub generation: Option<String>,
    /// What the extension contributes, by public id
    /// (its ref (`<kind>:<extension>/<id>`, `command:<bus id>`)).
    pub contributes: Vec<String>,
    /// What it contributes that this host can't run — a person sees it in
    /// the app (pages, panels, slots, decorators, replacements, skills).
    pub unsupported: Vec<String>,
    /// What the run made in a provider's own system and didn't remove —
    /// the refs a provider example's `create` returned, and the items the
    /// conformance suite filed and couldn't delete. Against a real
    /// service, a person (or a live test) cleans up exactly these.
    pub left: Vec<String>,
}

/// Run every test of extension `name` under `root`. `bless` writes the
/// golden transcripts instead of comparing them. A provider's credentials
/// and declared `env` come from this process's environment, as for a
/// person running `oxplow extension test`.
pub async fn test_extension(root: &Path, name: &str, bless: bool) -> Result<TestReport, SdkError> {
    test_extension_in(root, name, bless, host::process_env()).await
}

/// [`test_extension`] in the environment `env` names, not this process's:
/// for a caller that runs several at once (tests), whose environments
/// mustn't meet.
pub async fn test_extension_in(
    root: &Path,
    name: &str,
    bless: bool,
    env: host::HostEnv,
) -> Result<TestReport, SdkError> {
    let host = Host::start(root, env.clone())
        .await
        .map_err(SdkError::Invalid)?;
    host.start_runtime().map_err(SdkError::Invalid)?;
    let checked = crate::check(
        &host.root,
        name,
        &host.svc.extension_catalog,
        Some(&host.svc.sql),
        Some(host.svc.commands.as_ref()),
        None,
    )
    .await?;
    let mut report = TestReport {
        name: name.to_string(),
        ok: false,
        errors: checked.errors.clone(),
        warnings: checked.warnings.clone(),
        blessed: Vec::new(),
        ran: vec!["check".into()],
        generation: host
            .svc
            .extension_activation
            .current_pin()
            .and_then(|pin| pin.generation().map(str::to_string))
            .or_else(|| Some(host.svc.extension_catalog.generation(&host.root))),
        imports: crate::import_lines(&checked.extension),
        // The throwaway binds the built-in roles only: profiles fall back.
        profiles: crate::profile_lines(&checked.extension, &Default::default()),
        contributes: Vec::new(),
        unsupported: Vec::new(),
        left: Vec::new(),
    };
    // What activation held back: this extension's is an error (unless
    // check already says it), another's a warning — it's beside it.
    let prefix = format!("{name}: ");
    for problem in &host.problems {
        match problem.strip_prefix(&prefix) {
            Some(why) if !report.errors.iter().any(|e| e.contains(why)) => report
                .errors
                .push(format!("{name}: it doesn't activate: {why}")),
            Some(_) => {}
            None => report.warnings.push(format!(
                "another extension here doesn't activate — {problem}"
            )),
        }
    }
    let ext = checked.extension;
    report.contributes = ext.contributions.iter().map(|d| d.id.to_string()).collect();
    report.unsupported = unsupported(&ext);
    if report.errors.is_empty() {
        examples(&host, &ext, &mut report).await;
        for spec in &ext.providers {
            report.ran.push(format!("provider {}", spec.id));
            test_provider(root, &ext, spec, bless, &env, &mut report).await;
        }
        for decl in ext
            .implementations
            .iter()
            .filter(|d| d.capability == EFFORT_POLICY && d.script.is_some())
        {
            policy_suite(root, &ext, decl, &env, &mut report).await;
        }
        questions(&host, &ext, &mut report).await;
        incremental_models(&host, &ext, &mut report).await;
    }
    report.ok = report.errors.is_empty();
    Ok(report)
}

/// An intent example's fixture (`fixtures/<name>.yaml`): `None` when it
/// has none; the parsed document, or why it isn't one.
fn example_fixture(
    read: &(dyn Fn(&str) -> Option<String> + Sync),
    rel: &str,
    example: &str,
) -> Option<(String, Result<Value, String>)> {
    let file = format!("fixtures/{example}.yaml");
    let shown = format!("{rel}/{file}");
    let text = read(&file)?;
    let doc = serde_yaml::from_str::<Value>(&text)
        .map_err(|e| format!("{shown}:1: not YAML: {e} — fix: correct it"));
    Some((shown, doc))
}

/// The intent examples a throwaway oxplow runs: a lens's, a derived
/// collector's. A provider's (`input: { command, input }`) run in its
/// session.
async fn examples(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    // Read as the extension's files are everywhere: a bundled extension's
    // fixtures are embedded with it, not on disk.
    let read =
        |file: &str| oxplow_app::extensions::read_extension_file(&host.root, &ext.name, file);
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    for example in examples {
        let Some((shown, doc)) = example_fixture(&read, &rel, &example.name) else {
            report.warnings.push(format!(
                "{rel}/extension.yaml:1: intent example `{}` has no fixture, so it isn't run — fix: \
                 add fixtures/{}.yaml with its `input` (`{{ lens, params?, rows? }}`, `{{ collector, rows? }}`, a fact \
                 collector's `{{ collector, files }}`, `{{ event }}`, `{{ job, intentKey, request }}`, \
                 `{{ component }}`, `{{ form }}`, or a provider's `{{ command, input }}`) and `expect`",
                example.name, example.name
            ));
            continue;
        };
        let doc = match doc {
            Ok(d) => d,
            Err(e) => {
                report.errors.push(e);
                continue;
            }
        };
        let ex = Example {
            name: &example.name,
            shown: &shown,
            input: doc.get("input").cloned().unwrap_or(Value::Null),
            expect: doc.get("expect").cloned().unwrap_or(Value::Null),
        };
        if let Some(why) = withheld_target(ext, &ex.input) {
            report.warnings.push(format!(
                "{shown}:1: example `{}` isn't run: {why}",
                example.name
            ));
            continue;
        }
        if let Some(replies) = model_replies(&ex, report) {
            if let Err(e) = host.answer_with(replies) {
                report
                    .errors
                    .push(format!("{shown}:1: the fixture model: {e}"));
                continue;
            }
        } else {
            continue;
        }
        if let Some(slug) = ex.input.get("lens").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            lens_example(host, ext, &ex, slug, report).await;
        } else if let Some(slug) = ex.input.get("form").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            form_example(host, ext, &ex, slug, report).await;
        } else if let Some(id) = ex.input.get("job").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            job_example(host, ext, &ex, id, report).await;
        } else if let Some(id) = ex.input.get("component").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            component_example(host, ext, &ex, id, report).await;
        } else if ex.input.get("event").is_some()
            && ex.input.get("effect").is_none()
            && ex.input.get("implementation").is_none()
            && ex.input.get("collector").is_none()
        {
            report.ran.push(format!("example {}", ex.name));
            event_example(host, ext, &ex, report).await;
        } else if let Some(id) = ex.input.get("effect").and_then(Value::as_str) {
            effect_example(host, ext, &ex, id, report).await;
        } else if let Some(id) = ex.input.get("implementation").and_then(Value::as_str) {
            policy_example(host, ext, &ex, id, report).await;
        } else if let Some(t) = ex.input.get("event_type").and_then(Value::as_str) {
            event_type_example(ext, &ex, t, report);
        } else if let Some(link) = ex.input.get("wikilink").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            ref_kind_example(ext, &ex, link, report);
        } else if let Some(id) = ex.input.get("collector").and_then(Value::as_str) {
            collector_example(host, ext, &ex, id, report).await;
        } else if let Some(cmd) = ex
            .input
            .get("command")
            .and_then(Value::as_str)
            .and_then(|name| ext.commands.iter().find(|c| c.name == name))
        {
            report.ran.push(format!("example {}", ex.name));
            command_example(host, &ex, cmd, report).await;
        } else if ex.input.get("command").is_none() || ext.providers.is_empty() {
            report.errors.push(format!(
                "{shown}:1: example `{}`'s `input` names no lens, collector, command, effect, \
                 event, job, component, form, effort policy, event type, wikilink or provider command — \
                 fix: `{{ lens: <slug>, params?, rows? }}`, `{{ collector: <id>, rows?, event? }}`, \
                 `{{ command: <name>, input }}`, `{{ effect: <id>, event: {{ type, payload }} }}` \
                 (a dry run), `{{ event: {{ type, payload }} }}` (logged, and what reacts runs), \
                 `{{ job: <id>, intentKey, request }}`, `{{ component: <id>, items?, queries?, invokes? }}`, `{{ form: <lens>, params? }}`, \
                 `{{ implementation: <id>, event: {{ type, payload, anchors? }} }}`, \
                 `{{ event_type, v?, payload }}`, `{{ wikilink }}`, or (with a provider) \
                 `{{ command, input }}`",
                example.name
            ));
        }
    }
}

/// Why an example's target isn't here: the lens, form, job, component,
/// effect, collector or command it names was withheld because an import is
/// unavailable. `None` when it's here, or wasn't withheld.
fn withheld_target(ext: &Extension, input: &Value) -> Option<String> {
    use oxplow_domain::contribution::ContributionKind as Kind;
    let named = |key: &str| input.get(key).and_then(Value::as_str);
    let command = named("command").map(|name| {
        name.strip_prefix(&format!("{}.", ext.namespace))
            .unwrap_or(name)
    });
    let target = [
        (Kind::Lens, named("lens")),
        (Kind::Lens, named("form")),
        (Kind::Job, named("job")),
        (Kind::CustomComponent, named("component")),
        (Kind::Effect, named("effect")),
        (Kind::Collector, named("collector")),
        (Kind::Command, command),
    ];
    let withheld = ext.public_contracts.withheld.iter().find(|withheld| {
        target.iter().any(|(kind, local)| {
            withheld.contribution.kind == *kind
                && Some(withheld.contribution.local.as_str()) == *local
        })
    })?;
    let id = &withheld.contribution;
    if withheld.imports.is_empty() {
        return Some(format!(
            "`{id}` is withheld because what it mounts from another extension was withheld"
        ));
    }
    let lines = crate::import_lines(ext);
    let why: Vec<_> = withheld
        .imports
        .iter()
        .map(|binding| {
            let state = ext
                .public_contracts
                .bindings
                .iter()
                .zip(&lines)
                .find(|(b, _)| &b.id == binding)
                .and_then(|(_, line)| line.split_once(": ").map(|(_, state)| state.to_owned()))
                .unwrap_or_else(|| "unavailable".to_owned());
            format!("import `{binding}` is {state}")
        })
        .collect();
    Some(format!("`{id}` is withheld because {}", why.join(", and ")))
}

/// The example's `model: { replies: [...] }` — what the fixture model
/// answers its model calls with, in turn (a string, or `{ text?,
/// toolCalls?, chunkDelayMs? }`); none when it gives none. `None`, with
/// the error reported, when it isn't that shape.
fn model_replies(ex: &Example<'_>, report: &mut TestReport) -> Option<Vec<FixtureReply>> {
    let Some(model) = ex.input.get("model") else {
        return Some(Vec::new());
    };
    let replies = model
        .get("replies")
        .filter(|r| r.is_array())
        .map(|r| serde_json::from_value::<Vec<FixtureReply>>(r.clone()));
    match replies {
        Some(Ok(replies)) => Some(replies),
        Some(Err(e)) => {
            report.errors.push(format!(
                "{}:1: example `{}`: a model reply is a string or `{{ text?, toolCalls?: \
                 [{{ name, arguments }}], chunkDelayMs? }}`: {e}",
                ex.shown, ex.name
            ));
            None
        }
        None => {
            report.errors.push(format!(
                "{}:1: example `{}`: `model` must be `{{ replies: [ … ] }}` — what the fixture \
                 model answers, in turn",
                ex.shown, ex.name
            ));
            None
        }
    }
}

/// `got` with only what `expect` names: an object keeps the keys `expect`
/// has, an array its items shaped like `expect`'s (when as many). So a
/// fixture says what it cares about — an effect's `state`, not its run's
/// every field — and `first_mismatch` compares exactly that.
fn shaped_like(expect: &Value, got: &Value) -> Value {
    match (expect, got) {
        (Value::Object(want), Value::Object(have)) => Value::Object(
            want.keys()
                .filter_map(|k| have.get(k).map(|v| (k.clone(), shaped_like(&want[k], v))))
                .collect(),
        ),
        (Value::Array(want), Value::Array(have)) if want.len() == have.len() => Value::Array(
            want.iter()
                .zip(have)
                .map(|(w, h)| shaped_like(w, h))
                .collect(),
        ),
        _ => got.clone(),
    }
}

/// What `ext` contributes that only a person sees in the app — this host
/// renders nothing, so these aren't run here.
fn unsupported(ext: &Extension) -> Vec<String> {
    let mut out = Vec::new();
    let mut note = |n: usize, what: &str| {
        if n > 0 {
            out.push(format!("{n} {what}"));
        }
    };
    note(ext.pages.len(), "page(s)");
    note(ext.panels.len(), "panel(s)");
    note(ext.ui.slots.len(), "slot mount(s)");
    note(ext.ui.decorators.len(), "decorator(s)");
    note(ext.ui.replacements.len(), "replacement(s)");
    note(ext.skills.len(), "skill(s)");
    out
}

/// Log the fixture's event (`input: { event: { type, payload, subject?,
/// anchors? }, model? }`) in the throwaway and deliver it: the extension's
/// effects — approved here, as a person would — react for real, their
/// commands run through the bus. What happened against `expect`:
/// `{ effects: [{ id, state? }], commands: [names] }` (`$any` matches).
async fn event_example(host: &Host, ext: &Extension, ex: &Example<'_>, report: &mut TestReport) {
    let shown = ex.shown;
    if let Err(e) = approve_effects(host, ext).await {
        report
            .errors
            .push(format!("{shown}:1: approving its effects: {e}"));
        return;
    }
    let Some(view) = example_event(host, ex, report) else {
        return;
    };
    let mut env = match oxplow_domain::events::Envelope::new(
        view.event_type.clone(),
        view.v,
        "fixture",
        view.payload.clone(),
    ) {
        Ok(env) => env,
        Err(e) => {
            report.errors.push(format!("{shown}:1: the event: {e}"));
            return;
        }
    };
    env.subject = view.subject.clone();
    env.anchors = view.anchors.clone();
    let seq = match host.svc.event_log_store.append(env).await {
        Ok(seq) => seq,
        Err(e) => {
            report
                .errors
                .push(format!("{shown}:1: logging the event: {e}"));
            return;
        }
    };
    let (effects, commands) = effect_outcome(host, seq, "=").await;
    let got = shaped_like(
        &ex.expect,
        &json!({ "effects": effects, "commands": commands }),
    );
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: the event led to {got} at `{path}`, the example expects {want} — fix: \
             the effects, or the example's `expect` (`{{ effects: [{{ id, state }}], commands: \
             [names] }}`)"
        ));
    }
}

/// What the effects did after log position `seq`: each latest run on the
/// events at it (`at` is `=`: one event) or past it (`>`: whatever a job
/// logged), and the commands they ran, once the effects have heard the log.
async fn effect_outcome(host: &Host, seq: i64, at: &str) -> (Vec<Value>, Vec<String>) {
    host.svc
        .event_pump
        .settle(
            &[oxplow_app::effect_triggers::NAME],
            std::time::Duration::from_secs(10),
        )
        .await;
    let rows = |sql: &str| {
        let sql = sql.to_string();
        let svc = host.svc.clone();
        async move {
            svc.sql
                .query_sql(&sql, vec![oxplow_db::SqlCell::Int(seq)], None)
                .await
                .map(|r| r.rows)
                .unwrap_or_default()
        }
    };
    let text = |c: &oxplow_db::SqlCell| match c {
        oxplow_db::SqlCell::Text(t) => t.clone(),
        other => format!("{other:?}"),
    };
    let effects: Vec<Value> = rows(&format!(
        "SELECT effect, state, reason FROM v_effect_run WHERE event_seq {at} ?1 AND latest = 1 \
         ORDER BY effect"
    ))
    .await
    .iter()
    .map(|r| json!({ "id": text(&r[0]), "state": text(&r[1]), "reason": text(&r[2]) }))
    .collect();
    let commands: Vec<String> = rows(
        "SELECT json_extract(payload, '$.command') FROM v_event \
         WHERE type = 'command.executed' AND seq > ?1 \
           AND json_extract(payload, '$.actor_kind') = 'effect' ORDER BY seq",
    )
    .await
    .iter()
    .map(|r| text(&r[0]))
    .collect();
    (effects, commands)
}

/// Approve `ext`'s effects in the throwaway as a person does, each
/// starting at the log's head.
async fn approve_effects(host: &Host, ext: &Extension) -> Result<(), String> {
    oxplow_app::exec_consent::approve_every(
        &host.svc,
        oxplow_app::exec_consent::ProgramKind::Effect,
    )?;
    for decl in &ext.effects {
        oxplow_app::effects::approved(&host.svc.db, &decl.name())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Record intent for job `id` through the extension's own command over
/// `jobs.write/enqueue` (`input: { job, intentKey, request, model?,
/// setup?, rows? }`: `setup` and `rows` first make what its tools read), the
/// job approved here as a person would, and read its outcome through its
/// `jobs.read/snapshot` command once it ends: against `expect` — `{ state,
/// result?, text?, toolCalls?, output?, effects?, commands?, pages? }`
/// (`$any` matches): its streamed `text`, how many tools its model called
/// and its output rows (`[{ kind, … }]`, `v_job_run_output`). Its model calls
/// answer with the fixture model's replies; with `cancel: true` in its
/// input, the job is cancelled through the extension's `jobs.write/cancel`
/// command once its first output is written. An expectation of `effects`,
/// `commands` or `pages` delivers the `contribution.job.completed` the job
/// logs to the extension's effects, approved here as a person would: the
/// effects that reacted (`[{ id, state? }]`), the commands they ran
/// (`[names]`) and which of the named wiki pages (`[ref]`) now exist.
async fn job_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::extension_commands::CommandHandler;
    let shown = ex.shown;
    let by_op = |scope: &str, op: &str| {
        ext.commands
            .iter()
            .find(|c| matches!(&c.handler, CommandHandler::Scope { scope: s, op: o } if s == scope && o == op))
            .map(|c| c.name.clone())
    };
    let (Some(enqueue), Some(snapshot)) = (
        by_op("jobs.write", "enqueue"),
        by_op("jobs.read", "snapshot"),
    ) else {
        report.errors.push(format!(
            "{shown}:1: job `{id}`: the extension declares no command over \
             `jobs.write/enqueue` and `jobs.read/snapshot` to run it with — fix: declare both \
             (`scope: jobs.write, op: enqueue`; `scope: jobs.read, op: snapshot`)"
        ));
        return;
    };
    if let Err(e) = oxplow_app::exec_consent::approve_every(
        &host.svc,
        oxplow_app::exec_consent::ProgramKind::Job,
    ) {
        report
            .errors
            .push(format!("{shown}:1: approving its jobs: {e}"));
        return;
    }
    let delivers = ["effects", "commands", "pages"]
        .iter()
        .any(|k| ex.expect.get(k).is_some());
    if delivers {
        if let Err(e) = approve_effects(host, ext).await {
            report
                .errors
                .push(format!("{shown}:1: approving its effects: {e}"));
            return;
        }
    }
    if !seed_rows(host, ext, ex, report).await || !run_setup(host, ex, report).await {
        return;
    }
    let logged = host
        .svc
        .sql
        .query_sql("SELECT COALESCE(MAX(seq), 0) FROM v_event", vec![], None)
        .await
        .ok()
        .and_then(|r| match r.rows.first().and_then(|row| row.first()) {
            Some(oxplow_db::SqlCell::Int(n)) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);
    let input = json!({
        "job": id,
        "intentKey": ex.input.get("intentKey").cloned().unwrap_or_else(|| json!(ex.name)),
        "request": ex.input.get("request").cloned().unwrap_or(Value::Null),
    });
    // A job asked for in a thread: the effects that react to its end run
    // for that thread, as a writer-bound command needs.
    let asker = if delivers {
        Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(1)),
            stream_id: None,
        }
    } else {
        Actor::Human
    };
    let queued = match host.svc.commands.run(&asker, &enqueue, input, false).await {
        Ok(out) => out.result,
        Err(e) => {
            report
                .errors
                .push(format!("{shown}:1: job `{id}` wasn't recorded: {e}"));
            return;
        }
    };
    let Some(job) = queued.get("jobRun").and_then(Value::as_str).and_then(|r| {
        oxplow_domain::refs::build::id_of(r, oxplow_domain::refs::kind::JOB_RUN).ok()
    }) else {
        report
            .errors
            .push(format!("{shown}:1: job `{id}`: {queued} names no job"));
        return;
    };
    if ex.input.get("cancel") == Some(&json!(true)) {
        let Some(cancel) = by_op("jobs.write", "cancel") else {
            report.errors.push(format!(
                "{shown}:1: job `{id}`: `cancel: true` needs a command over `jobs.write/cancel` \
                 — fix: declare one (`scope: jobs.write, op: cancel`)"
            ));
            return;
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let snap = host
                .svc
                .commands
                .run(
                    &Actor::Human,
                    &snapshot,
                    json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&job) }),
                    false,
                )
                .await
                .map(|out| out.result)
                .unwrap_or(Value::Null);
            let started = snap
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.is_empty());
            let ended = snap.get("state").and_then(Value::as_str) != Some("running")
                && snap.get("state").and_then(Value::as_str) != Some("queued");
            if started || ended || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        if let Err(e) = host
            .svc
            .commands
            .run(
                &asker,
                &cancel,
                json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&job) }),
                false,
            )
            .await
        {
            report
                .errors
                .push(format!("{shown}:1: cancelling job `{id}`: {e}"));
            return;
        }
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let done = loop {
        let state = host
            .svc
            .commands
            .run(
                &Actor::Human,
                &snapshot,
                json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&job) }),
                false,
            )
            .await
            .map(|out| out.result);
        match state {
            Ok(s)
                if ["succeeded", "failed", "cancelled", "interrupted"]
                    .contains(&s.get("state").and_then(Value::as_str).unwrap_or("")) =>
            {
                break s
            }
            Ok(s) if tokio::time::Instant::now() >= deadline => break s,
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            Err(e) => {
                report
                    .errors
                    .push(format!("{shown}:1: job `{id}`'s snapshot: {e}"));
                return;
            }
        }
    };
    let output = host
        .svc
        .sql
        .query_sql(
            "SELECT kind, data FROM v_job_run_output WHERE job_id = ?1 ORDER BY attempt, seq",
            vec![oxplow_db::SqlCell::Text(job.clone())],
            None,
        )
        .await
        .map(|r| {
            r.rows
                .iter()
                .map(|row| {
                    let mut out = match row.get(1) {
                        Some(oxplow_db::SqlCell::Text(data)) => {
                            serde_json::from_str(data).unwrap_or_else(|_| json!({}))
                        }
                        _ => json!({}),
                    };
                    if let (Some(oxplow_db::SqlCell::Text(kind)), Some(obj)) =
                        (row.first(), out.as_object_mut())
                    {
                        obj.insert("kind".into(), json!(kind));
                    }
                    out
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut ended = json!({
        "state": done.get("state"),
        "result": done.get("result"),
        "text": done.get("text"),
        "toolCalls": done.get("toolCalls"),
        "output": output,
    });
    if delivers {
        let (effects, commands) = effect_outcome(host, logged, ">").await;
        ended["effects"] = Value::Array(effects);
        ended["commands"] = json!(commands);
        let wanted: Vec<String> = ex
            .expect
            .get("pages")
            .and_then(Value::as_array)
            .map(|p| {
                p.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut pages = Vec::new();
        for page in wanted {
            let found = host
                .svc
                .sql
                .query_sql(
                    "SELECT 1 FROM v_knowledge_page WHERE ref = ?1",
                    vec![oxplow_db::SqlCell::Text(page.clone())],
                    None,
                )
                .await
                .map(|r| !r.rows.is_empty())
                .unwrap_or(false);
            if found {
                pages.push(page);
            }
        }
        ended["pages"] = json!(pages);
    }
    let got = shaped_like(&ex.expect, &ended);
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: job `{id}` ended {got} at `{path}`, the example expects {want} — fix: \
             the job's request, or the example's `expect` (`{{ state, result?, text?, toolCalls?, \
             output?, effects?, commands?, pages? }}`)"
        ));
    }
}

/// The example's `rows: { <entity>: [ … ] }`, published for the
/// extension's own entities in place of what a collector would have
/// brought. `false`, with the error reported, when they couldn't be.
async fn seed_rows(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> bool {
    let shown = ex.shown;
    let Some(rows) = ex.input.get("rows") else {
        return true;
    };
    let rows = match serde_json::from_value::<std::collections::BTreeMap<String, Vec<Value>>>(
        rows.clone(),
    ) {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(format!(
                "{shown}:1: `rows` must map each entity to its rows: {e}"
            ));
            return false;
        }
    };
    if let Err(e) =
        oxplow_app::collector_runner::publish_declared_rows(&host.svc.db, ext, &rows).await
    {
        report.errors.push(format!("{shown}:1: `rows`: {e}"));
        return false;
    }
    true
}

/// The example's `setup: [{ command, input?, as? }]`: oxplow's own
/// commands, run in order before the example, to make the data its
/// contribution reads (a work item, an effort, a test run). Each runs as a
/// person, or with `as: agent` as the agent of `thread:thr1`. `false`,
/// with the error reported, when one fails.
async fn run_setup(host: &Host, ex: &Example<'_>, report: &mut TestReport) -> bool {
    let shown = ex.shown;
    let Some(steps) = ex.input.get("setup") else {
        return true;
    };
    let Some(steps) = steps.as_array() else {
        report.errors.push(format!(
            "{shown}:1: `setup` isn't a list — fix: `setup: [{{ command, input?, as? }}]`"
        ));
        return false;
    };
    if let Err(e) = host.svc.streams.ensure_primary().await {
        report
            .errors
            .push(format!("{shown}:1: `setup`: its thread: {e}"));
        return false;
    }
    for (index, step) in steps.iter().enumerate() {
        let Some(command) = step.get("command").and_then(Value::as_str) else {
            report.errors.push(format!(
                "{shown}:1: `setup[{index}]` names no command — fix: `{{ command, input?, as? }}`"
            ));
            return false;
        };
        let actor = match step.get("as").and_then(Value::as_str) {
            None | Some("person") => Actor::Human,
            Some("agent") => Actor::Agent {
                session_id: None,
                thread_id: Some(oxplow_domain::ThreadId::new(1)),
                stream_id: None,
            },
            Some(other) => {
                report.errors.push(format!(
                    "{shown}:1: `setup[{index}]`: `as: {other}` — fix: `person` or `agent`"
                ));
                return false;
            }
        };
        let input = step.get("input").cloned().unwrap_or_else(|| json!({}));
        if let Err(e) = host.svc.commands.run(&actor, command, input, false).await {
            report.errors.push(format!(
                "{shown}:1: `setup[{index}]` `{command}` failed: {e} — fix: its input, or the steps \
                 before it"
            ));
            return false;
        }
    }
    true
}

/// Load component `id` as its frame would, then ask what the fixture asks
/// (`input: { component, lens?, items?: [title], queries?: [{ asset,
/// params? }], invokes?: [{ command, input?, approved? }] }`): `items` are
/// work items filed first (`work_item:oxplow:tsk1`, …); each query's rows and each
/// invoke's outcome — refused until the component is approved, as for a
/// person — against `expect: { queries?: [{ rows }], invokes?: [{ ok?,
/// refused? }] }`. The frame itself isn't drawn here.
async fn component_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::lens_actions::{
        invoke_component_command, load_component, run_component_query, ComponentInvoke,
    };
    let shown = ex.shown;
    // The lens that shows it: the one the fixture names, else its first.
    let lens = ex
        .input
        .get("lens")
        .and_then(Value::as_str)
        .map(|slug| format!("{}/{slug}", ext.name))
        .or_else(|| {
            ext.lenses
                .iter()
                .find(|l| l.custom.as_ref().and_then(|c| c.component.as_deref()) == Some(id))
                .map(|l| l.id.clone())
        });
    let Some(lens) = lens else {
        report.errors.push(format!(
            "{shown}:1: component `{id}`: no lens of this extension shows it — fix: a `viz: \
             custom` lens with `custom: {{ component: {id} }}`, or name one with `lens:`"
        ));
        return;
    };
    let version = match load_component(&host.svc, &host.root, &lens) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("{shown}:1: component `{id}` doesn't load: {e}"));
            return;
        }
    };
    for title in ex
        .input
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let made = host
            .svc
            .work_items_client()
            .create(
                &Actor::Human,
                oxplow_app::work_items::NewItem {
                    title: title.as_str().unwrap_or_default().into(),
                    ..Default::default()
                },
            )
            .await;
        if let Err(e) = made {
            report
                .errors
                .push(format!("{shown}:1: filing its item `{title}`: {e}"));
            return;
        }
    }
    let ctx = oxplow_app::extensions::LensContext::default();
    let mut queries = Vec::new();
    for q in ex
        .input
        .get("queries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let asset = q
            .get("asset")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = q
            .get("params")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                    .collect()
            })
            .unwrap_or_default();
        queries.push(
            match run_component_query(&host.svc, &host.root, &lens, &asset, params, &ctx, &version)
                .await
            {
                Ok(run) => json!({ "rows": run.result.rows.len() }),
                Err(e) => json!({ "error": e.to_string() }),
            },
        );
    }
    let mut invokes = Vec::new();
    for i in ex
        .input
        .get("invokes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if i.get("approved").and_then(Value::as_bool).unwrap_or(false) {
            if let Err(e) = oxplow_app::exec_consent::approve_every(
                &host.svc,
                oxplow_app::exec_consent::ProgramKind::Component,
            ) {
                report
                    .errors
                    .push(format!("{shown}:1: approving its component: {e}"));
                return;
            }
        }
        let out = invoke_component_command(
            &host.svc,
            &host.root,
            ComponentInvoke {
                lens_id: lens.clone(),
                command: i
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                input: i.get("input").cloned().unwrap_or_else(|| json!({})),
                on_behalf_of: Actor::Human,
                confirmed: true,
                version: version.clone(),
            },
        )
        .await;
        invokes.push(match out {
            Ok(out) => json!({ "ok": true, "result": out.result }),
            Err(oxplow_domain::CommandError::Denied { reason }) => {
                json!({ "ok": false, "refused": reason })
            }
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        });
    }
    let got = shaped_like(
        &ex.expect,
        &json!({ "queries": queries, "invokes": invokes }),
    );
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: component `{id}` answered {got} at `{path}`, the example expects {want} \
             — fix: the component's declarations, or the example's `expect` (`{{ queries: [{{ \
             rows }}], invokes: [{{ ok, refused? }}] }}`)"
        ));
    }
}

/// Open form lens `slug` as a person would (`input: { form, params? }`):
/// `expect: { values?, fixed?, fields? }` against the values the fields
/// start from (the lens's `defaults`, its first row laid over them), the
/// fields shown read-only, and the fields shown (`null`: all).
async fn form_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    slug: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let params = ex
        .input
        .get("params")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let start = oxplow_app::lens_actions::form_start(
        &host.svc,
        &host.root,
        &format!("{}/{slug}", ext.name),
        params,
        &oxplow_app::extensions::LensContext::default(),
    )
    .await;
    let start = match start {
        Ok(start) => start,
        Err(e) => {
            report.errors.push(format!(
                "{shown}:1: form `{slug}` didn't open: {e} — fix: the form lens (`viz: form` with \
                 a `form:` naming a command), or the example's params"
            ));
            return;
        }
    };
    let got = shaped_like(
        &ex.expect,
        &json!({ "values": start.values, "fixed": start.fixed, "fields": start.fields }),
    );
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: form `{slug}` opened with {got} at `{path}`, the example expects {want} \
             — fix: the form's `defaults`, `fixed` or `fields`, or the example's `expect` (`{{ \
             values?, fixed?, fields? }}`)"
        ));
    }
}

/// One intent example with its fixture.
struct Example<'a> {
    name: &'a str,
    /// Its fixture file, as findings name it.
    shown: &'a str,
    input: Value,
    expect: Value,
}

/// Run lens `slug` with the fixture's params; its columns and row count
/// against `expect` (`$any` matches anything).
async fn lens_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    slug: &str,
    report: &mut TestReport,
) {
    let (shown, expect) = (ex.shown, &ex.expect);
    if !seed_rows(host, ext, ex, report).await {
        return;
    }
    let params = ex
        .input
        .get("params")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let run = oxplow_app::extensions::run_lens(
        &host.svc.sql,
        &host.svc.extension_catalog,
        &host.root,
        &format!("{}/{slug}", ext.name),
        params,
        &oxplow_app::extensions::LensContext::default(),
    )
    .await;
    let run = match run {
        Ok(run) => run,
        Err(e) => {
            report.errors.push(format!(
                "{shown}:1: lens `{slug}` failed: {e} — fix: the lens, or the example's params"
            ));
            return;
        }
    };
    let mut got = json!({ "rows": run.result.rows.len() });
    if expect.get("columns").is_some() {
        got["columns"] = json!(run.result.columns);
    }
    if let Some((path, want, got)) = first_mismatch(expect, &got) {
        report.errors.push(format!(
            "{shown}:1: lens `{slug}` returned {got} at `{path}`, the example expects {want} — \
             fix: the lens, or the example's `expect` (`{{ columns?, rows: n | $any }}`)"
        ));
    }
}

/// A fixture's `answers`: per scope, its calls' answers in call order,
/// standing in for the scopes' own (none: each is served for real);
/// `None`, with the error reported, when they aren't that shape.
fn answers(
    ex: &Example<'_>,
    what: &str,
    report: &mut TestReport,
) -> Option<std::collections::BTreeMap<String, Vec<Value>>> {
    match ex.input.get("answers").cloned().map(serde_json::from_value) {
        None => Some(Default::default()),
        Some(Ok(answers)) => Some(answers),
        Some(Err(e)) => {
            report.errors.push(format!(
                "{}:1: {what}: `answers` must map each scope to its answers in call \
                 order: {e}",
                ex.shown
            ));
            None
        }
    }
}

/// Dry-run one of the extension's own `commands:` on the fixture's
/// `input` (and `answers`, standing in for its scope calls' — per
/// scope, in call order): what it composes, checked against the
/// throwaway's registry, and what it answers, against `expect` — `{
/// commands: [names] }` and/or `{ result }` (only the keys it names), or
/// `{ refuses: <part of the reason> }`. Nothing runs.
async fn command_example(
    host: &Host,
    ex: &Example<'_>,
    cmd: &oxplow_app::extension_commands::ExtensionCommand,
    report: &mut TestReport,
) {
    use oxplow_app::extension_commands::{call_names, dry_run, Composed};
    let shown = ex.shown;
    let Some(answers) = answers(ex, &format!("command `{}`", cmd.name), report) else {
        return;
    };
    let input = ex.input.get("input").cloned().unwrap_or_else(|| json!({}));
    let decided = dry_run(
        &host.svc.sql,
        cmd,
        &input,
        &answers,
        host.svc.commands.as_ref(),
    )
    .await;
    let problem = match (decided, ex.expect.get("refuses").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Composed::Refused(why)), Some(want)) if why.contains(want) => None,
        (Ok(Composed::Refused(why)), _) => Some(format!("refused ({why})")),
        (Ok(Composed::Run { calls, .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to refuse ({want})",
            call_names(&calls).join(", ")
        )),
        (Ok(Composed::Run { calls, result, .. }), None) => {
            let got = json!({ "commands": call_names(&calls), "result": result });
            first_mismatch(&ex.expect, &shaped_like(&ex.expect, &got)).map(|(path, want, got)| {
                let did = if path.starts_with("/result") {
                    "answered"
                } else {
                    "composed"
                };
                format!("{did} {got} at `{path}`, the example expects {want}")
            })
        }
    };
    if let Some(p) = problem {
        report.errors.push(format!(
            "{shown}:1: command `{}` {p} — fix: its script, or the example's `expect` \
             (`{{ commands: [names], result: {{ … }} }}` or `{{ refuses: <part of the reason> }}`)",
            cmd.name
        ));
    }
}

/// Dry-run effect `id` on the fixture's event (`input: { effect, event: {
/// type, payload, subject? }, rows? }`): whether it reacts (`on`/`where`),
/// and what its script composes — checked against the throwaway's
/// registry — against `expect`: `{ commands: [names] }`, `{ skip: <part of
/// the reason> }` or `{ reacts: false }`. Nothing runs.
async fn effect_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::effects::{dry_run, reacts_to};
    let (shown, example) = (ex.shown, ex.name);
    let Some(decl) = ext.effects.iter().find(|e| e.id == id) else {
        report.errors.push(format!(
            "{shown}:1: example `{example}` names effect `{id}`, which `{}` doesn't declare — \
             fix: the effect's `id`",
            ext.name
        ));
        return;
    };
    report.ran.push(format!("example {example}"));
    let Some(event) = example_event(host, ex, report) else {
        return;
    };
    let event_type = event.event_type.as_str();
    let reacts = reacts_to(decl, event_type, &event.payload);
    let want_reacts = ex
        .expect
        .get("reacts")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if reacts != want_reacts {
        report.errors.push(format!(
            "{shown}:1: effect `{id}` {} a `{event_type}` event like this one, the example \
             expects it {} — fix: its `on`/`where`, or the example's `expect`",
            if reacts {
                "reacts to"
            } else {
                "doesn't react to"
            },
            if want_reacts { "to react" } else { "not to" },
        ));
        return;
    }
    if !reacts {
        return;
    }
    let event = event.to_json();
    let Some(answers) = answers(ex, &format!("effect `{id}`"), report) else {
        return;
    };
    let decided = dry_run(
        &host.svc.sql,
        decl,
        &decl.script,
        event,
        &answers,
        Some(host.svc.commands.as_ref()),
    )
    .await;
    if let Some(p) = reaction_problem(&ex.expect, decided) {
        report.errors.push(format!(
            "{shown}:1: effect `{id}` {p} — fix: its script, or the example's `expect` \
             (`{{ commands: [names] }}`, `{{ skip: <part of the reason> }}` or `{{ reacts: false }}`)"
        ));
    }
}

/// What's wrong with a script's reaction against an example's `expect` —
/// `{ commands: [names] }`, or `{ skip: <part of the reason> | $any }` —
/// or `None` when it matches.
fn reaction_problem(
    expect: &Value,
    decided: Result<oxplow_app::effects::Reaction, String>,
) -> Option<String> {
    use oxplow_app::effects::Reaction;
    match (decided, expect.get("skip").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Reaction::Skip(why)), Some(want)) if want == "$any" || why.contains(want) => None,
        (Ok(Reaction::Skip(why)), _) => Some(format!("skipped ({why})")),
        (Ok(r @ Reaction::Run { .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to skip ({want})",
            r.command_names().join(", ")
        )),
        (Ok(r @ Reaction::Run { .. }), None) => first_mismatch(
            expect,
            &json!({ "commands": r.command_names() }),
        )
        .map(|(path, want, got)| format!("composed {got} at `{path}`, the example expects {want}")),
    }
}

/// The example's `event:` as the event envelope the runtime would hand
/// over (`EventView::fixture`, through the throwaway's vocabulary); one
/// that couldn't happen is a finding at the fixture.
fn example_event(host: &Host, ex: &Example<'_>, report: &mut TestReport) -> Option<EventView> {
    let given = ex.input.get("event").cloned().unwrap_or_else(|| json!({}));
    match EventView::fixture(&given, &host.svc.vocabulary.current()) {
        Ok(view) => Some(view),
        Err(e) => {
            report.errors.push(format!(
                "{}:1: example `{}`: {} — fix: the fixture's `event` \
                 (extensions reference: the event envelope)",
                ex.shown,
                ex.name,
                e.message()
            ));
            None
        }
    }
}

/// Dry-run the extension's effort policy script `id` on the fixture's
/// event (`input: { implementation, event: { type, payload, subject?,
/// anchors? }, answers? }`, the answers standing in for its scope calls'):
/// what it composes, checked against the throwaway's registry, against
/// `expect` as an effect's is. A policy emits nothing. Nothing runs.
async fn policy_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::effects::{run_over, Reaction};
    let (shown, example) = (ex.shown, ex.name);
    let Some((decl, script)) = ext
        .implementations
        .iter()
        .filter(|d| d.id == id && d.capability == EFFORT_POLICY)
        .find_map(|d| d.script.as_deref().map(|s| (d, s)))
    else {
        report.errors.push(format!(
            "{shown}:1: example `{example}` names effort policy `{id}`, which `{}` doesn't \
             declare as a script — fix: an `implementations:` entry with `capability: \
             effort_policy`, `id: {id}` and a `.star` entry",
            ext.name
        ));
        return;
    };
    report.ran.push(format!("example {example}"));
    let Some(event) = example_event(host, ex, report) else {
        return;
    };
    let event = event.to_json();
    let Some(answers) = answers(ex, &format!("effort policy `{id}`"), report) else {
        return;
    };
    let run = oxplow_app::run_context::for_dry_run();
    let decided = run_over(
        &host.svc.sql,
        &run,
        &decl.needs,
        script,
        event,
        Some(&answers),
        &oxplow_app::services::Sources::default(),
    )
    .await
    .and_then(|reaction| match &reaction {
        Reaction::Run { events, .. } if !events.is_empty() => {
            Err("it appends events, and a policy emits nothing".to_string())
        }
        Reaction::Run { calls, .. } => {
            oxplow_app::extension_commands::check_calls(host.svc.commands.as_ref(), calls)
                .map(|()| reaction)
        }
        Reaction::Skip(_) => Ok(reaction),
    });
    if let Some(p) = reaction_problem(&ex.expect, decided) {
        report.errors.push(format!(
            "{shown}:1: effort policy `{id}` {p} — fix: its script, or the example's `expect` \
             (`{{ commands: [names] }}` or `{{ skip: <part of the reason> | $any }}`)"
        ));
    }
}

/// Check a payload against one of the extension's declared event types
/// (`input: { event_type, v?, payload }`, the newest version when `v` is
/// left out): `expect: { valid: true | false, upcast?: <payload at the
/// newest version> }`.
fn event_type_example(
    ext: &Extension,
    ex: &Example<'_>,
    event_type: &str,
    report: &mut TestReport,
) {
    use oxplow_domain::events::schema::EventSchemaRegistry;
    let shown = ex.shown;
    let mut registry = EventSchemaRegistry::new();
    for d in &ext.event_types.types {
        // What doesn't register is `check`'s finding already.
        let _ = registry.register_declared(&ext.name, d.declared());
    }
    let Some(v) = ex
        .input
        .get("v")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .or_else(|| registry.latest(event_type))
    else {
        report.errors.push(format!(
            "{shown}:1: example `{}` names event type `{event_type}`, which `{}` doesn't declare \
             — fix: the type, or its `event_types:`",
            ex.name, ext.name
        ));
        return;
    };
    report.ran.push(format!("example {}", ex.name));
    let payload = ex
        .input
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let valid = registry.validate(event_type, v, &payload);
    let want_valid = ex
        .expect
        .get("valid")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if valid.is_ok() != want_valid {
        report.errors.push(format!(
            "{shown}:1: `{event_type}@{v}` {} this payload{}, the example expects {} — fix: its \
             schema, or the example",
            if valid.is_ok() { "accepts" } else { "refuses" },
            valid.err().map(|e| format!(" ({e})")).unwrap_or_default(),
            if want_valid { "it valid" } else { "it refused" },
        ));
        return;
    }
    if let Some(want) = ex.expect.get("upcast") {
        match registry.upcast_to_latest(event_type, v, payload) {
            Ok((_, got)) => {
                if let Some((path, want, got)) = first_mismatch(want, &got) {
                    report.errors.push(format!(
                        "{shown}:1: `{event_type}@{v}` upcasts to {got} at `{path}`, the example \
                         expects {want} — fix: its upcast, or the example's `expect.upcast`"
                    ));
                }
            }
            Err(e) => report.errors.push(format!(
                "{shown}:1: `{event_type}@{v}` doesn't upcast: {e} — fix: its upcast"
            )),
        }
    }
}

/// What a `[[…]]` names with the extension's ref kinds beside core's
/// (`input: { wikilink: "pr:12" }`): `expect: { ref: "acme_pr:12" | null }`.
fn ref_kind_example(ext: &Extension, ex: &Example<'_>, link: &str, report: &mut TestReport) {
    let mut kinds = oxplow_domain::refs::kind::core_kinds();
    for k in &ext.ref_kinds {
        if let Ok(spec) = oxplow_app::extension_ref_kinds::kind_spec(k) {
            let _ = kinds.register(spec);
        }
    }
    let got = oxplow_domain::refs::canonical_wikilink(&kinds, link).map(|r| r.to_string());
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &json!({ "ref": got })) {
        report.errors.push(format!(
            "{}:1: `[[{link}]]` names {got} at `{path}`, the example expects {want} — fix: the \
             ref kind's `id`/`wikilink`, or the example's `expect`",
            ex.shown
        ));
    }
}

/// Run fact collector `spec` over the fixture's `files` (path → content;
/// its other input keys — `report`, `rows`, `event` — are the collector's
/// input), storing nothing; its facts against `expect: { facts: [...] }`,
/// each compared on the keys its expected fact names (tsk1047).
async fn fact_collector_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    spec: &oxplow_config::collectors::CollectorSpec,
    report: &mut TestReport,
) {
    let (shown, example, id) = (ex.shown, ex.name, spec.id.as_str());
    report.ran.push(format!("example {example}"));
    let files: std::collections::HashMap<String, String> = ex
        .input
        .get("files")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let mut input: serde_json::Map<String, Value> = ex
        .input
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "collector" | "files"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    // Its `event`, as the runtime would hand it over.
    if input.contains_key("event") {
        let Some(event) = example_event(host, ex, report) else {
            return;
        };
        input.insert("event".into(), event.to_json());
    }
    let facts = match host
        .svc
        .metrics
        .preview_fact_collector(
            &ext.name,
            spec,
            files,
            Value::Object(input),
            oxplow_domain::run::Mode::External,
        )
        .await
    {
        Ok(f) => f,
        Err(why) => {
            report.errors.push(format!(
                "{shown}:1: collector `{id}` failed: {why} — fix: its script, or the example's files"
            ));
            return;
        }
    };
    // Each fact compared on what its expected fact names.
    let wanted = ex.expect.get("facts").and_then(Value::as_array);
    let got: Vec<Value> = facts
        .iter()
        .enumerate()
        .map(
            |(i, f)| match wanted.and_then(|w| w.get(i)).and_then(Value::as_object) {
                Some(keys) => Value::Object(
                    keys.keys()
                        .map(|k| (k.clone(), f.get(k).cloned().unwrap_or(Value::Null)))
                        .collect(),
                ),
                None => f.clone(),
            },
        )
        .collect();
    if let Some((path, want, got)) = first_mismatch(&ex.expect, &json!({ "facts": got })) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ facts: [{{ measure, value?, path?, \
             subject?, line?, rule?, dims? }}] }}`)"
        ));
    }
}

/// Run derived collector `id` over the fixture's `rows` (else its `input`
/// query, on the empty throwaway), storing nothing; the rows it would
/// store per entity — typed against the declaration — against `expect`.
async fn collector_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let (shown, example, expect) = (ex.shown, ex.name, &ex.expect);
    let Some(spec) = ext.collectors.iter().find(|c| c.id == id) else {
        report.errors.push(format!(
            "{shown}:1: example `{example}` names collector `{id}`, which `{}` doesn't declare — \
             fix: the collector's `id`",
            ext.name
        ));
        return;
    };
    if oxplow_app::collector_runner::needs_approval(&host.root, &ext.name, spec) {
        use oxplow_config::collectors::CollectorRuntime as R;
        let does = match spec.runtime {
            // Calls a model: approved here as a person would, then run
            // against the fixture model.
            R::Starlark | R::Jaq => None,
            R::Read => Some("reads a provider"),
            R::Exec => Some("runs a program"),
        };
        if let Some(does) = does {
            report.warnings.push(format!(
                "{shown}:1: example `{example}` runs collector `{id}`, which {does}: `extension \
                 test` doesn't run it (it needs a person's approval) — fix: none; run it from \
                 Settings → Data"
            ));
            return;
        }
        let approved =
            oxplow_app::collector_runner::approval_hash(&host.root.join(&ext.path), spec).and_then(
                |hash| {
                    oxplow_app::collector_runner::approve(&host.svc.approvals, &ext.name, id, &hash)
                },
            );
        if let Err(e) = approved {
            report
                .errors
                .push(format!("{shown}:1: approving collector `{id}`: {e}"));
            return;
        }
    }
    if !spec.facts.is_empty() {
        return fact_collector_example(host, ext, ex, spec, report).await;
    }
    report.ran.push(format!("example {example}"));
    let rows = ex
        .input
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| r.to_vec());
    let preview = oxplow_app::collector_runner::preview_collector(
        &oxplow_app::collector_runner::Collectors::of(&host.svc, &host.root),
        &ext.name,
        id,
        ex.input.get("event"),
        rows,
    )
    .await;
    let preview = match preview {
        Ok(p) => p,
        Err(e) => {
            use oxplow_app::collector_runner::RunCollectorError as E;
            let why = match e {
                E::NotFound => "it isn't loaded".to_string(),
                E::NeedsApproval(m) | E::Failed(m) | E::Disabled(m) => m,
                E::Storage(e) => e.to_string(),
            };
            report.errors.push(format!(
                "{shown}:1: collector `{id}` failed: {why} — fix: its script, or the example's rows"
            ));
            return;
        }
    };
    let entities: serde_json::Map<String, Value> = preview
        .entities
        .iter()
        .map(|e| (e.entity.clone(), json!(e.total)))
        .collect();
    // The events it would log (its extension's own types, checked as a
    // run's are): `expect: { events: [{ type, payload? }] }`.
    let events: Vec<Value> = preview
        .events
        .iter()
        .map(|e| json!({ "type": e.event_type, "v": e.v, "payload": e.payload, "subject": e.subject }))
        .collect();
    // Said only when there is something to say (or the example asks):
    // an example of a collector that emits must say what it expects.
    let mut got = json!({ "entities": entities });
    if !events.is_empty() || expect.get("events").is_some() {
        got["events"] = Value::Array(events);
    }
    if let Some((path, want, got)) = first_mismatch(expect, &got) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ entities: {{ <name>: n | $any }}, \
             events?: [{{ type, payload? }}] }}`)"
        ));
    }
}

/// Each incremental model (P8.B5), on its fixture `fixtures/model-<name>.yaml`
/// (`before:` and `after:`, each `{ <table>: [rows] }`): built whole on
/// `before`, appended to past its watermark after `after` is written, and
/// compared with a full refill — in a rehearsal, so nothing stays. A
/// watermark that doesn't grow as rows arrive drops a row below it
/// unseen; nothing at run time can tell, so this is where it shows.
async fn incremental_models(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap_or_default();
    for model in &ext.models {
        let decl = &model.decl;
        if decl
            .materialize
            .as_ref()
            .and_then(oxplow_db::models::Materialize::incremental)
            .is_none()
        {
            continue;
        }
        let line = oxplow_app::extensions::manifest_v2::entry_line(
            &manifest, "models", "name", &decl.name,
        )
        .unwrap_or(1);
        let at = format!("{rel}/extension.yaml:{line}");
        let fixture = format!("fixtures/model-{}.yaml", decl.name);
        let Ok(text) = std::fs::read_to_string(dir.join(&fixture)) else {
            report.warnings.push(format!(
                "{at}: incremental model `{}` has no {rel}/{fixture}, so appending isn't checked \
                 against a full refill — fix: write its `before:` and `after:` rows \
                 (`{{ <table>: [{{ column: value }}] }}`)",
                decl.name
            ));
            continue;
        };
        let rows = |doc: &Value, part: &str| -> Result<oxplow_db::models::FixtureRows, String> {
            let Some(tables) = doc.get(part) else {
                return Ok(Vec::new());
            };
            let tables = tables
                .as_object()
                .ok_or_else(|| format!("`{part}` isn't a map of table → rows"))?;
            tables
                .iter()
                .map(|(table, rows)| {
                    let rows = rows
                        .as_array()
                        .ok_or_else(|| format!("`{part}.{table}` isn't a list of rows"))?
                        .iter()
                        .map(|r| {
                            r.as_object()
                                .cloned()
                                .ok_or_else(|| format!("a row of `{part}.{table}` isn't a map"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((table.clone(), rows))
                })
                .collect()
        };
        let parsed = serde_yaml::from_str::<Value>(&text)
            .map_err(|e| e.to_string())
            .and_then(|doc| Ok((rows(&doc, "before")?, rows(&doc, "after")?)));
        let (before, after) = match parsed {
            Ok(p) => p,
            Err(e) => {
                report
                    .errors
                    .push(format!("{rel}/{fixture}:1: {e} — fix: correct the fixture"));
                continue;
            }
        };
        report.ran.push(format!("incremental {}", decl.name));
        let view = oxplow_db::models::extension_view(&ext.name, &decl.name);
        let decl_c = decl.clone();
        let checked = host
            .svc
            .db
            .rehearse(move |tx| {
                oxplow_db::models::incremental_matches_full(tx, &view, &decl_c, &before, &after)
            })
            .await;
        match checked {
            Ok(None) => {}
            Ok(Some(problem)) => report.errors.push(format!(
                "{at}: incremental model `{}` {problem} — fix: `materialize: on_change`, or a \
                 watermark that only grows as rows arrive",
                decl.name
            )),
            Err(e) => report.errors.push(format!(
                "{rel}/{fixture}:1: incremental model `{}` couldn't run on the fixture: {e} — \
                 fix: the fixture's rows",
                decl.name
            )),
        }
    }
}

/// The extension's own `questions.yaml` (the answerability check): each
/// question's `skill` is a markdown file in the extension, its SQL runs
/// on the throwaway oxplow, and a command is one on its bus (core's, the
/// extension's own `commands:`) or one of its providers' (`<id>.<name>`,
/// not on the bus until an instance runs).
async fn questions(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let file = format!("{rel}/questions.yaml");
    let Ok(text) = std::fs::read_to_string(dir.join("questions.yaml")) else {
        return;
    };
    report.ran.push("questions".into());
    let questions = match crate::answerability::parse(&text) {
        Ok(q) => q,
        Err(e) => {
            report.errors.push(format!(
                "{file}:1: {e} — fix: a list of {{ question, skill, reaches, shape }}"
            ));
            return;
        }
    };
    let skill_text = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let commands: Vec<(String, Value)> = ext
        .providers
        .iter()
        .filter_map(|spec| {
            providers::spec::read_declarations(spec, &|f| std::fs::read_to_string(dir.join(f)).ok())
                .ok()
                .map(|d| {
                    d.commands
                        .into_iter()
                        .map(|c| (format!("{}.{}", spec.id, c.name), c.input_schema))
                        .collect::<Vec<_>>()
                })
        })
        .flatten()
        .collect();
    let command_schema = |name: &str| {
        commands
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s.clone())
            .or_else(|| host.svc.commands.spec(name).map(|s| s.input_schema))
    };
    let checked = crate::answerability::check(
        &file,
        &questions,
        &crate::answerability::Checker {
            skill_text: &skill_text,
            sql: Some(&host.svc.sql),
            command_schema: &command_schema,
        },
    )
    .await;
    report.errors.extend(checked.errors);
    report.warnings.extend(checked.warnings);
}

/// The instance config the kit checks with: `fixtures/provider-<id>.yaml`.
fn fixture_config(dir: &Path, rel: &str, spec: &ProviderSpec) -> Result<Value, String> {
    let file = format!("fixtures/provider-{}.yaml", spec.id);
    let text = std::fs::read_to_string(dir.join(&file)).map_err(|_| {
        format!(
            "{rel}/{file}:1: provider `{}` has no test config — fix: write `config: {{ … }}` \
             (the instance config its `check` accepts) to that file",
            spec.id
        )
    })?;
    let doc: Value = serde_yaml::from_str(&text)
        .map_err(|e| format!("{rel}/{file}:1: not YAML: {e} — fix: `config: {{ … }}`"))?;
    Ok(doc.get("config").cloned().unwrap_or_else(|| json!({})))
}

/// Credentials the kit hands the provider: the declared names, read from
/// the environment it runs in (the author's own). For one the person
/// signs in for, that is an access token the author got themselves — the
/// kit never signs in — and a client secret is never the process's, here
/// as in the host.
fn env_credentials(
    spec: &ProviderSpec,
    env: &host::HostEnv,
) -> std::collections::BTreeMap<String, String> {
    spec.credential_names()
        .into_iter()
        .filter(|n| !spec.is_client_secret(n))
        .filter_map(|n| env(&n).map(|v| (n, v)))
        .collect()
}

async fn test_provider(
    root: &Path,
    ext: &Extension,
    spec: &ProviderSpec,
    bless: bool,
    env: &host::HostEnv,
    report: &mut TestReport,
) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = root.join(&rel);
    let manifest = format!("{rel}/extension.yaml");
    let declared = match providers::spec::read_declarations(spec, &|f| {
        std::fs::read_to_string(dir.join(f)).ok()
    }) {
        Ok(d) => d,
        Err(e) => {
            report.errors.push(format!(
                "{manifest}:1: {e} — fix: correct {}",
                spec.declarations
            ));
            return;
        }
    };
    let config = match fixture_config(&dir, &rel, spec) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(e);
            return;
        }
    };
    let launch = Launch {
        name: spec.approval_name(&ext.name),
        // The kit tests the program, as its default instance.
        instance_id: spec.id.clone(),
        ext_dir: dir.clone(),
        spec: spec.clone(),
        declared: declared.clone(),
        credentials: env_credentials(spec, env),
        host_env: env.clone(),
        host_calls: None,
        on_changed: None,
    };
    let client = match ReferenceClient::start(&launch).await {
        Ok(c) => c,
        Err(e) => {
            let fix = match &spec.adapter {
                Some(_) => format!(
                    "make `{}` an MCP server whose tools are the pinned ones",
                    spec.program().0
                ),
                None => format!(
                    "make `{}` an executable that speaks the provider protocol",
                    spec.program().0
                ),
            };
            report
                .errors
                .push(format!("{manifest}:1: {e} — fix: {fix}"));
            return;
        }
    };
    let under_test = UnderTest {
        ext,
        spec,
        declared: &declared,
        dir: &dir,
        rel: &rel,
        env,
    };
    session(&client, &under_test, config.clone(), report).await;
    let recorded = client.finish().await;
    for v in &recorded.violations {
        report.errors.push(format!(
            "{manifest}:1: provider `{}`: {v} — fix: send what the protocol's schemas describe \
             (crates/oxplow-provider-protocol/schemas)",
            spec.id
        ));
    }
    transcript(&dir, &rel, spec, &recorded.transcript, bless, report);
    suite(root, ext, spec, config, env, report).await;
}

/// The provider a session tests.
struct UnderTest<'a> {
    ext: &'a Extension,
    spec: &'a ProviderSpec,
    /// Its declarations file, as read.
    declared: &'a InitializeResult,
    /// The extension folder, shown as `rel`.
    dir: &'a Path,
    rel: &'a str,
    /// The environment its credentials come from.
    env: &'a host::HostEnv,
}

/// Initialize, check, and the intent examples, over the tapped client.
async fn session(
    client: &ReferenceClient,
    t: &UnderTest<'_>,
    config: Value,
    report: &mut TestReport,
) {
    let UnderTest {
        ext,
        spec,
        declared,
        dir,
        rel,
        env,
    } = *t;
    let decl_file = format!("{rel}/{}", spec.declarations);
    let fixture = format!("{rel}/fixtures/provider-{}.yaml", spec.id);
    let live: InitializeResult = match client
        .peer
        .call(method::INITIALIZE, &host::initialize_params())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            report.errors.push(format!(
                "{decl_file}:1: initialize failed: {e} — fix: answer `initialize`"
            ));
            return;
        }
    };
    if live != *declared {
        report.errors.push(format!(
            "{decl_file}:1: the running provider declares something else: {} — fix: make {} and \
             the program agree (a changed declaration needs approving again)",
            host::first_difference(declared, &live),
            spec.declarations
        ));
    }
    let checked: CheckResult = match client
        .peer
        .call(
            method::CHECK,
            &CheckParams {
                config,
                credentials: env_credentials(spec, env).into_keys().collect(),
            },
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!(
                "{fixture}:1: check failed: {e} — fix: answer `check`"
            ));
            return;
        }
    };
    let Some(handle) = checked.handle.filter(|_| checked.problems.is_empty()) else {
        for p in &checked.problems {
            report.errors.push(format!(
                "{fixture}:1: check reports `{}`: {} — fix: set it in the fixture's `config`",
                if p.path.is_empty() { "/" } else { &p.path },
                p.message
            ));
        }
        if checked.problems.is_empty() {
            report.errors.push(format!(
                "{fixture}:1: check returned neither problems nor a handle — fix: return a handle \
                 for a clean config"
            ));
        }
        return;
    };
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    // The examples that invoke a command; the rest (and a missing or
    // broken fixture) are `examples`'.
    for example in examples {
        let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
        let Some((shown, Ok(doc))) = example_fixture(&read, rel, &example.name) else {
            continue;
        };
        let Some(command) = doc.pointer("/input/command").and_then(Value::as_str) else {
            continue;
        };
        let Some(input) = doc.pointer("/input/input") else {
            report.errors.push(format!(
                "{shown}:1: a provider example's `input` is `{{ command, input }}` — fix: give \
                 the command's input"
            ));
            continue;
        };
        let expect = doc.get("expect").cloned().unwrap_or(Value::Null);
        match client
            .peer
            .call::<_, InvokeResult>(
                method::INVOKE,
                &InvokeParams {
                    handle: handle.clone(),
                    command: command.into(),
                    input: input.clone(),
                    idempotency_key: None,
                    context: None,
                },
            )
            .await
        {
            Ok(out) => {
                // A ref of its own kind it answers with (an item it filed)
                // is left in the provider's system.
                let kind =
                    oxplow_domain::capability::contract(&spec.capability).and_then(|c| c.ref_kind);
                if let Some(made) = out
                    .result
                    .get("ref")
                    .and_then(Value::as_str)
                    .filter(|r| kind.is_some_and(|k| r.starts_with(&format!("{k}:"))))
                {
                    if !report.left.iter().any(|l| l == made) {
                        report.left.push(made.to_string());
                    }
                }
                if let Some((path, want, got)) = first_mismatch(&expect, &out.result) {
                    report.errors.push(format!(
                        "{shown}:1: `{command}` returned {got} at `{path}`, the example expects \
                         {want} — fix: the provider or the example's `expect`"
                    ));
                }
            }
            Err(e) => report.errors.push(format!(
                "{shown}:1: `{command}` failed: {e} — fix: the provider, or the example's input"
            )),
        }
    }
    if !declared.collectors.is_empty() {
        read_back(client, t, &handle, report).await;
    }
}

/// What the provider reads (P7.A7): `discover` lists every entity its
/// collectors declare, and each collector's `read` — from nothing, then
/// from its last `$/state` — streams records of its entity, as many as it
/// says, checkpoints them, and doesn't stream them all again from the
/// checkpoint (a cursor that doesn't advance).
async fn read_back(
    client: &ReferenceClient,
    t: &UnderTest<'_>,
    handle: &oxplow_provider_protocol::model::Handle,
    report: &mut TestReport,
) {
    use oxplow_provider_protocol::codec::notify;
    use oxplow_provider_protocol::model::{DiscoverParams, DiscoverResult, ReadParams, ReadResult};
    let decl_file = format!("{}/{}", t.rel, t.spec.declarations);
    report.ran.push("discover".into());
    match client
        .peer
        .call::<_, DiscoverResult>(
            method::DISCOVER,
            &DiscoverParams {
                handle: handle.clone(),
            },
        )
        .await
    {
        Err(e) => report.errors.push(format!(
            "{decl_file}:1: discover failed: {e} — fix: answer `discover` with the entities its \
             collectors read"
        )),
        Ok(found) => {
            for c in &t.declared.collectors {
                if !found.entities.iter().any(|e| e.name == c.entity) {
                    report.errors.push(format!(
                        "{decl_file}:1: collector `{}` reads entity `{}`, which `discover` \
                         doesn't list — fix: list it, or correct the collector's entity",
                        c.name, c.entity
                    ));
                }
            }
        }
    }
    for c in &t.declared.collectors {
        report.ran.push(format!("read {}", c.name));
        // One read from `state`: what it streamed, and its last checkpoint.
        let read = |state: Option<Value>| async move {
            let (records0, states0) = (
                client.provider_notifications(notify::RECORD).len(),
                client.provider_notifications(notify::STATE).len(),
            );
            let out = client
                .peer
                .call::<_, ReadResult>(
                    method::READ,
                    &ReadParams {
                        handle: handle.clone(),
                        collector: c.name.clone(),
                        state,
                    },
                )
                .await;
            let records = client.provider_notifications(notify::RECORD)[records0..].to_vec();
            let last = client.provider_notifications(notify::STATE)[states0..]
                .last()
                .and_then(|s| s.get("state").cloned());
            out.map(|r| (r.records, records, last))
        };
        let first = match read(None).await {
            Ok(r) => r,
            Err(e) => {
                report.errors.push(format!(
                    "{decl_file}:1: collector `{}`: read failed: {e} — fix: answer `read`",
                    c.name
                ));
                continue;
            }
        };
        let (said, records, last) = first;
        if said != records.len() as u64 {
            report.errors.push(format!(
                "{decl_file}:1: collector `{}`: read says {said} records but streamed {} — fix: \
                 count every `$/record` it sends",
                c.name,
                records.len()
            ));
        }
        if let Some(r) = records
            .iter()
            .find(|r| r.get("entity").and_then(Value::as_str) != Some(c.entity.as_str()))
        {
            report.errors.push(format!(
                "{decl_file}:1: collector `{}`: read streamed a record of `{}`, not its entity \
                 `{}` — fix: stream only what it declares",
                c.name,
                r.get("entity").and_then(Value::as_str).unwrap_or("?"),
                c.entity
            ));
        }
        if records.is_empty() {
            continue;
        }
        let Some(state) = last else {
            report.errors.push(format!(
                "{decl_file}:1: collector `{}`: read streamed {} records but no `$/state` \
                 checkpoint — fix: send `$/state` after each batch",
                c.name,
                records.len()
            ));
            continue;
        };
        match read(Some(state)).await {
            Err(e) => report.errors.push(format!(
                "{decl_file}:1: collector `{}`: a read from its own checkpoint failed: {e} — \
                 fix: accept the `$/state` it sent",
                c.name
            )),
            Ok((_, again, _)) if again.len() >= records.len() => report.errors.push(format!(
                "{decl_file}:1: collector `{}`: its cursor doesn't advance — read again from its \
                 last `$/state`, it streamed {} of its {} records again — fix: checkpoint past \
                 what it streamed",
                c.name,
                again.len(),
                records.len()
            )),
            Ok(_) => {}
        }
    }
}

/// Compare the session with its golden, or write it with `bless`.
fn transcript(
    dir: &Path,
    rel: &str,
    spec: &ProviderSpec,
    recorded: &[(crate::conformance::Side, Value)],
    bless: bool,
    report: &mut TestReport,
) {
    let file = format!("fixtures/transcripts/{}.jsonl", spec.id);
    let shown = format!("{rel}/{file}");
    let lines = normalize(recorded);
    if bless {
        let path = dir.join(&file);
        let written = path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| std::fs::write(&path, lines.join("\n") + "\n"));
        match written {
            Ok(()) => report.blessed.push(shown),
            Err(e) => report.errors.push(format!(
                "{shown}:1: couldn't write it: {e} — fix: check the folder"
            )),
        }
        return;
    }
    let Ok(golden) = std::fs::read_to_string(dir.join(&file)) else {
        report.errors.push(format!(
            "{shown}:1: no golden transcript — fix: run `oxplow extension test <name> --bless` and \
             commit it"
        ));
        return;
    };
    let golden: Vec<&str> = golden.lines().filter(|l| !l.trim().is_empty()).collect();
    for (i, want) in golden.iter().enumerate() {
        let Some(got) = lines.get(i) else {
            report.errors.push(format!(
                "{shown}:{}: the session ended before this message — fix: if the change is \
                 intended, run with --bless",
                i + 1
            ));
            return;
        };
        let (Ok(want_v), Ok(got_v)) = (
            serde_json::from_str::<Value>(want),
            serde_json::from_str::<Value>(got),
        ) else {
            report
                .errors
                .push(format!("{shown}:{}: not JSON — fix: re-bless it", i + 1));
            return;
        };
        if let Some((path, w, g)) = first_mismatch(&want_v, &got_v) {
            report.errors.push(format!(
                "{shown}:{}: the transcript differs at `{path}`: expected {w}, got {g} — fix: \
                 if the change is intended, run with --bless",
                i + 1
            ));
            return;
        }
    }
    if lines.len() > golden.len() {
        report.errors.push(format!(
            "{shown}:{}: the session sent more than the golden has — fix: if the change is \
             intended, run with --bless",
            golden.len() + 1
        ));
    }
}

/// The capability's conformance suite through a throwaway host: an
/// in-memory oxplow over a copy of the extension, the provider approved
/// there (the person running the kit consents) and enabled with the
/// fixture config.
/// Where a harness's launch looks for programs under the kit: `env`'s
/// PATH, then the well-known install dirs under its HOME — what
/// `agent_path::search_path` is for the app's own environment.
fn kit_search_path(env: &host::HostEnv) -> Vec<std::path::PathBuf> {
    let extra = env("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| oxplow_app::agent_path::well_known_bin_dirs(Path::new(&h)))
        .unwrap_or_default();
    oxplow_app::agent_path::search_path_in(env("PATH").as_deref().map(std::ffi::OsStr::new), &extra)
}

/// A throwaway oxplow over a copy of `ext` alone, in `env`: the folder
/// (kept alive by the guard), the services, and the extension as they
/// loaded it.
async fn suite_host(
    root: &Path,
    ext: &Extension,
    env: &host::HostEnv,
) -> Result<(tempfile::TempDir, Arc<oxplow_app::Services>, Extension), String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = tmp.path().join(EXTENSIONS_DIR).join(&ext.name);
    crate::throwaway::copy_dir(&root.join(&ext.path), &target).map_err(|e| e.to_string())?;
    oxplow_app::vcs::GitProvider
        .init_repository(tmp.path())
        .await
        .map_err(|e| e.to_string())?;
    let svc = oxplow_app::Services::in_memory_on_machine(
        tmp.path(),
        tmp.path().join(".oxplow/global-config"),
        Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        env.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let hosted = svc
        .extension_catalog
        .get(tmp.path())
        .iter()
        .find(|e| e.name == ext.name)
        .cloned()
        .ok_or("the copied extension didn't load")?;
    Ok((tmp, svc, hosted))
}

/// The effort-policy suite over the extension's policy script `decl`, in a
/// throwaway host: the script approved there and chosen as the project's
/// policy. A finding is an error at the manifest.
/// Publish the generation the throwaway project's config and files ask
/// for, as the app's owner of generations does on a change.
async fn publish_generation(svc: &Arc<oxplow_app::Services>) -> Result<(), String> {
    svc.extension_activation
        .activate(svc.clone())
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

async fn policy_suite(
    root: &Path,
    ext: &Extension,
    decl: &oxplow_app::extensions::implementations::ImplementationDecl,
    env: &host::HostEnv,
    report: &mut TestReport,
) {
    let manifest = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
    report.ran.push(format!("{EFFORT_POLICY} suite"));
    let run = async {
        let (tmp, svc, hosted) = suite_host(root, ext, env).await?;
        let program = oxplow_app::exec_consent::effort_policy_program(
            &hosted.path,
            &hosted.name,
            &decl.id,
            &decl.entry,
            &decl.needs,
        );
        let version = program
            .hash(tmp.path())
            .map_err(|e| format!("hashing it: {e}"))?;
        oxplow_app::exec_consent::approve_program(
            &svc.approvals,
            tmp.path(),
            &oxplow_app::config_service::read_config(&svc.config),
            std::slice::from_ref(&hosted),
            oxplow_app::exec_consent::ProgramKind::EffortPolicy,
            &program.name,
            &version,
        )?;
        svc.config
            .write()
            .map_err(|e| e.to_string())?
            .active_providers
            .insert(EFFORT_POLICY.into(), decl.id.clone());
        publish_generation(&svc).await?;
        Ok::<_, String>(oxplow_app::effort_policy_conformance::suite(&svc, &decl.id).await)
    };
    match run.await {
        Ok(run) => {
            report.left.extend(run.left);
            for f in run.findings {
                report.errors.push(format!(
                    "{manifest}:1: {EFFORT_POLICY} conformance `{}`: {} — fix: the policy \
                     script's `{}` handling",
                    f.check, f.message, f.check
                ));
            }
        }
        Err(e) => report.errors.push(format!(
            "{manifest}:1: effort policy `{}` couldn't run under a host: {e} — fix: see the message",
            decl.id
        )),
    }
}

async fn suite(
    root: &Path,
    ext: &Extension,
    spec: &ProviderSpec,
    config: Value,
    env: &host::HostEnv,
    report: &mut TestReport,
) {
    let manifest = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
    report.ran.push(format!("{} suite", spec.capability));
    let run = async {
        let (tmp, svc, hosted) = suite_host(root, ext, env).await?;
        let program = oxplow_app::exec_consent::provider_program(&hosted, spec);
        let version = program
            .hash(tmp.path())
            .map_err(|e| format!("hashing it: {e}"))?;
        oxplow_app::exec_consent::approve_program(
            &svc.approvals,
            tmp.path(),
            &oxplow_app::config_service::read_config(&svc.config),
            std::slice::from_ref(&hosted),
            oxplow_app::exec_consent::ProgramKind::Provider,
            &program.name,
            &version,
        )?;
        let project = oxplow_app::collector_runner::project_key(tmp.path());
        for (name, value) in env_credentials(spec, env) {
            svc.secrets
                .set(
                    &oxplow_app::collector_runner::instance_credential_account(
                        &project, &ext.name, &spec.id, &name,
                    ),
                    &value,
                )
                .map_err(|e| e.to_string())?;
        }
        svc.config
            .write()
            .map_err(|e| e.to_string())?
            .extension_instances
            .insert(
                format!("{}/{}", ext.name, spec.id),
                oxplow_config::ExtensionInstanceConfig {
                    enabled: true,
                    config,
                    sync_minutes: None,
                    provider: None,
                },
            );
        publish_generation(&svc).await?;
        match svc.providers.health(&format!("{}/{}", ext.name, spec.id)) {
            // A start that merely failed stays enabled; the suite reports it.
            Some(health)
                if matches!(
                    health.state,
                    oxplow_app::providers::InstanceState::Ready
                        | oxplow_app::providers::InstanceState::Failing { .. }
                ) => {}
            other => return Err(format!("it didn't start: {other:?}")),
        }
        let findings = match spec.capability.as_str() {
            providers::work_items::WorkItemsHost::CAPABILITY => {
                let provider = svc.work_items.get(&spec.id).map_err(|e| e.to_string())?;
                // Every create files on the active tracker (tsk1058): the
                // throwaway host's is the one under test.
                svc.config
                    .write()
                    .map_err(|e| e.to_string())?
                    .active_providers
                    .insert("work_items".into(), provider.id.clone());
                // …and published, as a `oxplow.config.set` would: the interface
                // reads the active list's items.
                publish_generation(&svc).await?;
                oxplow_app::work_items_conformance::suite(
                    &svc.work_items_client(),
                    &provider.id,
                    provider.features,
                    None,
                    &oxplow_app::work_items_conformance::ServicesProbe(&svc),
                    &Actor::Human,
                )
                .await
            }
            providers::effort_policy::EffortPolicyHost::CAPABILITY => {
                // The instance under test is the project's effort policy.
                svc.config
                    .write()
                    .map_err(|e| e.to_string())?
                    .active_providers
                    .insert(spec.capability.clone(), spec.id.clone());
                publish_generation(&svc).await?;
                oxplow_app::effort_policy_conformance::suite(&svc, &spec.id).await
            }
            providers::agent_harness::HarnessHost::CAPABILITY => {
                // Its launch looks for programs where the kit's
                // environment would: its PATH, then the install dirs.
                oxplow_app::harness_conformance::suite(
                    &svc,
                    &spec.id,
                    &json!({}),
                    &kit_search_path(env),
                )
                .await
            }
            providers::knowledge::KnowledgeHost::CAPABILITY => {
                // The instance under test is the project's knowledge store;
                // its record follows it once the switch is delivered, which
                // must happen before the suite writes (a late delivery
                // would empty what it wrote).
                svc.config
                    .write()
                    .map_err(|e| e.to_string())?
                    .active_providers
                    .insert(spec.capability.clone(), spec.id.clone());
                publish_generation(&svc).await?;
                svc.event_pump.run_once().await.map_err(|e| e.to_string())?;
                oxplow_app::work_items_conformance::SuiteRun {
                    findings: oxplow_app::knowledge_conformance::suite(&svc, &Actor::Human).await,
                    left: Vec::new(),
                }
            }
            providers::snapshots::SnapshotsHost::CAPABILITY => {
                // The instance under test is the project's snapshots, and
                // the captures follow it now (the throwaway host's pump
                // may not have delivered the switch yet).
                svc.config
                    .write()
                    .map_err(|e| e.to_string())?
                    .active_providers
                    .insert(spec.capability.clone(), spec.id.clone());
                publish_generation(&svc).await?;
                oxplow_app::snapshots::apply_active_policy(&svc.snapshots, &svc.snapshot_captures);
                let provider = svc
                    .snapshots
                    .get(&spec.id)
                    .ok_or_else(|| format!("`{}` isn't a snapshot implementation", spec.id))?;
                let stream = svc
                    .snapshot_captures
                    .primary()
                    .ok_or("the throwaway project has no worktree to mark")?
                    .stream_id()
                    .to_owned();
                oxplow_app::snapshot_conformance::suite(&svc, provider.as_ref(), stream).await
            }
            other => return Err(format!("no conformance suite for `{other}`")),
        };
        svc.providers.stop(&spec.approval_name(&ext.name)).await;
        Ok::<_, String>(findings)
    };
    match run.await {
        Ok(run) => {
            report.left.extend(run.left);
            for f in run.findings {
                report.errors.push(format!(
                    "{manifest}:1: {} conformance `{}`: {} — fix: the provider's `{}` handling",
                    spec.capability, f.check, f.message, f.check
                ));
            }
        }
        Err(e) => report.errors.push(format!(
            "{manifest}:1: provider `{}` couldn't run under a host: {e} — fix: see the message",
            spec.id
        )),
    }
}

/// The report as text (`error:` / `warning:` lines, then a summary) or
/// JSON.
pub fn render(report: &TestReport, format: crate::Format) -> String {
    match format {
        crate::Format::Json => serde_json::to_string_pretty(report).expect("report serializes"),
        crate::Format::Text => {
            let mut out = String::new();
            for e in &report.errors {
                out.push_str(&format!("error: {e}\n"));
            }
            for w in &report.warnings {
                out.push_str(&format!("warning: {w}\n"));
            }
            for b in &report.blessed {
                out.push_str(&format!("blessed: {b}\n"));
            }
            for l in &report.left {
                out.push_str(&format!("left in the provider: {l}\n"));
            }
            for i in &report.imports {
                out.push_str(&format!("import {i}\n"));
            }
            for line in &report.profiles {
                out.push_str(&format!("{line}\n"));
            }
            if !report.unsupported.is_empty() {
                out.push_str(&format!(
                    "not run here (the app shows them): {}\n",
                    report.unsupported.join(", ")
                ));
            }
            if let Some(generation) = &report.generation {
                out.push_str(&format!("generation: {generation}\n"));
            }
            out.push_str(&format!(
                "{}: {} error{}, {} warning{}; ran {}\n",
                report.name,
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" },
                report.warnings.len(),
                if report.warnings.len() == 1 { "" } else { "s" },
                report.ran.join(", ")
            ));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// The tally extension: a starlark collector of `thing`s, a model and a
    /// lens over them, a command noting one, a README and questions, and an
    /// intent example per kind, each with its fixture.
    fn tally(root: &Path, collector_expect: &str, command_expect: &str) {
        write(
            root,
            "oxplow/extensions/tally/extension.yaml",
            &format!(
                "manifest: 2
name: tally
intent:
  purpose: Count things.
  examples:
    - {{ name: labelled, input: {{ lens: labelled }}, expect: no labelled things yet }}
    - {{ name: things, input: {{ collector: things }}, expect: one thing per input row }}
    - {{ name: shell, input: {{ collector: shell }}, expect: whatever the shell says }}
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT ref FROM v_work_item\"
    entities:
      - {{ name: thing, key: id, columns: {{ id: int, label: text }} }}
  - id: shell
    runtime: exec
    entry: collectors/shell.sh
    entities:
      - {{ name: line, key: n, columns: {{ n: int }} }}
models:
  - name: labelled
    version: 1
    description: Labelled things.
    columns:
      - {{ name: id, type: INTEGER, doc: The thing. }}
      - {{ name: label, type: TEXT, doc: Its label. }}
commands:
  - name: work.note
    summary: Note a work item.
    input_schema: {{ type: object, required: [ref], properties: {{ ref: {{ type: string }} }} }}
    entry: handlers/note.star
    invokers: {{ human: true, agent: true, lens: true }}
    examples:
      - {{ name: happy, input: {{ ref: \"work_item:oxplow:tsk1\" }}, expect_commands: {command_expect} }}
"
            ),
        );
        write(
            root,
            "oxplow/extensions/tally/collectors/things.star",
            "def transform(x):\n    return {\"entities\": {\"thing\": [{\"id\": r[\"id\"], \"label\": \"t\"} for r in x[\"rows\"]]}}\n",
        );
        write(
            root,
            "oxplow/extensions/tally/collectors/shell.sh",
            "#!/bin/sh\necho '{}'\n",
        );
        write(
            root,
            "oxplow/extensions/tally/models/labelled.sql",
            "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
        );
        write(
            root,
            "oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.comment\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"body\": \"noted\"}}]}\n",
        );
        write(
            root,
            "oxplow/extensions/tally/lenses/labelled.yaml",
            "title: Labelled\nquery: SELECT id, label FROM v_tally_labelled\n",
        );
        write(
            root,
            "oxplow/extensions/tally/fixtures/labelled.yaml",
            "input: { lens: labelled }\nexpect: { columns: [id, label], rows: 0 }\n",
        );
        write(
            root,
            "oxplow/extensions/tally/fixtures/things.yaml",
            &format!(
                "input: {{ collector: things, rows: [{{ id: 1 }}, {{ id: 2 }}] }}\nexpect: {collector_expect}\n"
            ),
        );
        write(
            root,
            "oxplow/extensions/tally/fixtures/shell.yaml",
            "input: { collector: shell }\nexpect: { entities: { line: $any } }\n",
        );
        write(
            root,
            "oxplow/extensions/tally/README.md",
            "Read `v_tally_labelled`; note a work item with `tally.work.note`.\n",
        );
        write(
            root,
            "oxplow/extensions/tally/questions.yaml",
            "- question: Which things are labelled?\n  skill: README.md\n  reaches:\n    sql: SELECT id, label FROM v_tally_labelled\n  shape: { columns: [id, label] }\n- question: Note a work item.\n  skill: README.md\n  reaches:\n    command: tally.work.note\n    input: { ref: \"work_item:oxplow:tsk1\" }\n",
        );
    }

    /// A collector that calls a model is a program a person approves; the
    /// throwaway approves it as a person would, and its example runs
    /// against the fixture model — so it never spends a key.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_runs_a_collector_that_calls_a_model_against_the_fixture_model() {
        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 2 } }",
            "[oxplow.work_item.comment]",
        );
        write(
            dir.path(),
            "oxplow/extensions/tally/collectors/things.star",
            "def transform(x):\n    return {\"entities\": {\"thing\": [{\"id\": r[\"id\"], \"label\": ai_summarize(\"t\")} for r in x[\"rows\"]]}}\n",
        );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(
            report.ran.iter().any(|r| r == "example things"),
            "{:?}",
            report.ran
        );
        let warnings = report.warnings.join("\n");
        assert!(!warnings.contains("calls a model"), "{warnings}");
    }

    /// P7.C6: `extension test` runs every kind's examples on a throwaway
    /// oxplow — a lens's rows, a starlark collector's typed entities from
    /// its fixture rows, a command's composition against the real
    /// registry — and its questions may name its own commands. An exec
    /// collector's example isn't run (it needs a person's approval).
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_runs_every_kinds_examples() {
        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 2 } }",
            "[oxplow.work_item.comment]",
        );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        for ran in ["check", "example labelled", "example things", "questions"] {
            assert!(
                report.ran.iter().any(|r| r == ran),
                "{ran}: {:?}",
                report.ran
            );
        }
        let warnings = report.warnings.join("\n");
        assert!(
            warnings.contains(
                "fixtures/shell.yaml:1: example `shell` runs collector `shell`, which runs a program"
            ),
            "{warnings}"
        );

        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 3 } }",
            "[oxplow.work_item.comment]",
        );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains(
                "fixtures/things.yaml:1: collector `things` returned 2 at `/entities/thing`"
            ),
            "{errors}"
        );

        // A command's example is checked against the throwaway's registry.
        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 2 } }",
            "[oxplow.work_item.transition]",
        );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains("example `happy`: composed [oxplow.work_item.comment] but `expect_commands` is [oxplow.work_item.transition]"),
            "{errors}"
        );
    }

    /// A producer exporting `items` at `version`, and a consumer reading it
    /// through an optional import of version 1.
    fn producer_and_consumer(root: &Path, version: u32) {
        write(
            root,
            "oxplow/extensions/maker/extension.yaml",
            &format!(
                "manifest: 2\nname: maker\nintent:\n  purpose: Make items.\nmodels:\n  - name: items\n    version: {version}\n    description: Items.\n    columns:\n      - {{ name: ref, type: TEXT, doc: A work item. }}\nexports:\n  - {{ contribution: model:maker/items, version: {version}, visibility: stable }}\n"
            ),
        );
        write(
            root,
            "oxplow/extensions/maker/models/items.sql",
            "SELECT ref FROM ref('work_item')\n",
        );
        write(
            root,
            "oxplow/extensions/reader/extension.yaml",
            "manifest: 2\nname: reader\nintent:\n  purpose: Read the maker's items.\nimports:\n  - { id: items, contribution: model:maker/items, version: 1, optional: true }\n",
        );
        write(
            root,
            "oxplow/extensions/reader/lenses/seen.yaml",
            "title: Seen\nquery: SELECT ref FROM import_ref('items')\n",
        );
    }

    /// `extension test` says how each import resolved: available, or why
    /// not (the producer disabled, or exporting another version).
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_reports_how_each_import_resolved() {
        let dir = tempfile::tempdir().unwrap();
        producer_and_consumer(dir.path(), 1);
        let report = test_extension(dir.path(), "reader", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        assert_eq!(
            report.imports,
            ["`items` → model:maker/items v1: available"]
        );
        assert!(render(&report, crate::Format::Text)
            .contains("import `items` → model:maker/items v1: available\n"));
        let checked = crate::check(
            dir.path(),
            "reader",
            &oxplow_app::extension_catalog::ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(crate::render_findings(&checked, crate::Format::Text)
            .contains("import `items` → model:maker/items v1: available\n"));

        let dir = tempfile::tempdir().unwrap();
        producer_and_consumer(dir.path(), 2);
        let report = test_extension(dir.path(), "reader", false).await.unwrap();
        assert_eq!(
            report.imports,
            ["`items` → model:maker/items v1: unavailable (maker exports v2)"]
        );

        let dir = tempfile::tempdir().unwrap();
        producer_and_consumer(dir.path(), 1);
        write(
            dir.path(),
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [maker]\n",
        );
        let report = test_extension(dir.path(), "reader", false).await.unwrap();
        assert_eq!(
            report.imports,
            ["`items` → model:maker/items v1: unavailable (maker is disabled)"]
        );
    }

    /// A read command's example checks what it answers (`expect: {
    /// result }`), only the keys the example names, beside or instead of
    /// what it composes; a different answer is a finding.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_commands_example_checks_its_result() {
        for (want, ok) in [("{ count: 2, of: things }", true), ("{ count: 3 }", false)] {
            let dir = tempfile::tempdir().unwrap();
            tally(
                dir.path(),
                "{ entities: { thing: 2 } }",
                "[oxplow.work_item.comment]",
            );
            let manifest = dir.path().join("oxplow/extensions/tally/extension.yaml");
            let yaml = std::fs::read_to_string(&manifest).unwrap().replace(
                "    - {{ name: shell,".replace("{{", "{").as_str(),
                "    - { name: counted, input: { command: tally.things.count }, expect: the things counted }\n    - { name: shell,",
            ) + "  - name: things.count\n    summary: How many things there are.\n    entry: handlers/count.star\n    needs: [sql.read]\n    access: read\n    input_schema: { type: object }\n    invokers: { human: true, agent: true, lens: true }\n";
            std::fs::write(&manifest, yaml).unwrap();
            write(
                dir.path(),
                "oxplow/extensions/tally/handlers/count.star",
                "def transform(x):\n    rows = scope(\"sql.read\", {\"sql\": \"SELECT id FROM v_tally_labelled\"})\n    return {\"commands\": [], \"result\": {\"count\": len(rows), \"of\": \"things\"}}\n",
            );
            write(
                dir.path(),
                "oxplow/extensions/tally/fixtures/counted.yaml",
                &format!(
                    "input: {{ command: tally.things.count, input: {{}}, answers: {{ sql.read: [[{{ id: 1 }}, {{ id: 2 }}]] }} }}\nexpect: {{ result: {want} }}\n"
                ),
            );
            let report = test_extension(dir.path(), "tally", false).await.unwrap();
            let errors = report.errors.join("\n");
            if ok {
                assert_eq!(errors, "", "{want}");
                assert!(
                    report.ran.iter().any(|r| r == "example counted"),
                    "{:?}",
                    report.ran
                );
            } else {
                assert!(
                    errors.contains("fixtures/counted.yaml:1: command `tally.things.count` answered 2 at `/result/count`, the example expects 3"),
                    "{errors}"
                );
            }
        }
    }

    /// An extension with one incremental model over page visits, watermarked
    /// on `watermark`, and its `before` / `after` fixture.
    /// The `visits` extension: an incremental model keyed and watermarked
    /// on `id`, with a fixture whose `before` visit has id `before` and whose
    /// later `after` visit has id `after`.
    fn visits(root: &Path, before: i64, after: i64) {
        write(
            root,
            "oxplow/extensions/visits/extension.yaml",
            "manifest: 2
name: visits
intent:
  purpose: Long visits.
  origin: thread:thr1
  examples: []
models:
  - name: long_visits
    version: 1
    description: Visits and how long they lasted.
    key: [id]
    materialize: { incremental: id }
    columns:
      - { name: id, type: INTEGER, doc: The visit. }
      - { name: duration_ms, type: INTEGER, doc: How long it lasted. }
",
        );
        write(
            root,
            "oxplow/extensions/visits/models/long_visits.sql",
            "SELECT id, duration_ms FROM ref('page_visit') WHERE duration_ms IS NOT NULL\n",
        );
        write(
            root,
            "oxplow/extensions/visits/fixtures/model-long_visits.yaml",
            &format!(
                "before:
  page_visit:
    - {{ id: {before}, page_kind: task, page_id: \"task:1\", visited_at: \"2026-01-01T00:00:00.000000Z\", duration_ms: 50 }}
after:
  page_visit:
    - {{ id: {after}, page_kind: task, page_id: \"task:2\", visited_at: \"2026-01-01T00:00:01.000000Z\", duration_ms: 10 }}
"
            ),
        );
    }

    /// P8.B5: each incremental model is appended to on its fixture's
    /// `after` rows and checked against a full refill — a key that doesn't
    /// grow with arrivals (a row lands below the watermark, unseen) fails
    /// at the model's line; one that does passes.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_checks_incremental_models_against_a_full_refill() {
        let dir = tempfile::tempdir().unwrap();
        visits(dir.path(), 1, 2);
        let report = test_extension(dir.path(), "visits", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(
            report.ran.iter().any(|r| r == "incremental long_visits"),
            "{:?}",
            report.ran
        );

        let dir = tempfile::tempdir().unwrap();
        // A visit that arrives with a smaller id than one already appended.
        visits(dir.path(), 2, 1);
        let report = test_extension(dir.path(), "visits", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains(
                "oxplow/extensions/visits/extension.yaml:8: incremental model `long_visits`"
            ) && errors.contains("1 row(s) a full refill holds"),
            "{errors}"
        );
    }

    /// P8.D12: a scaffolded effect checks and tests clean; a fixture that
    /// expects other commands is a finding at its `file:line`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_scaffolded_effect_tests_clean_and_a_mismatch_names_its_fixture() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
        assert!(
            report.ran.contains(&"example basic".to_string()),
            "{:?}",
            report.ran
        );

        let fixture = dir
            .path()
            .join("oxplow/extensions/notes/fixtures/basic.yaml");
        let text = std::fs::read_to_string(&fixture).unwrap();
        std::fs::write(
            &fixture,
            text.replace(
                "commands: [oxplow.work_item.comment]",
                "commands: [oxplow.knowledge.add_note]",
            ),
        )
        .unwrap();
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains(
                "oxplow/extensions/notes/fixtures/basic.yaml:1: effect `on-done` composed"
            ),
            "{errors}"
        );
    }

    /// A policy that opens an item's effort when it starts on a thread,
    /// and closes its open efforts (read through `scope`) when it ends.
    const TIDY: &str = r#"
def transform(x):
    e = x["event"]
    if e["type"] != "work_item.state_changed":
        return {"skip": "not an item's move"}
    item = e["payload"]["work_item"]
    to = e["payload"]["to"]
    thread = e["anchors"].get("thread_id")
    if to == "in_progress" and thread != None:
        return {"commands": [{"name": "oxplow.effort.open",
                              "input": {"thread": ref.build("thread", thread), "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {"sql": "SELECT ref FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
                                  "params": {"w": item}})
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": r["ref"], "reason": "switch"}}
                             for r in rows]}
    return {"skip": "a move that changes no effort"}
"#;

    /// The acme extension: `script` as the effort policy `tidy`, with an
    /// example per reaction — a checkpoint (`basic`, expecting
    /// `basic_expect`), a start on a thread, a finish whose read is
    /// answered.
    fn policy(root: &Path, script: &str, basic_expect: &str) {
        write(
            root,
            "oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
sharing: private
intent:
  purpose: Open and close efforts as items start and finish.
  examples:
    - { name: basic, expect: nothing to do }
    - { name: start, expect: its effort opens }
    - { name: finish, expect: its effort closes }
implementations:
  - { capability: effort_policy, id: tidy, title: Tidy, entry: policies/tidy.star, needs: [sql.read] }
",
        );
        write(root, "oxplow/extensions/acme/policies/tidy.star", script);
        write(
            root,
            "oxplow/extensions/acme/fixtures/basic.yaml",
            &format!(
                "input: {{ implementation: tidy, event: {{ type: thread.checkpoint, payload: {{ thread: \"thread:thr1\", turn: \"turn:trn1\", reason: turn_end, snapshot: \"snapshot:1\", changed: false, writing_tools: 0 }} }} }}\nexpect: {basic_expect}\n"
            ),
        );
        write(
            root,
            "oxplow/extensions/acme/fixtures/start.yaml",
            "input: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: in_progress }, anchors: { thread_id: thr1 } } }\nexpect: { commands: [oxplow.effort.open] }\n",
        );
        write(
            root,
            "oxplow/extensions/acme/fixtures/finish.yaml",
            "input: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: done } }, answers: { sql.read: [[{ ref: \"effort:eff3\" }]] } }\nexpect: { commands: [oxplow.effort.close] }\n",
        );
    }

    /// A scripted effort policy's examples dry-run its script (a read
    /// answered by the fixture), and the effort-policy suite runs over it
    /// in a throwaway host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_policy_script_runs_its_examples_and_the_suite() {
        let dir = tempfile::tempdir().unwrap();
        policy(dir.path(), TIDY, "{ skip: $any }");
        let report = test_extension(dir.path(), "acme", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
        for ran in [
            "example basic",
            "example start",
            "example finish",
            "effort_policy suite",
        ] {
            assert!(
                report.ran.iter().any(|r| r == ran),
                "{ran}: {:?}",
                report.ran
            );
        }
    }

    /// A policy example that doesn't match what the script answers is a
    /// finding at its fixture; one naming no policy of the extension names
    /// what it named.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_policy_example_that_doesnt_match_names_its_fixture() {
        let dir = tempfile::tempdir().unwrap();
        policy(dir.path(), TIDY, "{ commands: [oxplow.effort.open] }");
        let finish = dir
            .path()
            .join("oxplow/extensions/acme/fixtures/finish.yaml");
        let text = std::fs::read_to_string(&finish).unwrap();
        std::fs::write(
            &finish,
            text.replace("implementation: tidy", "implementation: nope"),
        )
        .unwrap();
        let report = test_extension(dir.path(), "acme", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains("acme/fixtures/basic.yaml:1: effort policy `tidy` skipped"),
            "{errors}"
        );
        assert!(
            errors.contains(
                "acme/fixtures/finish.yaml:1: example `finish` names effort policy `nope`"
            ),
            "{errors}"
        );
    }

    /// An example's event is the event envelope the runtime would hand
    /// over: what the fixture doesn't give is `null` (no made-up id or
    /// seq), `source` is `fixture`; and a fixture that couldn't happen —
    /// an unknown type, a version newer than registered, a malformed
    /// anchor, a key the envelope doesn't have — is a finding at it.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_example_event_is_the_envelope_and_one_that_couldnt_happen_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let shape = "def transform(x):\n    e = x[\"event\"]\n    return {\"skip\": \"%s %s %s %s %s\" % (e[\"v\"], e[\"source\"], e[\"id\"], e[\"seq\"], e[\"anchors\"][\"turn_id\"])}\n";
        policy(root, shape, "{ skip: \"1 fixture None None None\" }");
        // `start` and `finish` aren't this script's; only `basic` stays.
        for gone in ["start", "finish"] {
            std::fs::remove_file(root.join(format!("oxplow/extensions/acme/fixtures/{gone}.yaml")))
                .unwrap();
        }
        let manifest = root.join("oxplow/extensions/acme/extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            text.replace(
                "    - { name: start, expect: its effort opens }\n    - { name: finish, expect: its effort closes }\n",
                "",
            ),
        )
        .unwrap();
        let report = test_extension(root, "acme", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(!errors.contains("fixtures/basic.yaml"), "{errors}");

        for (event, needle) in [
            (
                "{ type: acme.nope }",
                "`acme.nope` is not a registered event type",
            ),
            (
                "{ type: thread.checkpoint, v: 9, payload: {} }",
                "newer than the registered v1",
            ),
            (
                "{ type: thread.checkpoint, payload: {}, anchors: { turn_id: seven } }",
                "`anchors.turn_id`",
            ),
            (
                "{ type: thread.checkpoint, payload: {}, subjects: [] }",
                "unknown field `subjects`",
            ),
        ] {
            write(
                root,
                "oxplow/extensions/acme/fixtures/basic.yaml",
                &format!(
                    "input: {{ implementation: tidy, event: {event} }}\nexpect: {{ skip: $any }}\n"
                ),
            );
            let report = test_extension(root, "acme", false).await.unwrap();
            let errors = report.errors.join("\n");
            assert!(
                errors.contains("acme/fixtures/basic.yaml:1: example `basic`: event: ")
                    && errors.contains(needle),
                "{event}: {errors}"
            );
        }
    }

    /// An effect example's event at an older version of the extension's
    /// own type is carried to the newest through its upcast, as a logged
    /// one would be.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_effect_example_at_an_older_version_is_upcast() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
sharing: private
intent:
  purpose: Note merges.
  examples:
    - { name: old, expect: an old merge is read at v2 }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: A merge. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: A merge by someone., upcast: merged.star }
effects:
  - { id: note, summary: Note a merge., on: [acme.merged], entry: note.star }
",
        );
        write(
            root,
            "oxplow/extensions/acme/merged.v1.json",
            r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
        );
        write(
            root,
            "oxplow/extensions/acme/merged.v2.json",
            r#"{"type": "object", "required": ["number", "by"], "properties": {"number": {"type": "integer"}, "by": {"type": "string"}}}"#,
        );
        write(
            root,
            "oxplow/extensions/acme/merged.star",
            "def transform(x):\n    p = dict(x[\"payload\"])\n    p[\"by\"] = \"unknown\"\n    return p\n",
        );
        write(
            root,
            "oxplow/extensions/acme/note.star",
            "def transform(x):\n    e = x[\"event\"]\n    return {\"skip\": \"v%d by %s\" % (e[\"v\"], e[\"payload\"][\"by\"])}\n",
        );
        write(
            root,
            "oxplow/extensions/acme/fixtures/old.yaml",
            "input: { effect: note, event: { type: acme.merged, v: 1, payload: { number: 3 } } }\nexpect: { skip: v2 by unknown }\n",
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
        assert!(
            report.ran.iter().any(|r| r == "example old"),
            "{:?}",
            report.ran
        );
    }

    /// A policy that opens efforts and never closes one fails the suite,
    /// at the manifest.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_policy_script_that_never_closes_fails_the_suite() {
        let dir = tempfile::tempdir().unwrap();
        let opens_only = TIDY.replace(r#"if to == "done" or to == "canceled":"#, "if False:");
        policy(dir.path(), &opens_only, "{ skip: $any }");
        let report = test_extension(dir.path(), "acme", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains("acme/extension.yaml:1: effort_policy conformance `a_finished_item_closes_its_effort`"),
            "{errors}"
        );
    }

    /// Event-type and ref-kind fixtures: a payload checked against the
    /// declared schema (and its upcast), a wikilink against the declared
    /// kind; a wrong expectation is a finding at its fixture.
    #[tokio::test(flavor = "multi_thread")]
    async fn event_type_and_ref_kind_fixtures_check_what_they_declare() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
sharing: private
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged, payload: { pr: 12 } }, expect: valid }
    - { name: old, input: { event_type: acme.merged, v: 1, payload: { number: 12 } }, expect: upcast }
    - { name: link, input: { wikilink: pr:12 }, expect: the pull request }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: Merged. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: Merged., upcast: merged.star }
models:
  - name: prs
    version: 1
    description: PRs.
    columns:
      - { name: ref, type: \"\", doc: The ref. }
      - { name: title, type: \"\", doc: Its title. }
pages:
  - { id: pr, title: Pull request, category: Work, lens: open }
ref_kinds:
  - { kind: acme_pr, label: Pull request, id: '^\\d+$', resolve: prs, page: pr, wikilink: pr, icon: git-pull-request }
",
        );
        write(
            root,
            "oxplow/extensions/acme/merged.v1.json",
            r#"{"type": "object", "required": ["number"]}"#,
        );
        write(
            root,
            "oxplow/extensions/acme/merged.v2.json",
            r#"{"type": "object", "required": ["pr"]}"#,
        );
        write(
            root,
            "oxplow/extensions/acme/merged.star",
            "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"]}\n",
        );
        write(
            root,
            "oxplow/extensions/acme/models/prs.sql",
            "SELECT 'acme_pr:1' AS ref, 'One' AS title",
        );
        write(
            root,
            "oxplow/extensions/acme/lenses/open.yaml",
            "title: Open\nquery: \"SELECT 1 AS n\"\n",
        );
        write(
            root,
            "oxplow/extensions/acme/fixtures/merged.yaml",
            "input: { event_type: acme.merged, payload: { pr: 12 } }\nexpect: { valid: true }\n",
        );
        write(root, "oxplow/extensions/acme/fixtures/old.yaml", "input: { event_type: acme.merged, v: 1, payload: { number: 12 } }\nexpect: { valid: true, upcast: { pr: 12 } }\n");
        write(
            root,
            "oxplow/extensions/acme/fixtures/link.yaml",
            "input: { wikilink: \"pr:12\" }\nexpect: { ref: \"acme_pr:12\" }\n",
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
        for name in ["merged", "old", "link"] {
            assert!(
                report.ran.contains(&format!("example {name}")),
                "{:?}",
                report.ran
            );
        }

        write(root, "oxplow/extensions/acme/fixtures/merged.yaml", "input: { event_type: acme.merged, payload: { number: 12 } }\nexpect: { valid: true }\n");
        write(
            root,
            "oxplow/extensions/acme/fixtures/link.yaml",
            "input: { wikilink: \"pr:12\" }\nexpect: { ref: \"acme_pr:13\" }\n",
        );
        let errors = test_extension(root, "acme", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors
                .contains("oxplow/extensions/acme/fixtures/merged.yaml:1: `acme.merged@2` refuses"),
            "{errors}"
        );
        assert!(
            errors.contains("oxplow/extensions/acme/fixtures/link.yaml:1: `[[pr:12]]` names"),
            "{errors}"
        );
    }

    /// A `viz: form` lens is opened as a person would: its defaults bound
    /// from the params, its fixed and shown fields; a wrong expectation is
    /// a finding at its fixture.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_opens_a_form_lens_and_compares_what_it_starts_with() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "oxplow/extensions/editor/extension.yaml",
            "manifest: 2
name: editor
sharing: private
intent:
  purpose: Edit work items.
  examples:
    - { name: edit, input: { form: edit-item, params: { ref: 'work_item:oxplow:tsk1' } }, expect: the ref is fixed }
",
        );
        write(
            root,
            "oxplow/extensions/editor/lenses/edit-item.yaml",
            "title: Edit\nviz: form\nparams: [{ name: ref }]\nquery: SELECT title, body FROM v_work_item WHERE ref = :ref\nform: { command: oxplow.work_item.update, defaults: { ref: '{{param.ref}}' }, fixed: [ref], fields: [title, body] }\n",
        );
        write(
            root,
            "oxplow/extensions/editor/fixtures/edit.yaml",
            "input: { form: edit-item, params: { ref: 'work_item:oxplow:tsk1' } }\nexpect: { values: { ref: 'work_item:oxplow:tsk1' }, fixed: [ref], fields: [title, body] }\n",
        );
        let report = test_extension(root, "editor", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(
            report.ran.contains(&"example edit".to_string()),
            "{:?}",
            report.ran
        );

        write(
            root,
            "oxplow/extensions/editor/fixtures/edit.yaml",
            "input: { form: edit-item, params: { ref: 'work_item:oxplow:tsk1' } }\nexpect: { fixed: [ref, title] }\n",
        );
        let errors = test_extension(root, "editor", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains(
                "oxplow/extensions/editor/fixtures/edit.yaml:1: form `edit-item` opened with"
            ),
            "{errors}"
        );
    }
}
