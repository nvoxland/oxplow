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
//! The server may instead be one reached over HTTP (P9.B4): `mcp: { url:
//! https://mcp.example.com/mcp, auth: TOKEN }`, `auth` naming the
//! credential sent as its bearer token.
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
/// (`plugin.enabled` would clear another contribution's disable), and no
/// types of its own yet.
pub fn allowed_event_types(capability: &str) -> &'static [(&'static str, u32)] {
    match capability {
        WORK_ITEMS => &[("work_item.recorded", 2)],
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
    /// The provider's id: its program's approval (`<extension>/<id>`) and
    /// the id of its default instance — an instance's id is its refs'
    /// segment (`work_item:<id>:…`) and its commands' namespace
    /// (`<id>.estimate`).
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
    /// Credentials it gets from the keychain, as environment variables:
    /// each a name (a value the person pastes) or `{ name, oauth }` (one
    /// they sign in for; the variable holds the access token).
    #[serde(default)]
    pub credentials: Vec<CredentialDecl>,
    /// Hosts it may reach (enforced where the OS can).
    #[serde(default)]
    pub network: Vec<String>,
    /// The checked-in `InitializeResult` (JSON), relative to the folder.
    pub declarations: String,
    /// What a work list's own ids look like (a regex matched whole:
    /// `[A-Z]+-\d+`), so a loose id resolves to its item while it's the
    /// active work list.
    #[serde(default)]
    pub id_pattern: Option<String>,
    /// A work list's own fields (kept in an item's `native`), so screens
    /// render and edit them: `[{ name, title, kind: enum|text|number,
    /// values? }]`.
    #[serde(default)]
    pub fields: Vec<oxplow_domain::work_items::FieldDecl>,
}

/// How a credential is obtained by signing in (P9.B3): OAuth 2.1's
/// authorization-code flow with PKCE, run by oxplow — the provider only
/// ever sees the access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct OAuthDecl {
    /// Where the person signs in.
    pub authorize_url: String,
    /// Where a code, or a refresh token, is exchanged for an access token.
    pub token_url: String,
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The **name** of another (static) credential of this provider that
    /// holds the client secret, for a service that requires one.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// The loopback port the redirect comes back on, for a service that
    /// wants one registered; any free port otherwise.
    #[serde(default)]
    pub redirect_port: Option<u16>,
    /// How the client secret is sent with a token request: `basic` (an
    /// `Authorization` header, RFC 6749 §2.3.1 — what every service must
    /// take; the default) or `post` (in the form, for a service that wants
    /// that).
    #[serde(default)]
    pub client_auth: ClientAuth,
}

/// How a client authenticates to a token endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuth {
    #[default]
    Basic,
    Post,
}

/// One credential a provider declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct CredentialDecl {
    /// Its name: the environment variable the provider reads it from, and
    /// what its keychain account is named for.
    pub name: String,
    /// Obtained by signing in rather than pasted (P9.B3).
    pub oauth: Option<OAuthDecl>,
}

impl CredentialDecl {
    /// What a person approves, and what the approval's hash covers: the
    /// name, and for a signed-in one where it signs in, where its tokens
    /// come from, as which client and for what — so changing any of them
    /// asks again.
    pub fn grant(&self) -> String {
        let Some(oauth) = &self.oauth else {
            return self.name.clone();
        };
        let mut parts = vec![
            format!("signs in at {}", oauth.authorize_url),
            format!("tokens from {}", oauth.token_url),
            format!("client {}", oauth.client_id),
        ];
        if !oauth.scopes.is_empty() {
            parts.push(format!("scopes {}", oauth.scopes.join(", ")));
        }
        if let Some(secret) = &oauth.client_secret {
            parts.push(match oauth.client_auth {
                ClientAuth::Basic => format!("client secret {secret}"),
                ClientAuth::Post => format!("client secret {secret}, sent in the form"),
            });
        }
        if let Some(port) = oauth.redirect_port {
            parts.push(format!("redirect port {port}"));
        }
        format!("{} ({})", self.name, parts.join("; "))
    }
}

/// In the manifest a credential is a bare name or `{ name, oauth? }`.
impl<'de> Deserialize<'de> for CredentialDecl {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            name: String,
            #[serde(default)]
            oauth: Option<OAuthDecl>,
        }
        struct Either;
        impl<'de> serde::de::Visitor<'de> for Either {
            type Value = CredentialDecl;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a credential's name, or `{ name, oauth }`")
            }
            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<CredentialDecl, E> {
                Ok(CredentialDecl {
                    name: name.to_string(),
                    oauth: None,
                })
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<CredentialDecl, A::Error> {
                let full = Full::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(CredentialDecl {
                    name: full.name,
                    oauth: full.oauth,
                })
            }
        }
        deserializer.deserialize_any(Either)
    }
}

/// What's wrong with how `spec` declares its credentials: a name twice,
/// an OAuth endpoint that isn't https (plain http only on loopback), or a
/// `client_secret` that doesn't name a static credential of its own.
fn credentials_problem(spec: &ProviderSpec) -> Option<String> {
    let id = &spec.id;
    for (i, c) in spec.credentials.iter().enumerate() {
        if spec.credentials[..i].iter().any(|o| o.name == c.name) {
            return Some(format!(
                "provider `{id}`: credential `{}` is declared twice",
                c.name
            ));
        }
        let Some(oauth) = &c.oauth else { continue };
        for (key, value) in [
            ("authorize_url", &oauth.authorize_url),
            ("token_url", &oauth.token_url),
        ] {
            if secure_url(value).is_none() {
                return Some(format!(
                    "provider `{id}`: credential `{}`'s `{key}` must be https (http only on \
                     loopback): `{value}`",
                    c.name
                ));
            }
        }
        if let Some(secret) = &oauth.client_secret {
            let is_static = spec
                .credentials
                .iter()
                .any(|o| o.name == *secret && o.oauth.is_none());
            if !is_static {
                return Some(format!(
                    "provider `{id}`: credential `{}`'s `client_secret: {secret}` must name \
                     another credential of this provider whose value is pasted, not signed in for",
                    c.name
                ));
            }
        }
    }
    None
}

/// An MCP server as a provider (P7.A6): oxplow's adapter runs `mcp`'s
/// server and translates through `mapping`, refusing a server whose tools
/// aren't `tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterSpec {
    pub mcp: McpServer,
    /// The Starlark mapping (`transform(x)`), relative to the folder.
    pub mapping: String,
    /// The pinned tools (`[{ name, description, inputSchema }]`, JSON).
    pub tools: String,
}

/// How the adapter reaches the MCP server: exactly one of the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(untagged)]
pub enum McpServer {
    /// A program in the folder, spoken to over its stdio.
    Command { command: Vec<String> },
    /// A server reached over streamable HTTP (P9.B4). `auth` names the
    /// credential whose value is sent as its bearer token.
    Url { url: String, auth: Option<String> },
}

impl<'de> Deserialize<'de> for McpServer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct File {
            #[serde(default)]
            command: Option<Vec<String>>,
            #[serde(default)]
            url: Option<String>,
            #[serde(default)]
            auth: Option<String>,
        }
        let file = File::deserialize(deserializer)?;
        match (file.command, file.url, file.auth) {
            (Some(command), None, None) => Ok(McpServer::Command { command }),
            (Some(_), None, Some(_)) => Err(serde::de::Error::custom(
                "`auth` goes with a `url`: the bearer token of a server reached over HTTP",
            )),
            (None, Some(url), auth) => Ok(McpServer::Url { url, auth }),
            _ => Err(serde::de::Error::custom(
                "`mcp` names either `command` (a program in the folder) or `url` (a server \
                 reached over HTTP), not both or neither",
            )),
        }
    }
}

/// `value` as a URL a secret may be sent to: https, or plain http on
/// loopback (what a local service speaks).
fn secure_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    (url.scheme() == "https" || (url.scheme() == "http" && loopback)).then_some(url)
}

/// An instance's name: `<extension>/<instance id>` — its config key, its
/// health's key, what Settings → Integrations lists.
pub fn instance_name(extension: &str, id: &str) -> String {
    format!("{extension}/{id}")
}

impl ProviderSpec {
    /// Its credentials' names.
    pub fn credential_names(&self) -> Vec<String> {
        self.credentials.iter().map(|c| c.name.clone()).collect()
    }

    /// Its credentials as an approval lists them ([`CredentialDecl::grant`]),
    /// the one a server by url gets as its bearer token marked so — which
    /// secret leaves the machine for it is part of what is approved
    /// (tsk834).
    pub fn credential_grants(&self) -> Vec<String> {
        let bearer = match self.adapter.as_ref().map(|a| &a.mcp) {
            Some(McpServer::Url { auth, .. }) => auth.as_deref(),
            _ => None,
        };
        self.credentials
            .iter()
            .map(|c| {
                let grant = c.grant();
                if bearer == Some(c.name.as_str()) {
                    format!("{grant} (sent to the server as its bearer token)")
                } else {
                    grant
                }
            })
            .collect()
    }

    /// Whether `name` is a client secret: a credential a signed-in one's
    /// token requests carry. It is the host's to use, never the
    /// provider's process's.
    pub fn is_client_secret(&self, name: &str) -> bool {
        self.credentials
            .iter()
            .filter_map(|c| c.oauth.as_ref())
            .any(|o| o.client_secret.as_deref() == Some(name))
    }

    /// Its program's approval key: `provider:<extension>/<id>` — one
    /// approval however many instances run it (consent is about the code;
    /// instances differ in config and credential values, which it never
    /// covered).
    pub fn approval_name(&self, extension: &str) -> String {
        format!("{extension}/{}", self.id)
    }

    /// The program of its own it runs, relative to the folder, and its
    /// arguments: its `entry` and `args`, or its MCP server's command —
    /// what its approval names.
    pub fn program(&self) -> (String, Vec<String>) {
        match self.adapter.as_ref().map(|a| &a.mcp) {
            Some(McpServer::Command { command }) => command.split_first().map_or_else(
                || (String::new(), Vec::new()),
                |(s, rest)| (s.clone(), rest.to_vec()),
            ),
            // Nothing of its own runs here: what it "runs" is the server
            // at its url.
            Some(McpServer::Url { url, .. }) => (url.clone(), Vec::new()),
            None => (self.entry.clone().unwrap_or_default(), self.args.clone()),
        }
    }

    /// Whether what it runs is a server elsewhere (an MCP server by
    /// `url`) rather than a program of its folder.
    pub fn is_remote(&self) -> bool {
        matches!(
            self.adapter.as_ref().map(|a| &a.mcp),
            Some(McpServer::Url { .. })
        )
    }

    /// What the host executes in the folder: its `entry` with `args`, or
    /// `adapter_bin` (oxplow's MCP adapter) with the adapter's files and
    /// the server's command (after `--`) or its `--url` and the name of
    /// its bearer's credential — every path relative to the folder.
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
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                match &a.mcp {
                    McpServer::Command { command } => {
                        args.push("--".into());
                        args.extend(command.iter().cloned());
                    }
                    McpServer::Url { url, auth } => {
                        args.extend(["--url".to_string(), url.clone()]);
                        if let Some(name) = auth {
                            args.extend(["--auth-env".to_string(), name.clone()]);
                        }
                    }
                }
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
            if !spec.args.is_empty() {
                return Some(format!(
                    "provider `{id}`: `args` go with an `entry`; an adapter's server takes them in `command`"
                ));
            }
            match &adapter.mcp {
                McpServer::Command { command } => {
                    let Some((server, args)) = command.split_first() else {
                        return Some(format!("provider `{id}`: adapter `mcp.command` is empty"));
                    };
                    if !inside(server) {
                        return Some(format!(
                            "provider `{id}`: the MCP server `{server}` must be a program inside the \
                             extension folder (its approval covers what runs)"
                        ));
                    }
                    if let Some(bad) = args.iter().find(|a| !arg_inside(a)) {
                        return Some(format!(
                            "provider `{id}`: server arg `{bad}` names a path outside the extension folder"
                        ));
                    }
                }
                McpServer::Url { url, auth } => {
                    let Some(parsed) = secure_url(url) else {
                        return Some(format!(
                            "provider `{id}`: the MCP server's `url` must be https (http only on \
                             loopback): `{url}`"
                        ));
                    };
                    let host = parsed.host_str().unwrap_or_default();
                    if !crate::net_sandbox::host_allowed(host, &spec.network) {
                        return Some(format!(
                            "provider `{id}`: `network` must list `{host}`, its MCP server's host"
                        ));
                    }
                    if let Some(auth) = auth {
                        if !spec.credentials.iter().any(|c| c.name == *auth) {
                            return Some(format!(
                                "provider `{id}` declares no credential `{auth}` (`mcp.auth` names \
                                 the one sent as the server's bearer token)"
                            ));
                        }
                        // A client secret is the host's to send with token
                        // requests and never reaches the process.
                        if spec.is_client_secret(auth) {
                            return Some(format!(
                                "provider `{id}`: `mcp.auth` names `{auth}`, but `{auth}` is a \
                                 client secret — the host's alone, never sent to the server; \
                                 name the credential the server takes as its bearer token"
                            ));
                        }
                    }
                }
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
        // An operation name is one segment of a command id (`estimate`).
        if !crate::extension_commands::valid_op(&command.name) {
            return Err(format!(
                "provider `{id}` command `{}`: an operation is lowercase letters, digits and \
                 underscores, starting with a letter",
                command.name
            ));
        }
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
        if features.ordering {
            needed.push("reorder");
        }
        if features.lists {
            needed.push("move");
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
        let problem = if let Some(why) = oxplow_domain::work_items::provider_id_problem(&id) {
            Some(format!("provider id {why}"))
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
        } else if let Some(problem) = credentials_problem(&spec) {
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
        } else if let Some(problem) = (!spec.fields.is_empty() && spec.capability != "work_items")
            .then(|| {
                format!(
                    "provider `{id}`: fields are a work list's; `{}` has none",
                    spec.capability
                )
            })
            .or_else(|| {
                oxplow_domain::work_items::fields_problem(&spec.fields)
                    .map(|p| format!("provider `{id}`: {p}"))
            })
        {
            Some(problem)
        } else if let Some(pattern) = &spec.id_pattern {
            match regex::Regex::new(pattern) {
                Err(e) => Some(format!(
                    "provider `{id}`: id_pattern `{pattern}` isn't a regex: {e}"
                )),
                Ok(_) if spec.capability != "work_items" => Some(format!(
                    "provider `{id}`: id_pattern is a work list's; `{}` has no ids",
                    spec.capability
                )),
                Ok(_) => read_declarations(&spec, read).err(),
            }
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
