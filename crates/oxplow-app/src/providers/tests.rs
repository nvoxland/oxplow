//! P5.D3: the host over the real fake provider process — consent before
//! execution, the handshake against the approved declarations, and the
//! snapshots conformance suite through its host adapter.

use std::path::{Path, PathBuf};

use oxplow_domain::{Actor, DomainError, ThreadId};
use oxplow_oauth_sim::OAuthSim;
use serde_json::json;

use super::*;
use crate::extension::extensions::Extension;
#[cfg(unix)]
use crate::test_fixtures::lifecycle::write_extension;
use crate::test_fixtures::lifecycle::{fake_bin, EXT};
use crate::test_fixtures::{services_with_thread, ThreadFixture};

/// The manifest `text` with `secrets` declared beside its `process` permission.
fn permit_secrets(text: &str, secrets: &[&str]) -> String {
    text.replacen(
        "permissions: [process.run]",
        &format!(
            "permissions: [process.run, {}]",
            secrets
                .iter()
                .map(|s| format!("secrets:{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        1,
    )
}

/// Whether instance `id` runs: its health is ready.
fn running(svc: &crate::Services, id: &str) -> bool {
    svc.providers
        .health(&format!("tracker/{id}"))
        .is_some_and(|h| h.state == InstanceState::Ready)
}

fn extension(project: &Path) -> Extension {
    let ext = crate::extension::extensions::load_extensions(project)
        .into_iter()
        .find(|e| e.name == EXT)
        .expect("the tracker extension loads");
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    ext
}

/// Deploys the extension, which publishes a generation that can start
/// its provider.
async fn deploy(fx: &ThreadFixture, _ext: &Extension) {
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
}

/// The fixture with the tracker extension (running the fake with
/// `hooks`) deployed on this machine.
#[cfg(unix)]
async fn deployed(hooks: &str) -> (ThreadFixture, Extension) {
    let fx = services_with_thread().await;
    write_extension(&fx.svc.layout.project_dir, hooks);
    let ext = extension(&fx.svc.layout.project_dir);
    deploy(&fx, &ext).await;
    crate::test_fixtures::activate(&fx.svc).await;
    (fx, ext)
}

const INSTANCE: &str = "tracker/fake";

fn insert_instance(
    svc: &crate::Services,
    name: String,
    config: oxplow_config::ExtensionInstanceConfig,
) {
    use oxplow_config::settings::Level;
    let store = crate::settings::store(
        &svc.layout.project_dir,
        svc.providers.deps.global_dir.as_deref().unwrap(),
        svc.secrets.clone(),
        &[],
    )
    .unwrap();
    let mut all: std::collections::BTreeMap<String, oxplow_config::ExtensionInstanceConfig> = store
        .snapshot()
        .and_then(|snapshot| snapshot.get("oxplow.extensionInstances", Some(Level::Project)))
        .unwrap()
        .value
        .map(serde_json::from_value)
        .transpose()
        .unwrap()
        .unwrap_or_default();
    all.insert(name, config);
    store
        .set("oxplow.extensionInstances", json!(all), Level::Project)
        .unwrap();
    svc.reload_config_from_disk().unwrap();
}

fn persist_instances(svc: &crate::Services) {
    let all = svc.config.read().unwrap().extension_instances.clone();
    let store = crate::settings::store(
        &svc.layout.project_dir,
        svc.providers.deps.global_dir.as_deref().unwrap(),
        svc.secrets.clone(),
        &[],
    )
    .unwrap();
    store
        .set(
            "oxplow.extensionInstances",
            json!(all),
            oxplow_config::settings::Level::Project,
        )
        .unwrap();
}

/// Enable the instance through the project's stored settings.
fn configure(fx: &ThreadFixture, enabled: bool, config: serde_json::Value) {
    insert_instance(
        &fx.svc,
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled,
            config,
            sync_minutes: None,
            provider: None,
        },
    );
}

/// Enable `spec`'s default instance with `config`, as the person's
/// configuration does: write its entry, then publish the generation that
/// starts it. A refused or failing start shows in its health, not here.
async fn enable_on(
    svc: &std::sync::Arc<crate::Services>,
    ext: &Extension,
    spec: &spec::ProviderSpec,
    config: serde_json::Value,
) {
    // Deploying the sources turns them on in the person's file and
    // reloads the config, so the in-memory entry is written after.
    crate::test_fixtures::activate_with_problems(svc).await;
    insert_instance(
        svc,
        spec::instance_name(&ext.namespace, &spec.id),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config,
            sync_minutes: None,
            provider: None,
        },
    );
    crate::test_fixtures::activate_with_problems(svc).await;
}

/// Turn `instance` off in the project's configuration and publish the
/// generation without it: it stops, and what it registered goes.
async fn turn_off(svc: &std::sync::Arc<crate::Services>, instance: &str) {
    {
        let mut config = svc.config.write().unwrap();
        config
            .extension_instances
            .get_mut(instance)
            .expect("the instance is configured")
            .enabled = false;
    }
    persist_instances(svc);
    crate::test_fixtures::activate_with_problems(svc).await;
}

async fn logged(fx: &ThreadFixture, event_type: &str) -> Vec<serde_json::Value> {
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

#[cfg(unix)]
#[tokio::test]
async fn a_provider_must_answer_with_its_approved_declarations() {
    let (fx, ext) = deployed("bad-declarations").await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    assert!(!running(&fx.svc, "fake"));
    // Not what was approved: off, and logged, until a person looks.
    let state = fx.svc.providers.health(INSTANCE).unwrap().state;
    assert!(
        matches!(&state, InstanceState::Disabled { reason } if reason.contains("/commands")),
        "{state:?}"
    );
    assert_eq!(logged(&fx, "contribution.disabled").await.len(), 1);
}

/// P5.D4's red: an unconfigured instance can't be enabled — the check
/// refuses it naming the config field, and nothing is written.
#[cfg(unix)]
#[tokio::test]
async fn an_unconfigured_instance_cannot_be_enabled() {
    let (fx, _ext) = deployed("").await;
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
    assert!(!running(&fx.svc, "fake"));

    // Configured, it enables: written to the project's config and running.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
        .await
        .unwrap();
    assert!(view.enabled);
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Ready
    );
    assert!(running(&fx.svc, "fake"));
    let written =
        std::fs::read_to_string(oxplow_config::config_path(&fx.svc.layout.project_dir)).unwrap();
    assert!(written.contains("extensionInstances"), "{written}");
    assert_eq!(logged(&fx, "contribution.enabled").await.len(), 1);

    // Disabling stops it.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, false, json!({ "team": "core" }))
        .await
        .unwrap();
    assert!(!view.enabled);
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Off
    );
    assert!(!running(&fx.svc, "fake"));
}

/// P5.D4's red: three failures in a row disable the instance, logged
/// with the reason; it stays off across reconciles until
/// `oxplow.contribution.enable` runs, by a person or an agent.
#[cfg(unix)]
#[tokio::test]
async fn three_failures_in_a_row_disable_an_instance_until_it_is_enabled() {
    let (fx, _ext) = deployed("fail-next:3").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));

    // Every start's check fails (each process begins with fail-next:3):
    // the reconcile's start is the first failure, two calls the rest.
    crate::test_fixtures::activate(&fx.svc).await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Failing { .. }
    ));
    for _ in 0..2 {
        assert!(mark_on_fake(&fx).await.is_err());
    }
    let health = providers.health(INSTANCE).unwrap();
    let InstanceState::Disabled { reason } = &health.state else {
        panic!("{health:?}");
    };
    assert!(reason.contains("3 failures in a row"), "{reason}");
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(!running(&fx.svc, "fake"));
    let disabled = logged(&fx, "contribution.disabled").await;
    assert_eq!(disabled.len(), 1);
    assert_eq!(
        disabled[0]["contribution"],
        oxplow_domain::uri::build::keyed("oxplow/provider", "tracker/fake")
    );
    assert_eq!(disabled[0]["reason"], reason.as_str());

    // The log keeps it off.
    crate::test_fixtures::activate(&fx.svc).await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));

    // An agent may enable it again.
    write_extension(&fx.svc.layout.project_dir, "");
    deploy(&fx, &extension(&fx.svc.layout.project_dir)).await;
    let enabled = fx
        .svc
        .commands
        .run(
            &Actor::Agent {
                session_id: None,
                thread_id: Some(ThreadId::new(fx.thread.value())),
                stream_id: None,
            },
            "oxplow.contribution.enable",
            json!({ "namespace": "tracker", "kind": "provider", "contribution": "fake" }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(enabled.result["state"], "ok", "{}", enabled.result);
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Ready
    );
    assert!(running(&fx.svc, "fake"));
}

/// P6b.C2: a running instance's capability and features are a model row
/// (as the host reads them); stopped, it's no longer published.
#[cfg(unix)]
#[tokio::test]
async fn a_running_instance_publishes_its_features() {
    let (fx, ext) = deployed("").await;
    let store = oxplow_db::SqliteCapabilityStore::new(fx.svc.db.clone());
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    // Its start's first read, in the background.
    first_read(&fx).await;
    let fake = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.provider == "fake")
        .expect("published");
    assert_eq!(fake.capability, "snapshots");
    assert_eq!(fake.extension.as_deref(), Some("tracker"));
    turn_off(&fx.svc, INSTANCE).await;
    // A stopped instance is no longer published.
    assert!(store
        .list()
        .await
        .unwrap()
        .into_iter()
        .all(|r| r.provider != "fake"));
}

/// Protocol 3's `host/call`: a provider whose manifest `needs` `sql.read`
/// reads oxplow's models while it serves a command. One that doesn't need
/// it is refused, and so is its command.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_reads_the_host_through_what_it_needs() {
    for (needs, reads) in [(true, true), (false, false)] {
        let fx = services_with_thread().await;
        let root = fx.svc.layout.project_dir.clone();
        write_extension(&root, "host-read");
        if needs {
            let manifest = root.join(".oxplow/extensions/tracker/extension.yaml");
            let text = std::fs::read_to_string(&manifest).unwrap();
            let text = text.replacen(
                "permissions: [process.run]",
                "permissions: [sql.read, process.run]",
                1,
            );
            std::fs::write(&manifest, format!("{text}    needs: [sql.read]\n")).unwrap();
        }
        let ext = extension(&root);
        assert_eq!(ext.providers[0].needs.is_empty(), !needs);
        deploy(&fx, &ext).await;
        enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
        crate::test_fixtures::activate(&fx.svc).await;
        let worktree = tempfile::tempdir().unwrap();
        let ran = invoke_on(
            &fx,
            INSTANCE,
            "mark",
            json!({ "stream": "s1", "worktree": worktree.path(), "trigger": "manual" }),
        )
        .await;
        if reads {
            assert_eq!(ran.unwrap().result["read"], json!([{ "n": 7 }]));
        } else {
            let err = ran.unwrap_err().to_string();
            assert!(err.contains("isn't in the command's `needs`"), "{err}");
        }
    }
}

/// A provider reads its own extension's configuration over `host/call`
/// (`extension_config.read`): the effective values and origins, with its
/// project layer applied. One that doesn't list the scope in `needs` is
/// refused at load, so it can't ask.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_reads_its_extensions_configuration_through_host_call() {
    let fx = services_with_thread().await;
    let root = fx.svc.layout.project_dir.clone();
    write_extension(&root, "config-read");
    let manifest = root.join(".oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let text = text.replace(
        "intent:\n",
        "configuration:\n  version: 1\n  schema:\n    type: object\n    additionalProperties: false\n    properties:\n      limit: { type: integer, default: 4 }\nintent:\n",
    );
    std::fs::write(
        &manifest,
        format!("{text}    needs: [extension_config.read]\n"),
    )
    .unwrap();
    let ext = extension(&root);
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    deploy(&fx, &ext).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    crate::test_fixtures::activate(&fx.svc).await;
    let global = fx.svc.layout.state_dir.join("global-config");
    let store = crate::settings::store(
        &root,
        &global,
        fx.svc.secrets.clone(),
        std::slice::from_ref(&ext),
    )
    .unwrap();
    store
        .set(
            "tracker.limit",
            json!(9),
            oxplow_config::settings::Level::Project,
        )
        .unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let out = invoke_on(
        &fx,
        INSTANCE,
        "mark",
        json!({ "stream": "s1", "worktree": worktree.path(), "trigger": "manual" }),
    )
    .await
    .unwrap();
    assert_eq!(out.result["read"]["values"], json!({ "limit": 9 }));
    assert_eq!(out.result["read"]["origins"]["/limit"], json!("project"));
}
/// A provider's command is declared in its extension's manifest over an
/// operation its declarations list (`provider:` + `op:`): its input is the
/// operation's, with an optional `instance`; a capability verb, an
/// operation it doesn't declare or a provider it doesn't have is refused
/// at its line.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_command_is_declared_over_an_operation_it_lists_and_no_verb() {
    let fx = services_with_thread().await;
    let root = fx.svc.layout.project_dir.clone();
    write_extension(&root, "");
    let ext = extension(&root);
    assert!(ext.commands.is_empty());
    let manifest = root.join(".oxplow/extensions/tracker/extension.yaml");
    let base = std::fs::read_to_string(&manifest).unwrap();
    for (entry, says) in [
        (
            "  - { name: item.nope, summary: S., provider: fake, op: nope }\n",
            "its declarations have no command `nope`",
        ),
        (
            "  - { name: item.other, summary: S., provider: other, op: nope }\n",
            "no provider `other`",
        ),
        (
            "  - { name: item.mark, summary: S., provider: fake, op: mark }\n",
            "`mark` is a snapshots verb: core calls it",
        ),
    ] {
        std::fs::write(
            &manifest,
            base.replacen(
                "permissions:",
                &format!("commands:\n{entry}permissions:"),
                1,
            ),
        )
        .unwrap();
        let loaded = crate::extension::extensions::load_extensions(&root)
            .into_iter()
            .find(|e| e.name == EXT)
            .unwrap();
        let errs = loaded
            .errors
            .iter()
            .map(|d| loaded.shown(d))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(errs.contains(says), "{entry}: {errs}");
    }
}

/// A coverage run's profile path reaches the provider, so an instrumented
/// provider doesn't write its `.profraw` into its deployment.
#[test]
fn a_coverage_runs_profile_path_reaches_the_provider() {
    assert!(super::host::PASSED_ENV.contains(&"LLVM_PROFILE_FILE"));
}

/// A provider's args are read where it runs (its extension folder), and
/// an arg reaching outside the folder is refused.
#[cfg(unix)]
#[tokio::test]
async fn provider_args_stay_inside_the_extension_folder() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let manifest = project.join(".oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    args: [../../../tools/tracker.py]\n",
        ),
    )
    .unwrap();
    let loaded = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty());
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| loaded.shown(e).contains("../../../tools/tracker.py")),
        "{:?}",
        loaded.errors
    );
}

const PROGRAM_MANIFEST: &str = "manifest: 2\nname: tracker\nintent:\n  purpose: a program\n  examples: [{ name: a }]\npermissions: [process.run]\nproviders:\n  - id: notes\n    capability: snapshots\n    entry: bin/server\n    args: [--stdio]\n    declarations: provider.json\n";

/// A private extension whose provider is a program of its folder, started
/// with an argument.
fn write_program_extension(project: &Path) -> PathBuf {
    let dir = project.join(".oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("extension.yaml"), PROGRAM_MANIFEST).unwrap();
    std::fs::write(dir.join("bin/server"), "#!/bin/sh\n").unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
    )
    .unwrap();
    dir
}

/// `manifest` loaded as the tracker extension, whose providers are empty
/// and whose errors name `says`.
fn assert_refused(project: &Path, dir: &Path, manifest: &str, says: &str) {
    std::fs::write(dir.join("extension.yaml"), manifest).unwrap();
    let loaded = crate::extension::extensions::load_extensions(project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty(), "{says}");
    assert!(
        loaded.errors.iter().any(|e| loaded.shown(e).contains(says)),
        "{says}: {:?}",
        loaded.errors
    );
}

/// A provider names a program (an `entry`) and arguments, each inside
/// the extension folder.
#[tokio::test]
async fn a_provider_names_its_program_and_args_inside_the_folder() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_program_extension(&project);
    let ext = extension(&project);
    assert_eq!(
        ext.providers[0].program(),
        ("bin/server".into(), vec!["--stdio".into()])
    );
    for (from, to, says) in [
        ("    entry: bin/server\n", "", "name an `entry`"),
        (
            "entry: bin/server",
            "entry: /usr/local/bin/npx",
            "/usr/local/bin/npx",
        ),
        ("entry: bin/server", "entry: ../x", "../x"),
        ("args: [--stdio]", "args: [../../x.js]", "../../x.js"),
    ] {
        assert!(PROGRAM_MANIFEST.contains(from), "{from}");
        assert_refused(&project, &dir, &PROGRAM_MANIFEST.replace(from, to), says);
    }
}

/// A provider's credentials are declared once each, sign in only over
/// https (or loopback), and a client secret names a pasted credential of
/// the provider.
#[tokio::test]
async fn a_providers_credentials_are_declared_and_checked() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    let dir = write_program_extension(&project);
    let signed_in = |authorize: &str, secret: &str| {
        PROGRAM_MANIFEST.replace(
            "process.run]",
            "process.run, secrets:NOTES_TOKEN, secrets:SIGNED_IN]",
        ) + &format!(
            "    credentials:\n      - NOTES_TOKEN\n      - name: SIGNED_IN\n        oauth: \
                 {{ authorize_url: \"{authorize}\", token_url: \"https://svc.example.com/token\", \
                 client_id: oxplow, client_secret: {secret} }}\n"
        )
    };
    for (manifest, says) in [
        (
            PROGRAM_MANIFEST.replace("process.run]", "process.run, secrets:NOTES_TOKEN]")
                + "    credentials: [NOTES_TOKEN, NOTES_TOKEN]\n",
            "declared twice",
        ),
        (
            signed_in("http://svc.example.com/authorize", "NOTES_TOKEN"),
            "must be https",
        ),
        (
            signed_in("https://svc.example.com/authorize", "NOPE"),
            "must name another credential",
        ),
        (
            signed_in("https://svc.example.com/authorize", "SIGNED_IN"),
            "must name another credential",
        ),
    ] {
        assert_refused(&project, &dir, &manifest, says);
    }
    // On loopback, plain http is what a local service speaks.
    std::fs::write(
        dir.join("extension.yaml"),
        signed_in("http://127.0.0.1:8123/authorize", "NOTES_TOKEN"),
    )
    .unwrap();
    extension(&project);
}

/// A provider runs from its extension's deployment, not from the live
/// tree.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_runs_from_its_deployment() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let script = project.join(".oxplow/extensions/tracker/bin/provider");
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
    deploy(&fx, &ext).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    // Its start's first read, in the background.
    first_read(&fx).await;
    // It ran in its extension's deployment, not in the live tree.
    let ran_in = std::path::PathBuf::from(std::fs::read_to_string(&cwd_file).unwrap().trim())
        .canonicalize()
        .unwrap();
    let hash = fx.svc.extension_catalog.live()["tracker"].clone();
    let deployment = crate::extension::extension_store::deployment_dir(&project, EXT, &hash)
        .canonicalize()
        .unwrap();
    assert_eq!(ran_in, deployment);
    assert_ne!(
        ran_in,
        project
            .join(".oxplow/extensions/tracker")
            .canonicalize()
            .unwrap()
    );
}

/// a provider may emit only its capability's event types — a
/// declared `contribution.enabled` (which would clear another contribution's
/// automatic disable) is refused when the manifest loads.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_cannot_declare_another_core_event_type() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let mut declared = oxplow_provider_fake::declarations();
    declared
        .event_types
        .push(oxplow_provider_protocol::model::EventTypeDecl {
            event_type: "contribution.enabled".into(),
            v: 1,
            schema: oxplow_domain::events::schema::schema_for::<
                oxplow_domain::events::schema::ContributionEnabled,
            >(),
        });
    std::fs::write(
        project.join(".oxplow/extensions/tracker/provider.json"),
        serde_json::to_string(&declared).unwrap(),
    )
    .unwrap();
    let loaded = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty());
    assert!(
        loaded.errors.iter().any(|e| {
            let e = loaded.shown(e);
            e.contains("contribution.enabled@1") && e.contains("snapshots")
        }),
        "{:?}",
        loaded.errors
    );
}

/// a provider whose `check` hangs times out — counted as a
/// failure — and never blocks stopping it.
#[cfg(unix)]
#[tokio::test]
async fn a_hung_check_times_out_and_the_instance_can_still_stop() {
    let (fx, ext) = deployed("slow-check:30000").await;
    let providers = &fx.svc.providers;
    // A start that merely failed stays enabled, and the generation activates.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })),
    )
    .await
    .expect("activation returns once the check times out");
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

/// a hung `invoke` times out (with `$/cancel`) and counts as a
/// failure, instead of blocking the caller forever with health `ready`.
#[cfg(unix)]
#[tokio::test]
async fn a_hung_invoke_times_out_and_counts() {
    let (fx, ext) = deployed("").await;
    let providers = &fx.svc.providers;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    // Its start's first read, in the background.
    first_read(&fx).await;
    set_hooks(&fx, "slow:30000").await;
    let ran = tokio::time::timeout(std::time::Duration::from_secs(10), mark_on_fake(&fx))
        .await
        .expect("the call returns once it times out");
    let err = ran.unwrap_err().to_string();
    assert!(err.contains("timed out"), "{err}");
    assert_eq!(providers.health(INSTANCE).unwrap().consecutive_failures, 1);
}

/// a disable while an instance is starting wins — the start
/// doesn't bring back what was just disabled.
#[cfg(unix)]
#[tokio::test]
async fn a_disable_while_starting_keeps_the_instance_off() {
    let (fx, ext) = deployed("slow-check:400").await;
    let providers = fx.svc.providers.clone();
    let spec = ext.providers[0].clone();
    let starting = {
        let (svc, ext) = (fx.svc.clone(), ext.clone());
        tokio::spawn(async move { enable_on(&svc, &ext, &spec, json!({ "team": "core" })).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    providers
        .disable(INSTANCE, None, "a person turned it off".into())
        .await;
    starting.await.unwrap();
    assert!(!running(&fx.svc, "fake"));
    assert!(providers.get(INSTANCE).await.is_none());
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
}

/// when whether an instance was disabled can't be read, a
/// reconcile doesn't start it (it fails closed).
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_disable_record_keeps_the_instance_off() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    fx.svc
        .db
        .transaction(|c| {
            c.execute_batch(
                "ALTER TABLE contribution_health RENAME TO contribution_health_unreadable",
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    crate::test_fixtures::activate(&fx.svc).await;
    assert!(!running(&fx.svc, "fake"));
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert!(
        matches!(&health.state, InstanceState::Failing { errors } if errors[0].contains("disabled")),
        "{health:?}"
    );
}

/// Deploying an updated provider restarts a running instance on what was
/// deployed.
#[cfg(unix)]
#[tokio::test]
async fn deploying_updated_declarations_restarts_the_instance() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    assert!(running(&fx.svc, "fake"));

    // The provider updates: new behaviour and new declarations.
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "bad-declarations");
    std::fs::write(
        project.join(".oxplow/extensions/tracker/provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::bad_declarations(
            oxplow_provider_fake::declarations(),
        ))
        .unwrap(),
    )
    .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let running = fx.svc.providers.get(INSTANCE).await.expect("running");
    assert_eq!(
        running.declared,
        oxplow_provider_fake::bad_declarations(oxplow_provider_fake::declarations())
    );
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Ready
    );
    assert!(logged(&fx, "contribution.disabled").await.is_empty());
    // The restarted instance republished its capability row, with the
    // features it declares now.
    let row = oxplow_db::SqliteCapabilityStore::new(fx.svc.db.clone())
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.provider == "fake")
        .expect("republished");
    assert_eq!(
        row.features,
        serde_json::to_value(&running.declared.capabilities[0].features).unwrap()
    );
}

/// Declarations that don't parse fail the extension's load: the edit is
/// held back naming the file, and the provider that was working goes on
/// being described by its last working declarations.
#[cfg(unix)]
#[tokio::test]
async fn unreadable_declarations_are_held_back_naming_the_file() {
    let (fx, _ext) = deployed("").await;
    std::fs::write(
        fx.svc
            .layout
            .project_dir
            .join(".oxplow/extensions/tracker/provider.json"),
        "{ not json",
    )
    .unwrap();
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(
        crate::test_fixtures::shown_held(&fx.svc, &problems, EXT).contains("provider.json"),
        "{problems:?}"
    );
}

/// enabling from Settings writes nothing when the enable itself
/// fails — the config never says enabled for an instance that wasn't.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_enable_writes_no_config() {
    let (fx, _ext) = deployed("").await;
    fx.svc
        .db
        .transaction(|c| {
            c.execute_batch(
                "CREATE TRIGGER refuse_enabled BEFORE INSERT ON event_log
                 WHEN NEW.type = 'contribution.enabled'
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
    assert!(!running(&fx.svc, "fake"));
}

/// The fake's spec, as an implementation of `capability`.
#[cfg(unix)]
fn fake_spec(capability: &str) -> spec::ProviderSpec {
    let dir = tempfile::tempdir().unwrap();
    write_extension(dir.path(), "");
    let mut spec = extension(dir.path()).providers[0].clone();
    spec.capability = capability.into();
    spec
}

/// A harness's declarations: the fake harness's, its first verb copied
/// under each of `verbs`, with `features` and `data`.
fn harness_declarations(
    verbs: &[&str],
    features: serde_json::Value,
    data: serde_json::Value,
) -> oxplow_provider_protocol::model::InitializeResult {
    let mut declared = oxplow_provider_fake::harness_declarations();
    let verb = declared.commands[0].clone();
    declared.commands = verbs
        .iter()
        .map(|v| oxplow_provider_protocol::model::CommandDecl {
            name: (*v).into(),
            ..verb.clone()
        })
        .collect();
    declared.capabilities[0].features = features;
    declared.capabilities[0].data = data;
    declared
}

/// A process may be an agent harness: it declares `launch`, `tool_use`
/// and `render`, each optional verb its feature asks for, and what it
/// declares of itself (instruction files, environment markers, settings)
/// as its capability's `data`, checked against the contract.
#[cfg(unix)]
#[test]
fn a_provider_may_implement_an_agent_harness_by_its_contract() {
    let spec = fake_spec("agent_harness");
    let data = json!({
        "env_markers": ["ACME_SESSION"],
        "settings": [{ "key": "model", "title": "Model" }]
    });
    let base = ["launch", "tool_use", "render"];
    spec::check_declarations(&spec, &harness_declarations(&base, json!({}), data.clone())).unwrap();
    spec::check_declarations(
        &spec,
        &harness_declarations(
            &["launch", "tool_use", "render", "turns"],
            json!({ "transcript": true, "terminal": true }),
            serde_json::Value::Null,
        ),
    )
    .unwrap();
    let err = spec::check_declarations(
        &spec,
        &harness_declarations(&["tool_use", "render"], json!({}), data.clone()),
    )
    .unwrap_err();
    assert!(err.contains("`launch`"), "{err}");
    let err = spec::check_declarations(
        &spec,
        &harness_declarations(&base, json!({ "transcript": true }), data.clone()),
    )
    .unwrap_err();
    assert!(err.contains("`turns`"), "{err}");
    let err = spec::check_declarations(
        &spec,
        &harness_declarations(&base, json!({}), json!({ "interact": "chat" })),
    )
    .unwrap_err();
    assert!(err.contains("agent_harness data"), "{err}");
    let err = spec::check_declarations(
        &spec,
        &harness_declarations(
            &base,
            json!({}),
            json!({ "settings": [{ "title": "Model" }] }),
        ),
    )
    .unwrap_err();
    assert!(err.contains("key"), "{err}");
}

/// A capability whose contract asks for nothing more takes no `data`.
#[cfg(unix)]
#[test]
fn a_snapshots_provider_declares_no_data() {
    let spec = fake_spec("snapshots");
    let mut declared = oxplow_provider_fake::snapshots_declarations(false);
    declared.capabilities[0].data = json!({ "anything": 1 });
    let err = spec::check_declarations(&spec, &declared).unwrap_err();
    assert!(err.contains("declares no `data`"), "{err}");
}

/// The manifest names any capability with a contract; one without (the
/// VCS), or an item-keeping capability's own fields on another, is refused.
#[test]
fn a_manifest_names_a_capability_a_process_may_implement() {
    let manifest = |capability: &str, extra: &str| {
        format!(
            "  - id: fake\n    capability: {capability}\n    entry: bin/provider\n    declarations: provider.json\n{extra}"
        )
    };
    let read = |rel: &str| {
        (rel == "provider.json").then(|| {
            serde_json::to_string(&oxplow_provider_fake::snapshots_declarations(false)).unwrap()
        })
    };
    let parse = |text: String| {
        spec::parse_providers(&serde_yaml::from_str(&text).unwrap(), &|rel| {
            if rel == "bin/provider" {
                Some(String::new())
            } else {
                read(rel)
            }
        })
    };
    let (specs, errors) = parse(manifest("snapshots", ""));
    assert_eq!((specs.len(), errors), (1, Vec::<String>::new()));
    let (_, errors) = parse(manifest("vcs", ""));
    assert!(
        errors[0].contains("`vcs` isn't one a provider can implement"),
        "{errors:?}"
    );
    let (_, errors) = parse(manifest(
        "snapshots",
        "    fields: [{ name: points, title: Points, kind: number }]\n",
    ));
    assert!(errors[0].contains("unknown field `fields`"), "{errors:?}");
    let (_, errors) = parse(manifest("snapshots", "    idPattern: \"P-\\\\d+\"\n"));
    assert!(
        errors[0].contains("unknown field `idPattern`") || errors[0].contains("unknown field"),
        "{errors:?}"
    );
}

/// Mark an empty worktree on `instance`'s process as the person, and
/// answer the handle it marked (`m1`, `m2`, …).
async fn mark_on(
    fx: &ThreadFixture,
    instance: &str,
) -> Result<String, oxplow_domain::CommandError> {
    let worktree = tempfile::tempdir().unwrap();
    let out = invoke_on(
        fx,
        instance,
        "mark",
        json!({ "stream": "s1", "worktree": worktree.path(), "trigger": "manual" }),
    )
    .await?;
    Ok(out.result["handle"].as_str().unwrap().to_string())
}

/// Call `command` of `instance`'s process as the person, straight at it.
async fn invoke_on(
    fx: &ThreadFixture,
    instance: &str,
    command: &str,
    input: serde_json::Value,
) -> Result<oxplow_provider_protocol::model::InvokeResult, oxplow_domain::CommandError> {
    let running = fx
        .svc
        .providers
        .get(instance)
        .await
        .expect("the instance exists");
    let run =
        oxplow_domain::run::RunContext::core(Actor::Human, oxplow_domain::run::Mode::External);
    running.invoke(&run, command, input, None).await
}

/// Waits until the fake runs: its health is ready.
async fn first_read(fx: &ThreadFixture) {
    for _ in 0..200 {
        if running(&fx.svc, "fake") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "the fake never became ready: {:?}",
        fx.svc.providers.health(INSTANCE)
    );
}

/// Send the running fake new script hooks.
async fn set_hooks(fx: &ThreadFixture, hooks: &str) {
    let instance = fx.svc.providers.get(INSTANCE).await.expect("running");
    instance.hook(hooks).await;
}

/// a hooks notification replaces the last one — `set_hooks(fx,
/// "")` clears a slow call — so a test that sets hooks and clears them
/// runs on the fake it means to.
#[cfg(unix)]
#[tokio::test]
async fn the_fakes_hooks_are_replaced_not_merged() {
    let (fx, _ext) = enabled_fake().await;
    set_hooks(&fx, "slow:30000").await;
    set_hooks(&fx, "").await;
    let created = tokio::time::timeout(std::time::Duration::from_secs(10), mark_on_fake(&fx))
        .await
        .expect("the slow hook was cleared");
    created.unwrap();
}

/// Mark on the fake as the person.
async fn mark_on_fake(fx: &ThreadFixture) -> Result<String, oxplow_domain::CommandError> {
    mark_on(fx, INSTANCE).await
}

#[cfg(unix)]
async fn enabled_fake() -> (ThreadFixture, Extension) {
    let (fx, ext) = deployed("").await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    // Its start's first read, in the background.
    first_read(&fx).await;
    (fx, ext)
}

/// P7.A4: rate limits interleaved with failures neither count nor reset
/// the count: three real failures still disable the instance.
#[cfg(unix)]
#[tokio::test]
async fn three_failures_disable_it_with_rate_limits_interleaved() {
    let (fx, _ext) = enabled_fake().await;
    set_hooks(&fx, "fail-next:1").await;
    assert!(mark_on_fake(&fx).await.is_err());
    set_hooks(&fx, "rate-limit:60000").await;
    assert!(mark_on_fake(&fx).await.is_err());
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        1
    );
    set_hooks(&fx, "fail-next:2").await;
    assert!(mark_on_fake(&fx).await.is_err());
    assert!(mark_on_fake(&fx).await.is_err());
    assert!(matches!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
}

/// Another instance of the tracker's provider at project level.
fn configure_named(
    fx: &ThreadFixture,
    instance: &str,
    provider: Option<&str>,
    config: serde_json::Value,
) {
    insert_instance(
        &fx.svc,
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

async fn instance_rows(fx: &ThreadFixture) -> Vec<(String, String, i64, String)> {
    let out = fx
        .svc
        .sql
        .query_sql(
            "SELECT instance, state, enabled, scope FROM v_provider_instance ORDER BY instance",
            vec![],
            None,
        )
        .await
        .unwrap();
    serde_json::to_value(&out.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let r = r.as_array().unwrap();
            (
                r[0].as_str().unwrap().to_string(),
                r[1].as_str().unwrap().to_string(),
                r[2].as_i64().unwrap(),
                r[3].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Settings → Integrations is commands over `v_provider_instance`: an
/// instance is checked without being written, turned on with its config,
/// another added and removed (a person confirms that), and a refusal names
/// the field.
#[cfg(unix)]
#[tokio::test]
async fn provider_instances_are_managed_by_commands_and_read_as_a_model() {
    let (fx, _ext) = deployed("").await;
    let run = |name: &'static str, input: serde_json::Value, confirmed: bool| {
        let commands = fx.svc.commands.clone();
        async move { commands.run(&Actor::Human, name, input, confirmed).await }
    };
    assert_eq!(
        instance_rows(&fx).await,
        vec![(INSTANCE.into(), "off".into(), 0, "project".into())]
    );

    // A check enables and writes nothing; its answer is the view.
    let checked = run(
        "oxplow.provider.check_instance",
        json!({ "instance": INSTANCE, "config": { "team": "core" } }),
        false,
    )
    .await
    .unwrap();
    assert_eq!(checked.result["health"]["state"]["state"], "ready");
    assert!(fx.svc.config.read().unwrap().extension_instances.is_empty());

    run(
        "oxplow.provider.set_instance",
        json!({ "instance": INSTANCE, "enabled": true, "config": { "team": "core" } }),
        false,
    )
    .await
    .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(
        instance_rows(&fx).await,
        vec![(INSTANCE.into(), "ready".into(), 1, "project".into())]
    );

    run(
        "oxplow.provider.add_instance",
        json!({ "instance": SECOND, "provider": "fake", "scope": "project" }),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        instance_rows(&fx).await,
        vec![
            (INSTANCE.into(), "ready".into(), 1, "project".into()),
            (SECOND.into(), "off".into(), 0, "project".into()),
        ]
    );
    let pinned = fx
        .svc
        .sql
        .query_sql(
            "SELECT instance FROM v_provider_instance WHERE instance = ?",
            vec![oxplow_db::SqlCell::Text(SECOND.into())],
            None,
        )
        .await
        .unwrap();
    assert_eq!(pinned.rows.len(), 1);

    // The Integrations component re-reads when this page's rows change: one
    // row per instance, naming its state and what it holds.
    let page = crate::test_fixtures::run_as_page(&fx.svc, "oxplow/integrations", &[]).await;
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.columns[0], "instance");
    assert!(page.columns.contains(&"credentials".to_string()));
    assert!(page.columns.contains(&"collectors".to_string()));

    // A refusal is at the field, and nothing was written.
    let refused = run(
        "oxplow.provider.set_credential",
        json!({ "instance": "tracker/nope", "name": "TOKEN", "value": "x" }),
        false,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("tracker/nope"), "{refused}");

    run(
        "oxplow.provider.remove_instance",
        json!({ "instance": SECOND }),
        true,
    )
    .await
    .unwrap();
    assert_eq!(instance_rows(&fx).await.len(), 1);
}

/// an instance oxplow can't run as configured — its id is a
/// command namespace something else already owns — says so on its row,
/// rather than showing enabled and "Off" with no reason.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_enable_says_why() {
    let (fx, _ext) = deployed("").await;
    fx.svc
        .commands
        .register_namespace("fake_second", "a test", Vec::new())
        .unwrap();
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    crate::test_fixtures::activate(&fx.svc).await;
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

/// a person's Enables on two rows at once — the second while the
/// first still checks — both stand. Each write is a read-modify-write of
/// the instances as they are when it writes, not as they were before its
/// check.
#[cfg(unix)]
#[tokio::test]
async fn enabling_two_instances_at_once_keeps_both() {
    let (fx, _ext) = deployed("slow-check:400").await;
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
    persist_instances(&fx.svc);
    let providers = fx.svc.providers.clone();
    let first = tokio::spawn(async move {
        providers
            .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
            .await
    });
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    fx.svc
        .providers
        .set_instance(&Actor::Human, SECOND, true, json!({ "team": "second" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
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
#[cfg(unix)]
#[tokio::test]
async fn two_instances_of_one_provider_run_side_by_side() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    configure_named(&fx, SECOND, Some("fake"), json!({ "team": "second" }));
    crate::test_fixtures::activate(&fx.svc).await;
    for name in [INSTANCE, SECOND] {
        assert_eq!(
            fx.svc.providers.health(name).map(|h| h.state),
            Some(InstanceState::Ready),
            "{name}"
        );
    }
    assert!(running(&fx.svc, "fake"));
    assert!(running(&fx.svc, "fake_second"));

    // Each instance is its own process: each numbers its own marks.
    assert_eq!(mark_on(&fx, SECOND).await.unwrap(), "m1");
    assert_eq!(mark_on(&fx, INSTANCE).await.unwrap(), "m1");
    assert_eq!(mark_on(&fx, INSTANCE).await.unwrap(), "m2");
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

    // Stopping one leaves the other.
    turn_off(&fx.svc, SECOND).await;
    assert!(!running(&fx.svc, "fake_second"));
    assert!(running(&fx.svc, "fake"));
    assert!(mark_on(&fx, INSTANCE).await.is_ok());
}

/// P9.B1: a credential is an instance's — set for one, another instance
/// of the same provider runs without it.
#[cfg(unix)]
#[tokio::test]
async fn an_instance_credential_is_its_own() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    let manifest = project
        .join(".oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!(
            "{}    credentials: [FAKE_TOKEN]\n",
            permit_secrets(&text, &["FAKE_TOKEN"])
        ),
    )
    .unwrap();
    let ext = extension(&project);
    deploy(&fx, &ext).await;
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
    crate::test_fixtures::activate(&fx.svc).await;
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
#[cfg(unix)]
#[tokio::test]
async fn a_named_instance_says_which_provider_it_is() {
    let (fx, _ext) = deployed("").await;
    configure_named(&fx, SECOND, None, json!({ "team": "second" }));
    crate::test_fixtures::activate(&fx.svc).await;
    match fx.svc.providers.health(SECOND).map(|h| h.state) {
        Some(InstanceState::Missing { reason }) => {
            assert!(
                reason.contains("provider: fake") && reason.contains("fake_second"),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(!running(&fx.svc, "fake_second"));

    // One is added by naming the provider; a provider the extension
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
    // An agent may add one too.
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    providers
        .add_instance(&agent, "tracker/other", "fake", Scope::Project)
        .await
        .unwrap();
    providers
        .remove_instance(&Actor::Human, "tracker/acme")
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
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
#[cfg(unix)]
async fn sibling_project(
    fx: &ThreadFixture,
    hooks: &str,
    with_tracker: bool,
) -> (std::sync::Arc<crate::Services>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    if with_tracker {
        write_extension(dir.path(), hooks);
    }
    let global = fx.svc.layout.state_dir.join("global-config");
    let svc = {
        crate::Services::in_memory_on_machine(
            dir.path(),
            global,
            fx.svc.secrets.clone(),
            crate::providers::host::process_env(),
        )
        .await
        .unwrap()
    };
    if with_tracker {
        approve_in(&svc).await;
    }
    (svc, dir)
}

/// Deploy the tracker's provider as it is in `svc`'s project.
async fn approve_in(svc: &std::sync::Arc<crate::Services>) {
    crate::test_fixtures::activate_with_problems(svc).await;
}

/// The tracker's provider in `root` declares the credential `FAKE_TOKEN`.
fn declare_token(root: &Path) {
    let manifest = root
        .join(".oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!(
            "{}    credentials: [FAKE_TOKEN]\n",
            permit_secrets(&text, &["FAKE_TOKEN"])
        ),
    )
    .unwrap();
}

/// a global instance's credential is the person's in every
/// project, so changing it in one restarts it in every other — on their
/// next tick — not only here: a rotated token must never keep running on
/// the old value elsewhere.
#[cfg(unix)]
#[tokio::test]
async fn a_global_credential_change_restarts_it_in_every_project() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    declare_token(&project);
    deploy(&fx, &extension(&project)).await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("one"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let (other, _other_dir) = sibling_project(&fx, "needs:FAKE_TOKEN", false).await;
    write_extension(&other.layout.project_dir, "needs:FAKE_TOKEN");
    declare_token(&other.layout.project_dir);
    approve_in(&other).await;
    crate::test_fixtures::activate(&other).await;
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
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(other.providers.reconcile_if_global_changed().await);
    crate::test_fixtures::activate_with_problems(&other).await;
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

/// a person turns a global instance off in one project from its
/// row: the project gets its own entry, off, with the global one's config
/// — every other project keeps running it, and removing the entry brings
/// the global one back here.
#[cfg(unix)]
#[tokio::test]
async fn a_global_instance_is_turned_off_in_one_project() {
    let (fx, _ext) = deployed("").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let (other, _other_dir) = sibling_project(&fx, "", true).await;
    crate::test_fixtures::activate(&other).await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));

    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    // An agent may turn it off here too.
    let view = other.providers.off_here(&agent, SHARED).await.unwrap();
    crate::test_fixtures::activate_with_problems(&other).await;
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
    crate::test_fixtures::activate_with_problems(&other).await;
    let settings = crate::settings::store(
        &other.layout.project_dir,
        other.providers.deps.global_dir.as_deref().unwrap(),
        other.secrets.clone(),
        &[],
    )
    .unwrap();
    assert!(!settings.snapshot().and_then(|snapshot| snapshot.get("oxplow.extensionInstances", Some(oxplow_config::settings::Level::Project))).unwrap().is_set,
        "removing the last project instance restores inheritance rather than writing an empty override");
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));
}

/// P9.B2: a project's entry of the same name replaces the global one
/// there, whole — and only there.
#[cfg(unix)]
#[tokio::test]
async fn a_project_entry_overrides_a_global_instance_whole() {
    let (fx, _ext) = deployed("").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let (other, _other_dir) = sibling_project(&fx, "", true).await;
    crate::test_fixtures::activate(&other).await;
    assert_eq!(state_of(&other, SHARED).await, Some(InstanceState::Ready));

    // The other project turns it off for itself.
    insert_instance(
        &other,
        SHARED.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: false,
            provider: Some("fake".into()),
            ..Default::default()
        },
    );
    crate::test_fixtures::activate(&other).await;
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

/// a project's entry is the project's own — its config and its
/// credentials — whether or not the person has a global instance of the
/// same name. One appearing (added in another project) or going doesn't
/// move this project's instance onto the global credentials, nor take its
/// own away.
#[cfg(unix)]
#[tokio::test]
async fn a_project_instance_keeps_its_credentials_beside_a_global_one() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "needs:FAKE_TOKEN");
    let manifest = project
        .join(".oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!(
            "{}    credentials: [FAKE_TOKEN]\n",
            permit_secrets(&text, &["FAKE_TOKEN"])
        ),
    )
    .unwrap();
    let ext = extension(&project);
    deploy(&fx, &ext).await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Project)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("the project's"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "core" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let credential_key = super::registry::provider_credential_key(EXT, "fake_shared", "FAKE_TOKEN");
    // The generic API and the provider read exactly the same registered secret.
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "oxplow.config.set",
            json!({"key":credential_key,"value":"local token","level":"local"}),
            true,
        )
        .await
        .unwrap();
    let credential_store = crate::settings::store(
        &project,
        providers.deps.global_dir.as_deref().unwrap(),
        fx.svc.secrets.clone(),
        std::slice::from_ref(&ext),
    )
    .unwrap();
    assert_eq!(
        credential_store
            .snapshot()
            .and_then(|snapshot| snapshot.get(&credential_key, None))
            .unwrap()
            .value,
        Some(json!("local token"))
    );
    assert!(credential_store
        .snapshot()
        .unwrap()
        .inventory()
        .iter()
        .map(|row| &row.setting)
        .any(|row| row.definition.key == credential_key && row.value.is_none() && row.is_set));
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "oxplow.config.unset",
            json!({"key":credential_key,"level":"local"}),
            true,
        )
        .await
        .unwrap();

    assert_eq!(
        credential_store
            .snapshot()
            .and_then(|snapshot| snapshot.get(&credential_key, None))
            .unwrap()
            .value,
        Some(json!("the project's")),
        "unsetting local inherits the project secret"
    );
    // In another project the person adds a global one of the same name.
    let global = fx.svc.layout.state_dir.join("global-config");
    let store = crate::settings::store(&project, &global, fx.svc.secrets.clone(), &[]).unwrap();
    store
        .set(
            "oxplow.extensionInstances",
            json!({
                (SHARED): { "enabled": true, "provider": "fake" }
            }),
            oxplow_config::settings::Level::Global,
        )
        .unwrap();
    assert_eq!(
        credential_store
            .snapshot()
            .and_then(|snapshot| snapshot.get(&credential_key, None))
            .unwrap()
            .value,
        Some(json!("the project's")),
        "creating global.yaml must not change secret identity"
    );
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
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));

    // The global one goes: nothing of this project's goes with it.
    store
        .set(
            "oxplow.extensionInstances",
            json!({}),
            oxplow_config::settings::Level::Global,
        )
        .unwrap();
    let view = here(providers.list().await);
    assert_eq!((view.scope, view.overridden), (Scope::Project, false));
    assert_eq!(
        credential_store
            .snapshot()
            .and_then(|snapshot| snapshot.get(&credential_key, None))
            .unwrap()
            .value,
        Some(json!("the project's"))
    );
}

/// P9.B2: a global instance's credential is the person's, set once for
/// every project; a project instance's stays that project's.
#[cfg(unix)]
#[tokio::test]
async fn a_global_instances_credential_is_shared_across_projects() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    let credentialed = |root: &Path| {
        let manifest = root
            .join(".oxplow/extensions")
            .join(EXT)
            .join("extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            format!(
                "{}    credentials: [FAKE_TOKEN]\n",
                permit_secrets(&text, &["FAKE_TOKEN"])
            ),
        )
        .unwrap();
    };
    write_extension(&project, "needs:FAKE_TOKEN");
    credentialed(&project);
    let ext = extension(&project);
    deploy(&fx, &ext).await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    providers
        .set_credential(SHARED, "FAKE_TOKEN", Some("s3cret"))
        .unwrap();
    providers
        .set_instance(&Actor::Human, SHARED, true, json!({ "team": "shared" }))
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(state_of(&fx.svc, SHARED).await, Some(InstanceState::Ready));

    // Another project: the same extension, deployed there; nothing set there.
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    write_extension(dir.path(), "needs:FAKE_TOKEN");
    credentialed(dir.path());
    let global = fx.svc.layout.state_dir.join("global-config");
    let other = crate::Services::in_memory_on_machine(
        dir.path(),
        global,
        fx.svc.secrets.clone(),
        crate::providers::host::process_env(),
    )
    .await
    .unwrap();
    // Deploying its sources reloads its config, so the project instance is
    // added after.
    crate::test_fixtures::activate_with_problems(&other).await;
    // A project instance of the same provider there has no token of its own.
    insert_instance(
        &other,
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config: json!({ "team": "core" }),
            ..Default::default()
        },
    );
    crate::test_fixtures::activate(&other).await;
    // A project map replaces the global map atomically: SHARED is absent.
    assert_eq!(state_of(&other, SHARED).await, None);
    assert!(matches!(
        state_of(&other, INSTANCE).await,
        Some(InstanceState::Unconfigured { .. })
    ));
}

/// The host ends a process whose authority was revoked (its extension
/// disabled, its config turned off) before the generation that says so is
/// published. That exit is the host's doing: not a failure counted against
/// the instance, and not restarted.
#[cfg(unix)]
#[tokio::test]
async fn a_process_the_host_ended_for_revoked_authority_counts_no_failure() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    let running = fx.svc.providers.get(INSTANCE).await.unwrap();
    let (peer, _) = running.connection_if_running().await.unwrap();
    // Turned off in the config, before any generation says so.
    configure(&fx, false, json!({ "team": "core" }));
    tokio::time::timeout(std::time::Duration::from_secs(10), peer.closed())
        .await
        .expect("the monitor ends a revoked process");
    // The supervisor's side of the exit has run once the process is gone
    // from the instance.
    for _ in 0..200 {
        if !running.has_process().await {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(health.consecutive_failures, 0, "{:?}", health.state);
    assert!(!matches!(health.state, InstanceState::Failing { .. }));
}

/// a call's outcome is its instance's while that instance runs.
/// One that finishes after the instance was stopped — the first read an
/// enable starts, say — changes nothing: not "Ready" for something that
/// isn't running, not a failure counted against it.
#[cfg(unix)]
#[tokio::test]
async fn a_call_finishing_after_its_instance_stopped_changes_nothing() {
    let (fx, ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    let was_running = fx.svc.providers.get(INSTANCE).await.unwrap();
    configure(&fx, false, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
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
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    // The successor is the running one once it is Ready (its first read
    // is a read of the one before it, so it says nothing of this).
    for _ in 0..200 {
        let successor = fx.svc.providers.get(INSTANCE).await;
        if successor.is_some_and(|i| !std::sync::Arc::ptr_eq(&i, &was_running))
            && state() == Some((InstanceState::Ready, 0))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(state(), Some((InstanceState::Ready, 0)));
    fx.svc
        .providers
        .call_failed(&was_running, "the old process went away".into())
        .await;
    assert_eq!(state(), Some((InstanceState::Ready, 0)));
}

/// A stopped instance stays registered until a generation replaces it, so a
/// call that finishes in between must not count against it: whether it is the
/// running one is whether it was stopped, not only whether it is registered.
#[cfg(unix)]
#[tokio::test]
async fn a_call_finishing_after_a_plain_stop_changes_nothing() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    let was_running = fx.svc.providers.get(INSTANCE).await.unwrap();
    let before = fx
        .svc
        .providers
        .health(INSTANCE)
        .map(|h| h.consecutive_failures);
    assert!(fx.svc.providers.stop(INSTANCE).await);
    fx.svc
        .providers
        .call_failed(&was_running, "the process went away".into())
        .await;
    let after = fx
        .svc
        .providers
        .health(INSTANCE)
        .map(|h| h.consecutive_failures);
    assert_eq!(after, before, "a late failure after a stop");
}

/// A generation restarts only the instances it changed: an unrelated
/// extension's edit leaves a running provider's process alone, while an
/// edit to its own program or its config restarts it.
#[cfg(unix)]
#[tokio::test]
async fn a_generation_restarts_only_the_instances_it_changed() {
    let (fx, ext) = deployed("").await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    first_read(&fx).await;
    let running = fx.svc.providers.get(INSTANCE).await.expect("running");

    // Another extension is added: nothing of the provider changed.
    let other = fx.svc.layout.project_dir.join(".oxplow/extensions/other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("extension.yaml"),
        "manifest: 2\nname: other\nintent:\n  purpose: unrelated\n",
    )
    .unwrap();
    // Deploying `other` turns it on in the person's file and reloads the
    // config; the instance's in-memory entry is put back before the
    // generation that carries it publishes.
    let kept = fx.svc.config.read().unwrap().extension_instances.clone();
    let changes = crate::extension::extension_deploy::stage_sources(&fx.svc).await;
    fx.svc.config.write().unwrap().extension_instances = kept;
    persist_instances(&fx.svc);
    let authority = running.host_calls.authority_current();
    fx.svc
        .extension_activation
        .activate_live(fx.svc.clone(), &changes)
        .await
        .unwrap();
    let after = fx.svc.providers.get(INSTANCE).await.expect("still running");
    assert!(
        std::sync::Arc::ptr_eq(&running, &after),
        "an unrelated edit restarted the provider: authority before the activation {authority}, \
         same scope {}, config {}, spec {}, extension {}, declarations {}",
        running.scope == after.scope,
        running.config == after.config,
        running.spec == after.spec,
        {
            let mut other = after.ext.clone();
            other.public_contracts.activation_index = running.ext.public_contracts.activation_index;
            running.ext == other
        },
        running.declared == after.declared,
    );
    assert_eq!(
        fx.svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );

    // Its config changes: a new instance.
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "other" })).await;
    let reconfigured = fx.svc.providers.get(INSTANCE).await.expect("running");
    assert!(
        !std::sync::Arc::ptr_eq(&running, &reconfigured),
        "a config edit kept the old process"
    );
    assert!(running.is_stopped(), "the old one was retired");

    // Its own program changes (re-approved): a new instance again.
    let before = fx.svc.providers.get(INSTANCE).await.unwrap();
    let script = fx
        .svc
        .layout
        .project_dir
        .join(".oxplow/extensions/tracker/bin/provider");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, format!("{text}\n# edited\n")).unwrap();
    deploy(&fx, &extension(&fx.svc.layout.project_dir)).await;
    let edited = fx.svc.providers.get(INSTANCE).await.expect("running");
    assert!(
        !std::sync::Arc::ptr_eq(&before, &edited),
        "a program edit kept the old process"
    );
}

/// The tracker extension, its provider's credential `FAKE_TOKEN` obtained
/// by signing in at `authorize` / `token` (P9.B3), plus `more` credential
/// entries (YAML list items, indented six spaces).
#[cfg(unix)]
fn write_oauth_extension(project: &Path, hooks: &str, authorize: &str, token: &str, more: &str) {
    write_extension(project, hooks);
    let manifest = project
        .join(".oxplow/extensions")
        .join(EXT)
        .join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let mut names = vec!["FAKE_TOKEN".to_string()];
    for line in more.lines() {
        let line = line.trim().trim_start_matches("- ");
        if let Some(name) = line.strip_prefix("name: ") {
            names.push(name.to_string());
        } else if !line.is_empty() && !line.contains(':') {
            names.push(line.to_string());
        }
    }
    let text = permit_secrets(&text, &names.iter().map(String::as_str).collect::<Vec<_>>());
    std::fs::write(
        &manifest,
        format!(
            "{text}    credentials:\n      - name: FAKE_TOKEN\n        oauth:\n          authorize_url: {authorize}\n          token_url: {token}\n          client_id: oxplow-test\n          scopes: [read, write]\n{more}"
        ),
    )
    .unwrap();
}

/// P9.B3: a credential may be one the person signs in for. It is declared
/// with its endpoints and checked at load.
#[cfg(unix)]
#[tokio::test]
async fn an_oauth_credential_is_declared_and_checked() {
    let fx = services_with_thread().await;
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
    let refused = |authorize: &str, more: &str, says: &str| {
        write_oauth_extension(&project, "", authorize, token, more);
        let loaded = crate::extension::extensions::load_extensions(&project)
            .into_iter()
            .find(|e| e.name == EXT)
            .unwrap();
        assert!(loaded.providers.is_empty(), "{says}");
        assert!(
            loaded.errors.iter().any(|e| loaded.shown(e).contains(says)),
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
#[cfg(unix)]
async fn signing_in(hooks: &str, more: &str) -> (ThreadFixture, OAuthSim) {
    let fx = services_with_thread().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    write_oauth_extension(&project, hooks, &sim.authorize_url, &sim.token_url, more);
    deploy(&fx, &extension(&project)).await;
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    (fx, sim)
}

/// The port the shell catches a test sign-in's redirect on. Nothing
/// listens: the redirect is read off the authorization server's answer
/// and handed over as the shell would.
const REDIRECT_PORT: u16 = 8124;

/// What the person's browser does with a sign-in's page: the redirect it
/// is sent back with, as the shell catches it (its path and query).
async fn browse(page: &crate::providers::BegunSignIn) -> String {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = client.get(&page.url).send().await.unwrap();
    let to = url::Url::parse(resp.headers()["location"].to_str().unwrap()).unwrap();
    format!("{}?{}", to.path(), to.query().unwrap_or_default())
}

/// Hand `redirect` to the sign-in for `name`, as the shell does.
async fn complete(
    fx: &ThreadFixture,
    name: &str,
    redirect: &str,
) -> Result<SignInCompletion, DomainError> {
    fx.svc
        .providers
        .complete_sign_in(INSTANCE, name, redirect)
        .await
}

/// a sign-in stores its token to the account it began for. When
/// the instance's entry changes under it — a global one turned off here,
/// so the project's own entry replaces it — the redirect is refused and
/// nothing is stored to the old account.
#[cfg(unix)]
#[tokio::test]
async fn a_sign_in_whose_account_changed_meanwhile_is_refused() {
    let (fx, sim) = signing_in("", "").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
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
async fn sign_in(fx: &ThreadFixture, name: &str) -> Option<String> {
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
                ..
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
    // The saved credential asks for a generation that starts the instance.
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
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

fn unconfigured_at(fx: &ThreadFixture) -> (String, String) {
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
#[cfg(unix)]
#[tokio::test]
async fn signing_in_starts_the_instance_on_the_access_token_alone() {
    // The fake's service takes the first access token the stand-in issues
    // and nothing else — not the keychain's record of it.
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=at-2", "      - PLAIN\n").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    let (path, message) = unconfigured_at(&fx);
    assert_eq!(path, "/credentials/FAKE_TOKEN");
    assert!(message.contains("isn't signed in"), "{message}");
    assert!(!running(&fx.svc, "fake"), "nothing registered");
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
    mark_on_fake(&fx).await.unwrap();
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
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(unconfigured_at(&fx).0, "/credentials/FAKE_TOKEN");
    assert!(!running(&fx.svc, "fake"));
}

/// P9.B3: a token about to lapse is renewed before the process starts.
#[cfg(unix)]
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
    crate::test_fixtures::activate(&fx.svc).await;
    assert_eq!(
        fx.svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
}

/// P9.B3: a token its service refuses (`Auth`) although it looks good is
/// renewed, the process restarted on the new one, and the call tried once
/// more — none of which counts as a failure.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_token_is_renewed_and_the_call_tried_once_more() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    // Its service now takes only the next token; the process holds at-2.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
    mark_on_fake(&fx).await.unwrap();
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(
        (health.state, health.consecutive_failures),
        (InstanceState::Ready, 0)
    );
    // The process that answered was started on the renewed token.
    set_hooks(&fx, "accepts:FAKE_TOKEN=at-3").await;
    mark_on_fake(&fx).await.unwrap();
    assert_eq!(sim.grants().len(), 2);
    assert_eq!(
        fx.svc
            .providers
            .health(INSTANCE)
            .unwrap()
            .consecutive_failures,
        0
    );
}

/// a call that starts the process itself, and is refused, renews
/// the token — the process it started began after the read did, which
/// isn't "started since" the refusal.
#[cfg(unix)]
#[tokio::test]
async fn a_call_that_starts_the_process_renews_a_refused_token() {
    // Its service takes only the renewed token; its `check` doesn't look.
    let (fx, sim) = signing_in("accepts:FAKE_TOKEN=at-3,lax-check", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    // The process goes away, so the next call starts it.
    set_hooks(&fx, "crash").await;
    let _ = mark_on_fake(&fx).await;
    mark_on_fake(&fx).await.unwrap();
    assert_eq!(refreshes(&sim), 1, "{:?}", sim.grants());
}

/// Two credentials signed in at the stand-in, `FAKE_TOKEN` and
/// `OTHER_TOKEN`, the instance ready on both.
#[cfg(unix)]
async fn signed_in_twice(hooks: &str) -> (ThreadFixture, OAuthSim) {
    let fx = services_with_thread().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    let other = format!(
        "      - name: OTHER_TOKEN\n        oauth:\n          authorize_url: {}\n          token_url: {}\n          client_id: oxplow-test\n",
        sim.authorize_url, sim.token_url
    );
    write_oauth_extension(&project, hooks, &sim.authorize_url, &sim.token_url, &other);
    deploy(&fx, &extension(&project)).await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    assert_eq!(sign_in(&fx, "OTHER_TOKEN").await, None);
    first_read(&fx).await;
    (fx, sim)
}

/// a slow token exchange holds up only its own credential's
/// sign-in — another credential's begins meanwhile.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_exchange_holds_up_only_its_own_sign_in() {
    let fx = services_with_thread().await;
    let sim = OAuthSim::start().await;
    let project = fx.svc.layout.project_dir.clone();
    let other = format!(
        "      - name: OTHER_TOKEN\n        oauth:\n          authorize_url: {}\n          token_url: {}\n          client_id: oxplow-test\n",
        sim.authorize_url, sim.token_url
    );
    write_oauth_extension(&project, "", &sim.authorize_url, &sim.token_url, &other);
    deploy(&fx, &extension(&project)).await;
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

/// once a redirect is taken as the sign-in's, the finish runs to
/// its end even when its caller goes away mid-exchange (a dropped
/// connection to a remote daemon): the token is kept and the renderer
/// hears it, rather than the sign-in vanishing.
#[cfg(unix)]
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

/// the browser hears how its sign-in went as soon as the token is
/// kept — not after the instance has restarted on it (a slow `check`).
#[cfg(unix)]
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

/// The next `CredentialChanged` for `name`: its sign-in and error.
async fn next_news(
    ui: &mut tokio::sync::broadcast::Receiver<crate::events::OxplowEvent>,
    name: &str,
) -> (Option<u32>, Option<String>) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(crate::events::OxplowEvent::CredentialChanged {
                name: of,
                sign_in,
                error,
                ..
            }) = ui.recv().await
            {
                if of == name {
                    return (sign_in, error);
                }
            }
        }
    })
    .await
    .expect("news of the sign-in")
}

/// a sign-in replaced by a newer one (another window's Sign in)
/// says so, naming itself, so the row waiting on it stops waiting; and a
/// person's cancel ends a sign-in at once — nothing of it kept — with its
/// news saying so.
#[cfg(unix)]
#[tokio::test]
async fn a_replaced_or_cancelled_sign_in_is_announced_by_its_number() {
    let (fx, _sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let providers = &fx.svc.providers;
    let mut ui = fx.svc.events.subscribe_ui();
    let first = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let second = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    assert_ne!(first.sign_in, second.sign_in);
    let (of, error) = next_news(&mut ui, "FAKE_TOKEN").await;
    assert_eq!(of, Some(first.sign_in));
    assert!(error.unwrap().contains("replaced"));
    // A cancel for the old one does nothing; for the current one, it ends.
    providers
        .cancel_sign_in(INSTANCE, "FAKE_TOKEN", first.sign_in)
        .await;
    providers
        .cancel_sign_in(INSTANCE, "FAKE_TOKEN", second.sign_in)
        .await;
    let (of, error) = next_news(&mut ui, "FAKE_TOKEN").await;
    assert_eq!(of, Some(second.sign_in));
    assert!(error.unwrap().contains("cancelled"));
    let redirect = browse(&second).await;
    assert!(
        complete(&fx, "FAKE_TOKEN", &redirect).await.is_err(),
        "nothing kept"
    );
}

fn refreshes(sim: &OAuthSim) -> usize {
    sim.grants()
        .iter()
        .filter(|g| *g == "refresh_token")
        .count()
}

/// an `Auth` that names nothing, from a provider handed a pasted
/// key beside its sign-in, could be about either: it renews nothing, and
/// the sign-in stays good — the refusal is the call's failure.
#[cfg(unix)]
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
    let err = mark_on_fake(&fx).await.unwrap_err().to_string();
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

/// an `Auth` that names its credential renews that one alone.
#[cfg(unix)]
#[tokio::test]
async fn an_auth_renews_only_the_credential_it_names() {
    let (fx, sim) = signed_in_twice("").await;
    // A refused token restarts the instance through the owner of
    // generations, as it does in the app.
    fx.svc.extension_activation.spawn(&fx.svc);
    // Its first generation (from the sign-ins' requests) comes before the
    // hook is set: the hook is the running instance's.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    first_read(&fx).await;
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    // Renewed and tried again on a fresh process (which starts without
    // the hook), so the call lands.
    mark_on_fake(&fx).await.unwrap();
    assert_eq!(refreshes(&sim), 1, "FAKE_TOKEN only: {:?}", sim.grants());
}

/// renewing the credential an `Auth` named never touches another:
/// when the service refuses the renewal for good, only the named one
/// reads "sign in again".
#[cfg(unix)]
#[tokio::test]
async fn an_auth_never_lapses_a_credential_it_didnt_name() {
    let (fx, sim) = signed_in_twice("").await;
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    sim.revoke();
    assert!(mark_on_fake(&fx).await.is_err());
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

/// an `Auth` that names no credential renews one only when the
/// instance signs in for exactly one — with two, it can't know which, so
/// it renews none and the refusal is the call's failure.
#[cfg(unix)]
#[tokio::test]
async fn an_unnamed_auth_with_two_sign_ins_renews_nothing() {
    let (fx, sim) = signed_in_twice("").await;
    set_hooks(&fx, "refuse-auth").await;
    let err = mark_on_fake(&fx).await.unwrap_err().to_string();
    assert!(err.contains("authentication failed"), "{err}");
    assert_eq!(refreshes(&sim), 0, "{:?}", sim.grants());
}

/// with one signed-in credential, an unnamed `Auth` is its.
#[cfg(unix)]
#[tokio::test]
async fn an_unnamed_auth_with_one_sign_in_renews_it() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    set_hooks(&fx, "refuse-auth").await;
    mark_on_fake(&fx).await.unwrap();
    assert_eq!(refreshes(&sim), 1, "{:?}", sim.grants());
}

/// P9.B3: renewed once, not for ever — a service that refuses the renewed
/// token too is a failure like any other.
#[cfg(unix)]
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
#[cfg(unix)]
#[tokio::test]
async fn a_revoked_sign_in_stops_the_instance_and_says_sign_in_again() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    assert_eq!(sign_in(&fx, "FAKE_TOKEN").await, None);
    first_read(&fx).await;
    assert!(running(&fx.svc, "fake"));
    sim.revoke();
    set_hooks(&fx, "accepts:FAKE_TOKEN=never").await;
    let err = mark_on_fake(&fx).await.unwrap_err().to_string();
    assert!(err.contains("sign in again"), "{err}");
    // The next generation can't start it without its credential.
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let (path, message) = unconfigured_at(&fx);
    assert_eq!(path, "/credentials/FAKE_TOKEN");
    assert!(message.contains("sign in again"), "{message}");
    assert!(!running(&fx.svc, "fake"), "nothing registered");
    assert!(!running(&fx.svc, "fake"));
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
#[cfg(unix)]
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
    crate::test_fixtures::activate(&fx.svc).await;
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
#[cfg(unix)]
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
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
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
#[cfg(unix)]
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
#[cfg(unix)]
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

/// P10: finishing re-checks the endpoints: a provider whose token
/// endpoint changed since the sign-in began sends its code nowhere.
#[cfg(unix)]
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
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let failed = complete(&fx, "FAKE_TOKEN", &redirect).await.unwrap();
    assert!(
        matches!(&failed, SignInCompletion::Failed { error } if error.contains("changed since the sign-in began")),
        "{failed:?}"
    );
    assert!(
        sim.grants().is_empty(),
        "nothing reached any token endpoint"
    );
}

/// two sign-ins under way at once — another instance's — each
/// takes only its own redirect: one handed the other's is refused and
/// waits on, and nothing is exchanged for the wrong one.
#[cfg(unix)]
#[tokio::test]
async fn one_sign_ins_redirect_is_never_anothers() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    const OTHER: &str = "tracker/other";
    insert_instance(
        &fx.svc,
        OTHER.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config: json!({ "team": "core" }),
            sync_minutes: None,
            provider: Some("fake".into()),
        },
    );
    let providers = &fx.svc.providers;
    let mine = providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let theirs = providers
        .begin_sign_in(OTHER, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let (my_redirect, their_redirect) = (browse(&mine).await, browse(&theirs).await);
    let crossed = providers
        .complete_sign_in(OTHER, "FAKE_TOKEN", &my_redirect)
        .await
        .unwrap();
    assert!(
        matches!(crossed, SignInCompletion::NotThisSignIn { .. }),
        "{crossed:?}"
    );
    assert!(sim.grants().is_empty(), "nothing exchanged");
    // Both still wait for their own.
    for (instance, redirect) in [(OTHER, &their_redirect), (INSTANCE, &my_redirect)] {
        assert!(matches!(
            providers
                .complete_sign_in(instance, "FAKE_TOKEN", redirect)
                .await
                .unwrap(),
            SignInCompletion::SignedIn
        ));
    }
}

/// a sign-in finished after its new endpoints were approved
/// meanwhile still sends its code nowhere: it began against the old ones,
/// and only the comparison with the declaration it began with catches it
/// (the approval check passes).
#[cfg(unix)]
#[tokio::test]
async fn a_sign_in_whose_approved_endpoints_changed_meanwhile_sends_nothing() {
    let (fx, sim) = signing_in("", "").await;
    configure(&fx, true, json!({ "team": "core" }));
    let page = fx
        .svc
        .providers
        .begin_sign_in(INSTANCE, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    let redirect = browse(&page).await;
    let elsewhere = OAuthSim::start().await;
    write_oauth_extension(
        &fx.svc.layout.project_dir,
        "",
        &sim.authorize_url,
        &elsewhere.token_url,
        "",
    );
    // A person approves the new endpoints before the redirect comes back.
    deploy(&fx, &extension(&fx.svc.layout.project_dir)).await;
    let failed = complete(&fx, "FAKE_TOKEN", &redirect).await.unwrap();
    assert!(
        matches!(&failed, SignInCompletion::Failed { error } if error.contains("signs in changed")),
        "{failed:?}"
    );
    assert!(sim.grants().is_empty() && elsewhere.grants().is_empty());
}

/// P10: a sign-in never finished ends after its wait: the renderer hears
/// why, and its redirect, coming late, finds none under way.
#[cfg(unix)]
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

/// two sign-ins for one credential started at once leave one
/// under way — the later one — and the earlier one's redirect does
/// nothing.
#[cfg(unix)]
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

/// removing a project's replacement of a global instance ends a
/// sign-in under way for it too.
#[cfg(unix)]
#[tokio::test]
async fn removing_a_project_replacement_ends_its_sign_in() {
    let (fx, _sim) = signing_in("", "").await;
    let providers = &fx.svc.providers;
    providers
        .add_instance(&Actor::Human, SHARED, "fake", Scope::Global)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    configure_named(&fx, SHARED, Some("fake"), json!({ "team": "mine" }));
    let page = providers
        .begin_sign_in(SHARED, "FAKE_TOKEN", REDIRECT_PORT)
        .await
        .unwrap();
    providers
        .remove_instance(&Actor::Human, SHARED)
        .await
        .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let refused = providers
        .complete_sign_in(SHARED, "FAKE_TOKEN", &browse(&page).await)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("under way"), "{refused}");
}

/// what waits for the providers the config names (the automatic
/// retries' first tick) waits for the registry's first reconcile.
#[tokio::test]
async fn the_first_reconcile_is_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let svc = crate::Services::in_memory_inactive(dir.path()).unwrap();
    let waited = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        svc.providers.first_reconciled(),
    )
    .await;
    assert!(waited.is_err(), "not before it has run");
    svc.initialize_extensions().await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        svc.providers.first_reconciled(),
    )
    .await
    .expect("at once after it has");
}

/// Stopping an instance asks its process to shut down — it finishes and
/// persists what it was doing — rather than killing it outright.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_shuts_the_process_down_gracefully() {
    let marker = tempfile::tempdir().unwrap();
    let file = marker.path().join("shut-down");
    let (fx, _ext) = deployed(&format!("shutdown-file:{}", file.display())).await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    assert!(fx.svc.providers.stop(INSTANCE).await);
    assert!(file.is_file(), "the provider was told to shut down");
}

/// A process that answers `shutdown` and keeps running is killed once its
/// grace is up: a stop never hangs on it.
#[cfg(unix)]
#[tokio::test]
async fn a_process_that_wont_exit_is_killed_after_its_grace() {
    let (fx, _ext) = deployed("ignore-shutdown").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fx.svc.providers.stop(INSTANCE),
    )
    .await
    .expect("the stop ends once the grace is up");
    assert!(stopped);
}

/// A process that exits on its own is noticed when it exits, not at the
/// next call: the exit counts toward its health and it starts again.
#[cfg(unix)]
#[tokio::test]
async fn a_process_that_exits_is_restarted_and_counted() {
    let (fx, _ext) = deployed("").await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    let instance = fx.svc.providers.get(INSTANCE).await.unwrap();
    let (peer, _) = instance.connection().await.unwrap();
    peer.notify("fake/exit", json!({})).await.unwrap();
    let mut restarted = false;
    for _ in 0..200 {
        let health = fx.svc.providers.health(INSTANCE).unwrap();
        let live = instance.connection_if_running().await;
        if health.consecutive_failures >= 1
            && live.is_some_and(|(p, _)| !p.is_closed() && !p.same_connection(&peer))
        {
            restarted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(restarted, "{:?}", fx.svc.providers.health(INSTANCE));
}

/// Leaving (the daemon exits) shuts every running instance's process down
/// the orderly way, all at once.
#[cfg(unix)]
#[tokio::test]
async fn shutting_down_ends_every_process_gracefully() {
    let marker = tempfile::tempdir().unwrap();
    let file = marker.path().join("shut-down");
    let (fx, _ext) = deployed(&format!("shutdown-file:{}", file.display())).await;
    configure(&fx, true, json!({ "team": "core" }));
    crate::test_fixtures::activate(&fx.svc).await;
    first_read(&fx).await;
    fx.svc.providers.shutdown_all().await;
    assert!(file.is_file(), "the provider was told to shut down");
    assert!(fx
        .svc
        .providers
        .get(INSTANCE)
        .await
        .is_none_or(|instance| instance.is_stopped()));
}

/// A provider's `needs` are the scopes its `host/call`s reach — a
/// script's — so one `host/call` can't serve is refused at load, the rule
/// commands and effects meet.
#[cfg(unix)]
#[tokio::test]
async fn a_provider_needs_only_scopes_host_call_serves() {
    let root = tempfile::tempdir().unwrap();
    write_extension(root.path(), "");
    let manifest = root
        .path()
        .join(".oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}    needs: [config.read]\n")).unwrap();
    let loaded = crate::extension::extensions::load_extensions(root.path())
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    let errs = loaded
        .errors
        .iter()
        .map(|d| loaded.shown(d))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        errs.is_empty(),
        "registered settings reads are a host service: {errs}"
    );
    std::fs::write(&manifest, format!("{text}    needs: [config.write]\n")).unwrap();
    let loaded = crate::extension::extensions::load_extensions(root.path())
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    let errs = loaded
        .errors
        .iter()
        .map(|d| loaded.shown(d))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        errs.contains("`config.write` is a scope a script can't call"),
        "{errs}"
    );
}

const HARNESS: &str = "conductor/fake";

/// A private extension whose provider runs the fake as an agent harness
/// with `hooks`.
#[cfg(unix)]
fn write_harness_extension(project: &Path, hooks: &str) {
    let dir = project.join(".oxplow/extensions/conductor");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: conductor\nintent:\n  purpose: the fake agent harness\n  examples: [{ name: a }]\npermissions: [process.run]\nproviders:\n  - id: fake\n    capability: agent_harness\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=agent_harness OXPLOW_FAKE_HOOKS='{hooks}' OXPLOW_FAKE_STATE=\"{}/fake-harness-$OXPLOW_PROVIDER_ID.json\" exec '{}' \"$@\"\n",
            project.join(".oxplow").display(),
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::harness_declarations()).unwrap(),
    )
    .unwrap();
}

/// The fixture with the harness extension approved and its instance
/// running.
#[cfg(unix)]
async fn running_harness(hooks: &str) -> (ThreadFixture, Extension) {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_harness_extension(&project, hooks);
    let ext = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == "conductor")
        .expect("the harness extension loads");
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    deploy(&fx, &ext).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    (fx, ext)
}

/// An enabled harness provider is a registered harness under its instance
/// id, with what it declared — its instruction files, environment markers
/// and settings — beside the built-ins; a catalog reload keeps it, and
/// stopping it takes it out.
#[cfg(unix)]
#[tokio::test]
async fn a_harness_provider_is_a_registered_harness() {
    let (fx, _ext) = running_harness("").await;
    let h = fx.svc.harnesses.get("fake").expect("registered as `fake`");
    assert_eq!(h.title(), "fake");
    assert_eq!(h.env_markers(), ["OXPLOW_FAKE_HARNESS_SESSION".to_string()]);
    assert_eq!(h.settings()[0].key, "model");
    assert!(crate::agent_path::not_inherited(&fx.svc.harnesses)
        .contains(&"OXPLOW_FAKE_HARNESS_SESSION".to_string()));
    let listed =
        crate::harnesses::listing(&fx.svc.harnesses, &fx.svc.capabilities, &Default::default());
    assert!(listed
        .iter()
        .any(|l| l.id == "fake" && l.settings[0].key == "model"));

    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(fx.svc.harnesses.has("fake"), "a reload keeps it");
    assert!(fx.svc.harnesses.has("claude"), "beside the built-ins");

    turn_off(&fx.svc, HARNESS).await;
    assert!(!fx.svc.harnesses.has("fake"));
}

/// Its verbs are its process's: a tool hook maps, an answer renders in
/// its own shape, an export's records read in one call, and a launch
/// answers the command its terminal runs — the bearer in its env only. A
/// verb it doesn't declare (`turns`) answers none without a call.
#[cfg(unix)]
#[tokio::test]
async fn a_harness_providers_verbs_are_its_process() {
    use oxplow_domain::agent::observe::{AttrValue, Attrs, HookAnswer, OtlpRecord};
    let (fx, _ext) = running_harness("").await;
    let h = fx.svc.harnesses.get("fake").unwrap();

    let tool = h
        .tool_use(&json!({ "tool_name": "Edit", "tool_input": { "file_path": "a.txt" } }))
        .await
        .expect("an edit");
    assert_eq!(tool.kind, oxplow_domain::agent::tool::ToolKind::Edit);
    assert_eq!(tool.paths, ["a.txt"]);
    assert_eq!(h.tool_use(&json!({ "nothing": 1 })).await, None);

    assert_eq!(
        h.render(&HookAnswer::Deny {
            reason: "no".into()
        })
        .await,
        json!({ "fake": "deny", "reason": "no" })
    );

    let point = |kind: &str, value| OtlpRecord::Point {
        metric: oxplow_harness_fake::TOKEN_METRIC.into(),
        value,
        attributes: Attrs(vec![("kind".into(), AttrValue::Str(kind.into()))]),
        resource: Attrs(vec![("model".into(), AttrValue::Str("m".into()))]),
        time_unix_nano: 9,
        start_time_unix_nano: 0,
    };
    let readings = h
        .token_readings(&[point("input", 7), point("output", 3)])
        .await;
    assert_eq!(readings.iter().map(|r| r.value).collect::<Vec<_>>(), [7, 3]);
    assert!(h.turns("anything").await.is_empty());

    let bin_dir = tempfile::tempdir().unwrap();
    std::fs::write(bin_dir.path().join(oxplow_harness_fake::BIN), "").unwrap();
    let launch = h
        .launch(&launch_input(&fx, vec![bin_dir.path().to_path_buf()]))
        .await
        .expect("it launches");
    let oxplow_domain::agent::harness::LaunchSpec::Pty { command, env } = launch.spec else {
        panic!("a terminal launch");
    };
    assert!(command.contains(oxplow_harness_fake::BIN), "{command}");
    assert!(!command.contains("secret-bearer"), "{command}");
    assert!(env.iter().any(|(_, v)| v == "secret-bearer"));
}

/// A harness provider keeps the floor the built-ins keep: the harness
/// conformance suite over its process, its program found where the launch
/// looks.
#[cfg(unix)]
#[tokio::test]
async fn a_harness_provider_passes_the_harness_suite() {
    let (fx, _ext) = running_harness("").await;
    let bin_dir = tempfile::tempdir().unwrap();
    std::fs::write(bin_dir.path().join(oxplow_harness_fake::BIN), "").unwrap();
    let run = crate::harness_conformance::suite(
        &fx.svc,
        "fake",
        &json!({}),
        &[bin_dir.path().to_path_buf()],
    )
    .await;
    assert!(run.findings.is_empty(), "{:#?}", run.findings);
    // Its program missing, the launch fails — and the suite says so.
    let run = crate::harness_conformance::suite(&fx.svc, "fake", &json!({}), &[]).await;
    assert!(
        run.findings
            .iter()
            .any(|f| f.message.contains("launch failed")),
        "{:#?}",
        run.findings
    );
}

/// The hook route waits on a harness's mapping and answer: a provider
/// too slow to answer reads as no call and the empty answer, within the
/// hook verbs' bound — the route's fail-open stance.
#[cfg(unix)]
#[tokio::test]
async fn a_slow_harness_provider_reads_as_none_on_the_hook_path() {
    use oxplow_domain::agent::observe::HookAnswer;
    let (fx, _ext) = running_harness("slow:3000").await;
    let h = fx.svc.harnesses.get("fake").unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        h.tool_use(&json!({ "tool_name": "Edit", "tool_input": { "file_path": "a" } }))
            .await,
        None
    );
    assert_eq!(h.render(&HookAnswer::Ack).await, json!({}));
    assert!(
        started.elapsed() < std::time::Duration::from_millis(2_900),
        "bounded: {:?}",
        started.elapsed()
    );
}

/// A launch's input for the fixture's thread, programs looked for in
/// `search_path`.
fn launch_input(
    fx: &ThreadFixture,
    search_path: Vec<PathBuf>,
) -> oxplow_domain::agent::harness::LaunchInput {
    use oxplow_domain::agent::harness::{Endpoints, LaunchInput, SessionIds};
    LaunchInput {
        session: SessionIds {
            stream: oxplow_domain::StreamId::new(1),
            thread: fx.thread,
            session: oxplow_domain::AgentSessionId::new(1),
        },
        workspace: fx.svc.layout.project_dir.clone(),
        project_dir: fx.svc.layout.project_dir.clone(),
        endpoints: Endpoints {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "secret-bearer".into(),
        },
        identity_env: vec![("OXPLOW_HOOK_TOKEN".into(), "secret-bearer".into())],
        system_prompt: None,
        resume: None,
        fork: false,
        text: Default::default(),
        config: json!({}),
        home: None,
        search_path,
    }
}

/// A project extension `shutter` whose provider `shutter` is the fake as a
/// snapshots provider, keeping contents when `contents`.
#[cfg(unix)]
fn write_snapshots_extension(project: &Path, contents: bool) {
    let dir = project.join(".oxplow/extensions/shutter");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: shutter\nintent:\n  purpose: the fake snapshots provider\n  examples: [{ name: a }]\npermissions: [process.run]\nproviders:\n  - id: shutter\n    capability: snapshots\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    let features = if contents { "contents" } else { "" };
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=snapshots OXPLOW_FAKE_FEATURES='{features}' exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::snapshots_declarations(contents))
            .unwrap(),
    )
    .unwrap();
}

/// The fixture with the snapshots extension approved, its instance
/// running and chosen as the project's snapshots, and the primary
/// stream's capture service in place (its worktree is what the provider
/// marks).
#[cfg(unix)]
async fn running_snapshots(
    contents: bool,
) -> (
    ThreadFixture,
    oxplow_domain::StreamId,
    std::sync::Arc<crate::snapshot_capture::SnapshotCaptureService>,
) {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_snapshots_extension(&project, contents);
    let ext = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == "shutter")
        .expect("the snapshots extension loads");
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    deploy(&fx, &ext).await;
    let stream = oxplow_domain::StreamId::new(1);
    let capture = std::sync::Arc::new(
        crate::snapshot_capture::SnapshotCaptureService::new(
            fx.svc.snapshot_store.clone(),
            fx.svc.blobs.clone(),
            project.clone(),
            std::sync::Arc::new(crate::vcs::GitProvider),
            stream,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO),
    );
    fx.svc
        .snapshot_captures
        .insert_for_test(stream, capture.clone());
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    choose_snapshots(&fx, "shutter").await;
    (fx, stream, capture)
}

/// `id` as the project's snapshots, its switch applied.
async fn choose_snapshots(fx: &ThreadFixture, id: &str) {
    fx.svc
        .config
        .write()
        .unwrap()
        .active_providers
        .insert(crate::snapshots::CAPABILITY.into(), id.into());
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    fx.svc
        .event_pump
        .settle(&["snapshots.switch"], std::time::Duration::from_secs(10))
        .await;
}

async fn mark_turn(
    fx: &ThreadFixture,
    stream: oxplow_domain::StreamId,
) -> oxplow_domain::snapshot::Marked {
    let mut request = oxplow_domain::snapshot::MarkRequest::new(
        stream,
        oxplow_domain::snapshot::SnapshotTrigger::TurnEnd,
    );
    request.budget = Some(std::time::Duration::from_secs(30));
    fx.svc
        .snapshots
        .active()
        .expect("an active implementation")
        .mark(&request)
        .await
        .unwrap()
        .expect("a snapshot")
}

/// A provider process as the project's snapshots: it marks the worktree,
/// core records what it reports (its handle and budget on the op) and
/// keeps the bytes it ships, an unchanged mark returns the snapshot it
/// already has, and an edit is a change. Core's own capture idles
/// meanwhile, and resumes when core's implementation is chosen again.
#[cfg(unix)]
#[tokio::test]
async fn a_snapshots_provider_marks_and_core_records_and_reads_it() {
    let (fx, stream, capture) = running_snapshots(true).await;
    assert!(capture.is_idle(), "core's capture idles");
    let project = fx.svc.layout.project_dir.clone();
    std::fs::write(project.join("a.txt"), "alpha").unwrap();

    let first = mark_turn(&fx, stream).await;
    let op = fx
        .svc
        .snapshot_store
        .list_ops(stream, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(op.snapshot_id, first.snapshot);
    assert_eq!(
        (op.provider.as_deref(), op.contents, op.budget_ms),
        (Some("shutter"), true, Some(30_000))
    );
    assert!(op.handle.is_some(), "{op:?}");
    let active = fx.svc.snapshots.active().unwrap();
    assert_eq!(
        active.read_at(first.snapshot, "a.txt").await.unwrap(),
        b"alpha"
    );

    let again = mark_turn(&fx, stream).await;
    assert!(again.unchanged);
    assert_eq!(again.snapshot, first.snapshot);

    std::fs::write(project.join("a.txt"), "beta").unwrap();
    let edited = mark_turn(&fx, stream).await;
    assert_ne!(edited.snapshot, first.snapshot);
    let changed = active
        .changed(stream, Some(first.snapshot), edited.snapshot)
        .await
        .unwrap();
    assert_eq!(
        changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["a.txt"]
    );
    assert_eq!(
        active.read_at(edited.snapshot, "a.txt").await.unwrap(),
        b"beta"
    );

    // Core's capture takes nothing of its own while the provider marks.
    capture.mark_dirty(
        project.join("a.txt"),
        oxplow_fs_watch::WatchEventKind::Other,
    );
    assert_eq!(
        capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Quiet)
            .await
            .unwrap(),
        None
    );

    choose_snapshots(&fx, "oxplow").await;
    assert!(!capture.is_idle(), "core's capture resumes");
}

/// Without `contents` a provider's snapshots record what changed and no
/// bytes: a read says the snapshot was taken without contents, by it.
#[cfg(unix)]
#[tokio::test]
async fn a_snapshots_provider_without_contents_records_identities_only() {
    let (fx, stream, _capture) = running_snapshots(false).await;
    std::fs::write(fx.svc.layout.project_dir.join("a.txt"), "alpha").unwrap();
    let marked = mark_turn(&fx, stream).await;
    let active = fx.svc.snapshots.active().unwrap();
    assert!(!active.contents());
    match active.read_at(marked.snapshot, "a.txt").await {
        Err(oxplow_domain::snapshot::SnapshotError::NoContents { provider, .. }) => {
            assert_eq!(provider.as_deref(), Some("shutter"))
        }
        other => panic!("{other:?}"),
    }
    let rows = fx
        .svc
        .snapshot_store
        .list_files_for_snapshot(marked.snapshot)
        .await
        .unwrap();
    let row = rows
        .iter()
        .find(|r| r.path == "a.txt")
        .expect("a.txt recorded");
    assert!(
        !fx.svc.blobs.has(row.blob_hash.as_deref().unwrap()),
        "no bytes kept"
    );
}

/// A provider's `host/call` `sql.read` reads through the SQL gateway, as a
/// command's or an effect's script does: the metric functions answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_providers_host_read_reaches_the_metric_functions() {
    let fx = services_with_thread().await;
    let grant = oxplow_domain::grant::Grant::from_permissions(
        "tracker",
        &oxplow_domain::permissions::Permissions {
            sql: Some(oxplow_domain::permissions::SqlAccess::Read),
            ..Default::default()
        },
    );
    let calls = host::HostCalls::new(vec!["sql.read".into()], grant, fx.svc.sql.clone(), None);
    let run =
        oxplow_domain::run::RunContext::core(Actor::Human, oxplow_domain::run::Mode::External);
    let _serving = calls.register("k", run).unwrap();
    let read = calls
        .call(json!({
            "key": "k",
            "scope": "sql.read",
            "args": { "sql": "SELECT count(*) AS n FROM (SELECT capture_id, MEASURE('oxplow.coverage.abs_pct') AS c FROM metric_grid('capture'))" },
        }))
        .await
        .unwrap();
    assert_eq!(read, json!([{ "n": 0 }]));
}

/// A project extension `shelf` (or `name`) whose provider `store` is the
/// fake as a filesystem provider, holding `files` (path → content) and
/// taking writes when `writable`.
#[cfg(unix)]
fn write_filesystem_extension(
    project: &Path,
    name: &str,
    files: &[(String, Vec<u8>)],
    writable: bool,
) {
    use base64::Engine as _;
    let dir = project.join(".oxplow/extensions").join(name);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        format!("manifest: 2\nname: {name}\nintent:\n  purpose: the fake filesystem provider\n  examples: [{{ name: a }}]\npermissions: [process.run]\nproviders:\n  - id: store\n    capability: filesystem\n    entry: bin/provider\n    declarations: provider.json\n"),
    )
    .unwrap();
    let seeded: serde_json::Map<String, serde_json::Value> = files
        .iter()
        .map(|(p, c)| {
            (
                p.clone(),
                json!(base64::engine::general_purpose::STANDARD.encode(c)),
            )
        })
        .collect();
    let script = dir.join("bin/provider");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=filesystem OXPLOW_FAKE_FEATURES='{}' OXPLOW_FAKE_FILES='{}' exec '{}' \"$@\"\n",
            if writable { "writable" } else { "" },
            serde_json::Value::Object(seeded),
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::filesystem_declarations(writable))
            .unwrap(),
    )
    .unwrap();
}

/// The fixture with the filesystem extension `name` approved and its
/// instance `store` running.
#[cfg(unix)]
async fn running_filesystem(name: &str, writable: bool) -> ThreadFixture {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_filesystem_extension(
        &project,
        name,
        &crate::filesystem_conformance::fixture(),
        writable,
    );
    let ext = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == name)
        .expect("the filesystem extension loads");
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    deploy(&fx, &ext).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    fx
}

/// A filesystem provider's tree passes the filesystem conformance checks
/// as `oxplow:local` does, through the host.
#[cfg(unix)]
#[tokio::test]
async fn a_filesystem_provider_passes_the_filesystem_conformance_checks() {
    use crate::filesystem_conformance as c;
    let fx = running_filesystem("shelf", true).await;
    let fs = fx.svc.filesystems.resolve(Some("shelf:store")).unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    assert_eq!(fs.id(), "shelf:store");
    assert!(fs.has_feature("writable"));
    for rev in [None, Some("v1")] {
        c::stat_describes_what_read_returns(&*fs, &cx, rev).await;
        c::read_round_trips_and_ranges_slice(&*fs, &cx, rev).await;
        c::lists_one_level_or_the_whole_tree(&*fs, &cx, rev).await;
        c::a_path_never_leaves_the_filesystem(&*fs, &cx, rev).await;
    }
    c::writes_only_when_writable(&*fs, &cx, None).await;
    // A revision it doesn't have is refused as invalid.
    let e = fs
        .read(&cx, Some("v9"), &format!("{}/a.txt", c::ROOT), None)
        .await
        .unwrap_err();
    assert!(
        matches!(e, oxplow_domain::filesystem::FsError::Invalid(_)),
        "{e:?}"
    );
    // The same tree through the layers a reader uses.
    assert!(fx
        .svc
        .filesystems
        .ids()
        .contains(&"shelf:store".to_string()));
}

/// A provider that doesn't say `writable` is read-only, and its verbs
/// carry no `write`.
#[cfg(unix)]
#[tokio::test]
async fn a_read_only_filesystem_provider_refuses_writes() {
    use crate::filesystem_conformance as c;
    let fx = running_filesystem("shelf", false).await;
    let fs = fx.svc.filesystems.resolve(Some("shelf:store")).unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    assert!(!fs.has_feature("writable"));
    c::stat_describes_what_read_returns(&*fs, &cx, None).await;
    c::read_round_trips_and_ranges_slice(&*fs, &cx, None).await;
    c::lists_one_level_or_the_whole_tree(&*fs, &cx, None).await;
    c::a_path_never_leaves_the_filesystem(&*fs, &cx, None).await;
    c::writes_only_when_writable(&*fs, &cx, None).await;
}

/// A process that is gone makes the filesystem unavailable, not wrong:
/// the reader is told so, and the next call may succeed.
#[cfg(unix)]
#[tokio::test]
async fn a_filesystem_provider_whose_process_dies_is_unavailable() {
    let fx = running_filesystem("shelf", false).await;
    let fs = fx.svc.filesystems.resolve(Some("shelf:store")).unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    let path = format!("{}/a.txt", crate::filesystem_conformance::ROOT);
    assert!(fs.read(&cx, None, &path, None).await.unwrap().is_some());
    let instance = fx.svc.providers.get("shelf/store").await.expect("running");
    instance.hook("crash").await;
    let e = fs.read(&cx, None, &path, None).await.unwrap_err();
    assert!(
        matches!(&e, oxplow_domain::filesystem::FsError::Unavailable { filesystem, .. } if filesystem == "shelf:store"),
        "{e:?}"
    );
}

/// Turning the instance off takes its filesystem out of the registry: a
/// reader is refused with the registry's "no filesystem" message.
#[cfg(unix)]
#[tokio::test]
async fn unregistering_a_filesystem_provider_removes_its_filesystem() {
    let fx = running_filesystem("shelf", false).await;
    assert!(fx.svc.filesystems.has("shelf:store"));
    turn_off(&fx.svc, "shelf/store").await;
    assert!(!fx.svc.filesystems.has("shelf:store"));
    let e = fx
        .svc
        .filesystems
        .resolve(Some("shelf:store"))
        .err()
        .unwrap();
    assert!(e.to_string().contains("no filesystem `shelf:store`"), "{e}");
    // Core's own stay.
    assert!(fx.svc.filesystems.has("oxplow:local"));
}

/// An extension can't take oxplow's namespace: `oxplow:` ids are core's.
#[cfg(unix)]
#[tokio::test]
async fn a_filesystem_provider_cannot_take_the_oxplow_namespace() {
    let fx = running_filesystem("oxplow", false).await;
    assert!(!fx.svc.filesystems.has("oxplow:store"));
    assert_eq!(
        fx.svc
            .filesystems
            .ids()
            .into_iter()
            .filter(|id| id.starts_with("oxplow:"))
            .collect::<Vec<_>>(),
        ["oxplow:extensions", "oxplow:local", "oxplow:temp"]
    );
}
