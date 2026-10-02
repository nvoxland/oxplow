//! `oxplow plugin test` (P5.D5, P7.C6; `.context/extensions.md` "The
//! SDK"; `.context/providers.md` "The conformance kit"). Everything runs
//! on a throwaway oxplow — an in-memory one over a copy of the project's
//! `oxplow/extensions/` ([`Host`]) — so it never touches the project's
//! data: `check` (against its real command registry, so `commands:`
//! examples dry-run), then each intent example whose fixture names a lens
//! (`input: { lens, params? }`, `expect: { columns?, rows }`), a derived
//! collector (`input: { collector, rows? }`, `expect: { entities: { <name>:
//! n } }`; an exec collector's isn't run — it needs a person's approval),
//! or one of its own commands (`input: { command, input, rows? }`,
//! `expect: { commands: [names] }` or `{ refuses }`, dry-run),
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

/// What `plugin test` found.
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
}

/// Run every test of extension `name` under `root`. `bless` writes the
/// golden transcripts instead of comparing them.
pub async fn test_extension(root: &Path, name: &str, bless: bool) -> Result<TestReport, SdkError> {
    let host = Host::start(root).await.map_err(SdkError::Invalid)?;
    let checked = crate::check(
        &host.root,
        name,
        &host.svc.extension_catalog,
        Some(&host.svc.sql),
        Some(host.svc.commands.as_ref()),
    )
    .await?;
    let mut report = TestReport {
        name: name.to_string(),
        ok: false,
        errors: checked.errors.clone(),
        warnings: checked.warnings.clone(),
        blessed: Vec::new(),
        ran: vec!["check".into()],
    };
    let ext = checked.extension;
    if report.errors.is_empty() {
        examples(&host, &ext, &mut report).await;
        for spec in &ext.providers {
            report.ran.push(format!("provider {}", spec.id));
            test_provider(root, &ext, spec, bless, &mut report).await;
        }
        questions(&host, &ext, &mut report).await;
    }
    report.ok = report.errors.is_empty();
    Ok(report)
}

/// An intent example's fixture (`fixtures/<name>.yaml`): `None` when it
/// has none; the parsed document, or why it isn't one.
fn example_fixture(
    dir: &Path,
    rel: &str,
    example: &str,
) -> Option<(String, Result<Value, String>)> {
    let file = format!("fixtures/{example}.yaml");
    let shown = format!("{rel}/{file}");
    let text = std::fs::read_to_string(dir.join(&file)).ok()?;
    let doc = serde_yaml::from_str::<Value>(&text)
        .map_err(|e| format!("{shown}:1: not YAML: {e} — fix: correct it"));
    Some((shown, doc))
}

/// The intent examples a throwaway oxplow runs: a lens's, a derived
/// collector's. A provider's (`input: { command, input }`) run in its
/// session.
async fn examples(host: &Host, ext: &Extension, report: &mut TestReport) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = host.root.join(&rel);
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    for example in examples {
        let Some((shown, doc)) = example_fixture(&dir, &rel, &example.name) else {
            report.warnings.push(format!(
                "{rel}/fixtures/{}.yaml:1: example `{}` has no fixture, so it isn't run — fix: \
                 write its `input` (`{{ lens, params? }}`, `{{ collector, rows? }}`, or a \
                 provider's `{{ command, input }}`) and `expect`",
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
        if let Some(slug) = ex.input.get("lens").and_then(Value::as_str) {
            report.ran.push(format!("example {}", ex.name));
            lens_example(host, ext, &ex, slug, report).await;
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
                "{shown}:1: example `{}`'s `input` names no lens, collector or provider command \
                 — fix: `{{ lens: <slug>, params? }}`, `{{ collector: <id>, rows? }}`, or (with a \
                 provider) `{{ command, input }}`",
                example.name
            ));
        }
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

/// Dry-run one of the extension's own `commands:` on the fixture's
/// `input` (and `rows`): what it composes, checked against the throwaway's
/// registry, against `expect` — `{ commands: [names] }`, or `{ refuses:
/// <part of the reason> }`. Nothing runs.
async fn command_example(
    host: &Host,
    ex: &Example<'_>,
    cmd: &oxplow_app::extension_commands::ExtensionCommand,
    report: &mut TestReport,
) {
    use oxplow_app::extension_commands::{call_names, dry_run, Composed};
    let shown = ex.shown;
    let rows = ex
        .input
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| r.to_vec());
    let input = ex.input.get("input").cloned().unwrap_or_else(|| json!({}));
    let decided = dry_run(&host.svc.sql, cmd, &input, rows, host.svc.commands.as_ref()).await;
    let problem = match (decided, ex.expect.get("refuses").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Composed::Refused(why)), Some(want)) if why.contains(want) => None,
        (Ok(Composed::Refused(why)), _) => Some(format!("refused ({why})")),
        (Ok(Composed::Run { calls, .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to refuse ({want})",
            call_names(&calls).join(", ")
        )),
        (Ok(Composed::Run { calls, .. }), None) => first_mismatch(
            &ex.expect,
            &json!({ "commands": call_names(&calls) }),
        )
        .map(|(path, want, got)| format!("composed {got} at `{path}`, the example expects {want}")),
    };
    if let Some(p) = problem {
        report.errors.push(format!(
            "{shown}:1: command `{}` {p} — fix: its script, or the example's `expect` \
             (`{{ commands: [names] }}` or `{{ refuses: <part of the reason> }}`)",
            cmd.name
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
    if !spec.runtime.is_derived() {
        report.warnings.push(format!(
            "{shown}:1: example `{example}` runs exec collector `{id}`, which `plugin test` \
             doesn't run (it needs a person's approval) — fix: none; run it from Settings → Data"
        ));
        return;
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
    let got = json!({ "entities": entities });
    if let Some((path, want, got)) = first_mismatch(expect, &got) {
        report.errors.push(format!(
            "{shown}:1: collector `{id}` returned {got} at `{path}`, the example expects {want} — \
             fix: the script, or the example's `expect` (`{{ entities: {{ <name>: n | $any }} }}`)"
        ));
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
/// the environment it runs in (the author's own).
fn env_credentials(spec: &ProviderSpec) -> std::collections::BTreeMap<String, String> {
    spec.credentials
        .iter()
        .filter_map(|n| std::env::var(n).ok().map(|v| (n.clone(), v)))
        .collect()
}

async fn test_provider(
    root: &Path,
    ext: &Extension,
    spec: &ProviderSpec,
    bless: bool,
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
        ext_dir: dir.clone(),
        spec: spec.clone(),
        declared: declared.clone(),
        credentials: env_credentials(spec),
        host_env: Arc::new(|n| std::env::var(n).ok()),
    };
    let client = match ReferenceClient::start(&launch).await {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!(
                "{manifest}:1: {e} — fix: make `{}` an executable that speaks the provider protocol",
                spec.entry
            ));
            return;
        }
    };
    let under_test = UnderTest {
        ext,
        spec,
        declared: &declared,
        dir: &dir,
        rel: &rel,
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
    suite(root, ext, spec, config, report).await;
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
                credentials: env_credentials(spec).into_keys().collect(),
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
        let Some((shown, Ok(doc))) = example_fixture(dir, rel, &example.name) else {
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
                },
            )
            .await
        {
            Ok(out) => {
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
            "{shown}:1: no golden transcript — fix: run `oxplow plugin test <name> --bless` and \
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
async fn suite(
    root: &Path,
    ext: &Extension,
    spec: &ProviderSpec,
    config: Value,
    report: &mut TestReport,
) {
    let manifest = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
    report.ran.push(format!("{} suite", spec.capability));
    let run = async {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let target = tmp.path().join(EXTENSIONS_DIR).join(&ext.name);
        crate::throwaway::copy_dir(&root.join(&ext.path), &target).map_err(|e| e.to_string())?;
        oxplow_app::vcs::GitProvider
            .init_repository(tmp.path())
            .await
            .map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::in_memory(tmp.path()).map_err(|e| e.to_string())?;
        let hosted = svc
            .extension_catalog
            .get(tmp.path())
            .iter()
            .find(|e| e.name == ext.name)
            .cloned()
            .ok_or("the copied extension didn't load")?;
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
        for (name, value) in env_credentials(spec) {
            svc.secrets
                .set(
                    &oxplow_app::collector_runner::credential_account(&project, &ext.name, &name),
                    &value,
                )
                .map_err(|e| e.to_string())?;
        }
        svc.providers
            .enable(&hosted, spec, config)
            .await
            .map_err(|e| e.to_string())?;
        let findings = match spec.capability.as_str() {
            providers::spec::WORK_ITEMS => {
                let provider = svc.work_items.get(&spec.id).map_err(|e| e.to_string())?;
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
            other => return Err(format!("no conformance suite for `{other}`")),
        };
        svc.providers.stop(&spec.approval_name(&ext.name)).await;
        Ok::<_, String>(findings)
    };
    match run.await {
        Ok(findings) => {
            for f in findings {
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
    input: \"SELECT id FROM v_task\"
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
  - name: note
    summary: Note a work item.
    input_schema: {{ type: object, required: [ref], properties: {{ ref: {{ type: string }} }} }}
    entry: handlers/note.star
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
            "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.comment\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"body\": \"noted\"}}]}\n",
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
            "Read `v_tally_labelled`; note a work item with `tally.note`.\n",
        );
        write(
            root,
            "oxplow/extensions/tally/questions.yaml",
            "- question: Which things are labelled?\n  skill: README.md\n  reaches:\n    sql: SELECT id, label FROM v_tally_labelled\n  shape: { columns: [id, label] }\n- question: Note a work item.\n  skill: README.md\n  reaches:\n    command: tally.note\n    input: { ref: \"work_item:oxplow:tsk1\" }\n",
        );
    }

    /// P7.C6: `plugin test` runs every kind's examples on a throwaway
    /// oxplow — a lens's rows, a starlark collector's typed entities from
    /// its fixture rows, a command's composition against the real
    /// registry — and its questions may name its own commands. An exec
    /// collector's example isn't run (it needs a person's approval).
    #[tokio::test(flavor = "multi_thread")]
    async fn plugin_test_runs_every_kinds_examples() {
        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 2 } }",
            "[work_item.comment]",
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
            warnings.contains("fixtures/shell.yaml:1: example `shell` runs exec collector `shell`"),
            "{warnings}"
        );

        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 3 } }",
            "[work_item.comment]",
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
            "[work_item.transition]",
        );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        let errors = report.errors.join("\n");
        assert!(
            errors.contains("example `happy`: composed [work_item.comment] but `expect_commands` is [work_item.transition]"),
            "{errors}"
        );
    }
}
