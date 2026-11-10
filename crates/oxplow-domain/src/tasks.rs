//! The task tracker capability (P5.C2, P7.A1; `.context/tasks.md`):
//! tasks, issues, tickets — whatever a provider tracks — behind one
//! interface. oxplow's own tasks are one provider (`oxplow`); an issue
//! tracker is another. Reads are SQL over `v_task`; writes are the
//! `task.*` commands, which the bus dispatches by the ref's provider
//! segment (`oxplow:oxplow/task?id=<id>&provider=<provider>`) to that provider's
//! [`TaskVerbs`] — the same way for every list, oxplow's own included.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Actor, CommandCall, CommandError, Envelope};

/// What a task tracker's stage means to core: the bucket each of its declared
/// stages ([`StageDecl`]) is in. Core acts on buckets only — `backlog` is
/// filed but not ready, `todo` is ready (what an idle thread may pull),
/// `in_progress` is started, `blocked` waits on an answer, `done` and
/// `canceled` are closed — and the screens draw the list's own stages.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Backlog,
    Todo,
    InProgress,
    Blocked,
    Done,
    Canceled,
}

impl Bucket {
    pub const ALL: [Bucket; 6] = [
        Bucket::Backlog,
        Bucket::Todo,
        Bucket::InProgress,
        Bucket::Blocked,
        Bucket::Done,
        Bucket::Canceled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Backlog => "backlog",
            Bucket::Todo => "todo",
            Bucket::InProgress => "in_progress",
            Bucket::Blocked => "blocked",
            Bucket::Done => "done",
            Bucket::Canceled => "canceled",
        }
    }

    /// Done or canceled: the item's work is over, and it has no assignee.
    pub fn is_closed(self) -> bool {
        matches!(self, Bucket::Done | Bucket::Canceled)
    }

    /// The bucket a name spells (`in_progress`), if it is one.
    pub fn named(name: &str) -> Option<Bucket> {
        Bucket::ALL.into_iter().find(|b| b.as_str() == name)
    }
}

/// One of a task tracker's stages, as it declares them in order: its id (what
/// a record and an input name), how a person names it, and its bucket. The
/// first stage of a bucket is that bucket's default — where an input
/// naming the bucket (`to: done`) lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct StageDecl {
    /// snake_case, unique in the list (`in_review`).
    pub id: String,
    /// How a person names it (`In Review`).
    pub title: String,
    pub bucket: Bucket,
}

/// The buckets every task tracker must have a stage in: where a new item
/// lands (`todo`), what starting it means (`in_progress`) and finishing it
/// (`done`).
pub const REQUIRED_BUCKETS: [Bucket; 3] = [Bucket::Todo, Bucket::InProgress, Bucket::Done];

/// What's wrong with `stages`, if anything: ids snake_case and unique,
/// titles given, a stage in each required bucket, and a stage named like a
/// bucket in that bucket.
pub fn stages_problem(stages: &[StageDecl]) -> Option<String> {
    let snake = |n: &str| {
        n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    let bad = stages.iter().enumerate().find_map(|(i, st)| {
        if !snake(&st.id) {
            Some(format!("stage `{}`: its id must be snake_case", st.id))
        } else if stages[..i].iter().any(|o| o.id == st.id) {
            Some(format!("stage `{}` is declared twice", st.id))
        } else if st.title.trim().is_empty() {
            Some(format!("stage `{}` needs a title", st.id))
        } else if Bucket::named(&st.id).is_some_and(|b| b != st.bucket) {
            Some(format!(
                "stage `{}` is named like a bucket, so it must be in it, not `{}`",
                st.id,
                st.bucket.as_str()
            ))
        } else {
            None
        }
    });
    bad.or_else(|| {
        REQUIRED_BUCKETS
            .iter()
            .find(|b| !stages.iter().any(|st| st.bucket == **b))
            .map(|b| {
                format!(
                    "no stage is in `{}`: a task tracker needs one in each of todo, \
                     in_progress and done",
                    b.as_str()
                )
            })
    })
}

/// The bucket of stage `id` in `stages`; `None` for a stage it doesn't
/// declare.
pub fn bucket_of(stages: &[StageDecl], id: &str) -> Option<Bucket> {
    stages.iter().find(|s| s.id == id).map(|s| s.bucket)
}

/// The stage a name means in `stages`: a stage's id, else a bucket's name
/// for that bucket's default (its first stage). Else what's wrong, naming
/// the stages.
pub fn resolve_stage<'a>(stages: &'a [StageDecl], name: &str) -> Result<&'a StageDecl, String> {
    if let Some(stage) = stages.iter().find(|s| s.id == name) {
        return Ok(stage);
    }
    let ids = || {
        stages
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match Bucket::named(name) {
        Some(bucket) => stages.iter().find(|s| s.bucket == bucket).ok_or_else(|| {
            format!(
                "this task tracker has no stage in `{name}` (its stages: {})",
                ids()
            )
        }),
        None => Err(format!(
            "`{name}` isn't one of this task tracker's stages ({}) or a bucket \
             (backlog, todo, in_progress, blocked, done, canceled)",
            ids()
        )),
    }
}

/// What kind of value a declared field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// One of `values`.
    Enum,
    Text,
    Number,
}

/// One of a task tracker's own fields: kept in an item's `native` under
/// `name`, declared so screens render and edit it without knowing which
/// list is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct FieldDecl {
    /// Its key in `native` (snake_case).
    pub name: String,
    /// How a person names it.
    pub title: String,
    pub kind: FieldKind,
    /// An `enum`'s values, in the order they're offered (empty for any
    /// other kind).
    #[serde(default)]
    pub values: Vec<String>,
    /// Set by the list itself (who filed it, when it synced): shown, never
    /// edited — a `oxplow.task.update` that names it is refused.
    #[serde(default)]
    pub read_only: bool,
}

/// What's wrong with `fields`, if anything: names snake_case and unique,
/// an `enum` with values, any other kind without.
pub fn fields_problem(fields: &[FieldDecl]) -> Option<String> {
    let snake = |n: &str| {
        n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    fields.iter().enumerate().find_map(|(i, f)| {
        if !snake(&f.name) {
            Some(format!("field `{}`: its name must be snake_case", f.name))
        } else if fields[..i].iter().any(|o| o.name == f.name) {
            Some(format!("field `{}` is declared twice", f.name))
        } else if f.kind == FieldKind::Enum && f.values.is_empty() {
            Some(format!("field `{}`: an enum needs its values", f.name))
        } else if f.kind != FieldKind::Enum && !f.values.is_empty() {
            Some(format!("field `{}`: only an enum has values", f.name))
        } else {
            None
        }
    })
}

/// What a provider supports beyond create, update and transition.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type,
)]
pub struct TaskTrackerFeatures {
    /// Items nest (a parent ref).
    pub hierarchy: bool,
    pub comments: bool,
    pub links: bool,
    /// Items can be deleted (`oxplow.task.delete`).
    #[serde(default)]
    pub delete: bool,
    /// A write sent twice with one idempotency key is done once, the
    /// second answered as the first (the protocol's
    /// `InvokeParams.idempotency_key`): the host may send a write again
    /// when its reply was lost.
    #[serde(default)]
    pub idempotent_writes: bool,
}

/// One of an item's links, as its provider records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkRecord {
    /// What it points at: another item (`oxplow:oxplow/task?id=<id>&provider=<provider>`), or
    /// any ref the item is about (`oxplow:oxplow/collector?collector=triage/hot`, `oxplow:oxplow/effort?id=eff12`).
    pub target: String,
    /// The list's own type (oxplow: `blocks`, `discovered_from`,
    /// `relates_to`, `duplicates`, `supersedes`, `replies_to`, `about`).
    pub link_type: String,
}

/// One of an item's comments, as its provider records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentRecord {
    /// The provider's own id for it, unique on the item.
    pub id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// RFC 3339; absent, when the host recorded it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// An item as its provider now has it — what `v_task` holds, and
/// what a provider's `task.recorded` event carries. Who it's
/// assigned to and its place in that queue are core's, never a
/// provider's. `links` and `comments` are stated when the provider keeps them: absent, the
/// host keeps what it has; present, it's the whole set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    /// `oxplow:oxplow/task?id=<id>&provider=<provider>`.
    pub uri: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Which of the list's declared stages it's in (its bucket is core's
    /// lookup).
    pub stage: String,
    /// Provider-specific fields.
    #[serde(default)]
    pub native: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    /// Gone at the provider.
    #[serde(default)]
    pub deleted: bool,
    /// Its links (from it to others), the whole set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<LinkRecord>>,
    /// Its comments, the whole set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<CommentRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskTrackerError {
    #[error("`{0}` isn't a task ref (task:<provider>:<id>)")]
    NotARef(String),
    #[error("no task tracker provider `{provider}`; registered: {}", registered.join(", "))]
    UnknownProvider {
        provider: String,
        registered: Vec<String>,
    },
    #[error("{provider} tasks don't support {feature}")]
    Unsupported { provider: String, feature: String },
}

/// The provider a task's URI names.
pub fn provider_of(item_ref: &str) -> Result<String, TaskTrackerError> {
    crate::uri::build::task_of_uri(item_ref)
        .filter(|(p, id)| !p.is_empty() && !id.is_empty())
        .map(|(p, _)| p)
        .ok_or_else(|| TaskTrackerError::NotARef(item_ref.to_string()))
}

/// What one of a provider's verbs did, as the bus records it: the
/// handler's `result`, the events to log (the provider's
/// `task.recorded`), and the inverse — named by its **verb**
/// (`transition`), which the dispatching command turns back into
/// `task.<verb>` so an undo dispatches again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbOutcome {
    pub result: Value,
    pub events: Vec<Envelope>,
    pub inverse: Option<CommandCall>,
}

/// A provider's verbs: they run against its own system (its tables, a
/// process, a tracker) outside the bus's transaction, and are recorded
/// after they return; what they wrote reaches the interface through the
/// `task.recorded` events they answer with.
/// `input` is the `task.<verb>` input, less the host-side
/// `provider`; the implementation checks it against what the provider
/// declared and refuses anything else before calling.
#[async_trait]
pub trait TaskVerbs: Send + Sync {
    async fn invoke(
        &self,
        call: VerbCall,
        verb: &str,
        input: Value,
    ) -> Result<VerbOutcome, CommandError>;

    /// End the provider's process: the next call starts a new one. What
    /// the conformance kit re-sends a keyed write across (tsk916) — a
    /// provider that keeps its keys only in memory would do it again.
    async fn restart(&self);
}

/// Who calls a verb, and for which run: what the provider calls of
/// oxplow while it serves the verb (`host/call`) is counted on the run's
/// trace, with the `task.<verb>` run that called it.
pub struct VerbCall {
    pub run: crate::run::RunContext,
    /// The write's key when the caller has one (an effect's step: the same
    /// on every attempt); without one the provider's host mints one.
    pub idempotency_key: Option<String>,
}

impl VerbCall {
    /// Who calls it.
    pub fn actor(&self) -> &Actor {
        self.run.actor()
    }

    /// A call by `actor` outside any command run, its own run (the
    /// conformance kit's direct re-send).
    pub fn bare(actor: &Actor, idempotency_key: Option<String>) -> Self {
        Self {
            run: crate::run::RunContext::core(actor.clone(), crate::run::Mode::External),
            idempotency_key,
        }
    }
}

/// One source of tasks: its ref segment (`oxplow`, `issues`), what
/// it supports, and the verbs the dispatching commands call.
#[derive(Clone)]
pub struct TaskTracker {
    pub id: String,
    pub features: TaskTrackerFeatures,
    pub verbs: Arc<dyn TaskVerbs>,
    /// What its own ids look like (a regex, matched whole: `tsk\d+`), so a
    /// loose id in a command resolves to its item while it's the active
    /// task tracker; `None` declares none.
    pub id_pattern: Option<String>,
    /// It takes any item's ref and keeps nothing (none): verbs naming
    /// another list's items reach it instead of being refused.
    pub sink: bool,
    /// Its stages, in order, as it declares them ([`StageDecl`]).
    pub stages: Vec<StageDecl>,
}

/// A stage named like each bucket, in it: the stages of a list that has
/// no workflow of its own (none, a sink).
pub fn bucket_stages() -> Vec<StageDecl> {
    Bucket::ALL
        .into_iter()
        .map(|b| StageDecl {
            id: b.as_str().into(),
            title: b.as_str().replace('_', " "),
            bucket: b,
        })
        .collect()
}

impl std::fmt::Debug for TaskTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTracker")
            .field("id", &self.id)
            .field("features", &self.features)
            .field("id_pattern", &self.id_pattern)
            .field("sink", &self.sink)
            .finish()
    }
}

/// oxtask's provider id: `oxplow:oxplow/task?id=tsk<n>&provider=oxtask`.
pub const OXTASK: &str = "oxtask";

#[derive(Default)]
struct Providers {
    by_id: BTreeMap<String, TaskTracker>,
}

/// Where the active provider is read from: the config as it is now
/// (`activeProviders`), so a person's choice applies to the next `create`
/// — no copy for a reactor to refresh late (tsk1011).
pub type ActiveSource = Arc<dyn Fn() -> String + Send + Sync>;

/// The registered providers, by name. Cloning shares them.
#[derive(Clone)]
pub struct TaskTrackers {
    providers: Arc<std::sync::RwLock<Providers>>,
    active: ActiveSource,
    /// Each list's `id_pattern`, compiled whole-match, the first time a
    /// loose id is resolved against it.
    matchers: Arc<std::sync::RwLock<BTreeMap<String, regex::Regex>>>,
}

impl TaskTrackers {
    /// A registry whose active provider is what `active` says each time
    /// it's asked.
    pub fn new(active: ActiveSource) -> Self {
        Self {
            providers: Arc::default(),
            active,
            matchers: Arc::default(),
        }
    }

    pub fn register(&self, provider: TaskTracker) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .insert(provider.id.clone(), provider);
    }

    /// Remove a provider (an instance that stopped).
    pub fn unregister(&self, provider: &str) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .remove(provider);
    }

    /// Every registered provider's name, sorted.
    pub fn names(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .keys()
            .cloned()
            .collect()
    }

    pub fn get(&self, provider: &str) -> Result<TaskTracker, TaskTrackerError> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .get(provider)
            .cloned()
            .ok_or_else(|| TaskTrackerError::UnknownProvider {
                provider: provider.to_string(),
                registered: self.names(),
            })
    }

    /// The provider `item_ref` belongs to.
    pub fn for_ref(&self, item_ref: &str) -> Result<TaskTracker, TaskTrackerError> {
        self.get(&provider_of(item_ref)?)
    }

    /// The provider a `create` with no `provider` files on. Whether it is
    /// running is the caller's to check (`get`): an active provider that
    /// isn't is a failure naming it, never a silent fallback.
    /// The URI of `raw`, a loose id of the active task tracker's (`tsk42` →
    /// `oxplow:oxplow/task?id=tsk42&provider=oxtask`), or why it isn't
    /// one.
    pub fn loose_ref(&self, raw: &str) -> Result<String, String> {
        let active = self.active();
        let Some(pattern) = self.get(&active).ok().and_then(|p| p.id_pattern) else {
            return Err(format!(
                "`{raw}` isn't a task's URI, and the active work \
                 list declares no id of its own"
            ));
        };
        let matched = {
            let cached = self.matchers.read().unwrap_or_else(|e| e.into_inner());
            cached.get(&pattern).map(|r| r.is_match(raw))
        };
        let matched = match matched {
            Some(m) => m,
            None => {
                let compiled = regex::Regex::new(&format!("^(?:{pattern})$")).map_err(|e| {
                    format!("the active task tracker's id pattern `{pattern}`: {e}")
                })?;
                let m = compiled.is_match(raw);
                self.matchers
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(pattern.clone(), compiled);
                m
            }
        };
        if matched {
            Ok(crate::uri::build::task_uri(&active, raw))
        } else {
            Err(format!(
                "`{raw}` isn't a task's URI (oxplow:oxplow/task?id=<id>&provider=<provider>) or an id of the active \
                 task tracker (`{pattern}`)"
            ))
        }
    }

    pub fn active(&self) -> String {
        (self.active)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_checked() {
        let field = |name: &str, kind: FieldKind, values: &[&str]| FieldDecl {
            name: name.into(),
            title: name.into(),
            kind,
            values: values.iter().map(|v| v.to_string()).collect(),
            read_only: false,
        };
        assert_eq!(
            fields_problem(&[field("priority", FieldKind::Enum, &["high", "low"])]),
            None
        );
        let problem = |fields: &[FieldDecl]| fields_problem(fields).unwrap_or_default();
        assert!(problem(&[field("Priority", FieldKind::Text, &[])]).contains("snake_case"));
        assert!(problem(&[
            field("points", FieldKind::Number, &[]),
            field("points", FieldKind::Number, &[])
        ])
        .contains("twice"));
        assert!(problem(&[field("size", FieldKind::Enum, &[])]).contains("needs its values"));
        assert!(problem(&[field("note", FieldKind::Text, &["x"])]).contains("only an enum"));
    }

    struct Nothing;

    #[async_trait]
    impl TaskVerbs for Nothing {
        async fn invoke(
            &self,
            _call: VerbCall,
            _verb: &str,
            _input: Value,
        ) -> Result<VerbOutcome, CommandError> {
            unreachable!("the registry never calls a verb")
        }

        async fn restart(&self) {}
    }

    fn named(id: &str) -> TaskTracker {
        TaskTracker {
            id: id.into(),
            features: TaskTrackerFeatures::default(),
            verbs: Arc::new(Nothing),
            id_pattern: None,
            sink: false,
            stages: bucket_stages(),
        }
    }

    fn stage(id: &str, bucket: Bucket) -> StageDecl {
        StageDecl {
            id: id.into(),
            title: id.into(),
            bucket,
        }
    }

    /// A list declares its stages: snake_case ids once each, titled, one
    /// in each of todo, in_progress and done, and a stage named like a
    /// bucket in that bucket.
    #[test]
    fn stages_need_ids_titles_and_the_required_buckets() {
        let ok = vec![
            stage("ready", Bucket::Todo),
            stage("doing", Bucket::InProgress),
            stage("in_review", Bucket::InProgress),
            stage("shipped", Bucket::Done),
        ];
        assert_eq!(stages_problem(&ok), None);
        let mut twice = ok.clone();
        twice.push(stage("ready", Bucket::Todo));
        assert!(stages_problem(&twice).unwrap().contains("twice"));
        let mut misnamed = ok.clone();
        misnamed.push(stage("done", Bucket::Canceled));
        assert!(stages_problem(&misnamed)
            .unwrap()
            .contains("named like a bucket"));
        assert!(stages_problem(&ok[..2]).unwrap().contains("`done`"));
        assert!(stages_problem(&[stage("Ready", Bucket::Todo)])
            .unwrap()
            .contains("snake_case"));
    }

    /// A stage's id names it; a bucket's name its first stage; anything
    /// else is refused naming the stages.
    #[test]
    fn a_name_resolves_to_a_stage_or_its_buckets_default() {
        let stages = vec![
            stage("ready", Bucket::Todo),
            stage("doing", Bucket::InProgress),
            stage("in_review", Bucket::InProgress),
            stage("shipped", Bucket::Done),
        ];
        assert_eq!(resolve_stage(&stages, "in_review").unwrap().id, "in_review");
        assert_eq!(resolve_stage(&stages, "in_progress").unwrap().id, "doing");
        assert_eq!(resolve_stage(&stages, "done").unwrap().id, "shipped");
        let none = resolve_stage(&stages, "blocked").unwrap_err();
        assert!(none.contains("no stage in `blocked`"), "{none}");
        let bad = resolve_stage(&stages, "nope").unwrap_err();
        assert!(bad.contains("ready, doing, in_review, shipped"), "{bad}");
        assert_eq!(bucket_of(&stages, "in_review"), Some(Bucket::InProgress));
        assert_eq!(bucket_of(&stages, "nope"), None);
    }

    #[test]
    fn a_ref_finds_its_provider_and_a_foreign_one_names_the_registered() {
        let registry = TaskTrackers::new(Arc::new(|| OXTASK.to_string()));
        registry.register(named("oxtask"));
        registry.register(named("fake"));
        assert_eq!(
            registry
                .for_ref("oxplow:oxplow/task?id=tsk1&provider=oxtask")
                .unwrap()
                .id,
            "oxtask"
        );
        let err = registry
            .for_ref("oxplow:oxplow/task?id=ENG-12&provider=issues")
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "no task tracker provider `issues`; registered: fake, oxtask"
        );
        assert!(matches!(
            registry.for_ref("tsk1").err().unwrap(),
            TaskTrackerError::NotARef(_)
        ));
        assert!(matches!(
            provider_of("oxplow:oxplow/task?id=x&provider="),
            Err(TaskTrackerError::NotARef(_))
        ));
    }

    /// The active provider is what its source says each time it's asked;
    /// naming one doesn't check that it runs — a `create` on a missing
    /// active provider must fail naming it, which `get` does.
    #[test]
    fn the_active_provider_is_read_from_its_source_and_is_not_a_fallback() {
        let named_now = Arc::new(std::sync::Mutex::new(OXTASK.to_string()));
        let source = named_now.clone();
        let registry = TaskTrackers::new(Arc::new(move || source.lock().unwrap().clone()));
        registry.register(named("oxtask"));
        assert_eq!(registry.active(), "oxtask");
        *named_now.lock().unwrap() = "issues".into();
        assert_eq!(registry.active(), "issues");
        assert!(registry.get(&registry.active()).is_err());
    }

    /// An assignee is named by its URI or a thread's id, and prints as the
    /// URI `v_task.assignee` holds.
    #[test]
    fn an_assignee_parses_from_a_uri_or_a_threads_id() {
        let thread = Assignee::parse("thr3").unwrap();
        assert_eq!(thread.uri(), "oxplow:oxplow/thread?id=thr3");
        assert_eq!(Assignee::parse(&thread.uri()).unwrap(), thread);
        let person = Assignee::parse("oxplow:oxplow/person").unwrap();
        assert_eq!(person, Assignee::Person(None));
        assert_eq!(person.uri(), "oxplow:oxplow/person");
        assert_eq!(Assignee::parse("person").unwrap(), person);
        assert!(Assignee::parse("tsk3").is_err());
        assert!(Assignee::parse("oxplow:oxplow/effort?id=eff1").is_err());
    }

    #[test]
    fn a_record_round_trips_under_its_wire_names() {
        let record = TaskRecord {
            uri: "oxplow:oxplow/task?id=W-1&provider=fake".into(),
            title: "t".into(),
            body: String::new(),
            stage: "doing".into(),
            native: serde_json::json!({ "points": 3 }),
            parent_uri: None,
            deleted: false,
            links: None,
            comments: None,
        };
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["uri"], "oxplow:oxplow/task?id=W-1&provider=fake");
        assert_eq!(json["stage"], "doing");
        assert_eq!(serde_json::from_value::<TaskRecord>(json).unwrap(), record);
    }

    /// A provider that declares no `delete` reads as `false` (older
    /// declarations have no such flag).
    #[test]
    fn features_default_delete_to_false() {
        let f: TaskTrackerFeatures = serde_json::from_value(serde_json::json!({
            "hierarchy": true, "comments": false, "links": false
        }))
        .unwrap();
        assert!(!f.delete);
    }
}

// ---- The verbs' inputs: one shape for every list ----
//
// What a `oxplow.task.<verb>` command takes, and what a list's verb
// receives (`TaskVerbs::invoke`): the `v_task` columns, a
// `stage` (core resolves a bucket to the list's default before the list
// sees it), and `native` for its own fields.

/// Move an item to one of its list's stages.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskTransitionInput {
    /// The item's URI (`oxplow:oxplow/task?id=tsk42&provider=oxtask`, `oxplow:oxplow/task?id=ENG-12&provider=issues`).
    pub uri: String,
    /// A stage the list declares (`v_capability_provider.stages`), or a
    /// bucket — `backlog`, `todo`, `in_progress`, `blocked`, `done`,
    /// `canceled` — for that bucket's default stage. Core resolves it: the
    /// list receives its own stage's id.
    pub to: String,
}

/// A new item on the active tracker, optionally straight into
/// a state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCreateInput {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's URI, on the same provider (needs `hierarchy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    /// The stage it starts in — a stage the list declares, or a bucket for
    /// its default — `todo`'s default when absent. Core resolves it: the
    /// list receives its own stage's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// The tracker's own fields (oxplow: `{ priority? }`), as its
    /// `create` declares them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
    /// Who is going to work on it (`oxplow.task.assign`'s `to`):
    /// absent, nobody. Core's record: the provider never sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Its links, made with it (needs `links`): what it is `about`, what
    /// it blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkRecord>,
}

/// Edit an item's fields and, optionally, its state — one run. Absent
/// fields are left alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskUpdateInput {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's URI on the same provider, or `""` to detach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    /// The stage to move it to, as `transition`'s `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// The provider's own fields to change (oxplow: `{ priority? }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}

/// A typed link from an item to another item of the same provider, or to
/// any URI (`about` it) — and, as `unlink`'s input, that link's removal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskLinkInput {
    /// The item linked from.
    pub uri: String,
    /// What it links to: an item of the same provider, or any URI of a
    /// registered kind. Only an item is checked to exist; anything else
    /// may dangle.
    pub target: String,
    /// The provider names its own types (oxplow: blocks, relates_to,
    /// discovered_from, duplicates, supersedes, replies_to, about).
    pub link_type: String,
}

/// A comment on an item (oxplow: a task note).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCommentInput {
    pub uri: String,
    /// Markdown.
    pub body: String,
}

/// Remove an item (oxplow: soft — the row stays, marked deleted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskDeleteInput {
    pub uri: String,
}

/// `oxplow.task.reorder`: put an item before or after another in its
/// assignee's queue (neither: at its end). Core's, for every provider.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskReorderInput {
    /// The task's URI (`oxplow:oxplow/task?id=tsk42&provider=oxtask`).
    pub uri: String,
    /// Put it just before this item of the same queue.
    #[serde(default)]
    pub before: Option<String>,
    /// Put it just after this item of the same queue.
    #[serde(default)]
    pub after: Option<String>,
}

/// Who an item is assigned to: a thread, or the person (one of several
/// when it has an id). Nobody is no assignee at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignee {
    Thread(crate::ThreadId),
    Person(Option<String>),
}

impl Assignee {
    /// What `oxplow.task.assign`'s `to` names: a thread's URI or id
    /// (`thr3`), or the person's URI (`oxplow:oxplow/person`, `person`).
    pub fn parse(raw: &str) -> Result<Assignee, String> {
        use crate::uri::{build, resources};
        if raw == "person" {
            return Ok(Assignee::Person(None));
        }
        if let Ok(thread) = raw.parse::<crate::ThreadId>() {
            return Ok(Assignee::Thread(thread));
        }
        if let Some(thread) = build::thread_of_uri(raw) {
            return Ok(Assignee::Thread(thread));
        }
        if let Ok(person) = build::parse_resource(raw, resources::PERSON) {
            return Ok(Assignee::Person(person.param("id").map(str::to_string)));
        }
        Err(format!(
            "`{raw}` names no assignee: a thread (`oxplow:oxplow/thread?id=thr3`, `thr3`) or the \
             person (`oxplow:oxplow/person`)"
        ))
    }

    /// Its URI, as `v_task.assignee` holds it.
    pub fn uri(&self) -> String {
        use crate::uri::build;
        match self {
            Assignee::Thread(t) => build::thread_uri(*t),
            Assignee::Person(id) => build::person_uri(id.as_deref()),
        }
    }
}

/// `oxplow.task.assign`: who is going to work on an item — a thread
/// (`oxplow:oxplow/thread?id=thr3`), the person (`oxplow:oxplow/person`) or
/// nobody (`to` absent or null: the pool, or the backlog for a backlog
/// item) — at the end of that assignee's queue, or next to an item there.
/// Core's record, for every provider (`.context/tasks.md`
/// "Assignment").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskAssignInput {
    pub uri: String,
    /// The assignee's URI; absent or null, nobody.
    #[serde(default)]
    pub to: Option<String>,
    /// Put it just before this item of the assignee's queue.
    #[serde(default)]
    pub before: Option<String>,
    /// Put it just after this item of the assignee's queue.
    #[serde(default)]
    pub after: Option<String>,
}
