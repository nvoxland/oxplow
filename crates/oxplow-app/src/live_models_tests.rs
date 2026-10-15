//! The live models: `v_ai_role`, `v_ai_provider`, `v_lsp_server`,
//! `v_collector_state` and `v_collector_credential` read state that has no
//! change hook (a keychain, a yaml file, a process table), so the gateway
//! reads it when a query names them (`crate::live_models`).

use serde_json::{json, Value};

use crate::test_fixtures::{services_with_effort, EffortFixture};

async fn rows(fx: &EffortFixture, sql: &str) -> Vec<Vec<Value>> {
    let out = fx.svc.sql.query_sql(sql, vec![], None).await.unwrap();
    serde_json::to_value(&out.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().clone())
        .collect()
}

fn provider(id: &str) -> oxplow_ai::config::ProviderConfig {
    oxplow_ai::config::ProviderConfig {
        id: id.into(),
        kind: "typesafe".into(),
        base_url: None,
    }
}

/// A role's binding and whether its provider has a key are read when
/// asked: a binding saved or a key stored after the last query shows in the
/// next one, though no event announced it, and the key itself is nowhere.
#[tokio::test]
async fn ai_roles_and_providers_are_read_live_and_never_show_a_key() {
    let fx = services_with_effort().await;
    let roles = "SELECT role, provider, model, key_set FROM v_ai_role WHERE role IN ('main', 'decide') ORDER BY role";
    assert_eq!(
        rows(&fx, roles).await,
        vec![
            vec![json!("decide"), Value::Null, Value::Null, json!(0)],
            vec![json!("main"), Value::Null, Value::Null, json!(0)],
        ]
    );

    // An active extension's AI profile is a role too, after the built-ins.
    assert_eq!(
        rows(
            &fx,
            "SELECT role, extension, fallback FROM v_ai_role WHERE extension IS NOT NULL"
        )
        .await,
        vec![vec![
            json!("oxplow-bundled/decisions"),
            json!("oxplow-bundled"),
            json!("main")
        ]]
    );

    fx.svc
        .ai
        .save_provider(provider("ts"), Some("sk-very-secret".into()))
        .unwrap();
    fx.svc
        .ai
        .set_role(
            &oxplow_ai::config::Role::DECIDE,
            Some(oxplow_ai::config::RoleBinding::new("ts", "jev")),
        )
        .unwrap();
    assert_eq!(
        rows(&fx, roles).await,
        vec![
            vec![json!("decide"), json!("ts"), json!("jev"), json!(1)],
            vec![json!("main"), Value::Null, Value::Null, json!(0)],
        ]
    );
    assert_eq!(
        rows(&fx, "SELECT id, kind, key_set FROM v_ai_provider").await,
        vec![vec![json!("ts"), json!("typesafe"), json!(1)]]
    );
    let everything = format!(
        "{:?}{:?}",
        rows(&fx, "SELECT * FROM v_ai_role").await,
        rows(&fx, "SELECT * FROM v_ai_provider").await
    );
    assert!(!everything.contains("sk-very-secret"), "{everything}");

    // Remove the key from the keychain behind the service's back.
    fx.svc.secrets.delete("ts").unwrap();
    assert_eq!(
        rows(&fx, "SELECT key_set FROM v_ai_provider").await,
        vec![vec![json!(0)]]
    );
}

/// A configured language server is a row, with whether its binary exists.
#[tokio::test]
async fn language_servers_are_read_live() {
    let fx = services_with_effort().await;
    assert!(rows(&fx, "SELECT language_id FROM v_lsp_server")
        .await
        .is_empty());
    fx.svc
        .config
        .write()
        .unwrap()
        .lsp_servers
        .push(oxplow_config::LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "/definitely/not/a/binary".into(),
            args: vec!["--stdio".into()],
        });
    assert_eq!(
        rows(
            &fx,
            "SELECT language_id, extensions, command, args, source, binary_exists, running
             FROM v_lsp_server"
        )
        .await,
        vec![vec![
            json!("rust"),
            json!(r#"["rs"]"#),
            json!("/definitely/not/a/binary"),
            json!(r#"["--stdio"]"#),
            json!("yaml"),
            json!(0),
            json!(0),
        ]]
    );
}

const MANIFEST: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Pull.\ncollectors:\n  - id: pull\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: item, key: id, columns: { id: int } }\n";

/// A collector's approval and its credentials' state (set or not, never the
/// value) are read live.
#[tokio::test]
async fn collector_approval_and_credentials_are_read_live() {
    let fx = services_with_effort().await;
    let dir = fx.svc.layout.project_dir.join("oxplow/extensions/acme");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.yaml"), MANIFEST).unwrap();
    std::fs::write(dir.join("sync.sh"), "#!/bin/sh\necho '[]'\n").unwrap();
    crate::test_fixtures::activate(&fx.svc).await;

    let state = "SELECT ref, owner, id, approved FROM v_collector_state WHERE owner = 'acme'";
    assert_eq!(
        rows(&fx, state).await,
        vec![vec![
            json!("collector:acme/pull"),
            json!("acme"),
            json!("pull"),
            json!(0)
        ]]
    );
    let credentials = "SELECT collector, name, is_set FROM v_collector_credential WHERE collector = 'collector:acme/pull'";
    assert_eq!(
        rows(&fx, credentials).await,
        vec![vec![json!("collector:acme/pull"), json!("TOKEN"), json!(0)]]
    );

    let root = fx.svc.layout.project_dir.clone();
    crate::collector_runner::set_credential(
        &crate::collector_runner::Collectors::of(&fx.svc, &root),
        "acme",
        "TOKEN",
        Some("tok-very-secret"),
    )
    .unwrap();
    assert_eq!(
        rows(&fx, credentials).await,
        vec![vec![json!("collector:acme/pull"), json!("TOKEN"), json!(1)]]
    );
    let everything = format!(
        "{:?}",
        rows(&fx, "SELECT * FROM v_collector_credential").await
    );
    assert!(!everything.contains("tok-very-secret"), "{everything}");
}

/// A live model's view is an empty, typed stub; a gateway with nothing
/// bound to read it refuses rather than answering empty, and no model is
/// built on one (it would read the stub).
#[tokio::test]
async fn a_live_model_is_never_answered_empty_by_mistake() {
    let fx = services_with_effort().await;
    let bare = crate::sql_gateway::SqlGateway::new(fx.svc.db.clone());
    let err = bare
        .query_sql("SELECT role FROM v_ai_role", vec![], None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("v_ai_role"), "{err}");
    assert!(
        bare.check("SELECT role FROM v_ai_role").await.is_ok(),
        "a check compiles against the stub"
    );

    let mut sources = oxplow_db::models::core_sources().unwrap();
    sources.retain(|s| !crate::live_models::NAMES.contains(&s.decl.name.as_str()));
    for source in &sources {
        for name in crate::live_models::NAMES {
            assert!(
                !source.sql.contains(&format!("v_{name}")),
                "{} reads the live model {name}",
                source.decl.name
            );
        }
    }
}

/// The columns a live read returns are the columns its model declares.
#[test]
fn every_live_model_reads_the_columns_it_declares() {
    let sources = oxplow_db::models::core_sources().unwrap();
    for name in crate::live_models::NAMES {
        let decl = &sources
            .iter()
            .find(|s| s.decl.name == *name)
            .unwrap_or_else(|| panic!("no core model `{name}`"))
            .decl;
        let declared: Vec<&str> = decl.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(declared, crate::live_models::columns(name), "{name}");
    }
}
