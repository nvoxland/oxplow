//! The `commands:` kind of `extension.yaml` (P6b, `.context/extensions.md`
//! → "Commands"): a command whose handler is a Starlark script that
//! composes core commands.
//!
//! ```yaml
//! commands:
//!   - name: review.finish                # registered as <namespace>.review.finish
//!     summary: Mark the task done and leave a note.
//!     input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }
//!     entry: handlers/finish_review.star # defines transform({ input })
//!     needs: [sql.read]                  # the host capabilities it calls
//!     confirm: never                     # never | always | destructive
//!     effect: write                      # write | record | read
//!     invokers: { human: true, agent: true, lens: true }
//!     examples:
//!       - { name: happy, input: { ref: "work_item:oxplow:tsk1" }, expect_commands: [oxplow.work_item.transition] }
//!       - { name: gone, input: { ref: "work_item:oxplow:tsk9" }, answers: { sql.read: [[]] }, refuses: no such task }
//! ```
//!
//! The script reads with `capability("sql.read", { sql, params })` (one
//! of its `needs`; `crate::host_capabilities`) and returns `{ commands:
//! [{ name, input }], result? }`, or `{ refuse: "<why>" }` to decline (the
//! run is `Invalid` with that reason). The namespace is the extension's
//! name with `-` → `_`.

use std::collections::BTreeMap;

use oxplow_domain::{CommandCall, CommandEffect, CommandSpec, Confirm, InputValidator, Invokers};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::extensions::manifest_v2::{at, entry_line, key_line};
use crate::extensions::{CommandSchemas, Extension};

/// An example run of a command: its input, and the commands its script
/// should compose, in order — or the refusal it should make (checked by
/// `oxplow plugin check` / Settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandExample {
    pub name: String,
    #[specta(type = oxplow_domain::Json)]
    pub input: Value,
    /// Answers standing in for the capabilities' own, per capability in
    /// call order (so the example doesn't depend on the project's data);
    /// a capability without any is served for real.
    #[specta(type = BTreeMap<String, Vec<oxplow_domain::Json>>)]
    pub answers: BTreeMap<String, Vec<Value>>,
    pub expect_commands: Vec<String>,
    /// A part of the reason the script should refuse with.
    pub refuses: Option<String>,
}

/// A command an extension declares (valid ones; invalid ones are in the
/// extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCommand {
    /// Its name on the bus: `<namespace>.<name>`.
    pub name: String,
    pub summary: String,
    /// Its input's schema; `None` for one backed by a capability's
    /// operation (the operation's).
    #[specta(type = Option<oxplow_domain::Json>)]
    pub input_schema: Option<Value>,
    /// What runs it.
    pub handler: CommandHandler,
    pub confirm: Confirm,
    pub effect: CommandEffect,
    pub invokers: Invokers,
    /// The host capabilities its script calls, and the capabilities (or
    /// features) it needs active, as core's commands declare them
    /// (`oxplow_domain::capability::check_need`).
    pub needs: Vec<String>,
    /// How a person meets it (label, group, …), as core's commands do.
    pub ui: Option<oxplow_domain::CommandUi>,
    /// Stable: a shared extension's commands are; a private one's are
    /// experimental.
    pub stable: bool,
    pub examples: Vec<CommandExample>,
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
    /// One operation of a host capability (`capability:` + `op:`,
    /// `commands::ops`).
    Capability { capability: String, op: String },
}

/// A `commands:` entry as the manifest holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandFile {
    name: String,
    summary: String,
    #[serde(default)]
    input_schema: Option<Value>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    confirm: Option<String>,
    #[serde(default)]
    effect: Option<String>,
    #[serde(default)]
    invokers: Option<Invokers>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    ui: Option<oxplow_domain::CommandUi>,
    /// `experimental` keeps a shared extension's command experimental.
    #[serde(default)]
    lifecycle: Option<String>,
    #[serde(default)]
    examples: Vec<ExampleFile>,
}

#[derive(Debug, Deserialize)]
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
    refuses: Option<String>,
}

/// The namespace an extension's commands register under when its
/// manifest declares none: its name with `-` → `_` (a command id's
/// segments are snake_case).
pub fn command_namespace(extension: &str) -> String {
    extension.replace('-', "_")
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
    shared: bool,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
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
    for item in items {
        let entry: CommandFile = match serde_yaml::from_value(item.clone()) {
            Ok(e) => e,
            Err(e) => {
                errors.push(at(file, block_line, format!("command: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "commands", "name", &entry.name).or(block_line);
        match command_of(namespace, shared, entry, read) {
            Ok(c) if out.iter().any(|o| o.name == c.name) => errors.push(at(
                file,
                line,
                format!("command `{}` is declared twice", c.name),
            )),
            Ok(c) => out.push(c),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn command_of(
    namespace: &str,
    shared: bool,
    f: CommandFile,
    read: &dyn Fn(&str) -> Option<String>,
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
    let confirm = match f.confirm.as_deref() {
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
    if f.ui.as_ref().is_some_and(|ui| ui.label.trim().is_empty()) {
        return Err(at_name("`ui.label` must say what a person reads".into()));
    }
    let (handler, input_schema, effect) = match (f.capability, f.entry) {
        (Some(_), Some(_)) => {
            return Err(at_name(
                "declares both `capability:` and `entry:` — a command has one handler".into(),
            ))
        }
        (None, None) => {
            return Err(at_name(
                "has no handler: give `entry:` (a Starlark script) or `capability:` + `op:` \
                 (an operation of a host capability)"
                    .into(),
            ))
        }
        (Some(capability), None) => {
            let class = oxplow_domain::host_capability::host_capability(&capability)
                .ok_or_else(|| at_name(format!("`capability`: no host capability `{capability}`")))?
                .class;
            let op = f.op.filter(|op| valid_segment(op)).ok_or_else(|| {
                at_name(format!(
                    "`capability: {capability}` needs `op:`, the operation it runs"
                ))
            })?;
            for (given, key) in [
                (f.input_schema.is_some(), "input_schema"),
                (f.effect.is_some(), "effect"),
                (!f.examples.is_empty(), "examples"),
            ] {
                if given {
                    return Err(at_name(format!(
                        "`{key}` comes from the capability's operation — drop it"
                    )));
                }
            }
            (
                CommandHandler::Capability { capability, op },
                None,
                crate::commands::ops::effect_of(class),
            )
        }
        (None, Some(entry)) => {
            if f.op.is_some() {
                return Err(at_name(
                    "`op:` names an operation of a `capability:`".into(),
                ));
            }
            let input_schema = f
                .input_schema
                .ok_or_else(|| at_name("a script's command declares `input_schema`".into()))?;
            InputValidator::compile(&input_schema)
                .map_err(|e| at_name(format!("`input_schema` doesn't compile: {e}")))?;
            let effect = script_effect(&f.effect, &f.confirm, &f.needs).map_err(at_name)?;
            if !inside(&entry) {
                return Err(at_name(format!(
                    "entry `{entry}` must be a path inside the extension folder"
                )));
            }
            let script = read(&entry)
                .ok_or_else(|| at_name(format!("entry `{entry}` isn't a file in the extension")))?;
            oxplow_collect_plugin::runtime::check_starlark(&entry, &script)
                .map_err(|e| at_name(format!("entry `{entry}` {e}")))?;
            (
                CommandHandler::Script { entry, script },
                Some(input_schema),
                effect,
            )
        }
    };
    if f.examples.len() > MAX_EXAMPLES {
        return Err(at_name(format!(
            "declares {} examples; a command has at most {MAX_EXAMPLES} examples",
            f.examples.len()
        )));
    }
    // Stable when shared, unless it says it's experimental; a private
    // extension's are experimental.
    let stable = match f.lifecycle.as_deref() {
        None | Some("stable") => shared,
        Some("experimental") => false,
        Some(other) => {
            return Err(at_name(format!(
                "`lifecycle` must be `stable` or `experimental`, not `{other}`"
            )))
        }
    };
    Ok(ExtensionCommand {
        name,
        summary: f.summary,
        input_schema,
        handler,
        confirm,
        effect,
        invokers: f.invokers.unwrap_or(Invokers::ALL),
        needs: f.needs,
        ui: f.ui,
        stable,
        examples: f
            .examples
            .into_iter()
            .map(|e| {
                if e.refuses.is_some() && !e.expect_commands.is_empty() {
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
                    refuses: e.refuses,
                })
            })
            .collect::<Result<_, _>>()?,
    })
}

/// A script's declared `effect` (default `write`): `read` only when it
/// needs nothing that changes things, and is never confirmed.
fn script_effect(
    effect: &Option<String>,
    confirm: &Option<String>,
    needs: &[String],
) -> Result<CommandEffect, String> {
    let effect = match effect.as_deref() {
        None | Some("write") => CommandEffect::Write,
        Some("record") => CommandEffect::Record,
        Some("read") => CommandEffect::Read,
        Some(other) => {
            return Err(format!(
                "`effect` must be `write`, `record` or `read`, not `{other}`"
            ))
        }
    };
    if effect == CommandEffect::Read {
        use oxplow_domain::host_capability::{host_capability, EffectClass};
        if let Some(writes) = needs
            .iter()
            .find(|n| host_capability(n).is_some_and(|c| c.class > EffectClass::Read))
        {
            return Err(format!(
                "`effect: read` but it needs `{writes}`, which changes things"
            ));
        }
        if confirm.as_deref().is_some_and(|c| c != "never") {
            return Err("`effect: read`: a command that only reads is never confirmed".into());
        }
    }
    Ok(effect)
}

/// Two enabled extensions whose commands map to one namespace: both are
/// refused (an error on each, their commands dropped).
pub fn refuse_shared_namespaces(extensions: &mut [Extension]) {
    let mut by_ns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in extensions.iter().enumerate() {
        // `oxplow` is shared by oxplow's own extensions (only they may
        // declare it); any other namespace is one extension's.
        if e.enabled && !e.commands.is_empty() && e.namespace != oxplow_domain::OXPLOW_NAMESPACE {
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
                        oxplow_domain::events::schema::plugin_namespace(extension),
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

/// How long a command's script may run. It runs inside the bus's write
/// transaction, holding the write lock, so this is far tighter than a
/// collector's runaway catch: composing a few commands is milliseconds.
/// (A timeout detaches the worker rather than stopping it — see
/// `run_sandboxed` — but the transaction is released.)
pub const COMMAND_SCRIPT_BUDGET: oxplow_collect_plugin::SandboxBudget =
    oxplow_collect_plugin::SandboxBudget::with_timeout(std::time::Duration::from_secs(5));

/// The most examples one command may declare: `check_extension` runs
/// each one's script.
pub const MAX_EXAMPLES: usize = 10;

/// Run a command's script over `{ input }` in the sandbox
/// ([`COMMAND_SCRIPT_BUDGET`]), its `capability` calls answered by
/// `calls`, and read what it composes: the one compose step, for the
/// handler and the examples dry run alike. Blocks: the handler calls it
/// inside the bus's transaction (off the async runtime already).
pub fn compose_calls(
    script: &str,
    input: Value,
    calls: &mut crate::host_capabilities::Calls<'_>,
) -> Result<Composed, oxplow_domain::CommandError> {
    let out = oxplow_collect_plugin::capability::run_starlark_serving(
        &COMMAND_SCRIPT_BUDGET,
        script,
        &json!({ "input": input }),
        &mut |id, args| calls.serve(id, args),
    );
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
/// invokers, policy and confirmation; one audit row; the reversed children
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
    let spec = CommandSpec {
        id: decl.name.clone(),
        summary,
        input_schema: decl.input_schema.clone().unwrap_or(Value::Null),
        invokers: decl.invokers,
        confirm: decl.confirm,
        undoable: true,
        lifecycle: if decl.stable {
            Lifecycle::Stable
        } else {
            Lifecycle::Experimental
        },
        atomicity: Atomicity::Dispatch,
        effect: decl.effect,
        needs: decl.needs.clone(),
        ui: decl.ui.clone(),
        op: None,
    };
    let script = match &decl.handler {
        CommandHandler::Capability { capability, op } => {
            let backing = bus
                .op(capability, op)
                .ok_or_else(|| CommandError::Invalid {
                    field: None,
                    message: format!("the host capability `{capability}` has no op `{op}`"),
                })?;
            return backing.command(spec);
        }
        CommandHandler::Script { script, .. } => script.clone(),
    };
    let (needs, reads) = (decl.needs.clone(), decl.effect == CommandEffect::Read);
    let (vocabulary, extension) = (bus.vocabulary().clone(), extension.to_string());
    let source = format!("extension:{extension}/{}", decl.name);
    let compose: std::sync::Arc<Composer> = std::sync::Arc::new(
        move |conn: &rusqlite::Connection,
              trace: &crate::host_capabilities::CapabilityTrace,
              input: &Value| {
            let read =
                Box::new(|q: oxplow_db::SqlQuery| oxplow_db::semantic_layer::read_on(conn, &q));
            let mut calls = crate::host_capabilities::Calls::new(&needs, trace, read);
            match compose_calls(&script, input.clone(), &mut calls)? {
                Composed::Run { calls, events, .. }
                    if reads && !(calls.is_empty() && events.is_empty()) =>
                {
                    Err(CommandError::Invalid {
                        field: None,
                        message: "a command that only reads (`effect: read`) composes no \
                                  commands and logs no events — it returns a `result`"
                            .into(),
                    })
                }
                Composed::Run {
                    calls,
                    result,
                    events,
                } => Ok(Composition {
                    calls,
                    result,
                    events: own_events(&vocabulary.current(), &extension, &source, events)?,
                }),
                Composed::Refused(message) => Err(CommandError::Invalid {
                    field: None,
                    message,
                }),
            }
        },
    );
    let handler = Compose::handler(bus, spec.clone(), compose);
    Command::new(spec, handler)
}

/// Register the commands of oxplow's required extensions
/// (`oxplow-foundation`: its own commands) on `bus`, whose operations are
/// already added — when services are built, before anything runs a
/// command, and once: they're compiled in, so they never change. An error
/// is a broken build (a declaration naming an op that isn't there).
pub fn register_required(bus: &std::sync::Arc<crate::commands::CommandBus>) -> Result<(), String> {
    for b in crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| b.required)
    {
        let file = format!("{}/extension.yaml", b.name);
        let manifest = b
            .file("extension.yaml")
            .ok_or_else(|| format!("{file} is missing"))?;
        let doc: serde_yaml::Value =
            serde_yaml::from_str(manifest).map_err(|e| format!("{file}: {e}"))?;
        let namespace = doc["namespace"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| command_namespace(b.name));
        let (decls, errors) = parse_commands(
            &namespace,
            true,
            &doc["commands"],
            &file,
            manifest,
            &|path| b.file(path).map(str::to_string),
        );
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        let built = decls
            .iter()
            .map(|decl| {
                extension_command(bus, b.name, decl).map_err(|e| format!("`{}`: {e}", decl.name))
            })
            .collect::<Result<Vec<_>, _>>()?;
        bus.register_namespace(&namespace, &format!("extension:{}", b.name), built)
            .map_err(|e| format!("{file}: {e}"))?;
    }
    Ok(())
}

/// The commands oxplow's required extensions declare over the operations
/// `bus` has, skipping the rest — a bus with one area's operations (a
/// test's, `oxplow-dev`'s). Services register them all
/// (`register_required`).
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
        );
        let built: Vec<_> = decls
            .iter()
            .filter(|d| match &d.handler {
                CommandHandler::Capability { capability, op } => bus.op(capability, op).is_some(),
                CommandHandler::Script { .. } => false,
            })
            .map(|d| extension_command(bus, b.name, d).expect("it builds"))
            .collect();
        bus.register_namespace(
            oxplow_domain::OXPLOW_NAMESPACE,
            &format!("extension:{}", b.name),
            built,
        )
        .expect("they register");
    }
}

/// Keeps the bus's extension commands matching the enabled extensions of
/// the primary worktree (where, like providers, they run from): each
/// extension's namespace registered all-or-nothing, re-registered when
/// its declarations change, removed when it is disabled or gone. A
/// namespace someone else already holds (a provider) is refused and
/// reported as the extension's [`ExtensionCommands::problem`].
pub struct ExtensionCommands {
    bus: std::sync::Arc<crate::commands::CommandBus>,
    catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
    root: std::path::PathBuf,
    /// What's registered, by namespace: the extension and its commands.
    registered: tokio::sync::Mutex<BTreeMap<String, (String, Vec<ExtensionCommand>)>>,
    problems: parking_lot::Mutex<BTreeMap<String, String>>,
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
            problems: parking_lot::Mutex::default(),
        }
    }

    /// Why `extension`'s commands aren't registered, if they aren't.
    pub fn problem(&self, extension: &str) -> Option<String> {
        self.problems.lock().get(extension).cloned()
    }

    /// Make the registered commands match the enabled extensions.
    pub async fn reconcile(&self) {
        let mut registered = self.registered.lock().await;
        let wanted: BTreeMap<String, (String, Vec<ExtensionCommand>)> = self
            .catalog
            .get(&self.root)
            .iter()
            // A required one's are registered with the services
            // (`register_required`) and never change.
            .filter(|e| {
                e.enabled
                    && !e.commands.is_empty()
                    && !crate::bundled_extensions::is_required(&e.name)
            })
            .map(|e| (e.name.clone(), (e.namespace.clone(), e.commands.clone())))
            .collect();
        // Keyed by extension: several of oxplow's share the `oxplow`
        // namespace, so it is the extension whose commands come and go.
        let stale: Vec<String> = registered
            .iter()
            .filter(|(extension, have)| wanted.get(*extension) != Some(*have))
            .map(|(extension, _)| extension.clone())
            .collect();
        for extension in stale {
            self.bus
                .unregister_source(&format!("extension:{extension}"));
            registered.remove(&extension);
        }
        let mut problems = BTreeMap::new();
        for (extension, (ns, commands)) in wanted {
            if registered.contains_key(&extension) {
                continue;
            }
            match self.register(&ns, &extension, &commands) {
                Ok(()) => {
                    registered.insert(extension, (ns, commands));
                }
                Err(problem) => {
                    tracing::warn!(extension, problem, "extension commands not registered");
                    problems.insert(extension, problem);
                }
            }
        }
        *self.problems.lock() = problems;
    }

    /// Register every command of one extension, or none.
    fn register(
        &self,
        ns: &str,
        extension: &str,
        commands: &[ExtensionCommand],
    ) -> Result<(), String> {
        let built = commands
            .iter()
            .map(|decl| {
                extension_command(&self.bus, extension, decl)
                    .map_err(|e| format!("`{}`: {e}", decl.name))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.bus
            .register_namespace(ns, &format!("extension:{extension}"), built)
            .map_err(|e| match e {
                oxplow_domain::CommandError::Invalid { message, .. } => {
                    format!("{message}; declare another `namespace:`")
                }
                other => other.to_string(),
            })
    }
}

/// Keep the extension commands matching the enabled extensions of the
/// primary worktree: once at boot, then whenever they may have changed
/// (the extension catalog's signal: a file under `oxplow/extensions/`
/// there, or the `extensions` config key).
pub fn spawn_reconciler(state: std::sync::Arc<crate::Services>) {
    let mut changes = state.extension_catalog.changes();
    tokio::spawn(async move {
        state.extension_commands.reconcile().await;
        // Lagging only means it missed some: one pass covers them.
        while let Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
            changes.recv().await
        {
            state.extension_commands.reconcile().await;
        }
    });
}

/// Check `ext`'s commands (`check_extension`): with the running oxplow's
/// registry, its namespace is free (or already its own); then every
/// example is dry-run — the script in the sandbox, its capability calls
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
        let mine = format!("extension:{}", ext.name);
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
    match (decided, &ex.refuses) {
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

/// Dry-run `cmd` on `input`: the script in the sandbox, its capability
/// calls answered from `answers` or for real (reads only: `sql.read`
/// through `layer`), and — when it composes — each command against
/// `registry` (it exists, its input fits). What the script decided;
/// nothing runs. `check`'s examples and `oxplow plugin test`'s intent
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
            "`{}` runs a capability's operation; only a script's command is dry-run",
            cmd.name
        ));
    };
    let (script, input, needs) = (script.clone(), input.clone(), cmd.needs.clone());
    let (layer, answers) = (layer.clone(), answers.clone());
    let runtime = tokio::runtime::Handle::current();
    let decided = tokio::task::spawn_blocking(move || {
        let trace = crate::host_capabilities::CapabilityTrace::default();
        let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(layer.run(q)));
        let mut calls =
            crate::host_capabilities::Calls::new(&needs, &trace, read).with_answers(&answers);
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
    use super::*;
    use crate::extensions::{load_extensions, validate_extension, Extension};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Moves a task to done.
    const HANDLER: &str = "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"to\": \"done\"}}]}\n";

    const GOOD: &str = "  - name: review.finish
    summary: Mark the task done.
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } }, additionalProperties: false }
    entry: handlers/finish_review.star
    examples:
      - { name: happy, input: { ref: \"work_item:oxplow:tsk1\" }, expect_commands: [oxplow.work_item.transition] }
";

    /// Write extension `name` with `commands` (the block's entries) and
    /// `files` (relative to its folder).
    fn write_ext(root: &Path, name: &str, commands: &str, files: &[(&str, &str)]) {
        write(
            root,
            &format!("oxplow/extensions/{name}/extension.yaml"),
            &format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\ncommands:\n{commands}"),
        );
        for (rel, body) in files {
            write(root, &format!("oxplow/extensions/{name}/{rel}"), body);
        }
    }

    fn project(root: &Path, name: &str) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.origin == "project" && e.name == name)
            .unwrap()
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
            serde_json::json!(["ref"])
        );
        assert!(c.needs.is_empty());
        assert_eq!(c.confirm, Confirm::Never);
        assert_eq!(c.effect, CommandEffect::Write);
        assert_eq!(c.invokers, Invokers::ALL);
        assert_eq!(c.examples.len(), 1);
        assert_eq!(
            c.examples[0].expect_commands,
            vec!["oxplow.work_item.transition"]
        );
        assert_eq!(command_namespace("my-review"), "my_review");
    }

    /// A command declares the capabilities it needs and how a person meets
    /// it, like core's; a private extension's is experimental.
    #[test]
    fn a_command_declares_its_needs_and_how_a_person_meets_it() {
        let d = tempfile::tempdir().unwrap();
        let tail = "    needs: [work_items.comments]\n    ui:\n      label: Finish Review\n      group: Review\n      keywords: [done]\n      about: work_item\n      input: { ref: \"{{ref}}\" }\n";
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
        assert_eq!(ui.about.as_deref(), Some("work_item"));
        assert_eq!(ui.input, Some(serde_json::json!({ "ref": "{{ref}}" })));
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
                entry("a.b", "    effect: sometimes\n"),
                vec![("handlers/h.star", HANDLER)],
                "`effect` must be `write`, `record` or `read`",
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
                entry("a.b", "    effect: often\n"),
                vec![("handlers/h.star", HANDLER)],
                "effect",
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
                entry("a.b", "    capability: bookmarks.write\n    op: set\n"),
                vec![("handlers/h.star", HANDLER)],
                "both `capability:` and `entry:`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    capability: bookmarks.erase\n    op: set\n".into(),
                vec![],
                "no host capability `bookmarks.erase`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    capability: bookmarks.write\n".into(),
                vec![],
                "needs `op:`",
            ),
            (
                "  - name: a.b\n    summary: S.\n    capability: bookmarks.write\n    op: set\n    input_schema: { type: object }\n".into(),
                vec![],
                "`input_schema` comes from the capability's operation",
            ),
            (
                entry("a.b", "    effect: read\n    confirm: always\n"),
                vec![("handlers/h.star", HANDLER)],
                "only reads is never confirmed",
            ),
            (
                "  - name: a.b\n    summary: S.\n    input_schema: { type: nope }\n    entry: handlers/h.star\n".into(),
                vec![("handlers/h.star", HANDLER)],
                "input_schema",
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
    /// a disabled one doesn't count.
    #[test]
    fn a_namespace_is_one_extensions() {
        let d = tempfile::tempdir().unwrap();
        let files = [("handlers/finish_review.star", HANDLER)];
        write_ext(d.path(), "a-b", GOOD, &files);
        write_ext(d.path(), "a_b", GOOD, &files);
        for name in ["a-b", "a_b"] {
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
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [a_b]\n",
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
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let run = || async {
            fx.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "my_review.review.finish",
                    json!({ "ref": r }),
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
                        id: "held.review.held".into(),
                        summary: "A provider's.".into(),
                        input_schema: json!({ "type": "object" }),
                        invokers: Invokers::ALL,
                        confirm: Confirm::Never,
                        undoable: false,
                        lifecycle: oxplow_domain::Lifecycle::Experimental,
                        atomicity: oxplow_domain::Atomicity::Tx,
                        effect: CommandEffect::Write,
                        needs: Vec::new(),
                        ui: None,
                        op: None,
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

        write_ext(root, "mine", GOOD, &files);
        fx.svc.extension_commands.reconcile().await;
        assert_eq!(
            fx.svc.commands.namespace_owner("mine").as_deref(),
            Some("extension:mine")
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
            "required": ["ref", "to"],
            "properties": { "ref": { "type": "string" }, "to": { "type": "string" } },
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
                        "def transform(x):\n    rows = capability(\"sql.read\", {{\"sql\": \"{sql}\", \"params\": {{\"ref\": x[\"input\"][\"ref\"]}}}})\n    return {{\"commands\": [{{\"name\": \"oxplow.work_item.transition\", \"input\": {{\"ref\": rows[0][\"r\"], \"to\": \"done\"}}}}]}}\n"
                    ),
                )],
            );
            d
        };
        let d = reads("SELECT :ref AS r");
        let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        // A read of a table, not a published model, is refused.
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
                "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"ref\": 1}}]}\n",
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
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } }, additionalProperties: false }
    entry: handlers/finish.star
    needs: [sql.read]
",
            &[("handlers/finish.star", script)],
        );
        fx.svc.extension_commands.reconcile().await;
    }

    /// Transitions the task to done and renames it after its row.
    const FINISH: &str = "def transform(x):
    row = capability(\"sql.read\", {
        \"sql\": \"SELECT ref, title FROM v_work_item WHERE ref = :ref\",
        \"params\": {\"ref\": x[\"input\"][\"ref\"]},
    })[0]
    return {
        \"commands\": [
            {\"name\": \"oxplow.work_item.update\", \"input\": {\"ref\": row[\"ref\"], \"title\": row[\"title\"] + \" (reviewed)\"}},
            {\"name\": \"oxplow.work_item.transition\", \"input\": {\"ref\": row[\"ref\"], \"to\": \"done\"}},
        ],
        \"result\": {\"finished\": row[\"ref\"]},
    }
";

    fn agent(fx: &crate::test_fixtures::EffortFixture) -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn task(fx: &crate::test_fixtures::TaskEffortFixture) -> oxplow_domain::Task {
        use oxplow_domain::stores::TaskStore as _;
        fx.svc.task_store.get(fx.task).await.unwrap().unwrap()
    }

    /// `my-review`, declaring `my_review.finished@1`, with a `finish`
    /// command whose script emits `{type}` for its `ref`; registered.
    async fn with_emitting_finish(fx: &crate::test_fixtures::EffortFixture, event_type: &str) {
        let root = fx._dir.path();
        write(
            root,
            "oxplow/extensions/my-review/extension.yaml",
            "manifest: 2\nname: my-review\nintent:\n  purpose: p\nevent_types:\n  types:\n    - { type: my_review.finished, v: 1, schema: finished.json, summary: A review finished. }\ncommands:\n  - name: review.finish\n    summary: Finish the review.\n    input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }\n    entry: finish.star\n",
        );
        write(
            root,
            "oxplow/extensions/my-review/finished.json",
            r#"{"type": "object", "required": ["ref"], "properties": {"ref": {"type": "string"}}}"#,
        );
        write(
            root,
            "oxplow/extensions/my-review/finish.star",
            &format!(
                "def transform(x):\n    return {{\"commands\": [], \"events\": [{{\"type\": \"{event_type}\", \"payload\": {{\"ref\": x[\"input\"][\"ref\"]}}, \"subject\": [x[\"input\"][\"ref\"]]}}]}}\n"
            ),
        );
        fx.svc.vocabulary_service.sync().await.unwrap();
        fx.svc.extension_commands.reconcile().await;
    }

    /// P8.D4: a script's `events` append its extension's own types,
    /// caused by the run's `command.executed`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_emits_its_own_extensions_event_type() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_emitting_finish(&fx, "my_review.finished").await;
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let out = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "ref": r }),
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
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        for foreign in ["work_item.created", "my_review.undeclared"] {
            with_emitting_finish(&fx, foreign).await;
            let err = fx
                .svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "my_review.review.finish",
                    json!({ "ref": r }),
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
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let out = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                "my_review.review.finish",
                json!({ "ref": r }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["result"], json!({ "finished": r }));
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        let t = task(&fx).await;
        assert_eq!(t.status, oxplow_domain::TaskStatus::Done);
        assert_eq!(t.title, "t (reviewed)");
        let audits = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
        let ok: Vec<_> = audits
            .iter()
            .filter(|a| a.outcome == oxplow_domain::events::schema::CommandOutcome::Ok)
            .collect();
        assert_eq!(ok.len(), 1, "one audit row for the run");
        assert_eq!(ok[0].command, "my_review.review.finish");
        assert_eq!(
            ok[0].capabilities,
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
        // One undo reverses both children.
        fx.svc
            .commands
            .undo(&agent(&fx), out.audit_id.unwrap(), false)
            .await
            .unwrap();
        let t = task(&fx).await;
        assert_eq!(t.status, oxplow_domain::TaskStatus::InProgress);
        assert_eq!(t.title, "t");
    }

    /// A project extension declares its own command over one of the
    /// operations oxplow's commands are backed by: nothing about
    /// oxplow's is special. Its spec is the operation's (schema, undo,
    /// effect), the capability among its needs; one naming an operation
    /// that isn't there isn't registered, its problem said.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_extension_declares_a_command_over_an_operation() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let star = |op: &str| {
            write_ext(
                fx._dir.path(),
                "acme",
                &format!(
                    "  - name: page.star\n    summary: Star the page for the project.\n    capability: bookmarks.write\n    op: {op}\n    invokers: {{ human: true, agent: false, lens: false }}\n"
                ),
                &[],
            );
        };
        star("set");
        fx.svc.extension_commands.reconcile().await;
        let spec = fx.svc.commands.spec("acme.page.star").expect("registered");
        let oxplows = fx.svc.commands.spec("oxplow.bookmark.set").unwrap();
        assert_eq!(spec.input_schema, oxplows.input_schema);
        assert_eq!(spec.effect, CommandEffect::Record);
        assert!(spec.undoable);
        assert_eq!(spec.needs, vec!["bookmarks.write".to_string()]);
        let out = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "acme.page.star",
                json!({ "ref": "page:metrics", "page_kind": "metrics", "scope": "project", "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["scope"], json!("project"));

        star("erase");
        fx.svc.extension_commands.reconcile().await;
        assert!(fx.svc.commands.spec("acme.page.star").is_none());
        let problem = fx.svc.extension_commands.problem("acme").unwrap();
        assert!(problem.contains("has no op `erase`"), "{problem}");
    }

    /// A capability the command didn't declare in `needs` is refused: the
    /// run fails, writing nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_capability_the_command_didnt_declare_is_refused() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        write_ext(
            fx._dir.path(),
            "my-review",
            "  - name: review.finish\n    summary: Finish the task.\n    input_schema: { type: object }\n    entry: handlers/finish.star\n",
            &[("handlers/finish.star", FINISH)],
        );
        fx.svc.extension_commands.reconcile().await;
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "ref": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("`sql.read` isn't in the command's `needs`"),
            "{err}"
        );
        assert_eq!(
            task(&fx).await.status,
            oxplow_domain::TaskStatus::InProgress
        );
    }

    /// `effect: read`: it reads and answers, and nothing is recorded; one
    /// that composes a command is refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_command_answers_without_a_record() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let with_count = |script: &str| {
            write_ext(
                fx._dir.path(),
                "my-review",
                "  - name: work.count\n    summary: Count the work items.\n    input_schema: { type: object }\n    entry: handlers/count.star\n    effect: read\n    needs: [sql.read]\n",
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
            "def transform(x):\n    rows = capability(\"sql.read\", {\"sql\": \"SELECT count(*) AS n FROM v_work_item\"})\n    return {\"commands\": [], \"result\": {\"n\": rows[0][\"n\"]}}\n",
        );
        fx.svc.extension_commands.reconcile().await;
        let out = run().await.unwrap();
        assert_eq!(out.result["result"]["n"], json!(1));
        assert_eq!(out.audit_id, None, "a read isn't recorded");

        with_count(&format!(
            "def transform(x):\n    return {{\"commands\": [{{\"name\": \"oxplow.work_item.transition\", \"input\": {{\"ref\": \"{}\", \"to\": \"done\"}}}}]}}\n",
            oxplow_domain::refs::build::work_item_ref(fx.task)
        ));
        fx.svc.extension_commands.reconcile().await;
        let err = run().await.unwrap_err();
        assert!(err.to_string().contains("composes no commands"), "{err}");
        assert_eq!(
            task(&fx).await.status,
            oxplow_domain::TaskStatus::InProgress
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_destructive_child_makes_the_agents_run_a_proposal_with_its_children() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.delete\", \"input\": {\"ref\": x[\"input\"][\"ref\"]}}]}\n",
        )
        .await;
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                "my_review.review.finish",
                json!({ "ref": r }),
                false,
            )
            .await
            .unwrap_err();
        let oxplow_domain::CommandError::Proposed { preview, .. } = err else {
            panic!("{err:?}");
        };
        assert!(preview.destructive);
        let pending = fx
            .svc
            .commands
            .proposal_store()
            .list_pending()
            .await
            .unwrap();
        assert_eq!(
            pending[0].dry_run.as_ref().unwrap()["children"][0]["name"],
            "oxplow.work_item.delete"
        );
        assert!(task(&fx).await.deleted_at.is_none(), "nothing ran");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_that_misbehaves_writes_nothing() {
        for (script, says) in [
            ("def transform(x):\n    return [1]\n", "the script must return"),
            // No host: a model call is refused (and `files()` sees no files).
            (
                "def transform(x):\n    return {\"commands\": [], \"result\": ai_summarize(\"x\")}\n",
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
            let r = oxplow_domain::refs::build::work_item_ref(fx.task);
            let err = fx
                .svc
                .commands
                .run(&oxplow_domain::Actor::Human, "my_review.review.finish", json!({ "ref": r }), false)
                .await
                .unwrap_err();
            assert!(err.to_string().contains(says), "{script}: {err}");
            assert_eq!(task(&fx).await.status, oxplow_domain::TaskStatus::InProgress);
        }
    }

    /// A command that composes itself is refused at the nesting limit,
    /// not run until the stack overflows; nothing it composed is kept.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_self_composing_command_stops_at_the_nesting_limit() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    r = x[\"input\"][\"ref\"]\n    return {\"commands\": [\n        {\"name\": \"oxplow.work_item.update\", \"input\": {\"ref\": r, \"title\": \"again\"}},\n        {\"name\": \"my_review.review.finish\", \"input\": {\"ref\": r}},\n    ]}\n",
        )
        .await;
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let before = task(&fx).await.title;
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "ref": r }),
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

    /// The script runs inside the bus's write transaction, so a runaway
    /// one is given up on after `COMMAND_SCRIPT_BUDGET`, not the
    /// collectors' two minutes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_runaway_script_is_given_up_on_within_the_budget() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        with_finish(
            &fx,
            "def transform(x):\n    n = 0\n    for i in range(400000000):\n        n += i\n    return {\"commands\": []}\n",
        )
        .await;
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let started = std::time::Instant::now();
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "ref": r }),
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
        assert_eq!(
            task(&fx).await.status,
            oxplow_domain::TaskStatus::InProgress
        );
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
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [my-review]\n",
        );
        fx.svc.extension_commands.reconcile().await;
        assert!(
            bus.spec("my_review.review.finish").is_none(),
            "disabled: gone"
        );

        // A namespace someone else holds (a provider's) is refused whole.
        std::fs::remove_file(fx._dir.path().join(".oxplow/project.yaml")).unwrap();
        bus.register_namespace(
            "my_review",
            "provider:held",
            vec![crate::commands::Command::new(
                oxplow_domain::CommandSpec {
                    id: "my_review.review.held".into(),
                    summary: "A provider's.".into(),
                    input_schema: json!({ "type": "object" }),
                    invokers: Invokers::ALL,
                    confirm: Confirm::Never,
                    undoable: false,
                    lifecycle: oxplow_domain::Lifecycle::Experimental,
                    atomicity: oxplow_domain::Atomicity::Tx,
                    effect: CommandEffect::Write,
                    needs: Vec::new(),
                    ui: None,
                    op: None,
                },
                crate::commands::Handler::Tx(std::sync::Arc::new(|_, _| {
                    Ok(crate::commands::HandlerOutput::default())
                })),
            )
            .unwrap()],
        )
        .unwrap();
        fx.svc.extension_commands.reconcile().await;
        assert!(bus.spec("my_review.review.finish").is_none());
        assert!(
            bus.spec("my_review.review.held").is_some(),
            "the holder keeps it"
        );
        assert!(
            fx.svc
                .extension_commands
                .problem("my-review")
                .is_some_and(|p| p == "the command namespace `my_review` is already provider:held's; declare another `namespace:`"),
            "{:?}",
            fx.svc.extension_commands.problem("my-review")
        );
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
        let r = oxplow_domain::refs::build::work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "my_review.review.finish",
                json!({ "ref": r }),
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

    /// An example may stand in answers for its capability calls
    /// (`answers:`, so it doesn't depend on the project's data) and may
    /// expect a refusal (`refuses:`, a part of its reason).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_example_runs_on_its_answers_and_may_expect_a_refusal() {
        let transition_schema = serde_json::json!({
            "type": "object",
            "required": ["ref", "to"],
            "properties": { "ref": { "type": "string" }, "to": { "type": "string" } },
            "additionalProperties": false
        });
        let schema = move |name: &str| {
            (name == "oxplow.work_item.transition").then(|| transition_schema.clone())
        };
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        const SCRIPT: &str = "def transform(x):
    rows = capability(\"sql.read\", {\"sql\": \"SELECT ref FROM v_work_item WHERE ref = :ref\", \"params\": {\"ref\": x[\"input\"][\"ref\"]}})
    if not rows:
        return {\"refuse\": \"no such item\"}
    return {\"commands\": [{\"name\": \"oxplow.work_item.transition\", \"input\": {\"ref\": rows[0][\"ref\"], \"to\": \"done\"}}]}
";
        let check = |examples: &str| {
            let d = tempfile::tempdir().unwrap();
            write_ext(
                d.path(),
                "x",
                &format!(
                    "  - name: review.finish
    summary: Mark the task done.
    input_schema: {{ type: object, required: [ref], properties: {{ ref: {{ type: string }} }} }}
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
            "      - { name: happy, input: { ref: \"work_item:oxplow:tsk1\" }, answers: { sql.read: [[{ ref: \"work_item:oxplow:tsk1\" }]] }, expect_commands: [oxplow.work_item.transition] }\n      - { name: missing, input: { ref: \"work_item:oxplow:tsk9\" }, answers: { sql.read: [[]] }, refuses: no such item }\n",
        );
        assert_eq!(errors(d).await, "");

        for (example, says) in [
            (
                "      - { name: happy, input: { ref: \"work_item:oxplow:tsk1\" }, expect_commands: [oxplow.work_item.transition] }\n",
                "example `happy`: refused (no such item) but `expect_commands` is [oxplow.work_item.transition]",
            ),
            (
                "      - { name: missing, input: { ref: \"work_item:oxplow:tsk1\" }, answers: { sql.read: [[{ ref: \"work_item:oxplow:tsk1\" }]] }, refuses: no such item }\n",
                "example `missing`: composed [oxplow.work_item.transition] but it should refuse (no such item)",
            ),
            (
                "      - { name: missing, input: { ref: \"work_item:oxplow:tsk1\" }, answers: { sql.read: [[]] }, refuses: unverified }\n",
                "example `missing`: refused (no such item) but it should refuse (unverified)",
            ),
            (
                "      - { name: both, input: { ref: \"work_item:oxplow:tsk1\" }, answers: { sql.read: [[]] }, refuses: x, expect_commands: [oxplow.work_item.transition] }\n",
                "example `both` expects commands and a refusal",
            ),
        ] {
            let errs = errors(check(example)).await;
            assert!(errs.contains(says), "{example}: {errs}");
        }
    }
}
