//! The `commands:` kind of `extension.yaml` (P6b, `.context/extensions.md`
//! → "Commands"): a command whose handler is a Starlark script that
//! composes core commands.
//!
//! ```yaml
//! commands:
//!   - name: review.finish                # registered as <namespace>.review.finish
//!     summary: Mark the task done and leave a note.
//!     input_schema: { type: object, required: [uri], properties: { uri: { type: string } } }
//!     entry: handlers/finish_review.star # defines transform({ input })
//!     needs: [sql.read]                  # the scopes it calls
//!     confirm: never                     # never | always | destructive
//!     access: write                      # write | record | read
//!     examples:
//!       - { name: happy, input: { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }, expect_commands: [oxplow.work_item.transition] }
//!       - { name: gone, input: { uri: "oxplow:oxplow/work_item?id=tsk9&provider=oxplow" }, answers: { sql.read: [[]] }, refused: no such task }
//! ```
//!
//! The script reads with `scope("sql.read", { sql, params })` (one
//! of its `needs`; `crate::scope_calls`) and returns `{ commands:
//! [{ name, input }], result? }`, or `{ refuse: "<why>" }` to decline (the
//! run is `Invalid` with that reason). The namespace is the extension's
//! name with `-` → `_`.

use std::collections::BTreeMap;

use oxplow_domain::{Access, CommandCall, CommandSpec, Confirm, InputValidator};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::extensions::manifest_v2::{at, item_lines, key_line};
use crate::extensions::{CommandSchemas, Extension};

/// An example run of a command: its input, and the commands its script
/// should compose, in order — or the refusal it should make (checked by
/// `oxplow extension check` / Settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandExample {
    pub name: String,
    #[specta(type = oxplow_domain::Json)]
    pub input: Value,
    /// Answers standing in for the scopes' own, per scope in call order
    /// (so the example doesn't depend on the project's data); a scope
    /// without any is served for real.
    #[specta(type = BTreeMap<String, Vec<oxplow_domain::Json>>)]
    pub answers: BTreeMap<String, Vec<Value>>,
    pub expect_commands: Vec<String>,
    /// A part of the reason the script should refuse with.
    pub refused: Option<String>,
}

/// A command an extension declares (valid ones; invalid ones are in the
/// extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCommand {
    /// Its name on the bus: `<namespace>.<name>`.
    pub name: String,
    pub summary: String,
    /// Its input's schema; `None` for one backed by a scope's
    /// operation (the operation's).
    #[specta(type = Option<oxplow_domain::Json>)]
    pub input_schema: Option<Value>,
    /// What runs it.
    pub handler: CommandHandler,
    pub confirm: Confirm,
    pub access: Access,
    /// A durable job's model may call it (`job: true`, a handler of the
    /// extension's own). Anyone else may run every command.
    pub job: bool,
    /// The scopes its script calls, and the capabilities (or features)
    /// it needs active, as core's commands declare them
    /// (`oxplow_domain::capability::check_need`).
    pub needs: Vec<String>,
    /// How a person meets it (label, group, …), as core's commands do.
    pub ui: Option<oxplow_domain::CommandUi>,
    /// Stable: the commands of an extension that ships with oxplow are;
    /// any other's are experimental.
    pub stable: bool,
    pub examples: Vec<CommandExample>,
    /// Its example input (`example:`, else a script's first example that
    /// runs): JSON Schema's `examples` on the input schema it registers
    /// with — what `list_commands` shows an agent and a refusal quotes.
    #[specta(type = Option<oxplow_domain::Json>)]
    pub example: Option<Value>,
    /// The MCP tool it is also offered as (`tool:`), prefixed with its
    /// namespace: `oxplow_query_sql` ([`tool_name`]).
    pub tool: Option<String>,
    /// The inputs kept out of its record (`CommandSpec::unrecorded`).
    pub unrecorded: Vec<String>,
}

/// What runs an extension's command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CommandHandler {
    /// A Starlark script composing commands (`entry:`).
    Script {
        /// The script, relative to the extension folder.
        entry: String,
        /// The script's text, read at load.
        #[serde(skip)]
        script: String,
    },
    /// One operation of a scope (`scope:` + `op:`,
    /// `commands::ops`).
    Scope { scope: String, op: String },
    /// One operation of one of the extension's providers (`provider:` +
    /// `op:`): run on the instance its input names
    /// (`commands::ProviderRouter`).
    Provider {
        provider: String,
        op: String,
        /// It returns an inverse (its declarations say).
        undoable: bool,
    },
}

/// What a provider declares of operation `op` (its checked-in
/// declarations): the declaration, or why there isn't one to declare a
/// command over.
pub type ProviderOps<'a> =
    dyn Fn(&str, &str) -> Result<oxplow_provider_protocol::model::CommandDecl, String> + 'a;

/// No providers (oxplow's own extensions).
pub fn no_providers(
    provider: &str,
    _op: &str,
) -> Result<oxplow_provider_protocol::model::CommandDecl, String> {
    Err(format!("no provider `{provider}` in this extension"))
}

/// `schema` with an optional `instance` field: which of the provider's
/// instances runs it, when its input has no `uri` to say.
fn with_instance(mut schema: Value) -> Value {
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        props.insert(
            "instance".into(),
            json!({
                "type": "string",
                "description": "The instance that runs it (its id); else the one its `uri` names, else the provider's own."
            }),
        );
    }
    schema
}

/// A `commands:` entry as the manifest holds it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandFile {
    name: String,
    summary: String,
    #[serde(default)]
    input_schema: Option<Value>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    confirm: Option<String>,
    #[serde(default)]
    access: Option<String>,
    /// A durable job's model may call it.
    #[serde(default)]
    job: bool,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    ui: Option<oxplow_domain::CommandUi>,
    /// `experimental` keeps a bundled extension's command experimental.
    #[serde(default)]
    lifecycle: Option<String>,
    #[serde(default)]
    examples: Vec<ExampleFile>,
    /// An input that fits its schema: what an agent is shown beside the
    /// schema and a refused input quotes. A script's defaults to its
    /// first example that runs.
    #[serde(default)]
    example: Option<Value>,
    /// Offer it to agents as an MCP tool too, named
    /// `<namespace>_<tool>` ([`tool_name`]).
    #[serde(default)]
    tool: Option<String>,
    /// Its input's properties kept out of its record (a token, a key), or
    /// `*` for all of it.
    #[serde(default)]
    unrecorded: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExampleFile {
    name: String,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    answers: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    expect_commands: Vec<String>,
    #[serde(default)]
    refused: Option<String>,
}

/// An operation's name — a provider's command, a scope's op: one
/// snake_case segment.
pub fn valid_op(name: &str) -> bool {
    valid_segment(name)
}

/// The longest `tool:` name, before its namespace prefix.
const TOOL_NAME_MAX: usize = 48;

/// The MCP tool a `tool: <name>` entry declares: `<namespace>_<name>`, so
/// every extension's tools — oxplow's own as `oxplow_*` — are told apart
/// by the extension's namespace; only core's `list_commands` and
/// `run_command` go unprefixed.
pub fn tool_name(namespace: &str, name: &str) -> Result<String, String> {
    if !valid_segment(name) || name.len() > TOOL_NAME_MAX {
        return Err(format!(
            "tool name `{name}` must be lowercase letters, digits and underscores, starting \
             with a letter, at most {TOOL_NAME_MAX} characters (it is offered as \
             `{namespace}_<name>`)"
        ));
    }
    Ok(format!("{namespace}_{name}"))
}

/// Tool names offered more than once among the enabled extensions — by
/// two entries of one extension, or by two extensions under one namespace
/// (oxplow's own share `oxplow`): each owner is told, and none offers it.
pub fn refuse_duplicate_tools(extensions: &mut [Extension]) {
    let mut owners: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (i, e) in extensions.iter().enumerate() {
        if !e.enabled || e.public_contracts.blocked {
            continue;
        }
        let commands = e
            .commands
            .iter()
            .filter_map(|c| Some((c.tool.clone()?, format!("command `{}`", c.name))));
        let pages = e
            .page_files
            .iter()
            .filter_map(|p| Some((p.tool.clone()?, format!("page `{}`", p.id))));
        for (tool, owner) in commands.chain(pages) {
            owners.entry(tool).or_default().push((i, owner));
        }
    }
    for (tool, by) in owners.into_iter().filter(|(_, by)| by.len() > 1) {
        let mut exts: Vec<usize> = by.iter().map(|(i, _)| *i).collect();
        exts.dedup();
        let message = if exts.len() == 1 {
            let entries: Vec<&str> = by.iter().map(|(_, owner)| owner.as_str()).collect();
            format!(
                "tool `{tool}` is offered twice: by {}",
                entries.join(" and ")
            )
        } else {
            let names: Vec<String> = exts
                .iter()
                .map(|&i| format!("`{}`", extensions[i].name))
                .collect();
            format!(
                "tool `{tool}` is offered by {}; give one another `tool:` name",
                names.join(" and ")
            )
        };
        for &i in &exts {
            let ext = &mut extensions[i];
            ext.errors
                .push(format!("{}/extension.yaml: {message}", ext.path));
            for c in &mut ext.commands {
                if c.tool.as_deref() == Some(tool.as_str()) {
                    c.tool = None;
                }
            }
            for p in &mut ext.page_files {
                if p.tool.as_deref() == Some(tool.as_str()) {
                    p.tool = None;
                }
            }
        }
    }
}

fn valid_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A manifest's `namespace:`: one snake_case segment, and `oxplow` only
/// for an extension that ships with oxplow (`ships_with_oxplow`).
pub fn check_namespace(namespace: &str, ships_with_oxplow: bool) -> Result<(), String> {
    if !valid_segment(namespace) {
        return Err(format!(
            "`namespace` `{namespace}` must be lowercase letters, digits and underscores, \
             starting with a letter"
        ));
    }
    if namespace == oxplow_domain::OXPLOW_NAMESPACE && !ships_with_oxplow {
        return Err(format!(
            "`namespace: {namespace}` is reserved for the extensions that ship with oxplow"
        ));
    }
    Ok(())
}

/// A manifest command's `name`: `<area>.<verb>`, each snake_case — its id
/// is `<namespace>.<area>.<verb>`.
fn valid_command_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 2 && parts.iter().all(|p| valid_segment(p))
}

/// A path inside the extension folder.
fn inside(rel: &str) -> bool {
    let p = std::path::Path::new(rel);
    !rel.is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Parse an extension's `commands:` block: the valid commands, and an
/// error (`file:line: …`) for each broken one. `manifest` is the
/// manifest's text (for lines); `read` reads a file in the extension.
pub fn parse_commands(
    namespace: &str,
    bundled: bool,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
    provider_ops: &ProviderOps<'_>,
) -> (Vec<ExtensionCommand>, Vec<String>) {
    let block_line = key_line(manifest, "commands");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block_line, "`commands` must be a list")],
        );
    };
    let mut out: Vec<ExtensionCommand> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "commands");
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block_line);
        let entry: CommandFile = match serde_yaml::from_value(item.clone()) {
            Ok(e) => e,
            Err(e) => {
                errors.push(at(file, line, format!("command: {e}")));
                continue;
            }
        };
        match command_of(namespace, bundled, entry, read, provider_ops) {
            Ok(c) if out.iter().any(|o| o.name == c.name) => errors.push(at(
                file,
                line,
                format!("command `{}` is declared twice", c.name),
            )),
            // A provider's inverse names its operation, and maps back to
            // the one command over it (`CommandBus::command_for_op`).
            Ok(c) if same_provider_op(&out, &c).is_some() => {
                let (provider, op, other) = same_provider_op(&out, &c).expect("matched");
                errors.push(at(
                    file,
                    line,
                    format!(
                        "`{provider}`'s `{op}` already backs `{other}` — one command per provider \
                         operation (its inverse names the operation)"
                    ),
                ))
            }
            Ok(c) => out.push(c),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

/// The provider operation `c` runs, when one of `out` already runs it:
/// `(provider, op, that command's name)`.
fn same_provider_op<'a>(
    out: &'a [ExtensionCommand],
    c: &'a ExtensionCommand,
) -> Option<(&'a str, &'a str, &'a str)> {
    let CommandHandler::Provider { provider, op, .. } = &c.handler else {
        return None;
    };
    out.iter().find_map(|o| match &o.handler {
        CommandHandler::Provider {
            provider: p, op: q, ..
        } if p == provider && q == op => Some((provider.as_str(), op.as_str(), o.name.as_str())),
        _ => None,
    })
}

fn command_of(
    namespace: &str,
    bundled: bool,
    f: CommandFile,
    read: &dyn Fn(&str) -> Option<String>,
    provider_ops: &ProviderOps<'_>,
) -> Result<ExtensionCommand, String> {
    if !valid_command_name(&f.name) {
        return Err(format!(
            "command name `{}` must be `<area>.<verb>` — lowercase letters, digits and \
             underscores, each starting with a letter (its id is `{namespace}.<area>.<verb>`)",
            f.name
        ));
    }
    let name = format!("{namespace}.{}", f.name);
    CommandSpec::validate_id(&name).map_err(|e| e.to_string())?;
    let at_name = |m: String| format!("command `{}`: {m}", f.name);
    let mut confirm = match f.confirm.as_deref() {
        None | Some("never") => Confirm::Never,
        Some("always") => Confirm::Always,
        Some("destructive") => Confirm::Destructive,
        Some(other) => {
            return Err(at_name(format!(
                "`confirm` must be `never`, `always` or `destructive`, not `{other}`"
            )))
        }
    };
    for need in &f.needs {
        oxplow_domain::capability::check_need(need).map_err(at_name)?;
    }
    if let Some((field, message)) = f.ui.as_ref().and_then(|ui| ui.problem()) {
        return Err(at_name(format!("`{field}`: {message}")));
    }
    let handlers = [f.entry.is_some(), f.scope.is_some(), f.provider.is_some()];
    if handlers.iter().filter(|h| **h).count() > 1 {
        return Err(at_name(
            "declares more than one of `entry:`, `scope:` and `provider:` — a command has \
             one handler"
                .into(),
        ));
    }
    let (handler, input_schema, access) = match (f.scope, f.entry) {
        (None, None) if f.provider.is_none() => {
            return Err(at_name(
                "has no handler: give `entry:` (a Starlark script), `scope:` + `op:` (an \
                 operation of a scope) or `provider:` + `op:` (one of your provider's)"
                    .into(),
            ))
        }
        (None, None) => {
            let provider = f.provider.expect("a provider command");
            let op = f.op.ok_or_else(|| {
                at_name(format!(
                    "`provider: {provider}` needs `op:`, the operation it runs"
                ))
            })?;
            for (given, key) in [
                (f.input_schema.is_some(), "input_schema"),
                (f.access.is_some(), "access"),
                (!f.examples.is_empty(), "examples"),
            ] {
                if given {
                    return Err(at_name(format!(
                        "`{key}` comes from the provider's declarations — drop it"
                    )));
                }
            }
            let decl = provider_ops(&provider, &op)
                .map_err(|e| at_name(format!("`provider: {provider}`, `op: {op}`: {e}")))?;
            let access = crate::providers::host::access_of(&decl.access).map_err(at_name)?;
            // The manifest may ask more than the provider does, never less.
            let declared = crate::providers::host::confirm_of(&decl.confirm).map_err(at_name)?;
            if f.confirm.is_none() || stronger(declared, confirm) {
                confirm = declared;
            }
            (
                CommandHandler::Provider {
                    provider,
                    op,
                    undoable: decl.undoable,
                },
                Some(with_instance(decl.input_schema)),
                access,
            )
        }
        (Some(scope), None) => {
            let access = oxplow_domain::scope::scope(&scope)
                .ok_or_else(|| at_name(format!("`scope`: no scope `{scope}`")))?
                .access;
            let op = f.op.filter(|op| valid_segment(op)).ok_or_else(|| {
                at_name(format!(
                    "`scope: {scope}` needs `op:`, the operation it runs"
                ))
            })?;
            for (given, key) in [
                (f.input_schema.is_some(), "input_schema"),
                (f.access.is_some(), "access"),
                (!f.examples.is_empty(), "examples"),
            ] {
                if given {
                    return Err(at_name(format!(
                        "`{key}` comes from the scope's operation — drop it"
                    )));
                }
            }
            (CommandHandler::Scope { scope, op }, None, access)
        }
        (Some(_), Some(_)) => unreachable!("refused above"),
        (None, Some(entry)) => {
            // Its script calls the scopes it needs; a capability need
            // gates it.
            for need in f.needs.iter().filter(|n| {
                oxplow_domain::scope::scope(oxplow_domain::service::need_scope(n)).is_some()
            }) {
                crate::scope_calls::check_command_need(need).map_err(at_name)?;
            }
            if f.op.is_some() {
                return Err(at_name("`op:` names an operation of a `scope:`".into()));
            }
            let input_schema = f
                .input_schema
                .ok_or_else(|| at_name("a script's command declares `input_schema`".into()))?;
            InputValidator::compile(&input_schema)
                .map_err(|e| at_name(format!("`input_schema` doesn't compile: {e}")))?;
            let access = script_access(&f.access, &f.confirm, &f.needs).map_err(at_name)?;
            if !inside(&entry) {
                return Err(at_name(format!(
                    "entry `{entry}` must be a path inside the extension folder"
                )));
            }
            let script = read(&entry)
                .ok_or_else(|| at_name(format!("entry `{entry}` isn't a file in the extension")))?;
            oxplow_script::runtime::check_starlark(&entry, &script)
                .map_err(|e| at_name(format!("entry `{entry}` {e}")))?;
            crate::services::check_uses(&script, &f.needs, crate::scope_calls::check_command_need)
                .map_err(|e| at_name(format!("entry `{entry}` {e}")))?;
            (
                CommandHandler::Script { entry, script },
                Some(input_schema),
                access,
            )
        }
    };
    if f.examples.len() > MAX_EXAMPLES {
        return Err(at_name(format!(
            "declares {} examples; a command has at most {MAX_EXAMPLES} examples",
            f.examples.len()
        )));
    }
    // Stable when it ships with oxplow, unless it says it's experimental;
    // any other extension's are experimental.
    let stable = match f.lifecycle.as_deref() {
        None | Some("stable") => bundled,
        Some("experimental") => false,
        Some(other) => {
            return Err(at_name(format!(
                "`lifecycle` must be `stable` or `experimental`, not `{other}`"
            )))
        }
    };
    // Its example input: the declared one, else a script's first example
    // that runs (not one expecting a refusal).
    let example = f.example.or_else(|| {
        f.examples
            .iter()
            .find(|e| e.refused.is_none())
            .map(|e| e.input.clone())
    });
    let tool = f
        .tool
        .map(|t| tool_name(namespace, &t))
        .transpose()
        .map_err(at_name)?;
    let properties = input_schema
        .as_ref()
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object);
    if let Some(name) = f
        .unrecorded
        .iter()
        .find(|k| k.as_str() != "*" && !properties.is_some_and(|p| p.contains_key(k.as_str())))
    {
        return Err(at_name(format!(
            "`unrecorded` names `{name}`, which isn't a property of its input"
        )));
    }
    Ok(ExtensionCommand {
        example,
        tool,
        unrecorded: f.unrecorded,
        name,
        summary: f.summary,
        input_schema,
        handler,
        confirm,
        access,
        job: f.job,
        needs: f.needs,
        ui: f.ui,
        stable,
        examples: f
            .examples
            .into_iter()
            .map(|e| {
                if e.refused.is_some() && !e.expect_commands.is_empty() {
                    return Err(format!(
                        "example `{}` expects commands and a refusal — fix: one or the other",
                        e.name
                    ));
                }
                Ok(CommandExample {
                    name: e.name,
                    input: e.input,
                    answers: e.answers,
                    expect_commands: e.expect_commands,
                    refused: e.refused,
                })
            })
            .collect::<Result<_, _>>()?,
    })
}

/// `a` asks a person more than `b` does.
fn stronger(a: Confirm, b: Confirm) -> bool {
    let rank = |c: Confirm| match c {
        Confirm::Never => 0,
        Confirm::Destructive => 1,
        Confirm::Always => 2,
    };
    rank(a) > rank(b)
}

/// A script's declared `access` (default `write`): `read` only when it
/// needs nothing that changes things, and is never confirmed.
fn script_access(
    access: &Option<String>,
    confirm: &Option<String>,
    needs: &[String],
) -> Result<Access, String> {
    let access = match access.as_deref() {
        None | Some("write") => Access::Write,
        Some("record") => Access::Record,
        Some("read") => Access::Read,
        Some(other) => {
            return Err(format!(
                "`access` must be `write`, `record` or `read`, not `{other}`"
            ))
        }
    };
    if access.reads_only() {
        use oxplow_domain::scope::scope;
        if let Some(writes) = needs
            .iter()
            .find(|n| scope(n).is_some_and(|c| c.access.records()))
        {
            return Err(format!(
                "`access: read` but it needs `{writes}`, which changes things"
            ));
        }
        if confirm.as_deref().is_some_and(|c| c != "never") {
            return Err("`access: read`: a command that only reads is never confirmed".into());
        }
    }
    Ok(access)
}

/// Two enabled extensions whose commands map to one namespace: both are
/// refused (an error on each, their commands dropped).
pub fn refuse_shared_namespaces(extensions: &mut [Extension]) {
    let mut by_ns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in extensions.iter().enumerate() {
        // `oxplow` is shared by oxplow's own extensions (only they may
        // declare it); any other namespace is one extension's.
        if e.enabled
            && !e.public_contracts.blocked
            && !e.commands.is_empty()
            && e.namespace != oxplow_domain::OXPLOW_NAMESPACE
        {
            by_ns.entry(e.namespace.clone()).or_default().push(i);
        }
    }
    for (ns, owners) in by_ns.into_iter().filter(|(_, o)| o.len() > 1) {
        let names: Vec<String> = owners
            .iter()
            .map(|&i| format!("`{}`", extensions[i].name))
            .collect();
        for &i in &owners {
            let ext = &mut extensions[i];
            ext.errors.push(format!(
                "{}/extension.yaml: {} both register commands under `{ns}` (an extension's \
                 `namespace:`, else its name with `-` → `_`); declare another `namespace:` in one",
                ext.path,
                names.join(" and ")
            ));
            ext.commands.clear();
        }
    }
}

/// The commands a script composed, its optional `result`, and the events
/// it emits: `{ commands: [{ name, input }], result?, events?: [{ type,
/// payload, subject? }] }`.
pub fn composed(value: Value) -> Result<Composed, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Out {
        #[serde(default)]
        commands: Option<Vec<Call>>,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        refuse: Option<String>,
        #[serde(default)]
        events: Vec<ComposedEvent>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Call {
        name: String,
        #[serde(default = "empty_object")]
        input: Value,
    }
    fn empty_object() -> Value {
        json!({})
    }
    const SHAPE: &str =
        "the script must return `{ commands: [{ name, input }], result?, events? }` \
                         or `{ refuse: \"why\" }`";
    let out: Out = serde_json::from_value(value).map_err(|e| format!("{SHAPE}: {e}"))?;
    match (out.refuse, out.commands) {
        (Some(why), None) if out.result.is_none() && out.events.is_empty() => {
            Ok(Composed::Refused(why))
        }
        (Some(_), _) => Err(format!("{SHAPE}: a refusal composes nothing")),
        (None, None) => Err(format!("{SHAPE}: it returned neither")),
        (None, Some(commands)) => Ok(Composed::Run {
            calls: commands
                .into_iter()
                .map(|c| CommandCall {
                    name: c.name,
                    input: c.input,
                })
                .collect(),
            result: out.result,
            events: out.events,
        }),
    }
}

/// An event a script emits: one of its own extension's declared types, at
/// that type's newest version, appended caused by the run's
/// `command.executed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
    /// Refs it is about.
    #[serde(default)]
    pub subject: Vec<String>,
}

/// `events` as `extension` may emit them: each one of its own declared
/// types (by the vocabulary), at its newest version, from `source`.
pub fn own_events(
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    extension: &str,
    source: &str,
    events: Vec<ComposedEvent>,
) -> Result<Vec<oxplow_domain::Envelope>, oxplow_domain::CommandError> {
    let invalid = |message: String| oxplow_domain::CommandError::Invalid {
        field: None,
        message,
    };
    events
        .into_iter()
        .map(|e| {
            let v = vocabulary
                .latest(&e.event_type)
                .filter(|&v| vocabulary.owner(&e.event_type, v) == Some(Some(extension)))
                .ok_or_else(|| {
                    invalid(format!(
                        "the script may emit only the event types `{extension}` declares \
                         (`{}.*`); `{}` isn't one",
                        oxplow_domain::events::schema::extension_namespace(extension),
                        e.event_type
                    ))
                })?;
            Ok(
                oxplow_domain::Envelope::new(e.event_type, v, source, e.payload)
                    .map_err(|err| invalid(err.to_string()))?
                    .with_subject(e.subject),
            )
        })
        .collect()
}

/// An emitted event about an effort happened on that effort's thread: it
/// carries the thread, its stream and the effort as anchors, so an effect
/// reacting to it runs for that thread. One that already names a thread, or
/// is about no effort, is left as it is.
fn anchor_to_efforts(
    conn: &rusqlite::Connection,
    mut events: Vec<oxplow_domain::Envelope>,
) -> Result<Vec<oxplow_domain::Envelope>, oxplow_domain::DomainError> {
    use rusqlite::OptionalExtension as _;
    for event in &mut events {
        if event.anchors.thread_id.is_some() {
            continue;
        }
        let Some(effort) = event
            .subject
            .iter()
            .find_map(|r| oxplow_domain::uri::build::effort_of_uri(r))
        else {
            continue;
        };
        let thread: Option<i64> = conn
            .query_row(
                "SELECT thread_id FROM effort WHERE id = ?1",
                [effort.value()],
                |r| r.get(0),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)?;
        if let Some(thread) = thread {
            event.anchors = oxplow_db::event_log_store::anchors_for_thread_tx(
                conn,
                oxplow_domain::ThreadId::new(thread),
            )?;
            event.anchors.effort_id = Some(effort);
        }
    }
    Ok(events)
}

/// What a command's script decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Composed {
    /// Run these, in order, and emit `events`; the run's result is
    /// `result`.
    Run {
        calls: Vec<CommandCall>,
        result: Option<Value>,
        events: Vec<ComposedEvent>,
    },
    /// Decline, for this reason.
    Refused(String),
}

/// How long a command's script may run. The bus routes the run while it
/// composes, so this is far tighter than a collector's runaway catch:
/// composing a few commands is milliseconds. A script past it has its
/// worker stopped and the run refused.
pub const COMMAND_SCRIPT_BUDGET: oxplow_domain::run::Budget =
    oxplow_domain::run::Budget::with_timeout(std::time::Duration::from_secs(5));

/// Run a command script's (or an effect's) `transform` over `input` in a
/// script worker within [`COMMAND_SCRIPT_BUDGET`], with the services it was
/// `given` (`services`), its `scope(…)` calls answered by `serve` on this
/// thread (the command's transaction lives here). Blocks.
pub fn run_command_script(
    script: &str,
    input: Value,
    given: crate::services::Given,
    serve: &mut oxplow_script::workers::Serve<'_>,
    run: &oxplow_domain::run::RunContext,
) -> Result<Value, oxplow_script::CollectError> {
    use oxplow_script::workers::{shared, ScriptRun};
    shared()
        .run(
            &COMMAND_SCRIPT_BUDGET,
            given.onto(ScriptRun::starlark(script, input), Some(serve)),
            Some(run.cancel()),
        )
        .map(|ran| {
            run.used_cpu(ran.cpu_ms, ran.peak_bytes);
            ran.value
        })
}

/// The most examples one command may declare: `check_extension` runs
/// each one's script.
pub const MAX_EXAMPLES: usize = 10;

/// Run a command's script over `{ input }` in the sandbox
/// ([`COMMAND_SCRIPT_BUDGET`]), its `scope` calls answered by
/// `calls`, and read what it composes: the one compose step, for the
/// handler and the examples dry run alike. Blocks: the handler calls it
/// inside the bus's transaction (off the async runtime already), so the
/// script reads the semantic layer and nothing else (`services`).
pub fn compose_calls(
    script: &str,
    input: Value,
    calls: &mut crate::scope_calls::Calls<'_>,
) -> Result<Composed, oxplow_domain::CommandError> {
    let run = calls.run().clone();
    let given = crate::services::decide(&run, script, calls.needs())
        .and_then(crate::services::Wanted::scopes_only)
        .map_err(|message| oxplow_domain::CommandError::Failed {
            message: format!("the script failed: {message}"),
        })?;
    let mut serve = |id: &str, args: Value| calls.serve(id, args);
    let out = run_command_script(script, json!({ "input": input }), given, &mut serve, &run);
    // The database was busy answering a read: the run is retried.
    if let Some(message) = calls.take_busy() {
        return Err(oxplow_domain::CommandError::Busy { message });
    }
    let out = out.map_err(|e| oxplow_domain::CommandError::Failed {
        message: format!("the script failed: {e}"),
    })?;
    composed(out).map_err(|message| oxplow_domain::CommandError::Invalid {
        field: None,
        message,
    })
}

/// `decl` of extension `extension` as a command on `bus`: a composite
/// (`commands/compose.rs`) whose composer reads its `input` rows on the
/// run's connection, runs the script and returns what it composes. The bus
/// runs that in one transaction when every call can (each child's
/// a job's tools, policy and confirmation; one audit row; the reversed children
/// undo it), or as steps when one leaves it — a `work_item.*` call on
/// another provider's item (P7 review, tsk713). Pure, so the bus may
/// compose more than once.
pub fn extension_command(
    bus: &std::sync::Arc<crate::commands::CommandBus>,
    extension: &str,
    decl: &ExtensionCommand,
) -> Result<crate::commands::Command, oxplow_domain::CommandError> {
    use crate::commands::compose::{Compose, Composer, Composition};
    use crate::commands::Command;
    use oxplow_domain::{Atomicity, CommandError, Lifecycle};
    // oxplow's own say nothing of where they come from; another
    // extension's summary names it.
    let summary = if oxplow_domain::namespace_of(&decl.name) == oxplow_domain::OXPLOW_NAMESPACE {
        decl.summary.clone()
    } else {
        format!("{} (extension `{extension}`)", decl.summary)
    };
    // A scope's operation supplies the schema and keeps this example.
    let mut input_schema = decl.input_schema.clone().unwrap_or_else(|| json!({}));
    if let (Some(example), Some(schema)) = (&decl.example, input_schema.as_object_mut()) {
        schema.insert("examples".into(), json!([example]));
    }
    let spec = CommandSpec {
        output_schema: None,
        id: decl.name.clone(),
        summary,
        input_schema,
        job: decl.job,
        confirm: decl.confirm,
        undoable: true,
        lifecycle: if decl.stable {
            Lifecycle::Stable
        } else {
            Lifecycle::Experimental
        },
        atomicity: Atomicity::Dispatch,
        access: decl.access,
        needs: decl.needs.clone(),
        ui: decl.ui.clone(),
        op: None,
        unrecorded: decl.unrecorded.clone(),
    };
    let script = match &decl.handler {
        CommandHandler::Scope { scope, op } => {
            let backing = bus.op(scope, op).ok_or_else(|| CommandError::Invalid {
                field: None,
                message: format!("the scope `{scope}` has no op `{op}`"),
            })?;
            return backing.command(spec);
        }
        CommandHandler::Provider {
            provider,
            op,
            undoable,
        } => return provider_command(bus, extension, spec, provider, op, *undoable),
        CommandHandler::Script { script, .. } => script.clone(),
    };
    let (needs, reads) = (decl.needs.clone(), decl.access.reads_only());
    let (vocabulary, extension) = (bus.vocabulary().clone(), extension.to_string());
    let source = oxplow_domain::uri::build::command_uri(&decl.name);
    let compose: std::sync::Arc<Composer> = std::sync::Arc::new(
        move |composing: &crate::commands::compose::Composing<'_>,
              run: &oxplow_domain::run::RunContext,
              input: &Value| {
            // Read as an effect's script does: through the SQL gateway (its
            // metric functions included), on the run's activation pin.
            let (sql, runtime) = (composing.sql, composing.runtime);
            let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(sql.run_in(run, q)));
            let mut calls = crate::scope_calls::Calls::new(run, &needs, read)
                .with_state_source(sql.state_source(), runtime)
                .with_configuration_source(sql.configuration_source(), runtime)
                .with_jobs_source(sql.jobs_source(), runtime)
                .with_ai_source(sql.ai_source());
            match compose_calls(&script, input.clone(), &mut calls)? {
                Composed::Run { calls, events, .. }
                    if reads && !(calls.is_empty() && events.is_empty()) =>
                {
                    Err(CommandError::Invalid {
                        field: None,
                        message: "a command that only reads (`access: read`) composes no \
                                  commands and logs no events — it returns a `result`"
                            .into(),
                    })
                }
                Composed::Run {
                    calls,
                    result,
                    events,
                } => {
                    let events = own_events(&vocabulary.current(), &extension, &source, events)?;
                    let events = runtime
                        .block_on(composing.db.read(move |tx| anchor_to_efforts(tx, events)))
                        .map_err(oxplow_domain::CommandError::from)?;
                    Ok(Composition {
                        calls,
                        result,
                        events,
                    })
                }
                Composed::Refused(message) => Err(CommandError::Invalid {
                    field: None,
                    message,
                }),
            }
        },
    );
    let handler = Compose::handler(compose);
    Command::new(spec, handler)
}

/// The commands oxplow's required extensions declare over the operations
/// `bus` has, skipping the rest — a bus with one area's operations (a
/// test's, `oxplow-dev`'s), which never activates. Services register them
/// at activation, from foundation's active deployment.
pub fn register_declared(bus: &std::sync::Arc<crate::commands::CommandBus>) {
    for b in crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| b.required)
    {
        let manifest = b.file("extension.yaml").expect("a manifest");
        let doc: serde_yaml::Value = serde_yaml::from_str(manifest).expect("it parses");
        let (decls, _) = parse_commands(
            doc["namespace"].as_str().expect("a namespace"),
            true,
            &doc["commands"],
            "extension.yaml",
            manifest,
            &|path| b.file(path).map(str::to_string),
            &no_providers,
        );
        let built: Vec<_> = decls
            .iter()
            .filter(|d| match &d.handler {
                CommandHandler::Scope { scope, op } => bus.op(scope, op).is_some(),
                CommandHandler::Script { .. } | CommandHandler::Provider { .. } => false,
            })
            .map(|d| extension_command(bus, b.name, d).expect("it builds"))
            .collect();
        bus.register_namespace(
            oxplow_domain::OXPLOW_NAMESPACE,
            &oxplow_domain::uri::build::extension_uri(b.name),
            built,
        )
        .expect("they register");
    }
}

/// The command `spec` over operation `op` of `extension`'s provider
/// `provider`: it runs, outside the transaction, on the instance its input
/// names, through the bus's provider router (the provider registry). Its
/// spec names the operation (`provider:<extension>/<provider>`), which is
/// how an inverse naming an operation finds its command.
fn provider_command(
    bus: &std::sync::Arc<crate::commands::CommandBus>,
    extension: &str,
    mut spec: CommandSpec,
    provider: &str,
    op: &str,
    undoable: bool,
) -> Result<crate::commands::Command, oxplow_domain::CommandError> {
    use crate::commands::{Command, Handler, Invocation, ProviderCall};
    spec.atomicity = oxplow_domain::Atomicity::External;
    spec.undoable = undoable;
    spec.op = Some(oxplow_domain::OpRef {
        scope: oxplow_domain::contribution::ContributionId::new(
            extension,
            oxplow_domain::contribution::ContributionKind::Provider,
            provider,
        )
        .to_string(),
        op: op.into(),
    });
    let bus_weak = std::sync::Arc::downgrade(bus);
    let (extension, provider, op) = (extension.to_string(), provider.to_string(), op.to_string());
    Command::new(
        spec,
        Handler::External(std::sync::Arc::new(
            move |invocation: Invocation, input: Value| {
                let router = bus_weak.upgrade().and_then(|b| b.provider_router());
                let call = ProviderCall {
                    extension: extension.clone(),
                    provider: provider.clone(),
                    op: op.clone(),
                    input,
                    invocation,
                };
                match router {
                    Some(router) => router.run(call),
                    None => Box::pin(async {
                        Err(oxplow_domain::CommandError::Unavailable {
                            message: "providers aren't running".into(),
                            retry_after_ms: None,
                        })
                    }),
                }
            },
        )),
    )
}

#[derive(Clone, PartialEq)]
struct RegisteredCommands {
    namespace: String,
    /// What its extension may reach: its scripts' runs are within it.
    grant: oxplow_domain::grant::Grant,
    commands: Vec<ExtensionCommand>,
    version: Option<String>,
    /// The hash of the deployment its handlers run from.
    deployment: Option<String>,
    generation: String,
}

/// A fully validated command candidate, with metadata from one catalog
/// snapshot and an optimistic registry publication fence. Preparing it neither
/// changes handlers nor starts external work.
#[derive(Clone)]
pub struct StagedExtensionCommands {
    generation: String,
    source_generation: String,
    registrations: BTreeMap<String, RegisteredCommands>,
    commands: crate::commands::PreparedNamespaces,
}

/// Worker-local reservation of both handler storage and its owner metadata.
pub(crate) struct ExtensionCommandReservation<'a> {
    commands: crate::commands::NamespaceReservation<'a>,
    registered: tokio::sync::MutexGuard<'a, BTreeMap<String, RegisteredCommands>>,
    registrations: BTreeMap<String, RegisteredCommands>,
}

impl ExtensionCommandReservation<'_> {
    pub(crate) fn apply(&mut self) {
        self.commands.apply();
        *self.registered = std::mem::take(&mut self.registrations);
    }
}

impl StagedExtensionCommands {
    pub fn generation(&self) -> &str {
        &self.generation
    }
}

/// Keeps the primary worktree's extension commands in one staged batch.
/// Disabled/removed sources are withdrawn immediately; rejected replacements
/// retain the remaining old handlers and report candidate diagnostics. This
/// coordinates commands only, not the database or other extension registries.
pub struct ExtensionCommands {
    bus: std::sync::Arc<crate::commands::CommandBus>,
    catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
    root: std::path::PathBuf,
    /// What's registered, by namespace: the extension and its commands.
    registered: tokio::sync::Mutex<BTreeMap<String, RegisteredCommands>>,
}

impl ExtensionCommands {
    pub fn new(
        bus: &std::sync::Arc<crate::commands::CommandBus>,
        catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
        root: std::path::PathBuf,
    ) -> Self {
        Self {
            bus: bus.clone(),
            catalog,
            root,
            registered: tokio::sync::Mutex::default(),
        }
    }

    /// Prepare commands from one snapshot without changing the active bus.
    /// Enabled declarations with load errors reject the complete candidate;
    /// no partially parsed command list may silently replace a prior source.
    pub async fn stage(
        &self,
        snapshot: &crate::extension_catalog::CatalogSnapshot,
    ) -> Result<StagedExtensionCommands, BTreeMap<String, String>> {
        let registered = self.registered.lock().await;
        self.stage_from(snapshot, &registered)
    }

    fn wanted(
        snapshot: &crate::extension_catalog::CatalogSnapshot,
    ) -> BTreeMap<String, RegisteredCommands> {
        snapshot
            .extensions()
            .iter()
            .filter(|e| e.enabled && !e.commands.is_empty())
            .map(|e| {
                (
                    e.name.clone(),
                    RegisteredCommands {
                        namespace: e.namespace.clone(),
                        grant: crate::grants::grant_of(e),
                        commands: e.commands.clone(),
                        version: e.version.clone(),
                        deployment: e.deployment.hash().map(str::to_owned),
                        generation: snapshot.generation().to_string(),
                    },
                )
            })
            .collect()
    }

    fn stage_from(
        &self,
        snapshot: &crate::extension_catalog::CatalogSnapshot,
        registered: &BTreeMap<String, RegisteredCommands>,
    ) -> Result<StagedExtensionCommands, BTreeMap<String, String>> {
        let wanted = Self::wanted(snapshot);
        let mut problems: BTreeMap<String, String> = snapshot
            .extensions()
            .iter()
            .filter(|e| e.enabled && !e.errors.is_empty())
            .map(|e| (e.name.clone(), e.errors.join("\n")))
            .collect();
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut replacements = Vec::new();
        for (extension, registration) in &wanted {
            match self.build(extension, registration) {
                Ok(commands) => replacements.push(crate::commands::NamespaceCommands {
                    namespace: registration.namespace.clone(),
                    source: oxplow_domain::uri::build::extension_uri(extension),
                    commands,
                }),
                Err(problem) => {
                    problems.insert(extension.clone(), problem);
                }
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        for (extension, previous) in registered {
            if !wanted.contains_key(extension) {
                replacements.push(crate::commands::NamespaceCommands {
                    namespace: previous.namespace.clone(),
                    source: oxplow_domain::uri::build::extension_uri(extension),
                    commands: Vec::new(),
                });
            }
        }
        let commands = self
            .bus
            .prepare_namespaces(replacements)
            .map_err(|refusal| {
                let problem = registration_problem(refusal.error);
                // The source refused is held, and the one it collided with
                // when that is an extension of this batch; nothing else.
                let ours = |source: &str| {
                    oxplow_domain::uri::build::id_of(
                        source,
                        oxplow_domain::uri::resources::EXTENSION,
                    )
                    .ok()
                    .filter(|name| wanted.contains_key(name) || registered.contains_key(name))
                };
                std::iter::once(refusal.source.as_str())
                    .chain(refusal.holder.as_deref())
                    .filter_map(ours)
                    .map(|name| (name, problem.clone()))
                    .collect::<BTreeMap<_, _>>()
            })?;
        Ok(StagedExtensionCommands {
            generation: snapshot.generation().to_string(),
            source_generation: snapshot.source_generation().to_string(),
            registrations: wanted,
            commands,
        })
    }

    /// Publish a candidate only if both its catalog and command registry are
    /// unchanged. Callers must coordinate the other registries separately.
    pub async fn publish(
        &self,
        staged: StagedExtensionCommands,
    ) -> Result<(), oxplow_domain::CommandError> {
        let mut registered = self.registered.lock().await;
        self.publish_into(staged, &mut registered)?;
        Ok(())
    }

    /// Only call on the blocking publication worker. Lock ordering is owner
    /// metadata, then the command bus, matching reconciliation. The activation
    /// owner must exclude readers and other generation writers before this.
    pub(crate) fn reserve(
        &self,
        staged: &StagedExtensionCommands,
    ) -> Result<ExtensionCommandReservation<'_>, oxplow_domain::CommandError> {
        let registered = self.registered.blocking_lock();
        if self.catalog.snapshot(&self.root).generation() != staged.source_generation {
            return Err(oxplow_domain::CommandError::Unavailable {
                message: "the extension catalog changed while commands were staged".into(),
                retry_after_ms: None,
            });
        }
        let commands = self.bus.reserve_namespaces(&staged.commands)?;
        Ok(ExtensionCommandReservation {
            commands,
            registered,
            registrations: staged.registrations.clone(),
        })
    }

    fn publish_into(
        &self,
        staged: StagedExtensionCommands,
        registered: &mut BTreeMap<String, RegisteredCommands>,
    ) -> Result<(), oxplow_domain::CommandError> {
        if self.catalog.snapshot(&self.root).generation() != staged.source_generation {
            return Err(oxplow_domain::CommandError::Unavailable {
                message: "the extension catalog changed while commands were staged".into(),
                retry_after_ms: None,
            });
        }
        self.bus.publish_namespaces(staged.commands)?;
        *registered = staged.registrations;
        Ok(())
    }

    /// Build handlers with their original contribution metadata, without publication.
    fn build(
        &self,
        extension: &str,
        registration: &RegisteredCommands,
    ) -> Result<Vec<crate::commands::Command>, String> {
        registration
            .commands
            .iter()
            .map(|decl| {
                // Its local id, as its descriptor has it: the bus id
                // without the namespace (`<area>.<verb>`).
                // A command contribution is named by its bus id.
                let name = decl.name.clone();
                // Keep provenance with the handler, including when the
                // catalog reloads before this command is replaced.
                let contribution = oxplow_domain::run::ContributionRef {
                    id: oxplow_domain::contribution::ContributionId::new(
                        extension,
                        oxplow_domain::contribution::ContributionKind::Command,
                        &name,
                    ),
                    version: registration.version.clone(),
                    generation: Some(registration.generation.clone()),
                    deployment: registration.deployment.clone(),
                };
                // An operation's backing scope is implicit in its command
                // spec, and must also be present in the captured authority.
                let mut needs = decl.needs.clone();
                if let CommandHandler::Scope { scope, .. } = &decl.handler {
                    if !needs.contains(scope) {
                        needs.push(scope.clone());
                    }
                }
                let declared = crate::grants::script_request(&needs);
                let grant = registration.grant.clone();
                let (catalog, root, owner, deployment) = (
                    self.catalog.clone(),
                    self.root.clone(),
                    extension.to_string(),
                    registration.deployment.clone(),
                );
                let live: std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync> =
                    std::sync::Arc::new(move || {
                        let snapshot = catalog.live_snapshot(&root);
                        let current = crate::extensions::owner(snapshot.extensions(), &owner);
                        if current.is_some_and(|ext| {
                            ext.enabled && ext.deployment.hash() == deployment.as_deref()
                        }) {
                            Ok(())
                        } else {
                            Err("the command's owning extension was redeployed, disabled or removed"
                                .into())
                        }
                    });
                let named: std::sync::Arc<crate::commands::ContributionOf> =
                    std::sync::Arc::new(move |parent: &oxplow_domain::run::RunContext| {
                        // A script command may do what it declares, within its
                        // extension's permissions.
                        parent
                            .child(
                                Some(contribution.clone()),
                                oxplow_domain::run::Mode::External,
                            )
                            .authorized(oxplow_domain::run::Authority {
                                declared: declared.clone(),
                                grant: grant.clone(),
                                enforcement: crate::grants::enforcement(),
                            })
                            .guarded(live.clone())
                    });
                extension_command(&self.bus, extension, decl)
                    .map(|c| c.of_contribution(named))
                    .map_err(|e| format!("`{}`: {e}", decl.name))
            })
            .collect()
    }
}

fn registration_problem(error: oxplow_domain::CommandError) -> String {
    match error {
        oxplow_domain::CommandError::Invalid { message, .. } => {
            format!("{message}; declare another `namespace:`")
        }
        other => other.to_string(),
    }
}

/// Check `ext`'s commands (`check_extension`): with the running oxplow's
/// registry, its namespace is free (or already its own); then every
/// example is dry-run — the script in the sandbox, its scope calls
/// answered by the example's `answers` or for real, and what it composes against
/// the registry (each command exists, its input fits, and the names are
/// `expect_commands`, in order).
pub async fn check_extension_commands(
    layer: &crate::sql_gateway::SqlGateway,
    ext: &mut Extension,
    commands: Option<CommandSchemas<'_>>,
) {
    if let (Some(registry), false) = (commands, ext.commands.is_empty()) {
        let ns = ext.namespace.clone();
        let mine = oxplow_domain::uri::build::extension_uri(&ext.name);
        // `oxplow` is shared by oxplow's own extensions (its namespace check
        // already refused it to anyone else); any other is one owner's.
        if let Some(owner) = registry
            .namespace_owner(&ns)
            .filter(|o| *o != mine && ns != oxplow_domain::OXPLOW_NAMESPACE)
        {
            ext.errors.push(format!(
                "{}/extension.yaml: the command namespace `{ns}` is {owner}'s; declare another \
                 `namespace:`",
                ext.path
            ));
        }
    }
    let with_examples: Vec<ExtensionCommand> = ext
        .commands
        .iter()
        .filter(|c| !c.examples.is_empty())
        .cloned()
        .collect();
    if with_examples.is_empty() {
        return;
    }
    let Some(schema_of) = commands else {
        ext.warnings.push(format!(
            "{}/extension.yaml: command examples weren't checked (no running oxplow to ask which \
             commands exist) — check the extension from inside oxplow (Settings → Extensions)",
            ext.path
        ));
        return;
    };
    for cmd in with_examples {
        for ex in &cmd.examples {
            let prefix = format!(
                "{}/extension.yaml: command `{}` example `{}`",
                ext.path, cmd.name, ex.name
            );
            if let Err(e) = check_example(layer, &cmd, ex, schema_of).await {
                ext.errors.push(format!("{prefix}: {e}"));
            }
        }
    }
}

async fn check_example(
    layer: &crate::sql_gateway::SqlGateway,
    cmd: &ExtensionCommand,
    ex: &CommandExample,
    schema_of: CommandSchemas<'_>,
) -> Result<(), String> {
    let decided = dry_run(layer, cmd, &ex.input, &ex.answers, schema_of).await?;
    match (decided, &ex.refused) {
        (Composed::Refused(why), Some(want)) if why.contains(want.as_str()) => Ok(()),
        (Composed::Refused(why), Some(want)) => {
            Err(format!("refused ({why}) but it should refuse ({want})"))
        }
        (Composed::Refused(why), None) => Err(format!(
            "refused ({why}) but `expect_commands` is [{}]",
            ex.expect_commands.join(", ")
        )),
        (Composed::Run { calls, .. }, Some(want)) => Err(format!(
            "composed [{}] but it should refuse ({want})",
            call_names(&calls).join(", ")
        )),
        (Composed::Run { calls, .. }, None) if call_names(&calls) != ex.expect_commands => {
            Err(format!(
                "composed [{}] but `expect_commands` is [{}]",
                call_names(&calls).join(", "),
                ex.expect_commands.join(", ")
            ))
        }
        (Composed::Run { .. }, None) => Ok(()),
    }
}

/// The names of `calls`, in order.
pub fn call_names(calls: &[CommandCall]) -> Vec<&str> {
    calls.iter().map(|c| c.name.as_str()).collect()
}

/// Dry-run `cmd` on `input`: the script in the sandbox, its scope
/// calls answered from `answers` or for real (reads only: `sql.read`
/// through `layer`), and — when it composes — each command against
/// `registry` (it exists, its input fits). What the script decided;
/// nothing runs. `check`'s examples and `oxplow extension test`'s intent
/// examples both run it.
pub async fn dry_run(
    layer: &crate::sql_gateway::SqlGateway,
    cmd: &ExtensionCommand,
    input: &Value,
    answers: &BTreeMap<String, Vec<Value>>,
    registry: CommandSchemas<'_>,
) -> Result<Composed, String> {
    let CommandHandler::Script { script, .. } = &cmd.handler else {
        return Err(format!(
            "`{}` runs a scope's operation; only a script's command is dry-run",
            cmd.name
        ));
    };
    let (script, input, needs) = (script.clone(), input.clone(), cmd.needs.clone());
    let (layer, answers) = (layer.clone(), answers.clone());
    let runtime = tokio::runtime::Handle::current();
    let decided = tokio::task::spawn_blocking(move || {
        let run = crate::run_context::for_dry_run();
        let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(layer.run(q)));
        let mut calls = crate::scope_calls::Calls::new(&run, &needs, read).with_answers(&answers);
        compose_calls(&script, input, &mut calls)
    })
    .await
    .map_err(|e| format!("the script's worker failed: {e}"))?
    .map_err(|e| match e {
        oxplow_domain::CommandError::Failed { message }
        | oxplow_domain::CommandError::Invalid { message, .. } => message,
        other => other.to_string(),
    })?;
    if let Composed::Run { calls, .. } = &decided {
        check_calls(registry, calls)?;
    }
    Ok(decided)
}

/// Each composed call names a registered command and fits its input
/// schema — what a dry run (a command's examples, an effect's fixtures)
/// checks without running anything.
pub fn check_calls(registry: CommandSchemas<'_>, calls: &[CommandCall]) -> Result<(), String> {
    for call in calls {
        let Some(schema) = registry.input_schema(&call.name) else {
            return Err(format!("no command `{}`", call.name));
        };
        InputValidator::compile(&schema)
            .map_err(|e| e.to_string())
            .and_then(|v| v.check(&call.input).map_err(|e| e.to_string()))
            .map_err(|e| format!("the input doesn't fit `{}`: {e}", call.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Why `extension` is held back by the last activation, if it is.
    fn held_problem(fx: &crate::test_fixtures::EffortFixture, extension: &str) -> Option<String> {
        fx.svc
            .extension_activation
            .problems()
            .get(extension)
            .map(|lines| lines.join("\n"))
    }

    use super::*;
    use crate::extensions::{load_extensions, validate_extension, Extension};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Moves a task to done.
    const HANDLER: &str = "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"uri\": x[\"input\"][\"uri\"], \"to\": \"done\"}}]}\n";

    const GOOD: &str = "  - name: review.finish
    summary: Mark the task done.
    input_schema: { type: object, required: [uri], properties: { uri: { type: string } }, additionalProperties: false }
    entry: handlers/finish_review.star
    examples:
      - { name: happy, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, expect_commands: [oxplow.work_item.transition] }
";

    /// Write extension `name` with `commands` (the block's entries) and
    /// `files` (relative to its folder).
    fn write_ext(root: &Path, name: &str, commands: &str, files: &[(&str, &str)]) {
        write(
            root,
            &format!(".oxplow/extensions/{name}/extension.yaml"),
            &format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\npermissions: [sql.read, commands:oxplow.work_item.*]\ncommands:\n{commands}"),
        );
        for (rel, body) in files {
            write(root, &format!(".oxplow/extensions/{name}/{rel}"), body);
        }
    }

    fn project(root: &Path, name: &str) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.origin == "project" && e.name == name)
            .unwrap()
    }

    /// A script handler's `needs` are the scopes it calls: one a script
    /// can't call is refused at load — the rule effects meet too. A
    /// capability need still gates it.
    #[test]
    fn a_script_needs_only_scopes_it_can_call() {
        let d = tempfile::tempdir().unwrap();
        let commands = GOOD.replace(
            "    entry: handlers/finish_review.star",
            "    entry: handlers/finish_review.star\n    needs: [sql.read, threads.write, work_items]",
        );
        write_ext(
            d.path(),
            "my-review",
            &commands,
            &[("handlers/finish_review.star", HANDLER)],
        );
        let errors = project(d.path(), "my-review").errors.join("\n");
        assert!(
            errors.contains("`threads.write` is a scope a script can't use"),
            "{errors}"
        );
        assert!(!errors.contains("`work_items`"), "{errors}");
    }

    /// A command's `ui` is checked when its extension loads — a binding
    /// that isn't one is an error there, not a menu entry that never
    /// shows.
    #[test]
    fn a_commands_ui_is_checked_at_load() {
        let d = tempfile::tempdir().unwrap();
        let commands =
            format!("{GOOD}    ui: {{ label: Finish, input: {{ uri: \"{{{{tread}}}}\" }} }}\n");
        write_ext(
            d.path(),
            "my-review",
            &commands,
            &[("handlers/finish_review.star", HANDLER)],
        );
        let errors = project(d.path(), "my-review").errors.join("\n");
        assert!(
            errors.contains("`/ui/input`") && errors.contains("`{{tread}}` isn't a binding"),
            "{errors}"
        );
    }

    /// A command keeps the inputs it names out of its record (a token, a
    /// key): each must be one of its input's properties, or `*`.
    #[test]
    fn a_command_keeps_named_inputs_out_of_its_record() {
        let d = tempfile::tempdir().unwrap();
        let schema = "    input_schema: { type: object, required: [uri], properties: { uri: { type: string } }, additionalProperties: false }";
        let with_token = "    input_schema: { type: object, required: [uri], properties: { uri: { type: string }, token: { type: string } }, additionalProperties: false }";
        write_ext(
            d.path(),
            "keeps",
            &format!(
                "{}    unrecorded: [token]\n",
                GOOD.replace(schema, with_token)
            ),
            &[("handlers/finish_review.star", HANDLER)],
        );
        let ext = project(d.path(), "keeps");
        assert_eq!(ext.errors, Vec::<String>::new());
        assert_eq!(ext.commands[0].unrecorded, vec!["token".to_string()]);
        write_ext(
            d.path(),
            "gone",
            &format!("{GOOD}    unrecorded: [token]\n"),
            &[("handlers/finish_review.star", HANDLER)],
        );
        let errors = project(d.path(), "gone").errors.join("\n");
        assert!(
            errors.contains("`unrecorded` names `token`, which isn't a property of its input"),
            "{errors}"
        );
    }

    #[test]
    fn commands_load_with_their_script() {
        let d = tempfile::tempdir().unwrap();
        write_ext(
            d.path(),
            "my-review",
            GOOD,
            &[("handlers/finish_review.star", HANDLER)],
        );
        let ext = project(d.path(), "my-review");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.commands.len(), 1);
        let c = &ext.commands[0];
        assert_eq!(
            c.name, "my_review.review.finish",
            "the namespace is the name, `-` → `_`"
        );
        assert_eq!(c.summary, "Mark the task done.");
        assert!(
            matches!(&c.handler, CommandHandler::Script { entry, script } if entry == "handlers/finish_review.star" && script == HANDLER),
            "{:?}",
            c.handler
        );
        assert_eq!(
            c.input_schema.as_ref().unwrap()["required"],
            serde_json::json!(["uri"])
        );
        // Its first example that runs (not one expecting a refusal) is
        // the example input an agent is shown and a refusal quotes.
        assert_eq!(
            c.example,
            Some(serde_json::json!({ "uri": "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }))
        );
        assert!(c.needs.is_empty());
        assert_eq!(c.confirm, Confirm::Never);
        assert_eq!(c.access, Access::Write);
        assert!(!c.job);
        assert_eq!(c.examples.len(), 1);
        assert_eq!(
            c.examples[0].expect_commands,
            vec!["oxplow.work_item.transition"]
        );
        assert_eq!(
            oxplow_domain::contribution::extension_namespace("my-review"),
            "my_review"
        );
    }

    /// A command or a page that asks to be a tool (`tool: <name>`) is
    /// offered to agents as `<namespace>_<name>`; a name that isn't one is
    /// an error at its entry, and two entries can't share one.
    #[test]
    fn a_command_or_a_page_is_offered_as_a_tool_under_its_namespace() {
        let d = tempfile::tempdir().unwrap();
        let page = "title: Waiting\ndata:\n  n: { sql: SELECT 1 AS n }\nview: { viz: number, from: n }\ntool: waiting\n";
        write_ext(
            d.path(),
            "my-review",
            &format!("{GOOD}    tool: finish\n"),
            &[
                ("handlers/finish_review.star", HANDLER),
                ("pages/waiting.yaml", page),
            ],
        );
        let ext = project(d.path(), "my-review");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.commands[0].tool.as_deref(), Some("my_review_finish"));
        assert_eq!(ext.page_files[0].tool.as_deref(), Some("my_review_waiting"));

        for (command_tool, page_tool, says) in [
            ("Finish", "waiting", "tool name `Finish`"),
            ("finish", "finish-it", "tool name `finish-it`"),
            ("finish", "finish", "`my_review_finish` is offered twice"),
        ] {
            let d = tempfile::tempdir().unwrap();
            write_ext(
                d.path(),
                "my-review",
                &format!("{GOOD}    tool: {command_tool}\n"),
                &[
                    ("handlers/finish_review.star", HANDLER),
                    (
                        "pages/waiting.yaml",
                        &format!(
                            "title: Waiting\ndata:\n  n: {{ sql: SELECT 1 AS n }}\n\
                             view: {{ viz: number, from: n }}\ntool: {page_tool}\n"
                        ),
                    ),
                ],
            );
            let errors = project(d.path(), "my-review").errors.join("\n");
            assert!(
                errors.contains(says),
                "{command_tool}/{page_tool}: {errors}"
            );
        }
    }

    /// Two extensions under one namespace (oxplow's own) can't offer one
    /// tool name: each is told, and neither offers it.
    #[test]
    fn a_tool_name_two_extensions_declare_is_refused() {
        let d = tempfile::tempdir().unwrap();
        for name in ["first", "second"] {
            write(
                d.path(),
                &format!(".oxplow/extensions/{name}/extension.yaml"),
                &format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\n"),
            );
            write(
                d.path(),
                &format!(".oxplow/extensions/{name}/pages/waiting.yaml"),
                "title: Waiting\ndata:\n  n: { sql: SELECT 1 AS n }\nview: { viz: number, from: n }\ntool: waiting\n",
            );
        }
        let mut exts: Vec<Extension> = load_extensions(d.path())
            .into_iter()
            .filter(|e| e.origin == "project")
            .collect();
        for e in &mut exts {
            e.namespace = "shared".into();
            e.page_files[0].tool = Some("shared_waiting".into());
        }
        refuse_duplicate_tools(&mut exts);
        for e in &exts {
            assert_eq!(e.page_files[0].tool, None, "{}", e.name);
            assert!(
                e.errors
                    .join("\n")
                    .contains("`shared_waiting` is offered by `first` and `second`"),
                "{:?}",
                e.errors
            );
        }
    }

    /// A command declares what it needs and how a person meets
    /// it, like core's; a private extension's is experimental.
    #[test]
    fn a_command_declares_its_needs_and_how_a_person_meets_it() {
        let d = tempfile::tempdir().unwrap();
        let tail = "    needs: [work_items.comments]\n    ui:\n      label: Finish Review\n      group: Review\n      keywords: [done]\n      about: oxplow/work_item\n      input: { uri: \"{{uri}}\" }\n";
        write_ext(
            d.path(),
            "my-review",
            &format!("{GOOD}{tail}"),
            &[("handlers/finish_review.star", HANDLER)],
        );
        let ext = project(d.path(), "my-review");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let c = &ext.commands[0];
        assert_eq!(c.needs, vec!["work_items.comments".to_string()]);
        let ui = c.ui.as_ref().unwrap();
        assert_eq!(ui.label, "Finish Review");
        assert_eq!(ui.group.as_deref(), Some("Review"));
        assert_eq!(ui.about.as_deref(), Some("oxplow/work_item"));
        assert_eq!(ui.input, Some(serde_json::json!({ "uri": "{{uri}}" })));
        assert!(!c.stable, "a private extension's command is experimental");
    }

    /// Each broken entry is an error at its line, and is dropped.
    #[test]
    fn a_broken_command_is_an_error_at_its_line() {
        let entry = |name: &str, tail: &str| {
            format!(
                "  - name: {name}\n    summary: S.\n    input_schema: {{ type: object }}\n    entry: handlers/h.star\n{tail}"
            )
        };
        for (block, files, says) in [
            (entry("Finish", ""), vec![("handlers/h.star", HANDLER)], "command name `Finish`"),
            (entry("a.b", ""), vec![], "entry `handlers/h.star` isn't a file"),
            (
                entry("a.b", ""),
                vec![("handlers/h.star", "def transform(x:\n")],
                "doesn't parse",
            ),
            (
                entry("a.b", ""),
                vec![("handlers/h.star", "def other(x):\n    return {}\n")],
                "define `transform`",
            ),
            (
                entry("a.b", "    access: sometimes\n"),
                vec![("handlers/h.star", HANDLER)],
                "`access` must be `write`, `record` or `read`",
            ),
            (
                entry("a.b", "    needs: [warp_drive]\n"),
                vec![("handlers/h.star", HANDLER)],
                "isn't a capability",
            ),
            (
                entry("a.b", "    ui: { group: Review }\n"),
                vec![("handlers/h.star", HANDLER)],
                "label",
            ),
            (
                entry("a.b", "    access: often\n"),
                vec![("handlers/h.star", HANDLER)],
                "access",
            ),
            (
                entry("a.b", "    confirm: maybe\n"),
                vec![("handlers/h.star", HANDLER)],
                "confirm",
            ),
            (
                entry("a.b", "    input: \"SELECT 1\"\n"),
                vec![("handlers/h.star", HANDLER)],
                "unknown field `input`",
            ),
            (
                "  - name: a.b\n    summary: S.\n".into(),
                vec![],
                "has no handler",
            ),
            (
                entry("a.b", "    scope: bookmarks.write\n    op: set\n"),
                vec![("handlers/h.star", HANDLER)],
                "more than one of `entry:`, `scope:` and `provider:`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    scope: bookmarks.erase\n    op: set\n".into(),
                vec![],
                "no scope `bookmarks.erase`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    scope: bookmarks.write\n".into(),
                vec![],
                "needs `op:`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    scope: bookmarks.write\n    op: set\n    input_schema: { type: object }\n".into(),
                vec![],
                "`input_schema` comes from the scope's operation",
            ),
            (
                entry("a.b", "    access: read\n    confirm: always\n"),
                vec![("handlers/h.star", HANDLER)],
                "only reads is never confirmed",
            ),
            (
                "  - name: a.b\n    summary: S.\n    input_schema: { type: nope }\n    entry: handlers/h.star\n".into(),
                vec![("handlers/h.star", HANDLER)],
                "input_schema",
            ),
            // Who may run it isn't a command's to say: anyone may.
            (
                entry("a.b", "    invokers: { human: true, agent: false, page: true }\n"),
                vec![("handlers/h.star", HANDLER)],
                "unknown field `invokers`",
            ),
            (
                format!("{}{}", entry("a.b", ""), entry("a.b", "")),
                vec![("handlers/h.star", HANDLER)],
                "declared twice",
            ),
            (
                entry("a.b", "    entry_point: x\n"),
                vec![("handlers/h.star", HANDLER)],
                "entry_point",
            ),
            (
                entry("a.b", "    examples:\n      - { name: e0 }\n      - { name: e1 }\n      - { name: e2 }\n      - { name: e3 }\n      - { name: e4 }\n      - { name: e5 }\n      - { name: e6 }\n      - { name: e7 }\n      - { name: e8 }\n      - { name: e9 }\n      - { name: e10 }\n"),
                vec![("handlers/h.star", HANDLER)],
                "at most 10 examples",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            write_ext(d.path(), "x", &block, &files);
            let ext = project(d.path(), "x");
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{block}: {errs}"
            );
            assert!(
                ext.commands.iter().all(|c| c.name != "x.Finish"),
                "{block}"
            );
            if says != "declared twice" {
                assert!(ext.commands.is_empty(), "{block}: {:?}", ext.commands);
            }
        }
    }

    /// Two enabled extensions that map to one namespace are both refused;
    /// a disabled one doesn't count. (Extension names are dashes and
    /// letters, so the second claims `a_b` with an explicit `namespace:`.)
    #[test]
    fn a_namespace_is_one_extensions() {
        let d = tempfile::tempdir().unwrap();
        let files = [("handlers/finish_review.star", HANDLER)];
        write_ext(d.path(), "a-b", GOOD, &files);
        write_ext(d.path(), "ab", GOOD, &files);
        let manifest = d.path().join(".oxplow/extensions/ab/extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            text.replace("name: ab\n", "name: ab\nnamespace: a_b\n"),
        )
        .unwrap();
        for name in ["a-b", "ab"] {
            let ext = project(d.path(), name);
            assert!(
                ext.errors
                    .join("\n")
                    .contains("both register commands under `a_b`"),
                "{name}: {:?}",
                ext.errors
            );
            assert!(ext.commands.is_empty(), "{name}");
        }
        write(
            d.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [ab]\n",
        );
        let ext = project(d.path(), "a-b");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.commands.len(), 1);
    }

    /// A changed script re-registers its command with the new script.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_changed_script_reregisters_its_command() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    return {\"commands\": [], \"result\": 1}\n",
        )
        .await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let run = || async {
            fx.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "my_review.review.finish",
                    json!({ "uri": r }),
                    false,
                )
                .await
                .unwrap()
                .result["result"]
                .clone()
        };
        assert_eq!(run().await, json!(1));
        // A longer script: the catalog's fingerprint sees the new size.
        with_finish(
            &fx,
            "def transform(x):\n    return {\"commands\": [], \"result\": 22222}\n",
        )
        .await;
        assert_eq!(run().await, json!(22222));
    }

    /// Checked against the running registry, an extension whose command
    /// namespace something else holds (a provider's `held.*`) is an error;
    /// its own registered namespace is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_namespace_the_registry_holds_is_a_check_error() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx._dir.path();
        let files = [("handlers/finish_review.star", HANDLER)];
        fx.svc
            .commands
            .register_namespace(
                "held",
                "provider:held",
                vec![crate::commands::Command::new(
                    oxplow_domain::CommandSpec {
                        output_schema: None,
                        id: "held.review.held".into(),
                        summary: "A provider's.".into(),
                        input_schema: json!({ "type": "object" }),
                        job: false,
                        confirm: Confirm::Never,
                        undoable: false,
                        lifecycle: oxplow_domain::Lifecycle::Experimental,
                        atomicity: oxplow_domain::Atomicity::Tx,
                        access: Access::Write,
                        needs: Vec::new(),
                        ui: None,
                        op: None,
                        unrecorded: Vec::new(),
                    },
                    crate::commands::Handler::Tx(std::sync::Arc::new(|_, _| {
                        Ok(crate::commands::HandlerOutput::default())
                    })),
                )
                .unwrap()],
            )
            .unwrap();
        write_ext(root, "held", GOOD, &files);
        let v = crate::extensions::validate_extension(
            &fx.svc.sql,
            &fx.svc.extension_catalog,
            root,
            "held",
            Some(fx.svc.commands.as_ref()),
        )
        .await
        .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("the command namespace `held` is provider:held's"),
            "{errs}"
        );

        // Reconcile is all-or-nothing: an enabled extension with an error
        // (`held`) rejects the whole candidate, so drop it before `mine`
        // can register.
        std::fs::remove_dir_all(root.join(".oxplow/extensions/held")).unwrap();
        write_ext(root, "mine", GOOD, &files);
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert_eq!(
            fx.svc.commands.namespace_owner("mine").as_deref(),
            Some("oxplow:oxplow/extension?name=mine")
        );
        let v = crate::extensions::validate_extension(
            &fx.svc.sql,
            &fx.svc.extension_catalog,
            root,
            "mine",
            Some(fx.svc.commands.as_ref()),
        )
        .await
        .unwrap();
        assert!(!v.errors.join("\n").contains("namespace"), "{:?}", v.errors);
    }

    /// `check_extension` runs each example's script and checks what it
    /// composes against the registry: every command exists, its input
    /// fits, and the names are the ones `expect_commands` lists.
    #[tokio::test]
    async fn validate_dry_runs_command_examples_against_the_registry() {
        let transition_schema = serde_json::json!({
            "type": "object",
            "required": ["uri", "to"],
            "properties": { "uri": { "type": "string" }, "to": { "type": "string" } },
            "additionalProperties": false
        });
        let schema = move |name: &str| {
            (name == "oxplow.work_item.transition").then(|| transition_schema.clone())
        };
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        let check = |handler: &'static str, tail: &'static str| {
            let d = tempfile::tempdir().unwrap();
            write_ext(
                d.path(),
                "x",
                &format!("{GOOD}{tail}"),
                &[("handlers/finish_review.star", handler)],
            );
            d
        };

        let d = check(HANDLER, "");
        let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);

        // An example without `answers` reads for real: `sql.read`'s rows
        // reach the script.
        let reads = |sql: &str| {
            let d = tempfile::tempdir().unwrap();
            write_ext(
                d.path(),
                "x",
                &GOOD.replace(
                    "    entry: handlers/finish_review.star\n",
                    "    entry: handlers/finish_review.star\n    needs: [sql.read]\n",
                ),
                &[(
                    "handlers/finish_review.star",
                    &format!(
                        "def transform(x):\n    rows = scope(\"sql.read\", {{\"sql\": \"{sql}\", \"params\": {{\"uri\": x[\"input\"][\"uri\"]}}}})\n    return {{\"commands\": [{{\"name\": \"oxplow.work_item.transition\", \"input\": {{\"uri\": rows[0][\"r\"], \"to\": \"done\"}}}}]}}\n"
                    ),
                )],
            );
            d
        };
        let d = reads("SELECT :uri AS r");
        let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        // A read of a table, not a published dataset, is refused.
        let d = reads("SELECT title AS r FROM task");
        let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("example `happy`")
                && errs.contains("`sql.read`")
                && errs.contains("task"),
            "{errs}"
        );

        for (handler, says) in [
            (
                "def transform(x):\n    return {\"commands\": [{\"name\": \"no.such\", \"input\": {}}]}\n",
                "example `happy`: no command `no.such`",
            ),
            (
                "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"uri\": 1}}]}\n",
                "example `happy`: the input doesn't fit `oxplow.work_item.transition`",
            ),
            (
                "def transform(x):\n    return {\"commands\": []}\n",
                "example `happy`: composed [] but `expect_commands` is [oxplow.work_item.transition]",
            ),
            (
                "def transform(x):\n    return [1]\n",
                "example `happy`: the script must return",
            ),
        ] {
            let d = check(handler, "");
            let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
                .await
                .unwrap();
            let errs = v.errors.join("\n");
            assert!(errs.contains(says), "{handler}: {errs}");
        }

        // Without a registry the examples aren't checked, and it says so.
        let d = check(HANDLER, "");
        let v = validate_extension(&layer, &cat, d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            v.warnings
                .join("\n")
                .contains("command examples weren't checked"),
            "{:?}",
            v.warnings
        );
    }

    // ---- B2: the handler and the reconciler ----

    /// `my-review` (namespace `my_review`) with `finish`, whose script is
    /// `script`, in the fixture's project; registered.
    async fn with_finish(fx: &crate::test_fixtures::EffortFixture, script: &str) {
        write_ext(
            fx._dir.path(),
            "my-review",
            "  - name: review.finish
    summary: Finish the task.
    input_schema: { type: object, required: [uri], properties: { uri: { type: string } }, additionalProperties: false }
    entry: handlers/finish.star
    needs: [sql.read]
",
            &[("handlers/finish.star", script)],
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
    }

    /// Transitions the task to done and renames it after its row.
    const FINISH: &str = "def transform(x):
    row = scope(\"sql.read\", {
        \"sql\": \"SELECT uri, title FROM v_work_item WHERE uri = :uri\",
        \"params\": {\"uri\": x[\"input\"][\"uri\"]},
    })[0]
    return {
        \"commands\": [
            {\"name\": \"oxplow.work_item.update\", \"input\": {\"uri\": row[\"uri\"], \"title\": row[\"title\"] + \" (reviewed)\"}},
            {\"name\": \"oxplow.work_item.transition\", \"input\": {\"uri\": row[\"uri\"], \"to\": \"done\"}},
        ],
        \"result\": {\"finished\": row[\"uri\"]},
    }
";

    fn agent(fx: &crate::test_fixtures::EffortFixture) -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn task(fx: &crate::test_fixtures::TaskEffortFixture) -> oxplow_tasks::Task {
        use oxplow_tasks::TaskStore as _;
        fx.svc.task_store.get(fx.task).await.unwrap().unwrap()
    }

    /// `my-review`, declaring `my_review.finished@1`, with a `finish`
    /// command whose script emits `{type}` for its `uri`; registered.
    async fn with_emitting_finish(fx: &crate::test_fixtures::EffortFixture, event_type: &str) {
        let root = fx._dir.path();
        write(
            root,
            ".oxplow/extensions/my-review/extension.yaml",
            "manifest: 2\nname: my-review\nintent:\n  purpose: p\nevent_types:\n  types:\n    - { type: my_review.finished, v: 1, schema: finished.json, summary: A review finished. }\ncommands:\n  - name: review.finish\n    summary: Finish the review.\n    input_schema: { type: object, required: [uri], properties: { uri: { type: string } } }\n    entry: finish.star\n",
        );
        write(
            root,
            ".oxplow/extensions/my-review/finished.json",
            r#"{"type": "object", "required": ["uri"], "properties": {"uri": {"type": "string"}}}"#,
        );
        write(
            root,
            ".oxplow/extensions/my-review/finish.star",
            &format!(
                "def transform(x):\n    return {{\"commands\": [], \"events\": [{{\"type\": \"{event_type}\", \"payload\": {{\"uri\": x[\"input\"][\"uri\"]}}, \"subject\": [x[\"input\"][\"uri\"]]}}]}}\n"
            ),
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
    }

    /// P8.D4: a script's `events` append its extension's own types,
    /// caused by the run's `command.executed`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_emits_its_own_extensions_event_type() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_emitting_finish(&fx, "my_review.finished").await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let out = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap();
        let executed = out.event_id.unwrap().to_string();
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT subject, cause FROM v_event WHERE type = 'my_review.finished'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 1, "{rows:?}");
        assert!(format!("{:?}", rows.rows[0][0]).contains(&r), "{rows:?}");
        assert_eq!(rows.rows[0][1], oxplow_db::SqlCell::Text(executed));
    }

    /// Only its own declared types: a core type, or one in its namespace
    /// that it doesn't declare, refuses the run, which writes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_may_not_emit_a_foreign_or_undeclared_type() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        for foreign in ["work_item.created", "my_review.undeclared"] {
            with_emitting_finish(&fx, foreign).await;
            let err = fx
                .svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "my_review.review.finish",
                    json!({ "uri": r }),
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string()
                    .contains("only the event types `my-review` declares"),
                "{foreign}: {err}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_extension_command_composes_core_commands_in_one_run() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(&fx, FINISH).await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let out = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["result"], json!({ "finished": r }));
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        let t = task(&fx).await;
        assert_eq!(t.status, oxplow_tasks::TaskStatus::Done);
        assert_eq!(t.title, "t (reviewed)");
        let audits = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
        let ok: Vec<_> = audits
            .iter()
            .filter(|a| a.outcome == oxplow_domain::events::schema::CommandOutcome::Ok)
            .collect();
        assert_eq!(ok.len(), 1, "one audit row for the run");
        assert_eq!(ok[0].command, "my_review.review.finish");
        assert_eq!(
            ok[0].scopes,
            [("sql.read".to_string(), 1)].into(),
            "the run's own read, not the routing pass's"
        );
        let events = fx.svc.event_log_store.read_after(0, 200).await.unwrap();
        let changed = events
            .iter()
            .rev()
            .find(|e| e.envelope.event_type == "work_item.state_changed")
            .expect("the child's event");
        assert_eq!(changed.envelope.cause, out.event_id, "caused by the run");
        // A work list's verbs run outside the transaction: the children are
        // steps, and the run isn't undoable as a whole.
        assert!(out.inverse.is_none());
    }

    /// An operation's floor holds for any extension's declaration over
    /// it: one that asks less than `vcs.write/discard`'s `destructive`, or
    /// says a job may call it (a job calls only its extension's own
    /// handlers), isn't registered, and the problem says why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declaration_over_an_operation_stays_within_its_floor() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let declare = |block: &str| {
            write_ext(fx._dir.path(), "acme", block, &[]);
        };
        declare(
            "  - name: files.drop\n    summary: Discard.\n    scope: vcs.write\n    op: discard\n",
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(fx.svc.commands.spec("acme.files.drop").is_none());
        let problem = held_problem(&fx, "acme").unwrap();
        assert!(
            problem.contains("`vcs.write/discard` is confirmed `destructive` at least")
                && problem.contains("`acme.files.drop` declares `never`"),
            "{problem}"
        );

        declare(
            "  - name: page.star\n    summary: Star the page.\n    scope: bookmarks.write\n    op: set\n    job: true\n",
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(fx.svc.commands.spec("acme.page.star").is_none());
        let problem = held_problem(&fx, "acme").unwrap();
        assert!(
            problem.contains("a job calls only its extension's own handlers"),
            "{problem}"
        );

        declare(
            "  - name: page.star\n    summary: Star the page.\n    scope: bookmarks.write\n    op: set\n",
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(fx.svc.commands.spec("acme.page.star").is_some());
        assert!(held_problem(&fx, "acme").is_none());
    }

    /// A project extension declares its own command over one of the
    /// operations oxplow's commands are backed by: nothing about
    /// oxplow's is special. Its spec is the operation's (schema, undo,
    /// access), the scope among its needs; one naming an operation
    /// that isn't there is rejected (the old command stays), its problem said.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_extension_declares_a_command_over_an_operation() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let star = |op: &str| {
            write_ext(
                fx._dir.path(),
                "acme",
                &format!(
                    "  - name: page.star\n    summary: Star the page for the project.\n    scope: bookmarks.write\n    op: {op}\n"
                ),
                &[],
            );
        };
        star("set");
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let spec = fx.svc.commands.spec("acme.page.star").expect("registered");
        let oxplows = fx.svc.commands.spec("oxplow.bookmark.set").unwrap();
        // The operation's schema; each declaration keeps its own example.
        let without_examples = |mut schema: serde_json::Value| {
            schema.as_object_mut().unwrap().remove("examples");
            schema
        };
        assert_eq!(
            without_examples(spec.input_schema.clone()),
            without_examples(oxplows.input_schema.clone())
        );
        assert_eq!(spec.access, Access::Record);
        assert!(spec.undoable);
        assert_eq!(spec.needs, vec!["bookmarks.write".to_string()]);
        let out = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "acme.page.star",
                json!({ "uri": "oxplow:oxplow/page?page=metrics", "page_kind": "metrics", "scope": "project", "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["scope"], json!("project"));

        // A replacement that doesn't stage is rejected whole: the old
        // command stays as it was, and the problem is reported.
        star("erase");
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let kept = fx.svc.commands.spec("acme.page.star").expect("kept");
        assert_eq!(kept, spec);
        let problem = held_problem(&fx, "acme").unwrap();
        assert!(problem.contains("has no op `erase`"), "{problem}");
    }

    /// A scope the command's script calls but its `needs` don't declare is
    /// refused at load: the command isn't offered, and nothing runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_scope_the_command_didnt_declare_is_refused() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        write_ext(
            fx._dir.path(),
            "my-review",
            "  - name: review.finish\n    summary: Finish the task.\n    input_schema: { type: object }\n    entry: handlers/finish.star\n",
            &[("handlers/finish.star", FINISH)],
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(fx.svc.commands.spec("my_review.review.finish").is_none());
        let errors = held_problem(&fx, "my-review").unwrap();
        assert!(
            errors.contains("uses `scope()` without `sql.read` in its `needs`"),
            "{errors}"
        );
        assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::InProgress);
    }

    /// A truncated read must stop composition before its children can
    /// change any records. Narrowing the query explicitly still works.
    #[tokio::test(flavor = "multi_thread")]
    async fn sql_read_truncation_prevents_a_commands_child_writes() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let sql = "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM numbers WHERE n < 1001) SELECT n FROM numbers ORDER BY n";
        let handler = |query: &str| {
            format!(
                r#"def transform(x):
    rows = scope("sql.read", {{"sql": {query:?}}})
    return {{
        "commands": [{{
            "name": "oxplow.work_item.transition",
            "input": {{"uri": x["input"]["uri"], "to": "done"}},
        }}],
        "result": {{"rows": len(rows)}},
    }}
"#
            )
        };
        with_finish(&fx, &handler(sql)).await;
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("`sql.read` returned more than 1000 rows"),
            "{err}"
        );
        assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::InProgress);

        // A result bounded by the author's SQL is complete for that query.
        with_finish(&fx, &handler(&format!("{sql} LIMIT 1000"))).await;
        let out = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["result"]["rows"], json!(1000));
        assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::Done);
    }

    /// `access: read`: it reads and answers, and nothing is recorded; one
    /// that composes a command is refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_command_answers_without_a_record() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let with_count = |script: &str| {
            write_ext(
                fx._dir.path(),
                "my-review",
                "  - name: work.count\n    summary: Count the work items.\n    input_schema: { type: object }\n    entry: handlers/count.star\n    access: read\n    needs: [sql.read]\n",
                &[("handlers/count.star", script)],
            );
        };
        let agent = agent(&fx);
        let run = || {
            fx.svc
                .commands
                .run(&agent, "my_review.work.count", json!({}), false)
        };
        with_count(
            "def transform(x):\n    rows = scope(\"sql.read\", {\"sql\": \"SELECT count(*) AS n FROM v_work_item\"})\n    return {\"commands\": [], \"result\": {\"n\": rows[0][\"n\"]}}\n",
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let out = run().await.unwrap();
        assert_eq!(out.result["result"]["n"], json!(1));
        assert_eq!(out.audit_id, None, "a read isn't recorded");

        with_count(&format!(
            "def transform(x):\n    return {{\"commands\": [{{\"name\": \"oxplow.work_item.transition\", \"input\": {{\"uri\": \"{}\", \"to\": \"done\"}}}}]}}\n",
            oxplow_tasks::work_item_ref(fx.task)
        ));
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let err = run().await.unwrap_err();
        assert!(err.to_string().contains("composes no commands"), "{err}");
        assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::InProgress);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_destructive_child_makes_a_persons_run_ask_first() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.delete\", \"input\": {\"uri\": x[\"input\"][\"uri\"]}}]}\n",
        )
        .await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap_err();
        let oxplow_domain::CommandError::NeedsConfirmation { preview } = err else {
            panic!("{err:?}");
        };
        assert!(preview.destructive);
        // A work list's verb runs outside the transaction: nothing of it
        // runs before the person confirms.
        assert!(task(&fx).await.deleted_at.is_none(), "nothing ran");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_that_misbehaves_writes_nothing() {
        for (script, says) in [
            ("def transform(x):\n    return [1]\n", "the script must return"),
            (
                "def transform(x):\n    return {\"commands\": [], \"result\": 1 // 0}\n",
                "the script failed",
            ),
            (
                "def transform(x):\n    return {\"commands\": [{\"name\": \"no.such\", \"input\": {}}]}\n",
                "no.such",
            ),
            (
                "def transform(x):\n    return {\"commands\": [], \"note\": \"extra\"}\n",
                "the script must return",
            ),
        ] {
            let fx = crate::test_fixtures::services_with_task_effort().await;
            with_finish(&fx, script).await;
            let r = oxplow_tasks::work_item_ref(fx.task);
            let err = fx
                .svc
                .commands
                .run(&oxplow_domain::Actor::Human, "my_review.review.finish", json!({ "uri": r }), false)
                .await
                .unwrap_err();
            assert!(err.to_string().contains(says), "{script}: {err}");
            assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::InProgress);
        }
    }

    /// A command that composes itself is refused at the nesting limit,
    /// not run until the stack overflows; nothing it composed is kept.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_self_composing_command_stops_at_the_nesting_limit() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    r = x[\"input\"][\"uri\"]\n    return {\"commands\": [\n        {\"name\": \"oxplow.work_item.update\", \"input\": {\"uri\": r, \"title\": \"again\"}},\n        {\"name\": \"my_review.review.finish\", \"input\": {\"uri\": r}},\n    ]}\n",
        )
        .await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let before = task(&fx).await.title;
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, oxplow_domain::CommandError::Invalid { message, .. }
                if message.contains("nested")),
            "{err}"
        );
        assert_eq!(task(&fx).await.title, before, "nothing kept");
    }

    /// The script runs while the bus routes its run, so a runaway one is
    /// given up on after `COMMAND_SCRIPT_BUDGET`, not the collectors' two
    /// minutes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_runaway_script_is_given_up_on_within_the_budget() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    n = 0\n    for i in range(400000000):\n        n += i\n    return {\"commands\": []}\n",
        )
        .await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let started = std::time::Instant::now();
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            started.elapsed() < COMMAND_SCRIPT_BUDGET.timeout + std::time::Duration::from_secs(3),
            "{:?}: {err}",
            started.elapsed()
        );
        assert!(err.to_string().contains("time"), "{err}");
        assert_eq!(task(&fx).await.status, oxplow_tasks::TaskStatus::InProgress);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_reconciler_registers_enabled_extensions_commands_only() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        with_finish(&fx, FINISH).await;
        assert!(
            bus.spec("my_review.review.finish").is_some(),
            "enabled: registered"
        );
        let spec = bus.spec("my_review.review.finish").unwrap();
        assert_eq!(spec.lifecycle, oxplow_domain::Lifecycle::Experimental);
        assert!(
            spec.summary.contains("extension `my-review`"),
            "{}",
            spec.summary
        );

        write(
            fx._dir.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [my-review]\n",
        );
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(
            bus.spec("my_review.review.finish").is_none(),
            "disabled: gone"
        );

        // A namespace someone else holds (a provider's) is refused whole.
        std::fs::remove_file(fx._dir.path().join(".oxplow/personal.yaml")).unwrap();
        bus.register_namespace(
            "my_review",
            "provider:held",
            vec![crate::commands::Command::new(
                oxplow_domain::CommandSpec {
                    output_schema: None,
                    id: "my_review.review.held".into(),
                    summary: "A provider's.".into(),
                    input_schema: json!({ "type": "object" }),
                    job: false,
                    confirm: Confirm::Never,
                    undoable: false,
                    lifecycle: oxplow_domain::Lifecycle::Experimental,
                    atomicity: oxplow_domain::Atomicity::Tx,
                    access: Access::Write,
                    needs: Vec::new(),
                    ui: None,
                    op: None,
                    unrecorded: Vec::new(),
                },
                crate::commands::Handler::Tx(std::sync::Arc::new(|_, _| {
                    Ok(crate::commands::HandlerOutput::default())
                })),
            )
            .unwrap()],
        )
        .unwrap();
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(bus.spec("my_review.review.finish").is_none());
        assert!(
            bus.spec("my_review.review.held").is_some(),
            "the holder keeps it"
        );
        assert!(
            held_problem(&fx, "my-review")
                .is_some_and(|p| p.starts_with("the command namespace `my_review` is already provider:held's; declare another `namespace:`")),
            "{:?}",
            held_problem(&fx, "my-review")
        );
        // Only the colliding extension is held: oxplow's own commands stay.
        assert!(bus.spec("oxplow.config.get").is_some());
        assert_eq!(held_problem(&fx, "oxplow-foundation"), None);
    }

    /// A script declines with `{ refuse: "<why>" }`: the run is `Invalid`
    /// with that reason and writes nothing. A refusal composes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_refuses_with_its_reason() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    return {\"refuse\": \"it has unverified claims\"}\n",
        )
        .await;
        let r = oxplow_tasks::work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "uri": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, oxplow_domain::CommandError::Invalid { message, .. } if message == "it has unverified claims"),
            "{err:?}"
        );
        assert!(composed(json!({ "refuse": "no", "commands": [] }))
            .unwrap_err()
            .contains("a refusal composes nothing"),);
    }

    /// An example may stand in answers for its scope calls
    /// (`answers:`, so it doesn't depend on the project's data) and may
    /// expect a refusal (`refused:`, a part of its reason).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_example_runs_on_its_answers_and_may_expect_a_refusal() {
        let transition_schema = serde_json::json!({
            "type": "object",
            "required": ["uri", "to"],
            "properties": { "uri": { "type": "string" }, "to": { "type": "string" } },
            "additionalProperties": false
        });
        let schema = move |name: &str| {
            (name == "oxplow.work_item.transition").then(|| transition_schema.clone())
        };
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        const SCRIPT: &str = "def transform(x):
    rows = scope(\"sql.read\", {\"sql\": \"SELECT uri FROM v_work_item WHERE uri = :uri\", \"params\": {\"uri\": x[\"input\"][\"uri\"]}})
    if not rows:
        return {\"refuse\": \"no such item\"}
    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"uri\": rows[0][\"uri\"], \"to\": \"done\"}}]}
";
        let check = |examples: &str| {
            let d = tempfile::tempdir().unwrap();
            write_ext(
                d.path(),
                "x",
                &format!(
                    "  - name: review.finish
    summary: Mark the task done.
    input_schema: {{ type: object, required: [uri], properties: {{ uri: {{ type: string }} }} }}
    entry: handlers/finish_review.star
    needs: [sql.read]
    examples:
{examples}"
                ),
                &[("handlers/finish_review.star", SCRIPT)],
            );
            d
        };
        let schema = &schema;
        let errors = |d: tempfile::TempDir| {
            let (layer, cat) = (&layer, &cat);
            async move {
                validate_extension(layer, cat, d.path(), "x", Some(schema))
                    .await
                    .unwrap()
                    .errors
                    .join("\n")
            }
        };

        // The project has no such item, but the example's answers stand in.
        let d = check(
            "      - { name: happy, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, answers: { sql.read: [[{ uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }]] }, expect_commands: [oxplow.work_item.transition] }\n      - { name: missing, input: { uri: \"oxplow:oxplow/work_item?id=tsk9&provider=oxplow\" }, answers: { sql.read: [[]] }, refused: no such item }\n",
        );
        assert_eq!(errors(d).await, "");

        for (example, says) in [
            (
                "      - { name: happy, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, expect_commands: [oxplow.work_item.transition] }\n",
                "example `happy`: refused (no such item) but `expect_commands` is [oxplow.work_item.transition]",
            ),
            (
                "      - { name: missing, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, answers: { sql.read: [[{ uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }]] }, refused: no such item }\n",
                "example `missing`: composed [oxplow.work_item.transition] but it should refuse (no such item)",
            ),
            (
                "      - { name: missing, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, answers: { sql.read: [[]] }, refused: unverified }\n",
                "example `missing`: refused (no such item) but it should refuse (unverified)",
            ),
            (
                "      - { name: both, input: { uri: \"oxplow:oxplow/work_item?id=tsk1&provider=oxplow\" }, answers: { sql.read: [[]] }, refused: x, expect_commands: [oxplow.work_item.transition] }\n",
                "example `both` expects commands and a refusal",
            ),
        ] {
            let errs = errors(check(example)).await;
            assert!(errs.contains(says), "{example}: {errs}");
        }
    }
}
