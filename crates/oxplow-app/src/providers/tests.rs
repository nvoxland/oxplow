//! P5.D3: the host over the real fake provider process — consent before
//! execution, the handshake against the approved declarations, and the
//! work-items conformance suite through `ExternalWorkItems`.

use std::path::{Path, PathBuf};

use oxplow_domain::{Actor, DomainError, ThreadId};
use oxplow_oauth_sim::OAuthSim;
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
    // `plain-writes` declares no `idempotent_writes`: what is approved.
    let declared = if hooks.split(',').any(|h| h.trim() == "plain-writes") {
        oxplow_provider_fake::plain_declarations()
    } else {
        oxplow_provider_fake::declarations()
    };
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&declared).unwrap(),
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
            provider: None,
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
    .await
    .findings;
    assert!(
        findings
            .iter()
            .any(|f| f.check == "sync" && f.message.contains("stale")),
        "{findings:?}"
    );
}

/// P10: a provider that declares `idempotent_writes` and doesn't keep
/// it (the fake under `forget-keys`) fails the suite; one that doesn't
/// declare it isn't asked.
#[tokio::test]
async fn the_suite_finds_a_provider_that_forgets_its_keys() {
    for (hooks, finds) in [("forget-keys", true), ("plain-writes", false)] {
        let (fx, ext) = approved(hooks).await;
        fx.svc
            .providers
            .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
            .await
            .unwrap();
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
        .await
        .findings;
        assert_eq!(
            findings.iter().any(|f| f.check == "idempotent_writes"),
            finds,
            "{hooks}: {findings:?}"
        );
    }
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
    .await
    .findings;
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
    assert_eq!(
        adapter.mcp,
        spec::McpServer::Command {
            command: vec!["bin/server".into(), "--stdio".into()]
        }
    );
    for (from, to, says) in [
        ("    adapter:\n", "    entry: bin/server\n    adapter:\n", "either `entry` or `adapter`"),
        ("    adapter:\n      mcp: { command: [bin/server, --stdio] }\n      mapping: mcp/x.star\n      tools: mcp/tools.json\n", "", "either `entry` or `adapter`"),
        ("mapping: mcp/x.star", "mapping: ../x.star", "../x.star"),
        ("tools: mcp/tools.json", "tools: /tmp/tools.json", "/tmp/tools.json"),
        ("tools: mcp/tools.json", "tools: mcp/missing.json", "mcp/missing.json"),
        ("command: [bin/server, --stdio]", "command: [/usr/local/bin/npx, server]", "/usr/local/bin/npx"),
        ("command: [bin/server, --stdio]", "command: [bin/server, ../../x.js]", "../../x.js"),
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
    // P9.B1: the id is the instance's — a second instance of `fake` may
    // name its own items, not the first's.
    assert!(check_subject("fake_second", "tracker", "work_item:fake_second:W-1").is_ok());
    assert!(check_subject("fake_second", "tracker", "work_item:fake:W-1").is_err());
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
        .disable(INSTANCE, None, "a person turned it off".into())
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
            1,
            "its create's; the read restated it, so recorded nothing new (tsk799)"
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
            1,
            "its create's; the read restated it (tsk799)"
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

/// tsk799: an effect that writes another provider's item doesn't hear its
/// own write again when the next read restates it. The write's
/// `work_item.recorded` is caused by the effect's run (the loop guard
/// sees it); a read that brings back the same item records nothing new,
/// so nothing echoes — sync after sync.
#[tokio::test(flavor = "multi_thread")]
async fn an_effect_doesnt_hear_its_own_external_write_echoed_by_a_read() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let dir = project.join("oxplow/extensions").join(EXT);
    let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        format!("{manifest}effects:\n  - id: shout\n    summary: Shout a recorded item's title.\n    on: [work_item.recorded]\n    entry: shout.star\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("shout.star"),
        "def transform(x):\n    item = x[\"event\"][\"payload\"][\"item\"]\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": item[\"ref\"], \"title\": item[\"title\"] + \"!\"}}]}\n",
    )
    .unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    let config = fx.svc.config.read().unwrap().clone();
    let decl = &ext.effects[0];
    let effect = crate::effects::effect_program(&ext, decl);
    exec_consent::approve_program(
        &fx.svc.approvals,
        &project,
        &config,
        std::slice::from_ref(&ext),
        ProgramKind::Effect,
        &effect.name,
        &effect.hash(&project).unwrap(),
    )
    .unwrap();
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    first_read(&fx).await;
    crate::effects::approved(&fx.svc.db, &decl.name())
        .await
        .unwrap();

    let consumer = crate::effect_triggers::EffectTriggers::new(std::sync::Arc::downgrade(&fx.svc));
    let mut seen = fx
        .svc
        .event_log_store
        .read_after(0, 10_000)
        .await
        .unwrap()
        .len() as i64;
    // Hand the consumer what was logged since last time, as the pump would.
    let deliver = |seen: i64| {
        let (svc, consumer) = (&fx.svc, &consumer);
        async move {
            let mut at = seen;
            loop {
                let events = svc.event_log_store.read_after(at, 100).await.unwrap();
                if events.is_empty() {
                    return at;
                }
                for e in &events {
                    use crate::event_pump::AsyncEventConsumer as _;
                    consumer.handle(e).await.unwrap();
                    at = e.seq;
                }
            }
        }
    };
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
    seen = deliver(seen).await;
    for _ in 0..3 {
        fx.svc
            .commands
            .run(
                &Actor::Human,
                sync::SYNC,
                json!({ "instance": INSTANCE }),
                false,
            )
            .await
            .unwrap();
        seen = deliver(seen).await;
    }
    let runs = fx
        .svc
        .sql
        .query_sql("SELECT count(*) FROM v_effect_run", vec![], None)
        .await
        .unwrap()
        .rows;
    assert_eq!(
        serde_json::to_value(runs).unwrap(),
        json!([[1]]),
        "one reaction, to the person's create"
    );
    let titles: Vec<String> = logged(&fx, "work_item.recorded")
        .await
        .iter()
        .filter(|p| p["item"]["ref"] == item.as_str())
        .map(|p| p["item"]["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles.last().map(String::as_str), Some("theirs!"));
}

/// Another instance of the tracker's `provider` (in memory, as a
/// reconcile reads it).
fn configure_named(
    fx: &EffortFixture,
    instance: &str,
    provider: Option<&str>,
    config: serde_json::Value,
) {
    fx.svc.config.write().unwrap().extension_instances.insert(
        instance.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config,
            sync_minutes: Some(0),
            provider: provider.map(str::to_string),
        },
    );
}

const SECOND: &str = "tracker/fake_second";

/// tsk841: removing an instance — here one whose provider is gone, which
/// is when the row offers Remove — takes everything of it: its config,
/// the active-provider choice naming it, its read checkpoints (an
/// instance added again under the id starts fresh), its health row and
/// its credentials, read from the copy it last ran when the extension no
/// longer declares it.
#[tokio::test]
async fn removing_an_instance_leaves_nothing_behind() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    let manifest = project
        .join("oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}    credentials: [FAKE_TOKEN]\n")).unwrap();
    approve(&fx, &extension(&project));
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SECOND, "fake", Scope::Project)
        .await
        .unwrap();
    providers
        .set_credential(SECOND, "FAKE_TOKEN", Some("t0ken"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SECOND, true, json!({ "team": "second" }))
        .await
        .unwrap();
    let states = oxplow_db::SqliteProviderCollectorStore::new(fx.svc.db.clone());
    for _ in 0..200 {
        if states
            .get(SECOND, "work_items")
            .await
            .unwrap()
            .is_some_and(|s| s.status != "reading")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    fx.svc
        .commands
        .run(
            &Actor::Human,
            crate::commands::config_commands::SET,
            json!({ "key": "activeProviders", "value": { "work_items": "fake_second" } }),
            true,
        )
        .await
        .unwrap();
    let account = crate::collector_runner::instance_credential_account(
        &fx.svc.providers.deps.project,
        EXT,
        "fake_second",
        "FAKE_TOKEN",
    );
    assert!(fx.svc.secrets.get(&account).unwrap().is_some());

    // Its provider goes from the extension.
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace("- id: fake", "- id: fake_renamed")).unwrap();
    providers.reconcile().await;
    providers
        .remove_instance(&Actor::Human, SECOND)
        .await
        .unwrap();

    let config = fx.svc.config.read().unwrap().clone();
    assert!(!config.extension_instances.contains_key(SECOND));
    assert_eq!(config.active_providers.get("work_items"), None);
    assert_eq!(states.for_instance(SECOND).await.unwrap(), vec![]);
    assert_eq!(
        oxplow_db::plugin_health_store::SqlitePluginHealthStore::new(fx.svc.db.clone())
            .get(&registry::plugin_key(SECOND))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        fx.svc.secrets.get(&account).unwrap(),
        None,
        "its credential went"
    );

    // A global instance of an extension this project has, that doesn't
    // resolve, is listed — so it can be removed here.
    let global = fx.svc.layout.state_dir.join("global-config");
    oxplow_config::GlobalInstances::update(&global, |all| {
        all.insert(
            "tracker/fake_lost".into(),
            oxplow_config::ExtensionInstanceConfig {
                enabled: true,
                provider: Some("fake".into()),
                ..Default::default()
            },
        );
        Ok::<_, ()>(())
    })
    .unwrap()
    .unwrap();
    providers.reconcile().await;
    let lost = providers
        .list()
        .await
        .into_iter()
        .find(|v| v.instance == "tracker/fake_lost")
        .expect("listed");
    assert!(
        matches!(lost.health.state, InstanceState::Missing { .. }),
        "{:?}",
        lost.health.state
    );
    providers
        .remove_instance(&Actor::Human, "tracker/fake_lost")
        .await
        .unwrap();
    assert!(oxplow_config::GlobalInstances::load(&global)
        .unwrap()
        .instances
        .is_empty());
}

/// tsk840: an instance oxplow can't run as configured — its id is a
/// command namespace something else already owns — says so on its row,
/// rather than showing enabled and "Off" with no reason.
#[tokio::test]
async fn a_refused_enable_says_why() {
    let (fx, _ext) = approved("").await;
    fx.svc
        .commands
        .register_namespace("fake_second", "a test", Vec::new())
        .unwrap();
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    fx.svc.providers.reconcile().await;
    match fx.svc.providers.health(SECOND).map(|h| h.state) {
        Some(InstanceState::Refused { reason }) => {
            assert!(
                reason.contains("`fake_second` is already a test's"),
                "{reason}"
            )
        }
        other => panic!("{other:?}"),
    }
    // A reserved id is refused when it's added.
    let refused = fx
        .svc
        .providers
        .add_instance(&Actor::Human, "tracker/code", "fake", Scope::Project)
        .await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.to_string().contains("`code` is reserved for oxplow")),
        "{refused:?}"
    );
}

/// tsk839: instances of one program start at once — each makes its
/// approved copy. None removes another's copy under way (a temp folder
/// it is about to keep) or the one another kept: each start gets an
/// intact copy, and one copy of the program is left.
#[tokio::test(flavor = "multi_thread")]
async fn starts_of_one_program_at_once_each_get_their_copy() {
    let (fx, ext) = approved("").await;
    let spec = ext.providers[0].clone();
    let deps = fx.svc.providers.deps.clone();
    let starts: Vec<_> = (0..12)
        .map(|_| {
            let (deps, ext, spec) = (deps.clone(), ext.clone(), spec.clone());
            tokio::task::spawn_blocking(move || {
                host::approved_copy(
                    &deps.project_dir,
                    &deps.copies,
                    &deps.approvals,
                    &ext,
                    &spec,
                )
                .map(|copy| copy.ext_dir)
            })
        })
        .collect();
    for start in starts {
        let dir = start.await.unwrap().expect("each start gets its copy");
        assert!(dir.join("extension.yaml").is_file(), "{}", dir.display());
    }
    let kept: Vec<_> = std::fs::read_dir(deps.copies.join(EXT).join("fake"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
}

/// tsk837: a person's Enables on two rows at once — the second while the
/// first still checks — both stand. Each write is a read-modify-write of
/// the instances as they are when it writes, not as they were before its
/// check.
#[tokio::test]
async fn enabling_two_instances_at_once_keeps_both() {
    let (fx, _ext) = approved("slow-check:400").await;
    configure(&fx, false, json!({ "team": "core" }));
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    fx.svc
        .config
        .write()
        .unwrap()
        .extension_instances
        .get_mut(SECOND)
        .unwrap()
        .enabled = false;
    let providers = fx.svc.providers.clone();
    let first = tokio::spawn(async move {
        providers
            .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    fx.svc
        .providers
        .set_instance(&Actor::Human, SECOND, true, json!({ "team": "second" }))
        .await
        .unwrap();
    first.await.unwrap().unwrap();
    let written = fx.svc.config.read().unwrap().extension_instances.clone();
    for name in [INSTANCE, SECOND] {
        assert!(written[name].enabled, "{name}: {written:?}");
    }
    assert_eq!(written[SECOND].provider.as_deref(), Some("fake"));
}

/// P9.B1: two instances of one provider run side by side — one approved
/// program, two processes — each under its own id: its refs' segment, its
/// commands' namespace, its `v_capability_provider` row, its health.
#[tokio::test]
async fn two_instances_of_one_provider_run_side_by_side() {
    let (fx, _ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    fx.svc.providers.reconcile().await;
    for name in [INSTANCE, SECOND] {
        assert_eq!(
            fx.svc.providers.health(name).map(|h| h.state),
            Some(InstanceState::Ready),
            "{name}"
        );
    }
    assert_eq!(
        fx.svc.commands.namespace_owner("fake_second").as_deref(),
        Some("provider:tracker/fake_second")
    );
    assert!(fx.svc.commands.spec("fake.estimate").is_some());
    assert!(fx.svc.commands.spec("fake_second.estimate").is_some());

    let items = fx.svc.work_items_client();
    let new = |provider: &str| crate::work_items::NewItem {
        provider: Some(provider.into()),
        title: "theirs".into(),
        ..Default::default()
    };
    // Each instance numbers its own items: the instance is in the ref.
    assert_eq!(
        items
            .create(&Actor::Human, new("fake_second"))
            .await
            .unwrap(),
        "work_item:fake_second:W-1"
    );
    assert_eq!(
        items.create(&Actor::Human, new("fake")).await.unwrap(),
        "work_item:fake:W-1"
    );
    let listed = fx
        .svc
        .sql
        .query_sql(
            "SELECT provider FROM v_capability_provider WHERE extension = 'tracker' ORDER BY provider",
            vec![],
            None,
        )
        .await
        .unwrap()
        .rows;
    assert_eq!(
        serde_json::to_value(listed).unwrap(),
        json!([["fake"], ["fake_second"]])
    );
    let views = fx.svc.providers.list().await;
    let second = views.iter().find(|v| v.instance == SECOND).unwrap();
    assert_eq!(
        (second.provider.as_str(), second.instance_id.as_str()),
        ("fake", "fake_second")
    );
    assert!(second.approved, "one approval: the program's");

    // Stopping one leaves the other.
    assert!(fx.svc.providers.stop(SECOND).await);
    assert!(fx.svc.commands.spec("fake_second.estimate").is_none());
    assert!(fx.svc.commands.spec("fake.estimate").is_some());
    assert!(items.create(&Actor::Human, new("fake")).await.is_ok());
}

/// P9.B1: a credential is an instance's — set for one, another instance
/// of the same provider runs without it.
#[tokio::test]
async fn an_instance_credential_is_its_own() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    let manifest = project
        .join("oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}    credentials: [FAKE_TOKEN]\n")).unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    fx.svc
        .providers
        .set_credential(SECOND, "FAKE_TOKEN", Some("s3cret"))
        .unwrap_err(); // not configured yet: nothing to set it on
    configure(&fx, true, json!({ "team": "core" }));
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    fx.svc
        .providers
        .set_credential(SECOND, "FAKE_TOKEN", Some("s3cret"))
        .unwrap();
    let undeclared = fx
        .svc
        .providers
        .set_credential(SECOND, "OTHER", Some("x"))
        .unwrap_err();
    assert!(
        undeclared.to_string().contains("FAKE_TOKEN"),
        "{undeclared}"
    );
    fx.svc.providers.reconcile().await;
    assert_eq!(
        fx.svc.providers.health(SECOND).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    match fx.svc.providers.health(INSTANCE).map(|h| h.state) {
        Some(InstanceState::Unconfigured { problems }) => {
            assert_eq!(problems[0].path, "/credentials/FAKE_TOKEN");
        }
        other => panic!("the default instance has no token: {other:?}"),
    }
    let views = fx.svc.providers.list().await;
    let set = |instance: &str| {
        views
            .iter()
            .find(|v| v.instance == instance)
            .unwrap()
            .credentials
            .iter()
            .map(|c| (c.name.clone(), c.set))
            .collect::<Vec<_>>()
    };
    assert_eq!(set(SECOND), vec![("FAKE_TOKEN".to_string(), true)]);
    assert_eq!(set(INSTANCE), vec![("FAKE_TOKEN".to_string(), false)]);
}

/// P9.B1: an instance whose id isn't a provider's says which provider it
/// is; one that doesn't is listed missing, with the fix.
#[tokio::test]
async fn a_named_instance_says_which_provider_it_is() {
    let (fx, _ext) = approved("").await;
    configure_named(&fx, SECOND, None, json!({ "team": "second" }));
    fx.svc.providers.reconcile().await;
    match fx.svc.providers.health(SECOND).map(|h| h.state) {
        Some(InstanceState::Missing { reason }) => {
            assert!(
                reason.contains("provider: fake") && reason.contains("fake_second"),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(fx.svc.commands.spec("fake_second.estimate").is_none());

    // A person adds one by naming the provider; a provider the extension
    // doesn't declare, or an id already taken, is refused.
    let providers = &fx.svc.providers;
    let refused = providers
        .add_instance(&Actor::Human, "tracker/acme", "nope", Scope::Project)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("fake"),
        "names what it declares: {refused}"
    );
    let added = providers
        .add_instance(&Actor::Human, "tracker/acme", "fake", Scope::Project)
        .await
        .unwrap();
    assert_eq!(
        (
            added.provider.as_str(),
            added.instance_id.as_str(),
            added.enabled
        ),
        ("fake", "acme", false)
    );
    let taken = providers
        .add_instance(&Actor::Human, "tracker/acme", "fake", Scope::Project)
        .await
        .unwrap_err();
    assert!(taken.to_string().contains("already"), "{taken}");
    // An agent can't: instances are a person's (`extensionInstances`).
    let agent = Actor::Agent {
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    assert!(providers
        .add_instance(&agent, "tracker/other", "fake", Scope::Project)
        .await
        .is_err());
    providers
        .remove_instance(&Actor::Human, "tracker/acme")
        .await
        .unwrap();
    assert!(providers
        .list()
        .await
        .iter()
        .all(|v| v.instance != "tracker/acme"));
}

/// Another project on the same machine as `fx`: its own repo and
/// database, the same global config dir and keychain. The tracker
/// extension (running the fake with `hooks`) is installed and approved
/// there unless `with_tracker` is false.
async fn sibling_project(
    fx: &EffortFixture,
    hooks: &str,
    with_tracker: bool,
) -> (std::sync::Arc<crate::Services>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    if with_tracker {
        write_extension(dir.path(), hooks);
    }
    let global = fx.svc.layout.state_dir.join("global-config");
    let svc = std::sync::Arc::new(
        crate::Services::in_memory_on_machine(
            dir.path(),
            global,
            fx.svc.secrets.clone(),
            crate::providers::host::process_env(),
        )
        .unwrap(),
    );
    if with_tracker {
        approve_in(&svc);
    }
    (svc, dir)
}

/// Approve the tracker's provider program as it is in `svc`'s project.
fn approve_in(svc: &crate::Services) {
    let ext = extension(&svc.layout.project_dir);
    let config = svc.config.read().unwrap().clone();
    let program = exec_consent::list(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
        std::slice::from_ref(&ext),
    )
    .into_iter()
    .find(|p| p.kind == ProgramKind::Provider)
    .unwrap();
    exec_consent::approve_program(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
        std::slice::from_ref(&ext),
        ProgramKind::Provider,
        &program.name,
        program.version.as_deref().unwrap(),
    )
    .unwrap();
}

/// The tracker's provider in `root` declares the credential `FAKE_TOKEN`.
fn declare_token(root: &Path) {
    let manifest = root
        .join("oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}    credentials: [FAKE_TOKEN]\n")).unwrap();
}

/// tsk842: a global instance's credential is the person's in every
/// project, so changing it in one restarts it in every other — on their
/// next tick — not only here: a rotated token must never keep running on
/// the old value elsewhere.
#[tokio::test]
async fn a_global_credential_change_restarts_it_in_every_project() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    declare_token(&project);
    approve(&fx, &extension(&project));
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("one"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    let (other, _other_dir) = sibling_project(&fx, "needs:FAKE_TOKEN", false).await;
    write_extension(&other.layout.project_dir, "needs:FAKE_TOKEN");
    declare_token(&other.layout.project_dir);
    approve_in(&other);
    other.providers.reconcile().await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));
    let before = other.providers.get(SHARED).await.unwrap();
    assert!(
        !other.providers.reconcile_if_global_changed().await,
        "nothing changed"
    );

    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("two"))
        .unwrap();
    providers.credential_changed(SHARED).await;
    assert!(other.providers.reconcile_if_global_changed().await);
    let after = other.providers.get(SHARED).await.unwrap();
    assert!(
        !std::sync::Arc::ptr_eq(&before, &after),
        "it restarted there"
    );
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));
}

const SHARED: &str = "tracker/fake_shared";

async fn state_of(svc: &crate::Services, instance: &str) -> Option<InstanceState> {
    svc.providers
        .list()
        .await
        .into_iter()
        .find(|v| v.instance == instance)
        .map(|v| v.health.state)
}

/// P9.B2: a global instance belongs to the person, not a project: it runs
/// in every project of this machine that has its extension, and isn't
/// listed where the extension isn't.
#[tokio::test]
async fn a_global_instance_runs_in_every_project_with_the_extension() {
    let (fx, _ext) = approved("").await;
    let providers = &fx.svc.providers;
    let added = providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    assert_eq!((added.scope, added.overridden), (Scope::Global, false));
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    // It is in the machine's file, not the project's.
    assert!(fx.svc.config.read().unwrap().extension_instances.is_empty());
    let global = fx.svc.layout.state_dir.join("global-config");
    let file = oxplow_config::GlobalInstances::load(&global).unwrap();
    assert!(file.instances[SHARED].enabled);
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));

    let (other, _other_dir) = sibling_project(&fx, "", true).await;
    other.providers.reconcile().await;
    let there = other
        .providers
        .list()
        .await
        .into_iter()
        .find(|v| v.instance == SHARED)
        .expect("listed where its extension is");
    assert_eq!(
        (there.scope, there.health.state),
        (Scope::Global, InstanceState::Ready)
    );
    assert!(other.commands.spec("fake_shared.estimate").is_some());

    let (bare, _bare_dir) = sibling_project(&fx, "", false).await;
    bare.providers.reconcile().await;
    assert_eq!(
        state_of(&bare, SHARED).await,
        None,
        "no such extension there"
    );

    // An agent has no path to the machine's file.
    let agent = Actor::Agent {
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    assert!(providers
        .add_instance(&agent, "tracker/fake_other", "fake", Scope::Global)
        .await
        .is_err());
    // Removing it stops it everywhere it is read.
    providers
        .remove_instance(&Actor::Human, SHARED)
        .await
        .unwrap();
    assert_eq!(state_of(&fx.svc, SHARED).await, None);
    assert!(other.providers.reconcile_if_global_changed().await);
    assert_eq!(state_of(&other, SHARED).await, None);
    assert!(other.commands.spec("fake_shared.estimate").is_none());
}

/// tsk843: a person turns a global instance off in one project from its
/// row: the project gets its own entry, off, with the global one's config
/// — every other project keeps running it, and removing the entry brings
/// the global one back here.
#[tokio::test]
async fn a_global_instance_is_turned_off_in_one_project() {
    let (fx, _ext) = approved("").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    let (other, _other_dir) = sibling_project(&fx, "", true).await;
    other.providers.reconcile().await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));

    let agent = Actor::Agent {
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    assert!(other.providers.off_here(&agent, SHARED).await.is_err());
    let view = other
        .providers
        .off_here(&Actor::Human, SHARED)
        .await
        .unwrap();
    assert_eq!(
        (view.scope, view.overridden, view.enabled),
        (Scope::Project, true, false)
    );
    assert_eq!(view.config, json!({ "team": "shared" }));
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Off));
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));
    // Only a global instance not already replaced here.
    assert!(other
        .providers
        .off_here(&Actor::Human, SHARED)
        .await
        .is_err());

    other
        .providers
        .remove_instance(&Actor::Human, SHARED)
        .await
        .unwrap();
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));
}

/// P9.B2: a project's entry of the same name replaces the global one
/// there, whole — and only there.
#[tokio::test]
async fn a_project_entry_overrides_a_global_instance_whole() {
    let (fx, _ext) = approved("").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    let (other, _other_dir) = sibling_project(&fx, "", true).await;
    other.providers.reconcile().await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));

    // The other project turns it off for itself.
    other.config.write().unwrap().extension_instances.insert(
        SHARED.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: false,
            provider: Some("fake".into()),
            ..Default::default()
        },
    );
    other.providers.reconcile().await;
    let there = other
        .providers
        .list()
        .await
        .into_iter()
        .find(|v| v.instance == SHARED)
        .unwrap();
    assert_eq!(
        (
            there.scope,
            there.overridden,
            there.enabled,
            there.health.state
        ),
        (Scope::Project, true, false, InstanceState::Off)
    );
    assert_eq!(there.config, json!({}), "the entry replaces it whole");
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));
}

/// tsk838: a project's entry is the project's own — its config and its
/// credentials — whether or not the person has a global instance of the
/// same name. One appearing (added in another project) or going doesn't
/// move this project's instance onto the global credentials, nor take its
/// own away.
#[tokio::test]
async fn a_project_instance_keeps_its_credentials_beside_a_global_one() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    let manifest = project
        .join("oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}    credentials: [FAKE_TOKEN]\n")).unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Project)
        .await
        .unwrap();
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("the project's"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "core" }))
        .await
        .unwrap();
    let account = crate::collector_runner::instance_credential_account(
        &fx.svc.providers.deps.project,
        EXT,
        "fake_shared",
        "FAKE_TOKEN",
    );

    // In another project the person adds a global one of the same name.
    let global = fx.svc.layout.state_dir.join("global-config");
    oxplow_config::GlobalInstances::update(&global, |all| {
        all.insert(
            SHARED.into(),
            oxplow_config::ExtensionInstanceConfig {
                enabled: true,
                provider: Some("fake".into()),
                ..Default::default()
            },
        );
        Ok::<_, ()>(())
    })
    .unwrap()
    .unwrap();
    let here = |views: Vec<ProviderInstanceView>| {
        views.into_iter().find(|v| v.instance == SHARED).unwrap()
    };
    let view = here(providers.list().await);
    assert_eq!((view.scope, view.overridden), (Scope::Project, true));
    // Started again, it is on its own token still.
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "core" }))
        .await
        .expect("its own credential is set");
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));

    // The global one goes: nothing of this project's goes with it.
    oxplow_config::GlobalInstances::update(&global, |all| {
        all.remove(SHARED);
        Ok::<_, ()>(())
    })
    .unwrap()
    .unwrap();
    let view = here(providers.list().await);
    assert_eq!((view.scope, view.overridden), (Scope::Project, false));
    assert_eq!(
        fx.svc.secrets.get(&account).unwrap().as_deref(),
        Some("the project's")
    );
}

/// P9.B2: a global instance's credential is the person's, set once for
/// every project; a project instance's stays that project's.
#[tokio::test]
async fn a_global_instances_credential_is_shared_across_projects() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let credentialed = |root: &Path| {
        let manifest = root
            .join("oxplow/extensions")
            .join(EXT)
            .join("extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, format!("{text}    credentials: [FAKE_TOKEN]\n")).unwrap();
    };
    write_extension(&project, "needs:FAKE_TOKEN");
    credentialed(&project);
    let ext = extension(&project);
    approve(&fx, &ext);
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("s3cret"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));

    // Another project: the same extension, approved there; nothing set there.
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    write_extension(dir.path(), "needs:FAKE_TOKEN");
    credentialed(dir.path());
    let global = fx.svc.layout.state_dir.join("global-config");
    let other = std::sync::Arc::new(
        crate::Services::in_memory_on_machine(
            dir.path(),
            global,
            fx.svc.secrets.clone(),
            crate::providers::host::process_env(),
        )
        .unwrap(),
    );
    let other_ext = extension(dir.path());
    let config = other.config.read().unwrap().clone();
    let program = exec_consent::list(
        &other.approvals,
        dir.path(),
        &config,
        std::slice::from_ref(&other_ext),
    )
    .into_iter()
    .find(|p| p.kind == ProgramKind::Provider)
    .unwrap();
    exec_consent::approve_program(
        &other.approvals,
        dir.path(),
        &config,
        std::slice::from_ref(&other_ext),
        ProgramKind::Provider,
        &program.name,
        program.version.as_deref().unwrap(),
    )
    .unwrap();
    // A project instance of the same provider there has no token of its own.
    other.config.write().unwrap().extension_instances.insert(
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config: json!({ "team": "core" }),
            ..Default::default()
        },
    );
    other.providers.reconcile().await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));
    assert!(matches!(
        state_of(&other, INSTANCE).await,
        Some(InstanceState::Unconfigured { .. })
    ));
}

/// tsk820: a call's outcome is its instance's while that instance runs.
/// One that finishes after the instance was stopped — the first read an
/// enable starts, say — changes nothing: not "Ready" for something that
/// isn't running, not a failure counted against it.
#[tokio::test]
async fn a_call_finishing_after_its_instance_stopped_changes_nothing() {
    let (fx, ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    first_read(&fx).await;
    let was_running = fx.svc.providers.get(INSTANCE).await.unwrap();
    configure(&fx, false, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    let state = || {
        fx.svc
            .providers
            .health(INSTANCE)
            .map(|h| (h.state, h.consecutive_failures))
    };
    assert_eq!(state(), Some((InstanceState::Off, 0)));

    fx.svc
        .providers
        .call_succeeded(&was_running, std::time::Duration::from_millis(5))
        .await;
    assert_eq!(state(), Some((InstanceState::Off, 0)), "a late success");
    fx.svc
        .providers
        .call_failed(&was_running, "the process went away".into())
        .await;
    assert_eq!(state(), Some((InstanceState::Off, 0)), "a late failure");

    // Running again, it is a new instance: the old one's late results
    // still aren't its.
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    first_read(&fx).await;
    fx.svc
        .providers
        .call_failed(&was_running, "the old process went away".into())
        .await;
    assert_eq!(state(), Some((InstanceState::Ready, 0)));
}

/// tsk836: a stopped instance never starts again. A read still holding
/// it — a `provider.sync` looping over collectors when a person turned
/// it off and on — doesn't restart its process, and whatever that late
/// read meets is never the running successor's: here the extension's
/// folder changed since its approval, which a restart of the old one
/// would report as `unapproved` and stop the healthy successor for.
#[tokio::test]
async fn a_late_read_on_a_stopped_instance_doesnt_touch_its_successor() {
    let (fx, _ext) = approved("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    first_read(&fx).await;
    let was_running = fx.svc.providers.get(INSTANCE).await.unwrap();
    configure(&fx, false, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    let successor = fx.svc.providers.get(INSTANCE).await.unwrap();
    assert!(!std::sync::Arc::ptr_eq(&was_running, &successor));
    // The folder isn't what was approved any more.
    write_extension(&fx.svc.layout.project_dir, "progress");

    let late = was_running.read(&Actor::Human, "work_items").await;
    assert!(late.is_err(), "a stopped instance doesn't read: {late:?}");
    let still = fx.svc.providers.get(INSTANCE).await.expect("still running");
    assert!(std::sync::Arc::ptr_eq(&still, &successor));
    assert_eq!(
        fx.svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    // Nothing of it came back to life.
    assert!(!was_running.has_process().await);
}

/// The tracker extension, its provider's credential `FAKE_TOKEN` obtained
/// by signing in at `authorize` / `token` (P9.B3), plus `more` credential
/// entries (YAML list items, indented six spaces).
fn write_oauth_extension(project: &Path, hooks: &str, authorize: &str, token: &str, more: &str) {
    write_extension(project, hooks);
    let manifest = project
        .join("oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!(
            "{text}    credentials:\n      - name: FAKE_TOKEN\n        oauth:\n          authorize_url: {authorize}\n          token_url: {token}\n          client_id: oxplow-test\n          scopes: [read, write]\n{more}"
        ),
    )
    .unwrap();
}

/// P9.B3: a credential may be one the person signs in for. It is declared
/// with its endpoints, checked at load, and what approving the provider
/// shows.
#[tokio::test]
async fn an_oauth_credential_is_declared_checked_and_shown_for_approval() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let (authorize, token) = (
        "https://auth.example.com/authorize",
        "https://auth.example.com/token",
    );
    write_oauth_extension(&project, "", authorize, token, "      - PLAIN\n");
    let ext = extension(&project);
    let spec = &ext.providers[0];
    assert_eq!(spec.credential_names(), vec!["FAKE_TOKEN", "PLAIN"]);
    let oauth = spec.credentials[0].oauth.as_ref().expect("signs in");
    assert_eq!(
        (
            oauth.authorize_url.as_str(),
            oauth.client_id.as_str(),
            oauth.scopes.len()
        ),
        (authorize, "oxplow-test", 2)
    );
    assert!(
        spec.credentials[1].oauth.is_none(),
        "a bare name is a static one"
    );
    // What a person approves names where it signs in.
    let listed = program(&fx, &ext);
    assert_eq!(
        listed.credentials,
        vec![
            format!("FAKE_TOKEN (signs in at {authorize}; tokens from {token}; client oxplow-test; scopes read, write)"),
            "PLAIN".to_string()
        ]
    );
    let before = listed.version.clone();
    write_oauth_extension(
        &project,
        "",
        authorize,
        "https://auth.example.com/token2",
        "      - PLAIN\n",
    );
    assert_ne!(program(&fx, &extension(&project)).version, before);

    let refused = |authorize: &str, more: &str, says: &str| {
        write_oauth_extension(&project, "", authorize, token, more);
        let loaded = crate::extensions::load_extensions(&project)
            .into_iter()
            .find(|e| e.name == EXT)
            .unwrap();
        assert!(loaded.providers.is_empty(), "{says}");
        assert!(
            loaded.errors.iter().any(|e| e.contains(says)),
            "{says}: {:?}",
            loaded.errors
        );
    };
    // The token never crosses the network in the clear.
    refused("http://auth.example.com/authorize", "", "must be https");
    refused(authorize, "      - FAKE_TOKEN\n", "declared twice");
    // A client secret is another credential's value, by name — a static one.
    let secret_of = |name: &str| {
        format!("      - name: OTHER\n        oauth: {{ authorize_url: {authorize}, token_url: {token}, client_id: x, client_secret: {name} }}\n")
    };
    refused(authorize, &secret_of("NOPE"), "`client_secret: NOPE`");
    refused(
        authorize,
        &secret_of("FAKE_TOKEN"),
        "`client_secret: FAKE_TOKEN`",
    );
    refused(
        authorize,
        "      - name: BAD\n        oauth: { authorize_url: x }\n",
        "token_url",
    );
    // On loopback, plain http is what a local service speaks.
    write_oauth_extension(
        &project,
        "",
        "http://127.0.0.1:9/authorize",
        "http://localhost:9/token",
        "",
    );
    extension(&project);
}

/// The fixture with the tracker extension approved, its `FAKE_TOKEN` one
/// the person signs in for at a stand-in authorization server (`more`:
/// further lines of the manifest, continuing the `oauth:` block).
async fn signing_in(hooks: &str, more: &str) -> (EffortFixture, OAuthSim) {
    let fx = services_with_effort().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    write_oauth_extension(&project, hooks, &sim.authorize_url, &sim.token_url, more);
    approve(&fx, &extension(&project));
    (fx, sim)
}

/// The port the shell catches a test sign-in's redirect on. Nothing
/// listens: the redirect is read off the authorization server's answer
/// and handed over as the shell would.
const REDIRECT_PORT: u16 = 8124;

/// What the person's browser does with a sign-in's page: the redirect it
/// is sent back with, as the shell catches it (its path and query).
async fn browse(page: &str) -> String {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = client.get(page).send().await.unwrap();
    let to = url::Url::parse(resp.headers()["location"].to_str().unwrap()).unwrap();
    format!("{}?{}", to.path(), to.query().unwrap_or_default())
}

/// Hand `redirect` to the sign-in for `name`, as the shell does.
async fn complete(
    fx: &EffortFixture,
    name: &str,
    redirect: &str,
) -> Result<SignInCompletion, DomainError> {
    fx.svc
        .providers
        .complete_sign_in(INSTANCE, name, redirect)
        .await
}

/// tsk909: a sign-in stores its token to the account it began for. When
/// the instance's entry changes under it — a global one turned off here,
/// so the project's own entry replaces it — the redirect is refused and
/// nothing is stored to the old account.
#[tokio::test]
async fn a_sign_in_whose_account_changed_meanwhile_is_refused() {
    let (fx, sim) = signing_in("", "").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    let page = providers
        .begin_sign_in(SHARED, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    providers.off_here(&Actor::Human, SHARED).await.unwrap();
    let redirect = browse(&page).await;
    let done = providers
        .complete_sign_in(SHARED, "FAKE_TOKEN", &redirect)
        .await;
    assert!(
        matches!(
            &done,
            Ok(SignInCompletion::Failed { error }) if error.contains("sign in again")
        ),
        "{done:?}"
    );
    assert!(
        !sim.grants().contains(&"authorization_code".to_string()),
        "no code was exchanged: {:?}",
        sim.grants()
    );
}

/// Start a sign-in, do what the person's browser does, and hand the
/// redirect over as the shell does. What the renderer then hears.
async fn sign_in(fx: &EffortFixture, name: &str) -> Option<String> {
    let mut ui = fx.svc.events.subscribe_ui();
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, name, REDIRECT_PORT)
        .await
        .expect("the sign-in begins");
    let completion = complete(fx, name, &browse(&page).await)
        .await
        .expect("it is under way");
    let heard = async {
        loop {
            if let Ok(crate::events::OxplowEvent::CredentialChanged {
                instance,
                name: of,
                error,
            }) = ui.recv().await
            {
                assert_eq!((instance.as_str(), of.as_str()), (INSTANCE, name));
                return error;
            }
        }
    };
    let error = tokio::time::timeout(std::time::Duration::from_secs(20), heard)
        .await
        .expect("the renderer hears the sign-in finished");
    assert_eq!(
        completion,
        match &error {
            None => SignInCompletion::SignedIn,
            Some(error) => SignInCompletion::Failed {
                error: error.clone()
            },
        }
    );
    error
}

fn sign_in_state(views: &[ProviderInstanceView], name: &str) -> (bool, Option<oauth::SignInState>) {
    let c = views
        .iter()
        .find(|v| v.instance == INSTANCE)
        .unwrap()
        .credentials
        .iter()
        .find(|c| c.name == name)
        .unwrap();
    (c.set, c.sign_in.clone())
}

fn unconfigured_at(fx: &EffortFixture) -> (String, String) {
    match fx.svc.providers.health(INSTANCE).map(|h| h.state) {
        Some(InstanceState::Unconfigured { problems }) => {
            (problems[0].path.clone(), problems[0].message.clone())
        }
        other => panic!("expected it unconfigured: {other:?}"),
    }
}

/// P9.B3: a credential the person signs in for. Until they have, the
/// instance is unconfigured naming it; signing in keeps the token in the
/// keychain and starts the instance on the access token alone.
#[tokio::test]
async fn signing_in_starts_the_instance_on_the_access_token_alone() {
    // The fake's service takes the first access token the stand-in issues
    // and nothing else — not the keychain's record of it.
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=at-2", "      - PLAIN\n").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));
    providers.reconcile().await;
    let (path, message) = unconfigured_at(&fx);
    assert_eq!(path, "/credentials/FAKE_TOKEN");
    assert!(message.contains("isn't signed in"), "{message}");
    assert!(fx.svc.work_items.get("fake").is_err(), "nothing registered");
    assert_eq!(
        sign_in_state(&providers.list().await, "FAKE_TOKEN"),
        (false, Some(oauth::SignInState::NotSignedIn))
    );
    assert_eq!(
        sign_in_state(&providers.list().await, "PLAIN"),
        (false, None)
    );

    // A signed-in credential is never given a value; a pasted one is
    // never signed in for.
    let refused = providers
        .set_credential(INSTANCE, "FAKE_TOKEN", Some("pasted"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("sign in"), "{refused}");
    let refused = providers
        .begin_sign_in(INSTANCE, "PLAIN", REDIRECT_PORT)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("isn't a credential you sign in for"),
        "{refused}"
    );

    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    assert_eq!(
        providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    create_on_fake(&fx).await.unwrap();
    assert_eq!(sim.grants(), vec!["authorization_code"]);
    assert_eq!(
        sign_in_state(&providers.list().await, "FAKE_TOKEN"),
        (true, Some(oauth::SignInState::SignedIn { until: None }))
    );

    // Forgetting it signs the person out: the instance stops on it.
    providers
        .set_credential(INSTANCE, "FAKE_TOKEN", None)
        .unwrap();
    providers.credential_changed(INSTANCE).await;
    assert_eq!(unconfigured_at(&fx).0, "/credentials/FAKE_TOKEN");
    assert!(fx.svc.work_items.get("fake").is_err());
}

/// P9.B3: a token about to lapse is renewed before the process starts.
#[tokio::test]
async fn a_lapsed_token_is_renewed_before_the_instance_starts() {
    // The service takes only the renewed token.
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=at-3", "").await;
    // Signed in while the instance is off; the token it got has lapsed.
    configure(&fx, false, json!({ "team": "core" }));
    sim.set_ttl(-10);
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    sim.set_ttl(3600);
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc.providers.reconcile().await;
    assert_eq!(
        fx.svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
}

/// P9.B3: a token its service refuses (`Auth`) although it looks good is
/// renewed, the process restarted on the new one, and the call tried once
/// more — none of which counts as a failure.
#[tokio::test]
async fn a_refused_token_is_renewed_and_the_call_tried_once_more() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    // Its service now takes only the next token; the process holds at-2.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
    create_on_fake(&fx).await.unwrap();
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(
        (health.state, health.consecutive_failures),
        (InstanceState::Ready, 0)
    );
    // The process that answered was started on the renewed token.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
    create_on_fake(&fx).await.unwrap();
    assert_eq!(sim.grants().len(), 2);

    // A read is retried the same way.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-4").await;
    fx.svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(sim.grants().len(), 3);
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        0
    );
}

/// tsk907: a read that starts the process itself, and is refused, renews
/// the token — the process it started began after the read did, which
/// isn't "started since" the refusal.
#[tokio::test]
async fn a_read_that_starts_the_process_renews_a_refused_token() {
    // Its service takes only the renewed token; its `check` doesn't look.
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=at-3,lax-check", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    // The process goes away, so the next read starts it.
    set_hooks(&fx, "crash").await;
    let _ = create_on_fake(&fx).await;
    fx.svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(refreshes(&sim), 1, "{:?}", sim.grants());
}

/// Two credentials signed in at the stand-in, `FAKE_TOKEN` and
/// `OTHER_TOKEN`, the instance ready on both.
async fn signed_in_twice(hooks: &str) -> (EffortFixture, OAuthSim) {
    let fx = services_with_effort().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    let other = format!(
        "      - name: OTHER_TOKEN\n        oauth:\n          authorize_url: {}\n          token_url: {}\n          client_id: oxplow-test\n",
        sim.authorize_url, sim.token_url
    );
    write_oauth_extension(&project, hooks, &sim.authorize_url, &sim.token_url, &other);
    approve(&fx, &extension(&project));
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    assert_eq!(sign_in(&fx, "OTHER_TOKEN").await, None);
    first_read(&fx).await;
    (fx, sim)
}

/// tsk910: a slow token exchange holds up only its own credential's
/// sign-in — another credential's begins meanwhile.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_exchange_holds_up_only_its_own_sign_in() {
    let fx = services_with_effort().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    let other = format!(
        "      - name: OTHER_TOKEN\n        oauth:\n          authorize_url: {}\n          token_url: {}\n          client_id: oxplow-test\n",
        sim.authorize_url, sim.token_url
    );
    write_oauth_extension(&project, "", &sim.authorize_url, &sim.token_url, &other);
    approve(&fx, &extension(&project));
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    sim.delay_token_requests(3_000);
    let finishing = {
        let svc = fx.svc.clone();
        tokio::spawn(async move {
            svc.providers
                .complete_sign_in(INSTANCE, "FAKE_TOKEN", &redirect)
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let started = std::time::Instant::now();
    fx.svc
        .providers
        .begin_sign_in(INSTANCE, "OTHER_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "waited {:?} for another credential's exchange",
        started.elapsed()
    );
    assert!(matches!(
        finishing.await.unwrap(),
        Ok(SignInCompletion::SignedIn)
    ));
}

/// tsk906: once a redirect is taken as the sign-in's, the finish runs to
/// its end even when its caller goes away mid-exchange (a dropped
/// connection to a remote daemon): the token is kept and the renderer
/// hears it, rather than the sign-in vanishing.
#[tokio::test(flavor = "multi_thread")]
async fn a_finish_runs_to_its_end_when_its_caller_goes_away() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let mut ui = fx.svc.events.subscribe_ui();
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    sim.delay_token_requests(500);
    let caller = {
        let svc = fx.svc.clone();
        tokio::spawn(async move {
            svc.providers
                .complete_sign_in(INSTANCE, "FAKE_TOKEN", &redirect)
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    caller.abort();
    let heard = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Ok(crate::events::OxplowEvent::CredentialChanged { name, error, .. }) =
                ui.recv().await
            {
                if name == "FAKE_TOKEN" {
                    return error;
                }
            }
        }
    })
    .await
    .expect("the finish ran to its end");
    assert_eq!(heard, None);
    assert!(matches!(
        sign_in_state(&fx.svc.providers.list().await, "FAKE_TOKEN").1,
        Some(oauth::SignInState::SignedIn { .. })
    ));
}

/// tsk906: the browser hears how its sign-in went as soon as the token is
/// kept — not after the instance has restarted on it (a slow `check`).
#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_answers_before_the_instance_restarts() {
    let (fx, _sim) = signing_in("slow-check:2000", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    let started = std::time::Instant::now();
    let done = complete(&fx, "FAKE_TOKEN", &redirect).await.unwrap();
    assert_eq!(done, SignInCompletion::SignedIn);
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1500),
        "answered after {:?}",
        started.elapsed()
    );
}

fn refreshes(sim: &OAuthSim) -> usize {
    sim.grants()
        .iter()
        .filter(|g| *g == "refresh_token")
        .count()
}

/// tsk908: an `Auth` that names nothing, from a provider handed a pasted
/// key beside its sign-in, could be about either: it renews nothing, and
/// the sign-in stays good — the refusal is the call's failure.
#[tokio::test]
async fn an_unnamed_auth_beside_a_pasted_key_renews_nothing() {
    let (fx, sim) = signing_in("", "      - PLAIN\n").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc
        .providers
        .set_credential(INSTANCE, "PLAIN", Some("k3y"))
        .unwrap();
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    set_hooks(&fx, "refuse-auth").await;
    let err = create_on_fake(&fx).await.unwrap_err().to_string();
    assert!(err.contains("authentication failed"), "{err}");
    assert_eq!(refreshes(&sim), 0, "{:?}", sim.grants());
    assert!(
        matches!(
            sign_in_state(&fx.svc.providers.list().await, "FAKE_TOKEN").1,
            Some(oauth::SignInState::SignedIn { .. })
        ),
        "the sign-in stays good"
    );
}

/// tsk821: an `Auth` that names its credential renews that one alone.
#[tokio::test]
async fn an_auth_renews_only_the_credential_it_names() {
    let (fx, sim) = signed_in_twice("").await;
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    // Renewed and tried again on a fresh process (which starts without
    // the hook), so the call lands.
    create_on_fake(&fx).await.unwrap();
    assert_eq!(refreshes(&sim), 1, "FAKE_TOKEN only: {:?}", sim.grants());
}

/// tsk821: renewing the credential an `Auth` named never touches another:
/// when the service refuses the renewal for good, only the named one
/// reads "sign in again".
#[tokio::test]
async fn an_auth_never_lapses_a_credential_it_didnt_name() {
    let (fx, sim) = signed_in_twice("").await;
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    sim.revoke();
    assert!(create_on_fake(&fx).await.is_err());
    let views = fx.svc.providers.list().await;
    assert!(
        matches!(
            sign_in_state(&views, "FAKE_TOKEN").1,
            Some(oauth::SignInState::SignInAgain)
        ),
        "{:?}",
        sign_in_state(&views, "FAKE_TOKEN")
    );
    assert!(
        matches!(
            sign_in_state(&views, "OTHER_TOKEN").1,
            Some(oauth::SignInState::SignedIn { .. })
        ),
        "{:?}",
        sign_in_state(&views, "OTHER_TOKEN")
    );
}

/// tsk821: an `Auth` that names no credential renews one only when the
/// instance signs in for exactly one — with two, it can't know which, so
/// it renews none and the refusal is the call's failure.
#[tokio::test]
async fn an_unnamed_auth_with_two_sign_ins_renews_nothing() {
    let (fx, sim) = signed_in_twice("").await;
    set_hooks(&fx, "refuse-auth").await;
    let err = create_on_fake(&fx).await.unwrap_err().to_string();
    assert!(err.contains("authentication failed"), "{err}");
    assert_eq!(refreshes(&sim), 0, "{:?}", sim.grants());
}

/// tsk821: with one signed-in credential, an unnamed `Auth` is its.
#[tokio::test]
async fn an_unnamed_auth_with_one_sign_in_renews_it() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    set_hooks(&fx, "refuse-auth").await;
    create_on_fake(&fx).await.unwrap();
    assert_eq!(refreshes(&sim), 1, "{:?}", sim.grants());
}

/// P9.B3: renewed once, not for ever — a service that refuses the renewed
/// token too is a failure like any other.
#[tokio::test]
async fn a_token_refused_again_after_renewal_is_a_failure() {
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=never", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    match &health.state {
        InstanceState::Failing { errors } => {
            assert!(errors[0].contains("check"), "{errors:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(health.consecutive_failures, 1);
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
}

/// P9.B3: a sign-in the service revoked can't be renewed: the instance
/// stops, unconfigured, saying to sign in again — never a silent failure,
/// and the keychain's record stays so the row can say so.
#[tokio::test]
async fn a_revoked_sign_in_stops_the_instance_and_says_sign_in_again() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    assert!(fx.svc.work_items.get("fake").is_ok());
    sim.revoke();
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    let err = create_on_fake(&fx).await.unwrap_err().to_string();
    assert!(err.contains("sign in again"), "{err}");
    let (path, message) = unconfigured_at(&fx);
    assert_eq!(path, "/credentials/FAKE_TOKEN");
    assert!(message.contains("sign in again"), "{message}");
    assert!(fx.svc.work_items.get("fake").is_err(), "nothing registered");
    assert!(fx.svc.commands.namespace_owner("fake").is_none());
    assert_eq!(
        sign_in_state(&fx.svc.providers.list().await, "FAKE_TOKEN"),
        (false, Some(oauth::SignInState::SignInAgain))
    );
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        0,
        "the person's to fix, not a failure of the program"
    );
    // Signing in again is all it takes.
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    assert_eq!(
        fx.svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
}

/// P9.B3: a client secret is another credential's value, sent with the
/// token requests by oxplow — the provider's process never holds it.
#[tokio::test]
async fn a_client_secret_goes_to_the_token_endpoint_and_not_to_the_process() {
    // `needs`: the fake's `check` reports the credential if it lacks it.
    let (fx, sim) = signing_in(
        "needs:CLIENT_SECRET",
        "          client_secret: CLIENT_SECRET\n      - CLIENT_SECRET\n",
    )
    .await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));
    let refused = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("set `CLIENT_SECRET` first"), "{refused}");
    providers.reconcile().await;
    assert_eq!(unconfigured_at(&fx).0, "/credentials/CLIENT_SECRET");
    providers
        .set_credential(INSTANCE, "CLIENT_SECRET", Some("s3cret"))
        .unwrap();
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    assert_eq!(sim.secrets(), vec![Some("s3cret".to_string())]);
    // The process started (signed in) without it.
    let (path, message) = unconfigured_at(&fx);
    assert_eq!(path, "/credentials/CLIENT_SECRET");
    assert!(
        message.contains("isn't set"),
        "the fake's own check: {message}"
    );
}

/// P9.B3: starting a sign-in again abandons the one under way — its
/// redirect is no longer the sign-in's, and it stores nothing; removing
/// the instance abandons it too.
#[tokio::test]
async fn a_newer_sign_in_replaces_the_one_under_way() {
    let (fx, sim) = signing_in("", "").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));
    let first = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let second = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    // The first's redirect comes back to the second, which refuses it:
    // its `state` isn't the one the second started with.
    let stale = complete(&fx, "FAKE_TOKEN", &browse(&first).await)
        .await
        .unwrap();
    assert!(
        matches!(stale, SignInCompletion::NotThisSignIn { .. }),
        "{stale:?}"
    );
    assert!(
        sim.grants().is_empty(),
        "no code of the first was exchanged"
    );
    // The second, still waiting, finishes.
    assert_eq!(
        complete(&fx, "FAKE_TOKEN", &browse(&second).await)
            .await
            .unwrap(),
        SignInCompletion::SignedIn
    );
    assert_eq!(sim.grants(), vec!["authorization_code"]);

    // Removing the instance abandons a sign-in under way: its redirect
    // finds none, and nothing is kept for what's gone.
    let third = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    providers
        .remove_instance(&Actor::Human, INSTANCE)
        .await
        .unwrap();
    let refused = complete(&fx, "FAKE_TOKEN", &browse(&third).await)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("under way"), "{refused}");
    assert_eq!(sim.grants(), vec!["authorization_code"]);
    let account = crate::collector_runner::instance_credential_account(
        &fx.svc.providers.deps.project,
        EXT,
        "fake",
        "FAKE_TOKEN",
    );
    assert_eq!(fx.svc.secrets.get(&account).unwrap(), None, "no token kept");
}

/// P10: a redirect that isn't the sign-in's (another `state`: someone
/// else's page sent the browser there) does nothing, and the sign-in
/// waits on for the real one.
#[tokio::test]
async fn a_redirect_with_another_state_is_refused_and_the_wait_goes_on() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let forged = complete(&fx, "FAKE_TOKEN", "/callback?code=stolen&state=forged")
        .await
        .unwrap();
    assert!(
        matches!(&forged, SignInCompletion::NotThisSignIn { reason } if reason.contains("isn't the sign-in")),
        "{forged:?}"
    );
    assert!(sim.grants().is_empty(), "nothing was exchanged");
    assert_eq!(
        complete(&fx, "FAKE_TOKEN", &browse(&page).await)
            .await
            .unwrap(),
        SignInCompletion::SignedIn
    );
}

/// P10: a sign-in ends when it is completed: the same redirect again
/// finds none under way, and its code is exchanged once.
#[tokio::test]
async fn a_completed_sign_in_cannot_be_completed_again() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    assert_eq!(
        complete(&fx, "FAKE_TOKEN", &redirect).await.unwrap(),
        SignInCompletion::SignedIn
    );
    let again = complete(&fx, "FAKE_TOKEN", &redirect)
        .await
        .unwrap_err()
        .to_string();
    assert!(again.contains("under way"), "{again}");
    assert_eq!(sim.grants(), vec!["authorization_code"]);
}

/// P10: finishing re-checks what was approved: a provider whose token
/// endpoint changed since the sign-in began sends its code nowhere.
#[tokio::test]
async fn a_sign_in_whose_endpoints_changed_meanwhile_sends_nothing() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    write_oauth_extension(
        &fx.svc.layout.project_dir,
        "",
        &sim.authorize_url,
        "https://elsewhere.example/token",
        "",
    );
    let failed = complete(&fx, "FAKE_TOKEN", &redirect).await.unwrap();
    assert!(
        matches!(&failed, SignInCompletion::Failed { error } if error.contains("approv")),
        "{failed:?}"
    );
    assert!(
        sim.grants().is_empty(),
        "nothing reached any token endpoint"
    );
}

/// P10: a sign-in never finished ends after its wait: the renderer hears
/// why, and its redirect, coming late, finds none under way.
#[tokio::test]
async fn an_unfinished_sign_in_ends_after_its_wait() {
    let (fx, _sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let mut ui = fx.svc.events.subscribe_ui();
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    tokio::time::pause();
    tokio::time::advance(oauth::SIGN_IN_WAIT + std::time::Duration::from_secs(1)).await;
    let error = loop {
        if let Ok(crate::events::OxplowEvent::CredentialChanged { error, .. }) = ui.recv().await {
            break error;
        }
    };
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("wasn't finished")),
        "{error:?}"
    );
    tokio::time::resume();
    let late = complete(&fx, "FAKE_TOKEN", &redirect)
        .await
        .unwrap_err()
        .to_string();
    assert!(late.contains("under way"), "{late}");
}

/// The adapter manifest with its server reached by url, authenticated by
/// the credential `NOTES_TOKEN`.
fn url_manifest() -> String {
    ADAPTER_MANIFEST.replace(
        "      mcp: { command: [bin/server, --stdio] }\n",
        "      mcp: { url: \"https://mcp.example.com/mcp\", auth: NOTES_TOKEN }\n",
    ) + "    credentials: [NOTES_TOKEN]\n    network: [mcp.example.com]\n"
}

/// P9.B4: an adapter's MCP server may be one reached by `url` instead of
/// a command — https (or loopback), a host its `network` lists, and its
/// bearer a credential it declares.
#[tokio::test]
async fn an_adapter_server_by_url_is_declared_and_checked() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_adapter_extension(&project);
    std::fs::remove_file(dir.join("bin/server")).unwrap();
    let manifest = url_manifest();
    std::fs::write(dir.join("extension.yaml"), &manifest).unwrap();
    let ext = extension(&project);
    assert_eq!(
        ext.providers[0].adapter.as_ref().unwrap().mcp,
        spec::McpServer::Url {
            url: "https://mcp.example.com/mcp".into(),
            auth: Some("NOTES_TOKEN".into()),
        }
    );
    // What a person approves: the url, not a file of the folder.
    let listed = program(&fx, &ext);
    assert!(listed.remote);
    assert_eq!(listed.program, "https://mcp.example.com/mcp");
    assert_eq!(
        listed.args,
        ["mcp/x.star", "mcp/tools.json", "--auth-env", "NOTES_TOKEN"]
    );
    assert!(listed.version.is_some(), "no program file to read");

    for (from, to, says) in [
        (
            "mcp: { url: \"https://mcp.example.com/mcp\", auth: NOTES_TOKEN }",
            "mcp: { url: \"https://mcp.example.com/mcp\", command: [bin/server] }",
            "names either `command`",
        ),
        (
            "mcp: { url: \"https://mcp.example.com/mcp\", auth: NOTES_TOKEN }",
            "mcp: {}",
            "names either `command`",
        ),
        (
            "mcp: { url: \"https://mcp.example.com/mcp\", auth: NOTES_TOKEN }",
            "mcp: { command: [bin/server], auth: NOTES_TOKEN }",
            "`auth` goes with a `url`",
        ),
        // The bearer never crosses the network in the clear.
        (
            "https://mcp.example.com/mcp",
            "http://mcp.example.com/mcp",
            "must be https",
        ),
        (
            "https://mcp.example.com/mcp",
            "ftp://mcp.example.com/mcp",
            "must be https",
        ),
        (
            "network: [mcp.example.com]",
            "network: [other.example.com]",
            "`network` must list `mcp.example.com`",
        ),
        (
            "auth: NOTES_TOKEN }",
            "auth: OTHER }",
            "declares no credential `OTHER`",
        ),
        // tsk835: a client secret is the host's alone — never the
        // process's, so never its bearer either.
        (
            "    credentials: [NOTES_TOKEN]\n",
            "    credentials:\n      - NOTES_TOKEN\n      - name: SIGNED_IN\n        oauth: \
             { authorize_url: \"https://mcp.example.com/authorize\", token_url: \
             \"https://mcp.example.com/token\", client_id: oxplow, client_secret: NOTES_TOKEN }\n",
            "`NOTES_TOKEN` is a client secret",
        ),
        (
            "    adapter:\n",
            "    args: [--x]\n    adapter:\n",
            "`args` go with an `entry`",
        ),
    ] {
        assert!(manifest.contains(from), "{from}");
        std::fs::write(dir.join("extension.yaml"), manifest.replace(from, to)).unwrap();
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
    // On loopback, plain http is what a local server speaks.
    std::fs::write(
        dir.join("extension.yaml"),
        manifest
            .replace("https://mcp.example.com/mcp", "http://127.0.0.1:8123/mcp")
            .replace("network: [mcp.example.com]", "network: [127.0.0.1]"),
    )
    .unwrap();
    extension(&project);
}

/// P9.B4: approving a server by url approves the url, its pins, its
/// mapping and what it is authenticated with: changing any needs
/// approving again.
#[tokio::test]
async fn a_url_servers_approval_covers_the_url_its_pins_mapping_and_credentials() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_adapter_extension(&project);
    std::fs::remove_file(dir.join("bin/server")).unwrap();
    let manifest = url_manifest();
    let approved_now = |fx: &EffortFixture| {
        std::fs::write(dir.join("extension.yaml"), &manifest).unwrap();
        let ext = extension(&project);
        approve(fx, &ext);
        assert!(program(fx, &ext).approved);
    };
    for (from, to) in [
        (
            "https://mcp.example.com/mcp",
            "https://mcp.example.com/other",
        ),
        ("auth: NOTES_TOKEN }", "}"),
        (
            "credentials: [NOTES_TOKEN]",
            "credentials: [NOTES_TOKEN, MORE]",
        ),
    ] {
        approved_now(&fx);
        assert!(manifest.contains(from), "{from}");
        std::fs::write(dir.join("extension.yaml"), manifest.replace(from, to)).unwrap();
        assert!(!program(&fx, &extension(&project)).approved, "{to}");
    }
    for file in ["mcp/x.star", "mcp/tools.json"] {
        approved_now(&fx);
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = match file {
            "mcp/tools.json" => text.replace("List.", "List them all."),
            _ => format!("{text}# changed\n"),
        };
        std::fs::write(&path, &edited).unwrap();
        assert!(!program(&fx, &extension(&project)).approved, "{file}");
        std::fs::write(&path, text).unwrap();
    }
}

/// tsk824: signing in sends a code, a PKCE verifier and a client secret to
/// the endpoints the manifest names, so they must be the approved ones. An
/// edited endpoint makes the provider unapproved, and its sign-in is
/// refused until a person approves it again — nothing listens, nothing is
/// sent.
#[tokio::test]
async fn sign_in_is_refused_until_the_endpoints_are_approved() {
    let (fx, sim) = signing_in("", "").await;
    let project = fx.svc.layout.project_dir.clone();
    configure(&fx, true, json!({ "team": "core" }));
    // Someone points the token endpoint elsewhere.
    write_oauth_extension(
        &project,
        "",
        &sim.authorize_url,
        "https://elsewhere.example/token",
        "",
    );
    let refused = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("approv"), "{refused}");
    assert!(
        sim.grants().is_empty(),
        "nothing reached any token endpoint"
    );
    // The row says so: no Sign in on an unapproved instance.
    let view = fx
        .svc
        .providers
        .list()
        .await
        .into_iter()
        .find(|v| v.instance == INSTANCE)
        .unwrap();
    assert!(!view.approved);
    // Approved as it is now, it signs in.
    write_oauth_extension(&project, "", &sim.authorize_url, &sim.token_url, "");
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
}

/// tsk826: two sign-ins for one credential started at once leave one
/// under way — the later one — and the earlier one's redirect does
/// nothing.
#[tokio::test(flavor = "multi_thread")]
async fn two_sign_ins_at_once_leave_one_under_way() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let svc = fx.svc.clone();
    let begin = || {
        let svc = svc.clone();
        tokio::spawn(async move {
            svc.providers
                .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
                .await
        })
    };
    let (a, b) = tokio::join!(begin(), begin());
    let (a, b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
    let mut signed_in = 0;
    for page in [&a, &b] {
        if let Ok(SignInCompletion::SignedIn) =
            complete(&fx, "FAKE_TOKEN", &browse(page).await).await
        {
            signed_in += 1;
        }
    }
    assert_eq!(signed_in, 1, "exactly one sign-in was under way");
    assert_eq!(sim.grants(), vec!["authorization_code"]);
}

/// tsk826: removing a project's replacement of a global instance ends a
/// sign-in under way for it too.
#[tokio::test]
async fn removing_a_project_replacement_ends_its_sign_in() {
    let (fx, _sim) = signing_in("", "").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    configure_named(&fx, SHARED, Some("fake"), json!({ "team": "mine" }));
    let page = providers
        .begin_sign_in(SHARED, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    providers
        .remove_instance(&Actor::Human, SHARED)
        .await
        .unwrap();
    let refused = providers
        .complete_sign_in(SHARED, "FAKE_TOKEN", &browse(&page).await)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("under way"), "{refused}");
}

/// tsk828: several calls refused at once renew the sign-in once, and each
/// is tried again on the one renewed token.
#[tokio::test(flavor = "multi_thread")]
async fn calls_refused_together_renew_once() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    // Its service now takes only the next token; the process holds at-2.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
    let calls: Vec<_> = (0..3)
        .map(|_| {
            let svc = fx.svc.clone();
            tokio::spawn(async move {
                svc.commands
                    .run(
                        &Actor::Human,
                        "work_item.create",
                        json!({ "provider": "fake", "title": "x" }),
                        false,
                    )
                    .await
            })
        })
        .collect();
    for call in calls {
        call.await.unwrap().unwrap();
    }
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
}

/// P10: a call cut off because a renewal ended its process may have
/// landed: it is sent again only to a provider that keeps
/// `idempotent_writes` (under the same key); toward any other the cut-off
/// is the call's failure. A call its service refused (`Auth`) never
/// landed, and is sent again either way.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_cut_off_by_a_renewal_is_not_resent_without_a_key() {
    for (hooks, resent) in [("", true), ("plain-writes", false)] {
        let (fx, _sim) = signing_in(hooks, "").await;
        configure(&fx, true, json!({ "team": "core" }));
        assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
        first_read(&fx).await;
        // One call under way (it waits before it writes) ...
        set_hooks(&fx, "slow:500").await;
        let svc = fx.svc.clone();
        let under_way = tokio::spawn(async move {
            svc.commands
                .run(
                    &Actor::Human,
                    "work_item.create",
                    json!({ "provider": "fake", "title": "cut off" }),
                    false,
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        // ... when another is refused, and its renewal ends the process.
        set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
        create_on_fake(&fx).await.unwrap();
        let cut_off = under_way.await.unwrap();
        assert_eq!(cut_off.is_ok(), resent, "{hooks:?}: {cut_off:?}");
    }
}

/// The tracker extension with the fake (running with `hooks`) and an
/// effect `file` that reacts to an oxplow task moved to done with the
/// calls `script` composes; both approved, the provider enabled.
async fn with_effect(hooks: &str, script: &str) -> EffortFixture {
    with_effect_reading(hooks, None, script).await
}

/// [`with_effect`], its effect reading `input` rows first when given.
async fn with_effect_reading(hooks: &str, input: Option<&str>, script: &str) -> EffortFixture {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, hooks);
    let dir = project.join("oxplow/extensions").join(EXT);
    let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        manifest
            + "effects:\n  - id: file\n    summary: File an item on the tracker.\n    on: [work_item.transitioned]\n    where: { to: done }\n    entry: file.star\n"
            + &input
                .map(|sql| format!("    input: \"{sql}\"\n"))
                .unwrap_or_default(),
    )
    .unwrap();
    std::fs::write(dir.join("file.star"), script).unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    let decl = ext.effects[0].clone();
    let program = crate::effects::effect_program(&ext, &decl);
    let config = fx.svc.config.read().unwrap().clone();
    exec_consent::approve_program(
        &fx.svc.approvals,
        &project,
        &config,
        std::slice::from_ref(&ext),
        ProgramKind::Effect,
        &program.name,
        &program.hash(&project).unwrap(),
    )
    .unwrap();
    crate::effects::approved(&fx.svc.db, &decl.name())
        .await
        .unwrap();
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    first_read(&fx).await;
    fx
}

/// The effect's script: one write to the fake.
const FILE_ON_FAKE: &str = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.create\", \"input\": {\"provider\": \"fake\", \"title\": \"from effect\"}}]}\n";

/// Move the fixture's task to done (what the effect reacts to) and let
/// the effect react.
async fn react(fx: &EffortFixture) -> oxplow_domain::StoredEvent {
    use oxplow_domain::events::schema::{WorkItemTransitioned, WorkItemTransitionedV1};
    let env = oxplow_domain::Envelope::typed::<WorkItemTransitioned>(
        "human",
        &WorkItemTransitionedV1 {
            work_item: oxplow_domain::refs::build::work_item_ref(fx.task),
            from: oxplow_domain::TaskStatus::InProgress,
            to: oxplow_domain::TaskStatus::Done,
            effort: None,
        },
    );
    let id = env.id.clone();
    fx.svc.event_log_store.append(env).await.unwrap();
    let ev = fx.svc.event_log_store.get(id).await.unwrap().unwrap();
    use crate::event_pump::AsyncEventConsumer as _;
    crate::effect_triggers::EffectTriggers::new(std::sync::Arc::downgrade(&fx.svc))
        .handle(&ev)
        .await
        .unwrap();
    ev
}

async fn effect_runs(fx: &EffortFixture) -> serde_json::Value {
    let rows = fx
        .svc
        .sql
        .query_sql(
            "SELECT attempt, origin, state, retry_at IS NOT NULL FROM v_effect_run ORDER BY attempt",
            vec![],
            None,
        )
        .await
        .unwrap()
        .rows;
    serde_json::to_value(rows).unwrap()
}

/// The effect's consecutive failures, as its health counts them.
async fn effect_failures(fx: &EffortFixture) -> i64 {
    let health =
        crate::plugin_health::PluginHealth::new(fx.svc.db.clone(), fx.svc.vocabulary.clone());
    let key = oxplow_db::PluginKey {
        plugin: EXT.into(),
        contribution: "file".into(),
        kind: "effect",
    };
    health
        .get(&key)
        .await
        .unwrap()
        .map_or(0, |h| h.consecutive_failures)
}

/// `secs` from now.
fn in_secs(secs: i64) -> oxplow_domain::Timestamp {
    oxplow_domain::Timestamp::from_unix_ms(oxplow_domain::Timestamp::now().unix_ms() + secs * 1000)
}

/// P10: a step whose reply was lost (it landed; the call timed out) is
/// sent again by itself — the reaction's every step goes to a provider
/// that keeps `idempotent_writes` — under the same key, so it lands once.
/// The attempt awaiting its retry isn't counted against the effect.
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_reply_is_sent_again_and_lands_once() {
    let fx = with_effect("lose-reply", FILE_ON_FAKE).await;
    react(&fx).await;
    assert_eq!(effect_runs(&fx).await, json!([[1, "live", "failed", 1]]));
    assert_eq!(effect_failures(&fx).await, 0, "awaiting its retry");
    // Not due yet; then due.
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(11))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        effect_runs(&fx).await,
        json!([[1, "live", "failed", 1], [2, "auto", "ok", 0]])
    );
    let probe = ServicesProbe(&fx.svc);
    assert_eq!(probe.sync("fake").await, Ok(true));
    probe.settle().await;
    assert_eq!(probe.titled("fake", "from effect").await.len(), 1);
    assert_eq!(effect_failures(&fx).await, 0);
}

/// tsk887: an automatic retry sends exactly what the failed attempt
/// composed — not a fresh composition — so a write that landed under the
/// first attempt's key isn't made again when what the effect reads
/// changed in between.
#[tokio::test(flavor = "multi_thread")]
async fn an_automatic_retry_sends_what_the_failed_attempt_composed() {
    let titled_from_the_task = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.create\", \"input\": {\"provider\": \"fake\", \"title\": \"after \" + x[\"rows\"][0][\"title\"]}}]}\n";
    let fx = with_effect_reading(
        "lose-reply",
        Some("SELECT title FROM v_work_item WHERE ref = :work_item"),
        titled_from_the_task,
    )
    .await;
    let task = oxplow_domain::refs::build::work_item_ref(fx.task);
    let rows = fx
        .svc
        .sql
        .query_sql(
            &format!("SELECT title FROM v_work_item WHERE ref = '{task}'"),
            vec![],
            None,
        )
        .await
        .unwrap()
        .rows;
    let title = serde_json::to_value(rows).unwrap()[0][0]
        .as_str()
        .unwrap()
        .to_string();
    react(&fx).await;
    assert_eq!(effect_runs(&fx).await, json!([[1, "live", "failed", 1]]));
    // What the effect reads changes before its retry.
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "work_item.update",
            json!({ "ref": task, "title": "renamed" }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(11))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        effect_runs(&fx).await,
        json!([[1, "live", "failed", 1], [2, "auto", "ok", 0]])
    );
    let probe = ServicesProbe(&fx.svc);
    assert_eq!(probe.sync("fake").await, Ok(true));
    probe.settle().await;
    assert_eq!(
        probe.titled("fake", &format!("after {title}")).await.len(),
        1
    );
    assert_eq!(probe.titled("fake", "after renamed").await.len(), 0);
}

/// tsk887: a scheduled retry is sent only while every step it would send
/// still goes to a provider keeping `idempotent_writes`: with the
/// provider gone, it isn't sent, and the failure counts — a person's.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_whose_provider_went_away_is_not_sent() {
    let fx = with_effect("lose-reply", FILE_ON_FAKE).await;
    react(&fx).await;
    assert_eq!(effect_runs(&fx).await, json!([[1, "live", "failed", 1]]));
    assert!(fx.svc.providers.stop(INSTANCE).await);
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(11))
            .await
            .unwrap(),
        0
    );
    assert_eq!(effect_runs(&fx).await, json!([[1, "live", "failed", 0]]));
    assert_eq!(effect_failures(&fx).await, 1);
}

/// P10: only a reaction whose every step is a write to a provider
/// keeping `idempotent_writes` is sent again by itself: toward a provider
/// that doesn't declare it, or with a step inside oxplow beside it, the
/// failure waits for a person's retry, and counts.
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_attempt_retries_only_toward_a_declaring_provider() {
    let with_a_task_step = "def transform(x):\n    ref = x[\"event\"][\"payload\"][\"work_item\"]\n    return {\"commands\": [{\"name\": \"work_item.update\", \"input\": {\"ref\": ref, \"title\": \"filed\"}}, {\"name\": \"work_item.create\", \"input\": {\"provider\": \"fake\", \"title\": \"from effect\"}}]}\n";
    for (hooks, script) in [
        ("lose-reply,plain-writes", FILE_ON_FAKE),
        ("lose-reply", with_a_task_step),
    ] {
        let fx = with_effect(hooks, script).await;
        react(&fx).await;
        assert_eq!(
            effect_runs(&fx).await,
            json!([[1, "live", "failed", 0]]),
            "{hooks}"
        );
        assert_eq!(effect_failures(&fx).await, 1, "{hooks}");
        assert_eq!(
            crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(3600))
                .await
                .unwrap(),
            0,
            "{hooks}"
        );
    }
}

/// P10: at most two automatic attempts, 10 s then 60 s after the failure
/// before; the attempt that exhausts them counts as the one failure.
#[tokio::test(flavor = "multi_thread")]
async fn auto_retry_stops_after_two_and_counts_one_failure() {
    let fx = with_effect("", FILE_ON_FAKE).await;
    set_hooks(&fx, "fail-next:3").await;
    react(&fx).await;
    assert_eq!(effect_runs(&fx).await, json!([[1, "live", "failed", 1]]));
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(11))
            .await
            .unwrap(),
        1
    );
    // The second waits 60 s from the first retry's failure.
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(30))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(75))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        effect_runs(&fx).await,
        json!([
            [1, "live", "failed", 1],
            [2, "auto", "failed", 1],
            [3, "auto", "failed", 0]
        ])
    );
    assert_eq!(effect_failures(&fx).await, 1);
    assert_eq!(
        crate::effect_triggers::auto_retry_due(&fx.svc, in_secs(3600))
            .await
            .unwrap(),
        0
    );
}
