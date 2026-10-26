//! What a URI may name (`.context/resources.md`): every resource is
//! declared by an extension in its manifest's `resources:` block —
//! oxplow-foundation's included, so no resource is special. A declaration
//! says what its params are; the registry checks a URI against it.

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::oxplow_uri::{OxplowUri, ResourceName};

// oxplow's own resources, by name (oxplow-foundation declares each in its
// manifest's `resources:`).
pub const PROJECT: &str = "oxplow/project";
pub const STREAM: &str = "oxplow/stream";
pub const THREAD: &str = "oxplow/thread";
pub const EFFORT: &str = "oxplow/effort";
pub const TURN: &str = "oxplow/turn";
pub const SNAPSHOT: &str = "oxplow/snapshot";
pub const COMMIT: &str = "oxplow/commit";
pub const BRANCH: &str = "oxplow/branch";
pub const FILE: &str = "oxplow/file";
pub const DIR: &str = "oxplow/dir";
pub const SYMBOL: &str = "oxplow/symbol";
pub const WORK_ITEM: &str = "oxplow/work_item";
pub const WIKI: &str = "oxplow/wiki";
pub const COMMENT: &str = "oxplow/comment";
pub const EVENT: &str = "oxplow/event";
pub const PAGE: &str = "oxplow/page";
/// An ephemeral page an agent showed (`oxplow.page.show { spec }`): kept
/// in memory by the daemon, opened like a page.
pub const VIEW: &str = "oxplow/view";
pub const METRIC: &str = "oxplow/metric";
pub const MODEL: &str = "oxplow/model";
pub const AI_MODEL: &str = "oxplow/ai_model";
pub const EXTENSION: &str = "oxplow/extension";
pub const COMMAND: &str = "oxplow/command";
pub const CONFIG: &str = "oxplow/config";
pub const FINDING: &str = "oxplow/finding";
pub const AGENT_SESSION: &str = "oxplow/agent_session";
pub const HANDOFF: &str = "oxplow/handoff";
pub const TERMINAL: &str = "oxplow/terminal";
pub const SUBAGENT: &str = "oxplow/subagent";
pub const RUN: &str = "oxplow/run";
pub const PROPOSAL: &str = "oxplow/proposal";
pub const CLAIM: &str = "oxplow/claim";
pub const DECISION: &str = "oxplow/decision";
pub const COLLECTOR: &str = "oxplow/collector";
pub const JOB_RUN: &str = "oxplow/job_run";
pub const AI_CALL: &str = "oxplow/ai_call";
pub const CONSUMER: &str = "oxplow/consumer";
pub const BLOCK: &str = "oxplow/block";
pub const TEST: &str = "oxplow/test";
pub const NUDGE: &str = "oxplow/nudge";
pub const EXTENSION_DEPLOYMENT: &str = "oxplow/extension_deployment";
pub const PANEL: &str = "oxplow/panel";
pub const JOB: &str = "oxplow/job";

/// One of a resource's params: whether every URI carries it, and the
/// pattern its (decoded) value matches, anchored by its author.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ParamDecl {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub pattern: Option<String>,
}

/// `[[<prefix>:<value>]]` sugar: a wikilink with this prefix names the
/// resource with `param` set to the value (`[[git:abc1234]]` → a commit).
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct WikilinkDecl {
    pub prefix: String,
    pub param: String,
}

/// A declared resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDecl {
    /// `<extension>/<name>` (`oxplow/file`).
    #[specta(type = String)]
    pub name: ResourceName,
    pub label: String,
    pub params: BTreeMap<String, ParamDecl>,
    /// It takes params beyond its own (a page's or a lens's).
    pub additional: bool,
    /// The view whose rows name each one, by URI.
    pub resolve: Option<String>,
    /// The column of `resolve` that titles one (default `title`).
    pub title: Option<String>,
    /// The view whose `uri`, `title` and `body` search indexes.
    pub searchable: Option<String>,
    pub icon: Option<String>,
    pub wikilink: Option<WikilinkDecl>,
}

/// A `resources:` entry as a manifest holds it. `resolve` and `searchable`
/// name models as the manifest does; the app's loader turns them into the
/// views they publish as and checks them (`extension_resources`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceEntry {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub params: BTreeMap<String, ParamDecl>,
    #[serde(default)]
    pub additional: bool,
    #[serde(default)]
    pub resolve: Option<String>,
    /// The column of `resolve` that titles one (default `title`).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub searchable: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub wikilink: Option<WikilinkDecl>,
}

impl ResourceEntry {
    /// This entry as the resource `extension` declares (`resolve` and
    /// `searchable` left as written), or why its name isn't one.
    pub fn into_decl(self, extension: &str) -> Result<ResourceDecl, String> {
        let name = ResourceName::new(extension, &self.name).map_err(|_| {
            format!(
                "resource `{}`: a name is lowercase letters, digits and `_`, starting with a letter",
                self.name
            )
        })?;
        Ok(ResourceDecl {
            name,
            label: self.label,
            params: self.params,
            additional: self.additional,
            resolve: self.resolve,
            title: self.title,
            searchable: self.searchable,
            icon: self.icon,
            wikilink: self.wikilink,
        })
    }
}

impl ResourceDecl {
    /// A resource named by one required param `key` and nothing else:
    /// what a test or a small registry declares.
    pub fn keyed(name: &str, key: &str) -> Self {
        Self {
            name: name.parse().expect("a resource name"),
            label: name.to_string(),
            params: [(
                key.to_string(),
                ParamDecl {
                    required: true,
                    pattern: None,
                },
            )]
            .into(),
            additional: false,
            resolve: None,
            title: None,
            searchable: None,
            icon: None,
            wikilink: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    #[error("`{0}` is no declared resource (`resources:` in an extension's manifest)")]
    Undeclared(String),
    #[error("`{0}` is declared twice")]
    Collision(String),
    #[error("`{resource}`'s param `{param}` has a bad pattern: {error}")]
    BadPattern {
        resource: String,
        param: String,
        error: String,
    },
    #[error("wikilink prefix `{prefix}` is `{first}`'s and `{second}`'s")]
    WikilinkCollision {
        prefix: String,
        first: String,
        second: String,
    },
    #[error("`{resource}`'s wikilink names `{param}`, which isn't one of its params")]
    WikilinkParam { resource: String, param: String },
    #[error("`{uri}` has no `{param}`, which a `{resource}` needs")]
    MissingParam {
        uri: String,
        resource: String,
        param: String,
    },
    #[error("`{uri}`'s `{param}` isn't a `{resource}`'s (expected {pattern})")]
    BadValue {
        uri: String,
        resource: String,
        param: String,
        pattern: String,
    },
    #[error("`{uri}` has `{param}`, which a `{resource}` doesn't take")]
    UnknownParam {
        uri: String,
        resource: String,
        param: String,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    decl: ResourceDecl,
    patterns: BTreeMap<String, Regex>,
}

/// Every declared resource, by name.
#[derive(Debug, Clone, Default)]
pub struct ResourceRegistry {
    resources: BTreeMap<ResourceName, Entry>,
    wikilinks: BTreeMap<String, ResourceName>,
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

impl ResourceRegistry {
    /// This registry recognizing `provider`'s ids (`pattern`, a regex
    /// matched whole): the active work list's, set by the app.
    pub fn with_work_item_ids(
        mut self,
        provider: &str,
        pattern: &str,
    ) -> Result<Self, ResourceError> {
        let whole =
            Regex::new(&format!("^(?:{pattern})$")).map_err(|e| ResourceError::BadPattern {
                resource: WORK_ITEM.into(),
                param: "id".into(),
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

    /// `text`, whole, as one of the active work list's ids: that item's URI.
    pub fn work_item_uri_of(&self, text: &str) -> Option<String> {
        let ids = self.work_item_ids.as_ref()?;
        ids.whole
            .is_match(text)
            .then(|| super::build::work_item_uri(&ids.provider, text))
    }

    /// Every token of `text` that is one of the active work list's ids — a
    /// token being a run of letters, digits, `_` and `-` — as its item's
    /// URI, in order, each once.
    pub fn find_work_item_uris(&self, text: &str) -> Vec<String> {
        if self.work_item_ids.is_none() {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
            if let Some(uri) = self.work_item_uri_of(token) {
                if !out.contains(&uri) {
                    out.push(uri);
                }
            }
        }
        out
    }

    pub fn register(&mut self, decl: ResourceDecl) -> Result<(), ResourceError> {
        if self.resources.contains_key(&decl.name) {
            return Err(ResourceError::Collision(decl.name.to_string()));
        }
        let mut patterns = BTreeMap::new();
        for (param, p) in &decl.params {
            if let Some(pattern) = &p.pattern {
                let re = Regex::new(pattern).map_err(|e| ResourceError::BadPattern {
                    resource: decl.name.to_string(),
                    param: param.clone(),
                    error: e.to_string(),
                })?;
                patterns.insert(param.clone(), re);
            }
        }
        if let Some(w) = &decl.wikilink {
            if !decl.params.contains_key(&w.param) {
                return Err(ResourceError::WikilinkParam {
                    resource: decl.name.to_string(),
                    param: w.param.clone(),
                });
            }
            if let Some(first) = self.wikilinks.get(&w.prefix) {
                return Err(ResourceError::WikilinkCollision {
                    prefix: w.prefix.clone(),
                    first: first.to_string(),
                    second: decl.name.to_string(),
                });
            }
            self.wikilinks.insert(w.prefix.clone(), decl.name.clone());
        }
        self.resources
            .insert(decl.name.clone(), Entry { decl, patterns });
        Ok(())
    }

    pub fn get(&self, name: &ResourceName) -> Option<&ResourceDecl> {
        self.resources.get(name).map(|e| &e.decl)
    }

    pub fn resources(&self) -> impl Iterator<Item = &ResourceDecl> {
        self.resources.values().map(|e| &e.decl)
    }

    /// The resource whose `[[<prefix>:…]]` sugar this is.
    pub fn for_wikilink(&self, prefix: &str) -> Option<&ResourceDecl> {
        self.wikilinks.get(prefix).and_then(|n| self.get(n))
    }

    /// Whether `uri` names a declared resource with the params it takes:
    /// every required one, each matching its pattern, and no other unless
    /// it takes others.
    pub fn validate(&self, uri: &OxplowUri) -> Result<(), ResourceError> {
        let entry = self
            .resources
            .get(uri.resource())
            .ok_or_else(|| ResourceError::Undeclared(uri.resource().to_string()))?;
        let resource = entry.decl.name.to_string();
        for (param, p) in &entry.decl.params {
            if p.required && uri.param(param).is_none() {
                return Err(ResourceError::MissingParam {
                    uri: uri.to_string(),
                    resource,
                    param: param.clone(),
                });
            }
        }
        for (param, value) in uri.params() {
            match entry.patterns.get(param) {
                Some(re) if !re.is_match(value) => {
                    return Err(ResourceError::BadValue {
                        uri: uri.to_string(),
                        resource,
                        param: param.clone(),
                        pattern: re.as_str().to_string(),
                    })
                }
                _ if !entry.decl.params.contains_key(param) && !entry.decl.additional => {
                    return Err(ResourceError::UnknownParam {
                        uri: uri.to_string(),
                        resource,
                        param: param.clone(),
                    })
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// oxplow-foundation's resources, as its manifest declares them
/// (`resources:`): what a process names things with before the first
/// vocabulary rebuild registers every enabled extension's (foundation's
/// among them, from the same manifest).
pub fn foundation() -> &'static ResourceRegistry {
    static REGISTRY: std::sync::LazyLock<ResourceRegistry> = std::sync::LazyLock::new(|| {
        let manifest: serde_yaml::Value = serde_yaml::from_str(include_str!(
            "../../../../extensions/oxplow-foundation/extension.yaml"
        ))
        .expect("oxplow-foundation's manifest is YAML");
        let mut registry = ResourceRegistry::default();
        for entry in manifest["resources"]
            .as_sequence()
            .expect("oxplow-foundation declares resources")
        {
            let entry: ResourceEntry =
                serde_yaml::from_value(entry.clone()).expect("a resources: entry");
            registry
                .register(entry.into_decl("oxplow").expect("a resource name"))
                .expect("oxplow-foundation's resources register");
        }
        registry
    });
    &REGISTRY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, params: &[(&str, bool, Option<&str>)]) -> ResourceDecl {
        ResourceDecl {
            name: name.parse().unwrap(),
            label: name.to_string(),
            params: params
                .iter()
                .map(|(k, required, pattern)| {
                    (
                        k.to_string(),
                        ParamDecl {
                            required: *required,
                            pattern: pattern.map(str::to_string),
                        },
                    )
                })
                .collect(),
            additional: false,
            resolve: None,
            title: None,
            searchable: None,
            icon: None,
            wikilink: None,
        }
    }

    fn registry() -> ResourceRegistry {
        let mut r = ResourceRegistry::default();
        r.register(decl("oxplow/effort", &[("id", true, Some(r"^eff\d+$"))]))
            .unwrap();
        r.register(decl(
            "oxplow/file",
            &[
                ("path", true, None),
                ("rev", false, Some(r"^(git|snap):.+$")),
            ],
        ))
        .unwrap();
        r.register(ResourceDecl {
            additional: true,
            ..decl("oxplow/page", &[("page", true, None)])
        })
        .unwrap();
        r
    }

    fn check(uri: &str) -> Result<(), ResourceError> {
        registry().validate(&OxplowUri::parse(uri).unwrap())
    }

    #[test]
    fn a_uri_names_a_declared_resource_with_the_params_it_takes() {
        assert_eq!(check("oxplow:oxplow/effort?id=eff12"), Ok(()));
        assert_eq!(
            check("oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=1-2"),
            Ok(())
        );
        // A page takes its own params besides `page`.
        assert_eq!(check("oxplow:oxplow/page?page=tasks&scope=all"), Ok(()));
        assert!(matches!(
            check("oxplow:acme/pr?id=1"),
            Err(ResourceError::Undeclared(_))
        ));
        assert!(matches!(
            check("oxplow:oxplow/effort"),
            Err(ResourceError::MissingParam { .. })
        ));
        assert!(matches!(
            check("oxplow:oxplow/effort?id=12"),
            Err(ResourceError::BadValue { .. })
        ));
        assert!(matches!(
            check("oxplow:oxplow/effort?id=eff1&x=1"),
            Err(ResourceError::UnknownParam { .. })
        ));
        assert!(matches!(
            check("oxplow:oxplow/file?path=a&rev=HEAD"),
            Err(ResourceError::BadValue { .. })
        ));
    }

    #[test]
    fn a_resource_is_declared_once_and_its_sugar_is_its_own() {
        let mut r = registry();
        assert_eq!(
            r.register(decl("oxplow/effort", &[])),
            Err(ResourceError::Collision("oxplow/effort".into()))
        );
        let with_link = |name: &str, prefix: &str, param: &str| ResourceDecl {
            wikilink: Some(WikilinkDecl {
                prefix: prefix.into(),
                param: param.into(),
            }),
            ..decl(name, &[("sha", true, None)])
        };
        r.register(with_link("oxplow/commit", "git", "sha"))
            .unwrap();
        assert_eq!(
            r.for_wikilink("git").map(|d| d.name.to_string()),
            Some("oxplow/commit".into())
        );
        assert!(matches!(
            r.register(with_link("acme/rev", "git", "sha")),
            Err(ResourceError::WikilinkCollision { .. })
        ));
        assert!(matches!(
            r.register(with_link("acme/tag", "tag", "name")),
            Err(ResourceError::WikilinkParam { .. })
        ));
        assert!(matches!(
            r.register(decl("acme/bad", &[("id", true, Some("("))])),
            Err(ResourceError::BadPattern { .. })
        ));
    }
}
