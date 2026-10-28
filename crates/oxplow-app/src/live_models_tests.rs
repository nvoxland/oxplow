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

const MANIFEST: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Pull.\npermissions: [process.run, secrets:TOKEN]\ncollectors:\n  - id: pull\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: item, key: id, columns: { id: int } }\n";

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
        "SELECT uri, owner, id, network_enforced FROM v_collector_state WHERE owner = 'acme'";
    assert_eq!(
        rows(&fx, state).await,
        vec![vec![
            json!("oxplow:oxplow/collector?collector=acme/pull"),
            json!("acme"),
            json!("pull"),
            json!(if crate::net_sandbox::enforced() { 1 } else { 0 })
        ]]
    );
    let credentials = "SELECT collector, name, is_set FROM v_collector_credential WHERE collector = 'oxplow:oxplow/collector?collector=acme/pull'";
    assert_eq!(
        rows(&fx, credentials).await,
        vec![vec![
            json!("oxplow:oxplow/collector?collector=acme/pull"),
            json!("TOKEN"),
            json!(0)
        ]]
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
        vec![vec![
            json!("oxplow:oxplow/collector?collector=acme/pull"),
            json!("TOKEN"),
            json!(1)
        ]]
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
        let declared: Vec<&str> = decl.fields.iter().map(|c| c.name.as_str()).collect();
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

/// Rows of `sql` run as a page for stream `stream` would (its row id).
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
    let key = "SELECT origin FROM v_config_value WHERE key = 'snapshotRetentionDays'";
    assert_eq!(rows(&fx, key).await, vec![vec![json!("default")]]);
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

/// A query that says which folder (`WHERE parent = …`, a literal or a
/// parameter) reads that folder alone, and gets the rows the whole walk
/// has for it: its files and folders the filter keeps, an empty folder too.
/// A folder the walk can't enter elsewhere in the tree doesn't stop it.
#[tokio::test]
async fn a_folder_listing_reads_that_folder_alone() {
    use std::os::unix::fs::PermissionsExt;
    let fx = services_with_effort().await;
    let root = fx._dir.path();
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::create_dir_all(root.join("src/empty")).unwrap();
    std::fs::create_dir_all(root.join("src/skipped")).unwrap();
    std::fs::write(root.join("src/a.rs"), "a").unwrap();
    std::fs::write(root.join("src/deep/x.rs"), "x").unwrap();
    std::fs::write(root.join("src/skipped/y.rs"), "y").unwrap();
    std::fs::write(root.join(".gitignore"), "skipped/\n").unwrap();

    let walked =
        "SELECT path, name, kind FROM v_workspace_file WHERE parent || '' = 'src' ORDER BY path";
    let expected = vec![
        vec![json!("src/a.rs"), json!("a.rs"), json!("file")],
        vec![json!("src/deep"), json!("deep"), json!("dir")],
        vec![json!("src/empty"), json!("empty"), json!("dir")],
    ];
    assert_eq!(rows(&fx, walked).await, expected);
    let folder = "SELECT path, name, kind FROM v_workspace_file WHERE parent = ?1 ORDER BY path";
    let read = |sql: &'static str, p: &'static str| {
        let svc = fx.svc.clone();
        async move {
            let out = svc
                .sql
                .query_sql(sql, vec![oxplow_db::SqlCell::Text(p.into())], None)
                .await;
            out.map(|o| serde_json::to_value(&o.rows).unwrap())
        }
    };
    assert_eq!(read(folder, "src").await.unwrap(), json!(expected));
    // An ignored folder, or one under it, lists nothing — as the walk,
    // which never enters it; so does one that isn't there.
    for gone in ["src/skipped", "nowhere"] {
        assert_eq!(read(folder, gone).await.unwrap(), json!([]), "{gone}");
    }

    // A folder the walk can't read: the walk fails, the folder read doesn't.
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let walk = fx.svc.sql.query_sql(walked, vec![], None).await;
    let folder_read = read(folder, "src").await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(walk.is_err(), "the whole walk enters every folder");
    assert_eq!(folder_read.unwrap(), json!(expected));
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

/// How far one branch has moved from another, and the commits between
/// them, read from the VCS when a query names them, narrowed to the pair
/// it pins: with one side pinned the other ranges over every branch; with
/// none, every local branch is compared with the default. A side the VCS
/// can't resolve gives no row.
#[tokio::test]
async fn branch_divergence_and_the_commits_between_are_read_live() {
    let fx = services_with_effort().await;
    let (_, wt) = worktree_stream(&fx, "feature").await;
    let project = fx._dir.path();
    let split = fx.svc.vcs.resolve(project, "HEAD").await.unwrap();
    std::fs::write(wt.join("seed.txt"), "seed, grown").unwrap();
    std::fs::write(wt.join("feature.txt"), "f").unwrap();
    let on_feature = crate::test_fixtures::commit_all(&wt, "feature work");
    std::fs::write(project.join("seed.txt"), "seed, pruned").unwrap();
    let on_main = crate::test_fixtures::commit_all(project, "main work");

    assert_eq!(
        rows(
            &fx,
            "SELECT ahead, behind, merge_base, overlapping_files, readiness, stream_id IS NOT NULL
               FROM v_branch_divergence WHERE base = 'main' AND head = 'feature'"
        )
        .await,
        vec![vec![
            json!(1),
            json!(1),
            json!(split),
            json!("[\"seed.txt\"]"),
            json!("conflict"),
            json!(1)
        ]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT ahead, readiness FROM v_branch_divergence WHERE base = 'feature' AND head = 'main'"
        )
        .await,
        vec![vec![json!(1), json!("conflict")]]
    );
    // Head pinned alone: every other branch as the base.
    assert_eq!(
        rows(
            &fx,
            "SELECT base, head FROM v_branch_divergence WHERE head = 'feature' ORDER BY base"
        )
        .await,
        vec![vec![json!("main"), json!("feature")]]
    );
    // Nothing pinned: each local branch against the default, which is left out.
    assert_eq!(
        rows(
            &fx,
            "SELECT base, head, ahead FROM v_branch_divergence ORDER BY head"
        )
        .await,
        vec![vec![json!("main"), json!("feature"), json!(1)]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT ahead FROM v_branch_divergence WHERE base = 'nope' AND head = 'feature'"
        )
        .await,
        Vec::<Vec<Value>>::new()
    );

    assert_eq!(
        rows(
            &fx,
            "SELECT sha, short_sha = substr(sha, 1, length(short_sha)), author, subject, committed_at IS NOT NULL
               FROM v_commit_between WHERE base = 'main' AND head = 'feature'"
        )
        .await,
        vec![vec![json!(on_feature), json!(1), json!("test"), json!("feature work"), json!(1)]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT base, head, sha FROM v_commit_between WHERE base = 'feature' ORDER BY head"
        )
        .await,
        vec![vec![json!("feature"), json!("main"), json!(on_main)]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT sha FROM v_commit_between WHERE base = 'main' AND head = 'main'"
        )
        .await,
        Vec::<Vec<Value>>::new()
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
/// page re-runs on.
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
    // What a pair of branches compares moves with the refs, and the
    // branches an unpinned side ranges over are `v_branch`'s.
    let refs = after(OxplowEvent::VcsRefsChanged {
        stream_id: oxplow_domain::StreamId::new(1),
    });
    let branches = crate::models_changed::live_models_reading(&["v_branch".to_string()]);
    for model in ["v_branch_divergence", "v_commit_between"] {
        assert!(refs.contains(&model.to_string()), "{refs:?}");
        assert!(branches.contains(&model.to_string()), "{branches:?}");
    }
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
    // The settings a harness declares come with it (`claude` declares a
    // model), a harness with none has `[]`.
    let settings = rows(
        &fx,
        "SELECT id, json_array_length(settings) FROM v_agent_harness WHERE id IN ('alpha', 'claude') ORDER BY id",
    )
    .await;
    assert_eq!(settings[0], vec![json!("alpha"), json!(0)], "{settings:?}");
    if let Some(claude) = settings.iter().find(|r| r[0] == json!("claude")) {
        assert!(claude[1].as_i64().unwrap() > 0, "{settings:?}");
    }
}

/// A collector is readable as one `v_collector` row: declared, enabled and
/// never run; then with its last run.
#[tokio::test]
async fn a_collector_is_a_v_collector_row_with_its_standing() {
    let fx = services_with_effort().await;
    let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/acme");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.yaml"), MANIFEST).unwrap();
    std::fs::write(
        dir.join("sync.sh"),
        "#!/bin/sh\necho '{\"entities\":{\"item\":[]}}'\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("sync.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    crate::test_fixtures::activate(&fx.svc).await;

    let sql = "SELECT uri, extension, id, trigger, enabled, state, last_run_at, last_status \
               FROM v_collector WHERE extension = 'acme'";
    assert_eq!(
        rows(&fx, sql).await,
        vec![vec![
            json!("oxplow:oxplow/collector?collector=acme/pull"),
            json!("acme"),
            json!("pull"),
            json!("call"),
            json!(1),
            json!("never_run"),
            Value::Null,
            Value::Null,
        ]]
    );

    // After a run it has a last run and stands `ok`.
    let root = fx.svc.layout.project_dir.clone();
    crate::collector_runner::run_collector(
        &crate::collector_runner::Collectors::of(&fx.svc, &root),
        &oxplow_domain::run::RunContext::core(
            oxplow_domain::Actor::Human,
            oxplow_domain::run::Mode::External,
        ),
        "acme",
        "pull",
        crate::collector_runner::RunTrigger::Manual,
    )
    .await
    .unwrap();
    let after = rows(
        &fx,
        "SELECT state, last_status, last_run_at IS NOT NULL, error FROM v_collector WHERE id = 'pull'",
    )
    .await;
    assert_eq!(
        after,
        vec![vec![json!("ok"), json!("ok"), json!(1), Value::Null]]
    );
}
