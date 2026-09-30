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

/// The commands a work-items provider must declare (`link` and `comment`
/// too when its features say so): what `ExternalWorkItems` calls.
pub const WORK_ITEMS_COMMANDS: &[&str] = &["create", "update", "transition"];

/// One declared provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSpec {
    /// The provider's name: its refs' segment (`work_item:<id>:…`) and its
    /// commands' namespace (`<id>.create`).
    pub id: String,
    pub capability: String,
    /// The program, relative to the extension folder.
    pub entry: String,
    #[serde(default)]
    pub args: Vec<String>,
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

impl ProviderSpec {
    /// Its approval key: `provider:<extension>/<id>`.
    pub fn approval_name(&self, extension: &str) -> String {
        format!("{extension}/{}", self.id)
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
        if features.in_progress_opens_effort {
            return Err(format!(
                "provider `{id}`: only oxplow's own tasks open efforts on in_progress"
            ));
        }
        for name in needed {
            if !declared.commands.iter().any(|c| c.name == name) {
                return Err(format!(
                    "provider `{id}` implements work_items but declares no `{name}` command"
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
        } else if !inside(&spec.entry) {
            Some(format!(
                "provider `{id}`: entry `{}` must be a path inside the extension folder",
                spec.entry
            ))
        } else if let Some(bad) = spec.args.iter().find(|a| !arg_inside(a)) {
            Some(format!(
                "provider `{id}`: arg `{bad}` names a path outside the extension folder (or its \
                 manifest or lenses); a provider runs only what its approval covers"
            ))
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
