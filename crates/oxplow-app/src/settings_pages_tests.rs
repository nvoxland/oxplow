//! Foundation pages that replaced core pages: Settings' Overview and Language
//! Servers areas (mounted in each area's slot: what they list, how they group
//! and search, what Ask puts in the agent's input, what the language-server
//! rows offer), the Comments inbox, the commit and uncommitted pages, the Alerts page, Settings → Agents and AI, the Board, and the wiki page.

use oxplow_config::LspServerConfig;
use oxplow_db::SqlCell;

use crate::extensions::ViewContext;
use crate::pages::actions::{run_page_action, PageActionCall};
use crate::pages::open_runs::PageRuns;
use crate::pages::runner::{PageRun, RunRequest, RunTarget};
use crate::test_fixtures::services_with_effort;

type Row = Vec<SqlCell>;

struct Run {
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl Run {
    fn text(&self, row: &Row, column: &str) -> Option<String> {
        let at = self.columns.iter().position(|c| c == column).unwrap();
        match &row[at] {
            SqlCell::Text(t) => Some(t.clone()),
            _ => None,
        }
    }
    fn flag(&self, row: &Row, column: &str) -> bool {
        let at = self.columns.iter().position(|c| c == column).unwrap();
        matches!(&row[at], SqlCell::Int(1))
    }
    fn keyed(&self, column: &str, value: &str) -> &Row {
        self.rows
            .iter()
            .find(|r| self.text(r, column).as_deref() == Some(value))
            .unwrap_or_else(|| panic!("no row with {column} = {value}"))
    }
    fn has(&self, column: &str, value: &str) -> bool {
        self.rows
            .iter()
            .any(|r| self.text(r, column).as_deref() == Some(value))
    }
}

async fn run(
    fx: &crate::test_fixtures::EffortFixture,
    page: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let params: Vec<(String, SqlCell)> = params
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let run =
        crate::test_fixtures::run_as_page(svc, &format!("oxplow-foundation/{page}"), &params).await;
    Run {
        columns: run.columns,
        rows: run.rows,
    }
}

fn request(page: &str, params: Vec<(&str, SqlCell)>) -> RunRequest {
    RunRequest {
        target: RunTarget::Page(format!("oxplow-foundation/{page}")),
        params: params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        ctx: ViewContext::default(),
        viewer: oxplow_domain::Actor::Human,
    }
}

fn loader_of(run: &PageRun, loader: &str) -> Run {
    let ran = run
        .loaders
        .get(loader)
        .unwrap_or_else(|| panic!("{}: no loader {loader}", run.page.id));
    assert_eq!(ran.error, None, "{}/{loader}", run.page.id);
    Run {
        columns: ran.result.columns.clone(),
        rows: ran.result.rows.clone(),
    }
}

/// Press `action` on node `node` (none: the whole page) of a fresh open
/// run of foundation's page `page`, on the row whose `key` column is
/// `value` in the node's loader `loader` (none: no row).
async fn press(
    svc: &crate::Services,
    page: &str,
    params: Vec<(&str, SqlCell)>,
    node: Option<&str>,
    action: &str,
    row: Option<(&str, &str, &str)>,
) -> Result<oxplow_domain::CommandOutcome, oxplow_domain::CommandError> {
    let runs = PageRuns::default();
    let (run_id, run) = runs.open(svc, request(page, params), "w").await.unwrap();
    let row = row.map(|(loader, key, value)| {
        let rows = loader_of(&run, loader);
        let found = rows.keyed(key, value).clone();
        rows.columns.iter().cloned().zip(found).collect()
    });
    run_page_action(
        svc,
        &runs,
        PageActionCall {
            run_id,
            node: node.map(str::to_string),
            action: action.into(),
            row,
            input: None,
            drop: None,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed: true,
        },
    )
    .await
}

/// A server of `.oxplow/project.yaml` is listed, offers no Remove (it is the
/// file's), and the suggestions leave out its language.
#[tokio::test]
async fn a_configured_language_server_is_listed_and_not_suggested_again() {
    let fx = services_with_effort().await;
    let before = run(&fx, "lsp-suggestions", vec![]).await;
    assert!(before.has("package", "rust-analyzer"));

    fx.svc
        .config
        .write()
        .unwrap()
        .lsp_servers
        .push(LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "/opt/bin/rust-analyzer".into(),
            args: vec![],
        });

    let servers = run(&fx, "lsp-servers", vec![]).await;
    let rust = servers.keyed("language", "rust");
    assert_eq!(
        servers.text(rust, "program").as_deref(),
        Some("rust-analyzer")
    );
    assert_eq!(
        servers.text(rust, "source").as_deref(),
        Some("project.yaml")
    );
    assert!(!servers.flag(rust, "can_remove"));
    assert!(!servers.flag(rust, "can_restart"));

    let after = run(&fx, "lsp-suggestions", vec![]).await;
    assert!(!after.has("package", "rust-analyzer"));
}

async fn command(
    fx: &crate::test_fixtures::EffortFixture,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    fx.svc
        .commands
        .run(&oxplow_domain::Actor::Human, name, input, false)
        .await
        .unwrap()
        .result
}

fn sha_params(stream: i64, sha: &str) -> Vec<(&'static str, SqlCell)> {
    vec![
        ("stream_id", SqlCell::Int(stream)),
        ("sha", SqlCell::Text(sha.to_string())),
    ]
}

/// A commit page: its document's row makes the commit's change and hands the
/// change and its revisions to the parts; the files come as a tree, folders
/// first, each file with its status and the diff its link opens.
#[tokio::test]
async fn the_commit_page_reads_a_commit_its_change_and_its_files_as_a_tree() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    std::fs::write(root.join("base.txt"), "x\n").unwrap();
    crate::test_fixtures::commit_all(&root, "base");
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::write(root.join("src/deep/a.rs"), "a\n").unwrap();
    std::fs::write(root.join("top.txt"), "t\n").unwrap();
    let sha = crate::test_fixtures::commit_all(&root, "add things\n\nwith a body");
    let stream = fx.svc.streams.ensure_primary().await.unwrap().id.value();

    let doc = run(&fx, "commit", sha_params(stream, &sha)).await;
    assert_eq!(doc.rows.len(), 1);
    let row = &doc.rows[0];
    assert!(matches!(
        &row[doc.columns.iter().position(|c| c == "change_id").unwrap()],
        SqlCell::Int(_)
    ));
    assert_eq!(doc.text(row, "head_revision"), Some(format!("git:{sha}")));
    assert!(doc.text(row, "base_revision").is_some());

    let detail = run(&fx, "commit-detail", sha_params(stream, &sha)).await;
    let d = &detail.rows[0];
    assert_eq!(detail.text(d, "subject").as_deref(), Some("add things"));
    assert_eq!(detail.text(d, "sha"), Some(sha.clone()));

    let files = run(
        &fx,
        "commit-files",
        vec![
            ("sha", SqlCell::Text(sha.clone())),
            ("base_revision", SqlCell::Text("git:base".into())),
            ("head_revision", SqlCell::Text(format!("git:{sha}"))),
        ],
    )
    .await;
    let nodes: Vec<(String, Option<String>, String, String)> = files
        .rows
        .iter()
        .map(|r| {
            (
                files.text(r, "id").unwrap(),
                files.text(r, "parent"),
                files.text(r, "name").unwrap(),
                files.text(r, "icon").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        nodes,
        vec![
            (
                "src".to_string(),
                None,
                "src".to_string(),
                "folder".to_string()
            ),
            (
                "src/deep".to_string(),
                Some("src".to_string()),
                "deep".to_string(),
                "folder".to_string()
            ),
            (
                "src/deep/a.rs".to_string(),
                Some("src/deep".to_string()),
                "a.rs".to_string(),
                "added".to_string()
            ),
            (
                "top.txt".to_string(),
                None,
                "top.txt".to_string(),
                "added".to_string()
            ),
        ]
    );
    // A file carries the diff its link opens; a folder has none.
    assert_eq!(
        files.text(&files.rows[2], "file").as_deref(),
        Some("src/deep/a.rs")
    );
    assert_eq!(files.text(&files.rows[0], "file"), None);
    assert_eq!(
        files.text(&files.rows[2], "head_revision"),
        Some(format!("git:{sha}"))
    );
}

/// The uncommitted page makes the stream's working change and lists the
/// working tree's files as a tree, untracked ones as the change marks them.
#[tokio::test]
async fn the_uncommitted_page_lists_the_working_tree_with_untracked_marked() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    std::fs::write(root.join("tracked.txt"), "x\n").unwrap();
    crate::test_fixtures::commit_all(&root, "base");
    std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
    std::fs::write(root.join("loose.txt"), "n\n").unwrap();
    let stream = fx.svc.streams.ensure_primary().await.unwrap().id.value();

    let doc = run(
        &fx,
        "uncommitted",
        vec![("stream_id", SqlCell::Int(stream))],
    )
    .await;
    assert_eq!(doc.rows.len(), 1);
    let row = doc.rows[0].clone();
    let at = |name: &str| row[doc.columns.iter().position(|c| c == name).unwrap()].clone();
    let change = at("change_id");
    assert!(matches!(change, SqlCell::Int(_)), "{change:?}");

    // Its files are listed by the change's analysis, a moment later.
    let mut icons = Vec::new();
    for _ in 0..50 {
        let files = run(
            &fx,
            "uncommitted-files",
            // The page finds its own change from the stream.
            vec![("stream_id", SqlCell::Int(stream))],
        )
        .await;
        icons = files
            .rows
            .iter()
            .map(|r| (files.text(r, "id").unwrap(), files.text(r, "icon").unwrap()))
            .collect();
        if icons.iter().any(|(id, _)| id == "tracked.txt") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        icons.contains(&("tracked.txt".to_string(), "modified".to_string())),
        "{icons:?}"
    );
    // A file git doesn't track yet reads as untracked once the change's
    // status is in, added before.
    assert!(
        icons
            .iter()
            .any(|(id, icon)| id == "loose.txt" && (icon == "untracked" || icon == "added")),
        "{icons:?}"
    );

    // The commit form shows once there are files to commit.
    let form = run(
        &fx,
        "uncommitted-commit",
        vec![("stream_id", SqlCell::Int(stream)), ("change_id", change)],
    )
    .await;
    assert_eq!(form.rows.len(), 1);
    assert_eq!(
        form.text(&form.rows[0], "stream"),
        Some(format!("str{stream}"))
    );
}

async fn run_task(
    fx: &crate::test_fixtures::TaskEffortFixture,
    page: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let params: Vec<(String, SqlCell)> = params
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let run =
        crate::test_fixtures::run_as_page(svc, &format!("oxplow-foundation/{page}"), &params).await;
    Run {
        columns: run.columns,
        rows: run.rows,
    }
}

/// Settings → Extensions: a collector turned off after failures says why
/// and offers Enable Again and Repair; a credential an extension's
/// collectors declare is listed once, with whether a value is kept (never
/// the value).
#[tokio::test(flavor = "multi_thread")]
async fn the_extensions_area_lists_health_and_credentials() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/acme");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { process: true, secrets: [TOKEN] }\ncollectors:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row, key: id, columns: { id: int } }\n  - id: t\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row2, key: id, columns: { id: int } }\n",
    )
    .unwrap();
    std::fs::write(ext.join("sync.sh"), "#!/bin/sh\necho '{\"entities\":{}}'\n").unwrap();
    let fx = crate::test_fixtures::services_with_effort_in(dir).await;
    crate::test_fixtures::activate(&fx.svc).await;
    fx.svc
        .db
        .transaction(|tx| {
            oxplow_db::contribution_health_store::disable_tx(
                tx,
                &crate::collector_runner::contribution_key("acme", "s"),
                "3 failures in a row; the last: boom",
                "t",
            )
        })
        .await
        .unwrap();

    let health = run(&fx, "extensions-health", vec![]).await;
    let row = health.keyed("contribution", "s");
    assert_eq!(
        health.text(row, "status").as_deref(),
        Some("Disabled: 3 failures in a row; the last: boom")
    );
    assert!(health.flag(row, "can_enable"));
    // No open repair task yet: nothing to hand the agent.
    assert!(!health.flag(row, "can_repair"));

    let creds = run(&fx, "extensions-credentials", vec![]).await;
    assert_eq!(
        creds.rows.len(),
        1,
        "one TOKEN for the extension: {:?}",
        creds.rows
    );
    assert_eq!(
        creds.text(&creds.rows[0], "extension").as_deref(),
        Some("acme")
    );
    assert_eq!(
        creds.text(&creds.rows[0], "status").as_deref(),
        Some("Not set")
    );
    command(
        &fx,
        "oxplow.collector.set_credential",
        serde_json::json!({ "collector": creds.text(&creds.rows[0], "collector").unwrap(), "name": "TOKEN", "value": "v" }),
    )
    .await;
    let creds = run(&fx, "extensions-credentials", vec![]).await;
    assert_eq!(
        creds.text(&creds.rows[0], "status").as_deref(),
        Some("Saved in keychain")
    );
    assert!(creds.flag(&creds.rows[0], "is_set"));

    assert!(run(&fx, "extensions-secrets", vec![]).await.rows.is_empty());
}

/// The Git Dashboard: the stream's branch with no upstream, the working
/// tree, the last commits; with no other stream or remote the rest is empty.
#[tokio::test(flavor = "multi_thread")]
async fn the_git_dashboard_reads_the_branch_the_tree_and_the_recent_commits() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    std::fs::write(fx.fx._dir.path().join("new.txt"), "x\n").unwrap();
    let stream = vec![("stream_id", SqlCell::Int(1))];
    let page = run_task(&fx, "git-dashboard", stream.clone()).await;
    assert_eq!(page.rows.len(), 1, "{:?}", page.rows);
    let branch = page.text(&page.rows[0], "branch").unwrap();
    let with_branch = |extra: Vec<(&'static str, SqlCell)>| {
        let mut p = stream.clone();
        p.push(("branch", SqlCell::Text(branch.clone())));
        p.extend(extra);
        p
    };
    fx.svc
        .git_store
        .replace_branches(vec![oxplow_db::GitBranchRow {
            name: branch.clone(),
            kind: "local".into(),
            remote: None,
            head_sha: None,
            stream_id: Some(1),
            is_default: true,
            upstream: None,
            ahead: 0,
            behind: 0,
        }])
        .await
        .unwrap();
    let upstream = run_task(&fx, "git-dashboard-upstream", with_branch(vec![])).await;
    assert_eq!(
        upstream.text(&upstream.rows[0], "status").as_deref(),
        Some("No upstream")
    );
    fx.svc
        .git_store
        .upsert_commit(oxplow_db::GitCommitRow {
            sha: "a".repeat(40),
            author: "Ada".into(),
            email: "ada@example.com".into(),
            committed_secs: 1_700_000_000,
            subject: "first".into(),
            body: String::new(),
            parents: vec![],
            files: vec![],
        })
        .await
        .unwrap();
    fx.svc
        .git_store
        .replace_stream_heads(vec![(1, "a".repeat(40))])
        .await
        .unwrap();
    let recent = run_task(&fx, "git-dashboard-recent", stream.clone()).await;
    assert!(!recent.rows.is_empty(), "the fixture repo has a commit");
    assert!(recent
        .text(&recent.rows[0], "commit_ref")
        .unwrap()
        .contains("commit?sha="));
    let streams = run_task(&fx, "git-dashboard-streams", with_branch(vec![])).await;
    assert!(streams.rows.is_empty());
    assert!(
        run_task(&fx, "git-dashboard-readiness", with_branch(vec![]))
            .await
            .rows
            .is_empty()
    );
    assert!(run_task(&fx, "git-dashboard-remotes", with_branch(vec![]))
        .await
        .rows
        .is_empty());
    let between = run_task(
        &fx,
        "git-dashboard-between",
        vec![
            ("base", SqlCell::Text(branch.clone())),
            ("head", SqlCell::Text(branch)),
        ],
    )
    .await;
    assert!(between.rows.is_empty());
    let tree = run_task(&fx, "git-dashboard-uncommitted", stream).await;
    assert_eq!(tree.rows.len(), 1);
}

/// The Files page: all files a folder at a time, or the changed files in
/// their folders (the working tree, each opening a diff), with the actions
/// on a row offered where they apply.
#[tokio::test(flavor = "multi_thread")]
async fn the_files_page_lists_a_folder_or_the_changed_files() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let root = fx.fx._dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/new.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("top.txt"), "x\n").unwrap();
    let stream = vec![("stream_id", SqlCell::Int(1))];

    let branch = fx
        .svc
        .sql
        .query_sql("SELECT branch FROM v_stream WHERE id = 1", vec![], None)
        .await
        .unwrap()
        .rows[0][0]
        .clone();
    let SqlCell::Text(branch) = branch else {
        panic!("a stream has a branch")
    };
    fx.svc
        .git_store
        .replace_branches(vec![oxplow_db::GitBranchRow {
            name: branch,
            kind: "local".into(),
            remote: None,
            head_sha: None,
            stream_id: Some(1),
            is_default: true,
            upstream: None,
            ahead: 0,
            behind: 0,
        }])
        .await
        .unwrap();
    // The scopes on offer: at least all files and the working tree's.
    let scopes = run_task(&fx, "files-scopes", stream.clone()).await;
    assert!(scopes.has("value", "all"), "{:?}", scopes.rows);
    assert!(scopes.has("value", "uncommitted"), "{:?}", scopes.rows);

    // The page's row: the working tree scope names a working change.
    let mut p = stream.clone();
    p.push(("scope", SqlCell::Text("uncommitted".into())));
    let page = run_task(&fx, "files", p).await;
    assert_eq!(
        page.text(&page.rows[0], "change_kind").as_deref(),
        Some("working")
    );

    // All files, a folder at a time.
    let tree = |scope: &str, folder: &str, kind: SqlCell, target: SqlCell| {
        let mut p = stream.clone();
        p.push(("scope", SqlCell::Text(scope.into())));
        p.push(("folder", SqlCell::Text(folder.into())));
        p.push(("change_kind", kind));
        p.push(("change_target", target));
        p
    };
    let all = run_task(
        &fx,
        "files-tree",
        tree(
            "all",
            "",
            SqlCell::Text(String::new()),
            SqlCell::Text(String::new()),
        ),
    )
    .await;
    assert!(
        all.has("name", "top.txt") && all.has("name", "src"),
        "{:?}",
        all.rows
    );
    let src = run_task(
        &fx,
        "files-tree",
        tree(
            "all",
            "src",
            SqlCell::Text(String::new()),
            SqlCell::Text(String::new()),
        ),
    )
    .await;
    // A folder's rows are the folder's children, parented on it, so they
    // nest where the tree splices them in; only all-files folders open
    // lazily (`opens`).
    assert!(
        src.has("name", "new.rs") && !src.has("name", ".."),
        "{:?}",
        src.rows
    );
    let child = src.keyed("name", "new.rs");
    assert_eq!(src.text(child, "parent").as_deref(), Some("src"));
    assert!(all.flag(all.keyed("name", "src"), "opens"));
    assert!(!all.flag(all.keyed("name", "top.txt"), "opens"));
    let file = src
        .rows
        .iter()
        .find(|r| src.text(r, "name").as_deref() == Some("new.rs"))
        .unwrap();
    assert_eq!(
        src.text(file, "target").as_deref(),
        Some("oxplow:oxplow/file?path=src/new.rs")
    );

    // Only the changed files: nested, each opening a diff. (The working
    // change is made on first ask and computed after; the Uncommitted page
    // shows the same wait.)
    run_task(&fx, "uncommitted", stream.clone()).await;
    let changed_params = || {
        tree(
            "uncommitted",
            "",
            SqlCell::Text("working".into()),
            SqlCell::Text(String::new()),
        )
    };
    let mut changed = run_task(&fx, "files-tree", changed_params()).await;
    for _ in 0..50 {
        if !changed.rows.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        changed = run_task(&fx, "files-tree", changed_params()).await;
    }
    let new = changed
        .rows
        .iter()
        .find(|r| changed.text(r, "id").as_deref() == Some("src/new.rs"))
        .unwrap_or_else(|| panic!("{:?}", changed.rows));
    assert_eq!(changed.text(new, "parent").as_deref(), Some("src"));
    assert!(changed.text(new, "target").unwrap().contains("page=diff"));
    assert!(changed.has("id", "src"));
}

/// Settings → Integrations → Capabilities: each choosable capability's
/// implementations and a default row; choosing writes `activeProviders`
/// in its layer (the choices stay the page's own rows).
#[tokio::test(flavor = "multi_thread")]
async fn the_capabilities_area_lists_the_implementations_and_choosing_one_is_a_config_write() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let caps = run_task(&fx, "capabilities", vec![]).await;
    assert!(!caps.rows.is_empty());
    // Every capability has its default row; pick a capability's non-default one.
    let defaults: Vec<&Row> = caps
        .rows
        .iter()
        .filter(|r| caps.flag(r, "is_default"))
        .collect();
    assert!(!defaults.is_empty());
    let choice = caps
        .rows
        .iter()
        .find(|r| caps.flag(r, "can_choose") && caps.text(r, "provider").as_deref() != Some("none"))
        .expect("a choosable implementation");
    let capability = caps.text(choice, "capability").unwrap();
    let provider = caps.text(choice, "provider").unwrap();
    let integrations = || vec![("area", SqlCell::Text("integrations".into()))];
    // A capability's rows share its key: the row pressed is the one
    // with the chosen provider (or the default one).
    let on = |action: &'static str, provider: Option<String>| {
        let (svc, capability, integrations) = (fx.svc.clone(), capability.clone(), integrations());
        async move {
            let runs = PageRuns::default();
            let (run_id, run) = runs
                .open(&svc, request("settings", integrations), "w")
                .await
                .unwrap();
            let caps = loader_of(&run, "capabilities");
            let row = caps
                .rows
                .iter()
                .find(|r| {
                    caps.text(r, "capability").as_deref() == Some(capability.as_str())
                        && match &provider {
                            Some(p) => caps.text(r, "provider").as_deref() == Some(p.as_str()),
                            None => caps.flag(r, "is_default"),
                        }
                })
                .expect("the row pressed")
                .clone();
            run_page_action(
                &svc,
                &runs,
                PageActionCall {
                    run_id,
                    node: Some("capabilities".into()),
                    action: action.into(),
                    row: Some(caps.columns.iter().cloned().zip(row).collect()),
                    input: None,
                    drop: None,
                    on_behalf_of: oxplow_domain::Actor::Human,
                    confirmed: true,
                },
            )
            .await
            .unwrap();
        }
    };
    on("use-project", Some(provider.clone())).await;
    let chosen = crate::config_service::read_config(&fx.svc.config).active_providers;
    assert_eq!(chosen.get(&capability), Some(&provider));
    // The default row resets it.
    assert!(defaults
        .iter()
        .any(|r| caps.text(r, "capability").as_deref() == Some(capability.as_str())));
    on("default-project", None).await;
    assert!(!crate::config_service::read_config(&fx.svc.config)
        .active_providers
        .contains_key(&capability));
    // Replaced components: each target has the default (in use until one
    // is chosen), oxplow's own, and each extension replacing it; choosing
    // writes `replacements`.
    let replaced = run_task(&fx, "replacements", vec![]).await;
    let row = replaced
        .rows
        .iter()
        .find(|r| {
            replaced.text(r, "whose").as_deref() == Some("oxplow") && replaced.flag(r, "can_choose")
        })
        .expect("a target oxplow's own can be chosen for");
    let target = replaced.text(row, "target").unwrap();
    let row = row.clone();
    let runs = PageRuns::default();
    let (run_id, run) = runs
        .open(&fx.svc, request("settings", integrations()), "w")
        .await
        .unwrap();
    assert!(loader_of(&run, "replacements").rows.contains(&row));
    run_page_action(
        &fx.svc,
        &runs,
        PageActionCall {
            run_id,
            node: Some("replacements".into()),
            action: "use".into(),
            row: Some(replaced.columns.iter().cloned().zip(row).collect()),
            input: None,
            drop: None,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        crate::config_service::read_config(&fx.svc.config)
            .replacements
            .get(&target)
            .map(String::as_str),
        Some("oxplow")
    );
}

/// The Context page: its Waiting section lists what the person collected,
/// and Clear — a whole-page action, bound to the page's `thread_id` —
/// empties it through the page-action path, leaving what was delivered.
#[tokio::test]
async fn the_context_pages_clear_empties_the_threads_basket() {
    let fx = services_with_effort().await;
    let thread = fx.thread.to_string();
    let add = |uri: &str| {
        let svc = fx.svc.clone();
        let input = serde_json::json!({ "thread": thread, "uri": uri, "label": uri });
        async move {
            svc.commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "oxplow.context.add",
                    input,
                    false,
                )
                .await
                .unwrap()
        }
    };
    add("oxplow:oxplow/page?page=tasks").await;
    add("oxplow:oxplow/page?page=board").await;
    let row_id = fx.thread.value();
    let by_thread = || vec![("thread_id", SqlCell::Int(row_id))];
    assert_eq!(run(&fx, "context-waiting", by_thread()).await.rows.len(), 2);

    press(&fx.svc, "context", by_thread(), None, "clear", None)
        .await
        .unwrap();
    assert!(run(&fx, "context-waiting", by_thread())
        .await
        .rows
        .is_empty());
}
