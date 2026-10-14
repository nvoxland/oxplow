//! Extension configuration end to end: schema migrations at activation, the
//! stale-layer contract, and the hardening of the three native operations
//! and the hosts that serve them.

use std::sync::Arc;

use oxplow_db::extension_config_store::{ConfigNamespace, SqliteExtensionConfigStore};
use oxplow_domain::extension_state::Scope;
use oxplow_domain::run::{Authority, ContributionRef, Mode, RunContext};
use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use crate::extension_configuration::ConfigurationHost;
use crate::extension_private_state_tests::{declare, reconcile, run, write, CONFIGURATION};
use crate::test_fixtures::{services_with_effort, EffortFixture};

fn ext_file(fx: &EffortFixture, rel: &str, body: &str) {
    write(
        fx._dir.path(),
        &format!("oxplow/extensions/alpha/{rel}"),
        body,
    );
}

/// `CONFIGURATION` at version 2: `limit` was `max` at version 1.
const V2: &str = "configuration:
  version: 2
  schema:
    type: object
    required: [limit]
    additionalProperties: false
    properties:
      limit: { type: integer, minimum: 1, maximum: 100 }
      label: { type: string }
  defaults: { limit: 10 }
  migrations:
    - { from: 1, to: 2, entry: config/migrate_1_2.star }
";

const RENAME_MAX: &str = "def transform(values):
    out = dict(values)
    if \"max\" in out:
        out[\"limit\"] = out.pop(\"max\")
    return out
";

async fn store_layer(fx: &EffortFixture, scope: Scope, version: u32, values: Value) {
    let namespace = ConfigNamespace::new("alpha".into(), scope).unwrap();
    SqliteExtensionConfigStore::new(fx.svc.db.clone())
        .compare_exchange(namespace, None, version, values)
        .await
        .unwrap();
}

async fn stamp_of(fx: &EffortFixture, which: &str) -> Value {
    run(fx, "alpha.config.inspect", json!({})).await.unwrap()[which].clone()
}

/// A project at configuration version 1 with both layers stored in the
/// old shape, then `alpha` redeclared at version 2.
async fn old_layers_under_v2(script: &str, version_two: &str) -> EffortFixture {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    store_layer(&fx, Scope::Project, 1, json!({"max": 20})).await;
    store_layer(&fx, Scope::Personal, 1, json!({"max": 30, "label": "mine"})).await;
    ext_file(&fx, "config/migrate_1_2.star", script);
    declare(fx._dir.path(), "alpha", version_two);
    ext_file(&fx, "config/migrate_1_2.star", script);
    fx
}

// ---- migrations -------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn activation_carries_every_old_layer_to_the_declared_version() {
    let fx = old_layers_under_v2(RENAME_MAX, V2).await;
    reconcile(&fx).await;

    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"], json!({"limit": 30, "label": "mine"}));
    assert_eq!(config["origins"]["/limit"], json!("personal"));
    // Both layers moved in the one activation, each a revision on.
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 2, "revision": 2})
    );
    assert_eq!(
        stamp_of(&fx, "personal").await,
        json!({"schema_version": 2, "revision": 2})
    );
    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(
        inspected["needs_migration"],
        json!({"project": false, "personal": false})
    );
    // A second activation has nothing left to do.
    reconcile(&fx).await;
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 2, "revision": 2})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_step_leaves_the_layers_alone_and_holds_the_extension_back() {
    let fx = old_layers_under_v2("def transform(values):\n    fail(\"cannot cope\")\n", V2).await;
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let reason = format!("{problems:?}");
    assert!(
        reason.contains("alpha") && reason.contains("cannot cope") && reason.contains("1 -> 2"),
        "{reason}"
    );
    // Held like a broken model: the version that worked keeps serving, on
    // settings still at the version it reads.
    let config = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(config["schema_version"], json!(1));
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 1, "revision": 1})
    );
    assert_eq!(
        stamp_of(&fx, "personal").await,
        json!({"schema_version": 1, "revision": 1})
    );
    // Fixing the step lets the next activation migrate.
    ext_file(&fx, "config/migrate_1_2.star", RENAME_MAX);
    reconcile(&fx).await;
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 2, "revision": 2})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn migrated_settings_the_schema_refuses_hold_the_extension_back() {
    // The step forgets the rename, so `max` survives into a schema that
    // forbids it. Neither layer is written.
    let fx = old_layers_under_v2("def transform(values):\n    return values\n", V2).await;
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let reason = format!("{problems:?}");
    assert!(
        reason.contains("alpha") && reason.contains("satisf"),
        "{reason}"
    );
    assert!(
        !reason.contains("20") || !reason.contains("\"max\": 20"),
        "{reason}"
    );
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 1, "revision": 1})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_step_that_returns_a_non_object_holds_the_extension_back() {
    let fx = old_layers_under_v2("def transform(values):\n    return [1]\n", V2).await;
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(
        format!("{problems:?}").contains("must return an object"),
        "{problems:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chain_runs_each_step_from_the_stored_version() {
    let fx = services_with_effort().await;
    let v3 = "configuration:
  version: 3
  schema:
    type: object
    additionalProperties: false
    properties:
      limit: { type: integer }
      label: { type: string }
  migrations:
    - { from: 1, to: 2, entry: config/one.star }
    - { from: 2, to: 3, entry: config/two.star }
";
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    store_layer(&fx, Scope::Project, 1, json!({"a": 1})).await;
    store_layer(&fx, Scope::Personal, 2, json!({"b": 2})).await;
    declare(fx._dir.path(), "alpha", v3);
    ext_file(
        &fx,
        "config/one.star",
        "def transform(v):\n    return {\"limit\": v[\"a\"]}\n",
    );
    ext_file(
        &fx,
        "config/two.star",
        "def transform(v):\n    out = dict(v)\n    if \"b\" in out:\n        out[\"label\"] = str(out.pop(\"b\"))\n    return out\n",
    );
    reconcile(&fx).await;
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"], json!({"limit": 1, "label": "2"}));
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 3, "revision": 2})
    );
    assert_eq!(
        stamp_of(&fx, "personal").await,
        json!({"schema_version": 3, "revision": 2})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migration_declaration_that_cannot_chain_is_a_load_error() {
    let fx = services_with_effort().await;
    let bad = V2.replace("{ from: 1, to: 2,", "{ from: 1, to: 1,");
    declare(fx._dir.path(), "alpha", &bad);
    ext_file(&fx, "config/migrate_1_2.star", RENAME_MAX);
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let reason = format!("{problems:?}");
    assert!(reason.contains("migration_not_forward"), "{reason}");

    // A step whose script is missing, or doesn't define transform.
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", V2);
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(format!("{problems:?}").contains("isn't in the extension"));
    ext_file(&fx, "config/migrate_1_2.star", "x = 1\n");
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(
        format!("{problems:?}").contains("transform"),
        "{problems:?}"
    );
}

// ---- stale layers fail closed -----------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_layer_behind_the_declared_version_fails_closed_and_says_so() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", V2);
    ext_file(&fx, "config/migrate_1_2.star", RENAME_MAX);
    reconcile(&fx).await;
    // A row from before the schema moved (a restored backup, say).
    store_layer(&fx, Scope::Project, 1, json!({"max": 5})).await;

    let refused = run(&fx, "alpha.config.read", json!({}))
        .await
        .expect_err("a stale layer is not read");
    let CommandError::Invalid { field, message } = refused else {
        panic!("{refused:?}")
    };
    assert_eq!(field.as_deref(), Some("/schema_version"));
    assert!(message.contains("migration"), "{message}");

    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(
        inspected["needs_migration"],
        json!({"project": true, "personal": false})
    );
    // A write that leaves the old layer untouched is refused the same way.
    let untouched = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2,
               "personal": {"expected": null, "values": {"limit": 3}}}),
    )
    .await
    .expect_err("the stale project layer blocks it");
    let CommandError::Invalid { field, .. } = untouched else {
        panic!()
    };
    assert_eq!(field.as_deref(), Some("/schema_version"));

    // A stale stamp is a different failure, naming a different field.
    let conflict = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2,
               "project": {"expected": {"schema_version": 1, "revision": 9}, "values": {"limit": 3}}}),
    )
    .await
    .expect_err("a wrong stamp conflicts");
    let CommandError::Invalid { field, message } = conflict else {
        panic!()
    };
    assert_eq!(field.as_deref(), Some("/expected"));
    assert!(message.contains("changed"), "{message}");

    // Replacing the stale layer explicitly, at the new version, works.
    let stamps = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2,
               "project": {"expected": {"schema_version": 1, "revision": 1},
                           "values": {"limit": 5}}}),
    )
    .await
    .unwrap();
    assert_eq!(
        stamps["project"],
        json!({"schema_version": 2, "revision": 2})
    );
    assert_eq!(stamps["needs_migration"]["project"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_migration_writes_both_layers_in_one_commit() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", V2);
    ext_file(&fx, "config/migrate_1_2.star", RENAME_MAX);
    reconcile(&fx).await;
    store_layer(&fx, Scope::Project, 1, json!({"max": 5})).await;
    store_layer(&fx, Scope::Personal, 1, json!({"max": 6})).await;

    // One layer is invalid under the schema: neither layer is replaced.
    let refused = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2,
               "project": {"expected": {"schema_version": 1, "revision": 1},
                           "values": {"limit": 5}},
               "personal": {"expected": {"schema_version": 1, "revision": 1},
                            "values": {"limit": 5000}}}),
    )
    .await;
    assert!(matches!(refused, Err(CommandError::Invalid { .. })));
    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(inspected["project"]["schema_version"], json!(1));
    assert_eq!(inspected["personal"]["schema_version"], json!(1));

    let stamps = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2,
               "project": {"expected": {"schema_version": 1, "revision": 1},
                           "values": {"limit": 5}},
               "personal": {"expected": {"schema_version": 1, "revision": 1},
                            "values": {"limit": 6}}}),
    )
    .await
    .unwrap();
    assert_eq!(stamps["project"]["schema_version"], json!(2));
    assert_eq!(stamps["personal"]["schema_version"], json!(2));
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(6));
}

// ---- input hardening ---------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn every_operation_refuses_input_of_the_wrong_shape() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;

    for name in ["alpha.config.read", "alpha.config.inspect"] {
        for bad in [
            json!({"extension": "beta"}),
            json!([]),
            json!("x"),
            json!(7),
        ] {
            let refused = run(&fx, name, bad.clone()).await;
            assert!(
                matches!(refused, Err(CommandError::Invalid { .. })),
                "{name} {bad}: {refused:?}"
            );
        }
    }
    let layer = |values: Value| json!({"expected": null, "values": values});
    for bad in [
        json!({"schema_version": "1", "project": layer(json!({"limit": 5}))}),
        json!({"schema_version": -1, "project": layer(json!({"limit": 5}))}),
        json!({"schema_version": 1.5, "project": layer(json!({"limit": 5}))}),
        json!({"schema_version": 1}),
        json!({"project": layer(json!({"limit": 5}))}),
        json!({"schema_version": 1, "project": layer(json!({"limit": 5})), "extension": "beta"}),
        json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 5}, "extra": 1}}),
        json!({"schema_version": 1, "project": {"expected": "no", "values": {"limit": 5}}}),
        json!({"schema_version": 1, "project": {"expected": {"schema_version": 1}, "values": {"limit": 5}}}),
        json!({"schema_version": 1, "project": layer(json!([1, 2]))}),
        json!({"schema_version": 1, "project": layer(json!("limit"))}),
        json!({"schema_version": 1, "project": layer(json!({"limit": "5"}))}),
        json!({"schema_version": 1, "project": layer(json!({"nope": 5}))}),
        json!({"schema_version": 1, "project": null, "personal": null}),
        json!([]),
        json!(null),
    ] {
        let refused = run(&fx, "alpha.config.write", bad.clone()).await;
        assert!(
            matches!(refused, Err(CommandError::Invalid { .. })),
            "{bad}: {refused:?}"
        );
    }
    // None of it stored anything.
    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(inspected["project"], Value::Null);
    assert_eq!(inspected["personal"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_or_too_deep_value_is_refused_over_the_bus() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let huge = "x".repeat(2 * 1024 * 1024);
    let mut deep = json!(1);
    for _ in 0..200 {
        deep = json!({ "nested": deep });
    }
    for values in [json!({"limit": 5, "label": huge}), deep] {
        let refused = run(
            &fx,
            "alpha.config.write",
            json!({"schema_version": 1, "project": {"expected": null, "values": values}}),
        )
        .await
        .expect_err("the value is too big");
        let message = format!("{refused:?}");
        assert!(matches!(refused, CommandError::Invalid { .. }), "{message}");
        assert!(message.len() < 2000, "the error does not echo the value");
    }
    assert_eq!(stamp_of(&fx, "project").await, Value::Null);
}

// ---- races --------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_writers_to_one_layer_win_exactly_once() {
    let fx = Arc::new(services_with_effort().await);
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let mut tasks = Vec::new();
    for n in 1..=8 {
        let fx = fx.clone();
        tasks.push(tokio::spawn(async move {
            run(
                &fx,
                "alpha.config.write",
                json!({"schema_version": 1,
                       "project": {"expected": null, "values": {"limit": n}}}),
            )
            .await
        }));
    }
    let mut won = Vec::new();
    for task in tasks {
        if let Ok(stamps) = task.await.unwrap() {
            won.push(stamps);
        }
    }
    assert_eq!(won.len(), 1, "{won:?}");
    assert_eq!(
        won[0]["project"],
        json!({"schema_version": 1, "revision": 1})
    );
    // The stored value is the winner's, not a blend.
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert!((1..=8).contains(&config["values"]["limit"].as_i64().unwrap()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_racing_a_disable_and_a_generation_change_leave_a_consistent_layer() {
    let fx = Arc::new(services_with_effort().await);
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let writer = {
        let fx = fx.clone();
        tokio::spawn(async move {
            let mut accepted = 0u64;
            for n in 0..40 {
                let expected = if accepted == 0 {
                    Value::Null
                } else {
                    json!({"schema_version": 1, "revision": accepted})
                };
                let attempt = run(
                    &fx,
                    "alpha.config.write",
                    json!({"schema_version": 1,
                           "project": {"expected": expected, "values": {"limit": n % 90 + 1}}}),
                )
                .await;
                match attempt {
                    Ok(_) => accepted += 1,
                    // Refused while the extension is disabled or between
                    // generations; never a half write.
                    Err(
                        CommandError::Denied { .. }
                        | CommandError::Invalid { .. }
                        | CommandError::Unknown { .. },
                    ) => {}
                    Err(other) => panic!("{other:?}"),
                }
                tokio::task::yield_now().await;
            }
            accepted
        })
    };
    // Meanwhile: disable, change the generation (a sibling extension
    // appears), and enable again.
    let project = fx._dir.path().to_path_buf();
    write(
        &project,
        ".oxplow/project.yaml",
        "extensions:\n  disabled: [alpha]\n",
    );
    reconcile(&fx).await;
    declare(&project, "beta", "");
    reconcile(&fx).await;
    std::fs::remove_file(project.join(".oxplow/project.yaml")).unwrap();
    reconcile(&fx).await;
    let accepted = writer.await.unwrap();

    // The row counts exactly the writes that were accepted: none lost,
    // none invented, and still readable at the current schema.
    let stamp = stamp_of(&fx, "project").await;
    if accepted == 0 {
        assert_eq!(stamp, Value::Null);
    } else {
        assert_eq!(stamp["revision"].as_u64(), Some(accepted));
    }
    assert!(run(&fx, "alpha.config.read", json!({})).await.is_ok());
}

// ---- host denials with a hand-built run --------------------------------

fn host(fx: &EffortFixture) -> ConfigurationHost {
    ConfigurationHost::new(
        fx.svc.extension_catalog.clone(),
        fx.svc.layout.project_dir.clone(),
    )
}

fn owner(fx: &EffortFixture, generation: Option<String>) -> ContributionRef {
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
        version: ext.version.clone(),
        generation: Some(snapshot.generation().to_string()),
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
        grant: None,
        enforcement: crate::grants::enforcement(),
    })
}

async fn denied_snapshot(fx: &EffortFixture, run: RunContext) -> String {
    let host = host(fx);
    let refused = fx
        .svc
        .db
        .read_or(move |conn| {
            host.snapshot(conn, &run)
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
    let fx = services_with_effort().await;
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
        .read_or(move |conn| {
            host_ok
                .snapshot(conn, &ok)
                .map_err(oxplow_db::TxError::Aborted)
        })
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
async fn a_write_needs_a_transaction_a_cause_and_a_run_that_may_write() {
    use crate::commands::TxCtx;
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let input: crate::extension_configuration::WriteInput = serde_json::from_value(json!({
        "schema_version": 1,
        "project": {"expected": null, "values": {"limit": 5}}
    }))
    .unwrap();

    let attempt = |mode: Mode, cause: Option<oxplow_domain::EventId>| {
        let (host, input) = (host(&fx), input.clone());
        let run = declaring(
            "extension_config.write",
            RunContext::contribution(owner(&fx, None), Actor::Human, mode),
        );
        let db = fx.svc.db.clone();
        async move {
            db.transaction_or(move |tx| {
                let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
                let mut events = oxplow_db::EventCtx::system(&vocabulary, "test");
                events.cause = cause.clone();
                let ctx = TxCtx {
                    conn: tx,
                    run: &run,
                    events,
                    confirmed: true,
                    may_write: None,
                    depth: 0,
                    trace: &Default::default(),
                };
                host.compare_exchange(&ctx, &input)
                    .map(|_| ())
                    .map_err(oxplow_db::TxError::Aborted)
            })
            .await
        }
    };
    let cause = Some(oxplow_domain::EventId::generate());
    assert!(attempt(Mode::Pure, cause.clone()).await.is_err(), "pure");
    assert!(attempt(Mode::External, None).await.is_err(), "no cause");
    attempt(Mode::External, cause)
        .await
        .expect("a caused, external run writes");
    assert_eq!(stamp_of(&fx, "project").await["revision"], json!(1));
}

// ---- other readers -------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_script_reads_its_own_configuration() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 12}}}),
    )
    .await
    .unwrap();
    let peeked = run(&fx, "alpha.config.peek", json!({})).await.unwrap();
    assert_eq!(peeked["result"]["values"]["limit"], json!(12));
    assert_eq!(peeked["result"]["origins"]["/limit"], json!("project"));
    // Another extension's script reads its own namespace, never alpha's.
    declare(
        fx._dir.path(),
        "beta",
        &CONFIGURATION.replace("limit: 10", "limit: 77"),
    );
    reconcile(&fx).await;
    let theirs = run(&fx, "beta.config.peek", json!({})).await.unwrap();
    assert_eq!(theirs["result"]["values"]["limit"], json!(77));
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstalling_and_reinstalling_keeps_the_stored_layers() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1,
               "project": {"expected": null, "values": {"limit": 21}},
               "personal": {"expected": null, "values": {"label": "mine"}}}),
    )
    .await
    .unwrap();

    std::fs::remove_dir_all(fx._dir.path().join("oxplow/extensions/alpha")).unwrap();
    reconcile(&fx).await;
    assert!(fx.svc.commands.spec("alpha.config.read").is_none());

    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(21));
    assert_eq!(config["values"]["label"], json!("mine"));
    assert_eq!(
        stamp_of(&fx, "project").await,
        json!({"schema_version": 1, "revision": 1})
    );
}
