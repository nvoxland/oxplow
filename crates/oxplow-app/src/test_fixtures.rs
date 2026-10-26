//! Shared test setup for tests that need a real `Services`.

#![cfg(test)]

use oxplow_tasks::work_item_ref;
use std::path::Path;
use std::sync::Arc;

use oxplow_db::EffortStore as _;
use oxplow_domain::{EffortId, ThreadId};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore as _;
use oxplow_tasks::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};

/// A git repo with one empty commit (stream setup refuses non-git dirs).
pub fn init_git_repo(dir: &Path) {
    let repo = git2::Repository::init(dir).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "test").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    let sig = repo.signature().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
}

/// Stage everything in `dir` and commit it on HEAD; the new sha.
pub fn commit_all(dir: &Path, message: &str) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    idx.write().unwrap();
    let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
    let sig = repo.signature().unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .unwrap()
        .to_string()
}

/// Commit what's staged in `dir` on HEAD, as it is; the new sha.
pub fn commit_index(dir: &Path, message: &str) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let mut idx = repo.index().unwrap();
    let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
    let sig = repo.signature().unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .unwrap()
        .to_string()
}

/// Deploy every extension source the test wrote and publish the new
/// generation, and assert that nothing was held back. Fixtures activate once when built (as boot does);
/// a test that writes extension files after that calls this.
pub async fn activate(svc: &Arc<crate::Services>) {
    let problems = activate_with_problems(svc).await;
    assert!(
        problems.is_empty(),
        "the extensions activate without problems: {problems:?}"
    );
}

/// [`activate`] for a test that expects some extension to be held back or
/// rejected: what the activation recorded, by extension. Every source the
/// test wrote is deployed first (`extension_deploy::stage_sources`).
pub async fn activate_with_problems(
    svc: &Arc<crate::Services>,
) -> crate::vocabulary_reactor::VocabularyErrors {
    use oxplow_db::TxError;
    // A candidate overtaken by a change made while it was prepared (a
    // stop, an approval) is rejected whole; the generation owner takes
    // the next pass for the change that overtook it, so this does too.
    let mut attempts = 0;
    loop {
        attempts += 1;
        let changes = crate::extension_deploy::stage_sources(svc).await;
        match svc
            .extension_activation
            .activate_live(svc.clone(), &changes)
            .await
        {
            Ok(_) => return svc.extension_activation.problems(),
            // A rejected candidate: the previous generation stays, and why.
            Err(TxError::Aborted(problems)) if problems.contains_key("<host>") && attempts < 3 => {}
            Err(TxError::Aborted(problems)) => return problems,
            Err(error) => panic!("activation failed: {error:?}"),
        }
    }
}

/// Deploy and activate what the test wrote, then everything that is said
/// against extension `name`: the errors of what is live and what held the
/// new source back (a source that doesn't load never goes live).
pub async fn load_errors(svc: &Arc<crate::Services>, name: &str) -> String {
    let held = activate_with_problems(svc).await;
    let live = svc
        .extension_catalog
        .snapshot(&svc.layout.project_dir)
        .extensions()
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.errors.join("\n"))
        .unwrap_or_default();
    let held = held.get(name).map(|l| l.join("\n")).unwrap_or_default();
    format!("{live}\n{held}")
}

pub struct EffortFixture {
    pub svc: Arc<crate::Services>,
    /// Keep alive: the project directory.
    pub _dir: tempfile::TempDir,
    pub thread: ThreadId,
    /// The thread's agent session (the project's default agent).
    pub session: oxplow_domain::AgentSessionId,
    pub effort: EffortId,
}

/// An agent session (a Claude terminal) on `thread`, as setup — written to
/// the store, not run as a command, so it leaves no audit row behind.
pub async fn open_session(
    svc: &crate::Services,
    thread: ThreadId,
) -> oxplow_domain::AgentSessionId {
    use oxplow_domain::stores::AgentSessionStore as _;
    svc.agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
            thread, "claude",
        ))
        .await
        .unwrap()
        .id
}

/// In-memory services over a fresh repo with a primary stream, its first
/// thread and an open effort linked to nothing — an effort needs no work
/// item (`.context/work-tracking.md`), so a test that isn't about one
/// doesn't make one.
pub async fn services_with_effort() -> EffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    services_with_effort_in(dir).await
}

/// oxplow-foundation's build command, as its manifest quotes it: tests
/// that deploy its source swap it for a no-op (cargo built its output).
pub const FOUNDATION_BUILD: &str = "\"bun install --frozen-lockfile && bun run build\"";

/// [`services_with_effort`] while developing oxplow: oxplow's own
/// extensions run from their sources (`enableBundledExtensions: false`),
/// each a copy of the repo's folder in the project's
/// `.oxplow/extensions/`. Boot deploys nothing, so foundation is deployed
/// once booted, as `oxplow-dev extension deploy` does for a checkout.
pub async fn services_developing_oxplow() -> EffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    for b in crate::bundled_extensions::BUNDLED {
        let to = dir.path().join(".oxplow/extensions").join(b.name);
        for (rel, bytes) in b.files {
            let file = to.join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
    }
    // Its output is the one cargo built; a deploy here rebuilds nothing.
    let manifest = dir
        .path()
        .join(".oxplow/extensions/oxplow-foundation/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replacen(FOUNDATION_BUILD, "\"true\"", 1)).unwrap();
    std::fs::write(
        dir.path().join(".oxplow/project.yaml"),
        "enableBundledExtensions: false\n",
    )
    .unwrap();
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    crate::extension_deploy::deploy(&svc, "oxplow-foundation", true)
        .await
        .expect("foundation deploys from its source");
    with_effort(dir, svc).await
}

/// [`services_with_effort`] over the project `dir`, already set up.
pub async fn services_with_effort_in(dir: tempfile::TempDir) -> EffortFixture {
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    with_effort(dir, svc).await
}

/// A stream, a thread's session and an open effort over booted `svc`.
async fn with_effort(dir: tempfile::TempDir, svc: Arc<crate::Services>) -> EffortFixture {
    svc.streams.ensure_primary().await.unwrap();
    // Activation logged the capability choices; the policy has heard them
    // (they close open efforts) before the fixture opens one.
    svc.event_pump
        .settle(
            &[
                crate::effort_policy::NAME,
                crate::knowledge::KnowledgeSwitch::NAME,
            ],
            std::time::Duration::from_secs(5),
        )
        .await;
    let thread = ThreadId::new(1);
    let session = open_session(&svc, thread).await;
    let opened = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::effort::OPEN,
            serde_json::json!({ "thread": thread.to_string() }),
            false,
        )
        .await
        .unwrap();
    let effort = opened.result["effort"]
        .as_str()
        .and_then(|e| e.parse::<oxplow_domain::EffortId>().ok())
        .expect("effort.open names the effort");
    EffortFixture {
        svc,
        _dir: dir,
        thread,
        session,
        effort,
    }
}

/// [`EffortFixture`] whose effort is linked to an in-progress oxplow task
/// on the thread: for a test about tasks.
pub struct TaskEffortFixture {
    pub fx: EffortFixture,
    pub task: TaskId,
}

impl std::ops::Deref for TaskEffortFixture {
    type Target = EffortFixture;

    fn deref(&self) -> &EffortFixture {
        &self.fx
    }
}

/// In-memory services over a fresh repo with a primary stream, its first
/// thread, an in-progress task on it and an open effort linked to it.
pub async fn services_with_task_effort() -> TaskEffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.streams.ensure_primary().await.unwrap();
    // Activation logged the capability choices; the policy has heard them
    // (they close open efforts) before the fixture opens one.
    svc.event_pump
        .settle(
            &[
                crate::effort_policy::NAME,
                crate::knowledge::KnowledgeSwitch::NAME,
            ],
            std::time::Duration::from_secs(5),
        )
        .await;
    let thread = ThreadId::new(1);
    let session = open_session(&svc, thread).await;
    let now = oxplow_domain::Timestamp::now();
    let task = svc
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread),
            parent_id: None,
            title: "t".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        })
        .await
        .unwrap();
    restate_task(&svc, task).await;
    let effort = svc
        .effort_store
        .start(&work_item_ref(task), &thread, None)
        .await
        .unwrap()
        .id;
    TaskEffortFixture {
        fx: EffortFixture {
            svc,
            _dir: dir,
            thread,
            session,
            effort,
        },
        task,
    }
}

/// A task filed as a person through `oxplow.work_item.create` with
/// `input` (on the backlog unless it names a `thread`): its id.
pub async fn file_item(svc: &crate::Services, input: serde_json::Value) -> TaskId {
    let out = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::work_item::CREATE,
            input,
            false,
        )
        .await
        .unwrap();
    oxplow_tasks::task_of_work_item_ref(out.result["uri"].as_str().unwrap()).unwrap()
}

/// Restate task `task` in the work-item interface, as its list does after
/// a write: its `work_item.recorded`, logged and projected. For a test
/// that writes the task store directly rather than through the
/// `oxplow.work_item.*` commands.
pub async fn restate_task(svc: &crate::Services, task: TaskId) {
    let record = svc
        .db
        .read(move |tx| oxplow_tasks::record::record_tx(tx, task))
        .await
        .unwrap();
    svc.event_log_store
        .append(oxplow_tasks::provider::recorded(record))
        .await
        .unwrap();
    svc.event_pump.deliver_projections().await.unwrap();
}

/// A new thread on `stream`, made as a person through `oxplow.thread.create`.
pub async fn new_thread(
    svc: &crate::Services,
    stream: oxplow_domain::StreamId,
    title: &str,
) -> oxplow_domain::Thread {
    let out = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::thread::CREATE,
            serde_json::json!({
                "stream": stream.to_string(),
                "title": title,
            }),
            false,
        )
        .await
        .unwrap();
    serde_json::from_value(out.result).unwrap()
}

/// A tool hook's call as the hook route maps it (`AgentHarness::tool_use`
/// of the bearer's harness), for a test posting Claude Code's bodies;
/// `None` for any other hook.
pub fn claude_tool(
    kind: oxplow_domain::HookKind,
    body: &serde_json::Value,
) -> Option<oxplow_domain::agent::tool::ToolUse> {
    use oxplow_domain::HookKind;
    if !matches!(
        kind,
        HookKind::PreToolUse | HookKind::PostToolUse | HookKind::PermissionRequest
    ) {
        return None;
    }
    let claude =
        oxplow_harnesses::built_in("oxplow:claude-code", "claude", "Claude").expect("the built-in");
    futures::executor::block_on(claude.tool_use(body))
}

/// Keep the section of reference doc `doc` (repo-relative) between
/// `<!-- {name}:begin … -->` and `<!-- {name}:end -->` equal to
/// `generated`: `OXPLOW_BLESS=1` rewrites it, else a stale one fails.
pub fn generated_section(doc: &str, name: &str, generated: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(doc);
    let text = std::fs::read_to_string(&path).unwrap();
    let begin = text
        .find(&format!("<!-- {name}:begin"))
        .unwrap_or_else(|| panic!("{doc}: the {name}:begin marker"));
    let begin = begin + text[begin..].find("-->").unwrap() + 3;
    let end = text
        .find(&format!("<!-- {name}:end -->"))
        .unwrap_or_else(|| panic!("{doc}: the {name}:end marker"));
    let generated = format!("\n{generated}");
    if std::env::var_os("OXPLOW_BLESS").is_some() {
        std::fs::write(
            &path,
            format!("{}{generated}{}", &text[..begin], &text[end..]),
        )
        .unwrap();
        return;
    }
    assert_eq!(
        &text[begin..end],
        generated,
        "{doc}'s {name} table is stale; bless it with OXPLOW_BLESS=1"
    );
}

/// Start effect `id` (`<extension>/<effect>`) at the log's head now: it
/// reacts to what is logged from here on.
pub async fn start_effect(svc: &Arc<crate::Services>, id: &str) {
    crate::effects::start_at_head(&svc.db, id).await.unwrap();
}

/// Disable collector `extension`/`contribution` for `reason`, as three
/// failures in a row do; the `contribution.disabled` it logged.
pub async fn disable_collector(
    svc: &Arc<crate::Services>,
    extension: &str,
    contribution: &str,
    reason: &str,
) -> oxplow_domain::StoredEvent {
    let health = crate::contribution_health::ContributionHealth::new(
        svc.db.clone(),
        svc.event_log_store.vocabulary().clone(),
    );
    let key = crate::collector_runner::contribution_key(extension, contribution);
    health.enable(&key, "human").await.unwrap();
    health.disable(&key, reason).await.unwrap();
    svc.event_log_store
        .read_after(0, 10_000)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.envelope.event_type == "contribution.disabled")
        .expect("the disable is logged")
}

/// Deliver `event` to the effects, as the pump would.
pub async fn deliver(svc: &Arc<crate::Services>, event: &oxplow_domain::StoredEvent) {
    use crate::event_pump::AsyncEventConsumer as _;
    crate::effect_triggers::EffectTriggers::new(Arc::downgrade(svc))
        .handle(event)
        .await
        .unwrap();
}

/// The foundation's declared `metrics:` and `dimensions:`, parsed from the
/// bundled manifest — for a test that seeds the specs the way an activation
/// would, without a whole `Services`.
pub(crate) fn foundation_declarations() -> (
    Vec<oxplow_config::MetricEntry>,
    Vec<oxplow_config::DimensionEntry>,
) {
    let manifest: serde_yaml::Value = serde_yaml::from_str(include_str!(
        "../../../extensions/oxplow-foundation/extension.yaml"
    ))
    .expect("the foundation manifest parses");
    let block = |key: &str| {
        manifest
            .get(key)
            .cloned()
            .expect("the foundation declares it")
    };
    let metrics = oxplow_config::validate_metrics(
        Some(serde_yaml::from_value(block("metrics")).expect("metrics parse")),
        oxplow_config::CatalogLayer::Extension(oxplow_config::OXPLOW_NAMESPACE),
    )
    .expect("the foundation's metrics validate");
    let dimensions = oxplow_config::validate_dimensions(
        Some(serde_yaml::from_value(block("dimensions")).expect("dimensions parse")),
        oxplow_config::CatalogLayer::Extension(oxplow_config::OXPLOW_NAMESPACE),
    )
    .expect("the foundation's dimensions validate");
    (metrics, dimensions)
}

/// The specs the foundation's metrics resolve to, as an activation seeds them.
pub(crate) fn foundation_specs() -> Vec<oxplow_db::NewMetricSpec> {
    let (metrics, _) = foundation_declarations();
    oxplow_config::resolve_metrics(&[], &[], &[("oxplow-foundation".into(), metrics)], &[])
        .iter()
        .map(crate::metrics_service::spec_to_new_spec)
        .collect()
}

/// Only the specs over a measure's facts (what the producers feed), not the
/// entity metrics — the lifecycle and collection tests seed these.
pub(crate) fn foundation_producer_specs() -> Vec<oxplow_db::NewMetricSpec> {
    foundation_specs()
        .into_iter()
        .filter(|spec| spec.entity_json.is_none())
        .collect()
}

/// A harness that lists the conversations it is told to, or fails with
/// `answer`'s message; it launches nothing.
pub struct Listing {
    pub id: &'static str,
    pub answer: Result<Vec<oxplow_domain::agent::harness::NativeConversation>, String>,
}

#[async_trait::async_trait]
impl oxplow_domain::agent::harness::AgentHarness for Listing {
    fn id(&self) -> &str {
        self.id
    }
    fn title(&self) -> &str {
        self.id
    }
    async fn launch(
        &self,
        _input: &oxplow_domain::agent::harness::LaunchInput,
    ) -> Result<oxplow_domain::agent::harness::Launch, oxplow_domain::agent::harness::HarnessError>
    {
        Err(oxplow_domain::agent::harness::HarnessError::Unsupported {
            harness: self.id.into(),
            what: "launch".into(),
        })
    }
    async fn tool_use(
        &self,
        _body: &serde_json::Value,
    ) -> Option<oxplow_domain::agent::tool::ToolUse> {
        None
    }
    async fn render(
        &self,
        _answer: &oxplow_domain::agent::observe::HookAnswer,
    ) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn conversations(
        &self,
        _query: &oxplow_domain::agent::harness::ConversationQuery,
    ) -> Result<
        Vec<oxplow_domain::agent::harness::NativeConversation>,
        oxplow_domain::agent::harness::HarnessError,
    > {
        self.answer
            .clone()
            .map_err(oxplow_domain::agent::harness::HarnessError::Unavailable)
    }
}

/// A [`Listing`] harness that lists `found`.
pub fn listing_harness(
    id: &'static str,
    found: Vec<oxplow_domain::agent::harness::NativeConversation>,
) -> Arc<dyn oxplow_domain::agent::harness::AgentHarness> {
    Arc::new(Listing {
        id,
        answer: Ok(found),
    })
}

/// A saved conversation `id`, last active at minute `minute` of a fixed
/// hour, titled `about <id>`.
pub fn conversation(id: &str, minute: u32) -> oxplow_domain::agent::harness::NativeConversation {
    oxplow_domain::agent::harness::NativeConversation {
        id: id.into(),
        title: Some(format!("about {id}")),
        first_prompt: None,
        started_at: None,
        updated_at: Some(
            oxplow_domain::Timestamp::parse(&format!("2026-10-10T10:{minute:02}:00Z")).unwrap(),
        ),
        size_bytes: None,
        parent: None,
    }
}

/// The extensions that ship with oxplow, loaded as a project with none of
/// its own loads them (each from its shipped deployment, written once to a
/// store that lasts the test run).
pub fn shipped_extensions() -> Vec<crate::extensions::Extension> {
    static PROJECT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let project = PROJECT.get_or_init(|| tempfile::tempdir().unwrap());
    crate::extensions::load_extensions(project.path())
}

/// The page a lens became, run with `params`, and the rows of the loader
/// that carried the lens's query: `<extension>/<slug>` as a page of that
/// name (its loader named for the slug, else the one its first viz node
/// shows), or a loader named for the slug (`-` as `_`) in one of the
/// extension's other pages, where the lens was inlined as a part. Only the
/// params the page declares are given to it.
pub async fn run_as_page(
    svc: &crate::Services,
    lens: &str,
    params: &[(String, oxplow_db::SqlCell)],
) -> oxplow_db::SqlQueryResult {
    run_as_page_in(svc, lens, params, crate::extensions::ViewContext::default()).await
}

/// [`run_as_page`] from a viewer's context (`lens_context`).
pub async fn run_as_page_in(
    svc: &crate::Services,
    lens: &str,
    params: &[(String, oxplow_db::SqlCell)],
    ctx: crate::extensions::ViewContext,
) -> oxplow_db::SqlQueryResult {
    use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
    let (extension, slug) = lens.split_once('/').expect("<extension>/<slug>");
    // Lenses the conversion inlined into a page as a part named otherwise,
    // or renamed (`pages convert` prints both).
    const NAMED: &[(&str, &str, &str)] = &[
        ("oxplow-foundation/tasks-list", "tasks", "list"),
        ("oxplow-foundation/tasks-summary", "tasks", "summary"),
        ("oxplow-foundation/tasks-backlog", "tasks", "backlog"),
        ("oxplow-foundation/backlog-add", "backlog", "add"),
        ("oxplow-foundation/backlog-list", "backlog", "items"),
        ("oxplow-foundation/board-cards", "board", "board"),
        ("oxplow-foundation/context-waiting", "context", "waiting"),
        (
            "oxplow-foundation/context-delivered",
            "context",
            "delivered",
        ),
        ("oxplow-foundation/go-to-bookmarks", "go-to", "bookmarks"),
        ("oxplow-foundation/go-to-recent", "go-to", "recent"),
        ("oxplow-foundation/go-to-top", "go-to", "top"),
        ("oxplow-foundation/files-header", "files", "header"),
        ("oxplow-foundation/files-tree", "files", "tree"),
        (
            "oxplow-foundation/git-dashboard-upstream",
            "git-dashboard",
            "upstream",
        ),
        (
            "oxplow-foundation/git-dashboard-uncommitted",
            "git-dashboard",
            "uncommitted",
        ),
        (
            "oxplow-foundation/git-dashboard-recent",
            "git-dashboard",
            "recent",
        ),
        (
            "oxplow-foundation/git-dashboard-streams",
            "git-dashboard",
            "streams",
        ),
        (
            "oxplow-foundation/git-dashboard-readiness",
            "git-dashboard",
            "readiness",
        ),
        (
            "oxplow-foundation/git-dashboard-remotes",
            "git-dashboard",
            "remotes",
        ),
        (
            "oxplow-foundation/git-history-commits",
            "git-history",
            "history",
        ),
        (
            "oxplow-foundation/uncommitted-commit",
            "uncommitted-changes",
            "commit",
        ),
        (
            "oxplow-foundation/uncommitted-files",
            "uncommitted-changes",
            "files",
        ),
        (
            "oxplow-foundation/local-history-recent",
            "local-history",
            "recent",
        ),
        (
            "oxplow-foundation/local-history-commits",
            "local-history",
            "by-commit",
        ),
        ("oxplow-foundation/comments-threads", "comments", "threads"),
        (
            "oxplow-foundation/comments-conversation",
            "comments",
            "conversation",
        ),
        ("oxplow-foundation/wiki-visited", "wiki-index", "visited"),
        ("oxplow-foundation/wiki-pages", "wiki-index", "pages"),
        (
            "oxplow-foundation/settings-overview",
            "settings",
            "overview",
        ),
        (
            "oxplow-foundation/settings-proposals",
            "settings",
            "settings-proposals",
        ),
        (
            "oxplow-foundation/settings-values",
            "settings",
            "settings-values",
        ),
        ("oxplow-foundation/settings-agents", "settings", "agents"),
        ("oxplow-foundation/agents-list", "settings", "agents-list"),
        (
            "oxplow-foundation/agents-settings",
            "settings",
            "agents-settings",
        ),
        (
            "oxplow-foundation/agents-new-thread",
            "settings",
            "agents-new-thread",
        ),
        (
            "oxplow-foundation/agents-prompt",
            "settings",
            "agents-prompt",
        ),
        ("oxplow-foundation/settings-lsp", "settings", "lsp"),
        ("oxplow-foundation/lsp-servers", "settings", "lsp-servers"),
        ("oxplow-foundation/lsp-install", "settings", "lsp-install"),
        (
            "oxplow-foundation/lsp-suggestions",
            "settings",
            "lsp-suggestions",
        ),
        (
            "oxplow-foundation/settings-extensions",
            "settings",
            "extensions",
        ),
        (
            "oxplow-foundation/extensions-health",
            "settings",
            "extensions-health",
        ),
        (
            "oxplow-foundation/extensions-credentials",
            "settings",
            "extensions-credentials",
        ),
        (
            "oxplow-foundation/extensions-secrets",
            "settings",
            "extensions-secrets",
        ),
        ("oxplow-foundation/settings-data", "settings", "data"),
        (
            "oxplow-foundation/data-entities",
            "settings",
            "data-entities",
        ),
        ("oxplow-foundation/data-sources", "settings", "data-sources"),
        ("oxplow-foundation/data-effects", "settings", "data-effects"),
        (
            "oxplow-foundation/settings-integrations",
            "settings",
            "integrations",
        ),
        ("oxplow-foundation/capabilities", "settings", "capabilities"),
        ("oxplow-foundation/replacements", "settings", "replacements"),
        (
            "oxplow-foundation/integrations",
            "settings",
            "integrations-integrations",
        ),
        ("oxplow-foundation/settings-ai", "settings", "ai"),
        ("oxplow-foundation/ai-providers", "settings", "ai-providers"),
        (
            "oxplow-foundation/ai-provider-form",
            "settings",
            "ai-provider-form",
        ),
        ("oxplow-foundation/ai-roles", "settings", "ai-roles"),
        ("oxplow-foundation/ai-calls", "settings", "ai-calls"),
        ("oxplow-foundation/catalog-prompts", "catalog", "ask"),
        (
            "oxplow-foundation/catalog-config-keys",
            "catalog",
            "configure",
        ),
        ("oxplow-foundation/alerts-decisions", "alerts", "decisions"),
        ("oxplow-foundation/alerts-hints", "alerts", "hints"),
        (
            "oxplow-foundation/alerts-operations",
            "alerts",
            "operations",
        ),
        (
            "oxplow-foundation/alerts-extensions",
            "alerts",
            "extensions",
        ),
        (
            "oxplow-foundation/new-session-start",
            "new-session",
            "start",
        ),
        (
            "oxplow-foundation/new-session-earlier",
            "new-session",
            "earlier",
        ),
        (
            "oxplow-foundation/stream-settings-prompt",
            "stream-settings",
            "prompt",
        ),
        (
            "oxplow-foundation/thread-settings-sessions",
            "thread-settings",
            "sessions",
        ),
        (
            "oxplow-foundation/thread-settings-prompt",
            "thread-settings",
            "prompt",
        ),
        ("oxplow-foundation/changes-header", "diff-view", "header"),
        (
            "oxplow-foundation/changes-description",
            "diff-view",
            "description",
        ),
        ("oxplow-foundation/changes-verdict", "diff-view", "verdict"),
        (
            "oxplow-foundation/changes-concurrent",
            "diff-view",
            "concurrent",
        ),
        ("oxplow-foundation/changes-files", "diff-view", "files"),
        (
            "oxplow-foundation/changes-extensions",
            "diff-view",
            "extensions",
        ),
        ("oxplow-foundation/wiki-page-body", "wiki-page", "body"),
        ("oxplow-foundation/wiki-page-info", "wiki-page", "info"),
        ("oxplow-foundation/wiki-page-refs", "wiki-page", "refs"),
        ("oxplow-foundation/work-item-state", "work-item", "state"),
        (
            "oxplow-foundation/work-item-fields-form",
            "work-item",
            "fields",
        ),
        ("oxplow-foundation/work-item-body", "work-item", "body"),
        ("oxplow-foundation/work-item-info", "work-item", "info"),
        ("oxplow-foundation/work-item-links", "work-item", "links"),
        (
            "oxplow-foundation/work-item-comments",
            "work-item",
            "comments",
        ),
        (
            "oxplow-foundation/work-item-activity",
            "work-item",
            "activity",
        ),
        ("oxplow-foundation/commit-detail", "commit", "detail"),
        ("oxplow-foundation/commit-files", "commit", "files"),
        ("oxplow-foundation/metric-header", "metric-detail", "header"),
        ("oxplow-foundation/metric-chart", "metric-detail", "chart"),
        (
            "oxplow-foundation/metric-unchecked",
            "metric-detail",
            "unchecked",
        ),
        (
            "oxplow-foundation/metric-recordings",
            "metric-detail",
            "recordings",
        ),
        (
            "oxplow-foundation/agent-session-waiting",
            "agent-session",
            "agent-session-waiting",
        ),
        (
            "oxplow-foundation/agent-session-terminal",
            "agent-session",
            "agent-session-terminal",
        ),
        (
            "oxplow-bundled/interface-tool-calls",
            "agent-interface",
            "interface-tool-calls",
        ),
        (
            "oxplow-bundled/interface-commands",
            "agent-interface",
            "interface-commands",
        ),
        (
            "oxplow-bundled/interface-hints",
            "agent-interface",
            "interface-hints",
        ),
        (
            "oxplow-bundled/change-summary",
            "change-review",
            "change-summary",
        ),
        (
            "oxplow-bundled/change-look-here",
            "change-review",
            "change-look-here",
        ),
        (
            "oxplow-bundled/change-treemap",
            "change-review",
            "change-treemap",
        ),
        (
            "oxplow-bundled/change-functions",
            "change-review",
            "change-functions",
        ),
        (
            "oxplow-bundled/change-test-files",
            "change-review",
            "change-test-files",
        ),
        (
            "oxplow-bundled/change-co-change",
            "change-review",
            "change-co-change",
        ),
        (
            "oxplow-bundled/change-duplicates",
            "change-review",
            "change-duplicates",
        ),
        (
            "oxplow-bundled/change-cross-zone-imports",
            "change-review",
            "change-cross-zone-imports",
        ),
        (
            "oxplow-bundled/effort-coverage",
            "effort-tests",
            "effort-coverage",
        ),
        (
            "oxplow-bundled/effort-untested-files",
            "effort-tests",
            "effort-untested-files",
        ),
        (
            "oxplow-bundled/effort-test-runs",
            "effort-tests",
            "effort-test-runs",
        ),
        (
            "oxplow-bundled/effort-failed-tests",
            "effort-tests",
            "effort-failed-tests",
        ),
        (
            "oxplow-bundled/effort-analysis-findings",
            "effort-tests",
            "effort-analysis-findings",
        ),
        ("oxplow-bundled/ready-tasks", "planning", "ready-tasks"),
        ("oxplow-bundled/backlog-tasks", "planning", "backlog-tasks"),
        ("oxplow-bundled/recent-notes", "planning", "recent-notes"),
        (
            "oxplow-bundled/duplicate-blocks",
            "quality",
            "duplicate-blocks",
        ),
        (
            "oxplow-bundled/recent-snapshots",
            "review",
            "recent-snapshots",
        ),
        (
            "oxplow-bundled/task-token-summary",
            "task-tokens",
            "task-token-summary",
        ),
        ("oxplow-bundled/task-turns", "task-tokens", "task-turns"),
        ("oxplow-bundled/token-total", "usage", "token-total"),
        ("oxplow-bundled/tokens-by-agent", "usage", "tokens-by-agent"),
        ("oxplow-bundled/tokens-by-day", "usage", "tokens-by-day"),
        (
            "oxplow-bundled/page-visits-by-day",
            "usage",
            "page-visits-by-day",
        ),
        ("oxplow-bundled/top-pages", "usage", "top-pages"),
        ("oxplow-foundation/catalog", "data-catalog", ""),
        (
            "oxplow-foundation/uncommitted",
            "uncommitted-changes",
            "row",
        ),
        ("oxplow-foundation/board", "board", "row"),
    ];
    let named = NAMED.iter().find(|(l, _, _)| *l == lens);
    let (slug, loader_name) = match named {
        Some((_, page, loader)) => (*page, loader.to_string()),
        None => (slug, slug.replace('-', "_")),
    };
    let snapshot = svc.extension_catalog.snapshot(&svc.layout.project_dir);
    let ext = snapshot
        .extensions()
        .iter()
        .find(|e| e.name == extension)
        .expect("the extension");
    let own = ext.page_files.iter().find(|p| p.slug == slug);
    let inlined = || {
        ext.page_files.iter().find(|p| {
            p.loaders
                .iter()
                .any(|l| l.name == slug || l.name == loader_name)
        })
    };
    let page = own
        .or_else(inlined)
        .unwrap_or_else(|| panic!("no page for lens {lens}"));
    let loader = page
        .loaders
        .iter()
        .find(|l| {
            let plain = |name: &str| name.replace('-', "_");
            plain(&l.name) == plain(&loader_name) || (named.is_none() && l.name == slug)
        })
        .map(|l| l.name.clone())
        // An inlined part is a node of that id, showing a loader.
        .or_else(|| {
            named?;
            page.nodes().into_iter().find_map(|n| match &n.kind {
                crate::pages::schema::NodeKind::Viz {
                    from: Some(from), ..
                } if n.id == loader_name => Some(from.clone()),
                _ => None,
            })
        })
        .or_else(|| page.loaders.first().map(|l| l.name.clone()))
        .expect("a loader");
    // A loader only a switch's child shows runs when the switch's param
    // names that child.
    fn switch_for(node: &crate::pages::schema::Node, loader: &str) -> Option<(String, String)> {
        use crate::pages::schema::NodeKind;
        fn shows(node: &crate::pages::schema::Node, loader: &str) -> bool {
            match &node.kind {
                NodeKind::Viz { from, .. } => from.as_deref() == Some(loader),
                NodeKind::Stack { children }
                | NodeKind::Grid { children }
                | NodeKind::Switch { children, .. } => children.iter().any(|c| shows(c, loader)),
                _ => false,
            }
        }
        match &node.kind {
            NodeKind::Switch { param, children } => children
                .iter()
                .find(|c| shows(c, loader))
                .map(|c| (param.clone(), c.id.clone())),
            NodeKind::Stack { children } | NodeKind::Grid { children } => {
                children.iter().find_map(|c| switch_for(c, loader))
            }
            _ => None,
        }
    }
    let mut wanted: std::collections::BTreeMap<String, oxplow_db::SqlCell> = params
        .iter()
        .filter(|(name, _)| page.params.iter().any(|p| &p.name == name))
        .cloned()
        .collect();
    if let Some((param, child)) = switch_for(&page.view, &loader) {
        wanted
            .entry(param)
            .or_insert(oxplow_db::SqlCell::Text(child));
    }
    let run = PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(page.id.clone()),
            params: wanted,
            ctx,
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .unwrap();
    let ran: Vec<&String> = run.loaders.keys().collect();
    let run = run
        .loaders
        .get(&loader)
        .unwrap_or_else(|| panic!("lens {lens}: loader {loader} didn't run (ran {ran:?})"));
    if let Some(error) = &run.error {
        panic!("page {} loader {loader}: {error}", page.id);
    }
    run.result.clone()
}

/// Page `id` (`<extension>/<slug>`) run as a person, with no params.
pub async fn run_page(svc: &crate::Services, id: &str) -> crate::pages::runner::PageRun {
    use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
    PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(id.to_string()),
            params: Default::default(),
            ctx: crate::extensions::ViewContext::default(),
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .unwrap()
}
