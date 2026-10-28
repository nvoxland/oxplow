//! `oxplow extension test` (P5.D5, P7.C6; `.context/extensions.md` "The
//! SDK"; `.context/providers.md` "The conformance kit"). Everything runs
//! on a throwaway oxplow — an in-memory one over a copy of the project's
//! `.oxplow/extensions/` ([`Host`]) — so it never touches the project's
//! data: `check` (against its real command registry, so `commands:`
//! examples dry-run), then each intent example whose fixture names a page
//! (`when: { page, params? }`, `expect: { columns?, rows }`), a derived
//! collector (`when: { collector, rows? }`, `expect: { entities: { <name>:
//! n }, events?: [{ type, payload? }] }`; one that calls a model runs against the fixture model in the throwaway; an exec collector's isn't run — it is a program under the extension's `process.run` permission),
//! a fact collector (`when: { collector, files: { <path>: <text> } }`,
//! `expect: { facts: [{ measure, value?, path?, … }] }`, tsk1047),
//! or one of its own commands (`when: { command, input, rows? }`,
//! `expect: { commands: [names] }` and/or `{ result }` — only the keys it
//! names — or `{ refused }`, dry-run),
//! the extension's `questions.yaml`, and for each declared provider —
//!
//! 1. the live `initialize` equals its declarations file;
//! 2. `check` accepts the instance config in
//!    `fixtures/provider-<id>.yaml` (`config: { … }`);
//! 3. each `intent.examples[*]` with a fixture `fixtures/<name>.yaml`
//!    (`when: { command, input }`, `expect`) is invoked and its result
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

use oxplow_app::extension::extensions::Extension;
use oxplow_app::providers::host::{self, Launch};
use oxplow_app::providers::{self, ProviderSpec};
use oxplow_domain::Actor;
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, InitializeResult, InvokeParams, InvokeResult,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::conformance::{first_mismatch, normalize, ReferenceClient, ANY};
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
    /// Each thing that ran and how it went: `check`, `example <name>`,
    /// `answers <page>`, `provider <id>`, … — a test case each in the JUnit
    /// form.
    pub cases: Vec<TestCase>,
}

/// One thing `extension test` ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub name: String,
    pub ok: bool,
    /// The errors it added, one per line; `None` when it passed.
    pub message: Option<String>,
    pub duration_ms: u64,
}

impl TestReport {
    /// An empty report for part of extension `name`'s run, to [`Self::absorb`].
    fn part(name: &str) -> TestReport {
        TestReport {
            name: name.to_string(),
            ok: false,
            errors: Vec::new(),
            warnings: Vec::new(),
            blessed: Vec::new(),
            ran: Vec::new(),
            imports: Vec::new(),
            profiles: Vec::new(),
            generation: None,
            contributes: Vec::new(),
            unsupported: Vec::new(),
            left: Vec::new(),
            cases: Vec::new(),
        }
    }

    /// What `part` found and ran, after what this report has.
    fn absorb(&mut self, part: TestReport) {
        self.errors.extend(part.errors);
        self.warnings.extend(part.warnings);
        self.blessed.extend(part.blessed);
        self.ran.extend(part.ran);
        self.left.extend(part.left);
        self.cases.extend(part.cases);
    }

    /// Record case `name`, which started at `started` when the report had
    /// `errors_before` errors: it passed when it added none.
    fn finish_case(&mut self, name: String, errors_before: usize, started: std::time::Instant) {
        let added = &self.errors[errors_before.min(self.errors.len())..];
        self.cases.push(TestCase {
            name,
            ok: added.is_empty(),
            message: (!added.is_empty()).then(|| added.join("\n")),
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
    }
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
    let started = std::time::Instant::now();
    let host = Arc::new(
        Host::start(root, name, &[], env.clone(), true)
            .await
            .map_err(SdkError::Invalid)?,
    );
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
        cases: Vec::new(),
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
    report.finish_case("check".into(), 0, started);
    if report.errors.is_empty() {
        examples(&host, &ext, &mut report).await;
        for spec in &ext.providers {
            let (before, started) = (report.errors.len(), std::time::Instant::now());
            report.ran.push(format!("provider {}", spec.id));
            test_provider(&host, root, &ext, spec, bless, &env, &mut report).await;
            report.finish_case(format!("provider {}", spec.id), before, started);
        }
        for decl in ext
            .implementations
            .iter()
            .filter(|d| d.capability == EFFORT_POLICY && d.script.is_some())
        {
            let (before, started) = (report.errors.len(), std::time::Instant::now());
            policy_suite(&host, &ext, decl, &mut report).await;
            report.finish_case(format!("policy {}", decl.id), before, started);
        }
        page_answers(&host, &ext, &mut report).await;
        let (before, started) = (report.errors.len(), std::time::Instant::now());
        questions(&host, &ext, &mut report).await;
        incremental_datasets(&host, &ext, &mut report).await;
        if report.errors.len() > before {
            report.finish_case("questions and datasets".into(), before, started);
        }
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

/// The intent examples a throwaway oxplow runs: a page's, a derived
/// collector's. A provider's (`when: { command, input }`) run in its
/// session.
async fn examples(shared: &Arc<Host>, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    // Read as the extension's files are everywhere: a bundled extension's
    // fixtures are embedded with it, not on disk.
    let read = |file: &str| {
        oxplow_app::extension::extensions::read_extension_file(&shared.root, &ext.path, file)
    };
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    // Examples that make data, each on an oxplow of its own, run side by
    // side, a few at a time; their findings join the report in the
    // examples' order.
    let slots = Arc::new(tokio::sync::Semaphore::new(side_by_side()));
    let mut own = Vec::new();
    // The oxplows fixtures that turn on more extensions start from, by
    // what they turn on.
    let mut required: std::collections::BTreeMap<Vec<String>, Arc<Host>> = Default::default();
    for example in examples {
        let Some((shown, doc)) = example_fixture(&read, &rel, &example.name) else {
            report.warnings.push(format!(
                "{rel}/extension.yaml:1: intent example `{}` has no fixture, so it isn't run — fix: \
                 add fixtures/{}.yaml: `given?`, `expect` and a `when` that is one of {}",
                example.name,
                example.name,
                kinds_hint()
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
        if let Some(key) = doc
            .as_object()
            .and_then(|o| o.keys().find(|k| !FIXTURE_KEYS.contains(&k.as_str())))
        {
            report.errors.push(format!(
                "{shown}:1: `{key}` isn't a fixture key — fix: a fixture is `given` (the steps \
                 first), `when` (what it does), `expect`, `steps` (a scenario), `view`, `model` and `requires` (more extensions to turn on)"
            ));
            continue;
        }
        let fixture = Fixture::of(example.name.clone(), shown, &doc);
        let ex = fixture.example();
        if let Some(why) = withheld_target(ext, &ex.when) {
            report.warnings.push(format!(
                "{}:1: example `{}` isn't run: {why}",
                fixture.shown, fixture.name
            ));
            continue;
        }
        // A fixture that turns on more extensions runs on an oxplow with
        // them, of its own.
        if !fixture.requires.is_empty() {
            if let Some(unknown) = fixture
                .requires
                .iter()
                .find(|n| !Host::knows(&shared.checkout, n))
            {
                report.errors.push(format!(
                    "{}:1: `requires` names `{unknown}`, which isn't an extension here — fix: an \
                     extension oxplow ships (`oxplow-bundled`) or one in `.oxplow/extensions/`",
                    fixture.shown
                ));
                continue;
            }
            let base = match required.get(&fixture.requires) {
                Some(base) => base.clone(),
                None => {
                    let built = match Host::start(
                        &shared.checkout,
                        &ext.name,
                        &fixture.requires,
                        shared.env.clone(),
                        shared.builds,
                    )
                    .await
                    {
                        Ok(host) => host.start_runtime().map(|()| Arc::new(host)),
                        Err(e) => Err(e),
                    };
                    match built {
                        Ok(base) => {
                            required.insert(fixture.requires.clone(), base.clone());
                            base
                        }
                        Err(e) => {
                            report.errors.push(format!(
                                "{}:1: an oxplow with {}: {e}",
                                fixture.shown,
                                fixture.requires.join(", ")
                            ));
                            continue;
                        }
                    }
                }
            };
            let (ext, slots) = (ext.clone(), slots.clone());
            own.push(tokio::spawn(async move {
                let _slot = slots.acquire_owned().await;
                let mut part = TestReport::part(&ext.name);
                match base.fork().await {
                    Ok(host) => run_example(&host, &ext, &fixture.example(), &mut part).await,
                    Err(e) => part
                        .errors
                        .push(format!("{}:1: an oxplow of its own: {e}", fixture.shown)),
                }
                part
            }));
            continue;
        }
        // An example that makes data — steps, rows, a job, a logged event,
        // a component's filed items — runs on an oxplow of its own, so no
        // other example sees what it made or what reacted to it.
        if makes_data(&ex) {
            let (shared, ext, slots) = (shared.clone(), ext.clone(), slots.clone());
            own.push(tokio::spawn(async move {
                let _slot = slots.acquire_owned().await;
                let mut part = TestReport::part(&ext.name);
                match shared.fork().await {
                    Ok(host) => run_example(&host, &ext, &fixture.example(), &mut part).await,
                    Err(e) => part
                        .errors
                        .push(format!("{}:1: an oxplow of its own: {e}", fixture.shown)),
                }
                part
            }));
        } else {
            run_example(shared, ext, &ex, report).await;
        }
    }
    for task in own {
        match task.await {
            Ok(part) => report.absorb(part),
            Err(e) => report
                .errors
                .push(format!("{}: an example's run stopped: {e}", ext.name)),
        }
    }
}

/// How many examples run side by side: half the machine's cores, at most
/// four — each boots an oxplow of its own.
fn side_by_side() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() / 2)
        .clamp(1, 4)
}

/// The keys of a fixture file.
const FIXTURE_KEYS: [&str; 7] = [
    "requires", "given", "when", "expect", "steps", "view", "model",
];

/// An example's fixture, owned, so it can run on a task of its own.
struct Fixture {
    name: String,
    /// The fixture file, as findings name it.
    shown: String,
    when: Value,
    expect: Value,
    given: Value,
    steps: Option<Value>,
    view: Value,
    model: Value,
    /// The extensions it turns on beyond the one tested.
    requires: Vec<String>,
}

impl Fixture {
    fn of(name: String, shown: String, doc: &Value) -> Fixture {
        Fixture {
            name,
            shown,
            when: doc.get("when").cloned().unwrap_or(Value::Null),
            expect: doc.get("expect").cloned().unwrap_or(Value::Null),
            given: doc.get("given").cloned().unwrap_or(Value::Null),
            steps: doc.get("steps").cloned(),
            view: doc.get("view").cloned().unwrap_or(Value::Null),
            model: doc.get("model").cloned().unwrap_or(Value::Null),
            requires: doc
                .get("requires")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect(),
        }
    }

    fn example(&self) -> Example<'_> {
        Example {
            name: &self.name,
            shown: &self.shown,
            when: self.when.clone(),
            expect: self.expect.clone(),
            given: self.given.clone(),
            steps: self.steps.clone(),
            view: self.view.clone(),
            model: self.model.clone(),
        }
    }
}

/// Run example `ex` on `host` as one test case: its `given` steps, what
/// its `when` names checked against `expect`, then its `steps`, each
/// `{ given }` or a `{ when?, expect }` run the same way (`when` over the
/// fixture's). A page runs its own scenario (`page_example`).
async fn run_example(host: &Host, ext: &Extension, ex: &Example<'_>, report: &mut TestReport) {
    let shown = ex.shown;
    // A provider's command runs in its session (`test_provider`).
    let command = ex.when.get("command").and_then(Value::as_str);
    if command.is_some_and(|name| !ext.commands.iter().any(|c| c.name == name))
        && !ext.providers.is_empty()
    {
        return;
    }
    let (before, started) = (report.errors.len(), std::time::Instant::now());
    report.ran.push(format!("example {}", ex.name));
    match model_replies(ex, report).map(|replies| host.answer_with(replies)) {
        Some(Ok(())) => {
            if ex.when.get("page").is_some() {
                dispatch(host, ext, ex, report).await;
            } else {
                scenario(host, ext, ex, report).await;
            }
        }
        Some(Err(e)) => report
            .errors
            .push(format!("{shown}:1: the fixture model: {e}")),
        None => {}
    }
    report.finish_case(format!("example {}", ex.name), before, started);
}

/// A fixture that isn't a page's, run as [`run_example`] says.
async fn scenario(host: &Host, ext: &Extension, ex: &Example<'_>, report: &mut TestReport) {
    let shown = ex.shown;
    let mut binds = Binds::default();
    if !ex.given.is_null()
        && !run_steps(host, ext, shown, "given", &ex.given, &mut binds, report).await
    {
        return;
    }
    let steps = match &ex.steps {
        None => Vec::new(),
        Some(Value::Array(steps)) => steps.clone(),
        Some(_) => {
            report.errors.push(format!(
                "{shown}:1: `steps` isn't a list — fix: `steps: [{{ given: [..] }} | {{ when?, \
                 expect }}]`"
            ));
            return;
        }
    };
    if (ex.steps.is_none() || !ex.expect.is_null())
        && !step_run(host, ext, ex, (&ex.when, &ex.expect), &binds, "", report).await
    {
        return;
    }
    for (index, step) in steps.iter().enumerate() {
        let label = format!("steps[{index}]");
        if let Some(given) = step.get("given") {
            if !run_steps(
                host,
                ext,
                shown,
                &format!("{label}.given"),
                given,
                &mut binds,
                report,
            )
            .await
            {
                return;
            }
            continue;
        }
        let Some(expect) = step.get("expect") else {
            return report.errors.push(format!(
                "{shown}:1: `{label}` has no `given` or `expect` — fix: `{{ given: [..] }}` or \
                 `{{ when?, expect }}`"
            ));
        };
        // Its `when` keys over the fixture's — or, naming another kind,
        // its own.
        let when = match step.get("when") {
            Some(own) if kind_of(own) != kind_of(&ex.when) && kind_of(own).is_some() => own.clone(),
            over => {
                let mut when = ex.when.as_object().cloned().unwrap_or_default();
                when.extend(over.and_then(Value::as_object).cloned().unwrap_or_default());
                Value::Object(when)
            }
        };
        if !step_run(host, ext, ex, (&when, expect), &binds, &label, report).await {
            return;
        }
    }
}

/// The kind a `when` names: its one kind key (`event` also rides with an
/// effect's, a policy's or a collector's).
fn kind_of(when: &Value) -> Option<&'static str> {
    let named: Vec<&'static str> = KINDS
        .iter()
        .map(|k| k.key)
        .filter(|key| when.get(key).is_some())
        .collect();
    named
        .iter()
        .copied()
        .find(|key| *key != "event" || named.len() == 1)
}

/// Run `when` (as the fixture `ex`'s kind would) and check it against
/// `expect`, both read through `binds`; its findings name `label` (a step),
/// when it has one. False when it found anything.
async fn step_run(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    (when, expect): (&Value, &Value),
    binds: &Binds,
    label: &str,
    report: &mut TestReport,
) -> bool {
    let shown = ex.shown;
    let at = report.errors.len();
    match (binds.substitute(when), binds.substitute(expect)) {
        (Ok(when), Ok(expect)) => {
            let run = Example {
                when,
                expect,
                given: Value::Null,
                steps: None,
                ..ex.clone()
            };
            // A page's scenario runs steps, so this nests.
            Box::pin(dispatch(host, ext, &run, report)).await;
        }
        (Err(e), _) | (_, Err(e)) => report.errors.push(format!("{shown}:1: {e}")),
    }
    if !label.is_empty() {
        let prefix = format!("{shown}:1: ");
        for error in &mut report.errors[at..] {
            if let Some(rest) = error.strip_prefix(&prefix) {
                *error = format!("{prefix}{label}: {rest}");
            }
        }
    }
    report.errors.len() == at
}

/// Run what example `ex`'s `when` names and check it against its `expect`.
async fn dispatch(host: &Host, ext: &Extension, ex: &Example<'_>, report: &mut TestReport) {
    let shown = ex.shown;
    if let Some(page) = ex.when.get("page").and_then(Value::as_str) {
        page_example(host, ext, ex, page, report).await;
    } else if let Some(slug) = ex.when.get("form").and_then(Value::as_str) {
        form_example(host, ext, ex, slug, report).await;
    } else if let Some(id) = ex.when.get("job").and_then(Value::as_str) {
        job_example(host, ext, ex, id, report).await;
    } else if let Some(id) = ex.when.get("component").and_then(Value::as_str) {
        component_example(host, ext, ex, id, report).await;
    } else if ex.when.get("event").is_some()
        && ex.when.get("effect").is_none()
        && ex.when.get("implementation").is_none()
        && ex.when.get("collector").is_none()
    {
        event_example(host, ext, ex, report).await;
    } else if let Some(id) = ex.when.get("effect").and_then(Value::as_str) {
        effect_example(host, ext, ex, id, report).await;
    } else if let Some(id) = ex.when.get("implementation").and_then(Value::as_str) {
        policy_example(host, ext, ex, id, report).await;
    } else if let Some(t) = ex.when.get("event_type").and_then(Value::as_str) {
        event_type_example(ext, ex, t, report);
    } else if let Some(id) = ex.when.get("advisory").and_then(Value::as_str) {
        advisory_example(host, ext, ex, id, report).await;
    } else if let Some(sql) = ex.when.get("sql").and_then(Value::as_str) {
        sql_example(host, ex, sql, report).await;
    } else if let Some(link) = ex.when.get("wikilink").and_then(Value::as_str) {
        wikilink_example(ext, ex, link, report);
    } else if let Some(id) = ex.when.get("collector").and_then(Value::as_str) {
        collector_example(host, ext, ex, id, report).await;
    } else if let Some(cmd) = ex
        .when
        .get("command")
        .and_then(Value::as_str)
        .and_then(|name| ext.commands.iter().find(|c| c.name == name))
        .filter(|_| ex.when.get("answers").is_some())
    {
        command_example(host, ex, cmd, report).await;
    } else if let Some(command) = ex.when.get("command").and_then(Value::as_str) {
        command_run(host, ex, command, report).await;
    } else if ex.when.get("command").is_none() || ext.providers.is_empty() {
        report.errors.push(format!(
            "{shown}:1: example `{}`'s `when` names no kind of fixture — fix: one of {}",
            ex.name,
            kinds_hint()
        ));
    }
}

/// A kind of fixture: the key its `when` names it by, the `when` it takes
/// and what its `expect` checks.
struct Kind {
    key: &'static str,
    when: &'static str,
    expect: &'static str,
}

/// Every kind of fixture the runner runs, in the order `dispatch` tries
/// them. The one statement of the grammar: the runner's hints and the
/// skill's table (`testing.md`, held to it by a test) are made from it.
const KINDS: &[Kind] = &[
    Kind {
        key: "page",
        when: "{ page: <slug | ext/slug>, params? }",
        expect: "{ loaders?, nodes?, text?, alert?, badge?, inactive?, actions? }",
    },
    Kind {
        key: "form",
        when: "{ form: <page>, node?, params? }",
        expect: "{ values?, fixed?, fields? }",
    },
    Kind {
        key: "job",
        when: "{ job: <id>, intentKey, request, cancel?, setup?, rows? }",
        expect: "{ state, result?, text?, toolCalls?, output?, effects?, commands?, pages? }",
    },
    Kind {
        key: "component",
        when: "{ component: <id>, items?, invokes?: [{ command, input? }] }",
        expect: "{ invokes: [{ ok, result? } | { ok: false, refused }] }",
    },
    Kind {
        key: "event",
        when: "{ event: { type, v?, payload, subject?, anchors? } }",
        expect: "{ effects?: [{ id, state }], commands?: [names] }",
    },
    Kind {
        key: "effect",
        when: "{ effect: <id>, event, answers? } (a dry run)",
        expect: "{ commands } | { skip } | { reacts: false }",
    },
    Kind {
        key: "implementation",
        when: "{ implementation: <id>, event, answers? } (a dry run)",
        expect: "{ commands } | { skip }",
    },
    Kind {
        key: "event_type",
        when: "{ event_type, v?, payload }",
        expect: "{ valid, upcast? }",
    },
    Kind {
        key: "advisory",
        when: "{ advisory: <id>, thread? }",
        expect: "{ lines: [text] }",
    },
    Kind {
        key: "sql",
        when: "{ sql, params? }",
        expect: "{ rows, columns? }",
    },
    Kind {
        key: "wikilink",
        when: "{ wikilink }",
        expect: "{ uri }",
    },
    Kind {
        key: "collector",
        when: "{ collector: <id>, rows?, event? } | { collector: <id>, files }",
        expect: "{ entities?, events? } | { facts }",
    },
    Kind {
        key: "command",
        when: "{ command, input?, as?, confirmed? } (with answers: the extension's own, a dry run)",
        expect: "{ ok, result?, commands? } | { ok: false, refused }",
    },
];

/// The kinds, for a finding's fix: each `when` and its `expect`.
fn kinds_hint() -> String {
    KINDS
        .iter()
        .map(|k| format!("{}: `when: {}`, `expect: {}`", k.key, k.when, k.expect))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The skill's table of fixture kinds (`testing.md`).
#[cfg(test)]
fn kinds_table() -> String {
    let mut out = String::from("| kind | `when` | `expect` |\n|---|---|---|\n");
    for k in KINDS {
        // A `|` inside a cell would split it.
        let cell = |text: &str| text.replace('|', "\\|");
        out.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            k.key,
            cell(k.when),
            cell(k.expect)
        ));
    }
    out
}

/// Whether example `ex` changes its oxplow's data (and what reacts to
/// that): it has steps or rows, or runs a job, logs an event or files a
/// component's items. A dry run (a command's, an effect's, a policy's) or a
/// collector run that stores nothing doesn't.
fn makes_data(ex: &Example<'_>) -> bool {
    let input = &ex.when;
    let has = |key: &str| input.get(key).is_some();
    !ex.given.is_null()
        || ex.steps.is_some()
        || has("rows")
        || has("setup")
        || has("job")
        || has("component")
        || has("advisory")
        || (has("event") && !has("effect") && !has("implementation") && !has("collector"))
}

/// Why an example's target isn't here: the page, form, job, component,
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
        (Kind::Page, named("page")),
        (Kind::Page, named("form")),
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
    if ex.model.is_null() {
        return Some(Vec::new());
    }
    let model = &ex.model;
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

/// Where `got` first differs from what `want` (an example's `expect`, or
/// a part of it) says, as a JSON pointer with the two values. The one rule
/// for every expectation: an object compares only the keys it names (one
/// `got` lacks matches only `null`, not even `$any`); a list compares every item, in order;
/// `{ contains: [..] }` holds against a list having each item (compared
/// the same way) in any order, among others; `$any` matches anything;
/// `{ $exact: v }` compares the whole value, every key. `refused: <text>`
/// holds when the reason it said no contains that text.
fn compare(want: &Value, got: &Value) -> Option<(String, Value, Value)> {
    fn walk(path: &str, want: &Value, got: &Value) -> Option<(String, Value, Value)> {
        let differs = || {
            Some((
                if path.is_empty() { "/" } else { path }.to_string(),
                want.clone(),
                got.clone(),
            ))
        };
        match want {
            Value::String(any) if any == ANY => None,
            Value::Object(o) if o.len() == 1 && o.contains_key("$exact") => {
                first_mismatch(&o["$exact"], got).map(|(at, w, g)| {
                    let at = if at == "/" { String::new() } else { at };
                    (format!("{path}{at}"), w, g)
                })
            }
            Value::Object(o) if o.len() == 1 && o.contains_key("contains") && got.is_array() => {
                let items = got.as_array().expect("a list");
                o["contains"].as_array().map_or_else(differs, |wanted| {
                    wanted.iter().enumerate().find_map(|(i, item)| {
                        (!items.iter().any(|g| walk("", item, g).is_none()))
                            .then(|| (format!("{path}/contains/{i}"), item.clone(), got.clone()))
                    })
                })
            }
            Value::Object(o) => match got {
                Value::Object(g) => o.iter().find_map(|(k, w)| {
                    let at = format!("{path}/{k}");
                    match g.get(k) {
                        // Why it said no: any reason holding that text.
                        Some(Value::String(why))
                            if k == "refused" && w.as_str().is_some_and(|w| why.contains(w)) =>
                        {
                            None
                        }
                        Some(g) => walk(&at, w, g),
                        None if w.is_null() => None,
                        None => Some((at, w.clone(), Value::Null)),
                    }
                }),
                _ => differs(),
            },
            Value::Array(w) => match got {
                Value::Array(g) if g.len() == w.len() => w
                    .iter()
                    .zip(g)
                    .enumerate()
                    .find_map(|(i, (w, g))| walk(&format!("{path}/{i}"), w, g)),
                _ => differs(),
            },
            _ if want == got => None,
            _ => differs(),
        }
    }
    walk("", want, got)
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
    note(ext.panels.len(), "panel(s)");
    note(ext.ui.decorators.len(), "decorator(s)");
    note(ext.ui.replacements.len(), "replacement(s)");
    note(ext.skills.len(), "skill(s)");
    out
}

/// Log the fixture's event (`when: { event: { type, payload, subject?,
/// anchors? }, model? }`) in the throwaway and deliver it: the extension's
/// effects — running in the throwaway host — react for real, their
/// commands run through the bus. What happened against `expect`:
/// `{ effects: [{ id, state? }], commands: [names] }` (`$any` matches).
async fn event_example(host: &Host, ext: &Extension, ex: &Example<'_>, report: &mut TestReport) {
    let shown = ex.shown;
    if let Err(e) = start_effects(host, ext).await {
        report
            .errors
            .push(format!("{shown}:1: starting its effects: {e}"));
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
    let got = json!({ "effects": effects, "commands": commands });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: the event led to {got} at `{path}`, the example expects {want} — fix: \
             the effects, or the example's `expect` (`{{ effects: [{{ id, state }}], commands: \
             [names] }}`)"
        ));
    }
}

/// `commands` as audited (name, audit id): a sequence — how an effect's
/// commands run — is replaced by its steps, in order.
async fn without_sequences(host: &Host, audited: Vec<(String, Option<i64>)>) -> Vec<String> {
    let audits = oxplow_db::SqliteCommandAuditStore::new(host.svc.db.clone());
    let mut commands = Vec::new();
    for (name, id) in audited {
        let steps = match (id, name == "oxplow.command.sequence") {
            (Some(id), true) => audits
                .get(id)
                .await
                .ok()
                .flatten()
                .and_then(|a| a.result.and_then(|r| r.get("children").cloned())),
            _ => None,
        };
        match steps {
            Some(Value::Array(children)) => commands.extend(
                children
                    .iter()
                    .filter_map(|c| c.get("name").and_then(Value::as_str))
                    .map(str::to_string),
            ),
            _ => commands.push(name),
        }
    }
    commands
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
    let ran = rows(
        "SELECT json_extract(payload, '$.command'), json_extract(payload, '$.audit_id') FROM v_event \
         WHERE type = 'command.executed' AND seq > ?1 \
           AND json_extract(payload, '$.actor_kind') = 'effect' ORDER BY seq",
    )
    .await;
    let audited = ran
        .iter()
        .map(|r| match &r[1] {
            oxplow_db::SqlCell::Int(id) => (text(&r[0]), Some(*id)),
            _ => (text(&r[0]), None),
        })
        .collect();
    let commands = without_sequences(host, audited).await;
    (effects, commands)
}

/// Start `ext`'s effects at the log's head, so they react to what the
/// example logs next.
async fn start_effects(host: &Host, ext: &Extension) -> Result<(), String> {
    for decl in &ext.effects {
        oxplow_app::effects::start_at_head(&host.svc.db, &decl.name())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Record intent for job `id` through the extension's own command over
/// `jobs.write/enqueue` (`when: { job, intentKey, request, model?,
/// setup?, rows? }`: `setup` and `rows` first make what its tools read), the
/// job running in the throwaway host, and read its outcome through its
/// `jobs.read/snapshot` command once it ends: against `expect` — `{ state,
/// result?, text?, toolCalls?, output?, effects?, commands?, pages? }`
/// (`$any` matches): its streamed `text`, how many tools its model called
/// and its output rows (`[{ kind, … }]`, `v_job_run_output`). Its model calls
/// answer with the fixture model's replies; with `cancel: true` in its
/// input, the job is cancelled through the extension's `jobs.write/cancel`
/// command once its first output is written. An expectation of `effects`,
/// `commands` or `pages` delivers the `contribution.job.completed` the job
/// logs to the extension's effects, running there too: the
/// effects that reacted (`[{ id, state? }]`), the commands they ran
/// (`[names]`) and which of the named wiki pages (`[uri]`) now exist.
async fn job_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::extension::extension_commands::CommandHandler;
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
    let delivers = ["effects", "commands", "pages"]
        .iter()
        .any(|k| ex.expect.get(k).is_some());
    if delivers {
        if let Err(e) = start_effects(host, ext).await {
            report
                .errors
                .push(format!("{shown}:1: starting its effects: {e}"));
            return;
        }
    }
    if !seed_rows(host, ext, ex, report).await || !run_setup(host, ext, ex, report).await {
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
        "intentKey": ex.when.get("intentKey").cloned().unwrap_or_else(|| json!(ex.name)),
        "request": ex.when.get("request").cloned().unwrap_or(Value::Null),
    });
    // A job asked for in a thread: the effects that react to its end run
    // for that thread, as a command that changes the workspace needs.
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
        oxplow_domain::uri::build::id_of(r, oxplow_domain::uri::resources::JOB_RUN).ok()
    }) else {
        report
            .errors
            .push(format!("{shown}:1: job `{id}`: {queued} names no job"));
        return;
    };
    if ex.when.get("cancel") == Some(&json!(true)) {
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
                    json!({ "job_run": &job.to_string() }),
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
                json!({ "job_run": &job.to_string() }),
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
                json!({ "job_run": &job.to_string() }),
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
                    "SELECT 1 FROM v_knowledge_page WHERE uri = ?1",
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
    let got = ended;
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
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
    match ex.when.get("rows") {
        Some(rows) => publish_rows(host, ext, ex.shown, rows, report).await,
        None => true,
    }
}

/// `rows: { <entity>: [ … ] }` published for the extension's own entities.
async fn publish_rows(
    host: &Host,
    ext: &Extension,
    shown: &str,
    rows: &Value,
    report: &mut TestReport,
) -> bool {
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
/// person, or with `as: agent` as the agent of `oxplow:oxplow/thread?id=thr1`. `false`,
/// with the error reported, when one fails.
async fn run_setup(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> bool {
    match ex.when.get("setup") {
        Some(steps) => {
            let mut binds = Binds::default();
            run_steps(host, ext, ex.shown, "setup", steps, &mut binds, report).await
        }
        None => true,
    }
}

/// `steps` (the example's `key`, `setup` or `given`), run in order on the
/// throwaway's own database: each `{ command, input?, as?, confirmed?,
/// bind? }` or, for a `given`, `{ rows: { <entity>: [ … ] } }`. A step's
/// input may use what an earlier one bound (`$name.path`); `bind: <name>`
/// keeps a command's result for later. `false`, with the error reported,
/// when one fails.
async fn run_steps(
    host: &Host,
    ext: &Extension,
    shown: &str,
    key: &str,
    steps: &Value,
    binds: &mut Binds,
    report: &mut TestReport,
) -> bool {
    let Some(steps) = steps.as_array() else {
        report.errors.push(format!(
            "{shown}:1: `{key}` isn't a list — fix: `{key}: [{{ command, input?, as? }}]`"
        ));
        return false;
    };
    if let Err(e) = host.svc.streams.ensure_primary().await {
        report
            .errors
            .push(format!("{shown}:1: `{key}`: its thread: {e}"));
        return false;
    }
    for (index, step) in steps.iter().enumerate() {
        if let Some(rows) = step.get("rows") {
            if !publish_rows(host, ext, shown, rows, report).await {
                return false;
            }
            continue;
        }
        if let Some(project) = step.get("project") {
            if let Err(e) = given_project(host, project) {
                report
                    .errors
                    .push(format!("{shown}:1: `{key}[{index}]` `project`: {e}"));
                return false;
            }
            host.settle().await;
            continue;
        }
        if let Some(observed) = step.get("observe") {
            let observed = match binds.substitute(observed) {
                Ok(observed) => observed,
                Err(e) => {
                    report
                        .errors
                        .push(format!("{shown}:1: `{key}[{index}]`: {e}"));
                    return false;
                }
            };
            if let Err(e) = observe(host, ext, &observed).await {
                report
                    .errors
                    .push(format!("{shown}:1: `{key}[{index}]` `observe`: {e}"));
                return false;
            }
            host.settle().await;
            continue;
        }
        let Some(command) = step.get("command").and_then(Value::as_str) else {
            report.errors.push(format!(
                "{shown}:1: `{key}[{index}]` names no command — fix: `{{ command, input?, as? }}`, \
                 `{{ observe: {{ <kind>: {{ .. }} }} }}` or `{{ rows: {{ <entity>: [..] }} }}`"
            ));
            return false;
        };
        let actor = match step_actor(step) {
            Ok(actor) => actor,
            Err(e) => {
                report
                    .errors
                    .push(format!("{shown}:1: `{key}[{index}]`: {e}"));
                return false;
            }
        };
        let input = match binds.substitute(&step.get("input").cloned().unwrap_or_else(|| json!({})))
        {
            Ok(input) => input,
            Err(e) => {
                report
                    .errors
                    .push(format!("{shown}:1: `{key}[{index}]`: {e}"));
                return false;
            }
        };
        let confirmed = step
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match host
            .svc
            .commands
            .run(&actor, command, input, confirmed)
            .await
        {
            Ok(outcome) => {
                if let Some(name) = step.get("bind").and_then(Value::as_str) {
                    binds.0.insert(name.to_string(), outcome.result);
                }
                // What it set off — a reactor hearing its event, the
                // generation it asked for — happens before the next step.
                host.settle().await;
            }
            Err(e) => {
                report.errors.push(format!(
                    "{shown}:1: `{key}[{index}]` `{command}` failed: {e} — fix: its input, or the \
                     steps before it{}",
                    if confirmed {
                        ""
                    } else {
                        " (a command that asks first runs with `confirmed: true`)"
                    }
                ));
                return false;
            }
        }
    }
    true
}

/// `project: { <key>: <value>, .. }`: keys of `.oxplow/project.yaml` set on
/// the throwaway's root, as a person's edit of the file, and the config read
/// again from disk, as core does when the file changes.
fn given_project(host: &Host, project: &Value) -> Result<(), String> {
    let Some(keys) = project.as_object() else {
        return Err(
            "`project` is a map of project.yaml keys — fix: `project: { lspServers: [..] }`".into(),
        );
    };
    let file = host.root.join(".oxplow/project.yaml");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut doc: serde_yaml::Mapping = serde_yaml::from_str(&text).unwrap_or_default();
    for (key, value) in keys {
        doc.insert(
            serde_yaml::Value::String(key.clone()),
            serde_yaml::to_value(value).map_err(|e| e.to_string())?,
        );
    }
    std::fs::write(
        &file,
        serde_yaml::to_string(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    host.svc
        .reload_config_from_disk()
        .map_err(|e| e.to_string())
}

/// `observe: { <kind>: { .. } }`: a fact oxplow only learns from what
/// happens around it, made through the API core makes it with.
///
/// - `event: { type, v?, payload, subject?, anchors? }`: an event logged
///   and delivered, the extension's effects reacting.
/// - `evidence: { effort, observations?: [..], metric_deltas?: [..] }`: an
///   effort's evidence (a metric delta is `{ key, current, baseline?, warn_at?,
///   fail_at?, crossing?, kind?, direction?, .. }`).
/// - `commit: { sha, at, subject, files?, .. }`: a commit the indexer read.
/// - `change: { kind?, target, files?, functions? }`: a change's analysis,
///   as the change analysis stores it (`kind` is `commit` by default; an
///   effort's `target` is its id, like `eff1`).
/// - `test_run: { effort, at?, cases?: [{ name, status }], asserted_failed? }`:
///   a test run an effort recorded, as a run recorder writes it.
/// - `snapshot: { trigger?, elapsed_ms?, budget_ms?, revision?, branch?, files?: [{ path, hash }] }`:
///   one take in the stream's snapshot log.
/// - `diagnostics: { path, language?, items?: [{ severity, message, line?, col?, source?, code? }] }`:
///   what a language server reported for a file.
/// - `symbols: { path, language?, items?: [{ name, kind, id?, container?, signature?, line?, col? }] }`:
///   the definitions the index read from a file at a snapshot of it.
/// - `run: { extension, contribution, kind?, outcome?, days_ago?, cpu_ms?, ai_calls?, error? }`:
///   a finished run of an extension's code, as the host records it.
/// - `health: { extension, contribution, kind?, disable }`: a contribution
///   disabled after failures, with the reason.
/// - `token_usage: { thread?, effort?, session?, model?, prompt?, input_tokens?,
///   output_tokens?, cache_creation_input_tokens?, cache_read_input_tokens?, turn? }`:
///   the tokens a turn spent.
/// - `hint: { hint, evaluated?, thread? }`: a hint's query ran that many times.
/// - `nudge: { kind, message, thread?, effort?, audience?, delivered? }`: a
///   nudge a hint raised.
/// - `analyze: { kind: working | commit | …, sha?, streamId? }`: a change
///   analyzed now (`change_analysis::ensure_change`), so its files and
///   functions are there to read.
/// - `refs: { branches?, tags?, heads? }`: the branches, tags and stream
///   heads the git indexer read (each replaces what was stored).
/// - `visit: { page, kind?, label?, thread?, duration_ms? }`: a page the person
///   opened, as the shell records it.
/// - `facts: { measure, producer?, items?: [{ value?, path?, line?, rule?, severity?, detail?, subject?, dims? }] }`:
///   what a collector found, recorded as one capture of the measure.
/// - `retention: { days }`: the log's retention sweep, run as it is `days`
///   from now: payloads past their keep are expired, envelopes stay.
/// - `collect: { effort, collectors?: [key] }`: the fact collectors an
///   effort's end triggers, run over what the effort recorded.
/// - `turn: { prompt?, during?: [{ command, input?, as? }], tools?: [{ name, kind, paths?, command?, detail?,
///   ok?, exit_code? }], write?: { path, body }, answer?, compactions?: n,
///   context?: { tokens, window? }, thread?, session?, subagent?, peer? }`:
///   one turn on `thread` (thr1), in `session`, as
///   its harness's hooks report it — the prompt, each tool call, the file
///   the turn wrote (a snapshot of the worktree is taken after it), the
///   context compactions, and the turn's end with the agent's last message
///   and how full its context was.
async fn observe(host: &Host, ext: &Extension, observed: &Value) -> Result<(), String> {
    let Some(map) = observed.as_object().filter(|m| m.len() == 1) else {
        return Err("name one kind — fix: `observe: { turn: { .. } }`".into());
    };
    let (kind, body) = map.iter().next().expect("one key");
    match kind.as_str() {
        "turn" => observe_turn(host, body).await,
        "evidence" => observe_evidence(host, body).await,
        "event" => observe_event(host, ext, body).await,
        "commit" => observe_commit(host, body).await,
        "change" => observe_change(host, body).await,
        "test_run" => observe_test_run(host, body).await,
        "snapshot" => observe_snapshot(host, body).await,
        "collect" => observe_collect(host, body).await,
        "diagnostics" => observe_diagnostics(host, body).await,
        "symbols" => observe_symbols(host, body).await,
        "retention" => observe_retention(host, body).await,
        "facts" => observe_facts(host, body).await,
        "visit" => observe_visit(host, body).await,
        "refs" => observe_refs(host, body).await,
        "analyze" => observe_analyze(host, body).await,
        "run" => observe_run(host, body).await,
        "health" => observe_health(host, body).await,
        "token_usage" => observe_token_usage(host, body).await,
        "hint" => observe_hint(host, body).await,
        "nudge" => observe_nudge(host, body).await,
        other => {
            return Err(format!(
                "`{other}` isn't a kind — fix: `turn`, `evidence`, `event`, `commit`, `change`, `test_run`, `snapshot`, `collect`, `diagnostics`, `symbols`, `retention`, `facts`, `visit`, `refs`, `analyze`, `run`, `health`, `token_usage`, `hint` or `nudge`"
            ))
        }
    }?;
    // What the log now holds reaches the datasets, as a command's events do;
    // a reaction's own events (a checkpoint, a policy's answer to it) are
    // heard in turn.
    host.settle().await;
    host.svc
        .event_pump
        .deliver_projections()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `event: { type, v?, payload, subject?, anchors? }`: an event oxplow
/// logs when something outside it happens (a contribution disabled after
/// failing, say), logged and delivered as a fixture's `when: { event }` is,
/// the extension's effects reacting to it.
async fn observe_event(host: &Host, ext: &Extension, body: &Value) -> Result<(), String> {
    start_effects(host, ext).await?;
    let view = EventView::fixture(body, &host.svc.vocabulary.current()).map_err(|e| e.message())?;
    let mut env = oxplow_domain::events::Envelope::new(
        view.event_type.clone(),
        view.v,
        "fixture",
        view.payload.clone(),
    )
    .map_err(|e| e.to_string())?;
    env.subject = view.subject.clone();
    env.anchors = view.anchors.clone();
    host.svc
        .event_log_store
        .append(env)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The stream the throwaway's thread lives in.
async fn first_stream(host: &Host) -> Result<oxplow_domain::StreamId, String> {
    host.svc
        .streams
        .list_streams()
        .await
        .map_err(|e| e.to_string())?
        .first()
        .map(|s| s.id)
        .ok_or_else(|| "no stream".to_string())
}

/// `change: { kind?, target, files?: [{ path, status?, additions?,
/// deletions?, zone?, is_test? }], functions?: [{ path, symbol, name?, status?,
/// container?, signature_changed?, body_changed?, start_line?, visibility?,
/// is_test?, length?, params_before?, params_after?, complexity?, complexity_delta? }] }`:
/// what the change analysis stored for a change (`SqliteChangeStore::
/// store_results`). A `working` or `effort` change's head is the working
/// tree; a `commit`'s is the commit `target`.
async fn observe_change(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::{ChangeFileRow, ChangeFunctionRow, ChangeResults};
    use oxplow_domain::vcs::Revision;
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let kind = text(body, "kind").unwrap_or_else(|| "commit".into());
    let target = text(body, "target").ok_or("`change.target` names what the change is of")?;
    let target = match (kind.as_str(), target.strip_prefix("eff")) {
        ("effort", Some(n)) if n.parse::<i64>().is_ok() => n.to_string(),
        _ => target,
    };
    let head = match kind.as_str() {
        "working" | "effort" => Revision::Working,
        _ => Revision::git(target.clone()),
    };
    let int = |v: &Value, key: &str| v.get(key).and_then(Value::as_i64);
    let flag = |v: &Value, key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
    let list = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut results = ChangeResults::default();
    for f in list("files") {
        results.files.push(ChangeFileRow {
            path: text(&f, "path").ok_or("a change file names its `path`")?,
            status: text(&f, "status").unwrap_or_else(|| "modified".into()),
            additions: int(&f, "additions").unwrap_or(0),
            deletions: int(&f, "deletions").unwrap_or(0),
            zone: Some(text(&f, "zone").unwrap_or_else(|| "other".into())),
            is_test: flag(&f, "is_test"),
            ..Default::default()
        });
    }
    for f in list("functions") {
        let path = text(&f, "path").ok_or("a change function names its `path`")?;
        let symbol = text(&f, "symbol").ok_or("a change function names its `symbol`")?;
        results.functions.push(ChangeFunctionRow {
            symbol_ref: oxplow_domain::uri::build::symbol_uri(&path, &symbol),
            name: text(&f, "name").unwrap_or_else(|| symbol.clone()),
            container: text(&f, "container").unwrap_or_default(),
            status: text(&f, "status").unwrap_or_else(|| "modified".into()),
            signature_changed: flag(&f, "signature_changed"),
            body_changed: flag(&f, "body_changed"),
            start_line: int(&f, "start_line").unwrap_or(1),
            visibility: text(&f, "visibility").unwrap_or_else(|| "public".into()),
            is_test: flag(&f, "is_test"),
            length: int(&f, "length"),
            params_before: int(&f, "params_before"),
            params_after: int(&f, "params_after"),
            complexity: f.get("complexity").and_then(Value::as_f64),
            complexity_delta: f.get("complexity_delta").and_then(Value::as_f64),
            path,
            ..Default::default()
        });
    }
    let stream = first_stream(host).await?;
    let (row, _) = host
        .svc
        .change_store
        .get_or_create(stream.value(), &kind, &target, None, &head)
        .await
        .map_err(|e| e.to_string())?;
    host.svc
        .change_store
        .store_results(row.id, results, None, 0, 0, None)
        .await
        .map_err(|e| e.to_string())
}

/// `test_run: { effort, at?, producer?, branch?, cases?: [{ name, status }],
/// asserted_failed? }`: a run an effort recorded, as a run recorder writes
/// it: a `tests` capture with its detail and an `oxplow.test_case` fact
/// per case (or per failure asserted as a count with no report), through
/// the fact store. A `producer` other than `tests` is a record without
/// measurements.
async fn observe_test_run(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::effort_store::EffortStore as _;
    use oxplow_domain::{EffortId, Timestamp};
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`test_run.effort` is an effort's id, like `eff1`")?;
    let found = host
        .svc
        .effort_store
        .get_effort(&EffortId::new(effort))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no effort eff{effort}"))?;
    let at = match body.get("at").and_then(Value::as_str) {
        Some(at) => Timestamp::parse(at).map_err(|e| format!("`test_run.at`: {e}"))?,
        None => Timestamp::now(),
    };
    let producer = body
        .get("producer")
        .and_then(Value::as_str)
        .unwrap_or("tests");
    let branch = body.get("branch").and_then(Value::as_str).unwrap_or("main");
    let asserted_failed = body
        .get("asserted_failed")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut cases = Vec::new();
    for c in body
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| c.get(key).and_then(Value::as_str).map(str::to_string);
        cases.push((
            text("name").ok_or("a test case names its `name`")?,
            text("status").ok_or("a test case names its `status`")?,
        ));
    }
    let failed = cases
        .iter()
        .filter(|(_, status)| status == "failed")
        .count() as i64
        + asserted_failed;
    let suites: Vec<Value> = if cases.is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "name": "suite",
            "cases": cases.iter().map(|(name, status)| json!({
                "classname": "mod", "name": name, "status": status
            })).collect::<Vec<_>>()
        })]
    };
    let facts = &host.svc.fact_store;
    let case = facts
        .get_measure("oxplow.test_case")
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no `oxplow.test_case` measure")?;
    let stream = first_stream(host).await?;
    let mut capture = oxplow_db::NewMetricCapture::done(stream.value(), producer, "hook");
    capture.thread_id = Some(found.thread_id.value());
    capture.effort_id = Some(effort);
    capture.branch = Some(branch.to_string());
    capture.captured_at = Some(at);
    capture.detail_json = Some(
        json!({
            "kind": "test-detail",
            "payload": { "command": "cargo test", "failed": failed, "suites": suites }
        })
        .to_string(),
    );
    let mut rows: Vec<oxplow_db::NewFact> = cases
        .iter()
        .map(|(name, status)| oxplow_db::NewFact {
            subject: Some(oxplow_domain::uri::build::test_uri("mod", name)),
            dims_json: Some(json!({ "oxplow.status": status }).to_string()),
            ..oxplow_db::NewFact::new(case.id, 1.0)
        })
        .collect();
    for _ in 0..asserted_failed {
        rows.push(oxplow_db::NewFact {
            dims_json: Some(r#"{"oxplow.status":"failed"}"#.to_string()),
            ..oxplow_db::NewFact::new(case.id, 1.0)
        });
    }
    if producer != "tests" {
        rows.clear();
    }
    facts
        .record_facts(capture, rows)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `diagnostics: { path, language?, items?: [{ severity, message, line?,
/// col?, source?, code? }] }`: what a language server reported for one file
/// (`SqliteDiagnosticStore::replace_file`); no items says the file is clean.
async fn observe_diagnostics(host: &Host, body: &Value) -> Result<(), String> {
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .ok_or("`diagnostics.path` names the file")?;
    let language = body
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("rust");
    let mut rows = Vec::new();
    for d in body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
        let int = |key: &str, default: i64| d.get(key).and_then(Value::as_i64).unwrap_or(default);
        rows.push(oxplow_db::DiagnosticRow {
            severity: text("severity").ok_or("a diagnostic names its `severity`")?,
            message: text("message").ok_or("a diagnostic names its `message`")?,
            source: text("source"),
            code: text("code"),
            line: int("line", 1),
            col: int("col", 1),
            end_line: int("end_line", int("line", 1)),
            end_col: int("end_col", int("col", 1) + 1),
        });
    }
    let stream = first_stream(host).await?;
    host.svc
        .diagnostic_store
        .replace_file(stream.value(), language.into(), path.into(), rows)
        .await
        .map_err(|e| e.to_string())
}

/// `symbols: { path, language?, items?: [{ name, kind, id?, container?,
/// signature?, line?, col? }] }`: the definitions the symbol index read from
/// a file (`SqliteSymbolStore::record`), at a snapshot of the tree taken
/// with the file in it.
async fn observe_symbols(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::symbol_store::{FileSymbols, SymbolCapture, SymbolRow};
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .ok_or("`symbols.path` names the file")?;
    let language = body
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("rust");
    let mut rows = Vec::new();
    for sym in body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| sym.get(key).and_then(Value::as_str).map(str::to_string);
        let int = |key: &str, default: i64| sym.get(key).and_then(Value::as_i64).unwrap_or(default);
        let name = text("name").ok_or("a symbol names its `name`")?;
        rows.push(SymbolRow {
            id: text("id").unwrap_or_else(|| name.clone()),
            kind: text("kind").unwrap_or_else(|| "fn".into()),
            container: text("container"),
            signature: text("signature"),
            line: int("line", 1),
            col: int("col", 1),
            start_line: int("line", 1),
            start_col: int("col", 1),
            end_line: int("line", 1),
            end_col: int("col", 1) + name.len() as i64,
            name,
        });
    }
    let outcome = take_snapshot(
        host,
        &json!({ "trigger": "manual", "files": [{ "path": path, "hash": "symbols" }] }),
    )
    .await?
    .ok_or("no snapshot to read the symbols at")?;
    let stream = first_stream(host).await?;
    host.svc
        .symbol_store
        .record(
            stream.value(),
            outcome.snapshot_id,
            vec![FileSymbols {
                path: path.into(),
                language: language.into(),
                rows: Some(rows),
            }],
            SymbolCapture {
                files_collected: 1,
                files_failed: 0,
            },
        )
        .await
        .map_err(|e| e.to_string())
}

/// `run: { extension, contribution, kind?, outcome?, days_ago?, cpu_ms?,
/// ai_calls?, error? }`: a finished run of `extension`'s `contribution`
/// (`kind` a collector by default), recorded as the host does
/// (`extension_run_store::record_tx`); `days_ago` is when it began.
async fn observe_run(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_domain::contribution::{ContributionId, ContributionKind};
    use oxplow_domain::run::{ContributionRef, Mode, Outcome, RunContext};
    let text = |k: &str| body.get(k).and_then(Value::as_str);
    let extension = text("extension").ok_or("`run.extension` names the extension")?;
    let local = text("contribution").ok_or("`run.contribution` names the contribution")?;
    let kind = match text("kind").unwrap_or("collector") {
        "collector" => ContributionKind::Collector,
        "effect" => ContributionKind::Effect,
        "command" => ContributionKind::Command,
        "job" => ContributionKind::Job,
        "advisory" => ContributionKind::Advisory,
        "implementation" => ContributionKind::Implementation,
        other => return Err(format!("`run.kind: {other}` — fix: collector, effect, command, job, advisory or implementation")),
    };
    let outcome = match text("outcome").unwrap_or("ok") {
        "ok" => Outcome::Ok,
        "skipped" => Outcome::Skipped,
        "failed" => Outcome::Failed,
        "denied" => Outcome::Denied,
        "cancelled" => Outcome::Cancelled,
        other => {
            return Err(format!(
                "`run.outcome: {other}` — fix: ok, skipped, failed, denied or cancelled"
            ))
        }
    };
    let mut run = RunContext::contribution(
        ContributionRef {
            id: ContributionId::new(extension, kind, local),
            version: None,
            generation: None,
            deployment: None,
        },
        Actor::System,
        Mode::External,
    );
    if let Some(days) = body.get("days_ago").and_then(Value::as_f64) {
        let at = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() - (days * 86_400_000.0) as i64,
        );
        run = run.began_at(at);
    }
    run.used_cpu(body.get("cpu_ms").and_then(Value::as_u64).unwrap_or(0), 0);
    for _ in 0..body.get("ai_calls").and_then(Value::as_u64).unwrap_or(0) {
        run.used_ai();
    }
    let error = text("error").map(str::to_string);
    host.svc
        .db
        .transaction(move |tx| {
            oxplow_db::extension_run_store::record_tx(tx, &run, outcome, error.as_deref())
        })
        .await
        .map_err(|e| e.to_string())
}

/// `health: { extension, contribution, kind?, disable }`: the contribution
/// disabled after failures, with `disable` as the reason
/// (`contribution_health_store::disable_tx`).
async fn observe_health(host: &Host, body: &Value) -> Result<(), String> {
    let text = |k: &str| body.get(k).and_then(Value::as_str);
    let key = oxplow_db::ContributionKey {
        extension: text("extension").ok_or("`health.extension`")?.to_string(),
        contribution: text("contribution")
            .ok_or("`health.contribution`")?
            .to_string(),
        kind: match text("kind").unwrap_or("collector") {
            "collector" => "collector",
            "effect" => "effect",
            "provider" => "provider",
            other => {
                return Err(format!(
                    "`health.kind: {other}` — fix: collector, effect or provider"
                ))
            }
        },
    };
    let reason = text("disable")
        .ok_or("`health.disable` is the reason it was disabled")?
        .to_string();
    let now = oxplow_domain::Timestamp::now().to_string();
    host.svc
        .db
        .transaction(move |tx| {
            oxplow_db::contribution_health_store::disable_tx(tx, &key, &reason, &now)
        })
        .await
        .map_err(|e| e.to_string())
}

/// The id (`thr3`) or `thr1` of the thread a `thread` key names.
fn thread_text(body: &Value) -> String {
    body.get("thread")
        .and_then(Value::as_str)
        .unwrap_or("thr1")
        .to_string()
}

/// `token_usage: { thread?, effort?, session?, model?, prompt?, input_tokens?,
/// output_tokens?, cache_creation_input_tokens?, cache_read_input_tokens?,
/// turn? }`: the tokens a turn spent (`SqliteTokenUsageStore::record`).
async fn observe_token_usage(host: &Host, body: &Value) -> Result<(), String> {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let int = |k: &str| body.get(k).and_then(Value::as_i64).unwrap_or(0);
    let stream = first_stream(host).await?;
    host.svc
        .token_usage_store
        .record(oxplow_db::NewAgentTokenUsage {
            stream_id: stream.to_string(),
            thread_id: thread_text(body),
            effort_id: text("effort"),
            session_id: text("session").unwrap_or_else(|| "s".into()),
            agent_kind: text("agent").unwrap_or_else(|| "claude".into()),
            model: text("model").or_else(|| Some("m".into())),
            prompt: text("prompt"),
            input_tokens: int("input_tokens"),
            output_tokens: int("output_tokens"),
            cache_creation_input_tokens: int("cache_creation_input_tokens"),
            cache_read_input_tokens: int("cache_read_input_tokens"),
            message_count: body
                .get("message_count")
                .and_then(Value::as_i64)
                .unwrap_or(1),
            turn_id: body.get("turn").and_then(Value::as_i64),
            cause: None,
        })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `hint: { hint, evaluated?, thread? }`: the hint's query ran `evaluated`
/// times (1) on the thread (`SqliteAgentNudgeStore::evaluated`).
async fn observe_hint(host: &Host, body: &Value) -> Result<(), String> {
    let hint = body
        .get("hint")
        .and_then(Value::as_str)
        .ok_or("`hint.hint` names the hint")?;
    let thread = oxplow_domain::ThreadId::try_from_str(&thread_text(body))
        .ok_or("`hint.thread` is a thread, like `thr1`")?;
    for _ in 0..body.get("evaluated").and_then(Value::as_u64).unwrap_or(1) {
        host.svc
            .nudge_store
            .evaluated(thread.value(), vec![hint.to_string()])
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `nudge: { kind, message, thread?, effort?, audience?, delivered? }`: a
/// nudge a hint raised (`SqliteAgentNudgeStore::record`), delivered or not.
async fn observe_nudge(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::agent_nudge_store::{Audience, NewAgentNudge};
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let id = host
        .svc
        .nudge_store
        .record(NewAgentNudge {
            thread_id: thread_text(body),
            effort_id: text("effort"),
            kind: text("kind").ok_or("`nudge.kind`")?,
            message: text("message").unwrap_or_default(),
            trigger: None,
            turn_id: None,
            cause: None,
            audience: match text("audience").as_deref() {
                Some("person") => Audience::Person,
                _ => Audience::Agent,
            },
        })
        .await
        .map_err(|e| e.to_string())?;
    if let (Some(id), true) = (
        id,
        body.get("delivered")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) {
        host.svc
            .nudge_store
            .mark_delivered(vec![id])
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `analyze: { kind: working, streamId } | { kind: commit, sha, streamId? } |
/// …`: the change is made and analyzed to the end
/// (`change_analysis::ensure_change`, what `oxplow_ensure_change` calls), so a
/// page reads its files and functions without waiting for the background
/// analysis. `streamId` of `working` defaults to the primary stream's.
async fn observe_analyze(host: &Host, body: &Value) -> Result<(), String> {
    let mut target = body.clone();
    if target.get("kind").and_then(Value::as_str) == Some("working")
        && target.get("streamId").is_none()
    {
        let stream = first_stream(host).await?;
        target["streamId"] = json!(stream.to_string());
    }
    let target: oxplow_app::change_analysis::ChangeTarget =
        serde_json::from_value(target).map_err(|e| format!("`analyze`: {e}"))?;
    oxplow_app::change_analysis::ensure_change(&host.svc, target)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `refs: { branches?: [{ name, kind?, head, stream?, default?, upstream?,
/// ahead?, behind? }], tags?: [{ name, sha }], heads?: [{ stream, sha }] }`:
/// the repository's refs as the git indexer stores them
/// (`SqliteGitStore::replace_branches` / `replace_tags` /
/// `replace_stream_heads`); a part left out keeps what was stored.
async fn observe_refs(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::git_store::{GitBranchRow, GitTagRow};
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| body.get(k).and_then(Value::as_array).cloned();
    let git = &host.svc.git_store;
    if let Some(branches) = list("branches") {
        let mut rows = Vec::new();
        for b in &branches {
            rows.push(GitBranchRow {
                name: text(b, "name").ok_or("a branch names its `name`")?,
                kind: text(b, "kind").unwrap_or_else(|| "local".into()),
                remote: text(b, "remote"),
                head_sha: text(b, "head"),
                stream_id: b.get("stream").and_then(Value::as_i64),
                is_default: b.get("default").and_then(Value::as_bool).unwrap_or(false),
                upstream: text(b, "upstream"),
                ahead: b.get("ahead").and_then(Value::as_u64).unwrap_or(0) as u32,
                behind: b.get("behind").and_then(Value::as_u64).unwrap_or(0) as u32,
            });
        }
        git.replace_branches(rows)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(tags) = list("tags") {
        let mut rows = Vec::new();
        for t in &tags {
            rows.push(GitTagRow {
                name: text(t, "name").ok_or("a tag names its `name`")?,
                sha: text(t, "sha").ok_or("a tag names its `sha`")?,
            });
        }
        git.replace_tags(rows).await.map_err(|e| e.to_string())?;
    }
    if let Some(heads) = list("heads") {
        let mut rows = Vec::new();
        for h in &heads {
            rows.push((
                h.get("stream")
                    .and_then(Value::as_i64)
                    .ok_or("a head names its `stream`")?,
                text(h, "sha").ok_or("a head names its `sha`")?,
            ));
        }
        git.replace_stream_heads(rows)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `visit: { page, kind?, label?, thread?, duration_ms? }`: a page the person
/// opened (`page` its ref, `kind` its page kind, `page` by default), recorded
/// as the shell's `record_page_visit` does (`PageVisitStore::record`).
async fn observe_visit(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::analytics_stores::PageVisitStore as _;
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    let page = text("page").ok_or("`visit.page` is the page's ref")?;
    host.svc
        .page_visit_store
        .record(
            text("kind").unwrap_or("page"),
            page,
            text("label"),
            body.get("duration_ms").and_then(Value::as_i64),
            text("thread"),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `facts: { measure, producer?, items?: [{ value?, path?, line?, rule?,
/// severity?, detail?, subject?, dims? }] }`: findings of `measure` (declared by
/// core or an extension), recorded as one capture
/// (`SqliteFactStore::record_facts`), the way a collector's run records them.
async fn observe_facts(host: &Host, body: &Value) -> Result<(), String> {
    let key = body
        .get("measure")
        .and_then(Value::as_str)
        .ok_or("`facts.measure` names the measure")?;
    let producer = body.get("producer").and_then(Value::as_str).unwrap_or(key);
    let facts = &host.svc.fact_store;
    let measure = facts
        .get_measure(key)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no measure `{key}` — fix: declare it in `measures:`"))?;
    let rows: Vec<oxplow_db::NewFact> = body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            let text = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_string);
            oxplow_db::NewFact {
                subject: text("subject"),
                path: text("path"),
                line: f.get("line").and_then(Value::as_i64),
                severity: text("severity"),
                rule: text("rule"),
                detail: text("detail"),
                dims_json: f.get("dims").map(Value::to_string),
                ..oxplow_db::NewFact::new(
                    measure.id,
                    f.get("value").and_then(Value::as_f64).unwrap_or(1.0),
                )
            }
        })
        .collect();
    let stream = first_stream(host).await?;
    facts
        .record_facts(
            oxplow_db::NewMetricCapture::done(stream.value(), producer, "hook"),
            rows,
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `retention: { days }`: the event log's retention sweep
/// (`event_retention::sweep`) as it runs `days` from now — a payload past
/// its kind's keep expires, its envelope (type, subject, cause) stays.
async fn observe_retention(host: &Host, body: &Value) -> Result<(), String> {
    let days = body
        .get("days")
        .and_then(Value::as_i64)
        .ok_or("`retention.days` is how many days ahead the sweep runs")?;
    let later = oxplow_domain::Timestamp::from_unix_ms(
        oxplow_domain::Timestamp::now().unix_ms() + days * oxplow_db::event_retention::DAY_MS,
    );
    oxplow_db::event_retention::sweep(&host.svc.db, later, &Default::default())
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `collect: { effort, collectors?: [key] }`: the fact collectors an
/// `effort.finished` triggers (`MetricsService::run_effort_collectors`),
/// run for the effort now; `collectors` names the ones to run (by key),
/// else every one runs.
async fn observe_collect(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::effort_store::EffortStore as _;
    use oxplow_domain::EffortId;
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`collect.effort` is an effort's id, like `eff1`")?;
    let found = host
        .svc
        .effort_store
        .get_effort(&EffortId::new(effort))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no effort eff{effort}"))?;
    let wanted: Option<Vec<String>> = body.get("collectors").and_then(Value::as_array).map(|l| {
        l.iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect()
    });
    host.svc
        .metrics
        .run_effort_collectors(&found.thread_id, &found.id, None, &|c| {
            wanted.as_ref().is_none_or(|w| w.contains(&c.key))
        })
        .await;
    Ok(())
}

/// `snapshot: { trigger?, elapsed_ms?, budget_ms?, revision?, branch?, files?: [{ path, hash }] }`:
/// one take in the stream's snapshot log (`SqliteSnapshotStore::
/// record_take`): its parent is the take before it, and it is over budget
/// when `elapsed_ms` is more than `budget_ms`. A take whose files hash as
/// its parent's makes no new snapshot.
async fn observe_snapshot(host: &Host, body: &Value) -> Result<(), String> {
    take_snapshot(host, body).await.map(|_| ())
}

async fn take_snapshot(
    host: &Host,
    body: &Value,
) -> Result<Option<oxplow_db::TakeOutcome>, String> {
    use oxplow_db::{FileSnapshot, SnapshotStorage, TakeRecord};
    use oxplow_domain::snapshot::SnapshotTrigger;
    let stream = first_stream(host).await?;
    let trigger = match body.get("trigger").and_then(Value::as_str) {
        Some(t) => SnapshotTrigger::from_db_str(t)
            .ok_or_else(|| format!("`{t}` isn't a snapshot trigger"))?,
        None => SnapshotTrigger::Manual,
    };
    let mut rows = Vec::new();
    for f in body
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| f.get(key).and_then(Value::as_str);
        rows.push(FileSnapshot {
            id: 0,
            stream_id: stream,
            path: text("path")
                .ok_or("a snapshot file names its `path`")?
                .to_string(),
            blob_hash: Some(text("hash").unwrap_or("h").to_string()),
            size_bytes: 1,
            captured_at: oxplow_domain::Timestamp::now(),
            storage: SnapshotStorage::Oxplow,
            snapshot_id: None,
            mtime_ms: Some(1),
            content_hash: None,
        });
    }
    host.svc
        .snapshot_store
        .record_take(TakeRecord {
            stream_id: stream,
            rows,
            trigger,
            thread_id: None,
            turn_id: None,
            effort_id: None,
            branch: body
                .get("branch")
                .and_then(Value::as_str)
                .map(str::to_string),
            revision: match body.get("revision").and_then(Value::as_str) {
                Some(r) => Some(r.parse().map_err(|e| format!("`snapshot.revision`: {e}"))?),
                None => None,
            },
            elapsed_ms: body.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(1),
            budget_ms: body.get("budget_ms").and_then(Value::as_u64),
            source: "fixture".into(),
            provider: None,
            contents: false,
            handle: None,
        })
        .await
        .map_err(|e| e.to_string())
}

/// `commit: { sha, at, subject, author?, email?, body?, parents?, files?:
/// [{ path, status?, additions?, deletions? }] }`: a commit the git indexer
/// read, stored as it stores one (`SqliteGitStore::upsert_commit`).
async fn observe_commit(host: &Host, body: &Value) -> Result<(), String> {
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    let at = text("at").ok_or("`commit.at` is when it was committed (RFC 3339)")?;
    let committed = oxplow_domain::Timestamp::parse(at).map_err(|e| format!("`commit.at`: {e}"))?;
    let mut files = Vec::new();
    for f in body
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        files.push(oxplow_db::git_store::GitCommitFileRow {
            path: f
                .get("path")
                .and_then(Value::as_str)
                .ok_or("a commit file names its `path`")?
                .to_string(),
            status: f
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("M")
                .to_string(),
            additions: f.get("additions").and_then(Value::as_i64).unwrap_or(0),
            deletions: f.get("deletions").and_then(Value::as_i64).unwrap_or(0),
        });
    }
    host.svc
        .git_store
        .upsert_commit(oxplow_db::git_store::GitCommitRow {
            sha: text("sha").ok_or("`commit.sha` names it")?.to_string(),
            author: text("author").unwrap_or("fixture").to_string(),
            email: text("email").unwrap_or("fixture@example.test").to_string(),
            committed_secs: committed.unix_ms() / 1000,
            subject: text("subject").unwrap_or_default().to_string(),
            body: text("body").unwrap_or_default().to_string(),
            parents: body
                .get("parents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect(),
            files,
        })
        .await
        .map_err(|e| e.to_string())
}

/// `evidence: { effort, observations: [{ kind, metric_value?, payload?,
/// provenance?, source? }] }`: what an effort's evidence holds — its
/// `test-run`s and `diff-coverage` — as the evidence store keeps it
/// (`EffortEvidenceStore::replace`, what an effort's refresh calls); the
/// observations are the effort's whole set.
async fn observe_evidence(host: &Host, body: &Value) -> Result<(), String> {
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`evidence.effort` is an effort's id, like `eff1`")?;
    let mut observations = Vec::new();
    for o in body
        .get("observations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str, default: &str| {
            o.get(key)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string()
        };
        observations.push(oxplow_db::EffortObservation {
            kind: o
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("an observation names its `kind`")?
                .to_string(),
            provenance: text("provenance", "observed"),
            source: text("source", "fixture"),
            metric_value: o.get("metric_value").and_then(Value::as_f64),
            payload_json: o.get("payload").map(Value::to_string),
            local_snapshot_id: None,
            created_at: oxplow_domain::Timestamp::now(),
        });
    }
    let mut deltas = Vec::new();
    for d in body
        .get("metric_deltas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
        let num = |key: &str| d.get(key).and_then(Value::as_f64);
        let key = text("key").ok_or("a metric delta names its `key`")?;
        let current = num("current").ok_or("a metric delta names its `current`")?;
        deltas.push(oxplow_db::EffortMetricDelta {
            title: text("title").unwrap_or_else(|| key.clone()),
            key,
            unit: text("unit"),
            direction: text("direction").unwrap_or_else(|| "lower-better".into()),
            kind: text("kind").unwrap_or_else(|| "gauge".into()),
            category: text("category"),
            language: text("language"),
            agg: text("agg").unwrap_or_else(|| "files".into()),
            baseline: num("baseline"),
            current,
            delta: num("delta").or_else(|| num("baseline").map(|b| current - b)),
            changed: d.get("changed").and_then(Value::as_bool).unwrap_or(true),
            attributed_files: d.get("attributed_files").and_then(Value::as_i64),
            sample_count: d.get("sample_count").and_then(Value::as_i64).unwrap_or(1),
            target: num("target"),
            warn_at: num("warn_at"),
            fail_at: num("fail_at"),
            crossing: text("crossing"),
            latest_capture_id: None,
        });
    }
    host.svc
        .effort_evidence_store
        .replace(effort, deltas, observations, String::new())
        .await
        .map_err(|e| e.to_string())
}

async fn observe_turn(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_app::hook_ingest::{HookEnvelope, LAST_ASSISTANT_MESSAGE};
    use oxplow_domain::hook::HookKind;
    for key in body.as_object().into_iter().flat_map(|m| m.keys()) {
        if ![
            "prompt",
            "during",
            "tools",
            "write",
            "answer",
            "compactions",
            "context",
            "thread",
            "session",
            "subagent",
            "peer",
        ]
        .contains(&key.as_str())
        {
            return Err(format!(
                "`turn.{key}` — fix: `prompt`, `during`, `tools`, `write`, `answer`, `compactions`, `context`, `thread`, `session`, `subagent` or `peer`"
            ));
        }
    }
    let svc = &host.svc;
    let thread = oxplow_domain::ThreadId::try_from_str(&thread_text(body))
        .ok_or("`turn.thread` is a thread, like `thr2`")?;
    let session = match body.get("session").and_then(Value::as_str) {
        Some(s) => Some(
            oxplow_domain::AgentSessionId::try_from_str(s)
                .ok_or("`turn.session` is an agent session, like `ses2`")?,
        ),
        None => None,
    };
    let envelope = |kind: HookKind, payload: Value| HookEnvelope {
        session_title: None,
        turn_run: None,
        kind,
        thread_id: Some(thread),
        stream_id: None,
        agent_session_id: session,
        session_id: Some("fixture".into()),
        payload_json: payload.to_string(),
        prompt: None,
        decision: None,
        tool: None,
        subagent: None,
        peer: None,
    };
    let ingest = |env: HookEnvelope| async move {
        svc.hook_ingest
            .ingest(env)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    // A turn that writes starts from a snapshot of the tree as it is.
    let capture = match body.get("write") {
        Some(_) => {
            let stream = svc
                .streams
                .list_streams()
                .await
                .map_err(|e| e.to_string())?
                .first()
                .map(|s| s.id)
                .ok_or("no stream")?;
            let capture = svc
                .snapshot_captures
                .get(&stream)
                .ok_or("no snapshot capture")?;
            capture
                .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
                .await
                .map_err(|e| e.to_string())?;
            Some(capture)
        }
        None => None,
    };
    let mut prompt = envelope(HookKind::UserPromptSubmit, json!({}));
    // No prompt (`prompt: null`) is a subagent's or a peer's hand-back.
    prompt.prompt = match body.get("prompt") {
        Some(Value::Null) => None,
        Some(p) => Some(p.as_str().unwrap_or_default().to_string()),
        None => Some("Work on it".to_string()),
    };
    if let Some(sub) = body.get("subagent") {
        prompt.subagent =
            Some(serde_json::from_value(sub.clone()).map_err(|e| format!("`turn.subagent`: {e}"))?);
    }
    prompt.peer = body.get("peer").and_then(Value::as_str).map(str::to_string);
    ingest(prompt).await?;
    for step in body
        .get("during")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let command = step
            .get("command")
            .and_then(Value::as_str)
            .ok_or("`turn.during` steps are `{ command, input?, as? }`")?;
        svc.commands
            .run(
                &step_actor(step)?,
                command,
                step.get("input").cloned().unwrap_or_else(|| json!({})),
                step.get("confirmed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await
            .map_err(|e| format!("`turn.during` `{command}` failed: {e}"))?;
    }
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // `pre: true` is the call as it starts (PreToolUse), not as it ends.
        let mut tool = tool.clone();
        let pre = tool
            .as_object_mut()
            .and_then(|m| m.remove("pre"))
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let used: oxplow_domain::agent::tool::ToolUse =
            serde_json::from_value(tool).map_err(|e| format!("a tool: {e}"))?;
        let mut call = envelope(
            if pre {
                HookKind::PreToolUse
            } else {
                HookKind::PostToolUse
            },
            json!({}),
        );
        call.tool = Some(used);
        ingest(call).await?;
    }
    // The calls are in the datasets before the turn ends, as the app's pump
    // has them by then: the end's checkpoint counts them.
    svc.event_pump
        .deliver_projections()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(write) = body.get("write") {
        let (Some(path), Some(text)) = (
            write.get("path").and_then(Value::as_str),
            write.get("body").and_then(Value::as_str),
        ) else {
            return Err("`turn.write` is `{ path, body }`".into());
        };
        let file = svc.layout.project_dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        capture
            .ok_or("no snapshot capture")?
            .enqueue_startup_diff()
            .await
            .map_err(|e| e.to_string())?;
    }
    for _ in 0..body.get("compactions").and_then(Value::as_u64).unwrap_or(0) {
        ingest(envelope(HookKind::PostCompact, json!({}))).await?;
    }
    let mut payload = json!({});
    if let Some(answer) = body.get("answer") {
        payload[LAST_ASSISTANT_MESSAGE] = answer.clone();
    }
    let mut stop = envelope(HookKind::Stop, payload);
    if let Some(context) = body.get("context") {
        stop.turn_run = Some(oxplow_domain::hook::TurnRun {
            context_tokens: context.get("tokens").and_then(Value::as_i64),
            context_window: context.get("window").and_then(Value::as_i64),
            ..Default::default()
        });
    }
    ingest(stop).await
}

/// Who runs a step's command (`as:`): a person, or `agent` (the primary
/// thread's).
fn step_actor(step: &Value) -> Result<Actor, String> {
    match step.get("as").and_then(Value::as_str) {
        None | Some("person") => Ok(Actor::Human),
        Some("agent") => Ok(Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(1)),
            stream_id: None,
        }),
        Some(other) => Err(format!("`as: {other}` — fix: `person` or `agent`")),
    }
}

/// Run `when: { command, input?, as?, confirmed? }` for real, through the
/// bus, and let what it set off settle: `{ ok: true, result }` or `{ ok:
/// false, refused }`, and `commands`, the ones that ran because of it (its
/// script's, an effect's), checked against `expect`.
async fn command_run(host: &Host, ex: &Example<'_>, command: &str, report: &mut TestReport) {
    let shown = ex.shown;
    host.current().await;
    let actor = match step_actor(&ex.when) {
        Ok(actor) => actor,
        Err(e) => return report.errors.push(format!("{shown}:1: {e}")),
    };
    let head = host
        .svc
        .sql
        .query_sql(
            "SELECT COALESCE(MAX(id), 0) FROM v_command_audit",
            vec![],
            None,
        )
        .await
        .ok()
        .and_then(|r| r.rows.first().and_then(|row| row.first()).cloned())
        .unwrap_or(oxplow_db::SqlCell::Int(0));
    let outcome = host
        .svc
        .commands
        .run(
            &actor,
            command,
            ex.when.get("input").cloned().unwrap_or_else(|| json!({})),
            ex.when
                .get("confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
        .await;
    host.settle().await;
    // A composite (a script's command) answers `{ result, children }`:
    // its own answer, and the commands it ran.
    let mut composed = Vec::new();
    let mut got = match outcome {
        Ok(out) => match (out.result.get("result"), out.result.get("children")) {
            (Some(result), Some(Value::Array(children))) => {
                composed = children
                    .iter()
                    .filter_map(|c| c.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                json!({ "ok": true, "result": result })
            }
            _ => json!({ "ok": true, "result": out.result }),
        },
        Err(e) => json!({ "ok": false, "refused": e.to_string() }),
    };
    let mut audited: Vec<(String, Option<i64>)> = host
        .svc
        .sql
        .query_sql(
            "SELECT command, id FROM v_command_audit WHERE id > ?1 ORDER BY id",
            vec![head],
            None,
        )
        .await
        .map(|r| r.rows)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row.as_slice() {
            [oxplow_db::SqlCell::Text(name), oxplow_db::SqlCell::Int(id)] => {
                Some((name.clone(), Some(*id)))
            }
            _ => None,
        })
        .collect();
    if let Some(own) = audited.iter().position(|(name, _)| name == command) {
        audited.remove(own);
    }
    let mut ran = without_sequences(host, audited).await;
    composed.append(&mut ran);
    let ran = composed;
    got["commands"] = json!(ran);
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: command `{command}` gave {at} at `{path}`, the example expects {want} — \
             fix: the command's input or the steps before it, or the example's `expect` (`{{ ok, \
             result?, commands? }}` or `{{ ok: false, refused }}`)"
        ));
    }
}

/// Values a fixture bound (`bind:`), by name: a step's command result, or
/// a value of a page run. `$name` followed by `.key` and `[n]` parts reads
/// one; `$any` is never a name.
#[derive(Default)]
struct Binds(std::collections::BTreeMap<String, Value>);

impl Binds {
    /// `value` with every string that is exactly a reference replaced by
    /// what it reads, its type kept.
    fn substitute(&self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::String(text) => match reference(text) {
                Some(reference) => self.resolve(reference)?,
                None => value.clone(),
            },
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.substitute(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), self.substitute(v)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }

    /// What `name.path` reads.
    fn resolve(&self, reference: &str) -> Result<Value, String> {
        let end = reference.find(['.', '[']).unwrap_or(reference.len());
        let (name, path) = reference.split_at(end);
        let bound = self.0.get(name).ok_or_else(|| {
            format!("`${name}` isn't bound — fix: `bind: {name}` on an earlier step")
        })?;
        read_path(bound, path).ok_or_else(|| format!("`${reference}` reads nothing"))
    }
}

/// The name and path a string refers to (`$first.uri` → `first.uri`):
/// `$` and a letter or `_`, and not `$any`.
fn reference(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('$')?;
    (text != crate::conformance::ANY
        && rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
    .then_some(rest)
}

/// The value at `path` (`.key` and `[n]` parts, or none) in `value`.
fn read_path(value: &Value, path: &str) -> Option<Value> {
    let mut at = value;
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let close = after.find(']')?;
            at = at.get(after[..close].parse::<usize>().ok()?)?;
            rest = &after[close + 1..];
        } else {
            let after = rest.strip_prefix('.').unwrap_or(rest);
            let end = after.find(['.', '[']).unwrap_or(after.len());
            at = at.get(&after[..end])?;
            rest = &after[end..];
        }
    }
    Some(at.clone())
}

/// A run as a fixture's `bind` reads it: each loader's rows as maps by
/// column, and its alert, badge and what keeps it inactive.
fn run_json(run: &oxplow_app::pages::runner::PageRun) -> Value {
    let loaders: serde_json::Map<String, Value> = run
        .loaders
        .iter()
        .map(|(name, loader)| {
            let rows: Vec<Value> = loader
                .result
                .rows
                .iter()
                .map(|row| {
                    Value::Object(
                        loader
                            .result
                            .columns
                            .iter()
                            .zip(row)
                            .map(|(column, cell)| (column.clone(), json!(cell)))
                            .collect(),
                    )
                })
                .collect();
            (
                name.clone(),
                json!({ "rows": rows, "columns": loader.result.columns }),
            )
        })
        .collect();
    json!({ "loaders": loaders, "alert": run.alert, "badge": run.badge, "inactive": run.inactive })
}

/// Load component `id` as its frame would, then ask what the fixture asks
/// (`when: { component, items?: [title], invokes?: [{ command, input? }]
/// }`): `items` are work items filed first
/// (`oxplow:oxplow/work_item?id=tsk1&provider=oxplow`, …); each invoke's
/// outcome — what the command answers when the component calls it —
/// against `expect: { invokes?: [{ ok?, refused? }] }`. The data a
/// component draws is its page's loaders: a page example tests those. The
/// frame itself isn't drawn here.
async fn component_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::viz_actions::{invoke_component_command, load_component, ComponentInvoke};
    let shown = ex.shown;
    // The component itself: it acts as itself, whichever page shows it.
    let key = format!("{}/{id}", ext.name);
    let version = match load_component(&host.svc, &host.root, &key) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("{shown}:1: component `{id}` doesn't load: {e}"));
            return;
        }
    };
    for title in ex
        .when
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
    let mut invokes = Vec::new();
    for i in ex
        .when
        .get("invokes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let out = invoke_component_command(
            &host.svc,
            &host.root,
            ComponentInvoke {
                component: key.clone(),
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
            // Denied or failed alike: it said no, and why.
            Err(oxplow_domain::CommandError::Denied { reason }) => {
                json!({ "ok": false, "refused": reason })
            }
            Err(e) => json!({ "ok": false, "refused": e.to_string() }),
        });
    }
    let got = json!({ "invokes": invokes });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: component `{id}` answered {got} at `{path}`, the example expects {want} \
             — fix: the component's declarations, or the example's `expect` (`{{ invokes: [{{ \
             ok, refused? }}] }}`)"
        ));
    }
}

/// Open page `slug`'s form as a person would (`when: { form, node?,
/// params? }`): `expect: { values?, fixed?, fields? }` against the values
/// the fields start from (the form's `defaults`, its loader's first row
/// laid over them), the fields shown read-only, and the fields shown
/// (`null`: all).
async fn form_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    slug: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let params = ex
        .when
        .get("params")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let start = match ext.page_files.iter().find(|p| p.slug == slug) {
        Some(page) => page_form_start(host, page, ex.when.get("node"), params).await,
        None => Err(format!("`{slug}` isn't one of this extension's pages")),
    };
    let start = match start {
        Ok(start) => start,
        Err(e) => {
            report.errors.push(format!(
                "{shown}:1: form `{slug}` didn't open: {e} — fix: the form (a `viz: form` node with \
                 a `form:` naming a command; `node` names which), or the example's params"
            ));
            return;
        }
    };
    let got = json!({ "values": start.values, "fixed": start.fixed, "fields": start.fields });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: form `{slug}` opened with {got} at `{path}`, the example expects {want} \
             — fix: the form's `defaults`, `fixed` or `fields`, or the example's `expect` (`{{ \
             values?, fixed?, fields? }}`)"
        ));
    }
}

/// Open a page's form: its `node` (else its first form node) on a run of
/// the page with `params`.
async fn page_form_start(
    host: &Host,
    page: &oxplow_app::pages::schema::Page,
    node: Option<&Value>,
    params: std::collections::BTreeMap<String, oxplow_db::SqlCell>,
) -> Result<oxplow_app::viz_actions::FormStart, String> {
    use oxplow_app::pages::schema::NodeKind;
    let node = match node.and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => page
            .nodes()
            .into_iter()
            .find(|n| {
                matches!(&n.kind, NodeKind::Viz { config: cfg, .. }
                    if cfg.viz == oxplow_app::pages::spec::Viz::Form)
            })
            .map(|n| n.id.clone())
            .ok_or_else(|| "the page has no form node".to_string())?,
    };
    let (run_id, _) = host
        .svc
        .page_runs
        .open(
            &host.svc,
            oxplow_app::pages::runner::RunRequest {
                target: oxplow_app::pages::runner::RunTarget::Page(page.id.clone()),
                params,
                ctx: oxplow_app::pages::spec::ViewContext::default(),
                viewer: oxplow_domain::Actor::Human,
            },
            "extension-test",
        )
        .await
        .map_err(|e| e.to_string())?;
    let start =
        oxplow_app::pages::actions::page_form(&host.svc, &host.svc.page_runs, &run_id, &node)
            .await
            .map_err(|e| e.to_string());
    host.svc.page_runs.close(&run_id);
    start
}

/// One intent example with its fixture.
#[derive(Clone)]
struct Example<'a> {
    name: &'a str,
    /// Its fixture file, as findings name it.
    shown: &'a str,
    /// What it does: one key names the kind (`page`, `command`, `event`,
    /// …), the rest are that kind's.
    when: Value,
    expect: Value,
    /// Steps run before it (`given:`).
    given: Value,
    /// A page example's scenario (`steps:`): `{ given }` changes the state,
    /// `{ expect, params?, bind? }` runs the page again and checks it.
    steps: Option<Value>,
    /// The stream and thread a page is viewed from (`view: { stream,
    /// thread }`).
    view: Value,
    /// What the fixture model answers its model calls with (`model: {
    /// replies }`).
    model: Value,
}

/// Loader `name` of a run: its own, or — `<embed>/<loader>` — one of the
/// page an embed node or a slot mount (`<extension>.<page>`) runs there.
fn loader_at<'r>(
    run: &'r oxplow_app::pages::runner::PageRun,
    name: &str,
) -> Option<&'r oxplow_app::pages::runner::LoaderRun> {
    match name.split_once('/') {
        None => run.loaders.get(name),
        Some((embed, rest)) => embed_of(run, embed).and_then(|sub| loader_at(sub, rest)),
    }
}

/// The run of the page node `id` of `run` embeds (an embed, a slot's
/// mount, a replacement), in its view or its rail.
fn embed_of<'r>(
    run: &'r oxplow_app::pages::runner::PageRun,
    id: &str,
) -> Option<&'r oxplow_app::pages::runner::PageRun> {
    fn node<'r>(
        at: &'r oxplow_app::pages::runner::RunNode,
        id: &str,
    ) -> Option<&'r oxplow_app::pages::runner::RunNode> {
        if at.id == id {
            return Some(at);
        }
        at.children.iter().find_map(|c| node(c, id))
    }
    node(&run.tree, id)
        .or_else(|| run.rail.as_ref().and_then(|rail| node(rail, id)))
        .and_then(|n| n.embed.as_deref())
}

/// The run `path` (embed node ids, outermost first) leads to from `run`.
fn embedded_at(
    run: &oxplow_app::pages::runner::PageRun,
    path: &[String],
) -> Option<oxplow_app::pages::runner::PageRun> {
    match path.split_first() {
        None => Some(run.clone()),
        Some((head, rest)) => embedded_at(embed_of(run, head)?, rest),
    }
}

/// The component node a `ui` step drives in `run`: the one `named` names
/// (`<embed>/<node>` inside an embed, a mount or a replacement), else the
/// page's first component node — or, with none, the first in a page it
/// embeds. The embed path to the run holding it, that run, the node and
/// its component.
fn component_at(
    run: &oxplow_app::pages::runner::PageRun,
    named: Option<&str>,
) -> Option<(
    Vec<String>,
    oxplow_app::pages::runner::PageRun,
    String,
    String,
)> {
    let inside = |embed: &str, rest: Option<&str>| {
        let (mut path, inner, node, component) = component_at(embed_of(run, embed)?, rest)?;
        path.insert(0, embed.to_string());
        Some((path, inner, node, component))
    };
    if let Some((embed, rest)) = named.and_then(|n| n.split_once('/')) {
        return inside(embed, Some(rest));
    }
    let own = run.page.nodes().into_iter().find_map(|n| {
        let component = match &n.kind {
            oxplow_app::pages::schema::NodeKind::Component { component, .. } => component.clone(),
            oxplow_app::pages::schema::NodeKind::Viz { config, .. }
                if config.viz == oxplow_app::pages::spec::Viz::Custom =>
            {
                config.custom.as_ref()?.component.clone()?
            }
            _ => return None,
        };
        (named.is_none_or(|id| id == n.id)).then(|| (n.id.clone(), component))
    });
    if let Some((node, component)) = own {
        return Some((Vec::new(), run.clone(), node, component));
    }
    if named.is_some() {
        return None;
    }
    fn embeds(at: &oxplow_app::pages::runner::RunNode, out: &mut Vec<String>) {
        if at.embed.is_some() {
            out.push(at.id.clone());
        }
        at.children.iter().for_each(|c| embeds(c, out));
    }
    let mut ids = Vec::new();
    embeds(&run.tree, &mut ids);
    ids.iter().find_map(|id| inside(id, None))
}

/// A page run's node ids (`<embed>/<id>` inside an embed) with their kinds.
fn run_nodes(run: &oxplow_app::pages::runner::PageRun) -> Vec<(String, String)> {
    fn walk(
        run: &oxplow_app::pages::runner::PageRun,
        node: &oxplow_app::pages::runner::RunNode,
        prefix: &str,
        out: &mut Vec<(String, String)>,
    ) {
        let kind = run
            .page
            .nodes()
            .into_iter()
            .find(|n| n.id == node.id)
            .and_then(|n| serde_json::to_value(n).ok())
            .and_then(|v| v.get("kind").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        out.push((format!("{prefix}{}", node.id), kind));
        if let Some(sub) = &node.embed {
            walk(sub, &sub.tree, &format!("{prefix}{}/", node.id), out);
        }
        for child in &node.children {
            walk(run, child, prefix, out);
        }
    }
    let mut out = Vec::new();
    walk(run, &run.tree, "", &mut out);
    if let Some(rail) = &run.rail {
        walk(run, rail, "", &mut out);
    }
    out
}

/// Run `target` (a page, `<extension>/<slug>`, or an inline one) with
/// `params` as a person would open it from `ctx`; the failure to run, or a
/// loader that failed, as a message.
async fn run_page(
    host: &Host,
    target: oxplow_app::pages::runner::RunTarget,
    params: &Value,
    ctx: &oxplow_app::pages::spec::ViewContext,
) -> Result<oxplow_app::pages::runner::PageRun, String> {
    use oxplow_app::pages::runner::{PageRunner, RunRequest};
    host.current().await;
    let params = params
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let run = PageRunner::of(&host.svc)
        .run(RunRequest {
            target,
            params,
            ctx: ctx.clone(),
            viewer: Actor::Human,
        })
        .await
        .map_err(|e| e.to_string())?;
    if let Some((name, error)) = run
        .loaders
        .iter()
        .find_map(|(name, l)| l.error.as_ref().map(|e| (name, e)))
    {
        return Err(format!("loader `{name}` failed: {error}"));
    }
    Ok(run)
}

/// The example's `view: { stream?, thread? }` (`str1`, `thr1` or their
/// numbers) as the context its page runs in; `None`, with the error
/// reported, when it isn't that shape.
fn view_context(
    ex: &Example<'_>,
    report: &mut TestReport,
) -> Option<oxplow_app::pages::spec::ViewContext> {
    /// A view's id: its number, or its ref read by `parse` (`thr1`).
    fn id(v: Option<&Value>, parse: fn(&str) -> Option<i64>) -> Result<Option<i64>, ()> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => n.as_i64().map(Some).ok_or(()),
            Some(Value::String(s)) => parse(s).map(Some).ok_or(()),
            Some(_) => Err(()),
        }
    }
    let view = &ex.view;
    if !(view.is_null() || view.is_object()) {
        report.errors.push(format!(
            "{}:1: `view` isn't `{{ stream?, thread? }}`",
            ex.shown
        ));
        return None;
    }
    match (
        id(view.get("stream"), |s| {
            oxplow_domain::StreamId::try_from_str(s).map(|i| i.value())
        }),
        id(view.get("thread"), |s| {
            oxplow_domain::ThreadId::try_from_str(s).map(|i| i.value())
        }),
    ) {
        (Ok(stream_id), Ok(thread_id)) => Some(oxplow_app::pages::spec::ViewContext {
            stream_id,
            thread_id,
            active: None,
        }),
        _ => {
            report.errors.push(format!(
                "{}:1: `view` names a stream or thread that isn't one — fix: `{{ stream: str1, \
                 thread: thr1 }}`",
                ex.shown
            ));
            None
        }
    }
}

/// Run page `page` after the fixture's `given` steps, with its `params`,
/// and check it against `expect`; then each of its `steps:` in order — a
/// `given` changes the state, an `expect` (with `params` over the
/// example's, and `bind` to keep a value of the run) runs the page again
/// and checks it. `expect: { loaders?: { <name>: { rows: n | $any | [ {
/// col: val } | [ val, … ] ], columns? } }, nodes?: [{ id, kind?, actions?
/// }], text?, alert?, badge?, inactive?, actions? }`. A row map matches when
/// every column it names equals; a row list matches the loader's rows in
/// order. Anything may use what was bound (`$name.path`).
async fn page_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    page: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let mut binds = Binds::default();
    let Some(ctx) = view_context(ex, report) else {
        return;
    };
    if !ex.given.is_null()
        && !run_steps(host, ext, shown, "given", &ex.given, &mut binds, report).await
    {
        return;
    }
    if !seed_rows(host, ext, ex, report).await {
        return;
    }
    let id = if page.contains('/') {
        page.to_string()
    } else {
        format!("{}/{page}", ext.name)
    };
    let params = ex.when.get("params").cloned().unwrap_or(Value::Null);
    let at = PageAt {
        host,
        shown,
        page,
        target: oxplow_app::pages::runner::RunTarget::Page(id.clone()),
        ctx: &ctx,
    };
    let steps = match &ex.steps {
        None => Vec::new(),
        Some(Value::Array(steps)) => steps.clone(),
        Some(_) => {
            report.errors.push(format!(
                "{shown}:1: `steps` isn't a list — fix: `steps: [{{ given: [..] }} | {{ expect, \
                 params?, bind? }}]`"
            ));
            return;
        }
    };
    if (ex.steps.is_none() || !ex.expect.is_null())
        && at
            .check(&params, &ex.expect, &binds, "", report)
            .await
            .is_none()
    {
        return;
    }
    for (index, step) in steps.iter().enumerate() {
        let label = format!("steps[{index}]");
        if let Some(given) = step.get("given") {
            if !run_steps(
                host,
                ext,
                shown,
                &format!("{label}.given"),
                given,
                &mut binds,
                report,
            )
            .await
            {
                return;
            }
        }
        let mut run_params = params.as_object().cloned().unwrap_or_default();
        if let Some(over) = step.get("params").and_then(Value::as_object) {
            run_params.extend(over.clone());
        }
        if let Some(ui) = step.get("ui") {
            let params = Value::Object(run_params);
            if !at
                .ui(ext, &params, (step, ui), &mut binds, &label, report)
                .await
            {
                return;
            }
            continue;
        }
        if let Some(when) = step.get("when") {
            let expect = step.get("expect").cloned().unwrap_or(Value::Null);
            if !step_run(host, ext, ex, (when, &expect), &binds, &label, report).await {
                return;
            }
            continue;
        }
        if step.get("press").is_some() || step.get("submit").is_some() {
            let Some(outcome) = at
                .act(&Value::Object(run_params), step, &binds, &label, report)
                .await
            else {
                return;
            };
            if !bind_from(step, &outcome, &mut binds, shown, &label, report) {
                return;
            }
            continue;
        }
        let Some(expect) = step.get("expect") else {
            if step.get("given").is_none() {
                report.errors.push(format!(
                    "{shown}:1: `{label}` has no `given` or `expect` — fix: `{{ given: [..] }}` \
                     or `{{ expect, params?, bind? }}`"
                ));
                return;
            }
            continue;
        };
        let Some(run) = at
            .check(
                &Value::Object(run_params),
                expect,
                &binds,
                &format!("{label}: "),
                report,
            )
            .await
        else {
            return;
        };
        if !bind_from(step, &run_json(&run), &mut binds, shown, &label, report) {
            return;
        }
    }
}

/// Keep what a step's `bind: { <name>: <path> }` reads in `seen` (a run,
/// or what a press answered). False, with the finding, when a path reads
/// nothing.
fn bind_from(
    step: &Value,
    seen: &Value,
    binds: &mut Binds,
    shown: &str,
    label: &str,
    report: &mut TestReport,
) -> bool {
    for (name, path) in step
        .get("bind")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        match path
            .as_str()
            .and_then(|path| read_path(seen, &format!(".{path}")))
        {
            Some(value) => {
                binds.0.insert(name.clone(), value);
            }
            None => {
                report.errors.push(format!(
                    "{shown}:1: {label}: `bind.{name}` reads nothing in {seen} — fix: a path \
                     like `loaders.<name>.rows[0].<column>` (a run) or `result.<key>` (a press)"
                ));
                return false;
            }
        }
    }
    true
}

/// Run advisory `id` for a thread (`when: { advisory, thread? }`, the
/// primary thread when none) as the app does on its `on:` — over that
/// thread's open effort — and check what the agent is told, line by line:
/// `expect: { lines: [..] }` (none: `[]`).
async fn advisory_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let Some(advisory) = ext.advisories.iter().find(|a| a.id == id) else {
        return report.errors.push(format!(
            "{shown}:1: `{id}` isn't one of the extension's advisories — fix: the example's \
             `when.advisory`"
        ));
    };
    let thread = match ex.when.get("thread").and_then(Value::as_str) {
        None => oxplow_domain::ThreadId::new(1),
        Some(t) => match oxplow_domain::ThreadId::try_from_str(t) {
            Some(thread) => thread,
            None => {
                return report.errors.push(format!(
                    "{shown}:1: `when.thread: {t}` isn't a thread — fix: `thr1`"
                ))
            }
        },
    };
    if let Err(e) = host.svc.streams.ensure_primary().await {
        return report.errors.push(format!("{shown}:1: its thread: {e}"));
    }
    host.current().await;
    let hits =
        oxplow_app::advisories::for_thread(&host.svc.advisory_deps(), &thread, advisory.on, None)
            .await;
    let lines: Vec<String> = hits
        .iter()
        .filter(|hit| hit.id == id || hit.id.ends_with(&format!("/{id}")))
        .flat_map(|hit| hit.text.lines().map(str::to_string).collect::<Vec<_>>())
        .collect();
    let got = json!({ "lines": lines });
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(format!(
            "{shown}:1: advisory `{id}` said {at} at `{path}`, the example expects {want} — fix: \
             its query, or the example's `given` and `expect` (`{{ lines: [..] }}`)"
        ));
    }
}

/// Read the published datasets with `when: { sql, params? }` as a page's
/// loader would — a dataset, a decorator's labels, a metric's grid — through
/// an inline page of one `sql` loader: `expect: { rows, columns? }`,
/// compared as a loader's.
async fn sql_example(host: &Host, ex: &Example<'_>, sql: &str, report: &mut TestReport) {
    let shown = ex.shown;
    let params = ex.when.get("params").cloned().unwrap_or_else(|| json!({}));
    let declared: Vec<Value> = params
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| json!({ "name": name }))
        .collect();
    let spec = json!({
        "title": "Query",
        "params": declared,
        "loaders": { "rows": { "sql": sql } },
        "view": { "viz": "table", "from": "rows" },
    });
    let page = serde_yaml::to_string(&spec)
        .map_err(|e| e.to_string())
        .and_then(|text| oxplow_app::pages::schema::parse_inline_page(&text));
    let page = match page {
        Ok(page) => page,
        Err(e) => return report.errors.push(format!("{shown}:1: the query: {e}")),
    };
    let Some(ctx) = view_context(ex, report) else {
        return;
    };
    let at = PageAt {
        host,
        shown,
        page: "query",
        target: oxplow_app::pages::runner::RunTarget::Spec(Arc::new(page)),
        ctx: &ctx,
    };
    let expect = json!({ "loaders": { "rows": ex.expect } });
    at.check(&params, &expect, &Binds::default(), "", report)
        .await;
}

/// Where a page example runs: its host, fixture, page and view.
struct PageAt<'a> {
    host: &'a Host,
    shown: &'a str,
    /// The page as the fixture names it.
    page: &'a str,
    /// What runs: the page (`<extension>/<slug>`), or an inline one.
    target: oxplow_app::pages::runner::RunTarget,
    ctx: &'a oxplow_app::pages::spec::ViewContext,
}

impl PageAt<'_> {
    /// Act on a run of the page with `params`, as a person would: the
    /// step's `press` (`{ action, node?, row?, input?, confirmed? }`, one of
    /// its actions) or `submit` (`{ node, input, confirmed? }`, a form
    /// node), read through `binds`. What it gave is `{ ok: true, result }`,
    /// or `{ ok: false, refused }` when it said no — checked against the
    /// step's `expect` when it has one; without, one that says no is a
    /// finding. `None` when the step can't go on.
    async fn act(
        &self,
        params: &Value,
        step: &Value,
        binds: &Binds,
        label: &str,
        report: &mut TestReport,
    ) -> Option<Value> {
        let (shown, page) = (self.shown, self.page);
        let submit = step.get("submit").is_some();
        let kind = if submit { "submit" } else { "press" };
        let (params, what, expect) = match (
            binds.substitute(params),
            binds.substitute(&step[kind]),
            step.get("expect").map(|e| binds.substitute(e)).transpose(),
        ) {
            (Ok(p), Ok(what), Ok(e)) => (p, what, e),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                report.errors.push(format!("{shown}:1: {label}: {e}"));
                return None;
            }
        };
        let Some(action) = what
            .get(if submit { "node" } else { "action" })
            .and_then(Value::as_str)
        else {
            report.errors.push(format!(
                "{shown}:1: {label}: `press` is `{{ action, node?, row?, input?, confirmed? }}`, \
                 `submit` is `{{ node, input, confirmed? }}`"
            ));
            return None;
        };
        let cells = |v: &Value| -> std::collections::BTreeMap<String, oxplow_db::SqlCell> {
            v.as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        };
        let runs = oxplow_app::pages::open_runs::PageRuns::default();
        let opened = runs
            .open(
                &self.host.svc,
                oxplow_app::pages::runner::RunRequest {
                    target: self.target.clone(),
                    params: cells(&params),
                    ctx: self.ctx.clone(),
                    viewer: Actor::Human,
                },
                "fixture",
            )
            .await;
        let run_id = match opened {
            Ok((run_id, _)) => run_id,
            Err(e) => {
                report
                    .errors
                    .push(format!("{shown}:1: {label}: page `{page}`: {e}"));
                return None;
            }
        };
        let confirmed = what
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let input = what.get("input").cloned();
        let outcome = if submit {
            oxplow_app::pages::actions::submit_page_form(
                &self.host.svc,
                &runs,
                &run_id,
                action,
                input.unwrap_or_else(|| json!({})),
                Actor::Human,
                confirmed,
            )
            .await
        } else {
            oxplow_app::pages::actions::run_page_action(
                &self.host.svc,
                &runs,
                oxplow_app::pages::actions::PageActionCall {
                    run_id,
                    node: what.get("node").and_then(Value::as_str).map(str::to_string),
                    action: action.to_string(),
                    row: what.get("row").map(cells),
                    input,
                    drop: None,
                    on_behalf_of: Actor::Human,
                    confirmed,
                },
            )
            .await
        };
        self.host.settle().await;
        let got = match outcome {
            Ok(out) => json!({ "ok": true, "result": out.result }),
            Err(e) => json!({ "ok": false, "refused": e.to_string() }),
        };
        match expect {
            Some(expect) => {
                if let Some((path, want, at)) = compare(&expect, &got) {
                    report.errors.push(format!(
                        "{shown}:1: {label}: {kind} `{action}` on page `{page}` gave {at} at \
                         `{path}`, the example expects {want} — fix: the action, or the step's \
                         `expect` (`{{ ok, result? }}` or `{{ ok: false, refused }}`)"
                    ));
                    return None;
                }
            }
            None if got["ok"] == false => {
                report.errors.push(format!(
                    "{shown}:1: {label}: {kind} `{action}` on page `{page}` said no: {} — fix: \
                     the action, or `expect: {{ ok: false, refused }}` on the step",
                    got["refused"]
                ));
                return None;
            }
            None => {}
        }
        Some(got)
    }

    /// Run the page with `params` and check it against `expect`, both
    /// read through `binds`; findings start with `label`. The run, unless
    /// it couldn't run.
    async fn check(
        &self,
        params: &Value,
        expect: &Value,
        binds: &Binds,
        label: &str,
        report: &mut TestReport,
    ) -> Option<oxplow_app::pages::runner::PageRun> {
        let (shown, page) = (self.shown, self.page);
        let (params, expect) = match (binds.substitute(params), binds.substitute(expect)) {
            (Ok(params), Ok(expect)) => (params, expect),
            (Err(e), _) | (_, Err(e)) => {
                report.errors.push(format!("{shown}:1: {label}{e}"));
                return None;
            }
        };
        let run = match run_page(self.host, self.target.clone(), &params, self.ctx).await {
            Ok(run) => run,
            Err(e) => {
                report.errors.push(format!(
                    "{shown}:1: {label}page `{page}`: {e} — fix: the page, or the example's \
                     `params` and `given`"
                ));
                return None;
            }
        };
        if let Some(loaders) = expect.get("loaders").and_then(Value::as_object) {
            for (name, want) in loaders {
                let Some(loader) = loader_at(&run, name) else {
                    report.errors.push(format!(
                        "{shown}:1: {label}page `{page}` has no loader `{name}` — fix: the \
                         example's `expect.loaders`"
                    ));
                    continue;
                };
                let rows = &loader.result.rows;
                // Each row as its columns by name — or its cells, where the
                // example lists that row as a list; a count where it gives one.
                let named = |row: &Vec<oxplow_db::SqlCell>| -> Value {
                    Value::Object(
                        loader
                            .result
                            .columns
                            .iter()
                            .cloned()
                            .zip(row.iter().map(|c| json!(c)))
                            .collect(),
                    )
                };
                let listed = want.get("rows").and_then(Value::as_array);
                let got_rows = match want.get("rows") {
                    Some(Value::Number(_)) => json!(rows.len()),
                    _ => Value::Array(
                        rows.iter()
                            .enumerate()
                            .map(|(i, row)| match listed.and_then(|l| l.get(i)) {
                                Some(Value::Array(_)) => json!(row),
                                _ => named(row),
                            })
                            .collect(),
                    ),
                };
                let mut got = json!({ "rows": got_rows });
                if want.get("columns").is_some() {
                    got["columns"] = json!(loader.result.columns);
                }
                if let Some((path, want, got)) = compare(want, &got) {
                    report.errors.push(format!(
                        "{shown}:1: {label}page `{page}` loader `{name}` returned {got} at \
                         `{path}`, the example expects {want} — fix: the page, or the example's \
                         `expect.loaders.{name}` (`{{ rows: n | $any | [..] | {{ contains: [..] }}, \
                         columns? }}`)"
                    ));
                }
            }
        }
        // The run's own parts, each compared on what the example names.
        let parts = [
            ("alert", json!(run.alert)),
            ("badge", json!(run.badge)),
            ("inactive", json!(run.inactive)),
            ("actions", Value::Null),
        ];
        for (key, got) in parts {
            let Some(want) = expect.get(key) else {
                continue;
            };
            let got = match key {
                "actions" => actions_like(want, &run.page.actions),
                _ => got,
            };
            if let Some((path, want, got)) = compare(want, &got) {
                report.errors.push(format!(
                    "{shown}:1: {label}page `{page}`'s {key} is {got} at `{key}{path}`, the \
                     example expects {want} — fix: the page, or `expect.{key}`"
                ));
            }
        }
        if let Some(want) = expect.get("text") {
            match oxplow_app::pages::text::page_text(&self.host.svc, &self.host.root, &run).await {
                Err(e) => report.errors.push(format!(
                    "{shown}:1: {label}page `{page}` has no text: {e} — fix: the page"
                )),
                Ok(text) => {
                    let missing: Vec<&str> = want
                        .get("contains")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|needle| !text.contains(*needle))
                        .collect();
                    if matches!(want, Value::String(exact) if *exact != text) {
                        report.errors.push(format!(
                            "{shown}:1: {label}page `{page}`'s text is {text:?}, the example \
                             expects {want} — fix: the page, or `expect.text` (`{{ contains: [..] \
                             }}` or the exact text)"
                        ));
                    }
                    for needle in missing {
                        report.errors.push(format!(
                            "{shown}:1: {label}page `{page}`'s text doesn't contain {needle:?} — \
                             fix: the page, or `expect.text.contains`; the text is:\n{text}"
                        ));
                    }
                }
            }
        }
        if let Some(nodes) = expect.get("nodes").and_then(Value::as_array) {
            let present = run_nodes(&run);
            for want in nodes {
                let Some(id) = want.get("id").and_then(Value::as_str) else {
                    report.errors.push(format!(
                        "{shown}:1: {label}`expect.nodes` entries are `{{ id, kind?, actions? }}`"
                    ));
                    continue;
                };
                let Some((_, kind)) = present.iter().find(|(have, _)| have == id) else {
                    report.errors.push(format!(
                        "{shown}:1: {label}page `{page}` has no node `{id}` in this run (it has \
                         {}) — fix: the example, or the page's view",
                        present
                            .iter()
                            .map(|(n, _)| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    continue;
                };
                if let Some(k) = want.get("kind").and_then(Value::as_str) {
                    if k != kind {
                        report.errors.push(format!(
                            "{shown}:1: {label}page `{page}` node `{id}` is a `{kind}`, the \
                             example expects `{k}`"
                        ));
                    }
                }
                if let Some(want) = want.get("actions") {
                    let got = actions_like(want, &node_actions(&run, id));
                    if let Some((path, want, got)) = compare(want, &got) {
                        report.errors.push(format!(
                            "{shown}:1: {label}page `{page}` node `{id}`'s actions are {got} at \
                             `{path}`, the example expects {want} — fix: the page, or the node's \
                             `actions` in `expect.nodes`"
                        ));
                    }
                }
            }
        }
        Some(run)
    }
}

/// `actions` as an example names them: an id where it names an id, else
/// the action's fields (spelled as the page YAML spells them) that the
/// example names.
fn actions_like(want: &Value, actions: &[oxplow_app::pages::spec::PageAction]) -> Value {
    let named = want.as_array();
    Value::Array(
        actions
            .iter()
            .enumerate()
            .map(|(i, action)| match named.and_then(|w| w.get(i)) {
                Some(Value::Object(_)) => {
                    let fields = match json!(action) {
                        Value::Object(fields) => fields
                            .into_iter()
                            .map(|(k, v)| (snake_case(&k), v))
                            .collect(),
                        _ => serde_json::Map::new(),
                    };
                    Value::Object(fields)
                }
                _ => json!(action.id),
            })
            .collect(),
    )
}

/// `oneRow` → `one_row`.
fn snake_case(name: &str) -> String {
    name.chars()
        .flat_map(|c| {
            let lower = c.to_ascii_lowercase();
            (c.is_ascii_uppercase().then_some('_').into_iter()).chain([lower])
        })
        .collect()
}

/// The actions node `id` of a run's page offers (a viz's), in the order
/// it declares them.
fn node_actions(
    run: &oxplow_app::pages::runner::PageRun,
    id: &str,
) -> Vec<oxplow_app::pages::spec::PageAction> {
    let local = id.rsplit('/').next().unwrap_or(id);
    run.page
        .nodes()
        .into_iter()
        .find(|n| n.id == local)
        .and_then(|n| match &n.kind {
            oxplow_app::pages::schema::NodeKind::Viz { config, .. } => Some(config.actions.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// What a `ui` step list has changed since the page last ran: data it
/// changed, and the view params the frame set (merged; `null` drops one),
/// which the app's tab would make the page's params.
#[derive(Default)]
struct UiState {
    /// The embeds the component's page is inside (`component_at`).
    embed: Vec<String>,
    changed: bool,
    view: serde_json::Map<String, Value>,
    /// How many of the harness's records `view` has read.
    seen: usize,
}

impl UiState {
    fn merge_view(&mut self, change: &Value) {
        for (k, v) in change.as_object().into_iter().flatten() {
            let before = self.view.get(k).cloned();
            if v.is_null() {
                self.view.remove(k);
            } else {
                self.view.insert(k.clone(), v.clone());
            }
            if self.view.get(k) != before.as_ref() {
                self.changed = true;
            }
        }
    }

    /// The step's params with the view params over them.
    fn params(&self, step: &Value) -> Value {
        let mut merged = step.as_object().cloned().unwrap_or_default();
        for (k, v) in &self.view {
            merged.insert(k.clone(), v.clone());
        }
        Value::Object(merged)
    }
}

impl PageAt<'_> {
    /// Run a step's `ui:` list against the page's custom component — its
    /// `node:`, else the first component node — in the app's own webview
    /// (`component_harness`), on this oxplow. False when the scenario stops
    /// here: it found anything, or the window draws nothing.
    async fn ui(
        &self,
        ext: &Extension,
        params: &Value,
        (step, ui): (&Value, &Value),
        binds: &mut Binds,
        label: &str,
        report: &mut TestReport,
    ) -> bool {
        use crate::component_harness::{shell, Harness};
        let (shown, page) = (self.shown, self.page);
        let fail = |report: &mut TestReport, why: String| {
            report.errors.push(format!("{shown}:1: {label}: {why}"));
            false
        };
        let Some(shell) = shell() else {
            report.warnings.push(format!(
                "{shown}:1: {label}: its `ui` steps, and the steps after them, aren't run: no \
                 oxplow app here to draw them — fix: install the Oxplow app, or set \
                 OXPLOW_SHELL to it"
            ));
            return false;
        };
        let Some(steps) = ui.as_array() else {
            return fail(report, "`ui` is a list of steps".into());
        };
        let params = match binds.substitute(params) {
            Ok(params) => params,
            Err(e) => return fail(report, e),
        };
        let run = match run_page(self.host, self.target.clone(), &params, self.ctx).await {
            Ok(run) => run,
            Err(e) => return fail(report, format!("page `{page}`: {e}")),
        };
        // The component the step drives, by its node — in the page, or in
        // a page it embeds (a replacement, a mount).
        let named = step.get("node").and_then(Value::as_str);
        let Some((embed, run, node, component)) = component_at(&run, named) else {
            return fail(
                report,
                format!(
                    "page `{page}` has no component node to drive{}",
                    named.map(|n| format!(" named `{n}`")).unwrap_or_default()
                ),
            );
        };
        let owner = run
            .page
            .id
            .split('/')
            .next()
            .unwrap_or(&ext.name)
            .to_string();
        let key = if component.contains('/') {
            component
        } else {
            format!("{owner}/{component}")
        };
        let version =
            match oxplow_app::viz_actions::load_component(&self.host.svc, &self.host.root, &key) {
                Ok(version) => version,
                Err(e) => return fail(report, format!("component `{key}` doesn't load: {e}")),
            };
        // A component that is the whole page is told it is one, and has a view.
        let whole = run.page.view.id == node;
        let start = json!({
            "version": version,
            "run": run,
            "node": node,
            "page": whole.then(|| json!({ "streamId": null, "threadId": null })),
            "view": whole.then(|| json!({ "viewParams": {}, "historyState": {} })),
        });
        let harness = match Harness::open(
            &shell,
            self.host.svc.clone(),
            self.host.root.clone(),
            key.clone(),
            version,
            start,
        )
        .await
        {
            Ok(harness) => harness,
            Err(e) => return fail(report, e),
        };
        // A window that draws no frames (the display locked or asleep)
        // can't lay a component out: its steps aren't run, never failed, nor
        // are the steps after them, which read what they did.
        match harness.drawing(std::time::Duration::from_secs(10)).await {
            Some(true) => {}
            Some(false) => {
                harness.close().await;
                report.warnings.push(format!(
                    "{shown}:1: {label}: its `ui` steps, and the steps after them, aren't run: \
                     the oxplow app's window \
                     draws nothing while the display is locked or asleep — fix: run it \
                     again with the display awake"
                ));
                return false;
            }
            None => {
                harness.close().await;
                return fail(report, "the harness's page never started".into());
            }
        }
        let ok = self
            .drive(
                &harness,
                ext,
                (&params, steps, &embed),
                binds,
                label,
                report,
            )
            .await;
        harness.close().await;
        ok
    }

    /// The `ui` steps on an open harness.
    async fn drive(
        &self,
        harness: &crate::component_harness::Harness,
        ext: &Extension,
        (params, steps, embed): (&Value, &[Value], &[String]),
        binds: &mut Binds,
        label: &str,
        report: &mut TestReport,
    ) -> bool {
        let shown = self.shown;
        let wait = std::time::Duration::from_secs(10);
        // It starts: the frame says `ready`.
        let started = std::time::Instant::now();
        while !harness.records().iter().any(|(kind, _)| kind == "ready") {
            if started.elapsed() > std::time::Duration::from_secs(30) {
                let page = harness
                    .op(json!({ "op": "dom", "action": { "page": true } }), wait)
                    .await
                    .unwrap_or_default();
                report.errors.push(format!(
                    "{shown}:1: {label}: the component didn't start (it never connected) — the \
                     frame: {}",
                    page["value"]
                ));
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let mut state = UiState {
            embed: embed.to_vec(),
            ..UiState::default()
        };
        for (index, item) in steps.iter().enumerate() {
            let at = format!("{label}: ui[{index}]");
            let item = match binds.substitute(item) {
                Ok(item) => item,
                Err(e) => {
                    report.errors.push(format!("{shown}:1: {at}: {e}"));
                    return false;
                }
            };
            let dom = ["click", "fill", "key", "drag"]
                .into_iter()
                .any(|k| item.get(k).is_some());
            let done = if dom {
                match harness
                    .op(json!({ "op": "dom", "action": item }), wait)
                    .await
                {
                    Ok(out) if out["ok"] == true => Ok(()),
                    Ok(out) => Err(out["error"].as_str().unwrap_or("it failed").to_string()),
                    Err(e) => Err(e),
                }
            } else if let Some(id) = item.get("press").and_then(Value::as_str) {
                harness
                    .op(json!({ "op": "press", "id": id }), wait)
                    .await
                    .map(|_| ())
            } else if let Some(view) = item.get("view_params") {
                state.merge_view(view);
                harness
                    .op(json!({ "op": "viewParams", "params": view }), wait)
                    .await
                    .map(|_| ())
            } else if let Some(given) = item.get("given") {
                state.changed = true;
                if run_steps(
                    self.host,
                    ext,
                    shown,
                    &format!("{at}.given"),
                    given,
                    binds,
                    report,
                )
                .await
                {
                    Ok(())
                } else {
                    return false;
                }
            } else if let Some(yes) = item.get("confirm").and_then(Value::as_bool) {
                harness.confirm_next(yes);
                Ok(())
            } else if let Some(form) = item.get("form") {
                harness.submit_next(form.get("submit").cloned());
                Ok(())
            } else if let Some(want) = item.get("expect") {
                self.expect_ui(harness, params, want, &mut state).await
            } else {
                Err(
                    "a `ui` step is `click`, `fill`, `key`, `drag`, `press`, `view_params`, \
                     `given`, `confirm`, `form` or `expect`"
                        .into(),
                )
            };
            if let Err(why) = done {
                let page = harness
                    .op(json!({ "op": "dom", "action": { "page": true } }), wait)
                    .await
                    .unwrap_or_default();
                report.errors.push(format!(
                    "{shown}:1: {at}: {why} — the frame: {} {}",
                    page["value"]["html"].as_str().unwrap_or("(unreadable)"),
                    match page["value"]["errors"].as_array() {
                        Some(errors) if !errors.is_empty() =>
                            format!("— its errors: {}", json!(errors)),
                        _ => String::new(),
                    }
                ));
                return false;
            }
        }
        true
    }

    /// Wait (up to 5 s) for what the frame draws and asked the shell to be
    /// what `want` says; the page runs again and the frame hears its rows
    /// whenever a command ran or a step changed data.
    async fn expect_ui(
        &self,
        harness: &crate::component_harness::Harness,
        params: &Value,
        want: &Value,
        state: &mut UiState,
    ) -> Result<(), String> {
        let wait = std::time::Duration::from_secs(10);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // The view params the frame set are the page's next run's.
            let records = harness.records();
            for (kind, change) in records.iter().skip(state.seen) {
                if kind == "viewParams" {
                    state.merge_view(change);
                }
            }
            state.seen = records.len();
            if harness.take_acted() || std::mem::take(&mut state.changed) {
                self.host.settle().await;
                let run = run_page(
                    self.host,
                    self.target.clone(),
                    &state.params(params),
                    self.ctx,
                )
                .await?;
                let run = embedded_at(&run, &state.embed)
                    .ok_or("the page no longer embeds the component's page")?;
                harness
                    .op(json!({ "op": "update", "run": run }), wait)
                    .await?;
            }
            let (wanted, got) = self.ui_seen(harness, want, wait).await?;
            match compare(&wanted, &got) {
                None => return Ok(()),
                Some(_) if std::time::Instant::now() < until => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Some((path, want, at)) => {
                    return Err(format!(
                        "{at} at `{path}`, the example expects {want} — fix: the component, or \
                         the step's `expect` (`{{ visible, count, text, title, actions, \
                         view_params, invoked, navigated, confirmed, forms, shown }}`)"
                    ))
                }
            }
        }
    }

    /// What `want` names, as the frame draws it and as it asked the shell:
    /// `want` as compared (`visible: sel` as `{ sel: true }`, a text's
    /// `{ contains }` kept when it holds) and what was seen.
    async fn ui_seen(
        &self,
        harness: &crate::component_harness::Harness,
        want: &Value,
        wait: std::time::Duration,
    ) -> Result<(Value, Value), String> {
        let query = |selector: String| {
            let op = json!({ "op": "dom", "action": { "query": selector } });
            async move {
                let out = harness.op(op, wait).await?;
                Ok::<_, String>((
                    out["value"]["count"].as_u64().unwrap_or(0),
                    out["value"]["texts"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                ))
            }
        };
        // A text `{ contains }` holds when the text has it.
        let text = |want: &Value, got: Option<&str>| match (want.get("contains"), got) {
            (Some(Value::String(part)), Some(got)) if got.contains(part.as_str()) => want.clone(),
            (_, got) => json!(got),
        };
        let records = harness.records();
        let of = |kind: &str| -> Vec<Value> {
            records
                .iter()
                .filter(|(k, _)| k == kind)
                .map(|(_, v)| v.clone())
                .collect()
        };
        let (mut wanted, mut got) = (serde_json::Map::new(), serde_json::Map::new());
        for (key, w) in want.as_object().into_iter().flatten() {
            let seen = match key.as_str() {
                "visible" => {
                    let selectors: Vec<String> = match w {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(list) => list
                            .iter()
                            .filter_map(|s| s.as_str().map(str::to_string))
                            .collect(),
                        _ => return Err("`visible` is a selector or a list of them".into()),
                    };
                    let mut shown = serde_json::Map::new();
                    for selector in &selectors {
                        shown.insert(
                            selector.clone(),
                            json!(query(selector.clone()).await?.0 > 0),
                        );
                    }
                    wanted.insert(
                        key.clone(),
                        json!(selectors
                            .iter()
                            .map(|s| (s.clone(), json!(true)))
                            .collect::<serde_json::Map<_, _>>()),
                    );
                    Value::Object(shown)
                }
                "count" | "text" => {
                    let mut seen = serde_json::Map::new();
                    for (selector, w) in w.as_object().into_iter().flatten() {
                        let (count, texts) = query(selector.clone()).await?;
                        seen.insert(
                            selector.clone(),
                            if key == "count" {
                                json!(count)
                            } else {
                                text(w, texts.first().and_then(Value::as_str))
                            },
                        );
                    }
                    Value::Object(seen)
                }
                "title" => text(w, of("title").last().and_then(Value::as_str)),
                "actions" => json!(of("actions")
                    .last()
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|a| a["id"].clone()).collect::<Vec<_>>())
                    .unwrap_or_default()),
                "view_params" => {
                    let mut view = serde_json::Map::new();
                    for change in of("viewParams") {
                        for (k, v) in change.as_object().into_iter().flatten() {
                            if v.is_null() {
                                view.remove(k);
                            } else {
                                view.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    Value::Object(view)
                }
                "invoked" => json!(of("invoked")),
                "navigated" => json!(of("navigate")),
                "confirmed" => json!(of("confirm")),
                "forms" => json!(of("openForm")
                    .into_iter()
                    .chain(of("askForm"))
                    .collect::<Vec<_>>()),
                "shown" => of("showView").last().cloned().unwrap_or(Value::Null),
                other => {
                    return Err(format!(
                        "`expect.{other}` isn't something a `ui` step reads"
                    ))
                }
            };
            if !wanted.contains_key(key) {
                wanted.insert(key.clone(), w.clone());
            }
            got.insert(key.clone(), seen);
        }
        Ok((Value::Object(wanted), Value::Object(got)))
    }
}

/// Every page's `answers:` entry runs with its params: the page opens and
/// none of its loaders fails. A test case per page.
async fn page_answers(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let mut any = false;
    // An answer may name the primary stream's thread.
    if let Err(e) = host.svc.streams.ensure_primary().await {
        report
            .errors
            .push(format!("{rel}: the primary thread: {e}"));
        return;
    }
    let ctx = oxplow_app::pages::spec::ViewContext::default();
    for page in &ext.page_files {
        if page.answers.is_empty() {
            continue;
        }
        any = true;
        let (before, started) = (report.errors.len(), std::time::Instant::now());
        for answer in &page.answers {
            let params = json!(answer.params);
            if let Err(e) = run_page(
                host,
                oxplow_app::pages::runner::RunTarget::Page(page.id.clone()),
                &params,
                &ctx,
            )
            .await
            {
                report.errors.push(format!(
                    "{rel}/pages/{}.yaml:1: the answer to \"{}\" doesn't run: {e} — fix: the \
                     answer's `params`, or the page",
                    page.slug, answer.question
                ));
            }
        }
        report.finish_case(format!("answers {}", page.slug), before, started);
    }
    if any {
        report.ran.push("answers".into());
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
    match ex.when.get("answers").cloned().map(serde_json::from_value) {
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
/// `{ refused: <part of the reason> }`. Nothing runs.
async fn command_example(
    host: &Host,
    ex: &Example<'_>,
    cmd: &oxplow_app::extension::extension_commands::ExtensionCommand,
    report: &mut TestReport,
) {
    use oxplow_app::extension::extension_commands::{call_names, dry_run, Composed};
    let shown = ex.shown;
    let Some(answers) = answers(ex, &format!("command `{}`", cmd.name), report) else {
        return;
    };
    let input = ex.when.get("input").cloned().unwrap_or_else(|| json!({}));
    let decided = dry_run(
        &host.svc.sql,
        cmd,
        &input,
        &answers,
        host.svc.commands.as_ref(),
    )
    .await;
    let problem = match (decided, ex.expect.get("refused").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Composed::Refused(why)), Some(want)) if why.contains(want) => None,
        (Ok(Composed::Refused(why)), _) => Some(format!("refused ({why})")),
        (Ok(Composed::Run { calls, .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to refuse ({want})",
            call_names(&calls).join(", ")
        )),
        (Ok(Composed::Run { calls, result, .. }), None) => {
            let got = json!({ "commands": call_names(&calls), "result": result });
            compare(&ex.expect, &got).map(|(path, want, got)| {
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
             (`{{ commands: [names], result: {{ … }} }}` or `{{ refused: <part of the reason> }}`)",
            cmd.name
        ));
    }
}

/// Dry-run effect `id` on the fixture's event (`when: { effect, event: {
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
        (Ok(r @ Reaction::Run { .. }), None) => {
            compare(expect, &json!({ "commands": r.command_names() })).map(|(path, want, got)| {
                format!("composed {got} at `{path}`, the example expects {want}")
            })
        }
    }
}

/// The example's `event:` as the event envelope the runtime would hand
/// over (`EventView::fixture`, through the throwaway's vocabulary); one
/// that couldn't happen is a finding at the fixture.
fn example_event(host: &Host, ex: &Example<'_>, report: &mut TestReport) -> Option<EventView> {
    let given = ex.when.get("event").cloned().unwrap_or_else(|| json!({}));
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
/// event (`when: { implementation, event: { type, payload, subject?,
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
        Reaction::Run { calls, .. } => oxplow_app::extension::extension_commands::check_calls(
            host.svc.commands.as_ref(),
            calls,
        )
        .map(|()| reaction),
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
/// (`when: { event_type, v?, payload }`, the newest version when `v` is
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
        .when
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
    let payload = ex.when.get("payload").cloned().unwrap_or_else(|| json!({}));
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
                if let Some((path, want, got)) = compare(want, &got) {
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

/// What a `[[…]]` names with the extension's resources beside
/// oxplow-foundation's (`when: { wikilink: "pr:12" }`):
/// `expect: { uri: "oxplow:acme/pr?id=12" | null }`.
fn wikilink_example(ext: &Extension, ex: &Example<'_>, link: &str, report: &mut TestReport) {
    let mut resources = oxplow_domain::uri::resources::foundation().clone();
    for r in &ext.resources {
        let _ = resources.register(r.decl.clone());
    }
    let got = oxplow_domain::uri::canonical_wikilink(&resources, link).map(|u| u.to_string());
    if let Some((path, want, got)) = compare(&ex.expect, &json!({ "uri": got })) {
        report.errors.push(format!(
            "{}:1: `[[{link}]]` names {got} at `{path}`, the example expects {want} — fix: the \
             resource's `params`/`wikilink`, or the example's `expect`",
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
    let (shown, id) = (ex.shown, spec.id.as_str());
    let files: std::collections::HashMap<String, String> = ex
        .when
        .get("files")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let mut input: serde_json::Map<String, Value> = ex
        .when
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
    if let Some((path, want, got)) = compare(&ex.expect, &json!({ "facts": got })) {
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
    {
        use oxplow_config::collectors::CollectorRuntime as R;
        let does = match spec.runtime {
            R::Read => Some("reads a provider"),
            R::Exec if spec.bundled_parser().is_none() => Some("runs a program"),
            _ => None,
        };
        if let Some(does) = does {
            report.warnings.push(format!(
                "{shown}:1: example `{example}` runs collector `{id}`, which {does}: `extension \
                 test` doesn't run programs or providers — fix: none; run it with \
                 `oxplow.collector.sync` once it's deployed"
            ));
            return;
        }
    }
    if !spec.facts.is_empty() {
        return fact_collector_example(host, ext, ex, spec, report).await;
    }
    let rows = ex
        .when
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| r.to_vec());
    let preview = oxplow_app::collector_runner::preview_collector(
        &oxplow_app::collector_runner::Collectors::of(&host.svc, &host.root),
        &ext.name,
        id,
        ex.when.get("event"),
        rows,
    )
    .await;
    let preview = match preview {
        Ok(p) => p,
        Err(e) => {
            use oxplow_app::collector_runner::RunCollectorError as E;
            let why = match e {
                E::NotFound => "it isn't loaded".to_string(),
                E::Denied(m) | E::Failed(m) | E::Disabled(m) => m,
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
    if let Some((path, want, got)) = compare(expect, &got) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ entities: {{ <name>: n | $any }}, \
             events?: [{{ type, payload? }}] }}`)"
        ));
    }
}

/// Each incremental dataset (P8.B5), on its fixture `fixtures/dataset-<name>.yaml`
/// (`before:` and `after:`, each `{ <table>: [rows] }`): built whole on
/// `before`, appended to past its watermark after `after` is written, and
/// compared with a full refill — in a rehearsal, so nothing stays. A
/// watermark that doesn't grow as rows arrive drops a row below it
/// unseen; nothing at run time can tell, so this is where it shows.
async fn incremental_datasets(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap_or_default();
    for dataset in &ext.datasets {
        let decl = &dataset.decl;
        if decl
            .materialize
            .as_ref()
            .and_then(oxplow_db::datasets::Materialize::incremental)
            .is_none()
        {
            continue;
        }
        let line = oxplow_app::extension::extensions::manifest_v2::entry_line(
            &manifest, "datasets", "name", &decl.name,
        )
        .unwrap_or(1);
        let at = format!("{rel}/extension.yaml:{line}");
        let fixture = format!("fixtures/dataset-{}.yaml", decl.name);
        let Ok(text) = std::fs::read_to_string(dir.join(&fixture)) else {
            report.warnings.push(format!(
                "{at}: incremental dataset `{}` has no {rel}/{fixture}, so appending isn't checked \
                 against a full refill — fix: write its `before:` and `after:` rows \
                 (`{{ <table>: [{{ column: value }}] }}`)",
                decl.name
            ));
            continue;
        };
        let rows = |doc: &Value, part: &str| -> Result<oxplow_db::datasets::FixtureRows, String> {
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
        let view = oxplow_db::datasets::extension_view(&ext.name, &decl.name);
        let decl_c = decl.clone();
        let checked = host
            .svc
            .db
            .rehearse(move |tx| {
                oxplow_db::datasets::incremental_matches_full(tx, &view, &decl_c, &before, &after)
            })
            .await;
        match checked {
            Ok(None) => {}
            Ok(Some(problem)) => report.errors.push(format!(
                "{at}: incremental dataset `{}` {problem} — fix: `materialize: on_change`, or a \
                 watermark that only grows as rows arrive",
                decl.name
            )),
            Err(e) => report.errors.push(format!(
                "{rel}/{fixture}:1: incremental dataset `{}` couldn't run on the fixture: {e} — \
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
    let tools: std::collections::BTreeMap<String, Value> = oxplow_app::agent_tools::offered(
        &host.svc,
        &oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        },
    )
    .await
    .map(|tools| {
        tools
            .into_iter()
            .map(|t| (t.name, t.input_schema))
            .collect()
    })
    .unwrap_or_default();
    let tool_schema = |name: &str| tools.get(name).cloned();
    let checked = crate::answerability::check(
        &file,
        &questions,
        &crate::answerability::Checker {
            skill_text: &skill_text,
            sql: Some(&host.svc.sql),
            command_schema: &command_schema,
            tool_schema: &tool_schema,
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
    host: &Host,
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
            "{manifest}:1: provider `{}`: {v} — fix: send what the provider protocol's JSON schemas \
             describe (`initialize`, `discover`, `check`, …)",
            spec.id
        ));
    }
    transcript(&dir, &rel, spec, &recorded.transcript, bless, report);
    suite(host, ext, spec, config, env, report).await;
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
        let Some(command) = doc.pointer("/when/command").and_then(Value::as_str) else {
            continue;
        };
        let Some(input) = doc.pointer("/when/input") else {
            report.errors.push(format!(
                "{shown}:1: a provider example's `when` is `{{ command, input }}` — fix: give \
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
                let resource =
                    oxplow_domain::capability::contract(&spec.capability).and_then(|c| c.resource);
                if let Some(made) = out.result.get("uri").and_then(Value::as_str).filter(|u| {
                    resource
                        .is_some_and(|r| oxplow_domain::uri::build::parse_resource(u, r).is_ok())
                }) {
                    if !report.left.iter().any(|l| l == made) {
                        report.left.push(made.to_string());
                    }
                }
                if let Some((path, want, got)) = compare(&expect, &out.result) {
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
/// in-memory oxplow over a copy of the extension, the provider enabled
/// with the fixture config.
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

/// Publish the generation the throwaway project's config and files ask
/// for, as the app's owner of generations does on a change.
async fn publish_generation(svc: &Arc<oxplow_app::Services>) -> Result<(), String> {
    svc.extension_activation
        .activate(svc.clone())
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// An oxplow like `host` with a work list on, for a conformance suite that
/// files work: the first extension in the checkout (by name) that declares a
/// `work_items` implementation or provider. None is an error the suite
/// reports.
async fn with_work_list(host: &Host, ext: &Extension) -> Result<Host, String> {
    let mut lists: Vec<String> = oxplow_app::extension::extensions::load_extensions(&host.checkout)
        .into_iter()
        .filter(|e| {
            e.implementations
                .iter()
                .any(|d| d.capability == "work_items")
                || e.providers.iter().any(|p| p.capability == "work_items")
        })
        .map(|e| e.name)
        .collect();
    lists.sort();
    let Some(list) = lists.into_iter().next() else {
        return Err(
            "its suite files work, and no extension in the checkout declares a `work_items` \
             implementation or provider — fix: add one to the checkout"
                .into(),
        );
    };
    let started = Host::start(
        &host.checkout,
        &ext.name,
        &[list],
        host.env.clone(),
        host.builds,
    )
    .await?;
    started.start_runtime()?;
    Ok(started)
}

/// The effort-policy suite over the extension's policy script `decl`, on
/// a fork of `host`: the script chosen as the project's policy. A finding
/// is an error at the manifest.
async fn policy_suite(
    host: &Host,
    ext: &Extension,
    decl: &oxplow_app::extension::extensions::implementations::ImplementationDecl,
    report: &mut TestReport,
) {
    let manifest = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
    report.ran.push(format!("{EFFORT_POLICY} suite"));
    let run = async {
        // The suite files work, so it runs on an oxplow with a work list.
        let with_list = with_work_list(host, ext).await?;
        let fork = with_list.fork().await?;
        let svc = &fork.svc;
        svc.config
            .write()
            .map_err(|e| e.to_string())?
            .active_providers
            .insert(EFFORT_POLICY.into(), decl.id.clone());
        publish_generation(svc).await?;
        Ok::<_, String>(oxplow_app::effort_policy_conformance::suite(svc, &decl.id).await)
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
    host: &Host,
    ext: &Extension,
    spec: &ProviderSpec,
    config: Value,
    env: &host::HostEnv,
    report: &mut TestReport,
) {
    let manifest = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
    report.ran.push(format!("{} suite", spec.capability));
    let run = async {
        // On an oxplow of its own: what the suite files, no example sees. An
        // effort policy's suite files work, and oxplow's own work list is
        // bundled's: its oxplow has it on.
        let with_list;
        let base = if spec.capability == providers::effort_policy::EffortPolicyHost::CAPABILITY {
            with_list = with_work_list(host, ext).await?;
            &with_list
        } else {
            host
        };
        let fork = base.fork().await?;
        let svc = fork.svc.clone();
        let project = oxplow_app::collector_runner::project_key(&fork.root);
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
        crate::Format::Junit => junit(report),
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

/// `report` as one JUnit `<testsuite>` (no XML prolog, so a caller can
/// put several in one `<testsuites>`): a `<testcase>` per case, a
/// `<failure>` holding the errors it added.
fn junit(report: &TestReport) -> String {
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    let failures = report.cases.iter().filter(|c| !c.ok).count();
    let total_ms: u64 = report.cases.iter().map(|c| c.duration_ms).sum();
    let seconds = |ms: u64| format!("{:.3}", ms as f64 / 1000.0);
    let name = escape(&report.name);
    let mut out = format!(
        "<testsuite name=\"{name}\" tests=\"{}\" failures=\"{failures}\" time=\"{}\">\n",
        report.cases.len(),
        seconds(total_ms)
    );
    for case in &report.cases {
        out.push_str(&format!(
            "  <testcase name=\"{}\" classname=\"{name}\" time=\"{}\"",
            escape(&case.name),
            seconds(case.duration_ms)
        ));
        match &case.message {
            Some(message) => out.push_str(&format!(
                ">\n    <failure message=\"{}\">{}</failure>\n  </testcase>\n",
                escape(message.lines().next().unwrap_or_default()),
                escape(message)
            )),
            None => out.push_str("/>\n"),
        }
    }
    out.push_str("</testsuite>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Format;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// The tally extension: a starlark collector of `thing`s, a dataset and a
    /// page over them, a command noting one, a README and questions, and an
    /// intent example per kind, each with its fixture.
    fn tally(root: &Path, collector_expect: &str, command_expect: &str) {
        write(
            root,
            ".oxplow/extensions/tally/extension.yaml",
            &format!(
                "manifest: 2
name: tally
intent:
  purpose: Count things.
  examples:
    - {{ name: labelled, input: {{ page: labelled }}, expect: no labelled things yet }}
    - {{ name: things, input: {{ collector: things }}, expect: one thing per input row }}
    - {{ name: shell, input: {{ collector: shell }}, expect: whatever the shell says }}
permissions: [sql.read, ai.compute, process.run]
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT uri FROM v_work_item\"
    entities:
      - {{ name: thing, key: id, columns: {{ id: int, label: text }} }}
  - id: shell
    runtime: exec
    entry: collectors/shell.sh
    entities:
      - {{ name: line, key: n, columns: {{ n: int }} }}
datasets:
  - name: labelled
    version: 1
    description: Labelled things.
    fields:
      - {{ name: id, type: INTEGER, doc: The thing. }}
      - {{ name: label, type: TEXT, doc: Its label. }}
commands:
  - name: work.note
    summary: Note a work item.
    input_schema: {{ type: object, required: [uri], properties: {{ uri: {{ type: string }} }} }}
    entry: handlers/note.star
    examples:
      - {{ name: happy, input: {{ uri: \"work_item:oxplow:tsk1\" }}, expect_commands: {command_expect} }}
"
            ),
        );
        write(
            root,
            ".oxplow/extensions/tally/collectors/things.star",
            "def transform(x):\n    return {\"entities\": {\"thing\": [{\"id\": r[\"id\"], \"label\": \"t\"} for r in x[\"rows\"]]}}\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/collectors/shell.sh",
            "#!/bin/sh\necho '{}'\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/datasets/labelled.sql",
            "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.comment\", \"input\": {\"uri\": x[\"input\"][\"uri\"], \"body\": \"noted\"}}]}\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/pages/labelled.yaml",
            "title: Labelled\nloaders:\n  labelled: { sql: \"SELECT id, label FROM v_tally_labelled\" }\n\
             view: { viz: table, from: labelled }\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/fixtures/labelled.yaml",
            "when: { page: labelled }\nexpect: { loaders: { labelled: { columns: [id, label], rows: 0 } } }\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/fixtures/things.yaml",
            &format!(
                "when: {{ collector: things, rows: [{{ id: 1 }}, {{ id: 2 }}] }}\nexpect: {collector_expect}\n"
            ),
        );
        write(
            root,
            ".oxplow/extensions/tally/fixtures/shell.yaml",
            "when: { collector: shell }\nexpect: { entities: { line: $any } }\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/README.md",
            "Read `v_tally_labelled`; note a work item with `tally.work.note`.\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/questions.yaml",
            "- question: Which things are labelled?\n  skill: README.md\n  reaches:\n    sql: SELECT id, label FROM v_tally_labelled\n  shape: { columns: [id, label] }\n- question: Note a work item.\n  skill: README.md\n  reaches:\n    command: tally.work.note\n    input: { uri: \"work_item:oxplow:tsk1\" }\n",
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
            ".oxplow/extensions/tally/collectors/things.star",
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
    /// oxplow — a page's loader rows, a starlark collector's typed entities from
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
            ".oxplow/extensions/maker/extension.yaml",
            &format!(
                "manifest: 2\nname: maker\nintent:\n  purpose: Make items.\ndatasets:\n  - name: items\n    version: {version}\n    description: Items.\n    fields:\n      - {{ name: uri, type: TEXT, doc: A work item. }}\nexports:\n  - {{ contribution: oxplow:oxplow/dataset?dataset=maker/items, version: {version}, visibility: stable }}\n"
            ),
        );
        write(
            root,
            ".oxplow/extensions/maker/datasets/items.sql",
            "SELECT uri FROM ref('work_item')\n",
        );
        write(
            root,
            ".oxplow/extensions/reader/extension.yaml",
            "manifest: 2\nname: reader\nintent:\n  purpose: Read the maker's items.\nimports:\n  - { id: items, contribution: oxplow:oxplow/dataset?dataset=maker/items, version: 1, optional: true }\n",
        );
        write(
            root,
            ".oxplow/extensions/reader/pages/seen.yaml",
            "title: Seen\nloaders:\n  seen: { sql: \"SELECT uri FROM import_ref('items')\" }\n\
             view: { viz: table, from: seen }\n",
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
            ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: available"]
        );
        assert!(render(&report, crate::Format::Text).contains(
            "import `items` → oxplow:oxplow/dataset?dataset=maker/items v1: available\n"
        ));
        let checked = crate::check(
            dir.path(),
            "reader",
            &oxplow_app::extension::extension_catalog::ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            crate::render_findings(&checked, crate::Format::Text).contains(
                "import `items` → oxplow:oxplow/dataset?dataset=maker/items v1: available\n"
            )
        );

        let dir = tempfile::tempdir().unwrap();
        producer_and_consumer(dir.path(), 2);
        let report = test_extension(dir.path(), "reader", false).await.unwrap();
        assert_eq!(
            report.imports,
            ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: unavailable (maker exports v2)"]
        );

        // A person's own disable doesn't change what a test runs with.
        let dir = tempfile::tempdir().unwrap();
        producer_and_consumer(dir.path(), 1);
        write(
            dir.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [maker]\n",
        );
        let report = test_extension(dir.path(), "reader", false).await.unwrap();
        assert_eq!(
            report.imports,
            ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: available"]
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
            let manifest = dir.path().join(".oxplow/extensions/tally/extension.yaml");
            let yaml = std::fs::read_to_string(&manifest).unwrap().replace(
                "    - {{ name: shell,".replace("{{", "{").as_str(),
                "    - { name: counted, input: { command: tally.things.count }, expect: the things counted }\n    - { name: shell,",
            ) + "  - name: things.count\n    summary: How many things there are.\n    entry: handlers/count.star\n    needs: [sql.read]\n    access: read\n    input_schema: { type: object }\n";
            std::fs::write(&manifest, yaml).unwrap();
            write(
                dir.path(),
                ".oxplow/extensions/tally/handlers/count.star",
                "def transform(x):\n    rows = scope(\"sql.read\", {\"sql\": \"SELECT id FROM v_tally_labelled\"})\n    return {\"commands\": [], \"result\": {\"count\": len(rows), \"of\": \"things\"}}\n",
            );
            write(
                dir.path(),
                ".oxplow/extensions/tally/fixtures/counted.yaml",
                &format!(
                    "when: {{ command: tally.things.count, input: {{}}, answers: {{ sql.read: [[{{ id: 1 }}, {{ id: 2 }}]] }} }}\nexpect: {{ result: {want} }}\n"
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

    /// An extension with one incremental dataset over page visits, watermarked
    /// on `watermark`, and its `before` / `after` fixture.
    /// The `visits` extension: an incremental dataset keyed and watermarked
    /// on `id`, with a fixture whose `before` visit has id `before` and whose
    /// later `after` visit has id `after`.
    fn visits(root: &Path, before: i64, after: i64) {
        write(
            root,
            ".oxplow/extensions/visits/extension.yaml",
            "manifest: 2
name: visits
intent:
  purpose: Long visits.
  origin: oxplow:oxplow/thread?id=thr1
  examples: []
datasets:
  - name: long_visits
    version: 1
    description: Visits and how long they lasted.
    key: [id]
    materialize: { incremental: id }
    fields:
      - { name: id, type: INTEGER, doc: The visit. }
      - { name: duration_ms, type: INTEGER, doc: How long it lasted. }
",
        );
        write(
            root,
            ".oxplow/extensions/visits/datasets/long_visits.sql",
            "SELECT id, duration_ms FROM ref('page_visit') WHERE duration_ms IS NOT NULL\n",
        );
        write(
            root,
            ".oxplow/extensions/visits/fixtures/dataset-long_visits.yaml",
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

    /// P8.B5: each incremental dataset is appended to on its fixture's
    /// `after` rows and checked against a full refill — a key that doesn't
    /// grow with arrivals (a row lands below the watermark, unseen) fails
    /// at the dataset's line; one that does passes.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_checks_incremental_datasets_against_a_full_refill() {
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
                ".oxplow/extensions/visits/extension.yaml:8: incremental dataset `long_visits`"
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
            .join(".oxplow/extensions/notes/fixtures/basic.yaml");
        let text = std::fs::read_to_string(&fixture).unwrap();
        std::fs::write(
            &fixture,
            text.replace(
                "commands: [oxplow.work_item.comment]",
                "commands: [oxplow.work_item.update]",
            ),
        )
        .unwrap();
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains(
                ".oxplow/extensions/notes/fixtures/basic.yaml:1: effect `on-done` composed"
            ),
            "{errors}"
        );
    }

    /// An event fixture names the commands an effect ran, not the one
    /// sequence that carried them.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_event_fixture_lists_the_commands_an_effect_ran_not_their_sequence() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: T }, bind: t }]
when:
  event: { type: work_item.state_changed, payload: { work_item: $t.uri, to: done } }
expect:
  commands: [oxplow.work_item.comment]
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// A command fixture lists what reacted to it by the commands an effect
    /// ran, not the sequence that carried them.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_fixture_lists_the_commands_an_effect_ran_not_their_sequence() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: T, state: in_progress }, bind: t }]
when: { command: oxplow.work_item.transition, input: { uri: $t.uri, to: done } }
expect:
  ok: true
  commands: [oxplow.work_item.comment]
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { turn }` makes a turn the way hooks report one: its
    /// prompt, its tool calls (a failed one, one naming a file), its
    /// answer, and the file it wrote.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_turn_through_the_hook_ingest() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe:
      turn:
        prompt: Add CSV export
        tools:
          - { name: Edit, kind: edit, paths: [src/report.rs], ok: true }
          - { name: Bash, kind: shell, command: cargo test, detail: cargo test, ok: false }
        answer: Done; used the csv crate.
when:
  sql: SELECT tool, path, ok FROM v_tool_call ORDER BY id
expect:
  rows:
    - { tool: Edit, path: src/report.rs, ok: 1 }
    - { tool: Bash, ok: 0 }
",
        );
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/answer.yaml",
            "given:
  - observe: { turn: { prompt: Add CSV export, answer: Done; used the csv crate. } }
when:
  sql: SELECT prompt, answer FROM v_agent_turn
expect:
  rows: [{ prompt: Add CSV export, answer: Done; used the csv crate. }]
",
        );
        let manifest = dir.path().join(".oxplow/extensions/notes/extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            text.replacen(
                "  examples:\n",
                "  examples:\n    - { name: answer, expect: the turn's answer }\n",
                1,
            ),
        )
        .unwrap();
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
        assert!(
            report.ran.contains(&"example answer".to_string()),
            "{:?}",
            report.ran
        );
    }

    /// `observe: { event }` logs an event and lets the effects react, so a
    /// scenario reads what they did.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_an_event_the_effects_react_to() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: T }, bind: t }
  - observe: { event: { type: work_item.state_changed, payload: { work_item: $t.uri, to: done } } }
when: { sql: SELECT count(*) AS n FROM v_work_item_comment }
expect: { rows: [{ n: 1 }] }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { commit }` stores a commit the way the git indexer does.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_commit() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { commit: { sha: c1, at: \"2026-01-01T00:00:00Z\", subject: first, files: [{ path: a.rs, status: A, additions: 1 }] } }
  - observe: { commit: { sha: c2, at: \"2026-01-02T00:00:00Z\", subject: second, files: [{ path: a.rs, additions: 2, deletions: 1 }] } }
when:
  sql: SELECT c.subject, f.status, f.additions FROM v_commit c JOIN v_commit_file f ON f.sha = c.sha WHERE f.path = 'a.rs' ORDER BY c.committed_at DESC
expect:
  rows:
    - { subject: second, status: M, additions: 2 }
    - { subject: first, status: A, additions: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { change }` stores a change's analysis as the change
    /// analysis does: the files and functions the datasets read.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_changes_analysis() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe:
      change:
        kind: commit
        target: abc
        files:
          - { path: src/hot.rs, status: modified, additions: 10, deletions: 2 }
          - { path: src/new.rs, status: added, additions: 3, is_test: true }
        functions:
          - { path: src/hot.rs, symbol: grow, status: modified, body_changed: true, length: 20, complexity: 17, complexity_delta: 3 }
when:
  sql: SELECT f.path, f.additions, f.is_test, (SELECT count(*) FROM v_change_function g WHERE g.change_id = f.change_id AND g.complexity = 17) AS fns FROM v_change_file f JOIN v_change c ON c.id = f.change_id WHERE c.kind = 'commit' AND c.target = 'abc' ORDER BY f.path
expect:
  rows:
    - { path: src/hot.rs, additions: 10, is_test: 0, fns: 1 }
    - { path: src/new.rs, additions: 3, is_test: 1, fns: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { change }` of an effort's change takes the effort's id.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_an_efforts_change() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe: { change: { kind: effort, target: eff1, files: [{ path: src/a.rs, additions: 10, deletions: 4 }] } }
when:
  sql: SELECT c.kind, c.target, f.deletions FROM v_change c JOIN v_change_file f ON f.change_id = c.id
expect:
  rows:
    - { kind: effort, target: '1', deletions: 4 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { test_run }` records a run's cases as a run recorder does:
    /// a capture of the effort with a fact per case.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_test_run() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe: { test_run: { effort: eff1, at: \"2026-01-01T00:00:00Z\", cases: [{ name: a, status: passed }, { name: b, status: failed }] } }
  - observe: { test_run: { effort: eff1, at: \"2026-01-01T00:01:00Z\", asserted_failed: 2 } }
when:
  sql: SELECT dims_json, count(*) AS n FROM v_fact WHERE measure_key = 'oxplow.test_case' GROUP BY dims_json ORDER BY n
expect:
  rows:
    - { dims_json: '{\"oxplow.status\":\"passed\"}', n: 1 }
    - { dims_json: '{\"oxplow.status\":\"failed\"}', n: 3 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { snapshot }` appends to the stream's snapshot log: each
    /// take's parent is the one before it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_snapshot_log() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { snapshot: { trigger: startup, elapsed_ms: 5, files: [{ path: a.txt, hash: h1 }] } }
  - observe: { snapshot: { trigger: quiet, elapsed_ms: 900, budget_ms: 500, files: [{ path: a.txt, hash: h2 }] } }
when:
  sql: SELECT trigger, over_budget, file_count, parent_snapshot_id IS NULL AS root FROM v_snapshot_op ORDER BY seq DESC
expect:
  rows:
    - { trigger: quiet, over_budget: 1, file_count: 1, root: 0 }
    - { trigger: startup, over_budget: 0, file_count: 1, root: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { diagnostics }` is what a language server reported for a
    /// file; `observe: { symbols }` the definitions the index read from it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_diagnostics_and_symbols() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { diagnostics: { path: a.rs, items: [{ severity: error, message: broken, line: 3, col: 1 }, { severity: warning, message: unused, line: 9 }] } }
  - observe: { symbols: { path: src/lib.rs, language: rust, items: [{ name: main, kind: fn, line: 1 }, { name: helper, kind: fn, container: main, line: 5 }] } }
when:
  sql: SELECT 'd' AS k, path AS a, severity AS b FROM v_diagnostic UNION ALL SELECT 's', path, name FROM v_symbol ORDER BY 1, 3
expect:
  rows:
    - { k: d, a: a.rs, b: error }
    - { k: d, a: a.rs, b: warning }
    - { k: s, a: src/lib.rs, b: helper }
    - { k: s, a: src/lib.rs, b: main }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { facts }` records findings of a measure, as a collector's
    /// capture does.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_facts() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { facts: { measure: oxplow.test_case, items: [{ path: src/a.rs, line: 3, rule: bug }, { path: src/b.rs, line: 9, rule: cleanup, value: 2, dims: { oxplow.status: failed } }] } }
when:
  sql: SELECT path, line, rule, value, dims_json LIKE '%failed%' AS failed FROM v_fact WHERE measure_key = 'oxplow.test_case' ORDER BY path
expect:
  rows:
    - { path: src/a.rs, line: 3, rule: bug, value: 1.0, failed: null }
    - { path: src/b.rs, line: 9, rule: cleanup, value: 2.0, failed: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { visit }` is a page the person opened, as the shell
    /// records it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_page_visits() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { visit: { page: \"oxplow:oxplow/page?page=problems\", kind: problems, label: Problems, duration_ms: 1200 } }
  - observe: { visit: { page: \"oxplow:oxplow/page?page=problems\", kind: problems } }
  - observe: { visit: { page: \"oxplow:oxplow/page?page=symbols\", kind: symbols, thread: thr1 } }
when:
  sql: SELECT page_kind, page_id, label, duration_ms, thread_id FROM v_page_visit ORDER BY id
expect:
  rows:
    - { page_kind: problems, page_id: \"oxplow:oxplow/page?page=problems\", label: Problems, duration_ms: 1200 }
    - { page_kind: problems }
    - { page_kind: symbols, thread_id: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// A test's oxplow has only what is declared on: the required
    /// foundation, the extension tested and what it requires. Oxplow's own
    /// pages run complete without the default extensions adding to them.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_foundation_fixture_runs_without_bundled() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        let host = Host::start(
            dir.path(),
            "oxplow-foundation",
            &[],
            Arc::new(|_: &str| None),
            false,
        )
        .await
        .unwrap();
        observe_commit(
            &host,
            &json!({ "sha": "c1", "at": "2026-01-01T00:00:00Z", "subject": "first", "files": [{ "path": "a.rs" }] }),
        )
        .await
        .unwrap();
        let run = run_page(
            &host,
            oxplow_app::pages::runner::RunTarget::Page("oxplow-foundation/git-history".into()),
            &json!({ "stream_id": 1, "head_sha": "c1" }),
            &oxplow_app::pages::spec::ViewContext::default(),
        )
        .await
        .unwrap();
        fn walk<'a>(
            n: &'a oxplow_app::pages::runner::RunNode,
            out: &mut Vec<&'a oxplow_app::pages::runner::RunNode>,
        ) {
            out.push(n);
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut nodes = Vec::new();
        walk(&run.tree, &mut nodes);
        assert!(!nodes.is_empty());
        for n in nodes {
            assert_eq!(n.replaced_by, None, "{}", n.id);
            assert!(!n.id.starts_with("oxplow-bundled."), "{}", n.id);
        }
    }

    /// A fixture's `requires` turns on extensions beyond the one tested
    /// (and what they require): what only oxplow-bundled adds (its pages)
    /// is there when a fixture asks for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fixture_requires_the_extensions_it_needs() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        let fixture = |requires: &str| {
            format!(
                "{requires}given:
when: {{ sql: SELECT count(*) > 0 AS here FROM v_page WHERE extension = 'oxplow-bundled' }}
expect: {{ rows: [{{ here: 1 }}] }}
"
            )
        };
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            &fixture("requires: [oxplow-bundled]\n"),
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        // Without it its pages aren't there.
        std::fs::write(
            dir.path()
                .join(".oxplow/extensions/notes/fixtures/basic.yaml"),
            fixture(""),
        )
        .unwrap();
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(!report.errors.is_empty());
        // A name that isn't an extension here is a finding.
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            &fixture("requires: [no-such-extension]\n"),
        );
        let errors = test_extension(dir.path(), "notes", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/basic.yaml:1: `requires` names `no-such-extension`"),
            "{errors}"
        );
    }

    /// `observe: { refs }` is what the git indexer read of the repository's
    /// branches, tags and the stream's head; `snapshot` takes a `revision`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_git_refs_and_a_snapshots_revision() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { commit: { sha: c1, at: \"2026-01-01T00:00:00Z\", subject: first } }
  - observe: { refs: { branches: [{ name: main, head: c1, stream: 1 }, { name: side, head: c1 }], tags: [{ name: v1, sha: c1 }], heads: [{ stream: 1, sha: c1 }] } }
  - observe: { snapshot: { trigger: quiet, revision: \"git:c1\", files: [{ path: a.txt, hash: h1 }] } }
when:
  sql: SELECT (SELECT group_concat(name) FROM (SELECT name FROM v_branch ORDER BY name)) AS branches, (SELECT name FROM v_tag) AS tag, (SELECT head_sha FROM v_stream) AS head, (SELECT revision FROM v_snapshot) AS revision
expect:
  rows:
    - { branches: \"main,side\", tag: v1, head: c1, revision: \"git:c1\" }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { run }` and `{ health }` are what the extension host
    /// records of a contribution's run and its failures; `token_usage`,
    /// `hint` and `nudge` what a harness and the hints record.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_runs_health_usage_and_hints() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { run: { extension: acme, contribution: pull, kind: collector, outcome: failed, days_ago: 2, cpu_ms: 30, ai_calls: 2 } }
  - observe: { run: { extension: acme, contribution: pull, kind: collector, outcome: ok } }
  - observe: { health: { extension: acme, contribution: pull, kind: collector, disable: boom } }
  - observe: { token_usage: { prompt: fix it, input_tokens: 1999, output_tokens: 1 } }
  - observe: { hint: { hint: x/never, evaluated: 3 } }
  - observe: { nudge: { kind: x/spoke, message: m } }
when:
  sql: SELECT (SELECT group_concat(outcome || ':' || cpu_ms || ':' || ai_calls, ',') FROM (SELECT * FROM v_extension_run ORDER BY started_at)) AS runs, (SELECT state || ':' || reason FROM v_contribution_health) AS health, (SELECT input_tokens FROM v_token_usage) AS tokens, (SELECT evaluated FROM v_hint_stat WHERE hint = 'x/never') AS evaluated, (SELECT count(*) FROM v_agent_nudge WHERE kind = 'x/spoke') AS nudges
expect:
  rows:
    - { runs: \"failed:30:2,ok:0:0\", health: \"disabled:boom\", tokens: 1999, evaluated: 3, nudges: 1 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// A turn is observed on a thread of its own, in a session of its own,
    /// with a subagent's hand-back (no prompt) and a call as it starts.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_a_turn_on_a_thread_and_session_with_handbacks() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - { command: oxplow.thread.create, input: { stream: str1, title: Beta }, bind: beta }
  - { command: oxplow.agent_session.open, input: { thread: $beta.id, harness: codex }, bind: ses }
  - observe: { turn: { thread: $beta.id, session: $ses.id, prompt: first ask, answer: one, tools: [{ name: ExitPlanMode, kind: plan, plan: { text: \"1. First\" }, pre: true }] } }
  - observe: { turn: { thread: $beta.id, session: $ses.id, prompt: null, subagent: { id: a1, kind: Explore }, answer: report } }
when:
  sql: SELECT t.prompt, t.answer, th.title FROM v_agent_turn t JOIN v_thread th ON th.id = t.thread_id WHERE th.title = 'Beta' ORDER BY t.id
expect:
  rows:
    - { prompt: first ask, answer: one, title: Beta }
    - { prompt: \"\", answer: report, title: Beta }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
    }

    /// `observe: { evidence }` sets the observations an effort's page
    /// reads.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_observes_an_efforts_evidence() {
        let dir = tempfile::tempdir().unwrap();
        crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe:
      evidence:
        effort: eff1
        observations:
          - { kind: test-run, payload: { passed: 4, failed: 1 } }
          - { kind: diff-coverage, metric_value: 79.5, payload: { summaryPct: 79.5 } }
when:
  sql: SELECT kind, metric_value, provenance FROM v_effort_observation WHERE effort_id = 1 ORDER BY seq
expect:
  rows:
    - { kind: test-run, metric_value: null, provenance: observed }
    - { kind: diff-coverage, metric_value: 79.5 }
",
        );
        let report = test_extension(dir.path(), "notes", false).await.unwrap();
        assert!(report.ok, "{:?}", report.errors);
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
                              "input": {"thread": thread, "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {"sql": "SELECT uri FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
                                  "params": {"w": item}})
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": uri.param(r["uri"], "id"), "reason": "switch"}}
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
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: Open and close efforts as items start and finish.
  examples:
    - { name: basic, expect: nothing to do }
    - { name: start, expect: its effort opens }
    - { name: finish, expect: its effort closes }
permissions: [sql.read, commands:oxplow.effort.*]
implementations:
  - { capability: effort_policy, id: tidy, title: Tidy, entry: policies/tidy.star, needs: [sql.read] }
",
        );
        write(root, ".oxplow/extensions/acme/policies/tidy.star", script);
        write(
            root,
            ".oxplow/extensions/acme/fixtures/basic.yaml",
            &format!(
                "when: {{ implementation: tidy, event: {{ type: thread.checkpoint, payload: {{ thread: thr1, turn: trn1, reason: turn_end, snapshot: 1, changed: false, writing_tools: 0 }} }} }}\nexpect: {basic_expect}\n"
            ),
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/start.yaml",
            "requires: [oxplow-bundled]\nwhen: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: in_progress }, anchors: { thread_id: thr1 } } }\nexpect: { commands: [oxplow.effort.open] }\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/finish.yaml",
            "requires: [oxplow-bundled]\nwhen: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: done } }, answers: { sql.read: [[{ uri: \"oxplow:oxplow/effort?id=eff3\" }]] } }\nexpect: { commands: [oxplow.effort.close] }\n",
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
            .join(".oxplow/extensions/acme/fixtures/finish.yaml");
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
        policy(root, shape, "{ skip: \"2 fixture None None None\" }");
        // `start` and `finish` aren't this script's; only `basic` stays.
        for gone in ["start", "finish"] {
            std::fs::remove_file(
                root.join(format!(".oxplow/extensions/acme/fixtures/{gone}.yaml")),
            )
            .unwrap();
        }
        let manifest = root.join(".oxplow/extensions/acme/extension.yaml");
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
                "newer than the registered v2",
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
                ".oxplow/extensions/acme/fixtures/basic.yaml",
                &format!(
                    "when: {{ implementation: tidy, event: {event} }}\nexpect: {{ skip: $any }}\n"
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
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
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
            ".oxplow/extensions/acme/merged.v1.json",
            r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.v2.json",
            r#"{"type": "object", "required": ["number", "by"], "properties": {"number": {"type": "integer"}, "by": {"type": "string"}}}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.star",
            "def transform(x):\n    p = dict(x[\"payload\"])\n    p[\"by\"] = \"unknown\"\n    return p\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/note.star",
            "def transform(x):\n    e = x[\"event\"]\n    return {\"skip\": \"v%d by %s\" % (e[\"v\"], e[\"payload\"][\"by\"])}\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/old.yaml",
            "when: { effect: note, event: { type: acme.merged, v: 1, payload: { number: 3 } } }\nexpect: { skip: v2 by unknown }\n",
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
    /// The skill's table of fixture kinds is the runner's own (`KINDS`):
    /// what the runner accepts is what the skill teaches. `OXPLOW_BLESS=1`
    /// writes it.
    #[test]
    fn the_skill_lists_every_fixture_kind_the_runner_runs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oxplow-agent-text/assets/skills/oxplow-extension/references/testing.md");
        let text = std::fs::read_to_string(&path).unwrap();
        let (start, end) = ("<!-- fixture-kinds -->\n", "<!-- /fixture-kinds -->");
        let (Some(a), Some(b)) = (text.find(start), text.find(end)) else {
            panic!("testing.md has no `{start}` … `{end}` block");
        };
        let table = kinds_table();
        if std::env::var_os("OXPLOW_BLESS").is_some() {
            let blessed = format!("{}{start}{table}{}", &text[..a], &text[b..]);
            std::fs::write(&path, blessed).unwrap();
            return;
        }
        assert_eq!(
            text[a + start.len()..b],
            table,
            "testing.md's fixture kinds are stale: regenerate them with OXPLOW_BLESS=1"
        );
    }

    /// One rule compares every `expect`: an object only on the keys it
    /// names, a list item by item in order, `{ contains: [..] }` items a
    /// list has in any order, `$any` anything, `{ $exact: v }` the whole value.
    #[test]
    fn one_rule_compares_what_an_example_expects() {
        let got = json!({ "state": "done", "rows": [{ "a": 1, "b": 2 }, { "a": 3, "b": 4 }] });
        let holds = |want: Value| compare(&want, &got).is_none();
        assert!(holds(json!({ "state": "done" })));
        assert!(holds(json!({ "rows": [{ "a": 1 }, { "b": 4 }] })));
        assert!(
            !holds(json!({ "rows": [{ "a": 1 }] })),
            "a list is every item"
        );
        assert!(holds(json!({ "rows": { "contains": [{ "a": 3 }] } })));
        assert!(holds(
            json!({ "rows": { "contains": [{ "b": 4 }, { "a": 1 }] } })
        ));
        assert!(!holds(json!({ "rows": { "contains": [{ "a": 5 }] } })));
        assert!(holds(json!({ "rows": "$any", "state": "$any" })));
        assert!(!holds(json!({ "missing": 1 })));
        assert!(
            !holds(json!({ "missing": "$any" })),
            "$any is a value that's there"
        );
        assert!(holds(json!({ "missing": null })));
        let no = json!({ "ok": false, "refused": "`tsk9` isn't a work item" });
        assert!(compare(&json!({ "refused": "isn't a work item" }), &no).is_none());
        assert!(compare(&json!({ "refused": "is archived" }), &no).is_some());
        assert!(holds(
            json!({ "rows": { "$exact": [{ "a": 1, "b": 2 }, { "a": 3, "b": 4 }] } })
        ));
        assert!(!holds(
            json!({ "rows": { "$exact": [{ "a": 1 }, { "a": 3 }] } })
        ));
        assert_eq!(
            compare(&json!({ "rows": [{ "a": 1 }, { "a": 9 }] }), &got),
            Some(("/rows/1/a".into(), json!(9), json!(3)))
        );
    }

    /// Every kind of fixture has `given` steps whose bound values its
    /// `when` and `expect` read, and `steps`: each `{ when?, expect }` runs
    /// again with its `when` over the fixture's, and a finding names its step.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fixture_of_any_kind_has_given_and_steps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged }, expect: valid }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: Merged. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: Merged., upcast: merged.star }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.v1.json",
            r#"{"type": "object", "required": ["number"]}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.v2.json",
            r#"{"type": "object", "required": ["pr"]}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.star",
            "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"]}\n",
        );
        let scenario = |last: &str| {
            format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Fix it }}, bind: item }}]
when: {{ event_type: acme.merged, payload: {{ pr: $item.uri }} }}
expect: {{ valid: true }}
steps:
  - when: {{ v: 1, payload: {{ number: $item.uri }} }}
    expect: {{ valid: true, upcast: {{ pr: $item.uri }} }}
  - when: {{ payload: {{ number: 1 }} }}
    expect: {{ valid: {last} }}
"
            )
        };
        write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            &scenario("false"),
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            &scenario("true"),
        );
        let errors = test_extension(root, "acme", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains(".oxplow/extensions/acme/fixtures/merged.yaml:1: steps[1]: "),
            "{errors}"
        );
    }

    /// A step whose `when` names another kind — or, in a page's scenario,
    /// another page — runs that instead of the fixture's own, and the
    /// scenario goes on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_step_runs_another_kind_or_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged }, expect: valid }
    - { name: paged, input: { page: titles }, expect: titles }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.json, summary: Merged. }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.json",
            r#"{"type": "object", "required": ["pr"]}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/pages/titles.yaml",
            "title: Titles\nloaders:\n  titles: { sql: \"SELECT title FROM v_work_item\" }\nview: { viz: table, from: titles }\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/pages/count.yaml",
            "title: Count\nloaders:\n  n: { sql: \"SELECT COUNT(*) AS n FROM v_work_item\" }\nview: { viz: number, from: n }\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { event_type: acme.merged, payload: { pr: 1 } }
expect: { valid: true }
steps:
  - when: { sql: \"SELECT title FROM v_work_item\" }
    expect: { rows: [{ title: Fix it }] }
  - expect: { valid: true }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/paged.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { page: titles }
expect: { loaders: { titles: { rows: [{ title: Fix it }] } } }
steps:
  - when: { page: count }
    expect: { loaders: { n: { rows: [{ n: 1 }] } } }
  - when: { sql: \"SELECT COUNT(*) AS n FROM v_work_item\" }
    expect: { rows: [{ n: 1 }] }
  - expect: { loaders: { titles: { rows: 1 } } }
",
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn event_type_and_ref_kind_fixtures_check_what_they_declare() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
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
datasets:
  - name: prs
    version: 1
    description: PRs.
    fields:
      - { name: uri, type: \"\", doc: The ref. }
      - { name: title, type: \"\", doc: Its title. }
resources:
  - { name: pr, label: Pull request, params: { id: { required: true, pattern: '^[0-9]+$' } }, resolve: prs, wikilink: { prefix: pr, param: id }, icon: git-pull-request }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.v1.json",
            r#"{"type": "object", "required": ["number"]}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.v2.json",
            r#"{"type": "object", "required": ["pr"]}"#,
        );
        write(
            root,
            ".oxplow/extensions/acme/merged.star",
            "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"]}\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/datasets/prs.sql",
            "SELECT 'oxplow:acme/pr?id=1' AS uri, 'One' AS title",
        );
        write(
            root,
            ".oxplow/extensions/acme/pages/pr.yaml",
            "title: Pull request\ncategory: Work\nfor: acme/pr\ndefault: true\nparams: [{ name: id }]\n\
             loaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            "when: { event_type: acme.merged, payload: { pr: 12 } }\nexpect: { valid: true }\n",
        );
        write(root, ".oxplow/extensions/acme/fixtures/old.yaml", "when: { event_type: acme.merged, v: 1, payload: { number: 12 } }\nexpect: { valid: true, upcast: { pr: 12 } }\n");
        write(
            root,
            ".oxplow/extensions/acme/fixtures/link.yaml",
            "when: { wikilink: \"pr:12\" }\nexpect: { uri: \"oxplow:acme/pr?id=12\" }\n",
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

        write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            "when: { event_type: acme.merged, payload: { number: 12 } }\nexpect: { valid: true }\n",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/link.yaml",
            "when: { wikilink: \"pr:12\" }\nexpect: { uri: \"oxplow:acme/pr?id=13\" }\n",
        );
        let errors = test_extension(root, "acme", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains(
                ".oxplow/extensions/acme/fixtures/merged.yaml:1: `acme.merged@2` refuses"
            ),
            "{errors}"
        );
        assert!(
            errors.contains(".oxplow/extensions/acme/fixtures/link.yaml:1: `[[pr:12]]` names"),
            "{errors}"
        );
    }

    /// A page's `viz: form` node is opened as a person would: its defaults
    /// bound from the params, its fixed and shown fields; a wrong
    /// expectation is a finding at its fixture.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_opens_a_form_page_and_compares_what_it_starts_with() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/editor/extension.yaml",
            "manifest: 2
name: editor
intent:
  purpose: Edit work items.
  examples:
    - { name: edit, input: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }, expect: the ref is fixed }
",
        );
        write(
            root,
            ".oxplow/extensions/editor/pages/edit-item.yaml",
            "title: Edit\nparams: [{ name: uri }]\nloaders:\n  item: { sql: \"SELECT title, body FROM v_work_item WHERE uri = :uri\" }\n\
             view: { viz: form, from: item, form: { command: oxplow.work_item.update, defaults: { uri: '{{param.uri}}' }, fixed: [uri], fields: [title, body] } }\n",
        );
        write(
            root,
            ".oxplow/extensions/editor/fixtures/edit.yaml",
            "requires: [oxplow-bundled]\nwhen: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }\nexpect: { values: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' }, fixed: [uri], fields: [title, body] }\n",
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
            ".oxplow/extensions/editor/fixtures/edit.yaml",
            "requires: [oxplow-bundled]\nwhen: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }\nexpect: { fixed: [uri, title] }\n",
        );
        let errors = test_extension(root, "editor", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains(
                ".oxplow/extensions/editor/fixtures/edit.yaml:1: form `edit-item` opened with"
            ),
            "{errors}"
        );
    }

    /// A page with a work-item loader, one param and an answer.
    fn pager(root: &Path, query: &str, answer_params: &str, expect: &str) {
        write(
            root,
            ".oxplow/extensions/pager/extension.yaml",
            "manifest: 2
name: pager
intent:
  purpose: Show work items.
  examples:
    - { name: created, input: { page: items }, expect: the created item is listed }
",
        );
        write(
            root,
            ".oxplow/extensions/pager/pages/items.yaml",
            &format!(
                "title: Items
params: [{{ name: state }}]
loaders:
  items: {{ sql: \"{query}\" }}
view:
  stack:
    - {{ id: list, viz: table, from: items }}
answers:
  - {{ question: \"What is open?\", params: {answer_params} }}
"
            ),
        );
        write(
            root,
            ".oxplow/extensions/pager/fixtures/created.yaml",
            &format!(
                "requires: [oxplow-bundled]\ngiven:
  - {{ command: oxplow.work_item.create, input: {{ title: Write the page test }} }}
when: {{ page: items }}
expect: {expect}
"
            ),
        );
    }

    const LIST: &str = "SELECT title FROM v_work_item WHERE title = 'Write the page test'";

    /// A page fixture runs `given` steps first, then the page, and checks
    /// its loaders and nodes; the answers run too.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_runs_a_page_after_its_given_steps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pager(
            root,
            LIST,
            "{}",
            "{ loaders: { items: { rows: [{ title: Write the page test }], columns: [title] } }, nodes: [{ id: list, kind: viz }] }",
        );
        let report = test_extension(root, "pager", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(
            report.ran.contains(&"example created".to_string()),
            "{:?}",
            report.ran
        );
        assert!(
            report.ran.contains(&"answers".to_string()),
            "{:?}",
            report.ran
        );
    }

    /// A row that differs is a finding that names the loader, the place and
    /// both values.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_names_the_page_row_that_differs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pager(
            root,
            LIST,
            "{}",
            "{ loaders: { items: { rows: [{ title: Something else }] } } }",
        );
        let errors = test_extension(root, "pager", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/created.yaml:1: page `items` loader `items` returned")
                && errors.contains("Something else")
                && errors.contains("Write the page test"),
            "{errors}"
        );
    }

    /// An answer whose params break the page's query fails, naming the
    /// page and the question.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_fails_an_answer_that_breaks_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pager(
            root,
            // Runs at the page's defaults (no state); the answer's state
            // isn't JSON, so its run fails.
            "SELECT json_extract(:state, '$') AS state",
            "{ state: open }",
            "{ loaders: { items: { rows: $any } } }",
        );
        let errors = test_extension(root, "pager", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("pages/items.yaml:1: the answer to \"What is open?\" doesn't run"),
            "{errors}"
        );
    }

    /// `expect.text` checks a page's text: `contains` and the exact form.
    #[tokio::test(flavor = "multi_thread")]
    async fn extension_test_checks_a_pages_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pager(
            root,
            LIST,
            "{}",
            "{ text: { contains: [Items, Write the page test] } }",
        );
        let report = test_extension(root, "pager", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        pager(root, LIST, "{}", "{ text: { contains: [Nowhere] } }");
        let errors = test_extension(root, "pager", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("text doesn't contain \"Nowhere\""),
            "{errors}"
        );

        pager(root, LIST, "{}", "{ text: nothing like it }");
        let errors = test_extension(root, "pager", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("the example expects \"nothing like it\""),
            "{errors}"
        );
    }

    /// Extension `paged` with page `items` (`page`, its YAML) and one
    /// intent example per fixture, named by it.
    fn paged(root: &Path, page: &str, fixtures: &[(&str, &str)]) {
        let examples: String = fixtures
            .iter()
            .map(|(name, _)| {
                format!("    - {{ name: {name}, input: {{ page: items }}, expect: it }}\n")
            })
            .collect();
        write(
            root,
            ".oxplow/extensions/paged/extension.yaml",
            &format!(
                "manifest: 2\nname: paged\nintent:\n  purpose: Show work items.\n  examples:\n{examples}"
            ),
        );
        write(root, ".oxplow/extensions/paged/pages/items.yaml", page);
        for (name, body) in fixtures {
            write(
                root,
                &format!(".oxplow/extensions/paged/fixtures/{name}.yaml"),
                body,
            );
        }
    }

    /// A page of one work item by `uri`, with a row action.
    const BY_URI: &str = "title: Items
params: [{ name: uri }]
loaders:
  items: { sql: \"SELECT title, uri FROM v_work_item WHERE uri = :uri\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";

    /// A fixture binds a step's result (`bind: <name>`) and a value of a
    /// run (`steps[].bind`), and uses either as `$name.path` in a later
    /// step's input, the params and the expectation; `steps` change the
    /// state and run the page again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_binds_values_and_runs_its_steps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        paged(
            root,
            BY_URI,
            &[(
                "bound",
                "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First }, bind: first }
when: { page: items, params: { uri: $first.uri } }
expect:
  loaders: { items: { rows: [{ title: First, uri: $first.uri }] } }
  nodes: [{ id: items, kind: viz, actions: [start] }]
steps:
  - expect: { loaders: { items: { rows: 1 } } }
    bind: { title: 'loaders.items.rows[0].title' }
  - given: [{ command: oxplow.work_item.create, input: { title: Second }, bind: second }]
  - params: { uri: $second.uri }
    expect: { loaders: { items: { rows: [{ title: Second }] } } }
  - params: { uri: $first.uri }
    expect: { loaders: { items: { rows: [{ title: $title }] } } }
",
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        // A step that doesn't hold names its place in the scenario.
        paged(
            root,
            BY_URI,
            &[(
                "bound",
                "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First }, bind: first }
when: { page: items, params: { uri: $first.uri } }
steps:
  - expect: { loaders: { items: { rows: [{ title: Other }] } } }
",
            )],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors
                .contains("fixtures/bound.yaml:1: steps[0]: page `items` loader `items` returned"),
            "{errors}"
        );

        // A name nothing bound is a finding that says how to bind it.
        paged(
            root,
            BY_URI,
            &[(
                "bound",
                "when: { page: items, params: { uri: $nowhere.uri } }\n",
            )],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(errors.contains("`$nowhere` isn't bound"), "{errors}");
    }

    /// A loader's `contains` names rows it has, in any order, among
    /// others; an action named by an object is compared on the fields it
    /// names, spelled as the page YAML spells them.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_checks_rows_it_contains_and_an_actions_details() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let page = "title: Items
loaders:
  items: { sql: \"SELECT title, uri FROM v_work_item ORDER BY title\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";
        let given = "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First } }
  - { command: oxplow.work_item.create, input: { title: Second } }
  - { command: oxplow.work_item.create, input: { title: Third } }
when: { page: items }
";
        paged(
            root,
            page,
            &[(
                "contains",
                &format!(
                    "{given}expect:
  loaders: {{ items: {{ rows: {{ contains: [{{ title: Third }}, {{ title: First }}] }} }} }}
  nodes: [{{ id: items, actions: [{{ id: start, row: true, one_row: false }}] }}]
"
                ),
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        paged(
            root,
            page,
            &[(
                "contains",
                &format!(
                    "{given}expect:
  loaders: {{ items: {{ rows: {{ contains: [{{ title: Fourth }}] }} }} }}
  nodes: [{{ id: items, actions: [{{ id: start, row: false }}] }}]
"
                ),
            )],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("loader `items` returned") && errors.contains("at `/rows/contains/0`"),
            "{errors}"
        );
        assert!(
            errors.contains("node `items`'s actions are true at `/0/row`"),
            "{errors}"
        );
    }

    /// A page step presses one of its actions for real, as a person would:
    /// on a row (`row:` its columns), with what it answered or why it said
    /// no checked by `expect`, and the page run again shows what it did.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_presses_an_action() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let page = "title: Items
params: [{ name: uri }]
loaders:
  items: { sql: \"SELECT title, uri, state FROM v_work_item WHERE uri = :uri\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";
        let fixture = |press: &str| {
            format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: First }}, bind: first }}]
when: {{ page: items, params: {{ uri: $first.uri }} }}
expect: {{ loaders: {{ items: {{ rows: [{{ state: todo }}] }} }} }}
steps:
  - {press}
  - expect: {{ loaders: {{ items: {{ rows: [{{ state: in_progress }}] }} }} }}
"
            )
        };
        paged(
            root,
            page,
            &[(
                "pressed",
                &fixture(
                    "press: { action: start, node: items, row: { uri: $first.uri } }
    expect: { ok: true }",
                ),
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        // A press that says no is checked like any outcome.
        paged(
            root,
            page,
            &[(
                "pressed",
                "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First }, bind: first }]
when: { page: items, params: { uri: $first.uri } }
steps:
  - press: { action: start, node: items, row: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\" } }
    expect: { ok: false, refused: $any }
",
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        // One that fails where the example expects it to work names its step.
        paged(
            root,
            page,
            &[(
                "pressed",
                &fixture("press: { action: begin, node: items, row: { uri: $first.uri } }"),
            )],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/pressed.yaml:1: steps[0]: press `begin`"),
            "{errors}"
        );
    }

    /// A page step submits a form node with what a person would fill in,
    /// and the command it names runs for real.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_submits_a_form() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        paged(
            root,
            "title: New item
loaders:
  items: { sql: \"SELECT title FROM v_work_item\" }
view:
  stack:
    - { id: add, viz: form, form: { command: oxplow.work_item.create } }
    - { id: items, viz: table, from: items }
",
            &[(
                "submitted",
                "requires: [oxplow-bundled]
when: { page: items }
expect: { loaders: { items: { rows: 0 } } }
steps:
  - submit: { node: add, input: { title: Written } }
    expect: { ok: true }
  - expect: { loaders: { items: { rows: [{ title: Written }] } } }
",
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// What another extension mounts at a page's slot runs inside it: a
    /// fixture of the mounting extension runs the host page and checks the
    /// mount (`<mount id>`) and its loaders (`<mount id>/<loader>`).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_checks_what_is_mounted_at_a_slot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/host/extension.yaml",
            "manifest: 2\nname: host\nintent:\n  purpose: Hosts.\n",
        );
        write(
            root,
            ".oxplow/extensions/host/pages/items.yaml",
            "title: Items\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview:\n  stack:\n    - { id: count, viz: number, from: n }\n    - { slot: extra }\n",
        );
        write(
            root,
            ".oxplow/extensions/beta/extension.yaml",
            "manifest: 2\nname: beta\nintent:\n  purpose: Mounts.\n  examples:\n    - { name: mounted, input: { page: host/items }, expect: it }\nrequires: [host]\nui:\n  mounts:\n    - { page: extra, at: host/items.extra }\n",
        );
        write(
            root,
            ".oxplow/extensions/beta/pages/extra.yaml",
            "title: Extra\nloaders:\n  words: { sql: \"SELECT 'hello' AS word\" }\nview: { viz: table, from: words }\n",
        );
        write(
            root,
            ".oxplow/extensions/beta/fixtures/mounted.yaml",
            "when: { page: host/items }\nexpect:\n  nodes: [{ id: beta.extra }]\n  loaders: { n: { rows: 1 }, beta.extra/words: { rows: [{ word: hello }] } }\n",
        );
        let report = test_extension(root, "beta", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// `when: { sql }` reads the published datasets as a page loader would —
    /// a dataset, a decorator's labels, a metric — its rows checked like a
    /// loader's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fixture_reads_the_datasets_with_sql() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: read, input: { sql: x }, expect: rows }
datasets:
  - name: prs
    version: 1
    description: PRs.
    fields:
      - { name: title, type: \"\", doc: Its title. }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/datasets/prs.sql",
            "SELECT 'One' AS title UNION ALL SELECT 'Two'",
        );
        let fixture = |want: &str| {
            format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Three }} }}]
when: {{ sql: \"SELECT title FROM v_acme_prs UNION ALL SELECT title FROM v_work_item WHERE title = :t ORDER BY title\", params: {{ t: Three }} }}
expect: {{ rows: {want} }}
"
            )
        };
        write(
            root,
            ".oxplow/extensions/acme/fixtures/read.yaml",
            &fixture("[{ title: One }, { title: Three }, { title: Two }]"),
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        write(
            root,
            ".oxplow/extensions/acme/fixtures/read.yaml",
            &fixture("{ contains: [{ title: Four }] }"),
        );
        let errors = test_extension(root, "acme", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/read.yaml:1: page `query` loader `rows` returned"),
            "{errors}"
        );
    }

    /// `when: { command, input }` runs the command for real, as a person
    /// (or `as: agent`): what it answered, or why it said no, and the
    /// commands that ran because of it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fixture_runs_a_command_for_real() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        paged(
            root,
            BY_URI,
            &[
                (
                    "ran",
                    "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First }, bind: first }]
when: { command: oxplow.work_item.transition, input: { uri: $first.uri, to: in_progress } }
expect: { ok: true, commands: [] }
steps:
  - when: { input: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\", to: done } }
    expect: { ok: false, refused: $any }
",
                ),
                (
                    "wrong",
                    "requires: [oxplow-bundled]\nwhen: { command: oxplow.work_item.transition, input: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\", to: done } }
expect: { ok: true }
",
                ),
            ],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(!errors.contains("fixtures/ran.yaml"), "{errors}");
        assert!(
            errors.contains("fixtures/wrong.yaml:1: command `oxplow.work_item.transition` gave"),
            "{errors}"
        );
    }

    /// A materialized dataset is refilled before a fixture reads it, as the
    /// app's change loop refills it: what a step made is in it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_materialized_dataset_is_current_when_a_fixture_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: Titles.
  examples:
    - { name: stored, input: { sql: x }, expect: rows }
datasets:
  - name: titles
    version: 1
    key: [id]
    materialize: on_change
    description: Titles.
    fields:
      - { name: id, type: TEXT, doc: The item. }
      - { name: title, type: TEXT, doc: Its title. }
",
        );
        write(
            root,
            ".oxplow/extensions/acme/datasets/titles.sql",
            "SELECT uri AS id, title FROM ref('work_item')",
        );
        write(
            root,
            ".oxplow/extensions/acme/fixtures/stored.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { sql: \"SELECT title FROM v_acme_titles\" }
expect: { rows: [{ title: Fix it }] }
steps:
  - given: [{ command: oxplow.work_item.create, input: { title: Another } }]
  - expect: { rows: 2 }
",
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// `when: { advisory }` runs one of the extension's advisories for a
    /// thread as the app would: its `lines` are what the agent is told.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fixture_runs_an_advisory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: Greets.
  examples:
    - { name: greeted, input: { advisory: hello }, expect: a greeting per item }
permissions: [agent.write]
advisories:
  - id: hello
    on: prompt
    once_per: turn
    heading: \"# Hello\"
    query: \"SELECT uri AS key, 'Hello ' || title AS message FROM v_work_item ORDER BY title\"
",
        );
        let fixture = |want: &str| {
            format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Fix it }} }}]
when: {{ advisory: hello }}
expect: {{ lines: {want} }}
"
            )
        };
        write(
            root,
            ".oxplow/extensions/acme/fixtures/greeted.yaml",
            &fixture("{ contains: [Hello Fix it] }"),
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        write(
            root,
            ".oxplow/extensions/acme/fixtures/greeted.yaml",
            &fixture("[]"),
        );
        let errors = test_extension(root, "acme", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/greeted.yaml:1: advisory `hello` said"),
            "{errors}"
        );
    }

    /// A page's `ui:` step runs its custom component's own code in the
    /// app's webview against the test's oxplow: what it draws, what it asks
    /// the shell (a title, actions) and what it invokes, through clicks,
    /// typing and a nav-bar press — and a wrong expectation names the step
    /// and shows the frame.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_ui_step_runs_the_component_in_the_apps_webview() {
        assert!(
            crate::component_harness::shell().is_some(),
            "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/cards/extension.yaml",
            "manifest: 2
name: cards
intent:
  purpose: Cards.
  examples:
    - { name: drawn, input: { page: board }, expect: cards }
permissions: [commands:oxplow.work_item.create]
custom_components:
  - { id: cards, title: Cards, commands: [oxplow.work_item.create] }
",
        );
        write(
            root,
            ".oxplow/extensions/cards/pages/board.yaml",
            "title: Board\nloaders:\n  items: { sql: \"SELECT title FROM v_work_item ORDER BY title\" }\nview: { component: { id: cards, props: { heading: Work } } }\n",
        );
        write(
            root,
            ".oxplow/extensions/cards/components/cards/index.html",
            "<!doctype html><html><head><title>cards</title></head><body>\
             <h1 id=\"heading\"></h1><ul id=\"list\"></ul><input id=\"t\"><button id=\"go\">Go</button>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
        write(
            root,
            ".oxplow/extensions/cards/components/cards/app.js",
            "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  var draw = function (data) {
    list.innerHTML = '';
    data.items.rows.forEach(function (r) {
      var li = document.createElement('li');
      li.textContent = r[0];
      list.appendChild(li);
    });
  };
  document.getElementById('heading').textContent = c.props.heading;
  draw(c.data);
  c.onUpdate(function (u) { draw(u.data); });
  c.setTitle('Cards: ' + c.data.items.rows.length);
  c.setActions([{ id: 'add', label: 'Add' }]);
  c.onAction(function (id) { if (id === 'add') c.invoke('oxplow.work_item.create', { title: 'From the bar' }); });
  document.getElementById('go').addEventListener('click', function () {
    c.invoke('oxplow.work_item.create', { title: document.getElementById('t').value });
  });
});
",
        );
        let fixture = |last: u32| {
            format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: First }} }}]
when: {{ page: board }}
steps:
  - ui:
      - expect: {{ count: {{ li: 1 }}, text: {{ \"#heading\": Work }}, title: \"Cards: 1\", actions: [add] }}
      - fill: {{ \"#t\": Second }}
      - click: \"#go\"
      - expect: {{ count: {{ li: 2 }}, invoked: [{{ command: oxplow.work_item.create, ok: true }}] }}
      - press: add
      - expect: {{ count: {{ li: {last} }} }}
"
            )
        };
        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            &fixture(3),
        );
        let report = test_extension(root, "cards", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
        if report.warnings.iter().any(|w| w.contains("aren't run")) {
            // The display is locked or asleep: the window draws nothing, so the
            // steps were not run (`a_window_that_draws_nothing_…` covers that).
            return;
        }

        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            &fixture(5),
        );
        let errors = test_extension(root, "cards", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("fixtures/drawn.yaml:1: steps[0]: ui[5]:")
                && errors.contains("<li>From the bar</li>"),
            "{errors}"
        );
    }

    /// A window that draws no frames (the display locked or asleep) leaves
    /// a fixture's `ui` steps, and the steps after them, not run, with why,
    /// never failed. The shell is
    /// a stand-in that says so, as the harness's page does.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_that_draws_nothing_leaves_the_ui_steps_unrun() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let shell = root.join("fake-shell");
        std::fs::write(
            &shell,
            r#"#!/bin/sh
base="${2%/harness/}"
post() { curl -s -X POST -H 'content-type: application/json' -d "$2" "$base$1"; }
post /harness/record '{"kind":"drawing","value":false}' >/dev/null
while :; do case "$(post /harness/next '{"result":null}')" in *done*) exit 0;; esac; done
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("OXPLOW_SHELL", &shell);
        component_ext(root, "oxplow.connect();");
        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            "when: { page: board }\nsteps:\n  - ui:\n      - expect: { count: { li: 99 } }\n  \
             - when: { sql: SELECT 1 AS n }\n    expect: { rows: [{ n: 2 }] }\n",
        );
        let report = test_extension(root, "cards", false).await.unwrap();
        // What follows the `ui` steps reads what they did: it isn't run either.
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("the steps after them, aren't run") && w.contains("display")),
            "{:?}",
            report.warnings
        );
    }

    /// A component page of one custom component, for the `ui` tests: its
    /// `app.js` runs on `oxplow.connect()`'s component.
    fn component_ext(root: &Path, app: &str) {
        write(
            root,
            ".oxplow/extensions/cards/extension.yaml",
            "manifest: 2
name: cards
intent:
  purpose: Cards.
  examples:
    - { name: drawn, input: { page: board }, expect: cards }
permissions: [commands:oxplow.work_item.create]
custom_components:
  - { id: cards, title: Cards, commands: [oxplow.work_item.create] }
",
        );
        write(
            root,
            ".oxplow/extensions/cards/pages/board.yaml",
            "title: Board
params:
  - { name: kind, role: filter, default: all }
loaders:
  items: { sql: \"SELECT title FROM v_work_item WHERE :kind = 'all' OR title LIKE :kind || '%' ORDER BY title\" }
view: { component: { id: cards } }
",
        );
        write(
            root,
            ".oxplow/extensions/cards/components/cards/index.html",
            "<!doctype html><html><body><form id=\"f\"><input id=\"t\"><span id=\"err\"></span></form><ul id=\"list\"></ul>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
        write(
            root,
            ".oxplow/extensions/cards/components/cards/app.js",
            app,
        );
    }

    /// Enter in a field of a form submits it, as a browser does: the form's
    /// own `submit` handler runs, and one that stops the keydown keeps it
    /// from submitting.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_ui_enter_submits_the_form_a_field_is_in() {
        assert!(
            crate::component_harness::shell().is_some(),
            "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        component_ext(
            root,
            "oxplow.connect().then(function (c) {
  var f = document.getElementById('f'), t = document.getElementById('t');
  f.addEventListener('submit', function (e) {
    e.preventDefault();
    if (!t.value) { document.getElementById('err').textContent = 'Required'; return; }
    document.getElementById('err').textContent = '';
    c.invoke('oxplow.work_item.create', { title: t.value });
  });
});
",
        );
        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            "requires: [oxplow-bundled]\nwhen: { page: board }
steps:
  - ui:
      - key: { on: \"#t\", key: Enter }
      - expect: { text: { \"#err\": Required }, invoked: [] }
      - fill: { \"#t\": Second }
      - key: { on: \"#t\", key: Enter }
      - expect: { text: { \"#err\": \"\" }, invoked: [{ command: oxplow.work_item.create, ok: true }] }
",
        );
        let report = test_extension(root, "cards", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// A component's view params re-run its page, as the app's tab does: a
    /// filter it sets is the page's param for the next run, and Back
    /// (`view_params` with `null`) drops it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_ui_view_param_runs_the_page_again_with_it() {
        assert!(
            crate::component_harness::shell().is_some(),
            "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        component_ext(
            root,
            "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  var draw = function (data) {
    list.innerHTML = '';
    data.items.rows.forEach(function (r) {
      var li = document.createElement('li');
      li.textContent = r[0];
      list.appendChild(li);
    });
  };
  draw(c.data);
  c.onUpdate(function (u) { draw(u.data); });
  var b = document.createElement('button');
  b.id = 'only-b';
  b.textContent = 'B';
  b.addEventListener('click', function () { c.setViewParams({ kind: 'B' }); });
  document.body.appendChild(b);
});
",
        );
        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: Alpha } }
  - { command: oxplow.work_item.create, input: { title: Beta } }
when: { page: board }
steps:
  - ui:
      - expect: { count: { li: 2 } }
      - click: \"#only-b\"
      - expect: { count: { li: 1 }, text: { li: Beta }, view_params: { kind: B } }
      - view_params: { kind: null }
      - expect: { count: { li: 2 } }
",
        );
        let report = test_extension(root, "cards", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// `shown` is the view the core drew: the spec when it opened, `null`
    /// when the core refused it or the component cleared it — however fast
    /// the component asks.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_ui_shown_is_the_last_view_the_core_drew() {
        assert!(
            crate::component_harness::shell().is_some(),
            "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        component_ext(
            root,
            "oxplow.connect().then(function (c) {
  var spec = function (sql) {
    return 'title: Out\\nloaders:\\n  rows: { sql: \"' + sql + '\" }\\nview: { from: rows, viz: table }\\n';
  };
  var button = function (id, run) {
    var b = document.createElement('button');
    b.id = id;
    b.addEventListener('click', run);
    document.body.appendChild(b);
  };
  button('bad', function () { c.showView(spec('SELECT * FROM no_such_model')).catch(function () {}); });
  button('good', function () { c.showView(spec('SELECT 1 AS n')).catch(function () {}); });
  button('clear', function () { c.showView(null); });
  // Three asks at once: the last the core drew is the one that counts.
  button('burst', function () {
    c.showView(spec('SELECT 1 AS n')).catch(function () {});
    c.showView(spec('SELECT * FROM no_such_model')).catch(function () {});
    c.showView(spec('SELECT 2 AS n')).catch(function () {});
  });
});
",
        );
        write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            r##"when: { page: board }
steps:
  - ui:
      - click: "#bad"
      - expect: { shown: null }
      - click: "#good"
      - expect: { shown: "title: Out\nloaders:\n  rows: { sql: \"SELECT 1 AS n\" }\nview: { from: rows, viz: table }\n" }
      - click: "#bad"
      - expect: { shown: null }
      - click: "#good"
      - click: "#clear"
      - expect: { shown: null }
      - click: "#burst"
      - expect: { shown: "title: Out\nloaders:\n  rows: { sql: \"SELECT 2 AS n\" }\nview: { from: rows, viz: table }\n" }
"##,
        );
        let report = test_extension(root, "cards", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// A `ui` step drives a component inside what the page embeds — here
    /// the page another extension stands in with for a replaceable node —
    /// the first one found, or the one `node: <embed>/<node>` names.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_ui_step_drives_a_component_a_replacement_stands_in_with() {
        assert!(
            crate::component_harness::shell().is_some(),
            "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".oxplow/extensions/host/extension.yaml",
            "manifest: 2\nname: host\nintent:\n  purpose: Hosts.\n",
        );
        write(
            root,
            ".oxplow/extensions/host/pages/board.yaml",
            "title: Board\nparams: [{ name: scope, default: all }, { name: thread_id }]\nloaders:\n  own: { sql: \"SELECT 1 AS n\" }\nview: { id: cards, viz: table, from: own, replaceable: work_item.board }\n",
        );
        write(
            root,
            ".oxplow/extensions/lanes/extension.yaml",
            "manifest: 2
name: lanes
intent:
  purpose: Lanes.
  examples:
    - { name: drawn, input: { page: host/board }, expect: lanes }
requires: [host]
custom_components:
  - { id: lanes, title: Lanes }
ui:
  mounts:
    - { page: board, at: work_item.board }
",
        );
        write(
            root,
            ".oxplow/extensions/lanes/pages/board.yaml",
            "title: Lanes\nparams: [{ name: scope }, { name: thread_id }]\nloaders:\n  items: { sql: \"SELECT title FROM v_work_item ORDER BY title\" }\nview: { id: board, component: { id: lanes } }\n",
        );
        write(
            root,
            ".oxplow/extensions/lanes/components/lanes/index.html",
            "<!doctype html><html><head><title>lanes</title></head><body><ul id=\"list\"></ul>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
        write(
            root,
            ".oxplow/extensions/lanes/components/lanes/app.js",
            "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  c.data.items.rows.forEach(function (r) {
    var li = document.createElement('li');
    li.textContent = r[0];
    list.appendChild(li);
  });
});
",
        );
        write(
            root,
            ".oxplow/extensions/lanes/fixtures/drawn.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First } }]
when: { page: host/board }
expect: { nodes: [{ id: cards/board }] }
steps:
  - ui:
      - expect: { text: { li: First } }
  - node: cards/board
    ui:
      - expect: { count: { li: 1 } }
",
        );
        let report = test_extension(root, "lanes", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// `rows: []` is no rows, like `rows: 0`.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_row_list_is_no_rows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        paged(
            root,
            "title: Items\nloaders:\n  items: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: items }\n",
            &[("empty", "when: { page: items }\nexpect: { loaders: { items: { rows: [] } } }\n")],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(errors.contains("loader `items` returned"), "{errors}");
    }

    /// Each example with `given` runs on an oxplow of its own: what one
    /// made, the next doesn't see.
    #[tokio::test(flavor = "multi_thread")]
    async fn page_fixtures_dont_see_each_others_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let fixture = "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Mine } }]
when: { page: items }
expect: { loaders: { items: { rows: 1 } } }
";
        paged(
            root,
            "title: Items
loaders:
  items: { sql: \"SELECT title FROM v_work_item WHERE title = 'Mine'\" }
view: { from: items, viz: table }
",
            &[("one", fixture), ("two", fixture)],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }

    /// `confirmed: true` runs a step that asks for confirmation; `view:`
    /// gives the run its stream and thread; `expect.alert` checks the alert.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_fixture_confirms_views_from_a_thread_and_checks_the_alert() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let page = "title: Items
params: [{ name: thread_id }]
loaders:
  items: { sql: \"SELECT :thread_id AS thread\" }
view: { from: items, viz: table }
alert: { min_rows: 1, label: Configured }
";
        paged(
            root,
            page,
            &[(
                "viewed",
                "view: { stream: str1, thread: thr1 }
given:
  - { command: oxplow.ai.set_provider, input: { id: spare, kind: anthropic, key: sk-ant-test } }
  - { command: oxplow.ai.remove_provider, input: { id: spare }, confirmed: true }
when: { page: items }
expect:
  loaders: { items: { rows: [{ thread: 1 }] } }
  alert: { firing: true, count: 1, message: $any }
",
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());

        paged(
            root,
            page,
            &[(
                "viewed",
                "given:
  - { command: oxplow.ai.set_provider, input: { id: spare, kind: anthropic, key: sk-ant-test } }
  - { command: oxplow.ai.remove_provider, input: { id: spare } }
when: { page: items }
",
            )],
        );
        let errors = test_extension(root, "paged", false)
            .await
            .unwrap()
            .errors
            .join("\n");
        assert!(
            errors.contains("`given[1]` `oxplow.ai.remove_provider` failed")
                && errors.contains("confirmed: true"),
            "{errors}"
        );
    }

    /// The JUnit form has a case per thing that ran, a failure holding the
    /// findings it added.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_report_renders_as_junit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pager(
            root,
            LIST,
            "{}",
            "{ loaders: { items: { rows: [{ title: Something else }] } } }",
        );
        let report = test_extension(root, "pager", false).await.unwrap();
        let xml = render(&report, Format::Junit);
        assert!(
            xml.starts_with("<testsuite name=\"pager\" tests=\""),
            "{xml}"
        );
        assert!(
            xml.contains("<testcase name=\"check\" classname=\"pager\""),
            "{xml}"
        );
        assert!(
            xml.contains("<testcase name=\"example created\" classname=\"pager\"")
                && xml.contains("<failure message=\"")
                && xml.contains("Something else"),
            "{xml}"
        );
        assert!(xml.contains("<testcase name=\"answers items\""), "{xml}");
    }

    /// What a step's command sets off — a reactor hearing its event —
    /// has happened before the next step and the run: switching the work
    /// list off through its command empties the work-item interface.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_steps_reactions_land_before_the_page_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        paged(
            root,
            "title: Items
loaders:
  items: { sql: \"SELECT title FROM v_work_item\" }
view: { from: items, viz: table }
",
            &[(
                "switched",
                "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: Filed } }
when: { page: items }
expect: { loaders: { items: { rows: [{ title: Filed }] } } }
steps:
  - given: [{ command: oxplow.capability.choose, input: { capability: work_items, provider: none }, confirmed: true }]
  - expect: { loaders: { items: { rows: 0 } } }
",
            )],
        );
        let report = test_extension(root, "paged", false).await.unwrap();
        assert_eq!(report.errors, Vec::<String>::new());
    }
}
