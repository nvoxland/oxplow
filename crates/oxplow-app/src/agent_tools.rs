//! The agent's MCP tools that extensions declare (`.context/extensions.md`
//! "Tools"). A tool is a command or a lens that asks to be one (`tool:
//! <name>`, offered as `<namespace>_<name>`): oxplow's own are
//! `extensions/oxplow-foundation`'s declarations over core's operations,
//! any other extension's are its own. Core builds only `list_commands` and
//! `run_command`; everything else an agent reaches as a tool comes from
//! here, so the tool list follows the enabled extensions and the commands
//! the caller is offered.
//!
//! A command tool's description, schema and example are its command's, and
//! a call is a run of that command as the caller. A lens tool's input is its
//! params (and `format`), and a call runs `oxplow.lens.run` on it.

use std::collections::BTreeMap;

use oxplow_domain::{Actor, DomainError};
use serde_json::{json, Map, Value};

use crate::extensions::{Extension, Lens};
use crate::Services;

/// What a tool runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolTarget {
    /// A command, by id (`oxplow.sql.query`).
    Command(String),
    /// A lens, by id (`oxplow-foundation/thread-work`), through
    /// [`LENS_RUN`].
    Lens(String),
    /// A page, by id (`oxplow-foundation/thread-work`), through
    /// [`PAGE_RUN`].
    Page(String),
}

/// One tool an agent is offered.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTool {
    /// `<namespace>_<name>`.
    pub name: String,
    pub description: String,
    /// A JSON Schema object.
    pub input_schema: Value,
    /// Its result's JSON Schema, when its command declares one: the
    /// result then goes to the client as structured content too.
    pub output_schema: Option<Value>,
    /// It changes nothing the person keeps (a read, or only what they see).
    pub read_only: bool,
    pub target: ToolTarget,
}

/// The command a lens tool runs.
pub const LENS_RUN: &str = "oxplow.lens.run";

/// The command a page tool runs.
pub const PAGE_RUN: &str = "oxplow.page.run";

impl AgentTool {
    /// The command a call runs and its input.
    pub fn call(&self, input: Value) -> (String, Value) {
        match &self.target {
            ToolTarget::Command(id) => (id.clone(), input),
            ToolTarget::Lens(id) => {
                let mut params = match input {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                let format = params.remove("format");
                (
                    LENS_RUN.to_string(),
                    json!({ "id": id, "params": params, "format": format }),
                )
            }
            ToolTarget::Page(id) => {
                let mut params = match input {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                let mut call = json!({ "page": id, "params": {} });
                if let Some(format) = params.remove("format").filter(|f| !f.is_null()) {
                    call["format"] = format;
                }
                call["params"] = Value::Object(params);
                (PAGE_RUN.to_string(), call)
            }
        }
    }
}

/// The tools `actor` is offered now: each enabled extension's command
/// tools whose command is offered to it, and its lens tools, by name.
pub async fn offered(svc: &Services, actor: &Actor) -> Result<Vec<AgentTool>, DomainError> {
    svc.extension_activation
        .read(async {
            let extensions = svc.extension_catalog.get(svc.worktrees.project_dir());
            let commands: BTreeMap<String, oxplow_domain::CommandSpec> = svc
                .commands
                .list(actor)
                .into_iter()
                .map(|spec| (spec.id.clone(), spec))
                .collect();
            tools_of(&extensions, |id| commands.get(id).cloned())
        })
        .await
}

/// Every tool `extensions` declare, the command ones only where
/// `command(id)` offers that command.
pub fn tools_of(
    extensions: &[Extension],
    command: impl Fn(&str) -> Option<oxplow_domain::CommandSpec>,
) -> Vec<AgentTool> {
    let mut tools = Vec::new();
    for ext in extensions.iter().filter(|e| e.enabled) {
        for c in &ext.commands {
            let (Some(name), Some(spec)) = (&c.tool, command(&c.name)) else {
                continue;
            };
            tools.push(AgentTool {
                name: name.clone(),
                description: spec.summary.clone(),
                input_schema: spec.input_schema.clone(),
                output_schema: spec.output_schema.clone(),
                read_only: spec.access.reads_only() && !spends(&spec),
                target: ToolTarget::Command(spec.id.clone()),
            });
        }
        for lens in &ext.lenses {
            if let Some(name) = &lens.tool {
                tools.push(AgentTool {
                    name: name.clone(),
                    description: lens_description(lens),
                    input_schema: lens_schema(lens),
                    // What a lens tool returns depends on its `format`.
                    output_schema: None,
                    read_only: true,
                    target: ToolTarget::Lens(lens.id.clone()),
                });
            }
        }
        for page in &ext.page_files {
            if let Some(name) = &page.tool {
                tools.push(AgentTool {
                    name: name.clone(),
                    description: page_description(page),
                    input_schema: params_schema(&page.params),
                    // What a page tool returns depends on its `format`.
                    output_schema: None,
                    read_only: true,
                    target: ToolTarget::Page(page.id.clone()),
                });
            }
        }
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

/// Whether a run of `spec` spends the person's money: it needs a scope
/// that does (an operation's own scope is among its needs). Such a tool
/// asks before it runs, though it changes no record.
fn spends(spec: &oxplow_domain::CommandSpec) -> bool {
    spec.needs
        .iter()
        .any(|id| oxplow_domain::scope::scope(id).is_some_and(|scope| scope.spends))
}

fn lens_description(lens: &Lens) -> String {
    let about = if lens.description.is_empty() {
        lens.title.clone()
    } else {
        format!("{}: {}", lens.title, lens.description)
    };
    format!(
        "{about} Returns what the person would see on the lens's page, as text; \
         `format: \"json\"` returns its rows."
    )
}

/// A lens's params as a tool's input: one property per param (its label,
/// its choices, its default) and `format`. None is required: a param left
/// out takes its default or what the caller's context binds
/// (`thread_id`, `stream_id`).
fn lens_schema(lens: &Lens) -> Value {
    params_schema(&lens.params)
}

fn page_description(page: &crate::pages::schema::Page) -> String {
    let about = if page.description.is_empty() {
        page.title.clone()
    } else {
        format!("{}: {}", page.title, page.description)
    };
    format!(
        "{about} Returns what the person would see on the page, as text; \
         `format: \"json\"` returns its loaders' rows."
    )
}

/// A page's (or lens's) params as a tool's input: one property per param
/// (its label, its choices, its default) and `format`. None is required:
/// a param left out takes its default or what the caller's context binds
/// (`thread_id`, `stream_id`).
fn params_schema(params: &[crate::extensions::PageParam]) -> Value {
    let mut properties = Map::new();
    for param in params {
        let mut property = Map::new();
        // Options a lens lists aren't known until it runs: the agent runs
        // that lens to see them.
        let from = param
            .options_from
            .as_ref()
            .map(|from| format!("one of the `value`s `{from}` lists (run it to see them)"));
        let description = match (&param.label, from) {
            (Some(label), Some(from)) => Some(format!("{label} — {from}")),
            (Some(label), None) => Some(label.clone()),
            (None, Some(from)) => Some(from),
            (None, None) => None,
        };
        if let Some(description) = description {
            property.insert("description".into(), json!(description));
        }
        if !param.options.is_empty() {
            let values: Vec<&str> = param.options.iter().map(|o| o.value.as_str()).collect();
            property.insert("enum".into(), json!(values));
        }
        if let Some(default) = &param.default {
            property.insert("default".into(), json!(default));
        }
        properties.insert(param.name.clone(), Value::Object(property));
    }
    properties.insert(
        "format".into(),
        json!({
            "type": "string",
            "enum": ["text", "json"],
            "description": "`text` (default): what it shows. `json`: the full run, rows included."
        }),
    );
    json!({ "type": "object", "properties": properties, "additionalProperties": false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{activate_with_problems, services_with_task_effort};
    use oxplow_domain::Actor;

    fn agent(fx: &crate::test_fixtures::TaskEffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// oxplow's own tools are foundation's declarations, each named under
    /// `oxplow_`, its hint its command's access.
    #[tokio::test]
    async fn oxplows_tools_are_foundations_declarations() {
        let fx = services_with_task_effort().await;
        let tools = offered(&fx.svc, &agent(&fx)).await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "oxplow_ai_decide",
                "oxplow_ai_summarize",
                "oxplow_catalog",
                "oxplow_code_call_hierarchy",
                "oxplow_code_definition",
                "oxplow_code_hover",
                "oxplow_code_references",
                "oxplow_code_symbols",
                "oxplow_code_workspace_symbols",
                "oxplow_diff",
                "oxplow_effort_summary",
                "oxplow_ensure_change",
                "oxplow_find_page",
                "oxplow_get_open_page",
                "oxplow_get_skill",
                "oxplow_list_comments",
                "oxplow_list_tabs",
                "oxplow_navigate",
                "oxplow_preview_collector",
                "oxplow_preview_extension",
                "oxplow_query_sql",
                "oxplow_read",
                "oxplow_run_lens",
                "oxplow_run_lens_action",
                "oxplow_run_page",
                "oxplow_run_page_action",
                "oxplow_search",
                "oxplow_show_lens",
                "oxplow_show_page",
                "oxplow_thread_work",
                "oxplow_validate_extension",
                "oxplow_vcs_blame",
                "oxplow_vcs_branches",
                "oxplow_vcs_log",
                "oxplow_vcs_status",
                "oxplow_wiki_ref_drift",
                "oxplow_work_next",
            ]
        );
        let query = tools.iter().find(|t| t.name == "oxplow_query_sql").unwrap();
        assert_eq!(query.target, ToolTarget::Command("oxplow.sql.query".into()));
        assert!(query.read_only);
        assert!(query.input_schema["properties"]["sql"].is_object());
        assert!(query.input_schema["examples"].is_array(), "its example");
        // A page tool's params are its input's properties.
        let work = tools
            .iter()
            .find(|t| t.name == "oxplow_thread_work")
            .unwrap();
        assert_eq!(
            work.target,
            ToolTarget::Page("oxplow-foundation/thread-work".into())
        );
        assert!(work.read_only);
        assert!(work.input_schema["properties"]["thread"].is_object());
        let show = tools.iter().find(|t| t.name == "oxplow_show_lens").unwrap();
        assert!(!show.read_only, "it records the answer in the thread");
    }

    /// A tool that spends the person's money asks before it runs: a model
    /// call changes no record, but its hint isn't read-only. The reads that
    /// cost nothing keep theirs.
    #[tokio::test]
    async fn a_tool_over_a_spending_scope_is_not_hinted_read_only() {
        let fx = services_with_task_effort().await;
        let tools = offered(&fx.svc, &agent(&fx)).await.unwrap();
        let hinted = |name: &str| tools.iter().find(|t| t.name == name).unwrap().read_only;
        for spends in ["oxplow_ai_decide", "oxplow_ai_summarize"] {
            assert!(!hinted(spends), "{spends} spends the person's key");
        }
        for free in [
            "oxplow_query_sql",
            "oxplow_ensure_change",
            "oxplow_preview_collector",
        ] {
            assert!(hinted(free), "{free} costs nothing");
        }
    }

    /// An extension's command over a spending scope's operation is a spending
    /// tool too (a script can't need such a scope).
    #[tokio::test]
    async fn an_extension_command_over_ai_compute_is_not_hinted_read_only() {
        let fx = services_with_task_effort().await;
        let root = fx._dir.path().join(".oxplow/extensions/acme");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: p\ncommands:\n\
             \x20 - name: ask.model\n    summary: Ask the model.\n    scope: ai.compute\n    op: summarize\n\
             \x20   tool: ask_model\n\
             \x20 - name: work.count\n    summary: Count.\n    scope: sql.read\n    op: query\n\
             \x20   tool: count\n",
        )
        .unwrap();
        let problems = activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        let tools = offered(&fx.svc, &agent(&fx)).await.unwrap();
        let hinted = |name: &str| tools.iter().find(|t| t.name == name).map(|t| t.read_only);
        assert_eq!(hinted("acme_ask_model"), Some(false));
        assert_eq!(hinted("acme_count"), Some(true));
    }

    /// Any extension declares tools the same way: a command offered to
    /// the caller, and a lens whose params are its input. Every command is
    /// offered to an agent, so each one that names a tool is one.
    #[tokio::test]
    async fn an_extensions_command_and_lens_are_tools() {
        let fx = services_with_task_effort().await;
        let root = fx._dir.path().join(".oxplow/extensions/acme");
        std::fs::create_dir_all(root.join("lenses")).unwrap();
        std::fs::write(
            root.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: p\ncommands:\n\
             \x20 - name: work.count\n    summary: Count the work items.\n    scope: sql.read\n    op: query\n\
             \x20   example: { sql: \"SELECT 1\" }\n    tool: count\n\
             \x20 - name: work.secret\n    summary: Also for agents.\n    scope: sql.read\n    op: query\n\
             \x20   tool: secret\n",
        )
        .unwrap();
        std::fs::write(
            root.join("lenses/open.yaml"),
            "title: Open Work\ndescription: The items still open.\nquery: SELECT 1 AS n\n\
             params:\n  - { name: state, label: State, default: todo, options: [{ value: todo, label: To do }, { value: done, label: Done }] }\n\
             \x20 - { name: owner, label: Owner, options_from: owners }\n\
             tool: open_work\n",
        )
        .unwrap();
        std::fs::write(
            root.join("lenses/owners.yaml"),
            "title: Owners\nquery: SELECT 'me' AS value, 'Me' AS label\n",
        )
        .unwrap();
        activate_with_problems(&fx.svc).await;
        let tools = offered(&fx.svc, &agent(&fx)).await.unwrap();
        let acme: Vec<&AgentTool> = tools
            .iter()
            .filter(|t| t.name.starts_with("acme_"))
            .collect();
        assert_eq!(
            acme.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["acme_count", "acme_open_work", "acme_secret"]
        );
        let lens = acme[1];
        assert_eq!(
            lens.input_schema["properties"]["state"],
            json!({ "description": "State", "enum": ["todo", "done"], "default": "todo" })
        );
        // Options a lens lists aren't known until it runs: no enum, and
        // the description says which lens to run to see them.
        assert_eq!(
            lens.input_schema["properties"]["owner"],
            json!({ "description": "Owner — one of the `value`s `acme/owners` lists (run it to see them)" })
        );
        assert!(lens
            .description
            .starts_with("Open Work: The items still open."));
        assert_eq!(
            lens.call(json!({ "state": "done", "format": "json" })),
            (
                LENS_RUN.to_string(),
                json!({ "id": "acme/open", "params": { "state": "done" }, "format": "json" })
            )
        );
        assert_eq!(
            acme[0].call(json!({ "sql": "SELECT 1" })),
            ("acme.work.count".to_string(), json!({ "sql": "SELECT 1" }))
        );
    }

    /// A page asks to be a tool the same way (`tool:`): its params are the
    /// input, and a call runs `oxplow.page.run` on it.
    #[tokio::test]
    async fn an_extensions_page_is_a_tool() {
        let fx = services_with_task_effort().await;
        let root = fx._dir.path().join(".oxplow/extensions/acme");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: p\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/open.yaml"),
            "title: Open Work\ndescription: The items still open.\n\
             params:\n  - { name: state, label: State, default: todo, options: [{ value: todo, label: To do }, { value: done, label: Done }] }\n\
             data:\n  items:\n    sql: SELECT :state AS state\n\
             view:\n  viz: table\n  from: items\n\
             tool: open_work\n",
        )
        .unwrap();
        let problems = activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        let tools = offered(&fx.svc, &agent(&fx)).await.unwrap();
        let page = tools
            .iter()
            .find(|t| t.name == "acme_open_work")
            .expect("the page's tool");
        assert_eq!(page.target, ToolTarget::Page("acme/open".into()));
        assert!(page.read_only);
        assert_eq!(
            page.input_schema["properties"]["state"],
            json!({ "description": "State", "enum": ["todo", "done"], "default": "todo" })
        );
        assert!(page
            .description
            .starts_with("Open Work: The items still open."));
        assert_eq!(
            page.call(json!({ "state": "done", "format": "json" })),
            (
                PAGE_RUN.to_string(),
                json!({ "page": "acme/open", "params": { "state": "done" }, "format": "json" })
            )
        );
        let (command, input) = page.call(json!({ "state": "done" }));
        let out = fx
            .svc
            .commands
            .run(&agent(&fx), &command, input, false)
            .await
            .unwrap();
        let out = format!("{out:?}");
        assert!(out.contains("done"), "{out}");
    }

    /// The agent model doc's tool table names every tool oxplow offers.
    #[tokio::test]
    async fn every_foundation_tool_is_in_the_agent_model_doc() {
        let fx = services_with_task_effort().await;
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.context/agent-model.md"),
        )
        .unwrap();
        let missing: Vec<String> = offered(&fx.svc, &agent(&fx))
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .filter(|name| !doc.contains(&format!("`{name}")))
            .collect();
        assert!(
            missing.is_empty(),
            "not in .context/agent-model.md: {missing:?}"
        );
    }
}
