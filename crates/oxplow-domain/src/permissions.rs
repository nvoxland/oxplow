//! What an extension may reach beyond the baseline, declared in its
//! manifest's `permissions:` and approved when it is deployed
//! (`.context/extensions.md` "Permissions").
//!
//! `permissions:` is a list in the grammar of a script's `needs:` — a
//! scope id, or `<scope>:<qualifier>` — of the scopes a person decides
//! about: `[sql.read, files.read, ai.compute, process.run, agent.write,
//! network:api.github.com, secrets:GH_TOKEN, commands:oxplow.work_item.*]`.
//!
//! The baseline needs no declaration: pages and the rest of the
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
/// nothing. Written (and stored) as the list of [`PERMISSION_IDS`] it
/// grants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
#[specta(type = Vec<String>)]
pub struct Permissions {
    /// Ad-hoc SQL: a script's `scope("sql.read")`, a component's own query.
    pub sql: Option<SqlAccess>,
    /// Commands outside its own namespace it may run, as patterns:
    /// `oxplow.work_item.*` (`*` is one segment, `**` the rest).
    pub commands: Vec<String>,
    /// The project's files outside the bus.
    pub files: Option<FilesAccess>,
    /// Calling AI models, with the person's key.
    pub ai: bool,
    /// The hosts its programs may reach (`api.github.com`, `*.example.com`).
    pub network: Vec<String>,
    /// Running programs (providers, `exec` collectors). They run with the
    /// person's authority; oxplow doesn't confine them.
    pub process: bool,
    /// The secrets it may read, by name.
    pub secrets: Vec<String>,
    /// Contributing to the agent's context: skills, advisories, prompts.
    pub agent: bool,
}

/// The scopes a `permissions:` list may name: each alone, or (those ending
/// in `:`) with its qualifier.
pub const PERMISSION_IDS: &[(&str, &str)] = &[
    ("sql.read", "ad-hoc read-only SQL over the `v_*` models"),
    ("files.read", "read the project's files outside the bus"),
    (
        "files.write",
        "read and write the project's files outside the bus",
    ),
    ("ai.compute", "call AI models with the person's key"),
    (
        "process.run",
        "run programs (providers, `exec` collectors) with the person's authority",
    ),
    (
        "agent.write",
        "add skills, advisories and prompts to the agent's context",
    ),
    (
        "network:",
        "reach a host: `network:api.github.com`, `network:*.example.com`",
    ),
    ("secrets:", "read a secret by name: `secrets:GH_TOKEN`"),
    (
        "commands:",
        "run commands outside its own namespace: `commands:oxplow.work_item.*`",
    ),
];

impl TryFrom<Vec<String>> for Permissions {
    type Error = String;

    fn try_from(ids: Vec<String>) -> Result<Self, String> {
        let mut out = Permissions::default();
        for id in ids {
            match id.split_once(':') {
                Some(("network", host)) if !host.is_empty() => push(&mut out.network, host),
                Some(("secrets", name)) if !name.is_empty() => push(&mut out.secrets, name),
                Some(("commands", pattern)) if !pattern.is_empty() => {
                    push(&mut out.commands, pattern)
                }
                None => match id.as_str() {
                    "sql.read" => out.sql = Some(SqlAccess::Read),
                    "files.read" => out.files = out.files.max(Some(FilesAccess::Read)),
                    "files.write" => out.files = Some(FilesAccess::Write),
                    "ai.compute" => out.ai = true,
                    "process.run" => out.process = true,
                    "agent.write" => out.agent = true,
                    _ => return Err(unknown(&id)),
                },
                Some(_) => return Err(unknown(&id)),
            }
        }
        Ok(out)
    }
}

impl From<Permissions> for Vec<String> {
    fn from(p: Permissions) -> Self {
        let mut out = Vec::new();
        if p.sql.is_some() {
            out.push("sql.read".to_string());
        }
        match p.files {
            Some(FilesAccess::Write) => out.push("files.write".to_string()),
            Some(FilesAccess::Read) => out.push("files.read".to_string()),
            None => {}
        }
        if p.ai {
            out.push("ai.compute".to_string());
        }
        if p.process {
            out.push("process.run".to_string());
        }
        if p.agent {
            out.push("agent.write".to_string());
        }
        out.extend(p.network.iter().map(|h| format!("network:{h}")));
        out.extend(p.secrets.iter().map(|s| format!("secrets:{s}")));
        out.extend(p.commands.iter().map(|c| format!("commands:{c}")));
        out
    }
}

fn push(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|i| i == item) {
        list.push(item.to_string());
    }
}

fn unknown(id: &str) -> String {
    let known: Vec<String> = PERMISSION_IDS
        .iter()
        .map(|(id, _)| {
            if id.ends_with(':') {
                format!("{id}<…>")
            } else {
                id.to_string()
            }
        })
        .collect();
    format!(
        "`{id}` isn't a permission: `permissions:` lists {}",
        known.join(", ")
    )
}

/// The schema of the list a manifest writes.
impl schemars::JsonSchema for Permissions {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Permissions".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let exact: Vec<&str> = PERMISSION_IDS
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !id.ends_with(':'))
            .collect();
        schemars::json_schema!({
            "description": "The scopes a person approves at deploy, in a script's `needs:` grammar: `sql.read`, `files.read`, `files.write`, `ai.compute`, `process.run`, `agent.write`, `network:<host>`, `secrets:<name>`, `commands:<pattern>`.",
            "type": "array",
            "items": {
                "anyOf": [
                    { "enum": exact },
                    { "type": "string", "pattern": "^(network|secrets|commands):.+$" },
                ],
            },
        })
    }
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
    /// would list it in `permissions:` (`` `process.run` ``,
    /// `` `network:api.github.com` ``).
    pub fn missing(&self, needed: &Permissions) -> Vec<String> {
        let mut out = Vec::new();
        let list = |key: &str, items: Vec<&String>| {
            items
                .into_iter()
                .map(|item| format!("`{key}:{item}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if needed.sql.is_some() && self.sql.is_none() {
            out.push("`sql.read`".to_string());
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
                "`files.{}`",
                if want == FilesAccess::Write {
                    "write"
                } else {
                    "read"
                }
            )),
            _ => {}
        }
        if needed.ai && !self.ai {
            out.push("`ai.compute`".to_string());
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
            out.push("`process.run`".to_string());
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
            out.push("`agent.write`".to_string());
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
        assert!(!none.covers(&parse("[process.run]")));
        assert_eq!(
            none.missing(&parse("[process.run, network:api.github.com]")),
            vec!["`network:api.github.com`", "`process.run`"]
        );
    }

    #[test]
    fn write_files_covers_read() {
        assert!(parse("[files.write]").covers(&parse("[files.read]")));
        assert!(!parse("[files.read]").covers(&parse("[files.write]")));
        assert_eq!(
            parse("[files.read]").missing(&parse("[files.write]")),
            vec!["`files.write`"]
        );
    }

    #[test]
    fn added_over_lists_only_what_is_new_and_names_hosts_and_secrets() {
        let before = parse("[network:api.github.com, secrets:GH]");
        let after = parse(
            "[process.run, network:api.github.com, network:uploads.github.com, secrets:GH, secrets:NPM, ai.compute]",
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
        let p = parse("[commands:oxplow.work_item.*, commands:acme_pr.**]");
        assert!(p.command_allowed("mine", "mine.sync"));
        assert!(!p.command_allowed("mine", "minecraft.sync"));
        assert!(p.command_allowed("mine", "oxplow.work_item.transition"));
        assert!(!p.command_allowed("mine", "oxplow.work_item.link.add"));
        assert!(!p.command_allowed("mine", "oxplow.thread.close"));
        assert!(p.command_allowed("mine", "acme_pr.review.approve"));
        assert!(p.covers(&parse("[commands:oxplow.work_item.transition]")));
    }

    #[test]
    fn a_wildcard_host_covers_the_names_under_it() {
        let p = parse("[\"network:*.github.com\"]");
        assert!(p.host_allowed("api.github.com"));
        assert!(!p.host_allowed("github.com"));
        assert!(p.covers(&parse("[\"network:*.api.github.com\"]")));
    }

    /// The list round-trips, and is what a deployment stores.
    #[test]
    fn the_list_round_trips() {
        let text = "[sql.read, files.write, ai.compute, process.run, agent.write, network:api.github.com, secrets:GH, commands:oxplow.work_item.*]";
        let p = parse(text);
        let ids: Vec<String> = p.clone().into();
        assert_eq!(Permissions::try_from(ids.clone()).unwrap(), p);
        assert_eq!(serde_json::to_value(&p).unwrap(), serde_json::json!(ids));
    }

    /// An id that isn't one, or the old map, is refused naming the list.
    #[test]
    fn an_unknown_permission_is_an_error() {
        for bad in [
            "[proces.run]",
            "[sql.write]",
            "[network:]",
            "[ai]",
            "sql: read",
        ] {
            let err = serde_yaml::from_str::<Permissions>(bad)
                .unwrap_err()
                .to_string();
            assert!(!err.is_empty(), "{bad}");
        }
        let err = serde_yaml::from_str::<Permissions>("[proces.run]")
            .unwrap_err()
            .to_string();
        assert!(err.contains("`permissions:` lists sql.read"), "{err}");
    }
}
