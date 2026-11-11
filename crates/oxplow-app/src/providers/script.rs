//! A script implementation of a capability (`.context/extensions-commands.md`
//! "Implementations"): an extension's `implementations:` entry whose `entry`
//! is a Starlark file of the extension, for any capability whose interface
//! is a set of verbs (its provider contract). [`ScriptEndpoint`] is the
//! [`Endpoint`] its capability's host adapts — the same host, adapter and
//! checks as an external provider's instance — so core calls it through
//! the one verb dispatch every implementation of the capability has
//! (`task.<verb>` → `TaskVerbs::invoke` for a task tracker).
//!
//! **The call.** Each verb is the script's function of that name, called
//! with one dict:
//!
//! ```python
//! def create(x):
//!     # x["input"]  the verb's input, as the capability's contract gives it
//!     # x["config"] the declaration's `config:` (a map; `{}` when none)
//!     # x["id"]     the implementation's id (its refs' `provider`)
//!     # x["key"]    the call's idempotency key (the caller's, or a fresh one)
//!     return {"result": {...}, "events": [...], "inverse": {...}}
//! ```
//!
//! It answers what a provider's `invoke` answers (`InvokeResult`): a
//! `result`, the `events` it recorded (`{ type, v, payload, subject? }`,
//! only its contract's types and only its own items) and an optional
//! `inverse` (`{ command: <verb>, input }`), each checked as an external
//! provider's are. It runs pure, in the command scripts' sandbox
//! (`COMMAND_SCRIPT_BUDGET`), as its own contribution
//! (`oxplow:oxplow/implementation?…`, kind `implementation`) under its
//! extension's permissions, given only the scopes its `needs` names — what
//! it keeps between calls it reads back through them (a task tracker's
//! own items through `sql.read` on `v_task`, recorded by the events it
//! answered).
//!
//! **Lifecycle.** Its adapter is prepared with the extension generation's
//! other runtime adapters (`PreparedRuntimeAdapters`) and published with
//! them; the next generation retires the ones it no longer declares.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::diagnostic::Diagnostic;
use oxplow_domain::events::Envelope;
use oxplow_domain::run::{ContributionRef, Mode, RunContext};
use oxplow_domain::tasks::StageDecl;
use oxplow_domain::{Actor, CommandError};
use oxplow_provider_protocol::model::{EventDraft, InitializeResult, InvokeResult};
use serde_json::{json, Value};

use super::endpoint::Endpoint;
use super::registry::{PreparedProviderCapability, ProviderRegistry};
use crate::extension::extension_catalog::{CatalogSnapshot, ExtensionCatalog};
use crate::extension::extensions::implementations::ImplementationDecl;
use crate::extension::extensions::Extension;
use crate::vocabulary_reactor::VocabularyErrors;

/// One script implementation, as its capability's adapter calls it.
pub struct ScriptEndpoint {
    id: String,
    /// `<namespace>/<id>`.
    name: String,
    extension: String,
    capability: String,
    declared: InitializeResult,
    stages: Vec<StageDecl>,
    id_pattern: Option<String>,
    text: Arc<str>,
    needs: Vec<String>,
    config: Value,
    /// What its extension may reach: its calls are decided within it.
    grant: oxplow_domain::grant::Grant,
    /// The contribution it runs as.
    owner: ContributionRef,
    sql: crate::sql_gateway::SqlGateway,
    catalog: Arc<ExtensionCatalog>,
    root: PathBuf,
}

impl ScriptEndpoint {
    /// `decl`, a script implementation `ext` declares; `None` for any other.
    fn of(
        ext: &Extension,
        decl: &ImplementationDecl,
        sql: &crate::sql_gateway::SqlGateway,
        catalog: &Arc<ExtensionCatalog>,
        root: &std::path::Path,
    ) -> Option<Self> {
        let script = decl.script.as_ref()?;
        let local = oxplow_domain::contribution::implementation_local(&decl.capability, &decl.id);
        Some(Self {
            id: decl.id.clone(),
            name: format!("{}/{}", ext.namespace, decl.id),
            extension: ext.name.clone(),
            capability: decl.capability.clone(),
            declared: script.declared.clone(),
            stages: script.spec.stages.clone(),
            id_pattern: script.spec.id_pattern.clone(),
            text: script.text.as_str().into(),
            needs: decl.needs.clone(),
            config: decl.config.clone(),
            grant: crate::grants::grant_of(ext),
            // It belongs to its extension's deployment, rechecked at each
            // scope call (a script may outlive a generation).
            owner: crate::run_context::unpinned_contribution(
                ext,
                oxplow_domain::contribution::ContributionKind::Implementation,
                &local,
            ),
            sql: sql.clone(),
            catalog: catalog.clone(),
            root: root.to_path_buf(),
        })
    }

    /// The run a verb's script runs as: a child of the caller's, as its own
    /// contribution, pure, with only its `needs` declared, within its
    /// extension's permissions, for as long as its extension stays live.
    fn run_of(&self, caller: &RunContext) -> RunContext {
        caller
            .child(Some(self.owner.clone()), Mode::Pure)
            .authorized(oxplow_domain::run::Authority {
                declared: crate::grants::script_request(&self.needs),
                grant: self.grant.clone(),
                enforcement: crate::grants::enforcement(),
            })
            .guarded(crate::run_context::live_owner(
                self.catalog.clone(),
                &self.root,
                self.owner.clone(),
            ))
    }
}

#[async_trait]
impl Endpoint for ScriptEndpoint {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn extension(&self) -> &str {
        &self.extension
    }

    fn declared(&self) -> &InitializeResult {
        &self.declared
    }

    fn stages(&self) -> &[StageDecl] {
        &self.stages
    }

    fn id_pattern(&self) -> Option<&str> {
        self.id_pattern.as_deref()
    }

    async fn invoke(
        &self,
        run: &RunContext,
        verb: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<InvokeResult, CommandError> {
        let failed = |message: String| CommandError::Failed {
            message: format!("script implementation `{}` `{verb}`: {message}", self.name),
        };
        if !self.declared.commands.iter().any(|c| c.name == verb) {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("script implementation `{}` defines no `{verb}`", self.name),
            });
        }
        let key =
            idempotency_key.unwrap_or_else(|| format!("call:{}", uuid::Uuid::new_v4().simple()));
        let run = self.run_of(run);
        // A revocation reaches a script that is computing.
        let _watch = crate::run_context::watch_authority(&run);
        let given = crate::services::decide(&run, &self.text, &self.needs)
            .and_then(crate::services::Wanted::scopes_only)
            .map_err(failed)?;
        let arg = json!({
            "input": input,
            "config": self.config,
            "id": self.id,
            "key": key,
        });
        let (text, needs, sql, func) = (
            self.text.clone(),
            self.needs.clone(),
            self.sql.clone(),
            verb.to_string(),
        );
        let runtime = tokio::runtime::Handle::current();
        let out = tokio::task::spawn_blocking(move || {
            let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(sql.run_in(&run, q)));
            let mut calls =
                crate::scope_calls::Calls::new(&run, &needs, read).with_host(Some(&sql), &runtime);
            let out = {
                let mut serve = |id: &str, args: Value| calls.serve(id, args);
                run_verb(&text, &func, arg, given, &mut serve, &run)
            };
            // The database was busy answering a read: the call is retried.
            if let Some(message) = calls.take_busy() {
                return Err(CommandError::Busy { message });
            }
            out.map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })
        })
        .await
        .map_err(|e| failed(format!("the script's worker failed: {e}")))?
        .map_err(|e| match e {
            CommandError::Failed { message } => failed(format!("the script failed: {message}")),
            other => other,
        })?;
        serde_json::from_value::<InvokeResult>(out).map_err(|e| {
            failed(format!(
                "it answered something other than `{{ result, events?, inverse? }}`: {e}"
            ))
        })
    }

    fn envelope(&self, actor: &Actor, draft: EventDraft) -> Result<Envelope, CommandError> {
        let contract = oxplow_domain::capability::contract(&self.capability);
        super::endpoint::envelope_of(
            &self.name,
            &self.id,
            &self.extension,
            &self.capability,
            |event_type, v| contract.is_some_and(|c| c.events.contains(&(event_type, v))),
            actor,
            draft,
        )
    }

    /// No process: nothing to end.
    async fn restart(&self) {}
}

/// Call `verb` — the script's function of that name — with `arg`, in the
/// command scripts' sandbox, its scope calls answered by `serve`.
fn run_verb(
    text: &str,
    verb: &str,
    arg: Value,
    given: crate::services::Given,
    serve: &mut oxplow_script::workers::Serve<'_>,
    run: &RunContext,
) -> Result<Value, oxplow_script::CollectError> {
    use oxplow_script::workers::{shared, ScriptRun};
    shared()
        .run(
            &crate::extension::extension_commands::COMMAND_SCRIPT_BUDGET,
            given.onto(ScriptRun::starlark_fn(text, verb, arg), Some(serve)),
            Some(run.cancel()),
        )
        .map(|ran| {
            run.used_cpu(ran.cpu_ms, ran.peak_bytes);
            ran.value
        })
}

/// The script implementations one catalog generation declares, each
/// adapted by its capability's host and not yet published.
pub struct PreparedScripts {
    adapters: Vec<((String, String), PreparedProviderCapability)>,
}

impl ProviderRegistry {
    /// Adapt every script implementation `snapshot`'s enabled extensions
    /// declare with its capability's host. A host that refuses one (its
    /// declarations don't make an implementation) holds the generation
    /// back, naming the extension.
    pub fn prepare_scripts(
        &self,
        snapshot: &CatalogSnapshot,
    ) -> Result<PreparedScripts, VocabularyErrors> {
        let mut adapters = Vec::new();
        let mut errors = VocabularyErrors::new();
        for ext in snapshot.extensions().iter().filter(|e| e.enabled) {
            for decl in &ext.implementations {
                let Some(endpoint) = ScriptEndpoint::of(
                    ext,
                    decl,
                    &self.deps.sql,
                    &self.deps.catalog,
                    &self.deps.project_dir,
                ) else {
                    continue;
                };
                let endpoint: Arc<dyn Endpoint> = Arc::new(endpoint);
                let prepared = self
                    .host(&decl.capability)
                    .ok_or_else(|| format!("nothing hosts `{}` implementations", decl.capability))
                    .and_then(|host| host.prepare(&endpoint));
                match prepared {
                    Ok(capability) => {
                        adapters.push(((decl.capability.clone(), decl.id.clone()), capability))
                    }
                    Err(why) => errors.entry(ext.name.clone()).or_default().push(
                        Diagnostic::error(
                            "activation.script_implementation",
                            format!(
                                "`{}` implementation `{}` can't be adapted: {why}",
                                decl.capability, decl.id
                            ),
                        )
                        .contribution(format!("{}/{}", ext.namespace, decl.id)),
                    ),
                }
            }
        }
        if errors.is_empty() {
            Ok(PreparedScripts { adapters })
        } else {
            Err(errors)
        }
    }

    /// Publish `prepared`: the scripts the previous generation published
    /// and this one doesn't keep are retired, then this one's registered.
    pub(crate) fn publish_scripts(&self, prepared: PreparedScripts) {
        let mut published = self.published_scripts.lock();
        for (capability, id) in published.drain(..) {
            if let Some(host) = self.host(&capability) {
                host.retire(&id);
            }
        }
        for (identity, capability) in prepared.adapters {
            capability.publish();
            published.push(identity);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxplow_db::SqlCell;
    use oxplow_domain::{Actor, CommandOutcome};
    use serde_json::json;

    use crate::test_fixtures::ThreadFixture;

    /// The example's manifest and tracker (`examples/extensions/note-tracker`).
    const MANIFEST: &str =
        include_str!("../../../../examples/extensions/note-tracker/extension.yaml");
    const NOTES: &str =
        include_str!("../../../../examples/extensions/note-tracker/trackers/notes.star");

    /// The note tracker, its script `script`, in the project's extensions,
    /// activated and chosen as the task tracker, as a person's setting does.
    async fn chosen(script: &str) -> ThreadFixture {
        let fx = crate::test_fixtures::services_with_thread().await;
        let dir = fx
            .svc
            .layout
            .project_dir
            .join(".oxplow/extensions/note-tracker");
        std::fs::create_dir_all(dir.join("trackers")).unwrap();
        std::fs::write(dir.join("extension.yaml"), MANIFEST).unwrap();
        std::fs::write(dir.join("trackers/notes.star"), script).unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        run(
            &fx,
            "oxplow.capability.choose",
            json!({ "capability": "task_tracker", "provider": "notes" }),
        )
        .await
        .unwrap();
        assert!(fx.svc.event_pump.settle_all(Duration::from_secs(10)).await);
        crate::test_fixtures::activate(&fx.svc).await;
        fx
    }

    async fn rows(fx: &ThreadFixture, sql: &str, params: Vec<SqlCell>) -> Vec<Vec<SqlCell>> {
        fx.svc.sql.query_sql(sql, params, None).await.unwrap().rows
    }

    fn text(s: &str) -> SqlCell {
        SqlCell::Text(s.into())
    }

    async fn run(
        fx: &ThreadFixture,
        command: &str,
        input: serde_json::Value,
    ) -> Result<CommandOutcome, oxplow_domain::CommandError> {
        fx.svc
            .commands
            .run(&Actor::Human, command, input, true)
            .await
    }

    /// Chosen as the task tracker, an extension's script is one: it is
    /// listed as a script implementation, and `oxplow.task.create` and
    /// `oxplow.task.transition` run through its verbs — the note it files is
    /// in its first stage, a bucket named moves it to that bucket's stage,
    /// and its inverse is the task command that moves it back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_chosen_script_task_tracker_files_and_moves_through_its_verbs() {
        let fx = chosen(NOTES).await;
        assert_eq!(
            rows(
                &fx,
                "SELECT source, active, extension, title FROM v_capability_provider \
                 WHERE capability = 'task_tracker' AND provider = 'notes'",
                vec![],
            )
            .await,
            vec![vec![
                text("script"),
                SqlCell::Int(1),
                text("note-tracker"),
                text("Notes")
            ]]
        );
        let made = run(
            &fx,
            crate::commands::task::CREATE,
            json!({ "title": "Buy milk" }),
        )
        .await
        .unwrap()
        .result;
        let note = made["uri"].as_str().unwrap().to_string();
        let (provider, id) = oxplow_domain::uri::build::task_of_uri(&note).unwrap();
        assert_eq!(provider, "notes");
        assert!(id.starts_with('n'), "{id}");
        assert_eq!(made["stage"], "jotted");
        let row = || {
            rows(
                &fx,
                "SELECT title, stage, state FROM v_task WHERE uri = ?1",
                vec![text(&note)],
            )
        };
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row().await,
            vec![vec![text("Buy milk"), text("jotted"), text("todo")]]
        );
        let moved = run(
            &fx,
            crate::commands::task::NAME,
            json!({ "uri": note, "to": "in_progress" }),
        )
        .await
        .unwrap();
        assert_eq!(moved.result["stage"], "working");
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row().await,
            vec![vec![text("Buy milk"), text("working"), text("in_progress")]]
        );
        // The script's inverse, renamed to the task command, moves it back.
        let inverse = moved.inverse.expect("an inverse");
        assert_eq!(inverse.name, crate::commands::task::NAME);
        assert_eq!(inverse.input["to"], "jotted");
    }

    /// The task tracker conformance suite passes against the script
    /// tracker, as it does against oxplow's own and an external provider.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_task_tracker_conforms() {
        let fx = chosen(NOTES).await;
        let provider = fx.svc.tasks.get("notes").unwrap();
        let run = crate::task_tracker_conformance::suite(
            &fx.svc.tasks_client(),
            &provider.id,
            provider.features,
            None,
            &crate::task_tracker_conformance::ServicesProbe(&fx.svc),
            &Actor::Human,
        )
        .await;
        assert_eq!(run.findings, vec![]);
    }

    /// What a script answers is checked as an external provider's answer
    /// is: a record of another list's item is refused, and so is an answer
    /// that isn't `{ result, events?, inverse? }`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_scripts_answer_is_checked_as_a_providers() {
        let foreign = NOTES.replace("provider = x[\"id\"]", "provider = \"oxtask\"");
        assert_ne!(foreign, NOTES);
        let fx = chosen(&foreign).await;
        let err = run(&fx, crate::commands::task::CREATE, json!({ "title": "x" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("isn't one of its tasks"), "{err}");

        let shapeless = NOTES.replace(
            "return {\"result\": {\"uri\": item[\"uri\"]}, \"events\": [_recorded(item)]}\n\ndef update",
            "return {\"uri\": item[\"uri\"]}\n\ndef update",
        );
        assert_ne!(shapeless, NOTES);
        let fx = chosen(&shapeless).await;
        let err = run(&fx, crate::commands::task::CREATE, json!({ "title": "x" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("something other than"), "{err}");
    }
}
