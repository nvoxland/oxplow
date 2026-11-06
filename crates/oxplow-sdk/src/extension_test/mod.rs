//! `oxplow extension test` (P5.D5, P7.C6; `.context/extensions-sdk.md`
//! "The SDK"; `.context/providers.md` "The conformance kit"). Everything runs
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
use oxplow_app::pages::schema::PageKind;
use oxplow_app::providers::host::{self, Launch};
use oxplow_app::providers::{self, ProviderSpec};
use oxplow_domain::diagnostic::Diagnostic;
use oxplow_domain::Actor;
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, InitializeResult, InvokeParams, InvokeResult,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::conformance::{first_mismatch, normalize, ReferenceClient, ANY};
use crate::throwaway::Host;
use crate::SdkError;

mod collectors;
mod examples;
mod fixture;
mod observe;
mod page;
mod report;
mod runner;
mod steps;
mod suites;
#[cfg(test)]
mod tests;
mod ui;

use collectors::*;
use examples::*;
use fixture::*;
use observe::*;
use page::*;
use report::*;
pub use report::{render, CaseOutcome, TestCase, TestReport};
use runner::*;
use steps::*;
use suites::*;

/// The capability a policy script implements.
const EFFORT_POLICY: &str = providers::effort_policy::EffortPolicyHost::CAPABILITY;

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
    run(root, name, bless, env, None, false).await
}

/// [`test_extension`] with `require_ui`: a fixture's `ui` steps that can't
/// draw (no app build, a window that draws nothing) are errors, not skips.
pub async fn test_extension_with(
    root: &Path,
    name: &str,
    bless: bool,
    require_ui: bool,
) -> Result<TestReport, SdkError> {
    run(root, name, bless, host::process_env(), None, require_ui).await
}

/// The check, then only the intent examples named in `only` (their
/// fixtures' file stems): the quick run while working on a few of them.
/// Nothing else runs — no providers, suites or page answers.
pub async fn test_extension_only(
    root: &Path,
    name: &str,
    only: &[String],
    require_ui: bool,
) -> Result<TestReport, SdkError> {
    run(
        root,
        name,
        false,
        host::process_env(),
        Some(only),
        require_ui,
    )
    .await
}

async fn run(
    root: &Path,
    name: &str,
    bless: bool,
    env: host::HostEnv,
    only: Option<&[String]>,
    require_ui: bool,
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
        skipped: Vec::new(),
        root: oxplow_app::extension::extensions::source_root(root, name)
            .unwrap_or_else(|| checked.root.clone()),
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
        require_ui,
    };
    // What activation held back: this extension's is an error (unless
    // check already says it), another's a warning — it's beside it.
    for (owner, problem) in &host.problems {
        if owner == name {
            // A deploy that fails because the extension doesn't load says
            // what check already says, with the copy's path.
            let derived = problem.code == "activation.deploy_failed" && !report.errors.is_empty();
            if !derived && !report.errors.iter().any(|e| e.same_as(problem)) {
                let mut found = problem.clone();
                found.severity = oxplow_domain::diagnostic::Severity::Error;
                report.errors.push(found);
            }
        } else {
            report.warnings.push(
                Diagnostic::warning(
                    "activation.other_extension",
                    format!(
                        "another extension here, `{owner}`, doesn't activate: {}",
                        problem.message
                    ),
                )
                .contribution(owner.clone()),
            );
        }
    }
    let ext = checked.extension;
    report.contributes = ext.contributions.iter().map(|d| d.id.to_string()).collect();
    report.unsupported = unsupported(&ext);
    report.finish_case(
        "check".into(),
        Mark {
            errors: 0,
            skipped: 0,
            started,
        },
    );
    if let Some(only) = only {
        let there: Vec<String> = ext
            .intent
            .as_ref()
            .map(|i| i.examples.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default();
        for unknown in only.iter().filter(|n| !there.contains(n)) {
            report.errors.push(
                error(
                    "extension.yaml",
                    None,
                    "fixture.unknown",
                    format!("no example `{unknown}`"),
                )
                .fix(format!("one of {}", there.join(", "))),
            );
        }
        if report.errors.is_empty() {
            examples(&host, &ext, Some(only), &mut report).await;
        }
    } else if report.errors.is_empty() {
        examples(&host, &ext, None, &mut report).await;
        for spec in &ext.providers {
            let mark = report.mark();
            report.ran.push(format!("provider {}", spec.id));
            test_provider(&host, root, &ext, spec, bless, &env, &mut report).await;
            report.finish_case(format!("provider {}", spec.id), mark);
        }
        for decl in ext
            .implementations
            .iter()
            .filter(|d| d.capability == EFFORT_POLICY && d.script.is_some())
        {
            let mark = report.mark();
            policy_suite(&host, &ext, decl, &mut report).await;
            report.finish_case(format!("policy {}", decl.id), mark);
        }
        page_answers(&host, &ext, &mut report).await;
        let mark = report.mark();
        questions(&host, &ext, &mut report).await;
        incremental_datasets(&host, &ext, &mut report).await;
        if report.errors.len() > mark.errors {
            report.finish_case("questions and datasets".into(), mark);
        }
    }
    report.ok = report.errors.is_empty();
    Ok(report)
}

/// The intent examples a throwaway oxplow runs: a page's, a derived
/// collector's. A provider's (`when: { command, input }`) run in its
/// session. `only`: just the examples it names.
async fn examples(
    shared: &Arc<Host>,
    ext: &Extension,
    only: Option<&[String]>,
    report: &mut TestReport,
) {
    // Read as the extension's files are everywhere: a bundled extension's
    // fixtures are embedded with it, not on disk.
    let read = |file: &str| {
        oxplow_app::extension::extensions::read_extension_file(&shared.root, &ext.path, file)
    };
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| only.is_none_or(|only| only.contains(&e.name)));
    // Examples that make data, each on an oxplow of its own, run side by
    // side, a few at a time; their findings join the report in the
    // examples' order.
    let slots = Arc::new(tokio::sync::Semaphore::new(side_by_side()));
    let mut own = Vec::new();
    let require_ui = report.require_ui;
    // The oxplows fixtures that turn on more extensions start from, by
    // what they turn on.
    let mut required: std::collections::BTreeMap<Vec<String>, Arc<Host>> = Default::default();
    let manifest_text = read("extension.yaml").unwrap_or_default();
    let report_root = report.root.clone();
    for example in examples {
        let Some((shown, text, doc)) = example_fixture(&read, &example.name) else {
            report.warnings.push(
                warning(
                    "extension.yaml",
                    oxplow_app::extension::extensions::manifest_v2::line_under(
                        &manifest_text,
                        "intent",
                        &example.name,
                    ),
                    "fixture.missing",
                    format!(
                        "intent example `{}` has no fixture, so it isn't run",
                        example.name
                    ),
                )
                .fix(format!(
                    "add fixtures/{}.yaml: `given?`, `expect` and a `when` that is one of {}",
                    example.name,
                    kinds_hint()
                )),
            );
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
            report.errors.push(error(&shown, None, "fixture.unknown_key", format!("`{key}` isn't a fixture key")).fix("a fixture is `given` (the steps first), `when` (what it does), `expect`, `steps` (a scenario), `view`, `model`, `temp` (files on `oxplow:temp`) and `requires` (more extensions to turn on)"));
            continue;
        }
        let fixture = Fixture::of(example.name.clone(), shown, text, &doc);
        let ex = fixture.example();
        if let Some(why) = withheld_target(ext, &ex.when) {
            let mark = report.mark();
            report.skipped.push(warning(
                &fixture.shown,
                None,
                "example.not_run",
                format!("example `{}` isn't run: {why}", fixture.name),
            ));
            report.finish_case(format!("example {}", fixture.name), mark);
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
                report.errors.push(error(&fixture.shown, None, "fixture.unknown", format!("`requires` names `{unknown}`, which isn't an extension here")).fix("an extension oxplow ships (`oxplow-bundled`) or one in `.oxplow/extensions/`"));
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
                            report.errors.push(error(
                                &fixture.shown,
                                None,
                                "example.host_failed",
                                format!("an oxplow with {}: {e}", fixture.requires.join(", ")),
                            ));
                            continue;
                        }
                    }
                }
            };
            let (ext, slots, report_root) = (ext.clone(), slots.clone(), report_root.clone());
            own.push(tokio::spawn(async move {
                let _slot = slots.acquire_owned().await;
                let mut part = TestReport::part(&ext.name, &report_root);
                part.require_ui = require_ui;
                match base.fork().await {
                    Ok(host) => run_example(&host, &ext, &fixture.example(), &mut part).await,
                    Err(e) => part.errors.push(error(
                        &fixture.shown,
                        None,
                        "example.host_failed",
                        format!("an oxplow of its own: {e}"),
                    )),
                }
                part
            }));
            continue;
        }
        // An example that makes data — steps, rows, a job, a logged event,
        // a component's filed items — runs on an oxplow of its own, so no
        // other example sees what it made or what reacted to it.
        if makes_data(&ex) {
            let (shared, ext, slots, report_root) = (
                shared.clone(),
                ext.clone(),
                slots.clone(),
                report_root.clone(),
            );
            own.push(tokio::spawn(async move {
                let _slot = slots.acquire_owned().await;
                let mut part = TestReport::part(&ext.name, &report_root);
                part.require_ui = require_ui;
                match shared.fork().await {
                    Ok(host) => run_example(&host, &ext, &fixture.example(), &mut part).await,
                    Err(e) => part.errors.push(error(
                        &fixture.shown,
                        None,
                        "example.host_failed",
                        format!("an oxplow of its own: {e}"),
                    )),
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
            Err(e) => report.errors.push(
                Diagnostic::error(
                    "example.run_stopped",
                    format!("an example's run stopped: {e}"),
                )
                .contribution(ext.name.clone()),
            ),
        }
    }
}

/// How many examples run side by side: one per core, at most eight — each
/// boots an oxplow of its own, and nearly all of its time is CPU.
fn side_by_side() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8)
}
