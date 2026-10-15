//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Core builds two tools, the command
//! bus itself: `list_commands` and `run_command`. Every other tool is a
//! command or a lens an enabled extension declares with `tool:`
//! (`oxplow_app::agent_tools`) — oxplow's own are `oxplow-foundation`'s,
//! named `oxplow_*` — listed per caller and run through the bus as it.

use oxplow_app::agent_tools::AgentTool;
use oxplow_app::commands::agent_reads::extensions::{CheckError, ExtensionChecks};
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::*;
use rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::session_auth::Principal;
use oxplow_app::Services;

mod lenient_params;
// Drop-in for rmcp's `Parameters` that tolerates camelCase/kebab aliases
// of our snake_case param fields, so weak models stop tripping on
// `-32602 missing field`. See `lenient_params` for the rationale.
use lenient_params::Parameters;

#[derive(Clone)]
pub struct OxplowMcp {
    services: Arc<Services>,
    tool_router: ToolRouter<Self>,
}

/// The extension checks `oxplow.extension.validate` and `.preview` run
/// (`oxplow-sdk`'s, which `oxplow-app` can't link): installed on the
/// services of every host that serves these tools. Idempotent.
pub fn install_extension_checks(services: &Services) {
    services.extension_checks.install(Arc::new(SdkChecks));
}

struct SdkChecks;

#[async_trait::async_trait]
impl ExtensionChecks for SdkChecks {
    async fn validate(
        &self,
        svc: &Services,
        root: &std::path::Path,
        name: &str,
    ) -> Result<serde_json::Value, CheckError> {
        let report = oxplow_sdk::check(
            root,
            name,
            &svc.extension_catalog,
            Some(&svc.sql),
            Some(svc.commands.as_ref()),
            None,
        )
        .await
        .map_err(sdk_check_error)?;
        serde_json::to_value(report).map_err(|e| CheckError::Failed(e.to_string()))
    }

    async fn preview(
        &self,
        root: &std::path::Path,
        name: &str,
    ) -> Result<serde_json::Value, CheckError> {
        let report = oxplow_sdk::extension_test::test_extension_in(
            root,
            name,
            false,
            Arc::new(|_: &str| None),
        )
        .await
        .map_err(sdk_check_error)?;
        serde_json::to_value(report).map_err(|e| CheckError::Failed(e.to_string()))
    }
}

fn sdk_check_error(e: oxplow_sdk::SdkError) -> CheckError {
    match e {
        oxplow_sdk::SdkError::NotFound(_) => CheckError::NotFound,
        other => CheckError::Failed(other.to_string()),
    }
}

// ---------- caller identity ----------

/// Who is calling: the agent session, thread and stream behind an MCP
/// request.
///
/// The control plane admits an MCP request only with an agent session's
/// bearer and puts that session's [`Principal`] in the request's
/// extensions (`.context/agent-model.md` "Caller identity"); rmcp hands the
/// request's `http::request::Parts` to tools through the call's
/// `Extensions`, and [`caller_of`] reads the principal there. Nothing the
/// request says about itself — a header, the URL — names the caller. A
/// transport with no HTTP parts (stdio, a unit test) is an anonymous
/// agent: it may read, and `run_command` refuses to write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpCaller {
    pub session_id: Option<oxplow_domain::AgentSessionId>,
    pub thread_id: Option<oxplow_domain::ThreadId>,
    pub stream_id: Option<oxplow_domain::StreamId>,
}

impl McpCaller {
    pub fn from_parts(parts: &http::request::Parts) -> Self {
        parts
            .extensions
            .get::<Principal>()
            .map(Self::of)
            .unwrap_or_default()
    }

    /// The caller a session's bearer stands for.
    pub fn of(principal: &Principal) -> Self {
        Self {
            session_id: Some(principal.session),
            thread_id: Some(principal.thread),
            stream_id: Some(principal.stream),
        }
    }

    pub fn actor(&self) -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
            session_id: self.session_id,
            thread_id: self.thread_id,
            stream_id: self.stream_id,
        }
    }
}

/// The caller behind a tool call, from the request extensions rmcp
/// attaches; anonymous when the transport carried no HTTP parts.
pub fn caller_of(extensions: &rmcp::model::Extensions) -> McpCaller {
    extensions
        .get::<http::request::Parts>()
        .map(McpCaller::from_parts)
        .unwrap_or_default()
}

/// What `run_command` says to a caller with no thread identity.
const ANONYMOUS_WRITE: &str = "this MCP connection carries no thread identity (it didn't come \
    through an agent session's bearer), so it may not run commands; oxplow's own harness \
    configs connect with one — reconnect through one";

// ---------- request shapes ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunCommandParams {
    /// The command id, as `list_commands` reports it (`oxplow.config.set`,
    /// `oxplow.work_item.transition`).
    pub id: String,
    /// The command's input, matching its `input_schema`.
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListCommandsParams {
    /// One command's id (`oxplow.work_item.create`): it alone, in full.
    #[serde(default)]
    pub id: Option<String>,
    /// The start of the ids wanted (`oxplow.work_item.`): each, in full.
    #[serde(default)]
    pub prefix: Option<String>,
}

/// A summary's first sentence: what the index shows (the whole summary
/// comes with the command).
fn first_sentence(summary: &str) -> &str {
    // A sentence ends at ". " before a capital: `e.g. no` goes on.
    summary
        .match_indices(". ")
        .find(|(at, _)| {
            summary[at + 2..]
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
        })
        .map_or(summary, |(at, _)| &summary[..=at])
}

/// A command as `list_commands`' index shows it.
#[derive(Debug, Serialize)]
struct CommandIndexEntry {
    id: String,
    summary: String,
}

/// A command as an agent needs it to run one: the person-facing parts of
/// its spec (`ui`, `op`, `atomicity`, …) left out, and its example input
/// shown once, beside the schema rather than inside it.
#[derive(Debug, Serialize)]
struct CommandView {
    id: String,
    summary: String,
    input_schema: serde_json::Value,
    example: Option<serde_json::Value>,
    confirm: oxplow_domain::Confirm,
    undoable: bool,
}

impl CommandView {
    fn of(spec: &oxplow_domain::CommandSpec) -> Self {
        let mut input_schema = spec.input_schema.clone();
        let example = input_schema
            .as_object_mut()
            .and_then(|schema| schema.remove("examples"))
            .and_then(|examples| examples.as_array()?.first().cloned());
        Self {
            id: spec.id.clone(),
            summary: spec.summary.clone(),
            input_schema,
            example,
            confirm: spec.confirm,
            undoable: spec.undoable,
        }
    }
}

/// Why `list_commands` found nothing: the id or prefix isn't offered, and
/// the offered commands whose ids end like the asked one.
fn no_command_message(
    offered: &[oxplow_domain::CommandSpec],
    id: Option<&str>,
    prefix: Option<&str>,
) -> String {
    let asked = id.or(prefix).unwrap_or_default();
    let ending_in = |suffix: &str| -> Vec<&str> {
        offered
            .iter()
            .map(|spec| spec.id.as_str())
            .filter(|candidate| !suffix.is_empty() && candidate.ends_with(&format!(".{suffix}")))
            .collect()
    };
    // `config.get` → `oxplow.config.get`; else by its last segment.
    let whole = asked.trim_matches('.');
    let mut alike = ending_in(whole);
    if alike.is_empty() {
        alike = ending_in(whole.rsplit('.').next().unwrap_or(whole));
    }
    let what = if id.is_some() {
        "command"
    } else {
        "commands under"
    };
    if alike.is_empty() {
        format!(
            "no {what} `{asked}` is offered to you; `list_commands` with no arguments lists them"
        )
    } else {
        format!(
            "no {what} `{asked}` is offered to you; did you mean {}?",
            alike
                .iter()
                .map(|candidate| format!("`{candidate}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        install_extension_checks(&services);
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "The commands this agent may run through `run_command`. With no \
                       arguments: every command's `id` and one-line `summary`. With `id` (one \
                       command) or `prefix` (an area, `oxplow.work_item.`): each with its \
                       `input_schema`, an `example` input that fits it, whether a person \
                       `confirm`s it and whether it is `undoable`. Read a command this way \
                       before its first run. Commands are the one write path: every run is \
                       validated, policy-checked, audited and logged as `command.executed`."
    )]
    async fn list_commands(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ListCommandsParams>,
    ) -> Result<CallToolResult, McpError> {
        let actor = caller_of(&extensions).actor();
        let offered = self.services.commands.list(&actor);
        let p = params.0;
        if p.id.is_none() && p.prefix.is_none() {
            let index: Vec<CommandIndexEntry> = offered
                .into_iter()
                .map(|spec| CommandIndexEntry {
                    summary: first_sentence(&spec.summary).to_string(),
                    id: spec.id,
                })
                .collect();
            return json_result(&index);
        }
        let views: Vec<CommandView> = offered
            .iter()
            .filter(|spec| p.id.as_deref().is_none_or(|id| spec.id == id))
            .filter(|spec| {
                p.prefix
                    .as_deref()
                    .is_none_or(|pre| spec.id.starts_with(pre))
            })
            .map(CommandView::of)
            .collect();
        if views.is_empty() {
            return Err(McpError::invalid_params(
                no_command_message(&offered, p.id.as_deref(), p.prefix.as_deref()),
                None,
            ));
        }
        json_result(&views)
    }

    #[tool(
        description = "Run a command by its id with its input (`list_commands { id }` shows its \
                       schema and an example input). Returns \
                       `{ result, audit_id, event_id, inverse? }`. Invalid input names the \
                       failing field; a denied command says why. A command that needs a \
                       person's confirmation is not run: it is recorded as a proposal and \
                       this returns `{ proposal, message }` — tell the person it waits for \
                       them in this thread and in Alerts, and don't run it again; its \
                       `decision` is in `v_command_proposal`. \
                       Requires the connection's thread identity: an anonymous connection may \
                       not write."
    )]
    async fn run_command(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<RunCommandParams>,
    ) -> Result<CallToolResult, McpError> {
        let caller = caller_of(&extensions);
        self.run_command_as(&caller, params.0).await
    }
}

/// A declared tool as MCP lists it: its command's or lens's description
/// and schema, hinted read-only when it changes nothing the person keeps.
fn mcp_tool(tool: &AgentTool) -> Tool {
    let schema = match &tool.input_schema {
        serde_json::Value::Object(schema) => schema.clone(),
        _ => serde_json::Map::new(),
    };
    let mut listed = Tool::new(
        tool.name.clone(),
        tool.description.clone(),
        Arc::new(schema),
    );
    if tool.read_only {
        listed.annotations = Some(ToolAnnotations::default().read_only(true));
    }
    listed
}

impl ServerHandler for OxplowMcp {
    fn get_info(&self) -> ServerConfig {
        // `ServerConfig` (née `ServerInfo`, renamed in rmcp 3) is
        // #[non_exhaustive], so it can't be built with struct-expression
        // syntax (not even with a `..Default::default()` tail). Start from
        // the default and assign.
        let mut info = ServerConfig::default();
        info.instructions = Some(
            "Oxplow knows this project's work and code: work items, efforts, \
             test runs and coverage, commits, metrics and the wiki, readable \
             with `oxplow_query_sql` over the read-only `v_*` models (`v_model` lists \
             them). Every write is a command (`run_command`).\n\n\
             Answering the person: when they ask to see, list, compare or \
             track something about the work or the code (\"show me\", \
             \"which files\", \"how well\"), answer from oxplow's data and \
             show it with `oxplow_show_lens` (an existing lens with params, or your \
             own `spec`: a table, chart or tree). It lands beside the \
             conversation, live, with Keep This. Keep the prose around it \
             short, and don't repeat the table as text."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    // These three methods replace what `#[tool_handler]` would generate:
    // the router holds only core's `list_commands` and `run_command`, and
    // every other tool is one the enabled extensions declare
    // (`oxplow_app::agent_tools`), listed and resolved per caller. rmcp 3's
    // cache hints mirror the macro's output (rmcp-macros
    // `tool_handler.rs`); re-diff against it on an rmcp bump.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        let caller = caller_of(&context.extensions);
        let mut tools = self.tool_router.list_all();
        tools.extend(self.declared(&caller).await?.iter().map(mcp_tool));
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            // Which tools a caller is offered depends on who it is.
            cache_scope: supports_cache_hints.then_some(CacheScope::Private),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if self.tool_router.has_route(&request.name) {
            let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
            return self.tool_router.call(tcc).await;
        }
        let caller = caller_of(&context.extensions);
        let input = serde_json::Value::Object(request.arguments.unwrap_or_default());
        self.call_declared(&caller, &request.name, input)
            .await
            .map(Into::into)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// The tools core builds: the command bus itself. Every other tool is an
/// extension's declaration (`oxplow_app::agent_tools`).
pub fn core_tool_names() -> Vec<String> {
    OxplowMcp::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.into_owned())
        .collect()
}

fn internal<E: std::fmt::Display>(e: E) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

impl OxplowMcp {
    /// The tools the enabled extensions declare that `caller` is offered.
    async fn declared(&self, caller: &McpCaller) -> Result<Vec<AgentTool>, McpError> {
        oxplow_app::agent_tools::offered(&self.services, &caller.actor())
            .await
            .map_err(internal)
    }

    /// Call the declared tool `name` as `caller`: a run of its command (a
    /// lens tool's is `oxplow.lens.run`), whose result is the tool's — text
    /// as is, anything else as JSON. An anonymous connection may call a
    /// read-only one; one that waits for a person returns the proposal.
    pub async fn call_declared(
        &self,
        caller: &McpCaller,
        name: &str,
        mut input: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        let tool = self
            .declared(caller)
            .await?
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("no tool `{name}` is offered to you; `tools/list` lists them"),
                    None,
                )
            })?;
        lenient_params::fold_to_schema(&mut input, &tool.input_schema);
        let (command, input) = tool.call(input);
        let actor = if caller.thread_id.is_some() || !tool.read_only {
            self.verified_actor(caller).await?
        } else {
            caller.actor()
        };
        let outcome = match self
            .services
            .commands
            .run(&actor, &command, input, false)
            .await
        {
            Ok(outcome) => outcome,
            Err(oxplow_domain::CommandError::Proposed {
                proposal,
                preview,
                supersedes,
            }) => {
                return json_result(&serde_json::json!({
                    "kind": "proposed",
                    "proposal": proposal,
                    "message": proposed_message(&preview.command, &proposal, &supersedes),
                }));
            }
            Err(err) => return Err(command_error(err)),
        };
        if outcome.audit_id.is_some() {
            self.services.efforts.settle_lifecycle().await;
        }
        match outcome.result {
            serde_json::Value::String(text) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            other => json_result(&other),
        }
    }

    /// `run_command` for a known caller. Refuses an anonymous connection:
    /// a write with no actor behind it is not audited to anyone.
    pub async fn run_command_as(
        &self,
        caller: &McpCaller,
        params: RunCommandParams,
    ) -> Result<CallToolResult, McpError> {
        let actor = self.verified_actor(caller).await?;
        let outcome = match self
            .services
            .commands
            .run(&actor, &params.id, params.input, false)
            .await
        {
            Ok(outcome) => outcome,
            // Kept for a person: the agent's request is recorded and needs
            // nothing more from it.
            Err(oxplow_domain::CommandError::Proposed {
                proposal,
                preview,
                supersedes,
            }) => {
                // `kind` tells it apart from a run's outcome.
                return json_result(&serde_json::json!({
                    "kind": "proposed",
                    "proposal": proposal,
                    "message": proposed_message(&preview.command, &proposal, &supersedes),
                }));
            }
            Err(err) => return Err(command_error(err)),
        };
        // A write may have opened or closed an effort (`oxplow.effort.open`,
        // `oxplow.work_item.transition`): let its snapshot pin land before the
        // agent's next step.
        if outcome.audit_id.is_some() {
            self.services.efforts.settle_lifecycle().await;
        }
        json_result(&outcome)
    }

    async fn verified_actor(&self, caller: &McpCaller) -> Result<oxplow_domain::Actor, McpError> {
        use oxplow_domain::stores::ThreadStore as _;
        let Some(thread_id) = caller.thread_id else {
            return Err(McpError::invalid_params(ANONYMOUS_WRITE, None));
        };
        let thread = self
            .services
            .thread_store
            .get(&thread_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("unknown thread `{thread_id}` in this connection's identity"),
                    None,
                )
            })?;
        if let Some(stream) = caller.stream_id {
            if stream != thread.stream_id {
                return Err(McpError::invalid_params(
                    format!(
                        "thread `{thread_id}` belongs to stream `{}`, not `{stream}`",
                        thread.stream_id
                    ),
                    None,
                ));
            }
        }
        Ok(oxplow_domain::Actor::Agent {
            session_id: caller.session_id,
            thread_id: Some(thread_id),
            stream_id: Some(thread.stream_id),
        })
    }
}

/// A command-bus refusal as the MCP error the agent can act on: invalid
/// input and denials are the caller's to fix; a needed confirmation says
/// so; a handler failure is internal.
fn command_error(err: oxplow_domain::CommandError) -> McpError {
    use oxplow_domain::CommandError as E;
    match &err {
        E::Unknown { .. } | E::Invalid { .. } | E::Denied { .. } => {
            McpError::invalid_params(err.to_string(), None)
        }
        E::NeedsConfirmation { preview } => McpError::invalid_params(
            format!(
                "`{}` needs a person's confirmation; ask them to run it (input: {})",
                preview.command, preview.input
            ),
            None,
        ),
        E::Proposed {
            proposal,
            preview,
            supersedes,
        } => McpError::invalid_params(
            proposed_message(&preview.command, proposal, supersedes),
            None,
        ),
        E::Failed { .. } | E::Unavailable { .. } | E::Busy { .. } => internal(err.to_string()),
    }
}

/// What an agent is told when its run waits for a person (`proposal:N`).
fn proposed_message(command: &str, proposal: &str, supersedes: &[String]) -> String {
    let replaces = if supersedes.is_empty() {
        String::new()
    } else {
        format!(
            " It replaces {}, which no longer waits.",
            supersedes.join(", ")
        )
    };
    format!(
        "`{command}` needs a person's approval; it is recorded as {proposal} and waits for \
         them in this thread and in Alerts (and on the setting's row in Settings).{replaces} \
         Tell the person; don't run it again. To see what they decided, read its `decision` \
         from `v_command_proposal` (`WHERE ref = '{proposal}'`)."
    )
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

/// Convenience wrapper: spawn the server on stdio.
pub async fn serve_stdio(services: Arc<Services>) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;
    let server = OxplowMcp::new(services);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "a test seeds the database through its stores"
    )]

    use super::*;
    use oxplow_app::commands::agent_reads::ai::AI_TEXT_MAX;

    async fn boot() -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        boot_with_config("").await
    }

    /// `boot().await` with `project_yaml` written to `.oxplow/project.yaml`
    /// first, so the booted `Services` picks it up (e.g. a
    /// `generated:` block that shapes the workspace filter).
    async fn boot_with_config(project_yaml: &str) -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        let project = tempfile::tempdir().unwrap();
        if !project_yaml.is_empty() {
            std::fs::create_dir_all(project.path().join(".oxplow")).unwrap();
            std::fs::write(project.path().join(".oxplow/project.yaml"), project_yaml).unwrap();
        }
        // ensure_primary requires a real git repo.
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let services = Services::in_memory(project.path()).await.unwrap();
        let server = OxplowMcp::new(services.clone());
        (project, services, server)
    }

    /// Pull the first text block out of an MCP CallToolResult. Most
    /// of our handlers return a single JSON-encoded blob.
    fn text_payload(result: CallToolResult) -> String {
        for c in &result.content {
            if let Some(text) = c.as_text() {
                return text.text.clone();
            }
        }
        panic!("CallToolResult had no text content");
    }

    /// Call the declared tool `name` as the connection `ext` stands for.
    async fn call(
        server: &OxplowMcp,
        ext: &rmcp::model::Extensions,
        name: &str,
        input: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        server.call_declared(&caller_of(ext), name, input).await
    }

    /// A task filed as a person through `oxplow.work_item.create` — on
    /// `thread`'s list, else the backlog: its id.
    async fn file_task(
        services: &Services,
        thread: Option<oxplow_domain::ThreadId>,
        title: &str,
    ) -> oxplow_tasks::TaskId {
        let mut input = serde_json::json!({ "title": title });
        if let Some(t) = thread {
            input["thread"] = serde_json::Value::String(t.to_string());
        }
        let out = services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                input,
                false,
            )
            .await
            .unwrap();
        oxplow_tasks::task_of_work_item_ref(out.result["ref"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn server_constructs() {
        let (_proj, _svc, _server) = boot().await;
    }

    #[tokio::test]
    async fn get_open_page_reports_what_the_human_sees() {
        use oxplow_domain::stores::ThreadStore;
        let (proj, services, server) = boot().await;
        let stream = services.streams.list_streams().await.unwrap()[0].clone();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()[0]
            .id;
        let tid = thread.to_string();

        let none: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::default(),
                "oxplow_get_open_page",
                serde_json::json!({ "thread_id": tid.clone() }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(none["open"], serde_json::Value::Null);

        let p = proj.path().join("oxplow/extensions/demo/lenses");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            proj.path().join("oxplow/extensions/demo/extension.yaml"),
            "manifest: 2\nname: demo\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            p.join("kinds.yaml"),
            "title: Kinds\nparams:\n  - { name: kind, default: worktree }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        )
        .unwrap();
        services
            .extension_activation
            .activate(services.clone())
            .await
            .unwrap();
        services.thread_runtime.set_open_page(
            &thread,
            Some(oxplow_app::thread_runtime::OpenPage {
                page_id: "lens:demo/kinds".into(),
                kind: "lens".into(),
                detail_json: Some(r#"{"lensId":"demo/kinds","params":{"kind":"primary"}}"#.into()),
                reported_at: oxplow_domain::Timestamp::now(),
            }),
        );
        let open: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::default(),
                "oxplow_get_open_page",
                serde_json::json!({ "thread_id": tid }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(open["open"]["pageId"], "lens:demo/kinds");
        assert_eq!(open["open"]["detail"]["params"]["kind"], "primary");
        // The lens is re-run with the human's params, not the defaults,
        // and read as its text.
        assert_eq!(open["lens"]["params"]["kind"], "primary");
        assert_eq!(open["lens"]["rowCount"], 1);
        assert!(
            open["lens"]["text"]
                .as_str()
                .unwrap()
                .contains("| primary |"),
            "{}",
            open["lens"]
        );
        assert!(open["lens"].get("result").is_none());
    }

    #[tokio::test]
    async fn extension_and_lens_tools() {
        let (proj, services, server) = boot().await;
        let root = proj.path();
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w(
            "oxplow/extensions/demo/extension.yaml",
            "manifest: 2\nname: demo\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: effort.review.details, lens: review }\n",
        );
        w(
            "oxplow/extensions/demo/lenses/review.yaml",
            "title: Review\nparams:\n  - { name: effort_id }\nquery: SELECT :effort_id AS effort\n",
        );
        w(
            "oxplow/extensions/demo/lenses/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        );
        w(
            "oxplow/extensions/demo/lenses/broken.yaml",
            "title: Broken\nquery: SELECT x FROM v_nope\n",
        );
        services
            .extension_activation
            .activate(services.clone())
            .await
            .unwrap();

        // A lens's definition is what `read` gives for its ref.
        let read: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::new(),
                "oxplow_read",
                serde_json::json!({ "ref": "lens:demo/streams" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        let lens: serde_json::Value =
            serde_json::from_str(read["content"].as_str().unwrap()).unwrap();
        assert!(lens["query"].as_str().unwrap().contains("v_stream"));

        let run = |format: Option<&str>| {
            let mut input = serde_json::json!({
                "id": "demo/streams",
                "params": { "kind": "primary" },
            });
            if let Some(format) = format {
                input["format"] = serde_json::json!(format);
            }
            let server = &server;
            async move {
                serde_json::from_str::<serde_json::Value>(&text_payload(
                    call(
                        server,
                        &rmcp::model::Extensions::new(),
                        "oxplow_run_lens",
                        input,
                    )
                    .await
                    .unwrap(),
                ))
                .unwrap()
            }
        };
        // By default an agent reads the text rendering, not the rows.
        let text = run(None).await;
        assert!(text.get("result").is_none(), "{text}");
        assert_eq!(text["rowCount"], 1);
        assert!(
            text["text"].as_str().unwrap().contains("| primary |"),
            "{text}"
        );
        let json = run(Some("json")).await;
        assert_eq!(json["result"]["rows"], serde_json::json!([["primary"]]));

        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_read",
            serde_json::json!({ "ref": "lens:demo/nope" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("lens:demo/nope"), "{err:?}");

        let v: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::new(),
                "oxplow_validate_extension",
                serde_json::json!({ "name": "demo" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert!(
            v["errors"][0].as_str().unwrap().contains("demo/broken"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn semantic_layer_query_and_schema_tools() {
        let (_proj, _services, server) = boot().await;
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::default(),
                "oxplow_query_sql",
                serde_json::json!({
                    "sql": "SELECT kind FROM v_stream WHERE kind = ?1",
                    "params": ["primary"],
                    "limit": 5,
                }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["columns"], serde_json::json!(["kind"]));
        assert_eq!(out["rows"], serde_json::json!([["primary"]]));

        let err = call(
            &server,
            &rmcp::model::Extensions::default(),
            "oxplow_query_sql",
            serde_json::json!({ "sql": "DELETE FROM task" }),
        )
        .await
        .unwrap_err();
        assert!(
            err.message.contains("read-only") || err.message.contains("SELECT"),
            "{err:?}"
        );

        // The catalog is SQL too: every model's columns are documented.
        let cols: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::default(),
                "oxplow_query_sql",
                serde_json::json!({
                    "sql": "SELECT name FROM v_model_column WHERE view = 'v_work_item'",
                }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert!(cols["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[0] == "state"));
    }

    #[tokio::test]
    async fn list_comments_enriches_primary_and_context() {
        let (_proj, services, server) = boot().await;
        let stream = services.streams.list_streams().await.unwrap()[0].id;

        // A task to use as the primary target, and another as a context
        // ancestor (e.g. an epic the row sat under).
        let primary_task = file_task(&services, None, "Primary item").await;
        let parent_task = file_task(&services, None, "Parent epic").await;

        services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.knowledge.add_comment",
                serde_json::json!({
                    "stream": format!("stream:{stream}"),
                    "target": { "kind": "work_item", "id": format!("oxplow:{primary_task}") },
                    "quote": "the highlighted text",
                    "selectors_json": "[]",
                    "context_chain": [{ "kind": "work_item", "id": format!("oxplow:{parent_task}") }],
                    "referenced_refs": [{ "kind": "file", "id": "src/app.rs" }],
                    "intent": "followup",
                    "body": "what about this?",
                }),
                false,
            )
            .await
            .unwrap();

        let r = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_list_comments",
            serde_json::json!({ "scope": "stream", "id": stream.to_string() }),
        )
        .await
        .unwrap();
        let body = text_payload(r);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let row = &parsed.as_array().unwrap()[0];

        // Primary target resolved to the task title.
        assert_eq!(row["primary"]["kind"], "work_item");
        assert_eq!(row["primary"]["title"], "Primary item");
        // Context chain ancestor resolved.
        assert_eq!(row["context_chain"][0]["title"], "Parent epic");
        // Referenced file ref present but bare (no first-class label).
        assert_eq!(row["referenced"][0]["kind"], "file");
        assert_eq!(row["referenced"][0]["id"], "src/app.rs");
        assert!(row["referenced"][0]["title"].is_null());
        // The raw thread still travels under `thread`.
        assert_eq!(row["thread"]["comment"]["quote"], "the highlighted text");
    }

    #[tokio::test]
    async fn lsp_no_config_error_is_self_describing_for_agents() {
        let (_proj, services, server) = boot().await;
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_code_hover",
            serde_json::json!({
                "stream_id": stream_id,
                "path": "src/x.rs",
                "line": 1,
                "col": 1,
            }),
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("rust-analyzer"), "got: {msg}");
        assert!(msg.contains("oxplow.lsp.install_server"), "got: {msg}");
        assert!(msg.contains("project.yaml"), "got: {msg}");
    }

    #[tokio::test]
    async fn get_info_advertises_tool_capability() {
        let (_proj, _svc, server) = boot().await;
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }

    /// Asked to "show me", an agent printed text. The
    /// instructions every harness reads say to show an answer in the
    /// person's thread.
    #[tokio::test]
    async fn get_info_instructions_say_to_show_answers_in_the_thread() {
        let (_proj, _svc, server) = boot().await;
        let instructions = server.get_info().instructions.unwrap_or_default();
        assert!(instructions.contains("oxplow_show_lens"), "{instructions}");
        assert!(instructions.contains("oxplow_query_sql"), "{instructions}");
    }

    #[tokio::test]
    async fn get_info_instructions_have_no_repo_specific_context_path() {
        // The instructions ship to every downstream agent — they must not
        // point at this repo's own `.context/` docs, which don't exist
        // in a user's project.
        let (_proj, _svc, server) = boot().await;
        let info = server.get_info();
        let instructions = info.instructions.unwrap_or_default();
        assert!(
            !instructions.contains(".context/"),
            "leaked repo path: {instructions}"
        );
    }

    /// An agent that can't discover skill files reads them by name
    /// ; it's read-only.
    #[tokio::test]
    async fn get_skill_returns_a_skill_body() {
        let (_proj, _svc, server) = boot().await;
        let anon = rmcp::model::Extensions::default();
        let r = call(
            &server,
            &anon,
            "oxplow_get_skill",
            serde_json::json!({ "name": "oxplow-extension" }),
        )
        .await
        .unwrap();
        assert!(text_payload(r).contains("# Building oxplow lenses"));
        let err = call(
            &server,
            &anon,
            "oxplow_get_skill",
            serde_json::json!({ "name": "nope" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("oxplow-extension"), "{err:?}");
        // A skill's reference files are read by name too.
        let core = oxplow_app::capabilities::agent_text(&_svc);
        let skill = core
            .skills
            .iter()
            .find(|s| !s.references.is_empty())
            .expect("a skill with reference files");
        let reference = &skill.references[0];
        let r = call(
            &server,
            &anon,
            "oxplow_get_skill",
            serde_json::json!({ "name": skill.name, "reference": reference.name }),
        )
        .await
        .unwrap();
        assert_eq!(text_payload(r), reference.body);
        let err = call(
            &server,
            &anon,
            "oxplow_get_skill",
            serde_json::json!({ "name": skill.name, "reference": "nope" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains(&reference.name), "{err:?}");
    }

    /// An agent previews a source from its stream's worktree: rows back,
    /// nothing stored.
    #[tokio::test]
    async fn preview_collector_returns_rows_without_storing() {
        let (proj, svc, server) = boot().await;
        let ext = proj.path().join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nintent:\n  purpose: test\ncollectors:\n  - id: one\n    runtime: jaq\n    entry: one.jq\n    input: \"SELECT 1 AS n\"\n    entities:\n      - { name: nums, key: n, columns: { n: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("one.jq"), "{entities: {nums: .rows}}").unwrap();
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        let r = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_preview_collector",
            serde_json::json!({ "owner": "work", "id": "one" }),
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(v["entities"][0]["rows"], serde_json::json!([[1]]));
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
    }

    /// A fact collector previews through the fact engine: the facts it
    /// would record over the stream's latest snapshot, nothing stored.
    #[tokio::test]
    async fn preview_collector_returns_a_fact_collectors_facts() {
        let (proj, svc, server) = boot().await;
        let ext = proj.path().join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nintent:\n  purpose: test\ncollectors:\n  - id: files\n    runtime: starlark\n    entry: files.star\n    needs: [tree.read]\n    facts: [work.files]\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("files.star"),
            "def transform(x):\n    return {\"facts\": [{\"measure\": \"work.files\", \"value\": len(files(\"**\")), \"subject\": ref.build(\"file\", \"src/a.rs\")}]}\n",
        )
        .unwrap();
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        let primary = svc.streams.list_streams().await.unwrap()[0].id;
        let snap = svc.snapshot_store.create_snapshot(primary).await.unwrap();
        svc.snapshot_store
            .capture_batch(vec![oxplow_db::FileSnapshot {
                id: 0,
                stream_id: primary,
                path: "src/a.rs".into(),
                blob_hash: Some(svc.blobs.write(b"fn a() {}\n").unwrap()),
                size_bytes: 10,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: None,
                content_hash: None,
            }])
            .await
            .unwrap();
        let r = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_preview_collector",
            serde_json::json!({ "owner": "work", "id": "files" }),
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(v["facts"][0]["measure"], "work.files", "{v}");
        assert_eq!(v["facts"][0]["value"], 1, "{v}");
        assert_eq!(v["facts"][0]["subject"], "file:src/a.rs", "{v}");
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
    }

    /// `read` reads a work item from the active list, whichever it is: the
    /// item while oxplow's tasks are the list, nothing with none.
    #[tokio::test]
    async fn a_work_item_reads_from_the_active_list() {
        let (_proj, services, server) = boot().await;
        let id = file_task(&services, None, "round trip").await;
        let read = |server: &OxplowMcp| {
            let server = server.clone();
            async move {
                call(
                    &server,
                    &rmcp::model::Extensions::new(),
                    "oxplow_read",
                    serde_json::json!({ "ref": format!("work_item:oxplow:{id}") }),
                )
                .await
                .map(|r| serde_json::from_str::<serde_json::Value>(&text_payload(r)).unwrap())
            }
        };
        assert_eq!(read(&server).await.unwrap()["row"]["title"], "round trip");
        services
            .config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), "none".into());
        let config = oxplow_app::config_service::read_config(&services.config);
        services
            .capabilities
            .publish(&config, &services.db)
            .await
            .unwrap();
        let gone = read(&server).await.unwrap_err();
        assert!(gone.message.contains("nothing at"), "{gone:?}");
    }

    /// A request's parts as the control plane passes them on: the
    /// session's principal in its extensions, plus whatever `headers` it
    /// carried (which name nobody).
    fn parts_as(
        thread: oxplow_domain::ThreadId,
        stream: oxplow_domain::StreamId,
        headers: &[(&str, &str)],
    ) -> http::request::Parts {
        let mut b = http::Request::builder().uri("http://h/mcp");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut parts = b.body(()).unwrap().into_parts().0;
        parts.extensions.insert(Principal {
            session: oxplow_domain::AgentSessionId::new(1),
            thread,
            stream,
            harness: "claude".into(),
        });
        parts
    }

    /// The identity of the primary stream's writer thread — what oxplow's
    /// harness configs send, and what a status-changing tool needs.
    async fn as_writer(services: &oxplow_app::Services) -> rmcp::model::Extensions {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        extensions_for(parts_as(thread.id, stream.id, &[]))
    }

    fn extensions_for(parts: http::request::Parts) -> rmcp::model::Extensions {
        let mut ext = rmcp::model::Extensions::new();
        ext.insert(parts);
        ext
    }

    #[test]
    fn mcp_caller_is_the_bearers_session_whatever_the_request_says() {
        let c = McpCaller::from_parts(&parts_as(
            oxplow_domain::ThreadId::new(3),
            oxplow_domain::StreamId::new(2),
            &[("x-oxplow-thread", "thr9"), ("x-oxplow-stream", "str9")],
        ));
        assert_eq!(c.session_id, Some(oxplow_domain::AgentSessionId::new(1)));
        assert_eq!(c.thread_id, Some(oxplow_domain::ThreadId::new(3)));
        assert_eq!(c.stream_id, Some(oxplow_domain::StreamId::new(2)));
        assert_eq!(c.actor().source(), "agent:thr3");
        // Headers or a query alone, with no principal, are nobody.
        let bare = http::Request::builder()
            .uri("http://h/mcp?thread=thr5&stream=str1")
            .header("x-oxplow-thread", "thr5")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert_eq!(McpCaller::from_parts(&bare), McpCaller::default());
        assert_eq!(
            caller_of(&rmcp::model::Extensions::new()),
            McpCaller::default()
        );
    }

    #[tokio::test]
    async fn run_command_refuses_an_anonymous_connection_and_lists_for_agents() {
        let (_proj, _services, server) = boot().await;
        let err = server
            .run_command_as(
                &McpCaller::default(),
                RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({"key": "zones", "value": []}),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        // P4.8: recording a metric is a command like any write — refused
        // without an identity, and no fact lands.
        let err = server
            .run_command_as(
                &McpCaller::default(),
                RunCommandParams {
                    id: "oxplow.metric.record".into(),
                    input: serde_json::json!({"key": "oxplow.rust.unsafe_blocks", "value": 1.0}),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        // Listing needs no identity and shows agent-invocable commands.
        let specs = listed(&server, ListCommandsParams::default())
            .await
            .unwrap();
        let names: Vec<&str> = specs.iter().map(|s| s["id"].as_str().unwrap()).collect();
        for expected in [
            "oxplow.config.set",
            "oxplow.config.list_keys",
            "oxplow.work_item.transition",
            "oxplow.metric.record",
            "oxplow.collector.sync",
            "oxplow.metric.rebuild",
            "oxplow.metric.scaffold",
        ] {
            assert!(names.contains(&expected), "{names:?}");
        }
        assert!(specs.iter().all(|s| s["summary"].is_string()));
    }

    /// `list_commands` with no arguments: every command's id and summary,
    /// nothing more.
    async fn listed(
        server: &OxplowMcp,
        params: ListCommandsParams,
    ) -> Result<Vec<serde_json::Value>, McpError> {
        let result = server
            .list_commands(rmcp::model::Extensions::new(), Parameters(params))
            .await?;
        let text = result.content[0].as_text().unwrap().text.clone();
        Ok(serde_json::from_str(&text).unwrap())
    }

    /// An agent discovers a command in two small steps instead of one huge
    /// one: the index (ids and summaries), then one command — or one
    /// namespace — with its schema, its example input, whether a person
    /// confirms it and whether it undoes. The person-facing parts of a
    /// spec (`ui`, `op`, `atomicity`, …) never reach it.
    #[tokio::test]
    async fn list_commands_is_an_index_then_one_command_with_its_example() {
        let (_proj, _services, server) = boot().await;
        let index = listed(&server, ListCommandsParams::default())
            .await
            .unwrap();
        for entry in &index {
            let mut keys: Vec<&String> = entry.as_object().unwrap().keys().collect();
            keys.sort();
            assert_eq!(keys, ["id", "summary"], "{entry}");
        }
        let size = serde_json::to_string(&index).unwrap().len();
        // Fetched on demand, not every turn: ~4K tokens (every full spec was 164 KB).
        assert!(size < 17_000, "the index is {size} bytes");
        // A summary is cut at its first sentence; the command shows it whole.
        assert_eq!(
            first_sentence("Open a slot (`thread:thr1`). It starts nothing, e.g. no process."),
            "Open a slot (`thread:thr1`)."
        );
        assert_eq!(first_sentence("One sentence"), "One sentence");

        let one = listed(
            &server,
            ListCommandsParams {
                id: Some("oxplow.config.get".into()),
                prefix: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(one.len(), 1);
        let mut keys: Vec<&String> = one[0].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "confirm",
                "example",
                "id",
                "input_schema",
                "summary",
                "undoable"
            ]
        );
        assert!(one[0]["example"].is_object(), "{}", one[0]);
        // The example is shown once, not again inside the schema.
        assert!(one[0]["input_schema"].get("examples").is_none());

        let area = listed(
            &server,
            ListCommandsParams {
                id: None,
                prefix: Some("oxplow.config.".into()),
            },
        )
        .await
        .unwrap();
        let ids: Vec<&str> = area.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"oxplow.config.set"), "{ids:?}");
        assert!(
            ids.iter().all(|id| id.starts_with("oxplow.config.")),
            "{ids:?}"
        );
        assert!(area.iter().all(|s| s["input_schema"].is_object()));

        // An id it doesn't offer says so and names the ones that end alike.
        let err = listed(
            &server,
            ListCommandsParams {
                id: Some("config.get".into()),
                prefix: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("oxplow.config.get"), "{}", err.message);
    }

    /// The thread header is a claim, not a proof: an unknown thread, or a
    /// stream header that isn't the thread's stream, is refused, and
    /// `transition_tasks` needs an identity like `run_command` does.
    #[tokio::test]
    async fn commands_refuse_unknown_or_mismatched_callers() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot().await;
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let list = || RunCommandParams {
            id: "oxplow.config.list_keys".into(),
            input: serde_json::json!({}),
        };
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(oxplow_domain::ThreadId::new(999)),
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown thread"), "{err}");
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: Some(oxplow_domain::StreamId::new(999)),
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("belongs to"), "{err}");
        server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap();
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: None,
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
    }

    /// The zones write path after `set_zones`: the agent runs
    /// `oxplow.config.set` through `run_command`, and the run is audited to the
    /// calling thread.
    #[tokio::test]
    async fn run_command_sets_zones_as_the_calling_thread() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (proj, services, server) = boot().await;
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let parts = parts_as(thread.id, stream.id, &[]);
        let out = server
            .run_command(
                extensions_for(parts),
                Parameters(RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({
                        "key": "zones",
                        "value": [{"match": "src/**", "zone": "core"}]
                    }),
                }),
            )
            .await
            .unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        let outcome: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(outcome["result"]["changed"], true);
        assert!(outcome["audit_id"].is_number());
        let yaml = std::fs::read_to_string(proj.path().join(".oxplow/project.yaml")).unwrap();
        assert!(yaml.contains("zone: core"), "{yaml}");
        assert_eq!(services.config.read().unwrap().zones.len(), 1);
        let events = services.event_log_store.read_after(0, 20).await.unwrap();
        let executed = events
            .iter()
            .find(|e| e.envelope.event_type == "command.executed")
            .expect("command.executed logged");
        assert_eq!(executed.envelope.source, format!("agent:{}", thread.id));
        assert_eq!(executed.envelope.anchors.thread_id, Some(thread.id));
        assert_eq!(executed.envelope.payload["command"], "oxplow.config.set");
        // A human-only key is not the agent's to set: the run is kept as a
        // proposal for a person, and the agent is told so — a success,
        // since the request is recorded and needs nothing more from it.
        let out = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: Some(stream.id),
                },
                RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({"key": "agentPromptAppend", "value": "be brief"}),
                },
            )
            .await
            .unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        let proposed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(proposed["kind"], "proposed");
        let proposal = proposed["proposal"].as_str().unwrap().to_string();
        assert!(proposal.starts_with("proposal:"), "{proposed}");
        let message = proposed["message"].as_str().unwrap();
        assert!(message.contains(&proposal), "{message}");
        assert!(message.contains("Alerts"), "{message}");
        assert!(message.contains("in this thread"), "{message}");
        assert!(message.contains("v_command_proposal"), "{message}");
        assert!(message.contains("don't run it again"), "{message}");
        let pending = services
            .commands
            .proposal_store()
            .list_pending()
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].dry_run.as_ref().unwrap()["after"], "be brief");
        let yaml = std::fs::read_to_string(proj.path().join(".oxplow/project.yaml")).unwrap();
        assert!(!yaml.contains("be brief"), "nothing written: {yaml}");
    }

    /// A thread on `stream`, made as a person through `oxplow.thread.create`.
    async fn new_thread(
        services: &oxplow_app::Services,
        stream: oxplow_domain::StreamId,
        title: &str,
    ) -> oxplow_domain::Thread {
        let out = services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                oxplow_app::commands::thread::CREATE,
                serde_json::json!({
                    "stream": oxplow_domain::refs::build::stream_ref(stream),
                    "title": title,
                }),
                false,
            )
            .await
            .unwrap();
        serde_json::from_value(out.result).unwrap()
    }

    /// `read` reads a file in the calling session's stream, not the
    /// primary's.
    #[tokio::test]
    async fn read_reads_files_in_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (proj, services, server) = boot().await;
        let primary = services.stream_store.list().await.unwrap().pop().unwrap();
        std::fs::write(proj.path().join("who.txt"), "primary").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("who.txt"), "mine").unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = services.stream_store.upsert(&other).await.unwrap();
        let thread = new_thread(&services, other_id, "t").await;
        let out = call(
            &server,
            &extensions_for(parts_as(thread.id, other_id, &[])),
            "oxplow_read",
            serde_json::json!({ "ref": "file:who.txt" }),
        )
        .await
        .unwrap();
        let read: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(read["content"], "mine");
        // Naming a stream reads its worktree instead.
        let out = call(
            &server,
            &extensions_for(parts_as(thread.id, other_id, &[])),
            "oxplow_read",
            serde_json::json!({
                "ref": "file:who.txt",
                "stream": oxplow_domain::refs::build::stream_ref(primary.id),
            }),
        )
        .await
        .unwrap();
        let read: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(read["content"], "primary");
        let bad = call(
            &server,
            &extensions_for(parts_as(thread.id, other_id, &[])),
            "oxplow_read",
            serde_json::json!({ "ref": "file:who.txt", "stream": "str1" }),
        )
        .await
        .unwrap_err();
        assert!(bad.message.contains("stream:str2"), "{}", bad.message);
    }

    /// The extension and lens tools without a `stream_id` read
    /// the calling thread's stream — an agent in a worktree sees the
    /// extension it just wrote there, not the primary's.
    #[tokio::test]
    async fn validate_extension_defaults_to_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (_proj, services, server) = boot().await;
        let primary = services.stream_store.list().await.unwrap().pop().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let ext = elsewhere.path().join("oxplow/extensions/mine");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: mine\nintent:\n  purpose: test\n",
        )
        .unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = services.stream_store.upsert(&other).await.unwrap();
        let thread = new_thread(&services, other_id, "t").await;
        let caller = || extensions_for(parts_as(thread.id, other_id, &[]));
        call(
            &server,
            &caller(),
            "oxplow_validate_extension",
            serde_json::json!({ "name": "mine" }),
        )
        .await
        .unwrap();
    }

    /// P6.C1: an agent shows an answer in its thread; the tool returns its
    /// ref and what the person sees, as text.
    #[tokio::test]
    async fn show_lens_records_an_answer_and_returns_its_text() {
        let (_proj, services, server) = boot().await;
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &as_writer(&services).await,
                "oxplow_show_lens",
                serde_json::json!({
                    "spec": {
                        "title": "Streams",
                        "query": "SELECT kind FROM v_stream",
                    },
                }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert!(
            out["answer"].as_str().unwrap().starts_with("answer:"),
            "{out}"
        );
        assert!(
            out["text"].as_str().unwrap().contains("| primary |"),
            "{out}"
        );
        // Without a thread identity there's nowhere to show it.
        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_show_lens",
            serde_json::json!({ "lens": "x/y" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("thread identity"), "{err:?}");
    }

    /// Helper to write a wiki body to disk.
    async fn seed_wiki(project: &std::path::Path, slug: &str, body: &str) {
        let wiki_dir = project.join(".oxplow").join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(wiki_dir.join(format!("{slug}.md")), body).unwrap();
    }

    #[tokio::test]
    async fn wiki_ref_drift_reports_status_per_ref() {
        let (proj, svc, server) = boot().await;
        seed_wiki(proj.path(), "intro", "see [[crates/foo.rs]]").await;
        oxplow_app::wiki_pages::sync_page(&svc.db, &svc.vocabulary, proj.path(), "intro")
            .await
            .unwrap();
        // The file IS referenced but has no pin (no snapshot service in tests).
        let r = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_wiki_ref_drift",
            serde_json::json!({ "slug": "intro", "path": "crates/foo.rs" }),
        )
        .await
        .unwrap();
        let body = text_payload(r);
        assert!(body.contains("\"status\""), "{body}");
        assert!(body.contains("no_pin"), "expected no_pin: {body}");
        // A path the page doesn't reference at all.
        let r2 = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_wiki_ref_drift",
            serde_json::json!({ "slug": "intro", "path": "crates/zzz.rs" }),
        )
        .await
        .unwrap();
        assert!(text_payload(r2).contains("not_a_ref"));
    }

    // ---- Pure helpers: parse_status / parse_priority ----

    /// Point the in-memory services' `role` at a mock provider at `base`.
    fn assign_mock_role(services: &Services, base: String, role: &str) {
        use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
        use oxplow_app::exec_consent::{approve_program, list, ProgramKind};
        // The shipped providers are scripts a person approves first.
        let project = services.layout.project_dir.clone();
        let config = services.config.read().unwrap().clone();
        let extensions = services.extension_catalog.get(&project);
        for p in list(&services.approvals, &project, &config, &extensions) {
            if p.kind == ProgramKind::AiProvider {
                approve_program(
                    &services.approvals,
                    &project,
                    &config,
                    &extensions,
                    p.kind,
                    &p.name,
                    p.version.as_deref().unwrap(),
                )
                .unwrap();
            }
        }
        services
            .ai
            .save_provider(
                ProviderConfig {
                    id: "mock".into(),
                    kind: "openai_compatible".into(),
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        let role: Role = serde_json::from_value(serde_json::json!(role)).unwrap();
        services
            .ai
            .set_role(&role, Some(RoleBinding::new("mock", "m")))
            .unwrap();
    }

    fn risky() -> serde_json::Value {
        serde_json::json!({
            "risky": {
                "type": "choice",
                "instructions": "Is it risky?",
                "options": ["yes", "no"],
            },
        })
    }

    /// The model calls an agent asks for are paid with the person's key:
    /// each is a recorded computation, recorded as the asking thread — the
    /// same question twice is one call — on its tool's own role, with its
    /// text capped; a connection with no thread identity asks nothing.
    #[tokio::test]
    async fn ai_tools_decide_and_summarize() {
        let (_proj, services, server) = boot().await;
        let me = as_writer(&services).await;
        let thread = oxplow_domain::refs::build::thread_ref(caller_of(&me).thread_id.unwrap());
        // Unassigned role: a clear error, not a crash.
        let err = call(
            &server,
            &me,
            "oxplow_ai_summarize",
            serde_json::json!({ "text": "t" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("summarize"), "{}", err.message);

        let answer = serde_json::json!({"answers": {"risky": {"type": "choice", "choice": "yes", "probabilities": {"yes": 0.7, "no": 0.3}}}});
        let (base, seen) = oxplow_ai_fake::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": answer.to_string()}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        assign_mock_role(&services, base.clone(), "decide");
        for cached in [false, true] {
            let out: serde_json::Value = serde_json::from_str(&text_payload(
                call(
                    &server,
                    &me,
                    "oxplow_ai_decide",
                    serde_json::json!({ "state": "a diff", "questions": risky() }),
                )
                .await
                .unwrap(),
            ))
            .unwrap();
            assert_eq!(out["answers"]["risky"]["choice"], "yes");
            assert_eq!(out["cached"], cached);
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("Is it risky?"));

        let err = call(
            &server,
            &me,
            "oxplow_ai_decide",
            serde_json::json!({
                "state": "s",
                "questions": { "x": { "type": "maybe", "instructions": "?" } },
            }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("maybe"), "{}", err.message);
        let err = call(
            &server,
            &me,
            "oxplow_ai_decide",
            serde_json::json!({ "state": "x".repeat(AI_TEXT_MAX + 1), "questions": risky() }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("characters"), "{}", err.message);
        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_ai_decide",
            serde_json::json!({ "state": "a diff", "questions": risky() }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("thread identity"), "{}", err.message);

        let (base, seen) = oxplow_ai_fake::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": " short "}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        assign_mock_role(&services, base, "summarize");
        // A recorded computation: asking twice calls once.
        for _ in 0..2 {
            let out = text_payload(
                call(
                    &server,
                    &me,
                    "oxplow_ai_summarize",
                    serde_json::json!({ "text": "long", "focus": "risks" }),
                )
                .await
                .unwrap(),
            );
            assert_eq!(out, "short");
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("risks"));
        let callers = services
            .sql
            .query_sql("SELECT DISTINCT caller FROM v_ai_call", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&callers.rows).unwrap(),
            serde_json::json!([[thread]])
        );
    }

    /// A file's symbols come from its language server when one runs; with
    /// none, the tool reads the file's live content with oxplow's own
    /// tree-sitter analysis and says that's what answered.
    #[tokio::test]
    async fn code_symbols_fall_back_to_tree_sitter_and_say_so() {
        let (proj, services, server) = boot().await;
        std::fs::create_dir_all(proj.path().join("src")).unwrap();
        std::fs::write(
            proj.path().join("src/x.rs"),
            "fn alpha() {}\n\nstruct Beta;\n",
        )
        .unwrap();
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::new(),
                "oxplow_code_symbols",
                serde_json::json!({ "stream_id": stream_id, "path": "src/x.rs" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["source"], "tree_sitter");
        let names: Vec<_> = out["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"alpha".to_string()), "{out}");
        assert_eq!(out["symbols"][0]["startLine"], 1);
        // The caller's own stream when none is named.
        let again = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_code_symbols",
            serde_json::json!({ "path": "src/x.rs" }),
        )
        .await;
        assert!(again.is_ok(), "{again:?}");
    }

    /// A workspace query needs a running language server; without one the
    /// error says where the stored symbols are instead.
    #[tokio::test]
    async fn code_workspace_symbols_need_a_language_server_and_point_at_v_symbol() {
        let (_proj, services, server) = boot().await;
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_code_workspace_symbols",
            serde_json::json!({
                "stream_id": stream_id,
                "language": "rust",
                "query": "alpha",
            }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("v_symbol"), "{}", err.message);
    }

    /// Blame reads through the Vcs provider: each line of the working copy
    /// names the commit that last changed it, an uncommitted line none.
    #[tokio::test]
    async fn vcs_blame_names_the_commit_of_each_line() {
        let (proj, _services, server) = boot().await;
        std::fs::write(proj.path().join("b.txt"), "one\ntwo\n").unwrap();
        let repo = git2::Repository::open(proj.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("b.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "add b", &tree, &[&parent])
            .unwrap();
        std::fs::write(proj.path().join("b.txt"), "one\ntwo\nthree\n").unwrap();
        let lines: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::new(),
                "oxplow_vcs_blame",
                serde_json::json!({ "path": "b.txt" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        let lines = lines.as_array().unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].to_string().contains(&commit.to_string()),
            "{}",
            lines[0]
        );
        assert!(
            !lines[2].to_string().contains(&commit.to_string()),
            "{}",
            lines[2]
        );
    }

    #[tokio::test]
    async fn ensure_change_analyzes_a_commit_for_agents() {
        let (proj, _services, server) = boot().await;
        std::fs::write(proj.path().join("a.rs"), "fn a() {}\n").unwrap();
        let repo = git2::Repository::open(proj.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.rs")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "a", &tree, &[&parent])
            .unwrap();

        let out: serde_json::Value = serde_json::from_str(&text_payload(
            call(
                &server,
                &rmcp::model::Extensions::new(),
                "oxplow_ensure_change",
                serde_json::json!({ "kind": "commit", "sha": "HEAD" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["status"], "done");
        let err = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_ensure_change",
            serde_json::json!({ "kind": "sideways" }),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("sideways"), "{}", err.message);
    }

    /// The tools the extensions declare are offered with their hints: a
    /// read command's tool is hinted read-only and carries its command's
    /// input schema, and core builds only the command bus.
    #[tokio::test]
    async fn declared_tools_are_offered_with_their_hints() {
        let (_proj, _svc, server) = boot().await;
        let tools = server.declared(&McpCaller::default()).await.unwrap();
        let query = tools
            .iter()
            .find(|t| t.name == "oxplow_query_sql")
            .expect("oxplow_query_sql is declared");
        assert!(query.read_only);
        let show = tools
            .iter()
            .find(|t| t.name == "oxplow_show_lens")
            .expect("oxplow_show_lens is declared");
        assert!(!show.read_only);

        let listed = mcp_tool(query);
        assert_eq!(
            listed.annotations.and_then(|a| a.read_only_hint),
            Some(true)
        );
        assert!(listed.input_schema["properties"]["sql"].is_object());
        assert!(mcp_tool(show).annotations.is_none());

        let mut core = core_tool_names();
        core.sort();
        assert_eq!(core, ["list_commands", "run_command"]);
    }

    /// An anonymous connection may read through a declared tool, but a
    /// tool that writes refuses it, and an unknown name is no tool.
    #[tokio::test]
    async fn an_anonymous_caller_reads_but_does_not_write_through_tools() {
        let (_proj, _svc, server) = boot().await;
        let anon = McpCaller::default();
        server
            .call_declared(
                &anon,
                "oxplow_query_sql",
                serde_json::json!({ "sql": "SELECT 1 AS n" }),
            )
            .await
            .unwrap();
        let err = server
            .call_declared(
                &anon,
                "oxplow_show_lens",
                serde_json::json!({
                    "spec": { "title": "T", "query": "SELECT 1 AS n", "viz": "table" },
                }),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("carries no thread identity"),
            "{err:?}"
        );
        let err = server
            .call_declared(&anon, "oxplow_nope", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.message.contains("no tool `oxplow_nope`"), "{err:?}");
    }

    /// A client that spells a field in camelCase still reaches the
    /// declared tool: `stream_id` arrives as `streamId`.
    #[tokio::test]
    async fn a_camel_case_key_reaches_a_declared_tool() {
        let (proj, services, server) = boot().await;
        std::fs::create_dir_all(proj.path().join("src")).unwrap();
        std::fs::write(proj.path().join("src/x.rs"), "fn alpha() {}\n").unwrap();
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let out = call(
            &server,
            &rmcp::model::Extensions::new(),
            "oxplow_code_symbols",
            serde_json::json!({ "streamId": stream_id, "path": "src/x.rs" }),
        )
        .await
        .unwrap();
        let out: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(out["source"], "tree_sitter");
    }
}
