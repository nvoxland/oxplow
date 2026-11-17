//! Shared settings projections preserve extension host ownership.
use crate::extension::extension_configuration::ConfigurationHost;
use crate::extension::extension_private_state_tests::{declare, reconcile, run, CONFIGURATION};
use crate::test_fixtures::{services_with_thread, ThreadFixture};
use oxplow_domain::run::{Authority, ContributionRef, Mode, RunContext};
use oxplow_domain::Actor;
use serde_json::json;
use std::sync::Arc;
fn host(fx: &ThreadFixture) -> ConfigurationHost {
    ConfigurationHost::new(
        fx.svc.extension_catalog.clone(),
        fx.svc.layout.project_dir.clone(),
    )
    .with_settings(fx.svc.global_config_dir.clone(), fx.svc.secrets.clone())
}

fn owner(fx: &ThreadFixture, generation: Option<String>) -> ContributionRef {
    let root = &fx.svc.layout.project_dir;
    let snapshot = fx.svc.extension_catalog.live_snapshot(root);
    let ext = snapshot
        .extensions()
        .iter()
        .find(|ext| ext.name == "alpha")
        .expect("alpha is loaded");
    let id = ext
        .contributions
        .iter()
        .find(|item| {
            item.id.kind == oxplow_domain::contribution::ContributionKind::Command
                && item.id.local.ends_with("config.read")
        })
        .expect("alpha declares config.read")
        .id
        .clone();
    let mut owner = ContributionRef {
        id,
        extension: ext.name.clone(),
        version: ext.version.clone(),
        generation: Some(snapshot.generation().to_string()),
        deployment: ext.deployment.hash().map(str::to_owned),
    };
    if generation.is_some() {
        owner.generation = generation;
    }
    owner
}

fn declaring(scope: &str, run: RunContext) -> RunContext {
    run.authorized(Authority {
        declared: oxplow_domain::grant::GrantRequest {
            scopes: [scope.to_string()].into(),
            ..Default::default()
        },
        grant: oxplow_domain::grant::Grant::none(),
        enforcement: crate::grants::enforcement(),
    })
}

async fn denied_snapshot(fx: &ThreadFixture, run: RunContext) -> String {
    let host = host(fx);
    let refused = fx
        .svc
        .db
        .read_or(move |_conn| {
            host.snapshot(&run)
                .map(|_| ())
                .map_err(oxplow_db::TxError::Aborted)
        })
        .await
        .expect_err("the host refuses this run");
    match refused {
        oxplow_db::TxError::Aborted(error) => error.to_string(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_refuses_runs_that_do_not_own_the_service() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let read = || {
        declaring(
            "extension_config.read",
            RunContext::contribution(owner(&fx, None), Actor::Human, Mode::External),
        )
    };

    // The hand-built run itself is good: it reads.
    let host_ok = host(&fx);
    let ok = read();
    fx.svc
        .db
        .read_or(move |_conn| host_ok.snapshot(&ok).map_err(oxplow_db::TxError::Aborted))
        .await
        .expect("a declared, current run reads");

    // A generation the catalog has moved past.
    let stale = declaring(
        "extension_config.read",
        RunContext::contribution(
            owner(&fx, Some("a-generation-long-gone".into())),
            Actor::Human,
            Mode::External,
        ),
    );
    assert!(denied_snapshot(&fx, stale).await.contains("denied"));

    // A grant revoked while the run was under way.
    let revoked = read().guarded(Arc::new(|| Err("approval revoked".into())));
    assert!(denied_snapshot(&fx, revoked).await.contains("denied"));

    // No declaration of the scope, and core's own run (no owner).
    let undeclared = declaring(
        "state.read",
        RunContext::contribution(owner(&fx, None), Actor::Human, Mode::External),
    );
    assert!(denied_snapshot(&fx, undeclared).await.contains("denied"));
    let core = RunContext::core(Actor::Human, Mode::External);
    assert!(denied_snapshot(&fx, core).await.contains("denied"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_reads_shared_settings_with_atomic_precedence() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let extensions = fx
        .svc
        .extension_catalog
        .live_snapshot(&fx.svc.layout.project_dir);
    let store = crate::settings::store(
        &fx.svc.layout.project_dir,
        &fx.svc.global_config_dir,
        fx.svc.secrets.clone(),
        extensions.extensions(),
    )
    .unwrap();
    store
        .set(
            "alpha.limit",
            json!(12),
            oxplow_config::settings::Level::Project,
        )
        .unwrap();
    let peeked = run(&fx, "alpha.config.peek", json!({})).await.unwrap();
    assert_eq!(peeked["result"]["values"]["limit"], json!(12));
    assert_eq!(peeked["result"]["origins"]["/limit"], json!("project"));
    store
        .set(
            "alpha.limit",
            json!(17),
            oxplow_config::settings::Level::Local,
        )
        .unwrap();
    let peeked = run(&fx, "alpha.config.peek", json!({})).await.unwrap();
    assert_eq!(peeked["result"]["values"]["limit"], json!(17));
    assert_eq!(peeked["result"]["origins"]["/limit"], json!("local"));
    assert!(peeked["result"].get("layers").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_reads_a_registered_key_and_validates_its_request() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let read = run(&fx, "alpha.config.get", json!({"key":"alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(read["result"]["value"], json!(10));
    assert_eq!(read["result"]["origin"], json!("default"));
    assert!(run(&fx, "alpha.config.get", json!({"key":"beta.limit"}))
        .await
        .is_err());
    assert!(run(
        &fx,
        "alpha.config.get",
        json!({"key":"alpha.limit", "level":"personal"})
    )
    .await
    .is_err());
    assert!(run(
        &fx,
        "alpha.config.get",
        json!({"key":"alpha.limit", "extra":true})
    )
    .await
    .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn extension_reads_use_the_services_global_directory_and_fake_secrets() {
    let fx = services_with_thread().await;
    let declaration = CONFIGURATION.replace(
        "      label: { type: string, default: default }",
        "      token: { type: string, x-secret: true }\n      label: { type: string, default: default }",
    );
    declare(fx._dir.path(), "alpha", &declaration);
    reconcile(&fx).await;
    for (key, value) in [
        ("alpha.limit", json!(24)),
        ("alpha.token", json!("fake-backend-secret")),
    ] {
        run(
            &fx,
            "oxplow.config.set",
            json!({"key":key,"value":value,"level":"global"}),
        )
        .await
        .unwrap();
    }
    let read = run(&fx, "alpha.config.get", json!({"key":"alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(read["result"]["value"], json!(24));
    assert_eq!(read["result"]["origin"], json!("global"));
    let token = run(&fx, "alpha.config.get", json!({"key":"alpha.token"}))
        .await
        .unwrap();
    assert_eq!(token["result"]["value"], json!("fake-backend-secret"));
    let snapshot = run(&fx, "alpha.config.peek", json!({})).await.unwrap();
    assert_eq!(snapshot["result"]["values"]["limit"], json!(24));
    assert_eq!(
        snapshot["result"]["values"]["token"],
        json!("fake-backend-secret")
    );
    let catalog = fx
        .svc
        .extension_catalog
        .live_snapshot(&fx.svc.layout.project_dir);
    let ext = catalog
        .extensions()
        .iter()
        .find(|ext| ext.name == "alpha")
        .unwrap();
    assert_eq!(
        fx.svc
            .sql
            .with_extension_config(ext, "SELECT config('/limit')")
            .await
            .unwrap(),
        "SELECT 24"
    );
    assert!(fx
        .svc
        .sql
        .with_extension_config(ext, "SELECT config('/token')")
        .await
        .is_err());
    let values = crate::extension::extension_configuration::effective_values(
        &fx.svc.layout.project_dir,
        &fx.svc.global_config_dir,
        "alpha",
        ext.configuration.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(values["limit"], json!(24));
    assert!(values.get("token").is_none());
    assert!(
        !std::fs::read_to_string(fx.svc.global_config_dir.join("global.yaml"))
            .unwrap()
            .contains("fake-backend-secret")
    );
    assert!(!format!(
        "{:?}",
        fx.svc
            .commands
            .audit_store()
            .list_recent(1000)
            .await
            .unwrap()
    )
    .contains("fake-backend-secret"));
}
