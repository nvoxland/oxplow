//! What a contribution asks to do, what a person granted, and what each
//! call was decided (`.context/extensions.md` "Grants").
//!
//! A [`GrantRequest`] is derived from what a contribution declares — the
//! scopes it calls (a model among them, `ai.compute`), the program it
//! runs, the hosts it reaches, the credentials and environment it reads. A
//! declaration is never a grant: a [`Grant`] is a person's approval of
//! exactly a request, on this machine, until they revoke it or the request
//! changes. A run decides each thing it does against its grant (or, for a
//! contribution that needs no approval, its declaration) and records the
//! [`Decision`] — including where oxplow can't enforce what was granted.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

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

    /// What `self` asks beyond `granted`, named — empty when it asks for no
    /// more (a narrower request needs no approval again).
    pub fn widening(&self, granted: &GrantRequest) -> Vec<String> {
        let mut out = Vec::new();
        let more = |label: &str, want: &BTreeSet<String>, have: &BTreeSet<String>| {
            want.difference(have)
                .map(|w| format!("{label} {w}"))
                .collect::<Vec<_>>()
        };
        out.extend(
            self.scopes
                .iter()
                .filter(|need| {
                    !granted.scopes.iter().any(|have| {
                        have == *need
                            || (crate::service::need_scope(need) != need.as_str()
                                && have == crate::service::need_scope(need))
                            || (need
                                .strip_prefix("tree.read:")
                                .zip(have.strip_prefix("tree.read:"))
                                .is_some_and(|(want, prefix)| {
                                    prefix.ends_with('/') && want.starts_with(prefix)
                                }))
                    })
                })
                .map(|need| format!("calls {need}")),
        );
        out.extend(more("reaches", &self.hosts, &granted.hosts));
        out.extend(more(
            "reads credential",
            &self.credentials,
            &granted.credentials,
        ));
        out.extend(more("reads env", &self.env, &granted.env));
        out.extend(more("runs command", &self.commands, &granted.commands));
        if self.program != granted.program {
            if let Some(p) = &self.program {
                out.push(format!("runs {p}"));
            }
        }
        out
    }
}

/// Who granted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantedBy {
    /// A person, in Settings → Data → Programs.
    Human,
}

/// A person's approval of exactly `request`, on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub request: GrantRequest,
    pub granted_at: Timestamp,
    pub granted_by: GrantedBy,
    /// When a person revoked it: from then, nothing it granted is allowed.
    pub revoked_at: Option<Timestamp>,
}

impl Grant {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// One thing a run does that its grant decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "need", content = "name", rename_all = "snake_case")]
pub enum Need {
    Scope(String),
    /// A published model, including a derived model's inputs.
    Model(String),
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

/// Decide `need` against what a contribution may do: its `grant` when it
/// runs on a person's approval, else its `declared` request (a contribution
/// that needs no approval may do what it declares, no more).
pub fn decide(
    need: &Need,
    grant: Option<&Grant>,
    declared: &GrantRequest,
    enforcement: Enforcement,
) -> Decision {
    if let Some(g) = grant {
        if g.is_revoked() {
            return Decision::Denied("its approval was revoked".into());
        }
    }
    let allowed = grant.map_or(declared, |g| &g.request);
    let within = match need {
        Need::Scope(s) => allowed
            .scopes
            .iter()
            .any(|n| crate::service::need_scope(n) == s),
        Need::Model(model) => {
            allowed.scopes.contains("sql.read")
                || allowed.scopes.contains(&format!("sql.read:{model}"))
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
        Need::Command(c) => allowed.commands.contains(c),
        Need::Files => allowed.program.is_some(),
    };
    if !within {
        return Decision::Denied(match grant {
            Some(_) => "not in what was approved".into(),
            None => "not declared".into(),
        });
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

    fn granted(request: GrantRequest) -> Grant {
        Grant {
            request,
            granted_at: Timestamp::now(),
            granted_by: GrantedBy::Human,
            revoked_at: None,
        }
    }

    const MAC: Enforcement = Enforcement { hosts: true };
    const ELSEWHERE: Enforcement = Enforcement { hosts: false };

    /// A need inside the grant is allowed; outside it, denied.
    #[test]
    fn a_need_is_decided_against_the_grant() {
        let g = granted(request());
        let d = |need: Need| decide(&need, Some(&g), &GrantRequest::default(), MAC);
        assert_eq!(d(Need::Scope("sql.read".into())), Decision::Allowed);
        assert_eq!(d(Need::Credential("GH_TOKEN".into())), Decision::Allowed);
        assert_eq!(
            d(Need::Host("raw.githubusercontent.com".into())),
            Decision::Allowed
        );
        assert!(matches!(
            d(Need::Host("evil.com".into())),
            Decision::Denied(_)
        ));
        assert!(matches!(
            d(Need::Credential("AWS_KEY".into())),
            Decision::Denied(_)
        ));
        assert!(matches!(
            d(Need::Scope("ai.compute".into())),
            Decision::Denied(_)
        ));
    }

    /// What oxplow can't hold to the grant is said, not passed off as
    /// enforced.
    #[test]
    fn what_isnt_enforced_here_says_so() {
        let g = granted(request());
        assert!(matches!(
            decide(
                &Need::Host("api.github.com".into()),
                Some(&g),
                &GrantRequest::default(),
                ELSEWHERE
            ),
            Decision::Unenforced(_)
        ));
        match decide(&Need::Files, Some(&g), &GrantRequest::default(), MAC) {
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
            decide(
                &Need::Files,
                Some(&granted(none)),
                &GrantRequest::default(),
                MAC
            ),
            Decision::Denied(_)
        ));
    }

    /// A revoked grant allows nothing.
    #[test]
    fn a_revoked_grant_allows_nothing() {
        let mut g = granted(request());
        g.revoked_at = Some(Timestamp::now());
        assert_eq!(
            decide(
                &Need::Scope("sql.read".into()),
                Some(&g),
                &GrantRequest::default(),
                MAC
            ),
            Decision::Denied("its approval was revoked".into())
        );
    }

    /// Without an approval, what it declares is what it may do.
    #[test]
    fn without_a_grant_the_declaration_decides() {
        let declared = GrantRequest {
            scopes: ["sql.read".to_string()].into(),
            ..GrantRequest::default()
        };
        assert_eq!(
            decide(&Need::Scope("sql.read".into()), None, &declared, MAC),
            Decision::Allowed
        );
        assert_eq!(
            decide(&Need::Scope("ai.compute".into()), None, &declared, MAC),
            Decision::Denied("not declared".into())
        );
    }

    /// Narrowing asks nothing new; widening names what's new.
    #[test]
    fn a_narrower_request_needs_no_new_approval() {
        let granted = request();
        let mut narrower = request();
        narrower.hosts.remove("*.githubusercontent.com");
        assert!(narrower.widening(&granted).is_empty());
        let mut wider = request();
        wider.hosts.insert("evil.com".into());
        wider.scopes.insert("ai.compute".into());
        assert_eq!(
            wider.widening(&granted),
            vec!["calls ai.compute", "reaches evil.com"]
        );
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
