//! The live models: `v_ai_role`, `v_ai_provider`, `v_lsp_server`,
//! `v_collector_state`, `v_collector_credential`, `v_zone` and `v_zone_unmatched` read state that has no
//! change hook (a keychain, a yaml file, a process table, the worktree's files), so the gateway
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
            json!("oxplow-bundled/reasoning"),
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

const MANIFEST: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Pull.\npermissions: { process: true, secrets: [TOKEN] }\ncollectors:\n  - id: pull\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: item, key: id, columns: { id: int } }\n";

/// A collector's state and its credentials' state (set or not, never the
/// value) are read live.
#[tokio::test]
async fn collector_state_and_credentials_are_read_live() {
    let fx = services_with_effort().await;
    let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/acme");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.yaml"), MANIFEST).unwrap();
    std::fs::write(dir.join("sync.sh"), "#!/bin/sh\necho '[]'\n").unwrap();
    crate::test_fixtures::activate(&fx.svc).await;

    let state =
        "SELECT ref, owner, id, network_enforced FROM v_collector_state WHERE owner = 'acme'";
    assert_eq!(
        rows(&fx, state).await,
        vec![vec![
            json!("collector:acme/pull"),
            json!("acme"),
            json!("pull"),
            json!(if crate::net_sandbox::enforced() { 1 } else { 0 })
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

/// The zone table with the effect it has on this worktree: each rule's
/// matched files (first match wins), what fell through to `other`, and
/// the paths that did. A rule added to project.yaml, or a file added to
/// the worktree, shows in the next query with no event announcing either.
#[tokio::test]
async fn zones_show_what_each_rule_matches_in_the_worktree() {
    let fx = services_with_effort().await;
    let zone =
        "SELECT position, zone, patterns, files FROM v_zone ORDER BY position IS NULL, position";
    // No table declared: everything is `other`.
    for (path, body) in [
        ("src/a.rs", "a"),
        ("src/b.rs", "b"),
        ("docs/x.md", "x"),
        ("tools/y.sh", "y"),
    ] {
        let file = fx._dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    let none = rows(&fx, zone).await;
    assert_eq!(none.len(), 1, "{none:?}");
    assert_eq!(none[0][1], json!("other"));
    assert!(none[0][3].as_i64().unwrap() >= 4, "{none:?}");

    {
        let mut config = fx.svc.config.write().unwrap();
        let rule = |pattern: &str, zone: &str| oxplow_config::ZoneRuleConfig {
            patterns: vec![pattern.into()],
            zone: zone.into(),
            color: None,
        };
        config.zones = vec![
            rule("src/**", "core"),
            rule("docs/**", "docs"),
            rule("src/a.rs", "shadowed"),
        ];
    }
    let table = rows(&fx, zone).await;
    assert_eq!(
        table[..3],
        [
            vec![json!(1), json!("core"), json!("[\"src/**\"]"), json!(2)],
            vec![json!(2), json!("docs"), json!("[\"docs/**\"]"), json!(1)],
            // First match wins: `src/a.rs` never reaches this rule.
            vec![
                json!(3),
                json!("shadowed"),
                json!("[\"src/a.rs\"]"),
                json!(0)
            ],
        ]
    );
    // `other` closes the table, with no position.
    let other = table.last().unwrap();
    assert_eq!((&other[0], &other[1]), (&Value::Null, &json!("other")));

    let unmatched = rows(&fx, "SELECT path FROM v_zone_unmatched").await;
    assert!(
        unmatched.contains(&vec![json!("tools/y.sh")])
            && !unmatched.contains(&vec![json!("src/a.rs")]),
        "{unmatched:?}"
    );
}

/// The scopes a command can need, read from the table core ships: what
/// each lets a run change and where it runs — the agent's way to see what
/// `needs` can name.
#[tokio::test]
async fn scopes_list_what_a_command_may_need() {
    let fx = services_with_effort().await;
    let got = rows(
        &fx,
        "SELECT id, access, host FROM v_scope WHERE id IN ('records.read', 'work_items.write', 'tabs.write') ORDER BY id",
    )
    .await;
    assert_eq!(
        got,
        [
            vec![json!("records.read"), json!("read"), json!("daemon")],
            vec![json!("tabs.write"), json!("view"), json!("window")],
            vec![json!("work_items.write"), json!("record"), json!("daemon")],
        ]
    );
}

/// A scope that spends the person's money says so: a model call, and the
/// test call that checks a provider.
#[tokio::test]
async fn the_scope_that_spends_the_persons_key_says_so() {
    let fx = services_with_effort().await;
    let got = rows(
        &fx,
        "SELECT id, spends FROM v_scope WHERE spends = 1 ORDER BY id",
    )
    .await;
    assert_eq!(
        got,
        [
            vec![json!("ai.compute"), json!(1)],
            vec![json!("ai.test"), json!(1)]
        ]
    );
}

/// Rows of `sql` run as a lens for stream `stream` would (its row id).
async fn rows_in(fx: &EffortFixture, sql: &str, stream: i64) -> Vec<Vec<Value>> {
    let out = fx
        .svc
        .sql
        .run(oxplow_db::SqlQuery::new(sql).stream(Some(stream)))
        .await
        .unwrap();
    serde_json::to_value(&out.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().clone())
        .collect()
}

/// A second stream, working in a new git worktree of the project on branch
/// `branch`: its row id and its worktree.
async fn worktree_stream(fx: &EffortFixture, branch: &str) -> (i64, std::path::PathBuf) {
    use oxplow_domain::stores::StreamStore as _;
    let project = fx._dir.path();
    std::fs::write(project.join("seed.txt"), "seed").unwrap();
    crate::test_fixtures::commit_all(project, "seed");
    let wt = fx._dir.path().join(format!(
        "../{}-{branch}",
        project.file_name().unwrap().to_string_lossy()
    ));
    let status = std::process::Command::new("git")
        .args(["worktree", "add", "-q", "-b", branch, wt.to_str().unwrap()])
        .current_dir(project)
        .status()
        .unwrap();
    assert!(status.success());
    let primary = fx.svc.streams.ensure_primary().await.unwrap();
    let id = fx
        .svc
        .stream_store
        .upsert(&oxplow_domain::Stream {
            id: oxplow_domain::StreamId::placeholder(),
            kind: oxplow_domain::StreamKind::Worktree,
            title: branch.into(),
            branch: branch.into(),
            worktree_path: wt.to_string_lossy().into_owned(),
            ..primary
        })
        .await
        .unwrap();
    (id.value(), std::fs::canonicalize(wt).unwrap())
}

/// Every setting with its value and where it comes from, read when asked:
/// a value set in project.yaml after the last query shows, as `project`.
#[tokio::test]
async fn config_values_are_read_live_with_where_each_comes_from() {
    let fx = services_with_effort().await;
    let key = "SELECT origin, human_only FROM v_config_value WHERE key = 'snapshotRetentionDays'";
    assert_eq!(rows(&fx, key).await, vec![vec![json!("default"), json!(0)]]);
    std::fs::write(
        fx._dir.path().join(".oxplow/project.yaml"),
        "snapshotRetentionDays: 3\n",
    )
    .unwrap();
    fx.svc.config.write().unwrap().snapshot_retention_days = 3;
    assert_eq!(
        rows(
            &fx,
            "SELECT origin, value FROM v_config_value WHERE key = 'snapshotRetentionDays'"
        )
        .await,
        vec![vec![json!("project"), json!("3")]]
    );
    // The AI roles are settings too, documented like the rest.
    let roles = rows(
        &fx,
        "SELECT doc FROM v_config_value WHERE key = 'ai.roles.main'",
    )
    .await;
    assert_eq!(roles.len(), 1);
    assert!(
        roles[0][0].as_str().is_some_and(|d| !d.is_empty()),
        "{roles:?}"
    );
}

/// A stream's files are its own worktree's: a query run for a stream lists
/// that worktree, one run for none lists the primary checkout's; folders
/// are rows too, so a directory listing is `WHERE parent = ?`.
#[tokio::test]
async fn workspace_files_are_the_streams_worktrees() {
    let fx = services_with_effort().await;
    let (stream, wt) = worktree_stream(&fx, "side").await;
    std::fs::create_dir_all(fx._dir.path().join("src")).unwrap();
    std::fs::write(fx._dir.path().join("src/a.rs"), "a").unwrap();
    std::fs::create_dir_all(wt.join("lib")).unwrap();
    std::fs::write(wt.join("lib/b.rs"), "b").unwrap();

    let listing = "SELECT path, parent, name, kind FROM v_workspace_file WHERE path IN ('src', 'src/a.rs', 'lib', 'lib/b.rs') ORDER BY path";
    assert_eq!(
        rows(&fx, listing).await,
        vec![
            vec![json!("src"), json!(""), json!("src"), json!("dir")],
            vec![
                json!("src/a.rs"),
                json!("src"),
                json!("a.rs"),
                json!("file")
            ],
        ]
    );
    assert_eq!(
        rows_in(&fx, listing, stream).await,
        vec![
            vec![json!("lib"), json!(""), json!("lib"), json!("dir")],
            vec![
                json!("lib/b.rs"),
                json!("lib"),
                json!("b.rs"),
                json!("file")
            ],
        ]
    );
    // An untracked file says so; the rows carry the stream they list.
    assert_eq!(
        rows_in(
            &fx,
            "SELECT stream_id, status FROM v_workspace_file WHERE path = 'lib/b.rs'",
            stream
        )
        .await,
        vec![vec![json!(stream), json!("untracked")]]
    );
}

/// The repository's working copies, and the stream using each: one no
/// stream uses is what "adopt a worktree" offers.
#[tokio::test]
async fn vcs_workspaces_say_which_stream_uses_each() {
    let fx = services_with_effort().await;
    let (stream, _) = worktree_stream(&fx, "used").await;
    let project = fx._dir.path();
    let loose = project.join(format!(
        "../{}-loose",
        project.file_name().unwrap().to_string_lossy()
    ));
    let status = std::process::Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "loose",
            loose.to_str().unwrap(),
        ])
        .current_dir(project)
        .status()
        .unwrap();
    assert!(status.success());
    let all = rows(&fx, "SELECT branch, is_main, stream_id IS NOT NULL FROM v_vcs_workspace ORDER BY is_main DESC, branch").await;
    assert_eq!(all.len(), 3, "{all:?}");
    assert_eq!(all[0][1], json!(1));
    assert_eq!(
        rows(
            &fx,
            "SELECT branch FROM v_vcs_workspace WHERE NOT is_main AND stream_id IS NULL"
        )
        .await,
        vec![vec![json!("loose")]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT stream_id FROM v_vcs_workspace WHERE branch = 'used'"
        )
        .await,
        vec![vec![json!(stream)]]
    );
}

/// Every harness's saved conversations begun in the stream's worktree; a
/// harness that can't list now is a row saying why, not a failed query.
#[tokio::test]
async fn agent_conversations_are_each_harness_saved_ones() {
    let fx = services_with_effort().await;
    fx.svc
        .harnesses
        .register(crate::test_fixtures::listing_harness(
            "lists",
            vec![crate::test_fixtures::conversation("c1", 5)],
        ));
    fx.svc
        .harnesses
        .register(std::sync::Arc::new(crate::test_fixtures::Listing {
            id: "down",
            answer: Err("its daemon isn't running".into()),
        }));
    assert_eq!(
        rows(
            &fx,
            "SELECT native_id, title, problem FROM v_agent_conversation WHERE harness = 'lists'"
        )
        .await,
        vec![vec![json!("c1"), json!("about c1"), Value::Null]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT native_id, problem FROM v_agent_conversation WHERE harness = 'down'"
        )
        .await,
        vec![vec![Value::Null, json!("its daemon isn't running")]]
    );
}

/// A query over a live model says it read it, as over any model: what a
/// lens re-runs on.
#[tokio::test]
async fn a_query_over_a_live_model_reads_it() {
    let fx = services_with_effort().await;
    let out = fx
        .svc
        .sql
        .query_sql("SELECT zone FROM v_zone", vec![], None)
        .await
        .unwrap();
    assert!(
        out.reads.models.contains(&"v_zone".to_string()),
        "{:?}",
        out.reads
    );
}

/// A change to what a live model reads announces the model: the events
/// that change it, and the models it reads through.
#[test]
fn a_change_names_the_live_models_that_follow_it() {
    use crate::events::OxplowEvent;
    let after = |e: OxplowEvent| crate::models_changed::live_models_after(&e);
    let config = after(OxplowEvent::ConfigChanged);
    assert!(
        config.contains(&"v_config_value".to_string()) && config.contains(&"v_zone".to_string()),
        "{config:?}"
    );
    let files = after(OxplowEvent::WorkspaceChanged {
        stream_id: oxplow_domain::StreamId::new(1),
        change_kind: crate::events::WorkspaceChangeKind::Created,
        path: "src/a.rs".into(),
    });
    assert!(
        files.contains(&"v_workspace_file".to_string())
            && files.contains(&"v_zone_unmatched".to_string()),
        "{files:?}"
    );
    assert!(after(OxplowEvent::BackgroundTasksChanged).is_empty());
    assert_eq!(
        crate::models_changed::live_models_reading(&["v_agent_session".to_string()]),
        vec!["v_agent_conversation".to_string()]
    );
}

/// The registered agent harnesses, the ones `agents:` names first (a new
/// session's default is the first), each saying whether the project enables
/// it: what New Session's agent choice lists.
#[tokio::test]
async fn agent_harnesses_are_the_registered_ones_enabled_ones_first() {
    let fx = services_with_effort().await;
    fx.svc
        .harnesses
        .register(crate::test_fixtures::listing_harness("zeta", vec![]));
    fx.svc
        .harnesses
        .register(crate::test_fixtures::listing_harness("alpha", vec![]));
    let found = rows(
        &fx,
        "SELECT id, enabled FROM v_agent_harness WHERE id IN ('alpha', 'zeta') ORDER BY position",
    )
    .await;
    // With no `agents:`, every harness is enabled.
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|r| r[1] == json!(1)), "{found:?}");
    assert_eq!(
        rows(&fx, "SELECT title FROM v_agent_harness WHERE id = 'alpha'").await,
        vec![vec![json!("alpha")]]
    );
}
