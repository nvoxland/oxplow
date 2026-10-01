//! The `commands:` kind of `extension.yaml` (P6b, `.context/extensions.md`
//! → "Commands"): a command whose handler is a Starlark script that
//! composes core commands.
//!
//! ```yaml
//! commands:
//!   - name: finish_review                # registered as <namespace>.finish_review
//!     summary: Mark the task done and leave a note.
//!     input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }
//!     entry: handlers/finish_review.star # defines transform({ input, rows })
//!     input: "SELECT ref, status FROM v_task WHERE ref = :ref"   # optional; one read
//!     confirm: never                     # never | always | destructive
//!     effect: write                      # write | record
//!     invokers: { human: true, agent: true, lens: true }
//!     examples:
//!       - { name: happy, input: { ref: "work_item:oxplow:tsk1" }, expect_commands: [work_item.transition] }
//! ```
//!
//! The namespace is the extension's name with `-` → `_`.

use std::collections::BTreeMap;

use oxplow_domain::{
    CommandCall, CommandEffect, CommandSpec, Confirm, InputValidator, Invokers,
    RESERVED_COMMAND_NAMESPACES,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::extensions::manifest_v2::{at, key_line, line_under};
use crate::extensions::{CommandSchemas, Extension};

/// The most rows a command's `input` query hands its script.
pub const INPUT_ROW_CAP: usize = 1_000;

/// An example run of a command: its input, and the commands its script
/// should compose, in order (checked by `oxplow plugin check` / Settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandExample {
    pub name: String,
    #[specta(type = oxplow_domain::Json)]
    pub input: Value,
    pub expect_commands: Vec<String>,
}

/// A command an extension declares (valid ones; invalid ones are in the
/// extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCommand {
    /// Its name on the bus: `<namespace>.<name>`.
    pub name: String,
    pub summary: String,
    #[specta(type = oxplow_domain::Json)]
    pub input_schema: Value,
    /// The script, relative to the extension folder.
    pub entry: String,
    /// The script's text, read at load.
    #[serde(skip)]
    pub script: String,
    /// A read-only SQL query whose rows the script gets (`:field` binds
    /// the input's top-level fields).
    pub input: Option<String>,
    pub confirm: Confirm,
    pub effect: CommandEffect,
    pub invokers: Invokers,
    pub examples: Vec<CommandExample>,
}

/// A `commands:` entry as the manifest holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandFile {
    name: String,
    summary: String,
    input_schema: Value,
    entry: String,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    confirm: Option<String>,
    #[serde(default)]
    effect: Option<String>,
    #[serde(default)]
    invokers: Option<Invokers>,
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
    expect_commands: Vec<String>,
}

/// The namespace an extension's commands register under: its name with
/// `-` → `_` (a command name's segments are snake_case).
pub fn command_namespace(extension: &str) -> String {
    extension.replace('-', "_")
}

fn valid_command_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> (Vec<ExtensionCommand>, Vec<String>) {
    let block_line = key_line(manifest, "commands");
    let namespace = command_namespace(extension);
    if RESERVED_COMMAND_NAMESPACES.contains(&namespace.as_str()) {
        return (
            Vec::new(),
            vec![at(
                file,
                block_line,
                format!(
                    "the command namespace `{namespace}` is oxplow's (an extension's commands \
                     register under its name with `-` → `_`); rename the extension"
                ),
            )],
        );
    }
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
        let line =
            line_under(manifest, "commands", &format!("name: {}", entry.name)).or(block_line);
        match command_of(&namespace, entry, read) {
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
    f: CommandFile,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<ExtensionCommand, String> {
    if !valid_command_name(&f.name) {
        return Err(format!(
            "command name `{}` must be lowercase letters, digits and underscores, starting with a \
             letter",
            f.name
        ));
    }
    let name = format!("{namespace}.{}", f.name);
    CommandSpec::validate_name(&name).map_err(|e| e.to_string())?;
    let at_name = |m: String| format!("command `{}`: {m}", f.name);
    let effect = match f.effect.as_deref() {
        None | Some("write") => CommandEffect::Write,
        Some("record") => CommandEffect::Record,
        Some("read") => {
            return Err(at_name(
                "`effect: read`: a command that only reads is a lens — write a lens instead".into(),
            ))
        }
        Some(other) => {
            return Err(at_name(format!(
                "`effect` must be `write` or `record`, not `{other}`"
            )))
        }
    };
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
    InputValidator::compile(&f.input_schema)
        .map_err(|e| at_name(format!("`input_schema` doesn't compile: {e}")))?;
    if let Some(sql) = &f.input {
        oxplow_db::sql_tokens::check_single_read(sql).map_err(|e| {
            at_name(format!(
                "`input` must be one read (a single SELECT or WITH): {}",
                e.to_string().replacen("invalid value: ", "", 1)
            ))
        })?;
    }
    if !inside(&f.entry) {
        return Err(at_name(format!(
            "entry `{}` must be a path inside the extension folder",
            f.entry
        )));
    }
    let script = read(&f.entry)
        .ok_or_else(|| at_name(format!("entry `{}` isn't a file in the extension", f.entry)))?;
    oxplow_collect_plugin::runtime::check_starlark(&script)
        .map_err(|e| at_name(format!("entry `{}` {e}", f.entry)))?;
    Ok(ExtensionCommand {
        name,
        summary: f.summary,
        input_schema: f.input_schema,
        entry: f.entry,
        script,
        input: f.input,
        confirm,
        effect,
        invokers: f.invokers.unwrap_or(Invokers::ALL),
        examples: f
            .examples
            .into_iter()
            .map(|e| CommandExample {
                name: e.name,
                input: e.input,
                expect_commands: e.expect_commands,
            })
            .collect(),
    })
}

/// Two enabled extensions whose commands map to one namespace: both are
/// refused (an error on each, their commands dropped).
pub fn refuse_shared_namespaces(extensions: &mut [Extension]) {
    let mut by_ns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in extensions.iter().enumerate() {
        if e.enabled && !e.commands.is_empty() {
            by_ns.entry(command_namespace(&e.name)).or_default().push(i);
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
                 command namespace is its name with `-` → `_`); rename one",
                ext.path,
                names.join(" and ")
            ));
            ext.commands.clear();
        }
    }
}

/// The commands a script composed, and its optional `result`:
/// `{ commands: [{ name, input }], result? }`.
pub fn composed(value: Value) -> Result<(Vec<CommandCall>, Option<Value>), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Out {
        commands: Vec<Call>,
        #[serde(default)]
        result: Option<Value>,
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
    let out: Out = serde_json::from_value(value).map_err(|e| {
        format!("the script must return `{{ commands: [{{ name, input }}], result? }}`: {e}")
    })?;
    Ok((
        out.commands
            .into_iter()
            .map(|c| CommandCall {
                name: c.name,
                input: c.input,
            })
            .collect(),
        out.result,
    ))
}

/// A query result as the script sees it: one object per row.
pub fn rows_json(result: &oxplow_db::SqlQueryResult) -> Vec<Value> {
    result
        .rows
        .iter()
        .map(|row| {
            Value::Object(
                result
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(c, v)| (c.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
                    .collect(),
            )
        })
        .collect()
}

/// The named parameters an input binds: its top-level fields.
pub fn input_params(input: &Value) -> Vec<(String, oxplow_db::SqlCell)> {
    input
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default()
}

/// Run a command's script over `{ input, rows }` in the sandbox (no host:
/// no files, no `ai_*`), off the async runtime.
pub async fn run_script(script: String, input: Value, rows: Vec<Value>) -> Result<Value, String> {
    use oxplow_collect_plugin::runtime::{run_sandboxed, run_starlark};
    tokio::task::spawn_blocking(move || {
        run_sandboxed(
            &oxplow_collect_plugin::SandboxBudget::default(),
            move || run_starlark(&script, &json!({ "input": input, "rows": rows })),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("the script's worker failed: {e}"))?
}

/// Dry-run every command example of `ext` (`check_extension`): run the
/// script on the example's input (and its `input` query's rows), and
/// check what it composes against the registry — each command exists,
/// its input fits, and the names are `expect_commands`, in order.
pub async fn check_examples(
    layer: &crate::sql_gateway::SqlGateway,
    ext: &mut Extension,
    commands: Option<CommandSchemas<'_>>,
) {
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
    let rows = match &cmd.input {
        Some(sql) => {
            let result = layer
                .run(
                    oxplow_db::SqlQuery::new(sql)
                        .named(input_params(&ex.input))
                        .limit(Some(INPUT_ROW_CAP)),
                )
                .await
                .map_err(|e| {
                    format!(
                        "`input`: {}",
                        e.to_string().replacen("invalid value: ", "", 1)
                    )
                })?;
            rows_json(&result)
        }
        None => Vec::new(),
    };
    let out = run_script(cmd.script.clone(), ex.input.clone(), rows).await?;
    let (calls, _) = composed(out)?;
    for call in &calls {
        let Some(schema) = schema_of(&call.name) else {
            return Err(format!("no command `{}`", call.name));
        };
        InputValidator::compile(&schema)
            .map_err(|e| e.to_string())
            .and_then(|v| v.check(&call.input).map_err(|e| e.to_string()))
            .map_err(|e| format!("the input doesn't fit `{}`: {e}", call.name))?;
    }
    let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
    if names != ex.expect_commands {
        return Err(format!(
            "composed [{}] but `expect_commands` is [{}]",
            names.join(", "),
            ex.expect_commands.join(", ")
        ));
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
    const HANDLER: &str = "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.transition\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"to\": \"done\"}}]}\n";

    const GOOD: &str = "  - name: finish_review
    summary: Mark the task done.
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } }, additionalProperties: false }
    entry: handlers/finish_review.star
    examples:
      - { name: happy, input: { ref: \"work_item:oxplow:tsk1\" }, expect_commands: [work_item.transition] }
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
            c.name, "my_review.finish_review",
            "the namespace is the name, `-` → `_`"
        );
        assert_eq!(c.summary, "Mark the task done.");
        assert_eq!(c.script, HANDLER);
        assert_eq!(c.input_schema["required"], serde_json::json!(["ref"]));
        assert_eq!(c.input, None);
        assert_eq!(c.confirm, Confirm::Never);
        assert_eq!(c.effect, CommandEffect::Write);
        assert_eq!(c.invokers, Invokers::ALL);
        assert_eq!(c.examples.len(), 1);
        assert_eq!(c.examples[0].expect_commands, vec!["work_item.transition"]);
        assert_eq!(command_namespace("my-review"), "my_review");
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
            (entry("a", ""), vec![], "entry `handlers/h.star` isn't a file"),
            (
                entry("a", ""),
                vec![("handlers/h.star", "def transform(x:\n")],
                "doesn't parse",
            ),
            (
                entry("a", ""),
                vec![("handlers/h.star", "def other(x):\n    return {}\n")],
                "define `transform`",
            ),
            (
                entry("a", "    effect: read\n"),
                vec![("handlers/h.star", HANDLER)],
                "a command that only reads",
            ),
            (
                entry("a", "    effect: often\n"),
                vec![("handlers/h.star", HANDLER)],
                "effect",
            ),
            (
                entry("a", "    confirm: maybe\n"),
                vec![("handlers/h.star", HANDLER)],
                "confirm",
            ),
            (
                entry("a", "    input: \"DELETE FROM task\"\n"),
                vec![("handlers/h.star", HANDLER)],
                "one read",
            ),
            (
                "  - name: a\n    summary: S.\n    input_schema: { type: nope }\n    entry: handlers/h.star\n".into(),
                vec![("handlers/h.star", HANDLER)],
                "input_schema",
            ),
            (
                format!("{}{}", entry("a", ""), entry("a", "")),
                vec![("handlers/h.star", HANDLER)],
                "declared twice",
            ),
            (
                entry("a", "    entry_point: x\n"),
                vec![("handlers/h.star", HANDLER)],
                "entry_point",
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

    /// A namespace core uses is refused; two enabled extensions that map
    /// to one namespace are both refused; a disabled one doesn't count.
    #[test]
    fn a_namespace_is_one_extensions() {
        let d = tempfile::tempdir().unwrap();
        let files = [("handlers/finish_review.star", HANDLER)];
        write_ext(d.path(), "vcs", GOOD, &files);
        let errs = project(d.path(), "vcs").errors.join("\n");
        assert!(errs.contains("namespace `vcs` is oxplow's"), "{errs}");
        assert!(project(d.path(), "vcs").commands.is_empty());

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

    /// Every core command's namespace is reserved, so no extension can
    /// register beside it.
    #[tokio::test]
    async fn every_core_namespace_is_reserved() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for spec in fx.svc.commands.list(&oxplow_domain::Actor::Human) {
            let ns = spec.name.split('.').next().unwrap();
            assert!(
                RESERVED_COMMAND_NAMESPACES.contains(&ns),
                "`{}`: add `{ns}` to RESERVED_COMMAND_NAMESPACES",
                spec.name
            );
        }
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
        let schema =
            move |name: &str| (name == "work_item.transition").then(|| transition_schema.clone());
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

        // Rows from `input:` reach the script.
        let d = tempfile::tempdir().unwrap();
        write_ext(
            d.path(),
            "x",
            &GOOD.replace(
                "    entry: handlers/finish_review.star\n",
                "    entry: handlers/finish_review.star\n    input: \"SELECT :ref AS r\"\n",
            ),
            &[(
                "handlers/finish_review.star",
                "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.transition\", \"input\": {\"ref\": x[\"rows\"][0][\"r\"], \"to\": \"done\"}}]}\n",
            )],
        );
        let v = validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);

        for (handler, says) in [
            (
                "def transform(x):\n    return {\"commands\": [{\"name\": \"no.such\", \"input\": {}}]}\n",
                "example `happy`: no command `no.such`",
            ),
            (
                "def transform(x):\n    return {\"commands\": [{\"name\": \"work_item.transition\", \"input\": {\"ref\": 1}}]}\n",
                "example `happy`: the input doesn't fit `work_item.transition`",
            ),
            (
                "def transform(x):\n    return {\"commands\": []}\n",
                "example `happy`: composed [] but `expect_commands` is [work_item.transition]",
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
}
