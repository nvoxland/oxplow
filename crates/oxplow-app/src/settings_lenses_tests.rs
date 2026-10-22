//! Foundation lenses that replaced core pages: Settings' Overview and Language
//! Servers areas (mounted in each area's slot: what they list, how they group
//! and search, what Ask puts in the agent's input, what the language-server
//! rows offer), the Comments inbox, the commit and uncommitted pages, the Alerts page, and Settings → Agents and AI.

use oxplow_config::LspServerConfig;
use oxplow_db::SqlCell;

use crate::extensions::{run_lens, LensContext};
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
    lens: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let run = run_lens(
        &svc.sql,
        &svc.extension_catalog,
        fx._dir.path(),
        &format!("oxplow-foundation/{lens}"),
        params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        &LensContext::default(),
    )
    .await
    .unwrap();
    Run {
        columns: run.result.columns,
        rows: run.result.rows,
    }
}

fn q(text: &str) -> Vec<(&str, SqlCell)> {
    vec![("q", SqlCell::Text(text.to_string()))]
}

/// Every setting is a row under its group, with where its value comes from
/// and the request Ask puts in the agent's input.
#[tokio::test]
async fn every_setting_is_listed_by_group_with_its_source_and_ask() {
    let fx = services_with_effort().await;
    let all = run(&fx, "settings-values", q("")).await;
    assert!(!all.rows.is_empty());

    let agents = all.keyed("key", "agents");
    assert_eq!(all.text(agents, "grp").as_deref(), Some("Agents"));
    // Only the person sets it, and its control is on the Agents area's page.
    assert_eq!(all.text(agents, "who").as_deref(), Some("yours to set"));
    assert_eq!(
        all.text(agents, "control_page").as_deref(),
        Some("oxplow:oxplow/page?area=agents&page=settings")
    );

    let retention = all.keyed("key", "snapshotRetentionDays");
    assert_eq!(all.text(retention, "grp").as_deref(), Some("Snapshots"));
    assert_eq!(all.text(retention, "source").as_deref(), Some("default"));
    let ask = all.text(retention, "ask").unwrap();
    assert!(
        ask.starts_with("Change the setting `snapshotRetentionDays` — "),
        "{ask}"
    );
    assert!(ask.contains("(the default)"), "{ask}");
    assert!(ask.contains("oxplow.config.set"), "{ask}");

    // Groups come in a fixed order: Agents before Snapshots.
    let groups: Vec<String> = all
        .rows
        .iter()
        .map(|r| all.text(r, "grp").unwrap())
        .collect();
    let first_snapshots = groups.iter().position(|g| g == "Snapshots").unwrap();
    let first_agents = groups.iter().position(|g| g == "Agents").unwrap();
    assert!(first_agents < first_snapshots);
}

/// A search matches a setting's key, its doc or its whole value.
#[tokio::test]
async fn the_search_narrows_by_key_doc_or_value() {
    let fx = services_with_effort().await;
    let found = run(&fx, "settings-values", q("snapshotretention")).await;
    assert!(found.has("key", "snapshotRetentionDays"));
    assert!(!found.has("key", "agents"));
    let none = run(&fx, "settings-values", q("no such setting anywhere")).await;
    assert!(none.rows.is_empty());
    // No search (NULL) lists all.
    let all = run(&fx, "settings-values", vec![("q", SqlCell::Null(()))]).await;
    assert!(all.rows.len() > found.rows.len());
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

fn threads_params(stream: i64, days: &str) -> Vec<(&'static str, SqlCell)> {
    vec![
        ("stream_id", SqlCell::Int(stream)),
        ("days", SqlCell::Text(days.into())),
    ]
}

/// The inbox lists open threads grouped by what they're anchored to; a
/// resolved one only when the filter reaches back to it; a picked thread
/// reads as its messages in order.
#[tokio::test]
async fn the_comments_inbox_lists_threads_by_target_and_a_picked_one_reads_in_order() {
    let fx = services_with_effort().await;
    let stream = fx.svc.streams.ensure_primary().await.unwrap().id;
    let add = |quote: &'static str, intent: &'static str| {
        command(
            &fx,
            "oxplow.knowledge.add_comment",
            serde_json::json!({
                "stream": stream.to_string(),
                "target": "oxplow:oxplow/wiki?slug=a-page",
                "quote": quote,
                "intent": intent,
                "body": "first",
            }),
        )
    };
    let first = add("one", "note").await;
    let second = add("two", "followup").await;
    let id = |c: &serde_json::Value| c["comment"]["id"].as_str().unwrap().to_string();
    let (first, second) = (id(&first), id(&second));

    let open = run(&fx, "comments-threads", threads_params(stream.value(), "0")).await;
    assert_eq!(open.rows.len(), 2);
    assert!(open.has("quote", "one") && open.has("quote", "two"));
    let row = open.keyed("quote", "two");
    assert_eq!(open.text(row, "kind").as_deref(), Some("follow-up"));
    assert_eq!(
        open.text(row, "target_label").as_deref(),
        Some("wiki/a-page")
    );
    assert_eq!(open.text(row, "go").as_deref(), Some("Go to location"));
    assert_eq!(open.text(row, "latest").as_deref(), Some("user: first"));
    assert!(open.flag(row, "is_open") && open.flag(row, "is_followup"));

    // Resolving hides it until the filter reaches back to it.
    command(
        &fx,
        "oxplow.knowledge.update_comment",
        serde_json::json!({ "comment": first, "status": "resolved" }),
    )
    .await;
    let after = run(&fx, "comments-threads", threads_params(stream.value(), "0")).await;
    assert_eq!(after.rows.len(), 1);
    let week = run(&fx, "comments-threads", threads_params(stream.value(), "7")).await;
    assert_eq!(week.rows.len(), 2);
    let resolved = week.keyed("quote", "one");
    assert_eq!(week.text(resolved, "state").as_deref(), Some("resolved"));
    assert!(week.flag(resolved, "is_resolved"));

    // A picked thread: its messages in the order they were written.
    command(
        &fx,
        "oxplow.knowledge.reply_comment",
        serde_json::json!({ "comment": second, "body": "second" }),
    )
    .await;
    let conversation = run(
        &fx,
        "comments-conversation",
        vec![("comment", SqlCell::Text(second.clone()))],
    )
    .await;
    let bodies: Vec<String> = conversation
        .rows
        .iter()
        .map(|r| conversation.text(r, "body").unwrap())
        .collect();
    assert_eq!(bodies, vec!["first", "second"]);
    // Nothing picked, nothing shown.
    let none = run(
        &fx,
        "comments-conversation",
        vec![("comment", SqlCell::Null(()))],
    )
    .await;
    assert!(none.rows.is_empty());
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
            vec![
                ("change_id", change.clone()),
                ("base_revision", at("base_revision")),
                ("head_revision", at("head_revision")),
            ],
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

/// Alerts: a reported failure lists red until it is seen, then muted, and
/// leaves the list once dismissed; its expansion carries how it ended and the
/// event page its output is on. Every part of the page runs.
#[tokio::test]
async fn the_alerts_page_lists_failures_and_each_part_runs() {
    let fx = services_with_effort().await;
    command(
        &fx,
        crate::commands::ui::REPORT_ERROR,
        serde_json::json!({
            "label": "Merge bugfixes into current",
            "command": "git merge bugfixes",
            "message": "conflict",
            "exit_code": 1,
            "stderr": "CONFLICT in a.txt",
        }),
    )
    .await;

    let ops = run(&fx, "alerts-operations", vec![]).await;
    assert_eq!(ops.rows.len(), 1);
    let row = &ops.rows[0];
    assert_eq!(
        ops.text(row, "label").as_deref(),
        Some("Merge bugfixes into current")
    );
    assert_eq!(ops.text(row, "tone").as_deref(), Some("danger"));
    assert!(ops.flag(row, "unseen"));
    let event = ops.text(row, "event_id").unwrap();

    let detail = run(
        &fx,
        "alerts-operation",
        vec![("event_id", SqlCell::Text(event.clone()))],
    )
    .await;
    assert_eq!(detail.rows.len(), 1);
    let d = &detail.rows[0];
    assert_eq!(
        detail.text(d, "command").as_deref(),
        Some("git merge bugfixes")
    );
    assert_eq!(
        detail.text(d, "stderr").as_deref(),
        Some("CONFLICT in a.txt")
    );
    assert_eq!(detail.text(d, "output_note").as_deref(), Some(""));

    // A failure that captured nothing says so.
    command(
        &fx,
        crate::commands::ui::REPORT_ERROR,
        serde_json::json!({ "label": "Quiet", "message": "no output" }),
    )
    .await;
    let all = run(&fx, "alerts-operations", vec![]).await;
    let quiet = all
        .rows
        .iter()
        .find(|r| all.text(r, "label").as_deref() == Some("Quiet"))
        .unwrap();
    let quiet_detail = run(
        &fx,
        "alerts-operation",
        vec![(
            "event_id",
            SqlCell::Text(all.text(quiet, "event_id").unwrap()),
        )],
    )
    .await;
    assert_eq!(
        quiet_detail
            .text(&quiet_detail.rows[0], "output_note")
            .as_deref(),
        Some("No output was captured.")
    );

    command(
        &fx,
        "oxplow.op_error.mark_seen",
        serde_json::json!({ "event": event }),
    )
    .await;
    let seen = run(&fx, "alerts-operations", vec![]).await;
    let merged = seen.keyed("label", "Merge bugfixes into current");
    assert_eq!(seen.text(merged, "tone").as_deref(), Some("muted"));
    command(
        &fx,
        "oxplow.op_error.dismiss",
        serde_json::json!({ "event": event }),
    )
    .await;
    let left = run(&fx, "alerts-operations", vec![]).await;
    assert!(!left.has("label", "Merge bugfixes into current"));
    assert!(left.has("label", "Quiet"));

    // The other parts run over a quiet project and list nothing.
    for lens in ["alerts-hints", "alerts-deliveries", "alerts-reactions"] {
        assert!(run(&fx, lens, vec![]).await.rows.is_empty(), "{lens}");
    }
    // Extensions with problems: none in a clean fixture.
    let ext = run(&fx, "alerts-extensions", vec![]).await;
    assert!(ext.rows.iter().all(|r| ext.text(r, "settings").is_some()));
}

/// Settings → Agents: the agents in priority order with the lists a row's
/// actions hand `set_agents`; the declared settings of the enabled ones; what
/// new threads start with.
#[tokio::test]
async fn the_agents_area_hands_set_agents_each_new_order() {
    let fx = services_with_effort().await;
    let ids: Vec<String> = fx
        .svc
        .harnesses
        .all()
        .iter()
        .map(|h| h.id().to_string())
        .collect();
    assert!(ids.len() >= 3, "{ids:?}");

    let list = run(&fx, "agents-list", vec![]).await;
    assert_eq!(list.rows.len(), ids.len());
    let second = &list.rows[1];
    let second_id = list.text(second, "id").unwrap();
    let first_id = list.text(&list.rows[0], "id").unwrap();
    assert!(list.flag(second, "can_up") && list.flag(second, "can_disable"));
    assert!(!list.flag(&list.rows[0], "can_up"));
    // Move Up swaps the second with the first; the list is a JSON array.
    let up: Vec<String> = serde_json::from_str(&list.text(second, "moved_up").unwrap()).unwrap();
    assert_eq!(&up[..2], [second_id.clone(), first_id.clone()]);
    command(
        &fx,
        "oxplow.agent.set_agents",
        serde_json::json!({ "agents": up }),
    )
    .await;
    let after = run(&fx, "agents-list", vec![]).await;
    assert_eq!(after.text(&after.rows[0], "id"), Some(second_id.clone()));

    // Disable drops it from the order; Enable puts it back last.
    let without: Vec<String> =
        serde_json::from_str(&after.text(&after.rows[0], "without_it").unwrap()).unwrap();
    assert!(!without.contains(&second_id));
    command(
        &fx,
        "oxplow.agent.set_agents",
        serde_json::json!({ "agents": without }),
    )
    .await;
    let off = run(&fx, "agents-list", vec![]).await;
    let dropped = off.keyed("id", &second_id);
    assert!(off.flag(dropped, "can_enable") && !off.flag(dropped, "can_disable"));
    let with_it: Vec<String> =
        serde_json::from_str(&off.text(dropped, "with_it").unwrap()).unwrap();
    assert_eq!(with_it.last(), Some(&second_id));

    // What new threads start with: the picker, until a choice is made.
    let start = run(&fx, "agents-new-thread", vec![]).await;
    assert!(start.flag(start.keyed("value", "ask"), "current"));
    // `config.set` asks first; a lens action's click is the person's yes.
    fx.svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            "oxplow.config.set",
            serde_json::json!({ "key": "newThreadSession", "value": "none", "layer": "personal" }),
            true,
        )
        .await
        .unwrap();
    let chosen = run(&fx, "agents-new-thread", vec![]).await;
    assert!(chosen.flag(chosen.keyed("value", "none"), "current"));
    assert!(chosen.flag(chosen.keyed("value", "ask"), "can_ask"));

    // The prompt addition prefills the form with what is saved.
    command(
        &fx,
        "oxplow.agent.set_prompt_append",
        serde_json::json!({ "text": "Prefer TDD." }),
    )
    .await;
    let prompt = run(&fx, "agents-prompt", vec![]).await;
    assert_eq!(
        prompt.text(&prompt.rows[0], "text").as_deref(),
        Some("Prefer TDD.")
    );
}

/// Settings → AI: a provider lists with its kind and whether it has a key
/// (never the key), a role bound to it shows its binding and hands
/// `set_role` its current values, and the kinds and providers are the
/// choices the forms offer.
#[tokio::test]
async fn the_ai_area_lists_providers_roles_and_choices() {
    let fx = services_with_effort().await;
    let kinds = run(&fx, "ai-provider-kinds", vec![]).await;
    assert!(kinds.has("value", "openrouter"), "{:?}", kinds.rows);

    command(
        &fx,
        "oxplow.ai.set_provider",
        serde_json::json!({ "id": "or", "kind": "openrouter", "key": "sk-secret-123" }),
    )
    .await;
    let providers = run(&fx, "ai-providers", vec![]).await;
    let or = providers.keyed("id", "or");
    assert_eq!(providers.text(or, "key").as_deref(), Some("Key saved"));
    assert!(providers
        .rows
        .iter()
        .all(|r| !format!("{r:?}").contains("sk-secret-123")));
    assert!(run(&fx, "ai-provider-choices", vec![])
        .await
        .has("value", "or"));

    command(
        &fx,
        "oxplow.ai.set_role",
        serde_json::json!({ "role": "summarize", "provider": "or", "model": "m1", "restrict": ["tools"] }),
    )
    .await;
    let roles = run(&fx, "ai-roles", vec![]).await;
    let summarize = roles.keyed("role", "summarize");
    assert_eq!(roles.text(summarize, "bound").as_deref(), Some("or · m1"));
    assert_eq!(
        roles.text(summarize, "restrict").as_deref(),
        Some("[\"tools\"]")
    );
    assert!(roles.flag(summarize, "can_assign") && roles.flag(summarize, "can_unassign"));
    // A role bound to nothing says what it falls back to or that it is free.
    let decide = roles.keyed("role", "decide");
    assert!(!roles.flag(decide, "can_unassign"));
    // The test action starts from the model a role already uses on it.
    let again = run(&fx, "ai-providers", vec![]).await;
    assert_eq!(
        again.text(again.keyed("id", "or"), "test_model").as_deref(),
        Some("m1")
    );
    assert!(run(&fx, "ai-calls", vec![]).await.rows.is_empty());
}
