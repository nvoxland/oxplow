//! P5.D3: the host over the real fake provider process — consent before
//! execution, the handshake against the approved declarations, and the
//! work-items conformance suite through `ExternalWorkItems`.

use std::path::{Path, PathBuf};

use oxplow_domain::{Actor, ThreadId};
use serde_json::json;

use super::*;
use crate::exec_consent::{self, ProgramKind};
use crate::extensions::Extension;
use crate::test_fixtures::{services_with_effort, EffortFixture};
use crate::work_items_conformance::{suite, ServicesProbe, WorkItemsProbe as _};

const EXT: &str = "tracker";

/// The fake provider's binary, built beside this test binary (the
/// workspace build builds every crate's bins).
fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps");
    let bin = dir.join("oxplow-provider-fake");
    assert!(
        bin.is_file(),
        "{} is missing; build it with `cargo build -p oxplow-provider-fake`",
        bin.display()
    );
    bin
}

/// A private extension declaring the fake as its work-items provider,
/// its entry a script running the fake with `hooks`.
fn write_extension(project: &Path, hooks: &str) {
    let dir = project.join("oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: tracker\nsharing: private\nintent:\n  purpose: the fake tracker\n  examples: [{ name: a }]\nproviders:\n  - id: fake\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_HOOKS='{hooks}' exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
    )
    .unwrap();
}

fn extension(project: &Path) -> Extension {
    let ext = crate::extensions::load_extensions(project)
        .into_iter()
        .find(|e| e.name == EXT)
        .expect("the tracker extension loads");
    assert_eq!(ext.errors, Vec::<String>::new());
    ext
}

/// The listing's view of the provider, as a person sees it.
fn program(fx: &EffortFixture, ext: &Extension) -> exec_consent::ProjectProgram {
    let config = fx.svc.config.read().unwrap().clone();
    exec_consent::list(
        &fx.svc.approvals,
        &fx.svc.layout.project_dir,
        &config,
        std::slice::from_ref(ext),
    )
    .into_iter()
    .find(|p| p.kind == ProgramKind::Provider)
    .expect("the provider is listed")
}

fn approve(fx: &EffortFixture, ext: &Extension) {
    let p = program(fx, ext);
    let config = fx.svc.config.read().unwrap().clone();
    exec_consent::approve_program(
        &fx.svc.approvals,
        &fx.svc.layout.project_dir,
        &config,
        std::slice::from_ref(ext),
        ProgramKind::Provider,
        &p.name,
        p.version.as_deref().unwrap(),
    )
    .unwrap();
}

/// The fixture with the tracker extension (running the fake with
/// `hooks`) approved on this machine.
async fn approved(hooks: &str) -> (EffortFixture, Extension) {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir, hooks);
    let ext = extension(&fx.svc.layout.project_dir);
    approve(&fx, &ext);
    (fx, ext)
}

const INSTANCE: &str = "tracker/fake";

/// Enable the instance in the project's config (in memory, as a
/// reconcile reads it).
fn configure(fx: &EffortFixture, enabled: bool, config: serde_json::Value) {
    fx.svc.config.write().unwrap().extension_instances.insert(
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled,
            config,
            sync_minutes: None,
        },
    );
}

async fn logged(fx: &EffortFixture, event_type: &str) -> Vec<serde_json::Value> {
    let t = event_type.to_string();
    fx.svc
        .db
        .read(move |c| {
            let mut stmt = c
                .prepare("SELECT payload FROM event_log WHERE type = ?1 ORDER BY seq")
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map([t], |r| r.get::<_, String>(0))
                .and_then(|r| r.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            Ok(rows)
        })
        .await
        .unwrap()
        .into_iter()
        .map(|p| serde_json::from_str(&p).unwrap())
        .collect()
}

#[tokio::test]
async fn an_unapproved_provider_is_refused_and_registers_nothing() {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir, "");
    let ext = extension(&fx.svc.layout.project_dir);
    let listed = program(&fx, &ext);
    assert_eq!(listed.name, INSTANCE);
    assert!(!listed.approved);

    let refused = fx
        .svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert_eq!(refused, HostError::Unapproved(INSTANCE.into()));
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
    assert!(fx.svc.work_items.get("fake").is_err());
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Unapproved
    );
}

#[tokio::test]
async fn edited_declarations_need_approving_again() {
    let (fx, ext) = approved("").await;
    let providers = &fx.svc.providers;
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    assert!(fx.svc.commands.namespace_owner("fake").is_some());
    assert!(providers.stop(INSTANCE).await);
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());

    // A widened declaration is a new version: shown unapproved, refused.
    let project = fx.svc.layout.project_dir.clone();
    let file = project.join("oxplow/extensions/tracker/provider.json");
    let mut declared = oxplow_provider_fake::declarations();
    declared.commands[0].summary = "Create a work item, now everywhere.".into();
    std::fs::write(&file, serde_json::to_string(&declared).unwrap()).unwrap();
    let ext = extension(&project);
    assert!(!program(&fx, &ext).approved);
    assert_eq!(
        providers
            .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
            .await
            .unwrap_err(),
        HostError::Unapproved(INSTANCE.into())
    );
}

#[tokio::test]
async fn a_provider_must_answer_with_its_approved_declarations() {
    let (fx, ext) = approved("bad-declarations").await;
    let err = fx
        .svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, HostError::DeclarationsChanged { detail, .. } if detail.contains("/commands")),
        "{err}"
    );
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
    // Not what was approved: off, and logged, until a person looks.
    assert!(matches!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
    assert_eq!(logged(&fx, "plugin.disabled").await.len(), 1);
}

/// P5.D4's red: an unconfigured instance can't be enabled — the check
/// refuses it naming the config field, and nothing is written.
#[tokio::test]
async fn an_unconfigured_instance_cannot_be_enabled() {
    let (fx, _ext) = approved("").await;
    let providers = &fx.svc.providers;
    let checked = providers.check_instance(INSTANCE, json!({})).await.unwrap();
    assert!(
        matches!(&checked.health.state, InstanceState::Unconfigured { problems } if problems[0].path == "/team"),
        "{checked:?}"
    );
    let refused = providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/config/team"),
        "{refused:?}"
    );
    assert!(fx.svc.config.read().unwrap().extension_instances.is_empty());
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());

    // Configured, it enables: written to the project's config and running.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
        .await
        .unwrap();
    assert_eq!(view.health.state, InstanceState::Ready);
    assert!(view.enabled);
    assert!(fx.svc.commands.namespace_owner("fake").is_some());
    let written =
        std::fs::read_to_string(oxplow_config::config_path(&fx.svc.layout.project_dir)).unwrap();
    assert!(written.contains("extensionInstances"), "{written}");
    assert_eq!(logged(&fx, "plugin.enabled").await.len(), 1);

    // Disabling stops it.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, false, json!({ "team": "core" }))
        .await
        .unwrap();
    assert_eq!(view.health.state, InstanceState::Off);
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
}

/// P5.D4's red: three failures in a row disable the instance, logged
/// with the reason; it stays off across reconciles until a person runs
/// `plugin.enable`.
#[tokio::test]
async fn three_failures_in_a_row_disable_an_instance_until_a_person_enables_it() {
    let (fx, _ext) = approved("fail-next:3").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));

    // Every start's check fails (each process begins with fail-next:3):
    // the reconcile's start is the first failure, two calls the rest.
    providers.reconcile().await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Failing { .. }
    ));
    for _ in 0..2 {
        let failed = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "work_item.create",
                json!({ "provider": "fake", "title": "x" }),
                false,
            )
            .await;
        assert!(failed.is_err());
    }
    let health = providers.health(INSTANCE).unwrap();
    let InstanceState::Disabled { reason } = &health.state else {
        panic!("{health:?}");
    };
    assert!(reason.contains("3 failures in a row"), "{reason}");
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
    let disabled = logged(&fx, "plugin.disabled").await;
    assert_eq!(disabled.len(), 1);
    assert_eq!(disabled[0]["plugin"], "plugin:tracker");
    assert_eq!(disabled[0]["contribution"], "fake");
    assert_eq!(disabled[0]["reason"], reason.as_str());

    // The log keeps it off.
    providers.reconcile().await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));

    // Only a person enables it again.
    let denied = fx
        .svc
        .commands
        .run(
            &Actor::Agent {
                thread_id: Some(ThreadId::new(fx.thread.value())),
                stream_id: None,
            },
            "plugin.enable",
            json!({ "plugin": "tracker", "kind": "provider", "contribution": "fake" }),
            false,
        )
        .await;
    assert!(matches!(
        denied,
        Err(oxplow_domain::CommandError::Denied { .. })
    ));
    write_extension(&fx.svc.layout.project_dir, "");
    approve(&fx, &extension(&fx.svc.layout.project_dir));
    let enabled = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            "plugin.enable",
            json!({ "plugin": "tracker", "kind": "provider", "contribution": "fake" }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(enabled.result["state"], "ok", "{}", enabled.result);
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Ready
    );
    assert!(fx.svc.commands.namespace_owner("fake").is_some());
}

/// P7.A7: the suite reads the provider back after its writes — a read
/// that doesn't restate what the writes recorded is a finding.
#[tokio::test]
async fn the_suite_finds_a_read_that_doesnt_restate_the_writes() {
    let (fx, ext) = approved("stale-read").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let provider = fx.svc.work_items.get("fake").unwrap();
    let findings = suite(
        &fx.svc.work_items_client(),
        &provider.id,
        provider.features,
        None,
        &ServicesProbe(&fx.svc),
        &Actor::Human,
    )
    .await;
    assert!(
        findings
            .iter()
            .any(|f| f.check == "sync" && f.message.contains("stale")),
        "{findings:?}"
    );
}

#[tokio::test]
async fn the_work_items_suite_passes_through_the_host_over_the_fake() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let provider = fx.svc.work_items.get("fake").unwrap();
    let actor = Actor::Agent {
        thread_id: Some(ThreadId::new(fx.thread.value())),
        stream_id: None,
    };
    let findings = suite(
        &fx.svc.work_items_client(),
        &provider.id,
        provider.features,
        None,
        &ServicesProbe(&fx.svc),
        &actor,
    )
    .await;
    assert_eq!(findings, vec![]);

    // Its writes are `work_item.*` runs, audited once each; its health
    // counts the calls.
    let audited = fx
        .svc
        .db
        .read(|c| {
            c.query_row(
                "SELECT count(*) FROM command_audit WHERE command = 'work_item.transition' AND outcome = 'ok'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    assert!(audited >= 5, "{audited}");
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(health.state, InstanceState::Ready);
    assert!(health.mean_invoke_ms.is_some() && health.last_ok_at.is_some());
}

/// P7.A1: `work_item.*` is the one write surface — another provider's
/// item moves through its process as `work_item.transition`, one audit
/// row, its `work_item.recorded` caused by the run. The provider's verbs
/// aren't commands of their own; its extra command is.
#[tokio::test]
async fn work_item_commands_write_another_providers_items_through_its_process() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    assert!(fx.svc.commands.spec("fake.transition").is_none());
    assert!(fx.svc.commands.spec("fake.create").is_none());
    let estimate = fx
        .svc
        .commands
        .spec("fake.estimate")
        .expect("its own command");
    assert_eq!(estimate.atomicity, oxplow_domain::Atomicity::External);
    assert_eq!(
        fx.svc.commands.namespace_owner("fake").as_deref(),
        Some("provider:tracker/fake")
    );

    let items = fx.svc.work_items_client();
    let item = items
        .create(
            &Actor::Human,
            crate::work_items::NewItem {
                provider: Some("fake".into()),
                title: "theirs".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(item, "work_item:fake:W-1");
    let moved = items
        .transition(
            &Actor::Human,
            &item,
            oxplow_domain::work_items::CanonicalState::Done,
            None,
        )
        .await
        .unwrap();
    let executed = moved.event_id.clone().unwrap();
    let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
    let caused: Vec<&str> = events
        .iter()
        .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
        .map(|e| e.envelope.event_type.as_str())
        .collect();
    assert_eq!(caused, vec!["work_item.recorded"]);
    let commands: Vec<String> = fx
        .svc
        .commands
        .audit_store()
        .list_recent(10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.command)
        .collect();
    // Starting it read it once (as the system), then the two writes.
    assert_eq!(
        commands,
        vec!["work_item.transition", "work_item.create", "provider.sync"]
    );

    // Undo dispatches again: the provider's inverse is renamed to
    // `work_item.transition` and moves the item back.
    assert_eq!(moved.inverse.as_ref().unwrap().name, "work_item.transition");
    items
        .undo(&Actor::Human, moved.audit_id.unwrap())
        .await
        .unwrap();
    fx.svc.event_pump.run_once().await.unwrap();
    let state = ServicesProbe(&fx.svc).record(&item).await.unwrap().state;
    assert_eq!(state, oxplow_domain::work_items::CanonicalState::Todo);
}

/// P7 review (tsk713): a composite over another provider's item runs its
/// calls as steps through the provider's process — the comment, then the
/// transition — recorded as one run whose `work_item.recorded` events it
/// caused, and not undoable.
#[tokio::test]
async fn a_composite_writes_another_providers_item_as_steps() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    first_read(&fx).await;
    let item = fx
        .svc
        .work_items_client()
        .create(
            &Actor::Human,
            crate::work_items::NewItem {
                provider: Some("fake".into()),
                title: "theirs".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let out = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            "command.sequence",
            json!({ "calls": [
                { "name": "work_item.comment", "input": { "ref": item, "body": "Looks good." } },
                { "name": "work_item.transition", "input": { "ref": item, "to": "done" } },
            ] }),
            false,
        )
        .await
        .unwrap();
    assert!(out.inverse.is_none());
    let executed = out.event_id.clone().unwrap();
    let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
    let caused = events
        .iter()
        .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
        .filter(|e| e.envelope.event_type == "work_item.recorded")
        .count();
    assert_eq!(caused, 2, "each step's record, caused by the one run");
    let audits = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
    assert_eq!(audits[0].command, "command.sequence");
    assert!(!audits.iter().any(|a| a.command == "work_item.comment"));
    fx.svc.event_pump.run_once().await.unwrap();
    let state = ServicesProbe(&fx.svc).record(&item).await.unwrap().state;
    assert_eq!(state, oxplow_domain::work_items::CanonicalState::Done);
}

/// P7.A1: a provider's verb input is checked against what it declares
/// (its `native` fields included) before the process is called; another
/// provider's parent or link target is refused at its field; an agent's
/// external run that needs a person is proposed with no dry run.
#[tokio::test]
async fn an_external_verb_input_is_checked_and_stays_on_its_provider() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let run = |input: serde_json::Value, name: &'static str| {
        let svc = fx.svc.clone();
        async move { svc.commands.run(&Actor::Human, name, input, false).await }
    };
    let err = run(
        json!({ "provider": "fake", "title": "x", "native": { "points": "many" } }),
        "work_item.create",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/native/points"),
        "{err:?}"
    );
    let ours = crate::work_items::PROVIDER;
    let task = oxplow_domain::refs::build::work_item_ref(fx.task);
    let err = run(
        json!({ "provider": "fake", "title": "x", "parent_ref": task }),
        "work_item.create",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, oxplow_domain::CommandError::Invalid { field: Some(f), message }
            if f == "/parent_ref" && message.contains(ours)),
        "{err:?}"
    );
    let item = run(
        json!({ "provider": "fake", "title": "x" }),
        "work_item.create",
    )
    .await
    .unwrap()
    .result["ref"]
        .as_str()
        .unwrap()
        .to_string();
    let err = run(
        json!({ "ref": item, "target": task, "link_type": "relates_to" }),
        "work_item.link",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/target"),
        "{err:?}"
    );
    let err = run(
        json!({ "ref": item, "to": "done", "native_state": "Doing" }),
        "work_item.transition",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("native_state"), "{err}");

    // Delete is destructive: an agent's run is proposed, never dry-run
    // through the process.
    let agent = Actor::Agent {
        thread_id: Some(ThreadId::new(fx.thread.value())),
        stream_id: None,
    };
    let err = fx
        .svc
        .commands
        .run(&agent, "work_item.delete", json!({ "ref": item }), false)
        .await
        .unwrap_err();
    let oxplow_domain::CommandError::Proposed { proposal, .. } = err else {
        panic!("{err:?}");
    };
    let id: i64 = proposal.strip_prefix("proposal:").unwrap().parse().unwrap();
    let kept = fx
        .svc
        .commands
        .proposal_store()
        .get(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.dry_run, None);
    fx.svc.event_pump.run_once().await.unwrap();
    assert!(ServicesProbe(&fx.svc).record(&item).await.is_some());
}

/// P6b.C2: a running instance's capability and features are a model row
/// (as the host reads them); stopping it takes the row away.
#[tokio::test]
async fn a_running_instance_publishes_its_features() {
    let (fx, ext) = approved("").await;
    let store = oxplow_db::SqliteCapabilityStore::new(fx.svc.db.clone());
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let fake = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.provider == "fake")
        .expect("published");
    assert_eq!(fake.capability, "work_items");
    assert_eq!(fake.extension.as_deref(), Some("tracker"));
    let declared = fx.svc.work_items.get("fake").unwrap().features;
    assert_eq!(fake.features, serde_json::to_value(declared).unwrap());
    assert!(fx.svc.providers.stop(INSTANCE).await);
    assert!(store
        .list()
        .await
        .unwrap()
        .iter()
        .all(|r| r.provider != "fake"));
}

/// P6b.C4: an extension's `ui.commands` name registered commands — or,
/// for its own provider (not on the bus until its instance runs), commands
/// its declarations list that aren't its capability's verbs — and their
/// input must fit.
#[tokio::test]
async fn ui_commands_are_checked_against_the_registry_or_the_providers_declarations() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write_extension(&root, "");
    let manifest = root.join("oxplow/extensions/tracker/extension.yaml");
    let base = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!(
            "{base}ui:\n  commands:\n    - {{ command: fake.estimate, label: Estimate, about: work_item, input: {{ ref: \"{{{{ref}}}}\", points: 3 }} }}\n    - {{ command: fake.comment, label: Comment, about: work_item, input: {{ ref: \"{{{{ref}}}}\", body: hi }} }}\n    - {{ command: fake.nope, label: Nope, about: work_item }}\n    - {{ command: work_item.transition, label: Done, about: work_item, input: {{ ref: \"{{{{ref}}}}\", to: done }} }}\n    - {{ command: work_item.transition, label: Bad, about: work_item, input: {{ ref: \"{{{{ref}}}}\" }} }}\n"
        ),
    )
    .unwrap();
    let ext = extension(&root);
    assert_eq!(ext.ui.commands.len(), 5);
    assert_eq!(ext.ui.commands[0].group, "fake");
    let schema = |name: &str| fx.svc.commands.input_schema(name);
    let v = crate::extensions::validate_extension(
        &fx.svc.sql,
        &fx.svc.extension_catalog,
        &root,
        EXT,
        Some(&schema),
    )
    .await
    .unwrap();
    let errs = v.errors.join("\n");
    assert!(
        !errs.contains("`Estimate`"),
        "the provider declares it: {errs}"
    );
    // A capability verb isn't a command of its own: `work_item.comment`
    // is (P7.A1).
    assert!(
        errs.contains("`ui.commands` `Comment`: no command `fake.comment`"),
        "{errs}"
    );
    assert!(!errs.contains("`Done`"), "{errs}");
    assert!(
        errs.contains("`ui.commands` `Nope`: no command `fake.nope`"),
        "{errs}"
    );
    assert!(
        errs.contains("`ui.commands` `Bad`: the input doesn't fit `work_item.transition`"),
        "{errs}"
    );
}

/// P6b.E3: what approving a provider's declarations would change: all new
/// before it runs; against the running declarations after an edit.
#[tokio::test]
async fn declaration_effects_compare_the_files_with_what_runs() {
    let (fx, ext) = approved("").await;
    let first = fx
        .svc
        .providers
        .declaration_effects(INSTANCE)
        .await
        .unwrap();
    assert_eq!(first.change, crate::extension_effects::Change::Added);
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let path = fx
        .svc
        .layout
        .project_dir
        .join("oxplow/extensions/tracker/provider.json");
    let mut declared = oxplow_provider_fake::declarations();
    let mut extra = declared.commands[0].clone();
    extra.name = "archive".into();
    extra.confirm = "destructive".into();
    declared.commands.push(extra);
    std::fs::write(&path, serde_json::to_string_pretty(&declared).unwrap()).unwrap();
    let after = fx
        .svc
        .providers
        .declaration_effects(INSTANCE)
        .await
        .unwrap();
    assert_eq!(after.change, crate::extension_effects::Change::Changed);
    let archive = after.commands.iter().find(|c| c.name == "archive").unwrap();
    assert_eq!(archive.change, crate::extension_effects::Change::Added);
    assert!(after
        .commands
        .iter()
        .filter(|c| c.name != "archive")
        .all(|c| c.change == crate::extension_effects::Change::Unchanged));
}

/// R7: the baseline is the last approved copy, not the running instance —
/// a changed spec stops the instance (and a restart starts none), and the
/// re-approval must still show what changed.
#[tokio::test]
async fn declaration_effects_compare_against_the_last_approved_copy() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let project = fx.svc.layout.project_dir.clone();
    let manifest = project.join("oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    network: [api.example.com]\n",
        ),
    )
    .unwrap();
    fx.svc.providers.stop(INSTANCE).await;
    assert!(fx.svc.providers.get(INSTANCE).await.is_none());
    let effect = fx
        .svc
        .providers
        .declaration_effects(INSTANCE)
        .await
        .unwrap();
    assert_eq!(effect.change, crate::extension_effects::Change::Changed);
    assert_eq!(effect.before.unwrap().hosts, Vec::<String>::new());
    assert_eq!(effect.after.unwrap().hosts, vec!["api.example.com"]);
    assert!(effect
        .commands
        .iter()
        .all(|c| c.change == crate::extension_effects::Change::Unchanged));
}

/// tsk546: a provider's args are hashed where it runs (its extension
/// folder), and an arg reaching outside the folder is refused, so no file
/// it runs escapes the approval.
#[tokio::test]
async fn provider_args_stay_inside_the_approved_folder() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let manifest = project.join("oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    args: [../../../tools/tracker.py]\n",
        ),
    )
    .unwrap();
    let loaded = crate::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty());
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| e.contains("../../../tools/tracker.py")),
        "{:?}",
        loaded.errors
    );

    // An arg inside the folder is part of the approval.
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    args: [lib/main.py]\n",
        ),
    )
    .unwrap();
    let dir = project.join("oxplow/extensions/tracker");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/main.py"), "good()").unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    assert!(program(&fx, &ext).approved);
    std::fs::write(dir.join("lib/main.py"), "evil()").unwrap();
    assert!(!program(&fx, &ext).approved);
}

const ADAPTER_MANIFEST: &str = "manifest: 2\nname: tracker\nsharing: private\nintent:\n  purpose: notes over MCP\n  examples: [{ name: a }]\nproviders:\n  - id: notes\n    capability: work_items\n    adapter:\n      mcp: { command: [bin/server, --stdio] }\n      mapping: mcp/x.star\n      tools: mcp/tools.json\n    declarations: provider.json\n";

/// A private extension whose provider is an MCP server behind oxplow's
/// adapter: the server, its mapping and its pinned tools.
fn write_adapter_extension(project: &Path) -> PathBuf {
    let dir = project.join("oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::create_dir_all(dir.join("mcp")).unwrap();
    std::fs::write(dir.join("extension.yaml"), ADAPTER_MANIFEST).unwrap();
    std::fs::write(dir.join("bin/server"), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.join("mcp/x.star"), "def transform(x):\n    return {}\n").unwrap();
    std::fs::write(
        dir.join("mcp/tools.json"),
        r#"[{ "name": "list_items", "description": "List.", "inputSchema": { "type": "object" } }]"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
    )
    .unwrap();
    dir
}

/// P7.A6: an adapter provider names its MCP server (a command), its
/// mapping and its pinned tools, each inside the folder — instead of an
/// `entry`, never with one.
#[tokio::test]
async fn an_adapter_provider_names_its_server_mapping_and_tools_inside_the_folder() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_adapter_extension(&project);
    let ext = extension(&project);
    let adapter = ext.providers[0]
        .adapter
        .as_ref()
        .expect("an adapter provider");
    assert_eq!(adapter.mcp.command, ["bin/server", "--stdio"]);
    for (from, to, says) in [
        ("    adapter:\n", "    entry: bin/server\n    adapter:\n", "either `entry` or `adapter`"),
        ("    adapter:\n      mcp: { command: [bin/server, --stdio] }\n      mapping: mcp/x.star\n      tools: mcp/tools.json\n", "", "either `entry` or `adapter`"),
        ("mapping: mcp/x.star", "mapping: ../x.star", "../x.star"),
        ("tools: mcp/tools.json", "tools: /tmp/tools.json", "/tmp/tools.json"),
        ("tools: mcp/tools.json", "tools: mcp/missing.json", "mcp/missing.json"),
        ("command: [bin/server, --stdio]", "command: [/usr/local/bin/npx, server]", "/usr/local/bin/npx"),
        ("command: [bin/server, --stdio]", "command: [bin/server, ../../x.js]", "../../x.js"),
        ("mcp: { command: [bin/server, --stdio] }", "mcp: { url: \"https://mcp.example.com\" }", "url"),
    ] {
        assert!(ADAPTER_MANIFEST.contains(from), "{from}");
        std::fs::write(dir.join("extension.yaml"), ADAPTER_MANIFEST.replace(from, to)).unwrap();
        let loaded = crate::extensions::load_extensions(&project)
            .into_iter()
            .find(|e| e.name == EXT)
            .unwrap();
        assert!(loaded.providers.is_empty(), "{to}");
        assert!(
            loaded.errors.iter().any(|e| e.contains(says)),
            "{to}: {:?}",
            loaded.errors
        );
    }
    std::fs::write(dir.join("mcp/tools.json"), "{}").unwrap();
    std::fs::write(dir.join("extension.yaml"), ADAPTER_MANIFEST).unwrap();
    let loaded = crate::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(
        loaded.errors.iter().any(|e| e.contains("mcp/tools.json")),
        "pinned tools are a list: {:?}",
        loaded.errors
    );
}

/// P7.A6: approving an adapter provider approves its mapping, its pinned
/// tools and its server: changing any of them needs approving again.
#[tokio::test]
async fn an_adapter_providers_approval_covers_its_mapping_pins_and_server() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_adapter_extension(&project);
    for file in ["mcp/x.star", "mcp/tools.json", "bin/server"] {
        let ext = extension(&project);
        approve(&fx, &ext);
        assert!(program(&fx, &ext).approved);
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = match file {
            "mcp/tools.json" => text.replace("List.", "List them all."),
            _ => format!("{text}# changed\n"),
        };
        std::fs::write(&path, edited).unwrap();
        assert!(!program(&fx, &extension(&project)).approved, "{file}");
    }
}

/// tsk547: a provider runs from a verified copy of its approved folder,
/// outside the repo — not from the live tree — and a tampered copy is
/// replaced before it runs.
#[tokio::test]
async fn a_provider_runs_from_its_verified_copy() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let script = project.join("oxplow/extensions/tracker/bin/provider");
    let seen = tempfile::tempdir().unwrap();
    let cwd_file = seen.path().join("cwd");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\npwd > '{}'\nexec '{}' \"$@\"\n",
            cwd_file.display(),
            fake_bin().display()
        ),
    )
    .unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    let providers = &fx.svc.providers;
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let copies = fx
        .svc
        .layout
        .state_dir
        .join("global-config/provider-copies");
    let ran_in = std::path::PathBuf::from(std::fs::read_to_string(&cwd_file).unwrap().trim());
    let copies = copies.canonicalize().unwrap();
    assert!(
        ran_in.starts_with(&copies),
        "{} not under {}",
        ran_in.display(),
        copies.display()
    );
    assert!(
        ran_in.ends_with("oxplow/extensions/tracker"),
        "{}",
        ran_in.display()
    );

    // Someone edits the copy: the next start replaces it from the
    // approved tree before running.
    assert!(providers.stop(INSTANCE).await);
    let copied = ran_in.join("bin/provider");
    std::fs::write(&copied, "#!/bin/sh\necho tampered\nexit 1\n").unwrap();
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    assert_eq!(
        std::fs::read_to_string(&copied).unwrap(),
        std::fs::read_to_string(&script).unwrap()
    );
}

/// tsk548: a provider may emit only its capability's event types — a
/// declared `plugin.enabled` (which would clear another contribution's
/// automatic disable) is refused when the manifest loads.
#[tokio::test]
async fn a_provider_cannot_declare_another_core_event_type() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let mut declared = oxplow_provider_fake::declarations();
    declared
        .event_types
        .push(oxplow_provider_protocol::model::EventTypeDecl {
            event_type: "plugin.enabled".into(),
            v: 1,
            schema: oxplow_domain::events::schema::schema_for::<
                oxplow_domain::events::schema::PluginEnabled,
            >(),
        });
    std::fs::write(
        project.join("oxplow/extensions/tracker/provider.json"),
        serde_json::to_string(&declared).unwrap(),
    )
    .unwrap();
    let loaded = crate::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty());
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| e.contains("plugin.enabled@1") && e.contains("work_items")),
        "{:?}",
        loaded.errors
    );
}

/// tsk548: an event's subjects are the provider's own items and its
/// extension, nothing else.
#[test]
fn a_provider_event_names_only_its_own_refs() {
    use super::registry::check_subject;
    assert!(check_subject("fake", "tracker", "work_item:fake:W-1").is_ok());
    assert!(check_subject("fake", "tracker", "plugin:tracker").is_ok());
    for bad in [
        "work_item:oxplow:tsk1",
        "plugin:other",
        "wiki:page",
        "file:src/a.rs",
        "garbage",
    ] {
        assert!(check_subject("fake", "tracker", bad).is_err(), "{bad}");
    }
}

/// tsk549: a provider whose `check` hangs times out — counted as a
/// failure — and never blocks stopping it.
#[tokio::test]
async fn a_hung_check_times_out_and_the_instance_can_still_stop() {
    let (fx, ext) = approved("slow-check:30000").await;
    let providers = &fx.svc.providers;
    let enabled = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        providers.enable(&ext, &ext.providers[0], json!({ "team": "core" })),
    )
    .await
    .expect("enable returns once check times out");
    assert!(
        enabled.is_ok(),
        "a start that merely failed stays enabled: {enabled:?}"
    );
    let health = providers.health(INSTANCE).unwrap();
    let InstanceState::Failing { errors } = &health.state else {
        panic!("{health:?}");
    };
    assert!(errors[0].contains("timed out"), "{errors:?}");
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), providers.stop(INSTANCE))
            .await
            .is_ok(),
        "stop must not wait on a hung start"
    );
}

/// tsk549: a hung `invoke` times out (with `$/cancel`) and counts as a
/// failure, instead of blocking the caller forever with health `ready`.
#[tokio::test]
async fn a_hung_invoke_times_out_and_counts() {
    let (fx, ext) = approved("").await;
    let providers = &fx.svc.providers;
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    set_hooks(&fx, "slow:30000").await;
    let ran = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fx.svc.commands.run(
            &Actor::Human,
            "work_item.create",
            json!({ "provider": "fake", "title": "x" }),
            false,
        ),
    )
    .await
    .expect("the call returns once it times out");
    let err = ran.unwrap_err().to_string();
    assert!(err.contains("timed out"), "{err}");
    assert_eq!(providers.health(INSTANCE).unwrap().consecutive_failures, 1);
}

/// tsk569: a disable while an instance is starting wins — the start
/// doesn't bring back what was just disabled.
#[tokio::test]
async fn a_disable_while_starting_keeps_the_instance_off() {
    let (fx, ext) = approved("slow-check:400").await;
    let providers = fx.svc.providers.clone();
    let spec = ext.providers[0].clone();
    let starting = {
        let (providers, ext) = (providers.clone(), ext.clone());
        tokio::spawn(async move {
            providers
                .enable(&ext, &spec, json!({ "team": "core" }))
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    providers
        .disable(INSTANCE, "a person turned it off".into())
        .await;
    let _ = starting.await.unwrap();
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
    assert!(providers.get(INSTANCE).await.is_none());
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
}

/// tsk569: when whether an instance was disabled can't be read, a
/// reconcile doesn't start it (it fails closed).
#[tokio::test]
async fn an_unreadable_disable_record_keeps_the_instance_off() {
    let (fx, _ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc
        .db
        .transaction(|c| {
            c.execute_batch("ALTER TABLE plugin_health RENAME TO plugin_health_unreadable")
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    fx.svc.providers.reconcile().await;
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert!(
        matches!(&health.state, InstanceState::Failing { errors } if errors[0].contains("disabled")),
        "{health:?}"
    );
}

/// tsk569: approving an updated provider restarts a running instance on
/// what was approved, instead of leaving it to be disabled at its next
/// start as changed.
#[tokio::test]
async fn approving_updated_declarations_restarts_the_instance() {
    let (fx, _ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    assert!(fx.svc.commands.namespace_owner("fake").is_some());

    // The provider updates: new behaviour and new declarations, approved.
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "bad-declarations");
    std::fs::write(
        project.join("oxplow/extensions/tracker/provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::bad_declarations()).unwrap(),
    )
    .unwrap();
    approve(&fx, &extension(&project));
    fx.svc.providers.approved(INSTANCE).await;

    let running = fx.svc.providers.get(INSTANCE).await.expect("running");
    assert_eq!(running.declared, oxplow_provider_fake::bad_declarations());
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Ready
    );
    assert!(logged(&fx, "plugin.disabled").await.is_empty());
    // The restarted instance republished its capability row, with the
    // features it declares now.
    let row = oxplow_db::SqliteCapabilityStore::new(fx.svc.db.clone())
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.provider == "fake")
        .expect("republished");
    assert_eq!(row.features, running.declared.capabilities[0].features);
}

/// Declarations on disk that don't parse fail the extension's load, so
/// the provider isn't there to compare: `NotFound`, never an empty diff,
/// and the load error names the file.
#[tokio::test]
async fn declaration_effects_of_unreadable_declarations_is_an_error() {
    let (fx, _ext) = approved("").await;
    std::fs::write(
        fx.svc
            .layout
            .project_dir
            .join("oxplow/extensions/tracker/provider.json"),
        "{ not json",
    )
    .unwrap();
    let err = fx
        .svc
        .providers
        .declaration_effects(INSTANCE)
        .await
        .unwrap_err();
    assert!(
        matches!(err, oxplow_domain::DomainError::NotFound),
        "{err:?}"
    );
    let errors = crate::extensions::load_extensions(&fx.svc.layout.project_dir)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap()
        .errors
        .join("\n");
    assert!(errors.contains("provider.json"), "{errors}");
}

/// tsk569: enabling from Settings writes nothing when the enable itself
/// fails — the config never says enabled for an instance that wasn't.
#[tokio::test]
async fn a_failed_enable_writes_no_config() {
    let (fx, _ext) = approved("").await;
    fx.svc
        .db
        .transaction(|c| {
            c.execute_batch(
                "CREATE TRIGGER refuse_enabled BEFORE INSERT ON event_log
                 WHEN NEW.type = 'plugin.enabled'
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    let err = fx
        .svc
        .providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
        .await;
    assert!(err.is_err(), "{err:?}");
    assert!(fx.svc.config.read().unwrap().extension_instances.is_empty());
    assert!(!fx.svc.commands.namespace_owner("fake").is_some());
}

/// tsk569, P7.A1: a capability verb is run by `work_item.<verb>` (whose
/// spec is what a person confirms), never on its own: it is
/// `confirm: never` and `effect: record`; a provider declaring `delete`
/// must declare the verb.
#[test]
fn a_capability_verb_is_record_and_never_confirms() {
    let (spec, declared) = {
        let dir = tempfile::tempdir().unwrap();
        write_extension(dir.path(), "");
        (
            extension(dir.path()).providers[0].clone(),
            oxplow_provider_fake::declarations(),
        )
    };
    spec::check_declarations(&spec, &declared).unwrap();
    for (field, value) in [("confirm", "always"), ("effect", "write")] {
        let mut changed = declared.clone();
        for c in changed
            .commands
            .iter_mut()
            .filter(|c| c.name == "transition")
        {
            match field {
                "confirm" => c.confirm = value.into(),
                _ => c.effect = value.into(),
            }
        }
        let err = spec::check_declarations(&spec, &changed).unwrap_err();
        assert!(err.contains("transition") && err.contains(field), "{err}");
    }
    // The extra command isn't a verb: it may ask.
    let mut asking = declared.clone();
    for c in asking.commands.iter_mut().filter(|c| c.name == "estimate") {
        c.confirm = "always".into();
    }
    spec::check_declarations(&spec, &asking).unwrap();
    let mut no_delete = declared;
    no_delete.commands.retain(|c| c.name != "delete");
    let err = spec::check_declarations(&spec, &no_delete).unwrap_err();
    assert!(err.contains("`delete`"), "{err}");
}

/// The fake's three items, filed through `work_item.create`.
async fn three_items(fx: &EffortFixture) -> Vec<String> {
    let items = fx.svc.work_items_client();
    let mut refs = Vec::new();
    for title in ["one", "two", "three"] {
        refs.push(
            items
                .create(
                    &Actor::Human,
                    crate::work_items::NewItem {
                        provider: Some("fake".into()),
                        title: title.into(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap(),
        );
    }
    refs
}

async fn collector_state(fx: &EffortFixture) -> oxplow_db::CollectorState {
    oxplow_db::SqliteProviderCollectorStore::new(fx.svc.db.clone())
        .get(INSTANCE, "work_items")
        .await
        .unwrap()
        .expect("a read was recorded")
}

/// How many `work_item.recorded` events name `item`.
async fn recorded_for(fx: &EffortFixture, item: &str) -> usize {
    logged(fx, "work_item.recorded")
        .await
        .iter()
        .filter(|p| p["item"]["ref"] == item)
        .count()
}

/// P7.A3: a read streams the provider's items into `work_item` and keeps
/// its cursor; the next read resumes from it. Starting an instance reads
/// it once.
#[tokio::test]
async fn a_read_streams_records_into_work_item_and_keeps_its_cursor() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let first = first_read(&fx).await;
    assert_eq!(
        (first.status.as_str(), first.records),
        ("ok", 0),
        "read once on start, in the background"
    );
    let refs = three_items(&fx).await;

    let out = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.result["reads"][0]["records"], 3);
    let state = collector_state(&fx).await;
    assert_eq!(state.records, 3);
    assert_eq!(state.state, Some(json!({ "cursor": 3, "seen": 3 })));
    for r in &refs {
        assert_eq!(
            recorded_for(&fx, r).await,
            2,
            "its create's, then the read's"
        );
    }
    // The next read resumes after the cursor: nothing new.
    let again = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(again.result["reads"][0]["records"], 0);
    fx.svc.event_pump.run_once().await.unwrap();
    assert!(ServicesProbe(&fx.svc).record(&refs[2]).await.is_some());

    let err = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE, "collector": "nope" }),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/collector"),
        "{err:?}"
    );
}

/// P7 review (tsk716): starting an instance doesn't wait for its first
/// read — a team with thousands of issues would hold up the Enable button
/// and every reconcile — but the read still happens, in the background.
#[tokio::test]
async fn enabling_returns_before_the_first_read_finishes() {
    // Under the in-memory 2 s call timeout, so the read itself succeeds.
    let (fx, ext) = approved("slow:1500").await;
    let started = std::time::Instant::now();
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1000),
        "enable waited {:?} for the first read",
        started.elapsed()
    );
    let read = first_read(&fx).await;
    assert_eq!(read.status, "ok");
}

/// The instance's first read, once its background sync lands.
async fn first_read(fx: &EffortFixture) -> oxplow_db::CollectorState {
    let store = oxplow_db::SqliteProviderCollectorStore::new(fx.svc.db.clone());
    for _ in 0..200 {
        if let Some(state) = store.get(INSTANCE, "work_items").await.unwrap() {
            if state.status != "reading" {
                return state;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the first read never landed");
}

/// P7 review (tsk715): two syncs of one collector at once don't both read
/// from the same checkpoint — the second waits, then resumes after the
/// first — so each item is recorded once and the cursor never goes back.
#[tokio::test]
async fn concurrent_syncs_of_one_collector_record_each_item_once() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let refs = three_items(&fx).await;
    set_hooks(&fx, "slow:300").await;
    let sync = || {
        fx.svc.commands.run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
    };
    let (a, b) = tokio::join!(sync(), sync());
    let records = [a.unwrap(), b.unwrap()]
        .iter()
        .map(|r| r.result["reads"][0]["records"].as_u64().unwrap())
        .sum::<u64>();
    assert_eq!(records, 3, "the items were read once between them");
    for r in &refs {
        assert_eq!(
            recorded_for(&fx, r).await,
            2,
            "its create's, then one read's"
        );
    }
    assert_eq!(
        collector_state(&fx).await.state,
        Some(json!({ "cursor": 3, "seen": 3 }))
    );
}

/// P7.A3: a read that fails midway keeps the records its last checkpoint
/// covered (the next read resumes there), and the failure counts; a
/// record of another provider's item fails the read and writes nothing.
#[tokio::test]
async fn a_failed_read_keeps_what_its_checkpoints_covered() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    let refs = three_items(&fx).await;
    set_hooks(&fx, "read-fail-after:2").await;
    let err = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("after 2 records"), "{err}");
    let state = collector_state(&fx).await;
    assert_eq!((state.status.as_str(), state.records), ("error", 2));
    assert_eq!(state.state, Some(json!({ "cursor": 2, "seen": 2 })));
    assert_eq!(recorded_for(&fx, &refs[2]).await, 1, "only its create's");
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        1
    );

    set_hooks(&fx, "bad-record").await;
    let before = logged(&fx, "work_item.recorded").await.len();
    let err = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("work_item:other:X-1"), "{err}");
    assert_eq!(logged(&fx, "work_item.recorded").await.len(), before);
}

/// P7.A3: a read that sends nothing for the call timeout is cancelled
/// and counts as a failure.
#[tokio::test]
async fn a_silent_read_is_cancelled_and_counts() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    set_hooks(&fx, "slow:5000").await;
    let err = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("sent nothing"), "{err}");
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        1
    );
}

/// P7.A3: the schedule reads each running instance's collectors that are
/// due, as the system and audited; one read just now isn't due again; an
/// instance set to `syncMinutes: 0` is read only on request.
#[tokio::test]
async fn scheduled_syncs_run_due_collectors_as_the_system() {
    let (fx, _ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    // Starting read it once; it isn't due again for five minutes.
    assert_eq!(fx.svc.providers.sync_due().await, 0);
    fx.svc
        .db
        .transaction(|tx| {
            tx.execute(
                "UPDATE provider_collector_state SET last_read_at = '2020-01-01T00:00:00.000000Z'",
                [],
            )
            .map(|_| ())
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(fx.svc.providers.sync_due().await, 1);
    // P7 review (tsk722): the schedule says when the instance is next due:
    // its read plus five minutes plus the scheduler's tick.
    let (due, fresh) =
        crate::collector_runner::tests::due_and_fresh(&fx.svc, "tracker", "fake").await;
    let due = oxplow_domain::Timestamp::parse(&due.expect("next_due_at is set"))
        .unwrap()
        .unix_ms();
    let expected = oxplow_domain::Timestamp::now().unix_ms() + 6 * 60_000;
    assert!((expected - due).abs() < 30_000, "{due} vs {expected}");
    assert!(fresh);
    let audits = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
    let syncs: Vec<_> = audits.iter().filter(|r| r.command == sync::SYNC).collect();
    assert!(syncs.len() >= 2, "{audits:?}");
    assert!(syncs
        .iter()
        .all(|r| r.actor_kind == oxplow_domain::events::schema::ActorKind::System));

    fx.svc
        .config
        .write()
        .unwrap()
        .extension_instances
        .get_mut(INSTANCE)
        .unwrap()
        .sync_minutes = Some(0);
    fx.svc
        .db
        .transaction(|tx| {
            tx.execute(
                "UPDATE provider_collector_state SET last_read_at = '2020-01-01T00:00:00.000000Z'",
                [],
            )
            .map(|_| ())
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(fx.svc.providers.sync_due().await, 0);
    let (due, _) = crate::collector_runner::tests::due_and_fresh(&fx.svc, "tracker", "fake").await;
    assert_eq!(due, None, "read only on request: never late");
}

/// Send the running fake new script hooks.
async fn set_hooks(fx: &EffortFixture, hooks: &str) {
    let instance = fx.svc.providers.get(INSTANCE).await.expect("running");
    instance.hook(hooks).await;
}

/// Run `work_item.create` on the fake as the person.
async fn create_on_fake(
    fx: &EffortFixture,
) -> Result<oxplow_domain::CommandOutcome, oxplow_domain::CommandError> {
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "work_item.create",
            json!({ "provider": "fake", "title": "x" }),
            false,
        )
        .await
}

async fn enabled_fake() -> (EffortFixture, Extension) {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    // Its start's first read, in the background (tsk716).
    first_read(&fx).await;
    (fx, ext)
}

/// P7.A4: a short rate limit is waited out and the call retried once; it
/// never counts as a failure.
#[tokio::test]
async fn a_short_rate_limit_is_waited_out_and_doesnt_count() {
    let (fx, _ext) = enabled_fake().await;
    set_hooks(&fx, "rate-limit:200").await;
    let started = std::time::Instant::now();
    create_on_fake(&fx).await.unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_millis(200));
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(health.consecutive_failures, 0);
    assert_eq!(health.rate_limited_until, None, "the success clears it");

    // A read is retried the same way, from its last checkpoint.
    set_hooks(&fx, "rate-limit:100").await;
    let out = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.result["reads"][0]["records"], 1);
}

/// P7.A4: a long rate limit fails the call honestly, without counting,
/// and the schedule waits it out.
#[tokio::test]
async fn a_long_rate_limit_fails_without_counting_and_defers_the_schedule() {
    let (fx, _ext) = enabled_fake().await;
    set_hooks(&fx, "rate-limit:60000").await;
    let err = create_on_fake(&fx).await.unwrap_err().to_string();
    assert!(
        err.contains("rate limited") && err.contains("try again in 60s"),
        "{err}"
    );
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(health.consecutive_failures, 0);
    assert!(health.rate_limited_until.is_some());
    fx.svc
        .db
        .transaction(|tx| {
            tx.execute(
                "UPDATE provider_collector_state SET last_read_at = '2020-01-01T00:00:00.000000Z'",
                [],
            )
            .map(|_| ())
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(fx.svc.providers.sync_due().await, 0, "it waits");
}

/// P7.A4: what a read in progress says shows on the instance while it
/// runs, and goes when it ends.
#[tokio::test]
async fn progress_shows_on_the_instance_while_a_read_runs() {
    let (fx, _ext) = enabled_fake().await;
    three_items(&fx).await;
    set_hooks(&fx, "progress").await;
    let svc = fx.svc.clone();
    let reading = tokio::spawn(async move {
        svc.commands
            .run(
                &Actor::Human,
                sync::SYNC,
                json!({ "instance": INSTANCE }),
                false,
            )
            .await
    });
    let mut seen = Vec::new();
    while !reading.is_finished() {
        if let Some(a) = fx.svc.providers.health(INSTANCE).unwrap().activity {
            if !seen.contains(&a) {
                seen.push(a);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    reading.await.unwrap().unwrap();
    assert!(
        seen.contains(&"work_items: record 1 of 3 (33%)".to_string()),
        "{seen:?}"
    );
    assert_eq!(fx.svc.providers.health(INSTANCE).unwrap().activity, None);
}

/// P7.A4: rate limits interleaved with failures neither count nor reset
/// the count: three real failures still disable the instance.
#[tokio::test]
async fn three_failures_disable_it_with_rate_limits_interleaved() {
    let (fx, _ext) = enabled_fake().await;
    set_hooks(&fx, "fail-next:1").await;
    assert!(create_on_fake(&fx).await.is_err());
    set_hooks(&fx, "rate-limit:60000").await;
    assert!(create_on_fake(&fx).await.is_err());
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        1
    );
    set_hooks(&fx, "fail-next:2").await;
    assert!(create_on_fake(&fx).await.is_err());
    assert!(create_on_fake(&fx).await.is_err());
    assert!(matches!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
}
