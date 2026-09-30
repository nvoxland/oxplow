//! `oxplow plugin test` (P5.D5, `.context/extensions.md` "The SDK";
//! `.context/providers.md` "The conformance kit"): `check`, then for
//! each declared provider —
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

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_app::extensions::{Extension, EXTENSIONS_DIR};
use oxplow_app::providers::host::{self, Launch};
use oxplow_app::providers::{self, ProviderSpec};
use oxplow_app::sql_gateway::SqlGateway;
use oxplow_domain::Actor;
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, InitializeResult, InvokeParams, InvokeResult,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::conformance::{first_mismatch, normalize, ReferenceClient};
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
pub async fn test_extension(
    root: &Path,
    name: &str,
    layer: Option<&SqlGateway>,
    bless: bool,
) -> Result<TestReport, SdkError> {
    let checked = crate::check(root, name, &ExtensionCatalog::new(), layer, None).await?;
    let mut report = TestReport {
        name: name.to_string(),
        ok: false,
        errors: checked.errors.clone(),
        warnings: checked.warnings.clone(),
        blessed: Vec::new(),
        ran: vec!["check".into()],
    };
    let ext = checked.extension;
    let examples = ext
        .intent
        .as_ref()
        .map(|i| i.examples.clone())
        .unwrap_or_default();
    if ext.providers.is_empty() && !examples.is_empty() {
        report.warnings.push(format!(
            "{}/extension.yaml: intent.examples aren't run — nothing here declares a runtime \
             (`providers:`) — fix: none needed yet",
            ext.path
        ));
    }
    if report.errors.is_empty() {
        for spec in &ext.providers {
            report.ran.push(format!("provider {}", spec.id));
            test_provider(root, &ext, spec, bless, &mut report).await;
        }
        questions(root, &ext, layer, &mut report).await;
    }
    report.ok = report.errors.is_empty();
    Ok(report)
}

/// The extension's own `questions.yaml` (the answerability check): each
/// question's `skill` is a markdown file in the extension, its SQL runs
/// against the project's database (when there is one), and a command is
/// one of its providers' (`<id>.<name>`).
async fn questions(
    root: &Path,
    ext: &Extension,
    layer: Option<&SqlGateway>,
    report: &mut TestReport,
) {
    let rel = ext.path.trim_end_matches('/').to_string();
    let dir = root.join(&rel);
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
    };
    let checked = crate::answerability::check(
        &file,
        &questions,
        &crate::answerability::Checker {
            skill_text: &skill_text,
            sql: layer,
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
    for example in examples {
        let file = format!("fixtures/{}.yaml", example.name);
        let shown = format!("{rel}/{file}");
        let Ok(text) = std::fs::read_to_string(dir.join(&file)) else {
            report.warnings.push(format!(
                "{shown}:1: example `{}` has no fixture, so it isn't run — fix: write \
                 `input: {{ command, input }}` and `expect`",
                example.name
            ));
            continue;
        };
        let doc: Value = match serde_yaml::from_str(&text) {
            Ok(d) => d,
            Err(e) => {
                report
                    .errors
                    .push(format!("{shown}:1: not YAML: {e} — fix: correct it"));
                continue;
            }
        };
        let (Some(command), Some(input)) = (
            doc.pointer("/input/command").and_then(Value::as_str),
            doc.pointer("/input/input"),
        ) else {
            report.errors.push(format!(
                "{shown}:1: a provider example's `input` is `{{ command, input }}` — fix: name the \
                 command it invokes and its input"
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
        copy_dir(&root.join(&ext.path), &target).map_err(|e| e.to_string())?;
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
        let project = oxplow_app::source_runner::project_key(tmp.path());
        for (name, value) in env_credentials(spec) {
            svc.secrets
                .set(
                    &oxplow_app::source_runner::credential_account(&project, &ext.name, &name),
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
                    &*provider,
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

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
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
