//! The plugin SDK (`.context/extensions.md` "The SDK";
//! `.context/target-architecture.md` §10.5): scaffold, check and test an
//! extension, and the provider conformance kit
//! ([`conformance`], [`plugin_test`]).
//!
//! One library behind three callers — the `oxplow plugin new|check|test`
//! CLI in the Tauri binary, the RPC/MCP `validate_extension`, and
//! `save_lens` — so every message reads the same (`file:line: what — fix`)
//! wherever the author meets it. For an AI author that consistency is the
//! feature: the skill says "run `check` after every edit", and the error
//! it gets back names the file and line and says what to change.

pub mod answerability;
pub mod conformance;
pub mod plugin_test;
mod throwaway;

use std::path::{Path, PathBuf};

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_app::extensions::{self, Extension, EXTENSIONS_DIR};
use oxplow_app::sql_gateway::SqlGateway;
use oxplow_domain::DomainError;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("{0}")]
    Invalid(String),
    #[error("no extension `{0}` under {EXTENSIONS_DIR}/")]
    NotFound(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Domain(#[from] DomainError),
}

/// What `plugin new` can make. Each checks clean as written and passes
/// `plugin test` (P7.C6, `tests/just_works.rs`) — a provider once a real
/// program stands behind its stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One starter lens (open tasks in the viewer's stream) with a row
    /// action that starts one.
    Lens,
    /// A v2 manifest only.
    Extension,
    /// A work-items provider: its declarations, a stub program to replace,
    /// and the fixtures `plugin test` runs.
    Provider,
    /// A derived (Starlark) collector of an entity, a model over it and a
    /// lens over the model.
    Collector,
    /// A command whose script composes core commands, on a work item's
    /// Commands menu.
    Command,
    /// An effect (experimental, so private): a script reacting to a logged
    /// event by composing commands, run once a person approves it.
    Effect,
    /// A custom component (experimental, so private): a web bundle a
    /// `viz: custom` lens renders in a sandboxed frame, talking to oxplow
    /// through the served client library.
    Component,
}

impl Kind {
    pub const NAMES: &'static str =
        "`lens`, `extension`, `provider`, `collector`, `command`, `effect` or `component`";

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "lens" => Some(Kind::Lens),
            "extension" => Some(Kind::Extension),
            "provider" => Some(Kind::Provider),
            "collector" => Some(Kind::Collector),
            "command" => Some(Kind::Command),
            "effect" => Some(Kind::Effect),
            "component" => Some(Kind::Component),
            _ => None,
        }
    }
}

/// What `scaffold` wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scaffolded {
    pub name: String,
    /// The extension folder, relative to the root.
    pub dir: String,
    /// Every file written, relative to the root.
    pub files: Vec<String>,
}

/// Create `oxplow/extensions/<name>/` with a v2 manifest carrying an
/// `intent` (its `origin` = the thread/effort ref that asked for it), one
/// example and a matching fixture, and — for a lens — one starter lens the
/// example names. Refuses an existing folder, a bad name, or a non-ref
/// origin; `check` passes on what it writes.
pub fn scaffold(
    root: &Path,
    kind: Kind,
    name: &str,
    origin: Option<&str>,
) -> Result<Scaffolded, SdkError> {
    if !extensions::is_valid_name(name) {
        return Err(SdkError::Invalid(format!(
            "`{name}` must start with a letter and be lowercase letters, digits and single dashes (e.g. `review-notes`)"
        )));
    }
    if let Some(o) = origin {
        oxplow_domain::refs::grammar::CanonicalRef::parse(o).map_err(|e| {
            SdkError::Invalid(format!(
                "--origin `{o}` is not a canonical ref (`effort:eff42`, `thread:thr3`): {}",
                e.reason()
            ))
        })?;
    }
    let rel_dir = format!("{EXTENSIONS_DIR}/{name}");
    let dir = root.join(&rel_dir);
    if dir.exists() {
        return Err(SdkError::Invalid(format!(
            "{rel_dir} already exists; pick another name or edit it in place"
        )));
    }
    let mut files = Vec::new();
    let mut write = |rel: &str, body: String| -> Result<(), SdkError> {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
        files.push(rel.to_string());
        Ok(())
    };
    let ns = oxplow_app::extension_commands::command_namespace(name);
    // The intent example, and its fixture (what `plugin test` runs).
    let (example_input, example_expect, fixture_expect) = match kind {
        Kind::Lens | Kind::Component => (
            format!("{{ lens: {name}, params: {{ stream_id: 1 }} }}"),
            "one row per open task in the stream, newest first",
            Some("{ rows: $any }".to_string()),
        ),
        Kind::Extension => ("{}".to_string(), "TODO: what a run should show", None),
        Kind::Provider => (
            "{ command: create, input: { title: First } }".to_string(),
            "the new item's ref",
            Some("{ ref: $any }".to_string()),
        ),
        Kind::Collector => (
            "{ collector: items, rows: [{ id: 1, title: First, status: ready }, { id: 2, title: Second, status: done }] }"
                .to_string(),
            "one item per task that isn't done",
            Some("{ entities: { item: 1 } }".to_string()),
        ),
        Kind::Command => (
            format!("{{ command: {ns}.note, input: {{ ref: \"work_item:oxplow:tsk1\" }} }}"),
            "a comment on the work item",
            Some("{ commands: [work_item.comment] }".to_string()),
        ),
        Kind::Effect => (
            "{ effect: on-done, event: { type: work_item.transitioned, payload: { work_item: \"work_item:oxplow:tsk1\", from: in_progress, to: done } } }"
                .to_string(),
            "a comment on the finished work item",
            Some("{ commands: [work_item.comment] }".to_string()),
        ),
    };
    let provider_id = name.replace('-', "_");
    let mut manifest = extensions::scaffold_manifest(&extensions::ManifestScaffold {
        name,
        description: "TODO: one line on what this extension shows or does",
        purpose: "TODO: the question this answers, or the job it does",
        origin,
        example_name: "basic",
        example_input: &example_input,
        example_expect,
        shared: false,
    });
    match kind {
        Kind::Provider => manifest.push_str(&format!(
            "providers:\n  - id: {provider_id}\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n"
        )),
        Kind::Collector => manifest.push_str(&format!(
            "collectors:\n\
             \x20 # Runs its script over its `input` rows when synced (`trigger: manual`;\n\
             \x20 # `{{ every: 15m }}` or `{{ on: [snapshot.taken] }}` run it by themselves).\n\
             \x20 - id: items\n\
             \x20   runtime: starlark\n\
             \x20   entry: collectors/items.star\n\
             \x20   input: \"SELECT id, title, status FROM v_task\"\n\
             \x20   entities:\n\
             \x20     - {{ name: item, key: id, columns: {{ id: int, title: text }} }}\n\
             models:\n\
             \x20 # v_{ns}_open_items: SQL over the entity (`ref('item')`), checked at load.\n\
             \x20 - name: open_items\n\
             \x20   version: 1\n\
             \x20   description: \"TODO: what these rows are.\"\n\
             \x20   columns:\n\
             \x20     - {{ name: id, type: INTEGER, doc: \"The task.\" }}\n\
             \x20     - {{ name: title, type: TEXT, doc: \"Its title.\" }}\n"
        )),
        Kind::Command => manifest.push_str(&format!(
            "commands:\n\
             \x20 # Registered as {ns}.note; its script composes core commands, run as the\n\
             \x20 # caller in one transaction.\n\
             \x20 - name: note\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item.\"\n\
             \x20   input_schema:\n\
             \x20     type: object\n\
             \x20     required: [ref]\n\
             \x20     properties:\n\
             \x20       ref: {{ type: string, description: \"The work item (work_item:<provider>:<id>).\" }}\n\
             \x20     additionalProperties: false\n\
             \x20   entry: handlers/note.star\n\
             \x20   examples:\n\
             \x20     - {{ name: happy, input: {{ ref: \"work_item:oxplow:tsk1\" }}, expect_commands: [work_item.comment] }}\n\
             ui:\n\
             \x20 commands:\n\
             \x20   # On a work item's page (Commands) and a row's right-click.\n\
             \x20   - {{ command: {ns}.note, label: Add Note, about: work_item }}\n"
        )),
        Kind::Effect => manifest.push_str(
            "effects:\n\
             \x20 # Reacts to a logged event by composing commands, run with an agent's\n\
             \x20 # rights only once a person approves it (Settings → Data), and only on\n\
             \x20 # events logged after; a command that asks becomes a proposal.\n\
             \x20 - id: on-done\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item when it's done.\"\n\
             \x20   on: [work_item.transitioned]\n\
             \x20   where: { to: done }\n\
             \x20   entry: effects/on-done.star\n",
        ),
        Kind::Component => manifest.push_str(&format!(
            "custom_components:\n\
             \x20 # A web bundle (components/{name}/) a `viz: custom` lens renders in a\n\
             \x20 # sandboxed frame: scripts only, no network, no storage. Beyond its lens's\n\
             \x20 # own rows it may query the lenses in `assets` and run the commands in\n\
             \x20 # `commands` — as the person looking at it — and nothing else.\n\
             \x20 - id: {name}\n\
             \x20   assets: []\n\
             \x20   commands: []\n"
        )),
        Kind::Lens | Kind::Extension => {}
    }
    write(&format!("{rel_dir}/extension.yaml"), manifest)?;
    if let Some(expect) = fixture_expect {
        write(
            &format!("{rel_dir}/fixtures/basic.yaml"),
            format!(
                "# The acceptance example from extension.yaml as a fixture for `oxplow plugin test`\n\
                 # (input in, expected output out). Keep the two in step.\n\
                 name: basic\ninput: {example_input}\nexpect: {expect}\n"
            ),
        )?;
    }
    match kind {
        Kind::Provider => {
            write(
                &format!("{rel_dir}/provider.json"),
                serde_json::to_string_pretty(&provider_declarations(name))
                    .expect("declarations serialize")
                    + "\n",
            )?;
            write(
                &format!("{rel_dir}/bin/provider"),
                "#!/bin/sh\n\
                 # TODO: the provider program. It speaks the provider protocol (JSON-RPC 2.0,\n\
                 # one message per line) on stdin/stdout and answers what provider.json\n\
                 # declares; see the oxplow-extension skill.\n\
                 echo 'provider: not implemented yet' >&2\n\
                 exit 1\n"
                    .to_string(),
            )?;
            make_executable(&root.join(&rel_dir).join("bin/provider"))?;
            write(
                &format!("{rel_dir}/fixtures/provider-{provider_id}.yaml"),
                "# The instance config `oxplow plugin test` checks the provider with.\nconfig: {}\n"
                    .to_string(),
            )?;
        }
        Kind::Lens => write(
            &format!("{rel_dir}/lenses/{name}.yaml"),
            format!(
                "title: {title}\n\
                 description: \"TODO: what this lens answers.\"\n\
                 params:\n\
                 \x20 # Filled in with the viewer's stream unless a value is given.\n\
                 \x20 - {{ name: stream_id, label: Stream }}\n\
                 query: |\n\
                 \x20 SELECT id, 'work_item:oxplow:tsk' || id AS ref, title, status, updated_at\n\
                 \x20 FROM v_task\n\
                 \x20 WHERE stream_id = :stream_id AND status IN ('ready', 'in_progress', 'blocked')\n\
                 \x20 ORDER BY updated_at DESC\n\
                 viz: table\n\
                 columns:\n\
                 \x20 - {{ key: title, label: Task, link: {{ kind: task, from: id }} }}\n\
                 \x20 - {{ key: status }}\n\
                 actions:\n\
                 \x20 # In each row's right-click menu: a command run as the person, the\n\
                 \x20 # row's values bound in (`{{{{row.<column>}}}}`).\n\
                 \x20 - {{ id: start, label: Start, command: work_item.transition, input: {{ ref: \"{{{{row.ref}}}}\", to: in_progress }}, row: true }}\n\
                 empty: No open tasks in this stream.\n\
                 launcher: {{ category: Work }}\n",
                title = title_case(name)
            ),
        )?,
        Kind::Collector => {
            write(
                &format!("{rel_dir}/collectors/items.star"),
                "# Gets {\"rows\": [...]} (its `input` query's) and returns the entities it\n\
                 # declares. Sandboxed: no files, network or clock.\n\
                 def transform(x):\n\
                 \x20   return {\"entities\": {\"item\": [\n\
                 \x20       {\"id\": r[\"id\"], \"title\": r[\"title\"]}\n\
                 \x20       for r in x[\"rows\"]\n\
                 \x20       if r[\"status\"] != \"done\"\n\
                 \x20   ]}}\n"
                    .to_string(),
            )?;
            write(
                &format!("{rel_dir}/models/open_items.sql"),
                "SELECT id, title FROM ref('item')\n".to_string(),
            )?;
            write(
                &format!("{rel_dir}/lenses/{name}.yaml"),
                format!(
                    "title: {title}\n\
                     description: \"TODO: what this lens answers.\"\n\
                     query: SELECT id, title FROM v_{ns}_open_items ORDER BY id\n\
                     viz: table\n\
                     columns:\n\
                     \x20 - {{ key: title, label: Item, link: {{ kind: task, from: id }} }}\n\
                     empty: Nothing collected yet — sync the collector (Settings → Data).\n\
                     launcher: {{ category: Work }}\n",
                    title = title_case(name)
                ),
            )?;
        }
        Kind::Command => write(
            &format!("{rel_dir}/handlers/note.star"),
            "# Gets {\"input\": {...}, \"rows\": [...]} and returns the core commands to run\n\
             # as the caller, in one transaction — or {\"refuse\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"work_item.comment\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?,
        Kind::Effect => write(
            &format!("{rel_dir}/effects/on-done.star"),
            "# Gets {\"event\": {type, payload, subject, ...}, \"rows\": [...]} and returns the\n\
             # commands to run — or {\"skip\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   ref = x[\"event\"][\"payload\"][\"work_item\"]\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"work_item.comment\", \"input\": {\"ref\": ref, \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?,
        Kind::Component => {
            write(
                &format!("{rel_dir}/lenses/{name}.yaml"),
                format!(
                    "title: {title}\n\
                     description: \"TODO: what this view answers.\"\n\
                     params:\n\
                     \x20 # Filled in with the viewer's stream unless a value is given.\n\
                     \x20 - {{ name: stream_id, label: Stream }}\n\
                     # The rows the component gets (`component.run`), and what an agent reads:\n\
                     # agents always see the table, never the frame.\n\
                     query: |\n\
                     \x20 SELECT id, title, status\n\
                     \x20 FROM v_task\n\
                     \x20 WHERE stream_id = :stream_id AND status IN ('ready', 'in_progress', 'blocked')\n\
                     \x20 ORDER BY updated_at DESC\n\
                     viz: custom\n\
                     custom: {{ component: {name} }}\n\
                     launcher: {{ category: Work }}\n",
                    title = title_case(name)
                ),
            )?;
            write(
                &format!("{rel_dir}/components/{name}/index.html"),
                "<!doctype html>\n\
                 <meta charset=\"utf-8\">\n\
                 <div id=\"out\"></div>\n\
                 <!-- oxplow's client library (it defines `oxplow`), then this bundle's own\n\
                 \x20    script. Both are plain scripts: a sandboxed frame can't load modules. -->\n\
                 <script src=\"/component-lib/oxplow-component.js\"></script>\n\
                 <script src=\"app.js\"></script>\n"
                    .to_string(),
            )?;
            write(
                &format!("{rel_dir}/components/{name}/app.js"),
                "// Runs in a sandboxed frame. `oxplow.connect()` resolves once oxplow has\n\
                 // handed over the lens's rows:\n\
                 //   component.run                     the lens's latest run ({ result: { columns, rows } })\n\
                 //   component.onUpdate(fn)            the lens re-ran\n\
                 //   component.query(asset, params)    a lens listed in `assets`\n\
                 //   component.invoke(command, input)  a command listed in `commands`\n\
                 //   component.navigate(ref)           open one of oxplow's pages\n\
                 oxplow.connect().then((component) => {\n\
                 \x20 component.applyKitCss();\n\
                 \x20 const out = document.getElementById(\"out\");\n\
                 \x20 const render = (run) => {\n\
                 \x20   out.textContent = run.result.rows.length + \" open tasks\";\n\
                 \x20 };\n\
                 \x20 render(component.run);\n\
                 \x20 component.onUpdate(render);\n\
                 });\n"
                    .to_string(),
            )?;
        }
        Kind::Extension => {}
    }
    Ok(Scaffolded {
        name: name.to_string(),
        dir: rel_dir,
        files,
    })
}

/// A work-items provider's starting declarations: create, update and
/// transition (`record` effect, no confirmation), the core
/// `work_item.recorded@1` event and an empty config schema.
fn provider_declarations(name: &str) -> oxplow_provider_protocol::model::InitializeResult {
    use oxplow_domain::events::schema::{schema_for, EventType, WorkItemRecorded};
    use oxplow_provider_protocol::model::*;
    let command = |name: &str, summary: &str, input: serde_json::Value| CommandDecl {
        name: name.into(),
        summary: summary.into(),
        input_schema: input,
        confirm: "never".into(),
        effect: "record".into(),
        undoable: false,
    };
    let str_prop = serde_json::json!({ "type": "string" });
    let state = serde_json::json!({ "type": "string",
                                    "enum": ["todo", "in_progress", "blocked", "done", "canceled"] });
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: name.into(),
            version: "0.1.0".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "work_items".into(),
            features: serde_json::json!({
                "hierarchy": false,
                "comments": false,
                "links": false,
                "delete": false,
                "in_progress_opens_effort": false,
            }),
        }],
        // The work-items contract's verbs (`.context/work-items.md`); declare
        // `link` / `comment` / `delete` with their features.
        commands: vec![
            command(
                "create",
                "Create a work item.",
                serde_json::json!({ "type": "object", "required": ["title"], "additionalProperties": false,
                                    "properties": { "title": str_prop, "body": str_prop,
                                                    "state": state, "native_state": str_prop } }),
            ),
            command(
                "update",
                "Change a work item's title, body or state.",
                serde_json::json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                                    "properties": { "ref": str_prop, "title": str_prop, "body": str_prop,
                                                    "state": state, "native_state": str_prop } }),
            ),
            command(
                "transition",
                "Move a work item to a canonical state, optionally naming a native one.",
                serde_json::json!({ "type": "object", "required": ["ref", "to"], "additionalProperties": false,
                                    "properties": { "ref": str_prop, "to": state, "native_state": str_prop } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: WorkItemRecorded::TYPE.into(),
            v: WorkItemRecorded::V,
            schema: schema_for::<WorkItemRecorded>(),
        }],
        collectors: Vec::new(),
        config_schema: serde_json::json!({ "type": "object" }),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn title_case(name: &str) -> String {
    name.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What `check` found.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub name: String,
    /// No errors (warnings don't fail a check).
    pub ok: bool,
    /// `file:line: what — fix` lines.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// What its SQL was dry-run against.
    pub dry_run: DryRun,
    pub extension: Extension,
    /// With `--effects` (P8.C6): what going from `against` to the working
    /// tree changes; `null` without it.
    pub effects: Option<oxplow_app::extension_effects::EffectReport>,
    /// The revision `effects` compares the working tree with.
    pub against: Option<String>,
}

/// The database a check dry-runs an extension's SQL on. It always has one
/// (P7.C6): what it declares but hasn't published stands in either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DryRun {
    /// The project's own (its synced data, read-only).
    Project,
    /// A fresh, empty one: no usable project database.
    EmptyDatabase,
}

/// Load `name` under `root` and report everything wrong with it: the
/// manifest's shape and lifecycle, every cross-reference, every lens's
/// shape, and a dry run of its models, commands, lenses and advisories
/// against the semantic layer — `layer` (the project's), else a fresh
/// in-memory database. This is what `validate_extension` returns
/// and what `oxplow plugin check` prints. `commands` (the running app's
/// registry) checks launcher command entries; without it, a throwaway
/// oxplow's does. With `against` (a git revision), the report also says
/// what going from it to the working tree changes ([`CheckReport::effects`]).
pub async fn check(
    root: &Path,
    name: &str,
    catalog: &ExtensionCatalog,
    layer: Option<&SqlGateway>,
    commands: Option<extensions::CommandSchemas<'_>>,
    against: Option<&str>,
) -> Result<CheckReport, SdkError> {
    // No running oxplow to ask which commands exist: a throwaway one over
    // a copy of the project's extensions knows (core's and theirs).
    let host = match commands {
        Some(_) => None,
        None => Some(
            throwaway::Host::start(root)
                .await
                .map_err(SdkError::Invalid)?,
        ),
    };
    let commands = commands.or_else(|| {
        host.as_ref()
            .map(|h| h.svc.commands.as_ref() as extensions::CommandSchemas<'_>)
    });
    let empty;
    let (layer, dry_run) = match layer {
        Some(layer) => (layer, DryRun::Project),
        None => {
            empty = SqlGateway::new(oxplow_db::Database::in_memory());
            (&empty, DryRun::EmptyDatabase)
        }
    };
    let extension = extensions::validate_extension(layer, catalog, root, name, commands)
        .await
        .map_err(|e| match e {
            DomainError::NotFound => SdkError::NotFound(name.to_string()),
            other => SdkError::Domain(other),
        })?;
    let effects = match (against, commands) {
        (Some(rev), Some(commands)) => {
            Some(effects_against(root, name, catalog, layer, rev, commands).await?)
        }
        _ => None,
    };
    Ok(CheckReport {
        name: name.to_string(),
        ok: extension.errors.is_empty(),
        errors: extension.errors.clone(),
        warnings: extension.warnings.clone(),
        dry_run,
        extension,
        effects,
        against: against.map(str::to_string),
    })
}

/// What going from git revision `against` (`HEAD`, a branch, a sha) to the
/// working tree changes for extension `name` (P8.C6) — the review an
/// install or an effort shows, on the CLI: lenses, models and their rows,
/// collectors and their outputs, providers, config. It reads through
/// `layer` and writes nothing: each side's models are temp views.
async fn effects_against(
    root: &Path,
    name: &str,
    catalog: &ExtensionCatalog,
    layer: &SqlGateway,
    against: &str,
    commands: extensions::CommandSchemas<'_>,
) -> Result<oxplow_app::extension_effects::EffectReport, SdkError> {
    use oxplow_app::extensions::{effects_between, extension_tree_at, ReviewSide};
    // A revision's files through the VCS; nothing here reads a snapshot.
    let trees = oxplow_app::trees::Trees::new(
        std::sync::Arc::new(oxplow_app::vcs::GitProvider),
        std::sync::Arc::new(oxplow_db::SqliteSnapshotStore::new(
            oxplow_db::Database::in_memory(),
        )),
        oxplow_app::blob_store::BlobStore::new(std::env::temp_dir()),
        // The project's workspace filter decides what a revision holds.
        std::sync::Arc::new(std::sync::RwLock::new(
            oxplow_config::load_project_config(root)
                .map_err(|e| SdkError::Invalid(format!(".oxplow/project.yaml: {e}")))?,
        )),
    );
    let before = extension_tree_at(
        &trees,
        root,
        &oxplow_domain::vcs::Revision::git(against),
        name,
    )
    .await
    .map_err(SdkError::Domain)?;
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    let read_before = |file: &str| before.as_ref().and_then(|t| t.file(file));
    let read_after = |file: &str| extensions::read_extension_file(root, name, file);
    let after = catalog.named(root, name).map_err(|e| match e {
        DomainError::NotFound => SdkError::NotFound(name.to_string()),
        other => SdkError::Domain(other),
    })?;
    let mut after = ReviewSide {
        extension: after,
        read: &read_after,
    };
    Ok(effects_between(
        layer,
        catalog,
        root,
        before.as_ref().map(|t| ReviewSide {
            extension: t.load(name, &rel),
            read: &read_before,
        }),
        &mut after,
        commands,
    )
    .await)
}

/// `check` for a folder path (`oxplow/extensions/<name>` or an absolute
/// path to it) or a bare name: the name is its last component.
pub fn name_of(path_or_name: &str) -> &str {
    Path::new(path_or_name.trim_end_matches('/'))
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path_or_name)
}

/// How `render_findings` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One finding per line: `error: <file:line: what — fix>`.
    Text,
    Json,
}

/// The report as text or JSON. Text is what an agent reads back from the
/// CLI; JSON is for tooling.
pub fn render_findings(report: &CheckReport, format: Format) -> String {
    match format {
        Format::Json => serde_json::to_string_pretty(report).expect("report serializes"),
        Format::Text => {
            let mut out = String::new();
            for e in &report.errors {
                out.push_str("error: ");
                out.push_str(e);
                out.push('\n');
            }
            for w in &report.warnings {
                out.push_str("warning: ");
                out.push_str(w);
                out.push('\n');
            }
            let sql = match report.dry_run {
                DryRun::Project => "its SQL dry-run against the project's database",
                DryRun::EmptyDatabase => {
                    "its SQL dry-run on an empty database (no usable project database)"
                }
            };
            out.push_str(&format!(
                "{}: {} error{}, {} warning{}; {sql}\n",
                report.name,
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" },
                report.warnings.len(),
                if report.warnings.len() == 1 { "" } else { "s" },
            ));
            if let Some(effects) = &report.effects {
                out.push_str(&format!(
                    "effects against {}:\n",
                    report.against.as_deref().unwrap_or("HEAD")
                ));
                if effects.lines.is_empty() {
                    out.push_str("  (nothing changes)\n");
                }
                for line in &effects.lines {
                    out.push_str(&format!("  {line}\n"));
                }
            }
            out
        }
    }
}

/// The project's local database, when the folder is an oxplow project
/// that has been opened: what `check` dry-runs lens SQL against.
pub fn project_database(root: &Path) -> Option<PathBuf> {
    let path = root.join(".oxplow").join("local.sqlite");
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[tokio::test]
    async fn new_lens_then_check_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let made = scaffold(dir.path(), Kind::Lens, "demo", Some("effort:eff42")).unwrap();
        assert_eq!(made.dir, "oxplow/extensions/demo");
        assert_eq!(
            made.files,
            vec![
                "oxplow/extensions/demo/extension.yaml",
                "oxplow/extensions/demo/fixtures/basic.yaml",
                "oxplow/extensions/demo/lenses/demo.yaml",
            ]
        );
        let report = check(
            dir.path(),
            "demo",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(report.ok, "{}", render_findings(&report, Format::Text));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        // No project database: the SQL still dry-runs, on an empty one.
        assert_eq!(report.dry_run, DryRun::EmptyDatabase);
        assert_eq!(report.extension.manifest_version, 2);
        assert_eq!(
            report.extension.intent.as_ref().unwrap().origin.as_deref(),
            Some("effort:eff42")
        );
        assert_eq!(report.extension.lenses.len(), 1);
        assert_eq!(report.extension.lenses[0].title, "Demo");
        let text = render_findings(&report, Format::Text);
        assert!(text.contains("demo: 0 errors, 0 warnings"), "{text}");
        assert!(text.contains("dry-run on an empty database"), "{text}");
        let lens = dir.path().join("oxplow/extensions/demo/lenses/demo.yaml");
        let body = std::fs::read_to_string(&lens).unwrap();
        std::fs::write(&lens, body.replace("FROM v_task", "FROM v_no_such_view")).unwrap();
        let report = check(
            dir.path(),
            "demo",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            report.errors.join("\n").contains("v_no_such_view"),
            "{:?}",
            report.errors
        );
        std::fs::write(&lens, body).unwrap();
        // A second scaffold refuses to overwrite; bad names and origins refuse too.
        assert!(scaffold(dir.path(), Kind::Lens, "demo", None).is_err());
        assert!(scaffold(dir.path(), Kind::Lens, "Bad Name", None).is_err());
        assert!(scaffold(dir.path(), Kind::Lens, "other", Some("nope")).is_err());
        let ext_only = scaffold(dir.path(), Kind::Extension, "bare", None).unwrap();
        assert_eq!(
            ext_only.files,
            vec!["oxplow/extensions/bare/extension.yaml"]
        );
        assert!(
            check(
                dir.path(),
                "bare",
                &ExtensionCatalog::new(),
                None,
                None,
                None
            )
            .await
            .unwrap()
            .ok
        );
    }

    #[tokio::test]
    async fn check_reports_lifecycle_errors_with_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/team/extension.yaml",
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nref_kinds:\n  - kind: ticket\n",
        );
        let report = check(
            dir.path(),
            "team",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(!report.ok);
        let text = render_findings(&report, Format::Text);
        assert!(
            text.contains(
                "error: oxplow/extensions/team/extension.yaml:8: `ref_kinds` is experimental"
            ),
            "{text}"
        );
        assert!(text.contains("team: 1 error, 0 warnings"), "{text}");
        let json: serde_json::Value =
            serde_json::from_str(&render_findings(&report, Format::Json)).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["errors"].as_array().unwrap().len(), 1);
        assert!(matches!(
            check(
                dir.path(),
                "nope",
                &ExtensionCatalog::new(),
                None,
                None,
                None
            )
            .await,
            Err(SdkError::NotFound(_))
        ));
    }

    /// P9.D1: an effect reacting to another extension's event type checks
    /// clean with a warning — a check sees one extension, and the type's
    /// owner may simply not be in this repo.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reaction_to_another_extensions_type_is_a_warning_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/beta/extension.yaml",
            "manifest: 2\nname: beta\nsharing: private\nengine: \">=0.1\"\nintent:\n  purpose: Follows merges.\n  examples: [{ name: a }]\neffects:\n  - id: note\n    summary: Note a merge.\n    on: [acme_pr.merged]\n    entry: note.star\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/beta/note.star",
            "def transform(x):\n    return {\"skip\": \"nothing to do\"}\n",
        );
        let report = check(
            dir.path(),
            "beta",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let reaction = |lines: &[String]| {
            lines
                .iter()
                .filter(|l| l.contains("`acme_pr.merged`"))
                .count()
        };
        assert_eq!(reaction(&report.errors), 0, "{:?}", report.errors);
        assert_eq!(reaction(&report.warnings), 1, "{:?}", report.warnings);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("extension.yaml:9")
                    && w.contains("another extension's event type")),
            "{:?}",
            report.warnings
        );
    }

    /// Launcher command entries are always checked: against the registry
    /// when one is given, else a throwaway oxplow's (P7.C6).
    #[tokio::test(flavor = "multi_thread")]
    async fn launcher_commands_are_checked_without_a_database() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: x\n  examples: [{ name: a }]\nlauncher:\n  - { label: New Bug, category: Work, target: { command: work_item.create, input: { title: 7 } } }\n",
        );
        let throwaway = check(
            dir.path(),
            "acme",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            throwaway
                .errors
                .iter()
                .any(|e| e.contains("the input doesn't fit")),
            "{:?}",
            throwaway.errors
        );
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "title": { "type": "string" } },
            "required": ["title"]
        });
        let schemas = move |name: &str| (name == "work_item.create").then(|| schema.clone());
        let checked = check(
            dir.path(),
            "acme",
            &ExtensionCatalog::new(),
            None,
            Some(&schemas),
            None,
        )
        .await
        .unwrap();
        assert!(!checked.ok);
        assert!(
            checked
                .errors
                .iter()
                .any(|e| e.contains("the input doesn't fit")),
            "{:?}",
            checked.errors
        );
    }

    #[test]
    fn name_of_reads_a_folder_or_a_name() {
        assert_eq!(name_of("oxplow/extensions/old/"), "old");
        assert_eq!(name_of("old"), "old");
    }
}
