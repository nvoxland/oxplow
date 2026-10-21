//! What a URI may name (`.context/resources.md`): every resource is
//! declared by an extension in its manifest's `resources:` block —
//! oxplow-foundation's included, so no resource is special. A declaration
//! says what its params are; the registry checks a URI against it.

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::oxplow_uri::{OxplowUri, ResourceName};

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
}

impl ResourceRegistry {
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
