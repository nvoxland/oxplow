//! The kind registry (target-architecture §4.2): the one list of what a
//! ref can name. Each kind says what its ids look like, whether it takes a
//! `@rev`, and whether its id is `<provider>:<native id>`. Core kinds are
//! registered statically here; a plugin's `ref_kinds` register into the
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
}

impl KindRegistry {
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

/// A repo-relative path: no leading `/`, no `//`, no newline.
const PATH_ID: &str = r"^[^/\n][^\n]*$";
/// A prefixed entity id from `oxplow-domain::ids` (`thr1`, `eff362`).
fn prefixed(prefix: &str) -> String {
    format!(r"^{prefix}\d+$")
}

/// The core kinds (target-architecture §4.2). `work_item` is the only
/// provider-scoped kind in P1; `wiki`, `commit` and `symbol` use the
/// capability's active provider and keep bare ids.
pub fn core_kinds() -> KindRegistry {
    let mut reg = KindRegistry::default();
    let specs = [
        KindSpec::new("project", r"^[A-Za-z0-9._-]+$"),
        KindSpec::new("stream", &prefixed("str")),
        KindSpec::new("thread", &prefixed("thr")),
        KindSpec::new("effort", &prefixed("eff")),
        KindSpec::new("turn", &prefixed("trn")),
        KindSpec::new("snapshot", r"^[A-Za-z0-9]+$"),
        // 7..=40 hex for SHA-1 abbreviations and ids, up to 64 for SHA-256.
        KindSpec::new("commit", r"^[0-9a-f]{7,64}$").map(|k| k.wikilink_prefix("git")),
        KindSpec::new("branch", r"^[^\s]+$"),
        KindSpec::new("file", PATH_ID).map(KindSpec::revisioned),
        KindSpec::new("dir", PATH_ID).map(KindSpec::revisioned),
        KindSpec::new("symbol", r"^[a-z0-9_-]+/.+$").map(KindSpec::revisioned),
        KindSpec::new("work_item", r"^[a-z][a-z0-9_-]*:.+$").map(KindSpec::provider_scoped),
        KindSpec::new("wiki", r"^[A-Za-z0-9][A-Za-z0-9_-]*$"),
        KindSpec::new("comment", &prefixed("cmt")),
        KindSpec::new("event", r"^[0-9a-f-]{36}$"),
        KindSpec::new("lens", r"^[a-z0-9-]+/[a-z0-9-]+(\?.*)?$"),
        // An agent's answer in a thread (`thread_answer.id`, P6.C1).
        KindSpec::new("answer", r"^[0-9]+$"),
        // A shell route (`page:settings`) or an extension's page
        // (`page:ext.<extension>.<page>`, P6.G2).
        KindSpec::new(
            "page",
            r"^(?:[a-z0-9-]+|ext\.[a-z0-9-]+\.[a-z0-9-]+)(\?.*)?$",
        ),
        KindSpec::new("metric", r"^[a-z0-9_.-]+$"),
        KindSpec::new("model", r"^v_[a-z0-9_]+$"),
        KindSpec::new("plugin", r"^[a-z0-9-]+$"),
        // Two or more dot segments, as `CommandSpec::validate_name` allows.
        KindSpec::new("command", r"^[a-z0-9_]+(\.[a-z0-9_]+)+$"),
        // A `.oxplow/project.yaml` key (`config.changed`'s subject).
        KindSpec::new("config", r"^[A-Za-z][A-Za-z0-9]*$"),
        KindSpec::new("finding", r"^\S+$"),
        KindSpec::new("task_note", &prefixed("not")),
        KindSpec::new("run", r"^\d+$"),
        // A command waiting for a person's decision (`command_proposal.id`, P6b).
        KindSpec::new("proposal", r"^[0-9]+$"),
        // An agent's claim about its work and a decision it made or oxplow
        // inferred (`claim.id`, `decision.id`; P7.C4 reviews them).
        KindSpec::new("claim", r"^[0-9]+$"),
        KindSpec::new("decision", r"^[0-9]+$"),
        // A collector: `<owner>/<id>`, the owner an extension, `project`
        // or `built-in`, the id dotted identifiers (P7.B3).
        KindSpec::new(
            "collector",
            r"^[a-z0-9-]+/[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$",
        ),
    ];
    for spec in specs {
        reg.register(spec.expect("core kind spec is valid"))
            .expect("core kinds don't collide");
    }
    reg
}
