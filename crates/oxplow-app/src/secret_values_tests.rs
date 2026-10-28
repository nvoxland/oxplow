//! A secret is a kind of value (`writeOnly` in a command's input schema):
//! whoever sets it — a person, their agent, a page for either — it is
//! written and never read back. No published dataset, no audit row, no
//! event and no result carries it (`.context/commands.md` "Secrets").

use oxplow_domain::Actor;
use serde_json::json;

use crate::test_fixtures::services_with_effort;

const SECRET: &str = "sk-oxplow-secret-value-7731";

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_set_by_anyone_is_never_read_back() {
    let fx = services_with_effort().await;
    let svc = &fx.svc;
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    let page = Actor::Page {
        page_id: "oxplow-foundation/ai-providers".into(),
        on_behalf_of: Box::new(agent.clone()),
    };
    let mut results = Vec::new();
    for (i, actor) in [Actor::Human, agent, page].iter().enumerate() {
        let out = svc
            .commands
            .run(
                actor,
                crate::commands::ai_settings::SET_PROVIDER,
                json!({ "id": format!("p{i}"), "kind": "openrouter", "key": SECRET }),
                false,
            )
            .await
            .unwrap_or_else(|e| panic!("{actor:?}: {e:?}"));
        results.push(serde_json::to_string(&out.result).unwrap());
    }
    // It was stored: the provider says it has a key.
    assert!(svc
        .ai
        .settings()
        .await
        .unwrap()
        .providers
        .iter()
        .all(|p| p.key_set));

    leak_sweep(svc, &results, SECRET).await;
}

/// No result, audit row, event or published dataset carries `secret`.
async fn leak_sweep(svc: &crate::Services, results: &[String], secret: &str) {
    let leaks = |what: &str, text: &str| {
        assert!(!text.contains(secret), "{what} carries the secret: {text}")
    };
    for r in results {
        leaks("a result", r);
    }
    let audit = svc.commands.audit_store().list_recent(1_000).await.unwrap();
    leaks("the audit", &format!("{audit:?}"));
    let events = svc.event_log_store.read_after(0, 100_000).await.unwrap();
    leaks("the event log", &format!("{events:?}"));

    // Every published dataset, read whole.
    let datasets = svc
        .sql
        .query_sql("SELECT name FROM v_dataset", vec![], None)
        .await
        .unwrap();
    let names: Vec<String> = serde_json::to_value(&datasets.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r[0].as_str().map(str::to_string))
        .collect();
    assert!(names.len() > 20, "{names:?}");
    for name in names {
        // A dataset that needs params or a source this fixture lacks reads as
        // an error, which carries no rows either.
        if let Ok(out) = svc
            .sql
            .query_sql(&format!("SELECT * FROM {name}"), vec![], Some(100_000))
            .await
        {
            leaks(&name, &serde_json::to_string(&out.rows).unwrap());
        }
    }
}

/// A collector's credential and a provider's host secret are set by a
/// person, an agent and a page for either; the value is in none of the
/// places the AI key is not.
#[tokio::test(flavor = "multi_thread")]
async fn a_credential_or_secret_set_by_anyone_is_never_read_back() {
    const VALUE: &str = "ghp-collector-credential-5521";
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/acme");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: [process.run, secrets:TOKEN]\ncollectors:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row, key: id, columns: { id: int } }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("sync.sh"),
        "#!/bin/sh\necho '{\"entities\":{\"row\":[]}}'\n",
    )
    .unwrap();
    let fx = crate::test_fixtures::services_with_effort_in(dir).await;
    crate::test_fixtures::activate(&fx.svc).await;
    let svc = &fx.svc;
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    let page = Actor::Page {
        page_id: "oxplow-foundation/extensions-credentials".into(),
        on_behalf_of: Box::new(agent.clone()),
    };
    let mut results = Vec::new();
    for actor in [Actor::Human, agent, page] {
        let out = svc
            .commands
            .run(
                &actor,
                crate::commands::credentials::SET_CREDENTIAL,
                json!({ "collector": "acme/s", "name": "TOKEN", "value": VALUE }),
                false,
            )
            .await
            .unwrap_or_else(|e| panic!("{actor:?}: {e:?}"));
        results.push(serde_json::to_string(&out.result).unwrap());
    }
    leak_sweep(svc, &results, VALUE).await;
}
