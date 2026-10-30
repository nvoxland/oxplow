use super::*;

/// The instance config the kit checks with: `fixtures/provider-<id>.yaml`.
pub(super) fn fixture_config(dir: &Path, rel: &str, spec: &ProviderSpec) -> Result<Value, String> {
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
pub(super) fn env_credentials(
    spec: &ProviderSpec,
    env: &host::HostEnv,
) -> std::collections::BTreeMap<String, String> {
    spec.credential_names()
        .into_iter()
        .filter(|n| !spec.is_client_secret(n))
        .filter_map(|n| env(&n).map(|v| (n, v)))
        .collect()
}

pub(super) async fn test_provider(
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
pub(super) struct UnderTest<'a> {
    pub(super) ext: &'a Extension,
    pub(super) spec: &'a ProviderSpec,
    /// Its declarations file, as read.
    pub(super) declared: &'a InitializeResult,
    /// The extension folder, shown as `rel`.
    pub(super) dir: &'a Path,
    pub(super) rel: &'a str,
    /// The environment its credentials come from.
    pub(super) env: &'a host::HostEnv,
}

/// Initialize, check, and the intent examples, over the tapped client.
pub(super) async fn session(
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
pub(super) async fn read_back(
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
pub(super) fn transcript(
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
pub(super) fn kit_search_path(env: &host::HostEnv) -> Vec<std::path::PathBuf> {
    let extra = env("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| oxplow_app::agent_path::well_known_bin_dirs(Path::new(&h)))
        .unwrap_or_default();
    oxplow_app::agent_path::search_path_in(env("PATH").as_deref().map(std::ffi::OsStr::new), &extra)
}

/// Publish the generation the throwaway project's config and files ask
/// for, as the app's owner of generations does on a change.
pub(super) async fn publish_generation(svc: &Arc<oxplow_app::Services>) -> Result<(), String> {
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
pub(super) async fn with_work_list(host: &Host, ext: &Extension) -> Result<Host, String> {
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
pub(super) async fn policy_suite(
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

pub(super) async fn suite(
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
