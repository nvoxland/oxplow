//! What an external task provider's program declares and states
//! (`.context/tasks.md`): its stages and their buckets, its fields, what
//! it supports, and the items it records. A provider only fills the task
//! model (`oxplow/task_record`, bundled's): its items reach everything else
//! as rows of the model; its own writes are its program's.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
