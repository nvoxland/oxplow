//! The kind registry (`.context/refs.md`): the one list of what a
//! ref can name. Each kind says what its ids look like, whether it takes a
//! `@rev`, and whether its id is `<provider>:<native id>`. Core kinds are
//! registered statically here; an extension's `ref_kinds` register into the
//! same registry, and a collision is a load error.
//!
//! The registry validates refs; it doesn't render them. What renders a
//! kind (its page, title model, icon) is UI and asset configuration that
//! lands with P1.3 and P6.

use std::collections::BTreeMap;

use regex::Regex;

use super::grammar::{is_valid_kind, CanonicalRef};

#[derive(Debug, Clone)]
pub struct KindSpec {
    pub kind: String,
    /// Matched against the decoded id.
    pub id_regex: Regex,
    /// May carry `@rev`.
    pub revisioned: bool,
    /// The id is `<provider>:<native id>`; the provider varies per project.
    pub provider_scoped: bool,
    /// `[[prefix:…]]` sugar that means this kind, beyond `[[kind:…]]`
    /// itself (e.g. `git:` for `commit`).
    pub wikilink_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KindError {
    #[error("bad kind name `{0}`: lowercase letters, digits and `_`, starting with a letter")]
    BadName(String),
    #[error("bad id regex for kind `{kind}`: {error}")]
    BadRegex { kind: String, error: String },
    #[error("kind `{0}` is already registered")]
    Collision(String),
    #[error("unknown kind `{0}`")]
    UnknownKind(String),
    #[error("id `{id}` is not a valid `{kind}` id (expected {pattern})")]
    BadId {
        kind: String,
        id: String,
        pattern: String,
    },
    #[error("kind `{0}` doesn't take a revision")]
    NotRevisioned(String),
}

impl KindSpec {
    pub fn new(kind: &str, id_regex: &str) -> Result<Self, KindError> {
        if !is_valid_kind(kind) {
            return Err(KindError::BadName(kind.to_string()));
        }
        let id_regex = Regex::new(id_regex).map_err(|e| KindError::BadRegex {
            kind: kind.to_string(),
            error: e.to_string(),
        })?;
        Ok(Self {
            kind: kind.to_string(),
            id_regex,
            revisioned: false,
            provider_scoped: false,
            wikilink_prefixes: Vec::new(),
        })
    }

    pub fn revisioned(mut self) -> Self {
        self.revisioned = true;
        self
    }

    pub fn provider_scoped(mut self) -> Self {
        self.provider_scoped = true;
        self
    }

    pub fn wikilink_prefix(mut self, prefix: &str) -> Self {
        self.wikilink_prefixes.push(prefix.to_string());
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct KindRegistry {
    kinds: BTreeMap<String, KindSpec>,
    /// The active work list's own ids, when it declares them: what makes a
    /// loose `tsk42` (or `ENG-12`) in text one of its items.
    work_item_ids: Option<WorkItemIds>,
}

/// A work list's ids, as it declares them (`WorkItemsProvider::id_pattern`).
#[derive(Debug, Clone)]
struct WorkItemIds {
    provider: String,
    /// The pattern, matched whole.
    whole: Regex,
}

impl KindRegistry {
    /// This registry recognizing `provider`'s ids (`pattern`, a regex
    /// matched whole): the active work list's, set by the app.
    pub fn with_work_item_ids(mut self, provider: &str, pattern: &str) -> Result<Self, KindError> {
        let whole = Regex::new(&format!("^(?:{pattern})$")).map_err(|e| KindError::BadRegex {
            kind: "work_item".into(),
            error: e.to_string(),
        })?;
        self.work_item_ids = Some(WorkItemIds {
            provider: provider.into(),
            whole,
        });
        Ok(self)
    }

    /// This registry recognizing no work list's ids (none is active, or the
    /// active one declares none).
    pub fn without_work_item_ids(mut self) -> Self {
        self.work_item_ids = None;
        self
    }

    /// `text`, whole, as one of the active work list's ids: its work-item
    /// id (`oxplow:tsk42`, what follows `work_item:`).
    pub fn work_item_id(&self, text: &str) -> Option<String> {
        let ids = self.work_item_ids.as_ref()?;
        ids.whole
            .is_match(text)
            .then(|| format!("{}:{text}", ids.provider))
    }

    /// Every token of `text` that is one of the active work list's ids —
    /// a token being a run of letters, digits, `_` and `-` — as work-item
    /// ids, in order, each once.
    pub fn find_work_item_ids(&self, text: &str) -> Vec<String> {
        if self.work_item_ids.is_none() {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
            if let Some(id) = self.work_item_id(token) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }

    pub fn register(&mut self, spec: KindSpec) -> Result<(), KindError> {
        if self.kinds.contains_key(&spec.kind) {
            return Err(KindError::Collision(spec.kind));
        }
        self.kinds.insert(spec.kind.clone(), spec);
        Ok(())
    }

    pub fn get(&self, kind: &str) -> Option<&KindSpec> {
        self.kinds.get(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &KindSpec> {
        self.kinds.values()
    }

    /// The kind a `[[prefix:…]]` sugar prefix means, if any.
    pub fn kind_for_wikilink_prefix(&self, prefix: &str) -> Option<&KindSpec> {
        self.kinds
            .values()
            .find(|k| k.wikilink_prefixes.iter().any(|p| p == prefix))
    }

    /// Is `r` a ref this registry knows, with a well-formed id?
    pub fn validate(&self, r: &CanonicalRef) -> Result<(), KindError> {
        let spec = self
            .get(&r.kind)
            .ok_or_else(|| KindError::UnknownKind(r.kind.clone()))?;
        if !spec.id_regex.is_match(&r.id) {
            return Err(KindError::BadId {
                kind: r.kind.clone(),
                id: r.id.clone(),
                pattern: spec.id_regex.to_string(),
            });
        }
        if r.rev.is_some() && !spec.revisioned {
            return Err(KindError::NotRevisioned(r.kind.clone()));
        }
        Ok(())
    }
}

/// The core kinds' names: the one place in Rust each is spelled. Build a
/// ref with `refs::build`, read one with `refs::build::{parse_as, id_of}`.
pub const PROJECT: &str = "project";
pub const STREAM: &str = "stream";
pub const THREAD: &str = "thread";
pub const EFFORT: &str = "effort";
pub const TURN: &str = "turn";
pub const SNAPSHOT: &str = "snapshot";
pub const COMMIT: &str = "commit";
pub const BRANCH: &str = "branch";
pub const FILE: &str = "file";
pub const DIR: &str = "dir";
pub const SYMBOL: &str = "symbol";
pub const WORK_ITEM: &str = "work_item";
pub const WIKI: &str = "wiki";
pub const COMMENT: &str = "comment";
pub const EVENT: &str = "event";
pub const LENS: &str = "lens";
pub const ANSWER: &str = "answer";
pub const PAGE: &str = "page";
pub const METRIC: &str = "metric";
pub const MODEL: &str = "model";
/// A language model by the name its harness or provider reports.
pub const AI_MODEL: &str = "ai_model";
pub const EXTENSION: &str = "extension";
pub const COMMAND: &str = "command";
pub const CONFIG: &str = "config";
pub const FINDING: &str = "finding";
pub const THREAD_NOTE: &str = "thread_note";
pub const AGENT_SESSION: &str = "agent_session";
pub const RUN: &str = "run";
pub const PROPOSAL: &str = "proposal";
pub const CLAIM: &str = "claim";
pub const DECISION: &str = "decision";
pub const COLLECTOR: &str = "collector";
pub const JOB_RUN: &str = "job_run";

/// A repo-relative path: no leading `/`, no `//`, no newline.
const PATH_ID: &str = r"^[^/\n][^\n]*$";
/// A prefixed entity id from `oxplow-domain::ids` (`thr1`, `eff362`).
fn prefixed(prefix: &str) -> String {
    format!(r"^{prefix}\d+$")
}

/// The core kinds (`.context/refs.md`). `work_item` is the only
/// provider-scoped kind in P1; `wiki`, `commit` and `symbol` use the
/// capability's active provider and keep bare ids.
pub fn core_kinds() -> KindRegistry {
    let mut reg = KindRegistry::default();
    let specs = [
        KindSpec::new(PROJECT, r"^[A-Za-z0-9._-]+$"),
        KindSpec::new(STREAM, &prefixed("str")),
        KindSpec::new(THREAD, &prefixed("thr")),
        KindSpec::new(EFFORT, &prefixed("eff")),
        KindSpec::new(TURN, &prefixed("trn")),
        KindSpec::new(SNAPSHOT, r"^[A-Za-z0-9]+$"),
        // 7..=40 hex for SHA-1 abbreviations and ids, up to 64 for SHA-256.
        KindSpec::new(COMMIT, r"^[0-9a-f]{7,64}$").map(|k| k.wikilink_prefix("git")),
        KindSpec::new(BRANCH, r"^[^\s]+$"),
        KindSpec::new(FILE, PATH_ID).map(KindSpec::revisioned),
        KindSpec::new(DIR, PATH_ID).map(KindSpec::revisioned),
        KindSpec::new(SYMBOL, r"^[a-z0-9_-]+/.+$").map(KindSpec::revisioned),
        KindSpec::new(WORK_ITEM, r"^[a-z][a-z0-9_-]*:.+$").map(KindSpec::provider_scoped),
        KindSpec::new(WIKI, r"^[A-Za-z0-9][A-Za-z0-9_-]*$"),
        KindSpec::new(COMMENT, &prefixed("cmt")),
        KindSpec::new(EVENT, r"^[0-9a-f-]{36}$"),
        // An agent's answer in a thread (`thread_answer.id`, P6.C1).
        KindSpec::new(ANSWER, r"^[0-9]+$"),
        // A language model by the name its harness or provider reports
        // (`claude-opus-4-8`, `gpt-5.5`): a token-usage fact's subject.
        KindSpec::new(AI_MODEL, r"^\S+$"),
        KindSpec::new(EXTENSION, r"^[a-z0-9-]+$"),
        // A `.oxplow/project.yaml` key (`config.changed`'s subject).
        KindSpec::new(CONFIG, r"^[A-Za-z][A-Za-z0-9]*$"),
        KindSpec::new(FINDING, r"^\S+$"),
        KindSpec::new(THREAD_NOTE, &prefixed("not")),
        // An agent slot opened on a thread (`agent_session.id`).
        KindSpec::new(AGENT_SESSION, &prefixed("ses")),
        KindSpec::new(RUN, r"^\d+$"),
        // A command waiting for a person's decision (`command_proposal.id`, P6b).
        KindSpec::new(PROPOSAL, r"^[0-9]+$"),
        // An agent's claim about its work and a decision it made or oxplow
        // inferred (`claim.id`, `decision.id`; P7.C4 reviews them).
        KindSpec::new(CLAIM, r"^[0-9]+$"),
        KindSpec::new(DECISION, r"^[0-9]+$"),
        // One run of a declared job (`v_job_run.id`); the job itself is
        // the contribution `job:<extension>/<id>`.
        KindSpec::new(JOB_RUN, r"^[0-9a-f-]{36}$"),
    ];
    for spec in specs {
        reg.register(spec.expect("core kind spec is valid"))
            .expect("core kinds don't collide");
    }
    for kind in crate::contribution::KINDS.iter().map(|k| k.kind) {
        reg.register(contribution_kind(kind).expect("contribution kind spec is valid"))
            .expect("contribution kinds don't collide with core's");
    }
    reg
}

/// The ref kind of a kind of contribution: `<kind>:<extension>/<id>`
/// (the id as `contribution::valid_local` allows), or `<kind>:<name>` for
/// a kind whose names are global. A few also name things that aren't
/// contributions: a page is also a shell route (`page:settings`), a model
/// also one of core's (`model:effort`), a collector also a project's or
/// the built-in catalog's (`collector:project/repo.scan`), and a lens or
/// page ref may carry its params (`lens:acme/blocked?stream_id=2`).
fn contribution_kind(kind: crate::contribution::ContributionKind) -> Result<KindSpec, KindError> {
    use crate::contribution::ContributionKind as C;
    let local = if kind.spec().dotted {
        r"[a-z][a-z0-9_.-]*"
    } else {
        r"[a-z][a-z0-9_-]*"
    };
    let owned = format!(r"[a-z][a-z0-9-]*/{local}");
    let pattern = match kind {
        C::Command => r"^[a-z0-9_]+(\.[a-z0-9_]+)+$".to_string(),
        C::EventType => r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$".to_string(),
        C::RefKind => r"^[a-z][a-z0-9_]*$".to_string(),
        C::Measure | C::Metric | C::Dimension => r"^[a-z0-9_.-]+$".to_string(),
        C::Lens => format!(r"^{owned}(\?.*)?$"),
        C::Page => format!(r"^(?:[a-z0-9-]+|{owned})(\?.*)?$"),
        C::Model => format!(r"^(?:[a-z][a-z0-9_]*|{owned})$"),
        _ => format!("^{owned}$"),
    };
    KindSpec::new(kind.segment(), &pattern)
}
