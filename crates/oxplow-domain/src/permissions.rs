//! What an extension may reach beyond the baseline, declared in its
//! manifest's `permissions:` and approved when it is deployed
//! (`.context/extensions.md` "Permissions").
//!
//! The baseline needs no declaration: lenses, pages and the rest of the
//! declarative UI, its own models, its own commands and the scripts that
//! compose them, a collector's `input:` query. Each permission names one
//! thing a person decides about — running programs with their authority,
//! reaching a host, reading a secret, spending their AI key, steering
//! their agent. The active deployment's permissions are the approved set:
//! a deploy that adds one asks first.

use serde::{Deserialize, Serialize};

/// `sql: read`: ad-hoc read-only SQL over the `v_*` models.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SqlAccess {
    Read,
}

/// `files: read | write`: the project's files outside the bus. `write`
/// includes `read`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum FilesAccess {
    Read,
    Write,
}

/// What an extension may do beyond the baseline. Absent fields grant
/// nothing.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Ad-hoc SQL: a script's `scope("sql.read")`, a component's own query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<SqlAccess>,
    /// Commands outside its own namespace it may run, as patterns:
    /// `oxplow.work_item.*` (`*` is one segment, `**` the rest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    /// The project's files outside the bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesAccess>,
    /// Calling AI models, with the person's key.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ai: bool,
    /// The hosts its programs may reach (`api.github.com`, `*.example.com`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    /// Running programs (providers, `exec` collectors). They run with the
    /// person's authority; oxplow doesn't confine them.
    #[serde(default, skip_serializing_if = "is_false")]
    pub process: bool,
    /// The secrets it may read, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// Contributing to the agent's context: skills, advisories, prompts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub agent: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl Permissions {
    /// Nothing beyond the baseline.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Everything `needed` asks is granted here.
    pub fn covers(&self, needed: &Permissions) -> bool {
        self.missing(needed).is_empty()
    }

    /// What `needed` asks that isn't granted here, each as the manifest
    /// would declare it (`` `permissions.process` ``,
    /// `` `permissions.network: [api.github.com]` ``).
    pub fn missing(&self, needed: &Permissions) -> Vec<String> {
        let mut out = Vec::new();
        let list = |key: &str, items: Vec<&String>| {
            let items: Vec<&str> = items.into_iter().map(String::as_str).collect();
            format!("`permissions.{key}: [{}]`", items.join(", "))
        };
        if needed.sql.is_some() && self.sql.is_none() {
            out.push("`permissions.sql: read`".to_string());
        }
        let commands: Vec<&String> = needed
            .commands
            .iter()
            .filter(|c| !self.commands.iter().any(|p| pattern_covers(p, c)))
            .collect();
        if !commands.is_empty() {
            out.push(list("commands", commands));
        }
        match (needed.files, self.files) {
            (Some(want), have) if have.is_none_or(|have| have < want) => out.push(format!(
                "`permissions.files: {}`",
                if want == FilesAccess::Write {
                    "write"
                } else {
                    "read"
                }
            )),
            _ => {}
        }
        if needed.ai && !self.ai {
            out.push("`permissions.ai`".to_string());
        }
        let hosts: Vec<&String> = needed
            .network
            .iter()
            .filter(|h| !self.network.iter().any(|p| host_covers(p, h)))
            .collect();
        if !hosts.is_empty() {
            out.push(list("network", hosts));
        }
        if needed.process && !self.process {
            out.push("`permissions.process`".to_string());
        }
        let secrets: Vec<&String> = needed
            .secrets
            .iter()
            .filter(|s| !self.secrets.contains(s))
            .collect();
        if !secrets.is_empty() {
            out.push(list("secrets", secrets));
        }
        if needed.agent && !self.agent {
            out.push("`permissions.agent`".to_string());
        }
        out
    }

    /// What `self` grants beyond `baseline`, one line a person reads per
    /// permission: what a deploy asks before it goes live.
    pub fn added_over(&self, baseline: &Permissions) -> Vec<String> {
        let mut out = Vec::new();
        if self.process && !baseline.process {
            out.push(
                "process — runs programs with your authority; oxplow does not confine them"
                    .to_string(),
            );
        }
        let hosts: Vec<&str> = self
            .network
            .iter()
            .filter(|h| !baseline.network.iter().any(|p| host_covers(p, h)))
            .map(String::as_str)
            .collect();
        if !hosts.is_empty() {
            out.push(format!("network — may reach {}", hosts.join(", ")));
        }
        let secrets: Vec<&str> = self
            .secrets
            .iter()
            .filter(|s| !baseline.secrets.contains(s))
            .map(String::as_str)
            .collect();
        if !secrets.is_empty() {
            out.push(format!("secrets — may read {}", secrets.join(", ")));
        }
        if self.ai && !baseline.ai {
            out.push("ai — may call your AI provider with your key".to_string());
        }
        if self.sql.is_some() && baseline.sql.is_none() {
            out.push("sql — may run its own SQL over your data".to_string());
        }
        match (self.files, baseline.files) {
            (Some(FilesAccess::Write), have) if have != Some(FilesAccess::Write) => {
                out.push("files — may read and write the project's files".to_string())
            }
            (Some(FilesAccess::Read), None) => {
                out.push("files — may read the project's files".to_string())
            }
            _ => {}
        }
        let commands: Vec<&str> = self
            .commands
            .iter()
            .filter(|c| !baseline.commands.iter().any(|p| pattern_covers(p, c)))
            .map(String::as_str)
            .collect();
        if !commands.is_empty() {
            out.push(format!("commands — may run {}", commands.join(", ")));
        }
        if self.agent && !baseline.agent {
            out.push("agent — may add skills, advisories and prompts to your agent".to_string());
        }
        out
    }

    /// Everything `self` or `other` grants: what several contributions
    /// need together.
    pub fn union(mut self, other: &Permissions) -> Permissions {
        self.sql = self.sql.or(other.sql);
        self.files = self.files.max(other.files);
        self.ai |= other.ai;
        self.process |= other.process;
        self.agent |= other.agent;
        for (mine, theirs) in [
            (&mut self.commands, &other.commands),
            (&mut self.network, &other.network),
            (&mut self.secrets, &other.secrets),
        ] {
            for item in theirs {
                if !mine.contains(item) {
                    mine.push(item.clone());
                }
            }
        }
        self
    }

    /// May an extension whose namespace is `own_namespace` run command
    /// `id`: one of its own, or one a `commands` pattern names.
    pub fn command_allowed(&self, own_namespace: &str, id: &str) -> bool {
        id.strip_prefix(own_namespace)
            .is_some_and(|rest| rest.starts_with('.'))
            || self.commands.iter().any(|p| pattern_covers(p, id))
    }

    /// May its programs reach `host`.
    pub fn host_allowed(&self, host: &str) -> bool {
        self.network.iter().any(|p| host_covers(p, host))
    }
}

/// Does command pattern `pattern` cover `name` (itself a name or a
/// pattern): segment by segment, `*` one segment, `**` every one left.
fn pattern_covers(pattern: &str, name: &str) -> bool {
    let want: Vec<&str> = pattern.split('.').collect();
    let have: Vec<&str> = name.split('.').collect();
    fn walk(want: &[&str], have: &[&str]) -> bool {
        match (want.first(), have.first()) {
            (Some(&"**"), _) => true,
            (None, None) => true,
            (Some(&w), Some(&h)) => (w == "*" || w == h) && walk(&want[1..], &have[1..]),
            _ => false,
        }
    }
    walk(&want, &have)
}

/// Does host pattern `pattern` cover `host` (a host or a pattern):
/// `*.example.com` covers every name under it, `api.x.example.com` and
/// `*.x.example.com` alike.
fn host_covers(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host.ends_with(&format!(".{domain}")),
        None => pattern == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Permissions {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn empty_covers_nothing_but_empty() {
        let none = Permissions::default();
        assert!(none.is_empty());
        assert!(none.covers(&none));
        assert!(!none.covers(&parse("process: true")));
        assert_eq!(
            none.missing(&parse("process: true\nnetwork: [api.github.com]")),
            vec![
                "`permissions.network: [api.github.com]`",
                "`permissions.process`"
            ]
        );
    }

    #[test]
    fn write_files_covers_read() {
        assert!(parse("files: write").covers(&parse("files: read")));
        assert!(!parse("files: read").covers(&parse("files: write")));
        assert_eq!(
            parse("files: read").missing(&parse("files: write")),
            vec!["`permissions.files: write`"]
        );
    }

    #[test]
    fn added_over_lists_only_what_is_new_and_names_hosts_and_secrets() {
        let before = parse("network: [api.github.com]\nsecrets: [GH]");
        let after = parse(
            "process: true\nnetwork: [api.github.com, uploads.github.com]\nsecrets: [GH, NPM]\nai: true",
        );
        assert_eq!(
            after.added_over(&before),
            vec![
                "process — runs programs with your authority; oxplow does not confine them",
                "network — may reach uploads.github.com",
                "secrets — may read NPM",
                "ai — may call your AI provider with your key",
            ]
        );
        assert!(before.added_over(&after).is_empty());
        assert!(after.added_over(&after).is_empty());
    }

    #[test]
    fn commands_match_own_namespace_without_a_pattern_and_globs_otherwise() {
        let p = parse("commands: [oxplow.work_item.*, acme_pr.**]");
        assert!(p.command_allowed("mine", "mine.sync"));
        assert!(!p.command_allowed("mine", "minecraft.sync"));
        assert!(p.command_allowed("mine", "oxplow.work_item.transition"));
        assert!(!p.command_allowed("mine", "oxplow.work_item.link.add"));
        assert!(!p.command_allowed("mine", "oxplow.thread.close"));
        assert!(p.command_allowed("mine", "acme_pr.review.approve"));
        assert!(p.covers(&parse("commands: [oxplow.work_item.transition]")));
    }

    #[test]
    fn a_wildcard_host_covers_the_names_under_it() {
        let p = parse("network: [\"*.github.com\"]");
        assert!(p.host_allowed("api.github.com"));
        assert!(!p.host_allowed("github.com"));
        assert!(p.covers(&parse("network: [\"*.api.github.com\"]")));
    }

    #[test]
    fn an_unknown_permission_is_an_error() {
        assert!(serde_yaml::from_str::<Permissions>("proces: true").is_err());
        assert!(serde_yaml::from_str::<Permissions>("sql: write").is_err());
    }
}
