//! Foundation lenses that replaced core pages: Settings' Overview and Language
//! Servers areas (mounted in each area's slot: what they list, how they group
//! and search, what Ask puts in the agent's input, what the language-server
//! rows offer), the Comments inbox, the commit and uncommitted pages, the Alerts page, Settings → Agents and AI, the Board, and the wiki page.

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
    // Its control is on the Agents area's page.
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
    // Go to location opens the page at the comment: `#comment=cmt<id>`.
    assert!(open.text(row, "go_ref").unwrap().contains("comment=cmt"));
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

/// The Board: every work item in scope as a card in its state, with the
/// declared enum fields and the comment count; the scope narrows it.
#[tokio::test]
async fn the_board_lists_work_items_by_scope() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let thread = fx.thread.to_string();
    for (title, thread) in [
        ("On the thread", Some(thread.clone())),
        ("In the backlog", None),
    ] {
        let mut input = serde_json::json!({ "title": title });
        if let Some(t) = thread {
            input["thread"] = serde_json::json!(t);
        }
        fx.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                input,
                false,
            )
            .await
            .unwrap();
    }
    let params = |scope: &str| {
        vec![
            ("scope", SqlCell::Text(scope.into())),
            ("thread_id", SqlCell::Int(fx.thread.value())),
        ]
    };
    let thread_run = run_task(&fx, "board-cards", params("thread")).await;
    assert!(thread_run.has("title", "On the thread") && !thread_run.has("title", "In the backlog"));
    let backlog = run_task(&fx, "board-cards", params("backlog")).await;
    assert!(backlog.has("title", "In the backlog") && !backlog.has("title", "On the thread"));
    let all = run_task(&fx, "board-cards", params("all")).await;
    assert!(all.has("title", "On the thread") && all.has("title", "In the backlog"));
    let card = all.keyed("title", "On the thread");
    assert_eq!(all.text(card, "state").as_deref(), Some("todo"));
    assert!(all.flag(card, "not_done") && !all.flag(card, "not_todo"));

    // The page's row says which scope its part draws.
    let doc = run_task(
        &fx,
        "board",
        vec![
            ("show", SqlCell::Text("backlog".into())),
            ("thread_id", SqlCell::Int(fx.thread.value())),
        ],
    )
    .await;
    assert_eq!(doc.text(&doc.rows[0], "scope").as_deref(), Some("backlog"));
    let doc = run_task(
        &fx,
        "board",
        vec![
            ("show", SqlCell::Text("thread".into())),
            ("thread_id", SqlCell::Null(())),
        ],
    )
    .await;
    assert_eq!(doc.text(&doc.rows[0], "scope").as_deref(), Some("all"));
}

async fn run_task(
    fx: &crate::test_fixtures::TaskEffortFixture,
    lens: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let run = run_lens(
        &svc.sql,
        &svc.extension_catalog,
        svc.layout.project_dir.as_path(),
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

/// A wiki page: its body is the stored markdown (empty for a page not yet
/// written, so the first Save creates it), with when it was edited and
/// whether the files it cites have drifted.
#[tokio::test]
async fn the_wiki_page_reads_its_body_even_before_it_is_written() {
    let fx = services_with_effort().await;
    let slug = |s: &str| vec![("slug", SqlCell::Text(s.to_string()))];
    command(
        &fx,
        "oxplow.knowledge.write_page",
        serde_json::json!({ "slug": "otters", "title": "Otters", "body": "# Otters\n\nThey hold hands.\n" }),
    )
    .await;
    let body = run(&fx, "wiki-page-body", slug("otters")).await;
    assert_eq!(body.rows.len(), 1);
    assert!(body
        .text(&body.rows[0], "body")
        .unwrap()
        .contains("They hold hands."));
    assert_eq!(body.text(&body.rows[0], "slug").as_deref(), Some("otters"));

    let new = run(&fx, "wiki-page-body", slug("not-yet")).await;
    assert_eq!(new.rows.len(), 1);
    assert_eq!(new.text(&new.rows[0], "body").as_deref(), Some(""));
    assert_eq!(new.text(&new.rows[0], "slug").as_deref(), Some("not-yet"));

    let info = run(&fx, "wiki-page-info", slug("otters")).await;
    assert_eq!(
        info.text(&info.rows[0], "freshness").as_deref(),
        Some("fresh")
    );
    assert!(run(&fx, "wiki-page-info", slug("not-yet"))
        .await
        .rows
        .is_empty());
    assert!(run(&fx, "wiki-page-refs", slug("otters"))
        .await
        .rows
        .is_empty());
}

/// A work item's page: the parts read the item by its uri, its list's
/// features switch the gated buttons, and links show from both ends.
#[tokio::test]
async fn the_work_item_page_reads_the_item_its_links_and_its_comments() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let thread = fx.thread.to_string();
    for title in ["The item", "Its blocker"] {
        command_task(
            &fx,
            "oxplow.work_item.create",
            serde_json::json!({ "title": title, "body": "Some words.", "thread": thread }),
        )
        .await;
    }
    let all = run_task(
        &fx,
        "board-cards",
        vec![
            ("scope", SqlCell::Text("all".into())),
            ("thread_id", SqlCell::Int(fx.thread.value())),
        ],
    )
    .await;
    let uri = all.text(all.keyed("title", "The item"), "uri").unwrap();
    let blocker = all.text(all.keyed("title", "Its blocker"), "uri").unwrap();
    command_task(
        &fx,
        "oxplow.work_item.link",
        serde_json::json!({ "uri": uri, "target": blocker, "link_type": "blocks" }),
    )
    .await;
    command_task(
        &fx,
        "oxplow.work_item.comment",
        serde_json::json!({ "uri": uri, "body": "A note." }),
    )
    .await;
    // The page's row names its tab: the item's title.
    let page = run_task(&fx, "work-item", vec![("uri", SqlCell::Text(uri.clone()))]).await;
    assert_eq!(
        page.text(&page.rows[0], "title").as_deref(),
        Some("The item")
    );
    let item = |lens: &'static str, uri: &str| {
        let mut params = vec![("uri", SqlCell::Text(uri.to_string()))];
        if lens == "work-item-info" {
            params.push(("thread_id", SqlCell::Int(fx.thread.value())));
        }
        let fx = &fx;
        async move { run_task(fx, lens, params).await }
    };

    let state = item("work-item-state", &uri).await;
    assert_eq!(state.text(&state.rows[0], "to").as_deref(), Some("todo"));
    let fields = item("work-item-fields-form", &uri).await;
    assert_eq!(
        fields.text(&fields.rows[0], "title").as_deref(),
        Some("The item")
    );
    assert!(fields
        .text(&fields.rows[0], "native")
        .unwrap()
        .starts_with('{'));
    let body = item("work-item-body", &uri).await;
    assert_eq!(
        body.text(&body.rows[0], "body").as_deref(),
        Some("Some words.")
    );

    // On the viewed thread: it can be sent to the backlog and deleted, not brought.
    let info = item("work-item-info", &uri).await;
    assert!(info.flag(&info.rows[0], "can_send"));
    assert!(!info.flag(&info.rows[0], "can_bring"));
    assert!(info.flag(&info.rows[0], "can_delete"));

    // Both ends of the link show it; only the item's own end can be unlinked.
    let links = item("work-item-links", &uri).await;
    assert_eq!(links.rows.len(), 1);
    assert!(links.flag(&links.rows[0], "outbound"));
    let theirs = item("work-item-links", &blocker).await;
    assert_eq!(theirs.rows.len(), 1);
    assert!(!theirs.flag(&theirs.rows[0], "outbound"));
    assert_eq!(
        theirs.text(&theirs.rows[0], "other").as_deref(),
        Some(uri.as_str())
    );

    let comments = item("work-item-comments", &uri).await;
    assert_eq!(
        comments.text(&comments.rows[0], "body").as_deref(),
        Some("A note.")
    );
    assert!(item("work-item-activity", &uri).await.rows.is_empty());
    assert!(item(
        "work-item-state",
        "oxplow:oxplow/work_item?id=nope&provider=oxplow"
    )
    .await
    .rows
    .is_empty());
}

async fn command_task(
    fx: &crate::test_fixtures::TaskEffortFixture,
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

/// The Backlog: the items on no thread, in the list's order, an epic's
/// children indented under it, filtered by a search that reads the title
/// or any field; the gated actions follow what the list can do.
#[tokio::test(flavor = "multi_thread")]
async fn the_backlog_lists_items_on_no_thread_with_their_epics_and_search() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let thread = fx.thread.to_string();
    let mut epic = String::new();
    for (title, on_thread, parent) in [
        ("An epic", false, None),
        ("Child of the epic", false, Some("epic")),
        ("Plain item", false, None),
        ("On the thread", true, None),
    ] {
        let mut input = serde_json::json!({ "title": title });
        if on_thread {
            input["thread"] = serde_json::json!(thread);
        }
        if parent.is_some() {
            input["parent_uri"] = serde_json::json!(epic);
        }
        let out = command_task(&fx, "oxplow.work_item.create", input).await;
        if title == "An epic" {
            epic = out["uri"].as_str().unwrap().to_string();
        }
    }
    let params = |q: &str| {
        vec![
            ("q", SqlCell::Text(q.into())),
            ("thread_id", SqlCell::Int(fx.thread.value())),
        ]
    };
    let all = run_task(&fx, "backlog-list", params("")).await;
    assert!(!all.has("title", "On the thread"));
    let titles: Vec<String> = all
        .rows
        .iter()
        .map(|r| all.text(r, "title").unwrap())
        .collect();
    assert_eq!(titles, ["An epic", "Child of the epic", "Plain item"]);
    let child = all.keyed("title", "Child of the epic");
    let at = all.columns.iter().position(|c| c == "depth").unwrap();
    assert_eq!(child[at], SqlCell::Int(1));
    assert!(
        all.flag(child, "can_order")
            && all.flag(child, "can_delete")
            && all.flag(child, "can_bring")
    );
    assert!(all.flag(child, "can_prioritize"));
    assert!(all
        .text(all.keyed("title", "An epic"), "add_child")
        .unwrap()
        .contains("page=new-task"));
    assert_eq!(all.text(child, "thread_ref"), Some(thread.clone()));

    let found = run_task(&fx, "backlog-list", params("plain")).await;
    assert_eq!(found.rows.len(), 1);
    // A field's value is searchable too (priority defaults to medium).
    assert_eq!(
        run_task(&fx, "backlog-list", params("medium"))
            .await
            .rows
            .len(),
        3
    );

    // Both parts of the page run; the quick-add form starts a Ready item.
    let page = run_task(&fx, "backlog", params("")).await;
    assert!(page.rows.is_empty() || !page.columns.is_empty());
    assert_eq!(
        run_task(&fx, "backlog-add", vec![])
            .await
            .text(&run_task(&fx, "backlog-add", vec![]).await.rows[0], "state")
            .as_deref(),
        Some("todo")
    );
}

/// The Tasks page: the items in scope by state with epics nested, the
/// priority filter keeping an epic whose child matches, the last five
/// done, and the summary's counts.
#[tokio::test(flavor = "multi_thread")]
async fn the_tasks_page_lists_its_scope_by_state_with_epics_and_a_summary() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let thread = fx.thread.to_string();
    let mut epic = String::new();
    for (title, parent, priority) in [
        ("An epic", false, "low"),
        ("Urgent child", true, "urgent"),
        ("Plain child", true, "low"),
        ("Loose item", false, "low"),
    ] {
        let mut input = serde_json::json!({ "title": title, "thread": thread, "native": { "priority": priority } });
        if parent {
            input["parent_uri"] = serde_json::json!(epic);
        }
        let out = command_task(&fx, "oxplow.work_item.create", input).await;
        if title == "An epic" {
            epic = out["uri"].as_str().unwrap().to_string();
        }
    }
    command_task(
        &fx,
        "oxplow.work_item.create",
        serde_json::json!({ "title": "Backlogged" }),
    )
    .await;
    for i in 0..7 {
        let made = command_task(
            &fx,
            "oxplow.work_item.create",
            serde_json::json!({ "title": format!("Finished {i}"), "thread": thread, "state": "done" }),
        )
        .await;
        assert!(made["uri"].is_string());
    }
    let list = |priority: &str| {
        run_task(
            &fx,
            "tasks-list",
            vec![
                ("scope", SqlCell::Text("thread".into())),
                ("priority", SqlCell::Text(priority.into())),
                ("thread_id", SqlCell::Int(fx.thread.value())),
                ("stream_id", SqlCell::Int(1)),
            ],
        )
    };
    let all = list("any").await;
    assert!(!all.has("title", "Backlogged"));
    let finished = all
        .rows
        .iter()
        .filter(|r| all.text(r, "title").unwrap().starts_with("Finished"))
        .count();
    assert_eq!(finished, 5, "the last five done");
    let at = all.columns.iter().position(|c| c == "depth").unwrap();
    assert_eq!(all.keyed("title", "Plain child")[at], SqlCell::Int(1));
    assert_eq!(all.keyed("title", "An epic")[at], SqlCell::Int(0));

    let urgent = list("urgent").await;
    let titles: Vec<String> = urgent
        .rows
        .iter()
        .map(|r| urgent.text(r, "title").unwrap())
        .collect();
    assert_eq!(
        titles,
        ["An epic", "Urgent child"],
        "the epic stays for its matching child"
    );

    let doc = run_task(&fx, "tasks", vec![("show", SqlCell::Text("stream".into()))]).await;
    assert_eq!(doc.text(&doc.rows[0], "scope").as_deref(), Some("stream"));

    let summary = run_task(
        &fx,
        "tasks-summary",
        vec![
            ("scope", SqlCell::Text("thread".into())),
            ("thread_id", SqlCell::Int(fx.thread.value())),
            ("stream_id", SqlCell::Int(1)),
        ],
    )
    .await;
    let n = |c: &str| summary.rows[0][summary.columns.iter().position(|x| x == c).unwrap()].clone();
    assert_eq!(n("ready"), SqlCell::Int(4));
    assert_eq!(n("done"), SqlCell::Int(7));
    let backlog = run_task(&fx, "tasks-backlog", vec![]).await;
    assert_eq!(backlog.rows[0][0], SqlCell::Int(1));
}

/// The Changes page: an effort's page frames its own bracket (to the
/// working tree while it is open), the pickers override it with a range,
/// and the parts that belong to an effort have it to read.
#[tokio::test(flavor = "multi_thread")]
async fn the_changes_page_frames_an_effort_and_a_picked_range() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let effort = fx.fx.effort;
    let uri = format!("oxplow:oxplow/effort?id=eff{}", effort.value());
    let page = |extra: Vec<(&'static str, SqlCell)>| {
        let mut params = vec![
            ("uri", SqlCell::Text(uri.clone())),
            ("stream_id", SqlCell::Int(1)),
        ];
        params.extend(extra);
        run_task(&fx, "changes", params)
    };
    let own = page(vec![]).await;
    assert_eq!(own.rows.len(), 1, "{:?}", own.rows);
    let row = &own.rows[0];
    assert_eq!(own.text(row, "subject").as_deref(), Some("effort"));
    assert_eq!(own.text(row, "end_rev").as_deref(), Some("working"));
    assert!(own.flag(row, "in_progress"));
    assert_eq!(
        own.text(row, "effort").as_deref(),
        Some(format!("eff{}", effort.value()).as_str())
    );

    // A picked end turns it into a range, leaving the effort.
    let picked = page(vec![
        ("start", SqlCell::Text(String::new())),
        ("end", SqlCell::Text("working".into())),
    ])
    .await;
    assert_eq!(
        picked.text(&picked.rows[0], "subject").as_deref(),
        Some("range")
    );
    assert_eq!(
        picked.rows[0][picked
            .columns
            .iter()
            .position(|c| c == "effort_id")
            .unwrap()],
        SqlCell::Null(())
    );

    // The route page: no URI, no end - nothing to compare yet.
    let route = run_task(&fx, "changes", vec![("stream_id", SqlCell::Int(1))]).await;
    assert!(route.rows.is_empty());

    // Its parts.
    let effort_part = run_task(
        &fx,
        "changes-effort",
        vec![
            ("effort_id", SqlCell::Int(effort.value())),
            ("effort", SqlCell::Text(format!("eff{}", effort.value()))),
        ],
    )
    .await;
    assert_eq!(
        effort_part.text(&effort_part.rows[0], "state").as_deref(),
        Some("Open")
    );
    let header = run_task(
        &fx,
        "changes-header",
        vec![
            ("subject", SqlCell::Text("effort".into())),
            ("start_rev", SqlCell::Text(String::new())),
            ("end_rev", SqlCell::Text("working".into())),
            ("work_item", SqlCell::Null(())),
            ("in_progress", SqlCell::Int(1)),
        ],
    )
    .await;
    assert_eq!(
        header.text(&header.rows[0], "from_label").as_deref(),
        Some("(initial)")
    );
    assert_eq!(
        header.text(&header.rows[0], "to_label").as_deref(),
        Some("working tree")
    );
    assert!(header
        .text(&header.rows[0], "note")
        .unwrap()
        .contains("in progress"));
    for lens in ["changes-starts", "changes-ends"] {
        let options = run_task(&fx, lens, vec![("stream_id", SqlCell::Int(1))]).await;
        assert!(!options.rows.is_empty(), "{lens}");
    }
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

/// A dashboard page: its title, a metric added from the catalog with only
/// its key (the tile pins that metric's captures), then its tile rows,
/// resized and removed through the tiles' own actions.
#[tokio::test(flavor = "multi_thread")]
async fn the_dashboard_page_adds_a_metric_tile_and_edits_its_tiles() {
    use crate::lens_actions::{run_lens_action, LensActionCall};
    let fx = services_with_effort().await;
    let id = command(
        &fx,
        "oxplow.dashboard.create",
        serde_json::json!({ "title": "Health" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let by_id = vec![("id", SqlCell::Text(id.clone()))];
    // The page's row hands its parts the dashboard's id from the URI.
    let page = run(
        &fx,
        "dashboard",
        vec![(
            "uri",
            SqlCell::Text(format!("oxplow:oxplow/dashboard?id={id}")),
        )],
    )
    .await;
    assert_eq!(page.text(&page.rows[0], "id").as_deref(), Some(id.as_str()));
    assert_eq!(page.text(&page.rows[0], "title").as_deref(), Some("Health"));

    let title = run(&fx, "dashboard-title", by_id.clone()).await;
    assert_eq!(
        title.text(&title.rows[0], "title").as_deref(),
        Some("Health")
    );
    let metrics = run(&fx, "dashboard-metrics", vec![]).await;
    assert!(!metrics.rows.is_empty(), "the catalog lists metrics");
    let metric = metrics.text(&metrics.rows[0], "value").unwrap();
    let act = |lens: &str, action: &str, row, input| LensActionCall {
        lens_id: format!("oxplow-foundation/{lens}"),
        action_id: action.into(),
        params: std::collections::BTreeMap::from([("id".to_string(), SqlCell::Text(id.clone()))]),
        row,
        input,
        drop: None,
        on_behalf_of: oxplow_domain::Actor::Human,
        confirmed: false,
    };
    let root = fx._dir.path();
    let ctx = LensContext::default();
    run_lens_action(
        &fx.svc,
        root,
        act(
            "dashboard-title",
            "add-metric",
            None,
            Some(serde_json::json!({ "metric": metric })),
        ),
        &ctx,
    )
    .await
    .unwrap();
    let tiles = run(&fx, "dashboard-tiles", by_id.clone()).await;
    assert_eq!(tiles.rows.len(), 1);
    let row = &tiles.rows[0];
    assert_eq!(tiles.text(row, "display").as_deref(), Some("metric"));
    assert_eq!(tiles.text(row, "metric").as_deref(), Some(metric.as_str()));
    assert_eq!(tiles.text(row, "viz").as_deref(), Some("line"));
    let cells = |r: &Vec<SqlCell>| {
        tiles
            .columns
            .iter()
            .cloned()
            .zip(r.clone())
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    run_lens_action(
        &fx.svc,
        root,
        act("dashboard-tiles", "wide", Some(cells(row)), None),
        &ctx,
    )
    .await
    .unwrap();
    run_lens_action(
        &fx.svc,
        root,
        act("dashboard-tiles", "number", Some(cells(row)), None),
        &ctx,
    )
    .await
    .unwrap();
    let tiles = run(&fx, "dashboard-tiles", by_id.clone()).await;
    assert_eq!(tiles.text(&tiles.rows[0], "size").as_deref(), Some("wide"));
    assert_eq!(tiles.text(&tiles.rows[0], "viz").as_deref(), Some("number"));
    run_lens_action(
        &fx.svc,
        root,
        act(
            "dashboard-tiles",
            "remove",
            Some(cells(&tiles.rows[0])),
            None,
        ),
        &ctx,
    )
    .await
    .unwrap();
    assert!(run(&fx, "dashboard-tiles", by_id).await.rows.is_empty());
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

/// A page's tab is named by the one row of its lens (`label: {{row.title}}`):
/// a thread's and a stream's settings, by their titles.
#[tokio::test(flavor = "multi_thread")]
async fn the_settings_pages_row_names_the_thread_and_the_stream() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let thread = run_task(
        &fx,
        "thread-settings",
        vec![("thread", SqlCell::Text(format!("thr{}", fx.thread.value())))],
    )
    .await;
    assert_eq!(thread.rows.len(), 1, "{:?}", thread.rows);
    assert!(thread
        .text(&thread.rows[0], "title")
        .is_some_and(|t| !t.is_empty()));
    let stream = run_task(
        &fx,
        "stream-settings",
        vec![("stream", SqlCell::Text("str1".into()))],
    )
    .await;
    assert_eq!(stream.rows.len(), 1, "{:?}", stream.rows);
}

/// Settings → Data: the entities (counted when one is opened), the
/// collectors, and each effect's backfill (what it would react to).
#[tokio::test(flavor = "multi_thread")]
async fn the_data_area_lists_entities_collectors_and_what_a_backfill_would_do() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let entities = run_task(&fx, "data-entities", vec![]).await;
    assert!(
        entities.has("name", "v_work_item"),
        "{:?}",
        entities.rows.len()
    );
    // Open, a model counts its rows: this fixture has a task.
    let rows = run_task(
        &fx,
        "data-entity-rows",
        vec![("name", SqlCell::Text("v_work_item".into()))],
    )
    .await;
    assert!(
        rows.text(&rows.rows[0], "rows")
            .is_some_and(|t| t.ends_with(" rows")),
        "{:?}",
        rows.rows
    );
    let sources = run_task(&fx, "data-sources", vec![]).await;
    assert!(sources.columns.contains(&"collector".to_string()));
    let effects = run_task(&fx, "data-effects", vec![]).await;
    if let Some(first) = effects.rows.first() {
        let effect = effects.text(first, "effect").unwrap();
        let plan = run_task(
            &fx,
            "data-effect-backfill",
            vec![("effect", SqlCell::Text(effect))],
        )
        .await;
        assert_eq!(plan.rows.len(), 1, "{:?}", plan.rows);
        assert!(plan.text(&plan.rows[0], "what").is_some());
    }
    // An effect nothing declares has no plan.
    let none = run_task(
        &fx,
        "data-effect-backfill",
        vec![("effect", SqlCell::Text("nope/none".into()))],
    )
    .await;
    assert!(none.rows.is_empty());
}
