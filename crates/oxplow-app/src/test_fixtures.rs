//! Shared test setup for tests that need a real `Services`.

#![cfg(test)]

pub(crate) mod lifecycle;

/// Every shipped extension a project may turn off, for a test of what runs
/// without them, as a YAML list's items.
pub(crate) fn optional_shipped() -> String {
    crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| !b.required)
        .map(|b| b.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// An extension's errors as the lines a person reads.
pub(crate) fn shown_errors(ext: &crate::extension::extensions::Extension) -> Vec<String> {
    ext.errors.iter().map(|d| ext.shown(d)).collect()
}

/// `ext.shown_errors()` for a test that holds the extension in a chain.
pub(crate) trait ShownErrors {
    fn shown_errors(&self) -> Vec<String>;
}

impl ShownErrors for crate::extension::extensions::Extension {
    fn shown_errors(&self) -> Vec<String> {
        shown_errors(self)
    }
}

/// How often [`until`] looks.
const UNTIL_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Polls `probe` every 10 ms until it gives `Some`, for at most `deadline`;
/// the value, or `None` if the time ran out. For a test that waits for
/// something to happen — never for one that asserts nothing did (that waits
/// on the loop's own done signal, not a clock).
pub(crate) async fn until_some<T, F, Fut>(deadline: std::time::Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let end = tokio::time::Instant::now() + deadline;
    loop {
        if let Some(found) = probe().await {
            return Some(found);
        }
        if tokio::time::Instant::now() >= end {
            return None;
        }
        tokio::time::sleep(UNTIL_POLL).await;
    }
}

/// Polls `cond` every 10 ms until it holds, for at most `deadline`; whether
/// it held. Callers assert on the answer with what they waited for.
pub(crate) async fn until<F, Fut>(deadline: std::time::Duration, mut cond: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    until_some(deadline, || {
        let held = cond();
        async move { held.await.then_some(()) }
    })
    .await
    .is_some()
}

use oxplow_tasks::work_item_ref;
use std::path::Path;
use std::sync::Arc;

use oxplow_db::EffortStore as _;
use oxplow_domain::{EffortId, ThreadId};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore as _;
use oxplow_tasks::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};

/// A git repo with one empty commit on `main`, whatever the machine's
/// `init.defaultBranch` (stream setup refuses non-git dirs).
pub fn init_git_repo(dir: &Path) {
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = git2::Repository::init_opts(dir, &options).unwrap();
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
        let changes = crate::extension::extension_deploy::stage_sources(svc).await;
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

/// What `held` says against extension `name`, one line each, shown under
/// the folder the extension is edited in; empty when nothing is held back.
pub(crate) fn shown_held(
    svc: &crate::Services,
    held: &crate::vocabulary_reactor::VocabularyErrors,
    name: &str,
) -> String {
    let snapshot = svc.extension_catalog.snapshot(&svc.layout.project_dir);
    let root = snapshot
        .extensions()
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.root().to_string())
        .unwrap_or_default();
    held.get(name)
        .map(|lines| {
            lines
                .iter()
                .map(|d| d.render(&root))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
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
        .map(|e| {
            e.errors
                .iter()
                .map(|d| e.shown(d))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let held = shown_held(svc, &held, name);
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
        for (rel, bytes) in b.files() {
            let file = to.join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
    }
    std::fs::write(
        dir.path().join(".oxplow/project.yaml"),
        "enableBundledExtensions: false\n",
    )
    .unwrap();
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    crate::extension::extension_deploy::deploy(&svc, "oxplow-foundation", true)
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

/// The test-only extension of code-metric collectors, measures and metrics
/// (`testdata/code-metrics/`): real scripts for the tests of the collector
/// machinery. It ships nowhere.
pub(crate) const CODE_METRICS_EXT: &str = "code-metrics";

/// Where [`CODE_METRICS_EXT`]'s source is.
pub(crate) fn code_metrics_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/code-metrics")
}

/// Install [`CODE_METRICS_EXT`] in the project at `project_dir`
/// (`.oxplow/extensions/code-metrics/`), for the next [`activate`].
pub(crate) fn install_code_metrics(project_dir: &Path) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy(
        &code_metrics_dir(),
        &project_dir
            .join(".oxplow/extensions")
            .join(CODE_METRICS_EXT),
    );
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
pub fn shipped_extensions() -> Vec<crate::extension::extensions::Extension> {
    static PROJECT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let project = PROJECT.get_or_init(|| tempfile::tempdir().unwrap());
    crate::extension::extensions::load_extensions(project.path())
}

/// A bundled page, run with `params`, and the rows of the loader
/// that carried its query: `<extension>/<slug>` as a page of that
/// name (its loader named for the slug, else the one its first viz node
/// shows), or a loader named for the slug (`-` as `_`) in one of the
/// extension's other pages, where it was inlined as a part. Only the
/// params the page declares are given to it.
pub async fn run_as_page(
    svc: &crate::Services,
    id: &str,
    params: &[(String, oxplow_db::SqlCell)],
) -> oxplow_db::SqlQueryResult {
    run_as_page_in(svc, id, params, crate::pages::spec::ViewContext::default()).await
}

/// [`run_as_page`] from a viewer's context (`view_context`).
pub async fn run_as_page_in(
    svc: &crate::Services,
    id: &str,
    params: &[(String, oxplow_db::SqlCell)],
    ctx: crate::pages::spec::ViewContext,
) -> oxplow_db::SqlQueryResult {
    use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
    let (extension, slug) = id.split_once('/').expect("<extension>/<slug>");
    // Pages the conversion inlined into a page as a part named otherwise,
    // or renamed (`pages convert` prints both).
    const NAMED: &[(&str, &str, &str)] = &[
        ("oxplow-foundation/context-waiting", "context", "waiting"),
        (
            "oxplow-foundation/context-delivered",
            "context",
            "delivered",
        ),
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
        ("oxplow-foundation/commit-detail", "commit", "detail"),
        ("oxplow-foundation/commit-files", "commit", "files"),
        ("oxplow-foundation/metric-header", "metric", "about"),
        ("oxplow-foundation/metric-chart", "metric", "chart"),
        (
            "oxplow-foundation/metric-unchecked",
            "metric-data",
            "unchecked",
        ),
        (
            "oxplow-foundation/metric-recordings",
            "metric-data",
            "recordings",
        ),
        (
            "oxplow-foundation/agent-session-terminal",
            "agent-session",
            "agent-session-terminal",
        ),
        ("oxplow-foundation/catalog", "data-catalog", ""),
        (
            "oxplow-foundation/uncommitted",
            "uncommitted-changes",
            "row",
        ),
    ];
    let named = NAMED.iter().find(|(l, _, _)| *l == id);
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
        .unwrap_or_else(|| panic!("no page for {id}"));
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
        .or_else(|| page.first_loader().map(|l| l.name.clone()))
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
        .unwrap_or_else(|| panic!("page {id}: loader {loader} didn't run (ran {ran:?})"));
    if let Some(error) = &run.error {
        panic!("page {} loader {loader}: {error}", page.id);
    }
    run.result.clone()
}
