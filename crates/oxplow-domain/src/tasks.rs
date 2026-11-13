//! What a store of the shared task model declares of its workflow
//! (`.context/tasks.md`): its stages and their buckets, and its fields.

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
    /// edited — a `oxtask.task.update` that names it is refused.
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
}
