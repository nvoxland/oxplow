//! The `providers:` kind of `extension.yaml` (experimental: private
//! extensions only): a program that implements a capability over the
//! provider protocol.
//!
//! ```yaml
//! providers:
//!   - id: fake                     # the ref segment and command namespace
//!     capability: work_items
//!     entry: bin/provider          # a program in the extension folder
//!     args: [--stdio]
//!     env: [TRACKER_URL]           # host variables passed through
//!     credentials: [token]         # keychain values, as env
//!     network: [api.example.com]   # hosts it may reach
//!     declarations: provider.json  # the InitializeResult, checked in
//! ```
//!
//! Or, instead of an `entry`, an MCP server behind oxplow's adapter
//! (P7.A6) — its command, the Starlark mapping and the pinned tools, all
//! in the folder:
//!
//! ```yaml
//!     adapter:
//!       mcp: { command: [bin/notes-server, --stdio] }
//!       mapping: mcp/notes.star
//!       tools: mcp/tools.json
//! ```
//!
//! The declarations file is what a person approves (with the program):
//! the live `initialize` must equal it.

use oxplow_provider_protocol::model::InitializeResult;
use serde::{Deserialize, Serialize};

/// The capabilities an external provider may implement today.
pub const CAPABILITIES: &[&str] = &[WORK_ITEMS];
pub const WORK_ITEMS: &str = "work_items";

/// The event types a provider of each capability may declare (and emit):
/// its capability's projection event, nothing else — no other core type
/// (`provider.enabled` would clear another instance's disable), and no
/// types of its own yet.
pub fn allowed_event_types(capability: &str) -> &'static [(&'static str, u32)] {
    match capability {
        WORK_ITEMS => &[("work_item.recorded", 1)],
        _ => &[],
    }
}

/// The verbs a work-items provider must declare (`link`, `comment` and
/// `delete` too when its features say so): what the dispatching
/// `work_item.*` commands call through `ExternalWorkItems`.
pub const WORK_ITEMS_COMMANDS: &[&str] = &["create", "update", "transition"];

/// One declared provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSpec {
    /// The provider's name: its refs' segment (`work_item:<id>:…`) and its
    /// commands' namespace (`<id>.create`).
    pub id: String,
    pub capability: String,
    /// The program, relative to the extension folder — or `adapter`.
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// An MCP server run through oxplow's adapter, instead of an `entry`.
    #[serde(default)]
    pub adapter: Option<AdapterSpec>,
    /// Host environment variables passed through by name.
    #[serde(default)]
    pub env: Vec<String>,
    /// Credentials it gets from the keychain, as environment variables.
    #[serde(default)]
    pub credentials: Vec<String>,
    /// Hosts it may reach (enforced where the OS can).
    #[serde(default)]
    pub network: Vec<String>,
    /// The checked-in `InitializeResult` (JSON), relative to the folder.
    pub declarations: String,
}

/// An MCP server as a provider (P7.A6): oxplow's adapter runs `mcp`'s
/// server and translates through `mapping`, refusing a server whose tools
/// aren't `tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterSpec {
    pub mcp: McpServerSpec,
    /// The Starlark mapping (`transform(x)`), relative to the folder.
    pub mapping: String,
    /// The pinned tools (`[{ name, description, inputSchema }]`, JSON).
    pub tools: String,
}

/// How the adapter reaches the MCP server: a command in the folder (a
/// server by `url` isn't supported yet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerSpec {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl ProviderSpec {
    /// Its approval key: `provider:<extension>/<id>`.
    pub fn approval_name(&self, extension: &str) -> String {
        format!("{extension}/{}", self.id)
    }

    /// The program of its own it runs, relative to the folder, and its
    /// arguments: its `entry` and `args`, or its MCP server's command —
    /// what its approval names.
    pub fn program(&self) -> (String, Vec<String>) {
        match &self.adapter {
            Some(a) => {
                let (server, args) = a.mcp.command.split_first().map_or_else(
                    || (String::new(), Vec::new()),
                    |(s, rest)| (s.clone(), rest.to_vec()),
                );
                (server, args)
            }
            None => (self.entry.clone().unwrap_or_default(), self.args.clone()),
        }
    }

    /// What the host executes in the folder: its `entry` with `args`, or
    /// `adapter_bin` (oxplow's MCP adapter) with the adapter's files and
    /// the server's command — every path relative to the folder.
    pub fn launch(
        &self,
        adapter_bin: &std::path::Path,
    ) -> (Option<std::path::PathBuf>, Vec<String>) {
        match &self.adapter {
            Some(a) => {
                let mut args: Vec<String> = [
                    "--declarations",
                    &self.declarations,
                    "--mapping",
                    &a.mapping,
                    "--tools",
                    &a.tools,
                    "--",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                args.extend(a.mcp.command.iter().cloned());
                (Some(adapter_bin.to_path_buf()), args)
            }
            None => (None, self.args.clone()),
        }
    }
}

/// A provider as declared: its spec, its checked-in declarations (`None`
/// when they can't be read) and, behind the MCP adapter, its pinned tools.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredProvider {
    pub spec: ProviderSpec,
    pub declarations: Option<InitializeResult>,
    pub tools: Vec<serde_json::Value>,
}

impl DeclaredProvider {
    /// `spec` as the files `read` reads declare it.
    pub fn read(spec: &ProviderSpec, read: &dyn Fn(&str) -> Option<String>) -> DeclaredProvider {
        DeclaredProvider {
            spec: spec.clone(),
            declarations: read_declarations(spec, read).ok(),
            tools: read_pinned_tools(spec, read).unwrap_or_default(),
        }
    }
}

/// An adapter provider's pinned tools: a JSON list, each with a `name`.
/// Empty for a provider with an `entry`.
pub fn read_pinned_tools(
    spec: &ProviderSpec,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let Some(adapter) = &spec.adapter else {
        return Ok(Vec::new());
    };
    let id = &spec.id;
    let text = read(&adapter.tools).ok_or_else(|| {
        format!(
            "provider `{id}`: tools `{}` doesn't exist in the extension",
            adapter.tools
        )
    })?;
    let tools: Vec<serde_json::Value> = serde_json::from_str(&text)
        .ok()
        .filter(|t: &Vec<serde_json::Value>| t.iter().all(|t| t["name"].is_string()))
        .ok_or_else(|| {
            format!(
                "provider `{id}`: tools `{}` must be a JSON list of `{{ name, description, \
                 inputSchema }}` — the server's tools/list, pinned",
                adapter.tools
            )
        })?;
    Ok(tools)
}

/// What's wrong with how `spec` names what runs: exactly one of `entry`
/// and `adapter`, and every file it names inside the folder.
fn program_problem(spec: &ProviderSpec, read: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let id = &spec.id;
    match (&spec.entry, &spec.adapter) {
        (Some(_), Some(_)) | (None, None) => Some(format!(
            "provider `{id}`: name either `entry` or `adapter` (a program, or an MCP server behind oxplow's adapter), not both or neither"
        )),
        (Some(entry), None) => {
            if !inside(entry) {
                Some(format!(
                    "provider `{id}`: entry `{entry}` must be a path inside the extension folder"
                ))
            } else {
                spec.args.iter().find(|a| !arg_inside(a)).map(|bad| {
                    format!(
                        "provider `{id}`: arg `{bad}` names a path outside the extension folder (or its \
                         manifest or lenses); a provider runs only what its approval covers"
                    )
                })
            }
        }
        (None, Some(adapter)) => {
            if adapter.mcp.url.is_some() {
                return Some(format!(
                    "provider `{id}`: an MCP server by `url` isn't supported yet; run it with `command`"
                ));
            }
            if !spec.args.is_empty() {
                return Some(format!(
                    "provider `{id}`: `args` go with an `entry`; an adapter's server takes them in `command`"
                ));
            }
            let Some((server, args)) = adapter.mcp.command.split_first() else {
                return Some(format!("provider `{id}`: adapter `mcp.command` is empty"));
            };
            if !inside(server) {
                return Some(format!(
                    "provider `{id}`: the MCP server `{server}` must be a program inside the extension \
                     folder (its approval covers what runs)"
                ));
            }
            if let Some(bad) = args.iter().find(|a| !arg_inside(a)) {
                return Some(format!(
                    "provider `{id}`: server arg `{bad}` names a path outside the extension folder"
                ));
            }
            for (what, path) in [("mapping", &adapter.mapping), ("tools", &adapter.tools)] {
                if !inside(path) {
                    return Some(format!(
                        "provider `{id}`: {what} `{path}` must be a file inside the extension folder"
                    ));
                }
                if read(path).is_none() {
                    return Some(format!(
                        "provider `{id}`: {what} `{path}` doesn't exist in the extension"
                    ));
                }
            }
            read_pinned_tools(spec, read).err()
        }
    }
}

/// A provider id: lowercase snake_case (it is a command namespace).
fn valid_id(id: &str) -> bool {
    id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A path inside the extension folder that the approval's tree hash
/// covers: relative, no `..`, not the manifest, not under `lenses/`.
fn inside(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && path != "extension.yaml"
        && !p.starts_with("lenses")
}

/// An arg that names a path (it has a `/`, or is `.`/`..`; a flag's
/// `=value` counts) must stay inside the extension folder, since the
/// approval hashes that folder.
fn arg_inside(arg: &str) -> bool {
    let value = match arg.strip_prefix('-') {
        Some(flag) => match flag.split_once('=') {
            Some((_, v)) => v,
            None => return true,
        },
        None => arg,
    };
    let names_path = value.contains('/') || value == "." || value == "..";
    !names_path || inside(value)
}

/// Read and check the declarations a spec points at.
pub fn read_declarations(
    spec: &ProviderSpec,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<InitializeResult, String> {
    let text = read(&spec.declarations).ok_or_else(|| {
        format!(
            "provider `{}`: declarations `{}` doesn't exist in the extension",
            spec.id, spec.declarations
        )
    })?;
    let declared: InitializeResult = serde_json::from_str(&text).map_err(|e| {
        format!(
            "provider `{}`: `{}` isn't an initialize result: {e}",
            spec.id, spec.declarations
        )
    })?;
    check_declarations(spec, &declared)?;
    Ok(declared)
}

/// What the host needs from a provider's declarations: the protocol
/// version, the capability the spec names, and the commands that
/// capability calls, each a valid `<id>.<name>` command.
pub fn check_declarations(spec: &ProviderSpec, declared: &InitializeResult) -> Result<(), String> {
    use oxplow_provider_protocol::model::PROTOCOL_VERSION;
    let id = &spec.id;
    if declared.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "provider `{id}` speaks protocol {}, this oxplow speaks {PROTOCOL_VERSION}",
            declared.protocol_version
        ));
    }
    let Some(capability) = declared
        .capabilities
        .iter()
        .find(|c| c.capability == spec.capability)
    else {
        return Err(format!(
            "provider `{id}` doesn't declare the `{}` capability its manifest names",
            spec.capability
        ));
    };
    let allowed = allowed_event_types(&spec.capability);
    if let Some(t) = declared
        .event_types
        .iter()
        .find(|t| !allowed.contains(&(t.event_type.as_str(), t.v)))
    {
        return Err(format!(
            "provider `{id}` declares `{}@{}`, but a {} provider may emit only {}",
            t.event_type,
            t.v,
            spec.capability,
            allowed
                .iter()
                .map(|(t, v)| format!("`{t}@{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for command in &declared.commands {
        oxplow_domain::CommandSpec::validate_name(&format!("{id}.{}", command.name))
            .map_err(|e| format!("provider `{id}`: {e}"))?;
        crate::providers::host::confirm_of(&command.confirm)
            .and(crate::providers::host::effect_of(&command.effect))
            .map_err(|e| format!("provider `{id}` command `{}`: {e}", command.name))?;
    }
    if spec.capability == WORK_ITEMS {
        let features = crate::providers::work_items::features_of(&capability.features)
            .map_err(|e| format!("provider `{id}`: work_items features: {e}"))?;
        let mut needed: Vec<&str> = WORK_ITEMS_COMMANDS.to_vec();
        if features.links {
            needed.push("link");
        }
        if features.comments {
            needed.push("comment");
        }
        if features.delete {
            needed.push("delete");
        }
        if features.in_progress_opens_effort {
            return Err(format!(
                "provider `{id}`: only oxplow's own tasks open efforts on in_progress"
            ));
        }
        for name in needed {
            if !declared.commands.iter().any(|c| c.name == name) {
                return Err(format!(
                    "provider `{id}` implements work_items but declares no `{name}` verb"
                ));
            }
        }
        // `work_item.<verb>` runs it, and that command's spec — not the
        // verb's — is what is confirmed and gated: a verb records an item
        // and never asks on its own.
        for command in declared
            .commands
            .iter()
            .filter(|c| oxplow_domain::work_items::VERBS.contains(&c.name.as_str()))
        {
            if command.confirm != "never" || command.effect != "record" {
                return Err(format!(
                    "provider `{id}` verb `{}`: a work_items verb is `confirm: never` and \
                     `effect: record` (the `work_item.{}` command calling it is what a person \
                     confirms)",
                    command.name, command.name
                ));
            }
        }
    }
    Ok(())
}

/// Parse an extension's `providers:` block. Valid specs, and a message
/// for each invalid one. `read` reads a file inside the extension folder.
pub fn parse_providers(
    value: &serde_yaml::Value,
    read: &dyn Fn(&str) -> Option<String>,
) -> (Vec<ProviderSpec>, Vec<String>) {
    let mut specs: Vec<ProviderSpec> = Vec::new();
    let mut errors = Vec::new();
    let Some(items) = value.as_sequence() else {
        return (specs, vec!["`providers` must be a list".into()]);
    };
    for item in items {
        let spec: ProviderSpec = match serde_yaml::from_value(item.clone()) {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("provider: {e}"));
                continue;
            }
        };
        let id = spec.id.clone();
        let problem = if !valid_id(&id) {
            Some(format!(
                "provider id `{id}` must be lowercase letters, digits and underscores"
            ))
        } else if id == crate::work_items::PROVIDER
            || oxplow_domain::events::schema::CORE_NAMESPACES.contains(&id.as_str())
        {
            Some(format!("provider id `{id}` is reserved for oxplow"))
        } else if specs.iter().any(|s| s.id == id) {
            Some(format!("provider id `{id}` is declared twice"))
        } else if !CAPABILITIES.contains(&spec.capability.as_str()) {
            Some(format!(
                "provider `{id}`: capability `{}` isn't one a provider can implement ({})",
                spec.capability,
                CAPABILITIES.join(", ")
            ))
        } else if let Some(problem) = program_problem(&spec, read) {
            Some(problem)
        } else if !inside(&spec.declarations) {
            Some(format!(
                "provider `{id}`: declarations `{}` must be a file inside the extension folder",
                spec.declarations
            ))
        } else if let Some(bad) = spec
            .network
            .iter()
            .find(|h| !crate::net_sandbox::valid_host_pattern(h))
        {
            Some(format!("provider `{id}`: `{bad}` isn't a host pattern"))
        } else {
            read_declarations(&spec, read).err()
        };
        match problem {
            Some(p) => errors.push(p),
            None => specs.push(spec),
        }
    }
    (specs, errors)
}
