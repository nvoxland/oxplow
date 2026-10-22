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
/// `.oxplow/extensions/`, deployed at startup like any extension.
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
    services_with_effort_in(dir).await
}

/// [`services_with_effort`] over the project `dir`, already set up.
pub async fn services_with_effort_in(dir: tempfile::TempDir) -> EffortFixture {
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
