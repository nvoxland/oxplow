//! What a contribution asks to do, what its extension may do, and what
//! each call was decided (`.context/extensions-permissions.md` "Grants").
//!
//! A [`GrantRequest`] is derived from what a contribution declares — the
//! scopes it calls (a model among them, `ai.compute`), the program it
//! runs, the hosts it reaches, the credentials and environment it reads. A
//! [`Grant`] is what its extension may reach: the `permissions:` a person
//! approved when it was deployed, and its own command namespace. A run
//! decides each thing it does against both — declared, and permitted — and
//! records the [`Decision`], including where oxplow can't hold it to what
//! was permitted.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// What a contribution asks to do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRequest {
    /// The scopes it calls (`sql.read`), the services its scripts use
    /// among them (`tree.read`; `ai.compute`, which spends the person's AI
    /// provider key).
    pub scopes: BTreeSet<String>,
    /// The program it runs (its entry, an MCP server's url), when it runs
    /// one: a program reads and writes files as the person, which oxplow
    /// doesn't confine.
    pub program: Option<String>,
    /// The hosts it may reach (`api.github.com`, `*.example.com`).
    pub hosts: BTreeSet<String>,
    /// The keychain credentials it gets, by name.
    pub credentials: BTreeSet<String>,
    /// The host environment variables it gets, by name.
    pub env: BTreeSet<String>,
    /// The commands it may run with its viewer's rights (a component).
    pub commands: BTreeSet<String>,
}

impl GrantRequest {
    /// Its canonical JSON: the same request, the same text, whatever order
    /// it was declared in — what an approval's hash covers.
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("a grant request serializes")
    }
}

/// What a contribution's extension may reach: the permissions approved
/// when it was deployed, and its own commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Its command namespace: its own commands need no permission.
    pub namespace: String,
    pub permissions: crate::permissions::Permissions,
}

impl Grant {
    pub fn from_permissions(
        namespace: &str,
        permissions: &crate::permissions::Permissions,
    ) -> Self {
        Self {
            namespace: namespace.to_string(),
            permissions: permissions.clone(),
        }
    }

    /// The baseline and nothing else, with no commands of its own: a run
    /// that isn't any extension's.
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether its permissions allow `need`; else the permission that
    /// would (`ai`, `process`, …).
    pub fn permits(&self, need: &Need) -> Result<(), String> {
        let p = &self.permissions;
        let ok = match need {
            Need::Scope(s) => match crate::service::need_scope(s) {
                "sql.read" => p.sql.is_some(),
                "tree.read" => p.files.is_some(),
                "ai.compute" | "ai.discover" => p.ai,
                _ => true,
            },
            // A dataset's bounds are its declaration's; reading a dataset at all
            // is decided where SQL is asked for (`Scope("sql.read")`).
            Need::Dataset(_) => true,
            Need::Path(_) => p.files.is_some(),
            Need::Program(_) | Need::Files | Need::Env(_) => p.process,
            Need::Host(h) => p.host_allowed(h),
            Need::Credential(c) => p.secrets.contains(c),
            Need::Command(c) => p.command_allowed(&self.namespace, c),
        };
        if ok {
            return Ok(());
        }
        // What to add to `permissions:`, as the list spells it.
        Err(match need {
            Need::Scope(s) => match crate::service::need_scope(s) {
                "sql.read" => "sql.read".to_string(),
                "tree.read" => "files.read".to_string(),
                _ => "ai.compute".to_string(),
            },
            Need::Dataset(_) => "sql.read".to_string(),
            Need::Path(_) => "files.read".to_string(),
            Need::Program(_) | Need::Files | Need::Env(_) => "process.run".to_string(),
            Need::Host(h) => format!("network:{h}"),
            Need::Credential(c) => format!("secrets:{c}"),
            Need::Command(c) => format!("commands:{c}"),
        })
    }
}

/// One thing a run does that its grant decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "need", content = "name", rename_all = "snake_case")]
pub enum Need {
    Scope(String),
    /// A published dataset, including a derived dataset's inputs.
    Dataset(String),
    /// One portable path from the captured tree.
    Path(String),
    Program(String),
    Host(String),
    Credential(String),
    Env(String),
    Command(String),
    /// Files outside oxplow, read or written by a program it runs.
    Files,
}

/// What was decided for a [`Need`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    Allowed,
    /// Refused, and why.
    Denied(String),
    /// Allowed, but oxplow can't hold it to what was granted here: why.
    Unenforced(String),
}

/// A need and its decision, as a run records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decided {
    #[serde(flatten)]
    pub need: Need,
    #[serde(flatten)]
    pub decision: Decision,
}

/// Where oxplow can hold a granted need to what was granted, on this
/// machine: the one table the decisions and the review both read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enforcement {
    /// Whether all network paths are confined to the granted hosts.
    /// A direct-network sandbox alone cannot rule out local brokers.
    pub hosts: bool,
}

impl Enforcement {
    /// Why a granted `need` isn't held to its grant here, if it isn't.
    pub fn gap(&self, need: &Need) -> Option<&'static str> {
        match need {
            Need::Host(_) if !self.hosts => {
                Some("network isn't confined to the granted hosts on this OS")
            }
            Need::Files => {
                Some("a program runs with your file, network and process authority; oxplow cleans up its processes but doesn't confine it")
            }
            _ => None,
        }
    }
}

/// Decide `need`: within what the contribution `declared` (a command is
/// its extension's to allow, not the declaration's: a script's calls
/// aren't known before it runs), then within what its extension's `grant`
/// permits, then whether this machine holds it to that.
pub fn decide(
    need: &Need,
    declared: &GrantRequest,
    grant: &Grant,
    enforcement: Enforcement,
) -> Decision {
    let allowed = declared;
    let within = match need {
        Need::Scope(s) => allowed
            .scopes
            .iter()
            .any(|n| crate::service::need_scope(n) == s),
        Need::Dataset(dataset) => {
            allowed.scopes.contains("sql.read")
                || allowed.scopes.contains(&format!("sql.read:{dataset}"))
        }
        Need::Path(path) => {
            crate::service::valid_path(path)
                && (allowed.scopes.contains("tree.read")
                    || allowed.scopes.iter().any(|n| {
                        n.strip_prefix("tree.read:").is_some_and(|p| {
                            crate::service::valid_path(p)
                                && (p == path || (p.ends_with('/') && path.starts_with(p)))
                        })
                    }))
        }
        Need::Program(p) => allowed.program.as_deref() == Some(p.as_str()),
        Need::Host(h) => allowed.hosts.iter().any(|pattern| host_matches(pattern, h)),
        Need::Credential(c) => allowed.credentials.contains(c),
        Need::Env(e) => allowed.env.contains(e),
        Need::Command(_) => true,
        Need::Files => allowed.program.is_some(),
    };
    if !within {
        return Decision::Denied("not declared".into());
    }
    if let Err(permission) = grant.permits(need) {
        return Decision::Denied(format!(
            "needs `{permission}` in the extension's `permissions:`"
        ));
    }
    match enforcement.gap(need) {
        Some(why) => Decision::Unenforced(why.into()),
        None => Decision::Allowed,
    }
}

/// Whether `host` matches `pattern`: exactly, or `*.` meaning subdomains.
fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host.ends_with(&format!(".{domain}")),
        None => pattern == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GrantRequest {
        GrantRequest {
            scopes: ["sql.read".to_string()].into(),
            program: Some("bin/sync.sh".into()),
            hosts: [
                "api.github.com".to_string(),
                "*.githubusercontent.com".to_string(),
            ]
            .into(),
            credentials: ["GH_TOKEN".to_string()].into(),
            ..GrantRequest::default()
        }
    }

    fn granted(permissions: &str) -> Grant {
        Grant::from_permissions("acme", &serde_yaml::from_str(permissions).unwrap())
    }

    const MAC: Enforcement = Enforcement { hosts: true };
    const ELSEWHERE: Enforcement = Enforcement { hosts: false };

    /// A need inside the declaration is decided by what the extension's
    /// permissions allow, and a refusal names the permission.
    #[test]
    fn a_need_is_decided_against_the_extensions_permissions() {
        let g = granted(
            "[process.run, \"network:*.githubusercontent.com\", network:api.github.com, secrets:GH_TOKEN, commands:oxplow.work_item.*]",
        );
        let declared = GrantRequest {
            scopes: ["ai.compute".to_string(), "sql.read".to_string()].into(),
            ..request()
        };
        let d = |need: Need| decide(&need, &declared, &g, MAC);
        assert_eq!(d(Need::Credential("GH_TOKEN".into())), Decision::Allowed);
        assert_eq!(
            d(Need::Host("raw.githubusercontent.com".into())),
            Decision::Allowed
        );
        assert_eq!(d(Need::Program("bin/sync.sh".into())), Decision::Allowed);
        assert_eq!(
            d(Need::Scope("ai.compute".into())),
            Decision::Denied("needs `ai.compute` in the extension's `permissions:`".into())
        );
        assert_eq!(
            d(Need::Scope("sql.read".into())),
            Decision::Denied("needs `sql.read` in the extension's `permissions:`".into())
        );
        // Undeclared stays refused whatever is permitted.
        assert_eq!(
            d(Need::Credential("AWS_KEY".into())),
            Decision::Denied("not declared".into())
        );
        assert_eq!(
            d(Need::Command("oxplow.work_item.transition".into())),
            Decision::Allowed
        );
        assert_eq!(
            d(Need::Command("oxplow.thread.close".into())),
            Decision::Denied(
                "needs `commands:oxplow.thread.close` in the extension's `permissions:`".into()
            )
        );
    }

    /// Its own commands need no permission.
    #[test]
    fn a_command_in_the_own_namespace_needs_no_pattern() {
        let g = granted("[]");
        let d = |c: &str| decide(&Need::Command(c.into()), &GrantRequest::default(), &g, MAC);
        assert_eq!(d("acme.notes.add"), Decision::Allowed);
        assert!(matches!(d("acmex.notes.add"), Decision::Denied(_)));
        assert!(matches!(
            decide(
                &Need::Command("acme.notes.add".into()),
                &GrantRequest::default(),
                &Grant::none(),
                MAC
            ),
            Decision::Denied(_)
        ));
    }

    /// What oxplow can't hold to what was permitted is said, not passed off
    /// as enforced.
    #[test]
    fn what_isnt_enforced_here_says_so() {
        let g = granted("[process.run, network:api.github.com]");
        assert!(matches!(
            decide(
                &Need::Host("api.github.com".into()),
                &request(),
                &g,
                ELSEWHERE
            ),
            Decision::Unenforced(_)
        ));
        match decide(&Need::Files, &request(), &g, MAC) {
            Decision::Unenforced(why) => {
                assert!(why.contains("file, network and process authority"), "{why}");
                assert!(why.contains("doesn't confine"), "{why}");
            }
            other => panic!("a program's files are unenforced, got {other:?}"),
        }
        // No program, no files.
        let none = GrantRequest {
            program: None,
            ..request()
        };
        assert!(matches!(
            decide(&Need::Files, &none, &g, MAC),
            Decision::Denied(_)
        ));
    }

    /// The same request is the same text, whatever order it was built in.
    #[test]
    fn canonical_json_is_stable() {
        let mut a = GrantRequest::default();
        a.hosts.insert("b".into());
        a.hosts.insert("a".into());
        let mut b = GrantRequest::default();
        b.hosts.insert("a".into());
        b.hosts.insert("b".into());
        assert_eq!(a.canonical_json(), b.canonical_json());
    }

    /// A decision records as `{need, name?, decision, reason?}`.
    #[test]
    fn a_decision_records_flat() {
        let d = Decided {
            need: Need::Host("x.com".into()),
            decision: Decision::Unenforced("why".into()),
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({ "need": "host", "name": "x.com", "decision": "unenforced", "reason": "why" })
        );
    }
}
