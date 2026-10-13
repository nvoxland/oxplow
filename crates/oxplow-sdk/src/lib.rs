//! The extension SDK (`.context/extensions.md` "The SDK"): scaffold, check and test an
//! extension, and the provider conformance kit
//! ([`conformance`], [`extension_test`]).
//!
//! One library behind three callers — the `oxplow extension new|check|test`
//! CLI in the Tauri binary, the RPC/MCP `validate_extension`, and
//! `save_lens` — so every message reads the same (`file:line: what — fix`)
//! wherever the author meets it. For an AI author that consistency is the
//! feature: the skill says "run `check` after every edit", and the error
//! it gets back names the file and line and says what to change.

pub mod answerability;
pub mod conformance;
pub mod extension_test;
mod throwaway;

use std::path::{Path, PathBuf};

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_app::extensions::{self, Extension, EXTENSIONS_DIR};
pub use oxplow_app::metric_scaffold::MetricStarter;
use oxplow_app::sql_gateway::SqlGateway;
use oxplow_domain::contribution::ContributionKind;
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

/// What `extension new` can make. Each checks clean as written and passes
/// `extension test` (P7.C6, `tests/just_works.rs`) — a provider once a real
/// program stands behind its stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One starter lens (open tasks in the viewer's stream) with a row
    /// action that starts one.
    Lens,
    /// A v2 manifest only.
    Extension,
    /// A provider of `capability` (one a process may implement): its
    /// declarations from the capability's contract, a stub program to
    /// replace, and the fixtures `extension test` runs.
    Provider { capability: &'static str },
    /// A derived (Starlark) collector of an entity, a model over it and a
    /// lens over the model.
    Collector,
    /// A code metric: a per-path measure, a Starlark fact collector over the
    /// snapshot's files, the dimension its facts carry, the headline metric
    /// and a lens whose rows link to the file and line.
    Metric,
    /// A command whose script composes core commands, on a work item's
    /// Commands menu.
    Command,
    /// An effect: a script reacting to a logged
    /// event by composing commands, run once a person approves it.
    Effect,
    /// A custom component: a web bundle a
    /// `viz: custom` lens renders in a sandboxed frame, talking to oxplow
    /// through the served client library.
    Component,
    /// An effort policy written as a script: an `implementations:` entry
    /// whose script opens an item's effort when it starts on a thread and
    /// closes it when it ends, run once a person approves it.
    Policy,
    /// Guidance for the coding agent: a query run at a moment, whose rows
    /// reach the agent.
    Advisory,
    /// A skill the coding agent loads when it's relevant (`SKILL.md`).
    Skill,
    /// A full page showing one lens, in the launcher.
    Page,
    /// A left-nav panel showing one lens compact.
    Panel,
}

impl Kind {
    pub const NAMES: &'static str = "`lens`, `extension`, `provider`, `collector`, `metric`, `command`, `effect`, `component`, `policy`, `advisory`, `skill`, `page` or `panel`";

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "lens" => Some(Kind::Lens),
            "extension" => Some(Kind::Extension),
            "provider" => Some(Kind::Provider {
                capability: "work_items",
            }),
            "collector" => Some(Kind::Collector),
            "metric" => Some(Kind::Metric),
            "command" => Some(Kind::Command),
            "effect" => Some(Kind::Effect),
            "component" => Some(Kind::Component),
            "policy" => Some(Kind::Policy),
            "advisory" => Some(Kind::Advisory),
            "skill" => Some(Kind::Skill),
            "page" => Some(Kind::Page),
            "panel" => Some(Kind::Panel),
            _ => None,
        }
    }

    /// A provider of `capability`; refused unless a process may implement
    /// it (its spec carries a provider contract).
    pub fn provider(capability: &str) -> Result<Kind, SdkError> {
        match oxplow_domain::capability::spec(capability).filter(|c| c.provider.is_some()) {
            Some(spec) => Ok(Kind::Provider {
                capability: spec.id,
            }),
            None => {
                let may: Vec<String> = oxplow_domain::capability::CAPABILITIES
                    .iter()
                    .filter(|c| c.provider.is_some())
                    .map(|c| format!("`{}`", c.id))
                    .collect();
                Err(SdkError::Invalid(format!(
                    "`{capability}` isn't a capability a provider can implement; one of {}",
                    may.join(", ")
                )))
            }
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
    scaffold_with(root, kind, name, origin, &MetricStarter::default())
}

/// [`scaffold`], with the language and files a `Kind::Metric` scans
/// (`extension new metric <name> --language … --glob …`); every other kind
/// ignores `metric`.
pub fn scaffold_with(
    root: &Path,
    kind: Kind,
    name: &str,
    origin: Option<&str>,
    metric: &MetricStarter,
) -> Result<Scaffolded, SdkError> {
    if !oxplow_domain::contribution::valid_extension_name(name) {
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
    if kind == Kind::Metric {
        // The metric extension is one template, shared with the
        // `oxplow.metric.scaffold` command.
        let mut files = Vec::new();
        for file in oxplow_app::metric_scaffold::extension_files(name, origin, metric) {
            let path = root.join(&file.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, file.content)?;
            files.push(file.path);
        }
        return Ok(Scaffolded {
            name: name.to_string(),
            dir: rel_dir,
            files,
        });
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
    let ns = oxplow_domain::contribution::extension_namespace(name);
    // Each lens file names its schema, as the manifest does.
    let lens_modeline = extensions::schema::modeline(extensions::schema::LENS_SCHEMA_URL);
    // An identifier from the name: a provider's or a policy's id.
    let provider_id = oxplow_domain::contribution::extension_namespace(name);
    // The intent example, and its fixture (what `extension test` runs).
    let (example_input, example_expect, fixture_expect) = match kind {
        Kind::Lens | Kind::Component | Kind::Page | Kind::Panel => (
            format!("{{ lens: {name}, params: {{ stream_id: 1 }} }}"),
            "one row per open task in the stream, newest first",
            Some("{ rows: $any }".to_string()),
        ),
        Kind::Extension | Kind::Advisory | Kind::Skill => {
            ("{}".to_string(), "TODO: what a run should show", None)
        }
        Kind::Policy => (
            format!(
                "{{ implementation: {provider_id}, event: {{ type: thread.checkpoint, payload: {{ thread: \"thread:thr1\", turn: \"turn:trn1\", reason: turn_end, snapshot: \"snapshot:1\", changed: false, writing_tools: 0 }}, anchors: {{ thread_id: thr1, turn_id: 1 }} }} }}"
            ),
            "nothing to do: a checkpoint opens or closes no effort",
            Some("{ skip: $any }".to_string()),
        ),
        Kind::Provider { capability } => {
            let starter = provider_starter(capability);
            (starter.input, starter.expect, Some(starter.fixture))
        }
        Kind::Collector => (
            "{ collector: items, rows: [{ ref: \"work_item:demo:1\", title: First, state: todo }, { ref: \"work_item:demo:2\", title: Second, state: done }] }"
                .to_string(),
            "one item per work item that isn't done",
            Some("{ entities: { item: 1 } }".to_string()),
        ),
        Kind::Metric => unreachable!("a metric is written from its template above"),
        Kind::Command => (
            format!("{{ command: {ns}.notes.add, input: {{ ref: \"work_item:oxplow:tsk1\" }} }}"),
            "a comment on the work item",
            Some("{ commands: [oxplow.work_item.comment] }".to_string()),
        ),
        Kind::Effect => (
            "{ effect: on-done, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: done } } }"
                .to_string(),
            "a comment on the finished work item",
            Some("{ commands: [oxplow.work_item.comment] }".to_string()),
        ),
    };
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
    // Every service the table offers, with the builtin that reaches it.
    let services = oxplow_domain::service::SERVICES
        .iter()
        .map(|service| {
            let via = match service.builtins.first() {
                Some(builtin) if service.builtins.len() > 1 => format!(" ({builtin}() and more)"),
                Some(builtin) => format!(" ({builtin}())"),
                None => String::new(),
            };
            let approval = if service.approval {
                ", you approve it"
            } else {
                ""
            };
            format!("{}{via}{approval}", service.scope)
        })
        .collect::<Vec<_>>()
        .join(", ");
    match kind {
        Kind::Provider { capability } => {
            manifest.push_str(&format!(
                "providers:\n  - id: {provider_id}\n    capability: {capability}\n    entry: bin/provider\n    declarations: provider.json\n"
            ));
            if let Some(needs) = provider_starter(capability).needs {
                manifest.push_str(needs);
            }
        }
        Kind::Collector => manifest.push_str(&format!(
            "collectors:\n\
             \x20 # Runs its script over its `input` rows when synced (`trigger: manual`;\n\
             \x20 # `{{ every: 15m }}` or `{{ on: [snapshot.taken] }}` run it by themselves).\n\
             \x20 - id: items\n\
             \x20   runtime: starlark\n\
             \x20   entry: collectors/items.star\n\
             \x20   # Services its script may use: {services}.\n\
             \x20   needs: []\n\
             \x20   input: \"SELECT ref, title, state FROM v_work_item\"\n\
             \x20   entities:\n\
             \x20     - {{ name: item, key: ref, columns: {{ ref: text, title: text }} }}\n\
             models:\n\
             \x20 # v_{ns}_open_items: SQL over the entity (`ref('item')`), checked at load.\n\
             \x20 - name: open_items\n\
             \x20   version: 1\n\
             \x20   description: \"TODO: what these rows are.\"\n\
             \x20   columns:\n\
             \x20     - {{ name: ref, type: TEXT, doc: \"The work item (work_item:<provider>:<id>).\" }}\n\
             \x20     - {{ name: title, type: TEXT, doc: \"Its title.\" }}\n"
        )),
        Kind::Command => manifest.push_str(&format!(
            "commands:\n\
             \x20 # Registered as {ns}.notes.add (<namespace>.<area>.<verb>); its script composes core commands, run as the\n\
             \x20 # caller: in one transaction when every one can, else as steps (a work list's verbs run outside it).\n\
             \x20 - name: notes.add\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item.\"\n\
             \x20   input_schema:\n\
             \x20     type: object\n\
             \x20     required: [ref]\n\
             \x20     properties:\n\
             \x20       ref: {{ type: string, description: \"The work item (work_item:<provider>:<id>).\" }}\n\
             \x20     additionalProperties: false\n\
             \x20   entry: handlers/note.star\n\
             \x20   # Who may run it: a person, an agent, a lens (each child command checks its own too).\n\
             \x20   invokers: {{ human: true, agent: true, lens: true }}\n\
             \x20   # On a work item's page (Commands) and a row's right-click.\n\
             \x20   ui: {{ label: Add Note, about: work_item, input: {{ ref: \"{{{{ref}}}}\" }} }}\n\
             \x20   examples:\n\
             \x20     - {{ name: happy, input: {{ ref: \"work_item:oxplow:tsk1\" }}, expect_commands: [oxplow.work_item.comment] }}\n"
        )),
        Kind::Effect => manifest.push_str(
            "effects:\n\
             \x20 # Reacts to a logged event by composing commands, run with an agent's\n\
             \x20 # rights only once a person approves it (Settings → Data), and only on\n\
             \x20 # events logged after; a command that asks becomes a proposal.\n\
             \x20 - id: on-done\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item when it's done.\"\n\
             \x20   on: [work_item.state_changed]\n\
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
        Kind::Policy => manifest.push_str(&format!(
            "implementations:\n\
             \x20 # The project's effort policy, once chosen in Settings → Capabilities: its\n\
             \x20 # script hears core's events and composes the commands that open, close\n\
             \x20 # and link efforts. It runs once a person approves it (Settings → Data →\n\
             \x20 # Programs), and reads oxplow only through the scopes it `needs`.\n\
             \x20 - capability: effort_policy\n\
             \x20   id: {provider_id}\n\
             \x20   title: {title}\n\
             \x20   entry: policies/{name}.star\n\
             \x20   needs: [sql.read]\n",
            title = title_case(name)
        )),
        Kind::Advisory => manifest.push_str(&format!(
            "advisories:\n\
             \x20 # A {advisory}: a query run at a moment (`on`: post-tool-use, prompt or\n\
             \x20 # turn-end) whose rows reach the agent (`audience: person` raises them in\n\
             \x20 # Alerts instead). Each row's `message` is what it hears; `once_per` says how\n\
             \x20 # often. Params: :thread_id, :stream_id, :turn_id, :effort_id. A shared\n\
             \x20 # extension's advisories run once a person approves them.\n\
             \x20 - id: {name}\n\
             \x20   on: turn-end\n\
             \x20   once_per: effort\n\
             \x20   query: |\n\
             \x20     SELECT printf('TODO: what the agent should hear. Here: this effort has changed %d files.',\n\
             \x20                    count(DISTINCT f.path)) AS message\n\
             \x20     FROM v_effort e JOIN v_effort_file f ON f.effort_id = e.id\n\
             \x20     WHERE e.id = :effort_id AND e.ended_at IS NULL\n\
             \x20     HAVING count(DISTINCT f.path) >= 15\n",
            advisory = ContributionKind::Advisory.spec().title.to_lowercase()
        )),
        Kind::Skill => manifest.push_str(&format!(
            "skills:\n\
             \x20 # A {skill} the coding agent loads when it's relevant; `kind: command`\n\
             \x20 # makes it a slash command a person runs. `needs: [work_items]` offers it only\n\
             \x20 # while that capability is active.\n\
             \x20 - name: {name}\n\
             \x20   file: skills/{name}/SKILL.md\n",
            skill = ContributionKind::Skill.spec().title.to_lowercase()
        )),
        Kind::Page => manifest.push_str(&format!(
            "pages:\n\
             \x20 # A {page}: one of this extension's lenses, full size, in the launcher\n\
             \x20 # under `category`.\n\
             \x20 - {{ id: {name}, title: {title}, category: Work, lens: {name} }}\n",
            page = ContributionKind::Page.spec().title.to_lowercase(),
            title = title_case(name)
        )),
        Kind::Panel => manifest.push_str(&format!(
            "panels:\n\
             \x20 # A {panel} in the left nav: its `body` lens, compact. `scope` says what it\n\
             \x20 # follows (project, stream or thread) and the lens declares that param.\n\
             \x20 - {{ id: {name}, title: {title}, scope: stream, body: {name} }}\n",
            panel = ContributionKind::Panel.spec().title.to_lowercase(),
            title = title_case(name)
        )),
        Kind::Lens | Kind::Extension | Kind::Metric => {}
    }
    write(&format!("{rel_dir}/extension.yaml"), manifest)?;
    if let Some(expect) = fixture_expect {
        write(
            &format!("{rel_dir}/fixtures/basic.yaml"),
            format!(
                "# The acceptance example from extension.yaml as a fixture for `oxplow extension test`\n\
                 # (input in, expected output out). Keep the two in step.\n\
                 name: basic\ninput: {example_input}\nexpect: {expect}\n"
            ),
        )?;
    }
    match kind {
        Kind::Provider { capability } => {
            write(
                &format!("{rel_dir}/provider.json"),
                serde_json::to_string_pretty(&provider_declarations(name, capability))
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
                "# The instance config `oxplow extension test` checks the provider with.\nconfig: {}\n"
                    .to_string(),
            )?;
            if capability == "snapshots" {
                write(
                    &format!("{rel_dir}/fixtures/tree/hello.txt"),
                    "The tree the example marks: one file.\n".to_string(),
                )?;
            }
        }
        Kind::Lens => write(
            &format!("{rel_dir}/lenses/{name}.yaml"),
            format!(
                "{modeline}title: {title}\n\
                 description: \"TODO: what this lens answers.\"\n\
                 params:\n\
                 \x20 # Filled in with the viewer's stream unless a value is given.\n\
                 \x20 - {{ name: stream_id, label: Stream }}\n\
                 query: |\n\
                 \x20 SELECT w.ref, w.title, w.state, w.updated_at\n\
                 \x20 FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id\n\
                 \x20 WHERE t.stream_id = :stream_id AND w.state IN ('todo', 'in_progress', 'blocked')\n\
                 \x20 ORDER BY w.updated_at DESC\n\
                 viz: table\n\
                 columns:\n\
                 \x20 - {{ key: title, label: Item, link: {{ kind: page, from: ref }} }}\n\
                 \x20 - {{ key: state }}\n\
                 actions:\n\
                 \x20 # In each row's right-click menu: a command run as the person, the\n\
                 \x20 # row's values bound in (`{{{{row.<column>}}}}`).\n\
                 \x20 - {{ id: start, label: Start, command: oxplow.work_item.transition, input: {{ ref: \"{{{{row.ref}}}}\", to: in_progress }}, row: true }}\n\
                 empty: No open work items in this stream.\n\
                 launcher: {{ category: Work }}\n",
                modeline = lens_modeline,
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
                 \x20       {\"ref\": r[\"ref\"], \"title\": r[\"title\"]}\n\
                 \x20       for r in x[\"rows\"]\n\
                 \x20       if r[\"state\"] != \"done\"\n\
                 \x20   ]}}\n"
                    .to_string(),
            )?;
            write(
                &format!("{rel_dir}/models/open_items.sql"),
                "SELECT ref, title FROM ref('item')\n".to_string(),
            )?;
            write(
                &format!("{rel_dir}/lenses/{name}.yaml"),
                format!(
                    "{modeline}title: {title}\n\
                     description: \"TODO: what this lens answers.\"\n\
                     query: SELECT ref, title FROM v_{ns}_open_items ORDER BY title\n\
                     viz: table\n\
                     columns:\n\
                     \x20 - {{ key: title, label: Item, link: {{ kind: page, from: ref }} }}\n\
                     empty: Nothing collected yet — sync the collector (Settings → Data).\n\
                     launcher: {{ category: Work }}\n",
                    modeline = lens_modeline,
                    title = title_case(name)
                ),
            )?;
        }
        Kind::Command => write(
            &format!("{rel_dir}/handlers/note.star"),
            "# Gets {\"input\": {...}, \"rows\": [...]} and returns the core commands to run\n\
             # as the caller (in one transaction when every one can, else as steps) — or {\"refuse\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"oxplow.work_item.comment\", \"input\": {\"ref\": x[\"input\"][\"ref\"], \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?,
        Kind::Effect => write(
            &format!("{rel_dir}/effects/on-done.star"),
            "# Gets {\"event\": {id, type, v, seq, at, source, subject, anchors, payload, cause}}\n\
             # (the event envelope, at its type's newest version) and returns the\n\
             # commands to run — or {\"skip\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   ref = x[\"event\"][\"payload\"][\"work_item\"]\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"oxplow.work_item.comment\", \"input\": {\"ref\": ref, \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?,
        Kind::Component => {
            write(
                &format!("{rel_dir}/lenses/{name}.yaml"),
                format!(
                    "{modeline}title: {title}\n\
                     description: \"TODO: what this view answers.\"\n\
                     params:\n\
                     \x20 # Filled in with the viewer's stream unless a value is given.\n\
                     \x20 - {{ name: stream_id, label: Stream }}\n\
                     # The rows the component gets (`component.run`), and what an agent reads:\n\
                     # agents always see the table, never the frame.\n\
                     query: |\n\
                     \x20 SELECT w.ref, w.title, w.state\n\
                     \x20 FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id\n\
                     \x20 WHERE t.stream_id = :stream_id AND w.state IN ('todo', 'in_progress', 'blocked')\n\
                     \x20 ORDER BY w.updated_at DESC\n\
                     viz: custom\n\
                     custom: {{ component: {name} }}\n\
                     launcher: {{ category: Work }}\n",
                    modeline = lens_modeline,
                    title = title_case(name)
                ),
            )?;
            write(
                &format!("{rel_dir}/components/{name}/index.html"),
                "<!doctype html>\n\
                 <meta charset=\"utf-8\">\n\
                 <!-- oxplow's look: the kit's classes over the theme, set by applyTheme(). -->\n\
                 <link rel=\"stylesheet\" href=\"/component-lib/oxplow-kit.css\">\n\
                 <div id=\"out\" class=\"ox-muted\"></div>\n\
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
                 \x20 component.applyTheme();\n\
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
        Kind::Page | Kind::Panel => write(
            &format!("{rel_dir}/lenses/{name}.yaml"),
            format!(
                "{modeline}title: {title}\n\
                 description: \"TODO: what this answers.\"\n\
                 params:\n\
                 \x20 # Filled in with the viewer's stream unless a value is given.\n\
                 \x20 - {{ name: stream_id, label: Stream }}\n\
                 query: |\n\
                 \x20 SELECT w.ref, w.title, w.state\n\
                 \x20 FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id\n\
                 \x20 WHERE t.stream_id = :stream_id AND w.state IN ('todo', 'in_progress', 'blocked')\n\
                 \x20 ORDER BY w.updated_at DESC\n\
                 viz: table\n\
                 columns:\n\
                 \x20 - {{ key: title, label: Item, link: {{ kind: page, from: ref }} }}\n\
                 \x20 - {{ key: state }}\n\
                 empty: No open work items in this stream.\n",
                modeline = lens_modeline,
                title = title_case(name)
            ),
        )?,
        Kind::Skill => write(
            &format!("{rel_dir}/skills/{name}/SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: \"TODO: when the agent should load this.\"\n---\n\n\
                 # {title}\n\nTODO: what the agent should know or do.\n",
                title = title_case(name)
            ),
        )?,
        Kind::Policy => write(
            &format!("{rel_dir}/policies/{name}.star"),
            r#"# Gets {"event": {id, type, v, seq, at, source, subject, anchors, payload, cause}}
# — the event envelope of one of core's events a policy hears — and returns
# the commands to run, as the policy, or {"skip": "why"}. No I/O but its
# `needs` (`scope("sql.read", …)`).
def transform(x):
    e = x["event"]
    if e["type"] != "work_item.state_changed":
        return {"skip": "not an item's move"}
    item = e["payload"]["work_item"]
    to = e["payload"]["to"]
    # The thread an agent moved it on, when one did.
    thread = e["anchors"].get("thread_id")
    if to == "in_progress" and thread != None:
        return {"commands": [{"name": "oxplow.effort.open",
                              "input": {"thread": "thread:" + thread, "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {"sql": "SELECT id FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
                                  "params": {"w": item}})
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": "effort:eff" + str(r["id"]), "reason": "switch"}}
                             for r in rows]}
    return {"skip": "a move that changes no effort"}
"#
            .to_string(),
        )?,
        Kind::Extension | Kind::Advisory | Kind::Metric => {}
    }
    Ok(Scaffolded {
        name: name.to_string(),
        dir: rel_dir,
        files,
    })
}

/// What a scaffolded provider of `capability` starts with beyond its
/// contract: the intent example's input, what it should show and the
/// fixture's `expect`, the scopes it `needs` (the manifest's lines), and
/// its capability's `data` (null when the contract asks none).
struct ProviderStarter {
    input: String,
    expect: &'static str,
    fixture: String,
    needs: Option<&'static str>,
    data: serde_json::Value,
}

fn provider_starter(capability: &str) -> ProviderStarter {
    match capability {
        "work_items" => ProviderStarter {
            input: "{ command: create, input: { title: First } }".to_string(),
            expect: "the new item's ref",
            fixture: "{ ref: $any }".to_string(),
            needs: None,
            data: serde_json::Value::Null,
        },
        "effort_policy" => ProviderStarter {
            input: "{ command: react, input: { event: { id: e1, type: thread.checkpoint, v: 1, seq: 1, source: system, subject: [], payload: {}, anchors: {} } } }"
                .to_string(),
            expect: "nothing to do: a checkpoint opens or closes no effort here",
            fixture: "{ skip: $any }".to_string(),
            // It hears events only: what it closes, it reads of oxplow.
            needs: Some(
                "    # A policy hears events only; it reads oxplow (`host/call`) for the rest,\n\
                 \x20   # such as the open efforts of an item that finished.\n\
                 \x20   needs: [sql.read]\n",
            ),
            data: serde_json::Value::Null,
        },
        "agent_harness" => ProviderStarter {
            input: "{ command: render, input: { answer: { kind: ack } } }".to_string(),
            expect: "its hooks' acknowledgement",
            fixture: "{ body: $any }".to_string(),
            needs: None,
            // The environment variables that mark a shell as one of its
            // sessions; the settings a person gives its launch
            // (`agentConfig.<id>`).
            data: serde_json::json!({
                "env_markers": [],
                "settings": [],
            }),
        },
        "snapshots" => ProviderStarter {
            // A small tree beside the fixtures (`fixtures/tree`) stands in for a worktree.
            input: "{ command: mark, input: { stream: 'stream:str1', worktree: fixtures/tree, trigger: manual, parent: null } }"
                .to_string(),
            expect: "a handle for the marked state",
            fixture: "{ handle: $any, unchanged: false, file_count: 1 }".to_string(),
            needs: None,
            data: serde_json::Value::Null,
        },
        "knowledge" => ProviderStarter {
            input: "{ command: write_page, input: { slug: example, body: '# Example', verified_refs: [], removed_refs: [] } }"
                .to_string(),
            expect: "the page's ref",
            fixture: "{ page: 'wiki:example' }".to_string(),
            needs: None,
            data: serde_json::Value::Null,
        },
        _ => {
            let verb = oxplow_domain::capability::contract(capability)
                .and_then(|c| c.verbs.first())
                .map_or("TODO", |v| v.name);
            ProviderStarter {
                input: format!("{{ command: {verb}, input: {{}} }}"),
                expect: "TODO: what it answers",
                fixture: "$any".to_string(),
                needs: None,
                data: serde_json::Value::Null,
            }
        }
    }
}

/// The summary and input schema a scaffold declares for `verb` of
/// `capability`: what core calls it with.
fn verb_input(capability: &str, verb: &str) -> (String, serde_json::Value) {
    use serde_json::json;
    let str_prop = json!({ "type": "string" });
    let state = json!({ "type": "string",
                        "enum": ["todo", "in_progress", "blocked", "done", "canceled"] });
    match (capability, verb) {
        ("work_items", "create") => (
            "Create a work item.".into(),
            json!({ "type": "object", "required": ["title"], "additionalProperties": false,
                    "properties": { "title": str_prop, "body": str_prop,
                                    "state": state, "native_state": str_prop } }),
        ),
        ("work_items", "update") => (
            "Change a work item's title, body or state.".into(),
            json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                    "properties": { "ref": str_prop, "title": str_prop, "body": str_prop,
                                    "state": state, "native_state": str_prop } }),
        ),
        ("work_items", "transition") => (
            "Move a work item to a canonical state, optionally naming a native one.".into(),
            json!({ "type": "object", "required": ["ref", "to"], "additionalProperties": false,
                    "properties": { "ref": str_prop, "to": state, "native_state": str_prop } }),
        ),
        ("agent_harness", "launch") => (
            "How a session of it starts: the launch's input in, { spec: { kind: pty, command, env } } out — \
             the session's bearer in `env`, never in `command`."
                .into(),
            json!({ "type": "object", "required": ["session", "endpoints"] }),
        ),
        ("agent_harness", "tool_use") => (
            "A tool hook's body in oxplow's vocabulary: { tool } (null when it names none).".into(),
            json!({ "type": "object", "required": ["body"],
                    "properties": { "body": { "type": "object" } } }),
        ),
        ("agent_harness", "render") => (
            "A hook answer (ack, deny or context) in its hooks' shape: { body }.".into(),
            json!({ "type": "object", "required": ["answer"],
                    "properties": { "answer": { "type": "object" } } }),
        ),
        ("agent_harness", "prompt") => (
            "What a prompt hook's body is (feature `subagents`): { prompt: { kind: person, text } | \
             { kind: handback, subagent: { id, kind? } } } — null for a body with no prompt."
                .into(),
            json!({ "type": "object", "required": ["body"],
                    "properties": { "body": { "type": "object" } } }),
        ),
        ("agent_harness", "subagent") => (
            "The subagent a SubagentStart / SubagentStop hook's body names (feature `subagents`): \
             { subagent: { id, kind? } }, null for none."
                .into(),
            json!({ "type": "object", "required": ["body"],
                    "properties": { "body": { "type": "object" } } }),
        ),
        ("snapshots", "mark") => (
            "Mark the worktree's state: { handle, unchanged, file_count } — `unchanged` when the tree \
             equals `parent`'s (null: the empty tree)."
                .into(),
            json!({ "type": "object", "required": ["stream", "worktree", "trigger"],
                    "additionalProperties": false,
                    "properties": { "stream": str_prop, "worktree": str_prop, "trigger": str_prop,
                                    "thread": str_prop, "turn": { "type": "integer" },
                                    "effort": str_prop, "budget_ms": { "type": "integer" },
                                    "parent": { "type": ["string", "null"] } } }),
        ),
        ("snapshots", "changed") => (
            "What changed between two marked states: { changes: [{ path, kind: added|modified|deleted, \
             identity (xxh3-128 hex), size }] } — `from` null is the empty tree."
                .into(),
            json!({ "type": "object", "required": ["stream", "from", "to"],
                    "additionalProperties": false,
                    "properties": { "stream": str_prop, "from": { "type": ["string", "null"] },
                                    "to": str_prop } }),
        ),
        ("snapshots", "read_at") => (
            "A file's bytes at a marked state (feature `contents`): { bytes }, base64.".into(),
            json!({ "type": "object", "required": ["handle", "path"],
                    "additionalProperties": false,
                    "properties": { "handle": str_prop, "path": str_prop } }),
        ),
        ("knowledge", "write_page") => (
            "Write a page, creating it or replacing its body: { page } (`wiki:<slug>`), and answer its \
             `knowledge.page.recorded` event with its `refs` (`file:…` / `wiki:…`)."
                .into(),
            json!({ "type": "object", "required": ["slug", "body", "verified_refs", "removed_refs"],
                    "additionalProperties": false,
                    "properties": { "slug": str_prop, "title": str_prop, "body": str_prop,
                                    "verified_refs": { "type": "array", "items": str_prop },
                                    "removed_refs": { "type": "array", "items": str_prop } } }),
        ),
        ("knowledge", "delete_page") => (
            "Delete a page: {}, answering its record with `deleted: true`.".into(),
            json!({ "type": "object", "required": ["slug"], "additionalProperties": false,
                    "properties": { "slug": str_prop } }),
        ),
        ("knowledge", "link") => (
            "Link a page to a target (a wikilink's interior, such as `other-page` or `src/lib.rs`): {}.".into(),
            json!({ "type": "object", "required": ["page", "target"],
                    "additionalProperties": false,
                    "properties": { "page": str_prop, "target": str_prop } }),
        ),
        ("effort_policy", "react") => (
            "Compose the commands an event calls for: { commands } to run, or { skip }.".into(),
            json!({ "type": "object", "required": ["event"],
                    "properties": { "event": { "type": "object" } } }),
        ),
        _ => (
            format!("TODO: what `{verb}` does."),
            json!({ "type": "object" }),
        ),
    }
}

/// A provider's starting declarations, from `capability`'s contract
/// (`.context/providers.md` "What a provider may implement"): the verbs it
/// always needs (`record` access, no confirmation; declare the others with
/// their features), every feature off, the events it may emit with core's
/// schemas, a collector of what it records, and an empty config schema.
fn provider_declarations(
    name: &str,
    capability: &str,
) -> oxplow_provider_protocol::model::InitializeResult {
    use oxplow_provider_protocol::model::*;
    let spec = oxplow_domain::capability::spec(capability).expect("a provider kind's capability");
    let contract = spec
        .provider
        .expect("a provider kind's capability has a contract");
    let features: serde_json::Map<String, serde_json::Value> = spec
        .features
        .iter()
        .map(|f| (f.to_string(), serde_json::Value::Bool(false)))
        .collect();
    let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: name.into(),
            version: "0.1.0".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: capability.into(),
            features: serde_json::Value::Object(features.clone()),
            data: provider_starter(capability).data,
        }],
        commands: contract
            .required(&serde_json::Value::Object(features))
            .into_iter()
            .map(|verb| {
                let (summary, input_schema) = verb_input(capability, verb);
                CommandDecl {
                    name: verb.into(),
                    summary,
                    input_schema,
                    confirm: "never".into(),
                    access: "record".into(),
                    undoable: false,
                }
            })
            .collect(),
        event_types: contract
            .events
            .iter()
            .map(|(event_type, v)| EventTypeDecl {
                event_type: (*event_type).into(),
                v: *v,
                schema: vocabulary
                    .schema(event_type, *v)
                    .expect("a contract's events are core's")
                    .clone(),
            })
            .collect(),
        collectors: contract
            .records
            .map(|r| CollectorDecl {
                name: capability.into(),
                entity: r.entity.into(),
                description: format!("Every {} changed after the cursor.", r.entity),
            })
            .into_iter()
            .collect(),
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
    /// With `--impact`: what going from `against` to the working
    /// tree changes; `null` without it.
    pub impact: Option<oxplow_app::extension_impact::ImpactReport>,
    /// The revision `impact` compares the working tree with.
    pub against: Option<String>,
    /// The services each of its Starlark scripts uses or declares
    /// (`--services` prints them).
    pub services: Vec<oxplow_app::services::ScriptServices>,
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
/// and what `oxplow extension check` prints. `commands` (the running app's
/// registry) checks launcher command entries; without it, a throwaway
/// oxplow's does. With `against` (a git revision), the report also says
/// what going from it to the working tree changes ([`CheckReport::impact`]).
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
            // Check runs no program: it sees no environment.
            throwaway::Host::start(root, std::sync::Arc::new(|_: &str| None))
                .await
                .map_err(SdkError::Invalid)?,
        ),
    };
    let commands = commands.or_else(|| {
        host.as_ref()
            .map(|h| h.svc.commands.as_ref() as extensions::CommandSchemas<'_>)
    });
    // No project database: the throwaway oxplow's, which is empty but has
    // the extension's models and metrics (a lens may read both).
    let empty;
    let (layer, dry_run) = match (layer, host.as_ref()) {
        (Some(layer), _) => (layer, DryRun::Project),
        (None, Some(host)) => (&host.svc.sql, DryRun::EmptyDatabase),
        (None, None) => {
            empty = SqlGateway::with_metrics(oxplow_db::Database::in_memory());
            (&empty, DryRun::EmptyDatabase)
        }
    };
    let extension = extensions::validate_extension(layer, catalog, root, name, commands)
        .await
        .map_err(|e| match e {
            DomainError::NotFound => SdkError::NotFound(name.to_string()),
            other => SdkError::Domain(other),
        })?;
    let impact = match (against, commands) {
        (Some(rev), Some(commands)) => {
            Some(impact_against(root, name, catalog, layer, rev, commands).await?)
        }
        _ => None,
    };
    Ok(CheckReport {
        name: name.to_string(),
        ok: extension.errors.is_empty(),
        errors: extension.errors.clone(),
        warnings: extension.warnings.clone(),
        dry_run,
        services: oxplow_app::services::of_extension(&extension, &|file| {
            extensions::read_extension_file(root, name, file)
        }),
        extension,
        impact,
        against: against.map(str::to_string),
    })
}

/// What going from git revision `against` (`HEAD`, a branch, a sha) to the
/// working tree changes for extension `name` — the review an
/// install or an effort shows, on the CLI: lenses, models and their rows,
/// collectors and their outputs, providers, config. It reads through
/// `layer` and writes nothing: each side's models are temp views.
async fn impact_against(
    root: &Path,
    name: &str,
    catalog: &ExtensionCatalog,
    layer: &SqlGateway,
    against: &str,
    commands: extensions::CommandSchemas<'_>,
) -> Result<oxplow_app::extension_impact::ImpactReport, SdkError> {
    use oxplow_app::extensions::{extension_tree_at, impact_between, ReviewSide};
    // A revision's files through the VCS; nothing here reads a snapshot.
    let trees = oxplow_app::trees::Trees::new(
        std::sync::Arc::new(oxplow_app::vcs::GitProvider),
        std::sync::Arc::new(oxplow_db::SqliteSnapshotStore::new(
            oxplow_db::Database::in_memory(),
        )),
        oxplow_app::blob_store::BlobStore::new(std::env::temp_dir()),
        // The project's workspace filter decides what a revision holds
        // (the main worktree's, for a stream's working copy).
        std::sync::Arc::new(std::sync::RwLock::new(
            oxplow_config::load_project_config(catalog.project_of(root))
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
    Ok(impact_between(
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
/// Each script's services, as `check --services` prints them: the script,
/// then a line per service — declared or implied by its kind, used or not,
/// and whether a person approves it.
/// How each of `ext`'s `imports:` resolved, one line apiece: `` `id` →
/// <contribution> v<n>: available `` or `unavailable (<why>)` — what an
/// author reads to see whether a contract it consumes is there, and what
/// it withholds when it isn't.
pub fn import_lines(ext: &Extension) -> Vec<String> {
    ext.public_contracts
        .bindings
        .iter()
        .map(|b| {
            let state = match &b.unavailable_reason {
                None if b.available => "available".to_string(),
                None => "unavailable".to_string(),
                Some(why) => format!("unavailable ({})", why.describe(&b.contribution.extension)),
            };
            format!("`{}` → {} v{}: {state}", b.id, b.contribution, b.version)
        })
        .collect()
}

/// One line per `ai_profiles:` entry: what serves it. For now every profile
/// falls back to its built-in role's model.
pub fn profile_lines(ext: &Extension) -> Vec<String> {
    ext.ai_profiles
        .iter()
        .map(|p| {
            let features = if p.features.is_empty() {
                String::new()
            } else {
                let names: Vec<&str> = p
                    .features
                    .iter()
                    .map(|f| match f {
                        oxplow_ai::capabilities::ModelFeature::Text => "text",
                        oxplow_ai::capabilities::ModelFeature::ToolCalling => "tool_calling",
                        oxplow_ai::capabilities::ModelFeature::Streaming => "streaming",
                        oxplow_ai::capabilities::ModelFeature::Image => "image",
                    })
                    .collect();
                format!(" (needs {})", names.join(", "))
            };
            let role = format!("{:?}", p.fallback).to_lowercase();
            format!("ai profile `{}` → falls back to `{role}`{features}", p.id)
        })
        .collect()
}

pub fn render_services(report: &CheckReport) -> String {
    use oxplow_app::services::Declared;
    let mut out = String::new();
    for s in &report.services {
        out.push_str(&format!("{} ({}):\n", s.script, s.entry));
        if s.services.is_empty() {
            out.push_str("  (none)\n");
        }
        for u in &s.services {
            let declared = match u.declared {
                Declared::Declared => "declared",
                Declared::Implied => "implied",
                Declared::Undeclared => "NOT declared",
            };
            let used = if u.used { "used" } else { "not used" };
            let approval = if u.approval {
                " — a person approves it"
            } else {
                ""
            };
            out.push_str(&format!("  {}: {declared}, {used}{approval}\n", u.scope));
        }
    }
    out
}

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
            for i in import_lines(&report.extension) {
                out.push_str(&format!("import {i}\n"));
            }
            for line in profile_lines(&report.extension) {
                out.push_str(&format!("{line}\n"));
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
            if let Some(impact) = &report.impact {
                out.push_str(&format!(
                    "impact against {}:\n",
                    report.against.as_deref().unwrap_or("HEAD")
                ));
                if impact.lines.is_empty() {
                    out.push_str("  (nothing changes)\n");
                }
                for line in &impact.lines {
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

    /// Every kind's scaffold names its schema on each file's first line,
    /// and validates against it.
    #[test]
    fn scaffolds_name_and_satisfy_their_schemas() {
        use oxplow_app::extensions::schema;
        let manifest_schema =
            jsonschema::validator_for(&schema::manifest_schema()).expect("compiles");
        let lens_schema = jsonschema::validator_for(&schema::lens_schema()).expect("compiles");
        for kind in [
            Kind::Lens,
            Kind::Extension,
            Kind::provider("work_items").unwrap(),
            Kind::provider("effort_policy").unwrap(),
            Kind::provider("agent_harness").unwrap(),
            Kind::provider("snapshots").unwrap(),
            Kind::provider("knowledge").unwrap(),
            Kind::Collector,
            Kind::Metric,
            Kind::Command,
            Kind::Effect,
            Kind::Component,
            Kind::Policy,
            Kind::Advisory,
            Kind::Skill,
            Kind::Page,
            Kind::Panel,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let made = scaffold(dir.path(), kind, "demo", None).unwrap();
            for file in made.files.iter().filter(|f| f.ends_with(".yaml")) {
                let (url, validator) = if file.ends_with("/extension.yaml") {
                    (schema::MANIFEST_SCHEMA_URL, &manifest_schema)
                } else if file.contains("/lenses/") {
                    (schema::LENS_SCHEMA_URL, &lens_schema)
                } else {
                    continue;
                };
                let text = std::fs::read_to_string(dir.path().join(file)).unwrap();
                assert!(
                    text.starts_with(&schema::modeline(url)),
                    "{kind:?} {file}: {text}"
                );
                let yaml: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
                let value = serde_json::to_value(yaml).unwrap();
                let problems: Vec<String> = validator
                    .iter_errors(&value)
                    .map(|e| e.to_string())
                    .collect();
                assert!(problems.is_empty(), "{kind:?} {file}: {problems:?}");
            }
        }
    }

    /// A scaffolded collector declares `needs:` (none yet) under a comment
    /// that names the services it could take.
    #[test]
    fn a_scaffolded_collector_declares_its_needs_with_the_services_named() {
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), Kind::Collector, "demo", None).unwrap();
        let manifest =
            std::fs::read_to_string(dir.path().join("oxplow/extensions/demo/extension.yaml"))
                .unwrap();
        let comment = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("# Services its script may use"))
            .expect("the needs comment");
        for service in oxplow_domain::service::SERVICES {
            assert!(comment.contains(service.scope), "{comment}");
        }
        assert!(manifest.contains("needs: []"), "{manifest}");
    }

    fn declarations(root: &Path, name: &str) -> serde_json::Value {
        let text =
            std::fs::read_to_string(root.join(format!("oxplow/extensions/{name}/provider.json")))
                .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// A provider scaffolds for any capability a process may implement,
    /// its declarations from the capability's contract: the verbs it
    /// always needs (`confirm: never`, `access: record`), the events it
    /// may emit with core's schema, and a collector of what it records.
    /// Its example and fixture are the capability's.
    #[tokio::test]
    async fn a_provider_scaffolds_any_capability_by_its_contract() {
        assert_eq!(
            Kind::parse("provider"),
            Some(Kind::provider("work_items").unwrap())
        );
        for no_contract in ["vcs", "nope"] {
            let err = Kind::provider(no_contract).unwrap_err().to_string();
            assert!(
                err.contains(&format!("`{no_contract}`"))
                    && err.contains("work_items")
                    && err.contains("effort_policy"),
                "{err}"
            );
        }

        let dir = tempfile::tempdir().unwrap();
        scaffold(
            dir.path(),
            Kind::provider("work_items").unwrap(),
            "tracker",
            None,
        )
        .unwrap();
        let d = declarations(dir.path(), "tracker");
        assert_eq!(d["capabilities"][0]["capability"], "work_items");
        let verbs: Vec<&str> = d["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(verbs, ["create", "update", "transition"]);
        let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        assert_eq!(d["event_types"][0]["type"], "work_item.recorded");
        assert_eq!(d["event_types"][0]["v"], 2);
        assert_eq!(
            Some(&d["event_types"][0]["schema"]),
            vocabulary.schema("work_item.recorded", 2)
        );
        assert_eq!(d["collectors"][0]["entity"], "work_item");

        let dir = tempfile::tempdir().unwrap();
        let made = scaffold(
            dir.path(),
            Kind::provider("effort_policy").unwrap(),
            "steward",
            None,
        )
        .unwrap();
        let manifest =
            std::fs::read_to_string(dir.path().join(format!("{}/extension.yaml", made.dir)))
                .unwrap();
        assert!(
            manifest.contains("capability: effort_policy\n"),
            "{manifest}"
        );
        let d = declarations(dir.path(), "steward");
        assert_eq!(d["capabilities"][0]["capability"], "effort_policy");
        assert_eq!(d["commands"].as_array().unwrap().len(), 1);
        assert_eq!(d["commands"][0]["name"], "react");
        assert_eq!(d["commands"][0]["confirm"], "never");
        assert_eq!(d["commands"][0]["access"], "record");
        assert_eq!(d["event_types"], serde_json::json!([]));
        assert_eq!(d["collectors"], serde_json::json!([]));
        let fixture =
            std::fs::read_to_string(dir.path().join(format!("{}/fixtures/basic.yaml", made.dir)))
                .unwrap();
        assert!(
            fixture.contains("command: react") && fixture.contains("expect: { skip: $any }"),
            "{fixture}"
        );
        let report = check(
            dir.path(),
            "steward",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(report.ok, "{}", render_findings(&report, Format::Text));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    /// The feature-gated harness verbs have real descriptions to declare
    /// them with, not a TODO.
    #[test]
    fn the_subagent_verbs_have_a_description_to_declare() {
        for verb in ["prompt", "subagent"] {
            let (doc, schema) = verb_input("agent_harness", verb);
            assert!(!doc.starts_with("TODO"), "{verb}: {doc}");
            assert_eq!(schema["required"], serde_json::json!(["body"]));
        }
    }

    /// A harness provider scaffolds its hook-path verbs and its `data`
    /// (the instruction files it reads, no markers, no settings); its
    /// example renders an acknowledgement.
    #[tokio::test]
    async fn a_harness_provider_scaffolds_its_data_and_hook_verbs() {
        let dir = tempfile::tempdir().unwrap();
        let made = scaffold(
            dir.path(),
            Kind::provider("agent_harness").unwrap(),
            "relay",
            None,
        )
        .unwrap();
        let d = declarations(dir.path(), "relay");
        let verbs: Vec<&str> = d["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(verbs, ["launch", "tool_use", "render"]);
        assert_eq!(
            d["capabilities"][0]["data"],
            serde_json::json!({ "env_markers": [], "settings": [] })
        );
        let fixture =
            std::fs::read_to_string(dir.path().join(format!("{}/fixtures/basic.yaml", made.dir)))
                .unwrap();
        assert!(
            fixture.contains("command: render") && fixture.contains("expect: { body: $any }"),
            "{fixture}"
        );
        let report = check(
            dir.path(),
            "relay",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(report.ok, "{}", render_findings(&report, Format::Text));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    /// The scaffolded collector, with its `needs:` line, still checks clean
    /// and passes its own `test`.
    #[tokio::test]
    async fn a_scaffolded_collector_checks_clean_and_passes_its_test() {
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), Kind::Collector, "demo", None).unwrap();
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
        let tested = crate::extension_test::test_extension(dir.path(), "demo", false)
            .await
            .unwrap();
        assert!(tested.ok, "{:?}", tested.errors);
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
        std::fs::write(
            &lens,
            body.replace("FROM v_work_item", "FROM v_no_such_view"),
        )
        .unwrap();
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
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=99.0\"\nversion: 0.1.0\nintent:\n  purpose: x\n  examples: [{ name: a }]\n",
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
            text.contains("error: oxplow/extensions/team/extension.yaml:4: needs oxplow >=99.0"),
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

    /// `check` prints a line per `ai_profiles:` entry: what serves it.
    #[tokio::test]
    async fn check_prints_a_line_per_ai_profile() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/prof/extension.yaml",
            "manifest: 2\nname: prof\nsharing: private\nintent:\n  purpose: x\n  examples: [{ name: a }]\nai_profiles:\n  - { id: review, description: Reviews., fallback: summarize, features: [tool_calling, streaming] }\n  - { id: plain, description: Plain., fallback: main }\n",
        );
        let report = check(
            dir.path(),
            "prof",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let text = render_findings(&report, Format::Text);
        assert!(
            text.contains(
                "ai profile `review` → falls back to `summarize` (needs tool_calling, streaming)\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("ai profile `plain` → falls back to `main`\n"),
            "{text}"
        );
    }

    /// A shared extension travels with the repository: a token in its
    /// manifest, a script or a fixture is a warning that names the file and
    /// line and never the value. A private extension isn't scanned.
    #[tokio::test]
    async fn check_warns_about_a_credential_in_a_shared_extension_only() {
        let token = format!("ghp_{}", "Zz9Yy8Xx7W".repeat(4));
        let dir = tempfile::tempdir().unwrap();
        for (name, sharing) in [("team", "shared"), ("mine", "private")] {
            let shared = if sharing == "shared" {
                "engine: \">=0.1\"\nversion: 0.1.0\n"
            } else {
                ""
            };
            write(
                dir.path(),
                &format!("oxplow/extensions/{name}/extension.yaml"),
                &format!("manifest: 2\nname: {name}\nsharing: {sharing}\n{shared}intent:\n  purpose: x\n  examples: [{{ name: a }}]\n# token: {token}\n"),
            );
            write(
                dir.path(),
                &format!("oxplow/extensions/{name}/fixtures/a.yaml"),
                &format!("input: {{}}\nsecret: {token}\n"),
            );
        }
        let text = |report: &CheckReport| render_findings(report, Format::Text);
        let team = check(
            dir.path(),
            "team",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let team_text = text(&team);
        assert!(team.ok, "{team_text}");
        assert!(
            team_text.contains("warning: oxplow/extensions/team/extension.yaml:9: looks like a GitHub token (`ghp_…`, 44 characters)"),
            "{team_text}"
        );
        assert!(
            team_text.contains(
                "warning: oxplow/extensions/team/fixtures/a.yaml:2: looks like a GitHub token"
            ),
            "{team_text}"
        );
        assert!(!team_text.contains(&token), "{team_text}");
        let mine = check(
            dir.path(),
            "mine",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(!text(&mine).contains("GitHub token"), "{}", text(&mine));
    }

    /// A fixture that no intent example names never runs, and an example
    /// without a fixture runs nothing: each is a finding that names the file
    /// or example and the fix, not a bare "not found" later.
    #[tokio::test]
    async fn check_reports_fixtures_and_intent_examples_that_do_not_pair_up() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/pairs/extension.yaml",
            "manifest: 2\nname: pairs\nsharing: private\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: listed }, { name: missing }]\n",
        );
        for fixture in ["listed", "orphan", "model-counts", "provider-feed"] {
            write(
                dir.path(),
                &format!("oxplow/extensions/pairs/fixtures/{fixture}.yaml"),
                "input: {}\n",
            );
        }
        let report = check(
            dir.path(),
            "pairs",
            &ExtensionCatalog::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let text = render_findings(&report, Format::Text);
        assert!(
            text.contains(
                "oxplow/extensions/pairs/fixtures/orphan.yaml:1: no intent example names this fixture, so it never runs — fix: add `orphan` to intent.examples"
            ),
            "{text}"
        );
        let tested = extension_test::test_extension(dir.path(), "pairs", false)
            .await
            .unwrap();
        assert!(
            tested.warnings.iter().any(|w| w.starts_with(
                "oxplow/extensions/pairs/extension.yaml:1: intent example `missing` has no fixture, so it isn't run — fix: add fixtures/missing.yaml"
            )),
            "{:?}",
            tested.warnings
        );
        // A kit's fixtures aren't intent examples.
        assert!(
            !text.contains("model-counts") && !text.contains("provider-feed"),
            "{text}"
        );
        assert!(!text.contains("`listed`"), "{text}");
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

    #[test]
    fn name_of_reads_a_folder_or_a_name() {
        assert_eq!(name_of("oxplow/extensions/old/"), "old");
        assert_eq!(name_of("old"), "old");
    }
}
