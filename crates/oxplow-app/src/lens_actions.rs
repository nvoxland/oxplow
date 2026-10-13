//! Running a lens's actions (P6.B1, `.context/extensions.md`): each is a
//! command the lens offers, run through the bus as `Actor::Lens` acting for
//! whoever pressed it — a person from the UI, the calling agent from MCP.
//! The bus applies that actor's whole policy (the command's invokers, the
//! agent policy, confirmation), so a lens can offer a button but never grant
//! a power: an agent can't reach a human-only command through a lens, and a
//! lens can't confirm on anyone's behalf.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, CommandError, CommandOutcome};
use serde_json::Value;

use crate::extensions;

/// One press of a lens's action.
pub struct LensActionCall {
    pub lens_id: String,
    pub action_id: String,
    /// Param overrides, as for running the lens.
    pub params: BTreeMap<String, SqlCell>,
    /// For a row action, the row it was invoked on (column → value).
    pub row: Option<BTreeMap<String, SqlCell>>,
    /// For a form action (`form: true`), what the person filled in: laid
    /// over the action's bound `input`, its `fixed` fields imposed again.
    /// Any other action runs as declared and takes none.
    pub input: Option<Value>,
    /// Who pressed it: a person from the UI, the calling agent from MCP.
    pub on_behalf_of: Actor,
    /// The person confirmed a command that asks (an agent never can).
    pub confirmed: bool,
}

/// Run `call`'s action, with the lens resolved in `lens_root` (the stream's
/// worktree) and its params seen from `ctx`.
pub async fn run_lens_action(
    svc: &crate::Services,
    lens_root: &Path,
    call: LensActionCall,
    ctx: &extensions::LensContext,
) -> Result<CommandOutcome, CommandError> {
    svc.sql
        .read_generation(async { run_lens_action_unpinned(svc, lens_root, call, ctx).await })
        .await
        .map_err(CommandError::from)?
}

async fn run_lens_action_unpinned(
    svc: &crate::Services,
    lens_root: &Path,
    call: LensActionCall,
    ctx: &extensions::LensContext,
) -> Result<CommandOutcome, CommandError> {
    let LensActionCall {
        lens_id,
        action_id,
        params,
        row,
        input: given,
        on_behalf_of,
        confirmed,
    } = call;
    let (lens_id, action_id, row) = (lens_id.as_str(), action_id.as_str(), row.as_ref());
    let invalid = |field: &str, message: String| CommandError::Invalid {
        field: Some(field.into()),
        message,
    };
    let lens = svc
        .extension_catalog
        .find_lens(lens_root, lens_id)
        .map_err(CommandError::from)?;
    let action = lens
        .actions
        .iter()
        .find(|a| a.id == action_id)
        .cloned()
        .ok_or_else(|| {
            let ids: Vec<&str> = lens.actions.iter().map(|a| a.id.as_str()).collect();
            invalid(
                "/action",
                format!("lens `{lens_id}` has no action `{action_id}` (it has {ids:?})"),
            )
        })?;
    match (action.row, row) {
        (true, None) => {
            return Err(invalid(
                "/row",
                format!("`{action_id}` is a row action; it runs on a row"),
            ))
        }
        (false, Some(_)) => {
            return Err(invalid(
                "/row",
                format!("`{action_id}` runs on the whole lens, not a row"),
            ))
        }
        _ => {}
    }
    let params = extensions::resolve_params(&lens, &params, ctx).map_err(CommandError::from)?;
    let bound = bind_input(&action.input, &params, row)?;
    let input = match given {
        None => bound,
        Some(_) if !action.form => {
            return Err(invalid(
                "/input",
                format!(
                    "`{action_id}` runs as declared; only a form action (`form: true`) takes input"
                ),
            ))
        }
        Some(given) => filled(bound, given, &action.fixed)?,
    };
    svc.commands
        .run(
            &Actor::Lens {
                lens_id: lens_id.to_string(),
                on_behalf_of: Box::new(on_behalf_of),
            },
            &action.command,
            input,
            confirmed,
        )
        .await
}

/// What a form shows: its command (the fields come from its input
/// schema), the values the fields start from, the fields shown but not
/// editable, and which fields to show (none: all).
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct FormStart {
    pub command: oxplow_domain::CommandSpec,
    #[specta(type = oxplow_domain::Json)]
    pub values: Value,
    pub fixed: Vec<String>,
    pub fields: Option<Vec<String>>,
}

/// `given` (what a person filled in, a map) laid over `start`, with the
/// `fixed` fields put back to `start`'s values: what a form runs with.
fn filled(start: Value, given: Value, fixed: &[String]) -> Result<Value, CommandError> {
    let Value::Object(given) = given else {
        return Err(CommandError::Invalid {
            field: Some("/input".into()),
            message: "a form's input is its fields, a map".into(),
        });
    };
    let mut out = match start {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let kept: Vec<(String, Value)> = fixed
        .iter()
        .filter_map(|k| out.get(k).map(|v| (k.clone(), v.clone())))
        .collect();
    out.extend(given);
    out.extend(kept);
    Ok(Value::Object(out))
}

/// The form a form action (`form: true`) opens: its command, its bound
/// `input` as the starting values, and its `fixed` fields.
pub async fn action_form(
    svc: &crate::Services,
    lens_root: &Path,
    call: LensActionCall,
    ctx: &extensions::LensContext,
) -> Result<FormStart, CommandError> {
    svc.sql
        .read_generation(async {
            let lens = svc
                .extension_catalog
                .find_lens(lens_root, &call.lens_id)
                .map_err(CommandError::from)?;
            let action = lens
                .actions
                .iter()
                .find(|a| a.id == call.action_id && a.form)
                .cloned()
                .ok_or_else(|| CommandError::Invalid {
                    field: Some("/action".into()),
                    message: format!(
                        "lens `{}` has no form action `{}`",
                        call.lens_id, call.action_id
                    ),
                })?;
            let params =
                extensions::resolve_params(&lens, &call.params, ctx).map_err(CommandError::from)?;
            let values = bind_input(&action.input, &params, call.row.as_ref())?;
            let command =
                svc.commands
                    .spec(&action.command)
                    .ok_or_else(|| CommandError::Unknown {
                        name: action.command.clone(),
                    })?;
            Ok(FormStart {
                command,
                values,
                fixed: action.fixed,
                fields: None,
            })
        })
        .await
        .map_err(CommandError::from)?
}

pub async fn form_start(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &extensions::LensContext,
) -> Result<FormStart, CommandError> {
    svc.sql
        .read_generation(async { form_start_unpinned(svc, lens_root, lens_id, params, ctx).await })
        .await
        .map_err(CommandError::from)?
}

async fn form_start_unpinned(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &extensions::LensContext,
) -> Result<FormStart, CommandError> {
    let run = extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        lens_root,
        lens_id,
        params,
        ctx,
    )
    .await
    .map_err(CommandError::from)?;
    let form = match (&run.lens.viz, &run.lens.form) {
        (extensions::LensViz::Form, Some(form)) => form.clone(),
        _ => {
            return Err(CommandError::Invalid {
                field: Some("/lens".into()),
                message: format!("`{lens_id}` isn't a form"),
            })
        }
    };
    let name = form.command.clone().unwrap_or_default();
    let command = svc
        .commands
        .spec(&name)
        .ok_or_else(|| CommandError::Unknown { name: name.clone() })?;
    let mut values = match form.defaults.as_ref() {
        Some(d) => match bind_input(d, &run.params, None)? {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        },
        None => serde_json::Map::new(),
    };
    if let Some(row) = run.result.rows.first() {
        for (col, cell) in run.result.columns.iter().zip(row) {
            if *cell != SqlCell::Null(()) {
                values.insert(
                    col.clone(),
                    serde_json::to_value(cell).unwrap_or(Value::Null),
                );
            }
        }
    }
    Ok(FormStart {
        command,
        values: Value::Object(values),
        fixed: form.fixed,
        fields: form.fields,
    })
}

/// One submit of a form lens.
pub struct FormSubmission {
    pub lens_id: String,
    /// What its fields hold (a map, the command's input).
    pub input: Value,
    /// Param overrides, as for running the lens.
    pub params: BTreeMap<String, SqlCell>,
    pub on_behalf_of: Actor,
    pub confirmed: bool,
}

/// A form lens is submitted: its command runs as the lens, acting for
/// whoever submitted, with the form's `defaults` (placeholders bound from
/// its params) under the submitted input.
pub async fn submit_form(
    svc: &crate::Services,
    lens_root: &Path,
    submission: FormSubmission,
    ctx: &extensions::LensContext,
) -> Result<CommandOutcome, CommandError> {
    svc.sql
        .read_generation(async { submit_form_unpinned(svc, lens_root, submission, ctx).await })
        .await
        .map_err(CommandError::from)?
}

async fn submit_form_unpinned(
    svc: &crate::Services,
    lens_root: &Path,
    submission: FormSubmission,
    ctx: &extensions::LensContext,
) -> Result<CommandOutcome, CommandError> {
    let FormSubmission {
        lens_id,
        input,
        params,
        on_behalf_of,
        confirmed,
    } = submission;
    let lens_id = lens_id.as_str();
    let lens = svc
        .extension_catalog
        .find_lens(lens_root, lens_id)
        .map_err(CommandError::from)?;
    let form = match (&lens.viz, &lens.form) {
        (extensions::LensViz::Form, Some(form)) => form.clone(),
        _ => {
            return Err(CommandError::Invalid {
                field: Some("/lens".into()),
                message: format!("`{lens_id}` isn't a form"),
            })
        }
    };
    let params = extensions::resolve_params(&lens, &params, ctx).map_err(CommandError::from)?;
    let defaults = match form.defaults.as_ref() {
        Some(d) => bind_input(d, &params, None)?,
        None => Value::Object(serde_json::Map::new()),
    };
    let merged = filled(defaults, input, &form.fixed)?;
    svc.commands
        .run(
            &Actor::Lens {
                lens_id: lens_id.to_string(),
                on_behalf_of: Box::new(on_behalf_of),
            },
            form.command.as_deref().unwrap_or_default(),
            merged,
            confirmed,
        )
        .await
}

/// The component a `custom` lens renders, from its extension.
fn component_of(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
) -> Result<
    (
        extensions::Extension,
        extensions::custom_components::CustomComponent,
    ),
    CommandError,
> {
    // One catalog read: the lens and its extension's components together.
    let not_found = || CommandError::from(oxplow_domain::DomainError::NotFound);
    let (ext_name, slug) = lens_id.split_once('/').ok_or_else(not_found)?;
    let ext = svc
        .extension_catalog
        .named(lens_root, ext_name)
        .map_err(CommandError::from)?;
    let lens = ext
        .lenses
        .iter()
        .find(|l| l.slug == slug)
        .ok_or_else(not_found)?;
    let component = match (&lens.viz, &lens.custom) {
        (extensions::LensViz::Custom, Some(c)) => c.component.clone().unwrap_or_default(),
        _ => {
            return Err(CommandError::Invalid {
                field: Some("/lens".into()),
                message: format!("`{lens_id}` isn't a custom component lens"),
            })
        }
    };
    let component = ext
        .custom_components
        .iter()
        .find(|c| c.id == component)
        .cloned()
        .ok_or_else(|| CommandError::Invalid {
            field: Some("/lens".into()),
            message: format!("`{lens_id}`'s component `{component}` isn't loaded"),
        })?;
    Ok((ext, component))
}

/// A custom component reads one of its declared lenses (`assets`; a bare
/// slug is its extension's) — never SQL: the frame names a lens, the
/// lens's own query runs, read-only and parameterised, like any lens run.
pub async fn run_component_query(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
    asset: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &extensions::LensContext,
    version: &str,
) -> Result<extensions::LensRun, CommandError> {
    svc.sql
        .read_generation(async {
            run_component_query_unpinned(svc, lens_root, lens_id, asset, params, ctx, version).await
        })
        .await
        .map_err(CommandError::from)?
}

async fn run_component_query_unpinned(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
    asset: &str,
    params: BTreeMap<String, SqlCell>,
    ctx: &extensions::LensContext,
    version: &str,
) -> Result<extensions::LensRun, CommandError> {
    let (ext, component) = component_of(svc, lens_root, lens_id)?;
    let loaded = svc
        .component_bundles
        .get(version)
        .filter(|bundle| {
            bundle.extension == ext.name
                && bundle.component == component.id
                && svc.component_bundles.is_current(lens_root, bundle)
        })
        .ok_or_else(|| CommandError::Denied {
            reason: "the component's loaded version is unavailable; reload its lens".into(),
        })?;
    let asset = if asset.contains('/') {
        asset.to_string()
    } else {
        format!("{}/{asset}", component.extension)
    };
    if !loaded.assets.contains(&asset) {
        return Err(CommandError::Invalid {
            field: Some("/asset".into()),
            message: format!(
                "`{asset}` isn't one of component `{}`'s assets ({})",
                component.id,
                loaded.assets.join(", ")
            ),
        });
    }
    extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        lens_root,
        &asset,
        params,
        ctx,
    )
    .await
    .map_err(CommandError::from)
}

/// A custom component invokes one of its declared commands.
pub struct ComponentInvoke {
    /// The `custom` lens whose frame asked.
    pub lens_id: String,
    pub command: String,
    /// Sent as is: a component's input is literal (no placeholders).
    pub input: Value,
    /// Who is looking at the frame.
    pub on_behalf_of: Actor,
    /// The person confirmed this call, in the host (never in the frame).
    pub confirmed: bool,
    /// The bundle version the frame was loaded at
    /// ([`crate::component_bundles`]): what is running, and what must be
    /// approved.
    pub version: String,
}

/// Run `call`'s command as the lens acting for its viewer, so every
/// policy applies as if they ran it — a component can offer an action but
/// never grant a power — when the bundle its frame was loaded at declares
/// the command and a person approved that version (tsk960, tsk984): what
/// acts is what was approved, whatever the disk says now.
pub async fn invoke_component_command(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentInvoke,
) -> Result<CommandOutcome, CommandError> {
    svc.sql
        .read_generation(async { invoke_component_command_unpinned(svc, lens_root, call).await })
        .await
        .map_err(CommandError::from)?
}

async fn invoke_component_command_unpinned(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentInvoke,
) -> Result<CommandOutcome, CommandError> {
    let (ext, component) = component_of(svc, lens_root, &call.lens_id)?;
    let loaded = svc
        .component_bundles
        .get(&call.version)
        .filter(|bundle| {
            bundle.extension == ext.name
                && bundle.component == component.id
                && svc.component_bundles.is_current(lens_root, bundle)
        })
        .ok_or_else(|| CommandError::Denied {
            reason: format!(
                "component `{}/{}` isn't loaded at that version any more; reload its lens",
                ext.name, component.id
            ),
        })?;
    if !loaded.commands.contains(&call.command) {
        return Err(CommandError::Invalid {
            field: Some("/command".into()),
            message: format!(
                "`{}` isn't one of component `{}`'s commands ({})",
                call.command,
                component.id,
                loaded.commands.join(", ")
            ),
        });
    }
    let program = crate::exec_consent::component_program(&ext, &component);
    if !svc.approvals.is_approved(&program.key(), &loaded.version) {
        return Err(CommandError::Denied {
            reason: crate::exec_consent::needs_approval(
                program.kind,
                &program.name,
                &program.program,
            ),
        });
    }
    svc.commands
        .run(
            &Actor::Lens {
                lens_id: call.lens_id,
                on_behalf_of: Box::new(call.on_behalf_of),
            },
            &call.command,
            call.input,
            call.confirmed,
        )
        .await
}

/// Load the bundle of `lens_id`'s component as it is now under
/// `lens_root`, for its frame: the version to serve it at and invoke with.
pub fn load_component(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
) -> Result<String, CommandError> {
    let (ext, component) = component_of(svc, lens_root, lens_id)?;
    let loaded = svc
        .component_bundles
        .load(lens_root, &ext, &component)
        .map_err(CommandError::from)?;
    Ok(loaded.version.clone())
}

/// Load a component under one pinned extension catalog generation.
pub async fn load_component_pinned(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
) -> Result<String, CommandError> {
    svc.sql
        .read_generation(async { load_component(svc, lens_root, lens_id) })
        .await
        .map_err(CommandError::from)?
}

/// `input` with its placeholders bound: a string that is exactly
/// `{{param.x}}` / `{{row.x}}` becomes that value (typed), one that
/// contains them has them spliced in as text. The values are data — they
/// go into the command's input, never into SQL.
pub fn bind_input(
    input: &Value,
    params: &BTreeMap<String, SqlCell>,
    row: Option<&BTreeMap<String, SqlCell>>,
) -> Result<Value, CommandError> {
    let lookup = |scope: &str, name: &str| -> Result<SqlCell, CommandError> {
        let found = match scope {
            "param" => params.get(name),
            "row" => row.and_then(|r| r.get(name)),
            _ => None,
        };
        found.cloned().ok_or_else(|| CommandError::Invalid {
            field: Some("/input".into()),
            message: format!("`{{{{{scope}.{name}}}}}` has no value here"),
        })
    };
    let text = |c: &SqlCell| match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.clone(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    };
    Ok(match input {
        Value::String(s) => match oxplow_domain::template::whole_placeholder(s) {
            Some(p) => serde_json::to_value(lookup(&p.scope, &p.name)?).unwrap_or(Value::Null),
            None => Value::String(oxplow_domain::template::splice(s, |p| {
                lookup(&p.scope, &p.name).map(|c| text(&c))
            })?),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|i| bind_input(i, params, row))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), bind_input(v, params, row)?)))
                .collect::<Result<_, CommandError>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_tasks::work_item_ref;
    use serde_json::json;

    /// A task, and a lens over it whose actions run commands.
    async fn fixture() -> (crate::test_fixtures::TaskEffortFixture, std::path::PathBuf) {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let ext = root.join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/tasks.yaml"),
            r#"title: Tasks
query: "SELECT ref, title FROM v_work_item"
params: [{ name: item, default: "" }]
actions:
  - { id: finish, label: Finish, command: oxplow.work_item.transition, input: { ref: "{{param.item}}", to: done } }
  - { id: finish-row, label: Finish, command: oxplow.work_item.transition, row: true, input: { ref: "{{row.ref}}", to: done } }
  - { id: commit, label: Commit, command: oxplow.vcs.commit, input: { stream: str1, message: "x" } }
  - { id: prompt, label: Prompt, command: oxplow.config.set, input: { key: agentPromptAppend, value: be brief } }
  - { id: retitle, label: Retitle…, command: oxplow.work_item.update, row: true, form: true, fixed: [ref], input: { ref: "{{row.ref}}", title: "{{row.title}}" } }
"#,
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        (fx, root)
    }

    fn call(
        action: &str,
        params: BTreeMap<String, SqlCell>,
        row: Option<BTreeMap<String, SqlCell>>,
        on_behalf_of: Actor,
        confirmed: bool,
    ) -> LensActionCall {
        LensActionCall {
            lens_id: "acme/tasks".into(),
            action_id: action.into(),
            params,
            row,
            input: None,
            on_behalf_of,
            confirmed,
        }
    }

    async fn audit_actor(svc: &crate::Services) -> (String, Option<String>) {
        svc.db
            .read(|c| {
                c.query_row(
                    "SELECT actor_kind, actor_id FROM command_audit ORDER BY id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap()
    }

    async fn state(svc: &crate::Services, task: oxplow_tasks::TaskId) -> String {
        use oxplow_tasks::TaskStore as _;
        let t = svc.task_store.get(task).await.unwrap().unwrap();
        serde_json::to_value(t.status)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    /// P6.B1's red: a person's click runs the command as the lens, acting
    /// for them — audited as `lens:<id>`.
    #[tokio::test]
    async fn an_action_runs_its_command_as_the_lens() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        run_lens_action(
            &fx.svc,
            &root,
            call(
                "finish",
                BTreeMap::from([("item".to_string(), SqlCell::Text(item))]),
                None,
                Actor::Human,
                false,
            ),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
        let (kind, _) = audit_actor(&fx.svc).await;
        assert_eq!(kind, "lens");
    }

    /// A row action binds the row it was invoked on; a whole-lens action
    /// refuses a row and a row action needs one.
    #[tokio::test]
    async fn a_row_action_binds_its_row() {
        let (fx, root) = fixture().await;
        let row = BTreeMap::from([(
            "ref".to_string(),
            SqlCell::Text(oxplow_tasks::work_item_ref(fx.task)),
        )]);
        let run = |action: &'static str, row: Option<BTreeMap<String, SqlCell>>| {
            let (svc, root) = (fx.svc.clone(), root.clone());
            async move {
                run_lens_action(
                    &svc,
                    &root,
                    call(action, BTreeMap::new(), row, Actor::Human, false),
                    &extensions::LensContext::default(),
                )
                .await
            }
        };
        assert!(run("finish-row", None).await.is_err());
        assert!(run("finish", Some(row.clone())).await.is_err());
        run("finish-row", Some(row)).await.unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A command that doesn't take lens invokers is denied, and a lens
    /// acting for an agent can't reach what the agent can't.
    #[tokio::test]
    async fn a_lens_grants_no_power() {
        let (fx, root) = fixture().await;
        let err = run_lens_action(
            &fx.svc,
            &root,
            call("commit", BTreeMap::new(), None, Actor::Human, true),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = run_lens_action(
            &fx.svc,
            &root,
            call("prompt", BTreeMap::new(), None, agent, true),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, CommandError::Proposed { .. }),
            "an agent's lens can't set a human-only key, only propose it: {err:?}"
        );
    }

    /// A form action opens the command's form starting from its bound
    /// input, with its fixed fields; what the person fills in runs, the
    /// fixed fields put back whatever was sent. A plain action takes no
    /// input.
    #[tokio::test]
    async fn a_form_action_runs_what_the_person_filled_in_keeping_its_fixed_fields() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        let row = BTreeMap::from([
            ("ref".to_string(), SqlCell::Text(item.clone())),
            ("title".to_string(), SqlCell::Text("Old title".into())),
        ]);
        let start = action_form(
            &fx.svc,
            &root,
            call(
                "retitle",
                BTreeMap::new(),
                Some(row.clone()),
                Actor::Human,
                false,
            ),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.command.id, "oxplow.work_item.update");
        assert_eq!(start.values, json!({ "ref": item, "title": "Old title" }));
        assert_eq!(start.fixed, vec!["ref".to_string()]);

        let mut press = call("retitle", BTreeMap::new(), Some(row), Actor::Human, false);
        press.input = Some(json!({ "ref": "work_item:oxplow:tsk999999", "title": "New title" }));
        run_lens_action(&fx.svc, &root, press, &extensions::LensContext::default())
            .await
            .unwrap();
        let title: String = fx
            .svc
            .db
            .read({
                let item = item.clone();
                move |c| {
                    c.query_row(
                        "SELECT title FROM v_work_item WHERE ref = ?1",
                        [item],
                        |r| r.get(0),
                    )
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                }
            })
            .await
            .unwrap();
        assert_eq!(
            title, "New title",
            "the fixed ref was kept, so this item changed"
        );

        let mut plain = call(
            "finish",
            BTreeMap::from([("item".to_string(), SqlCell::Text(item))]),
            None,
            Actor::Human,
            false,
        );
        plain.input = Some(json!({ "to": "canceled" }));
        let err = run_lens_action(&fx.svc, &root, plain, &extensions::LensContext::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("only a form action"), "{err}");
        let err = action_form(
            &fx.svc,
            &root,
            call("finish", BTreeMap::new(), None, Actor::Human, false),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no form action"), "{err}");
    }

    /// A form lens's fixed fields keep their `defaults` on submit, and its
    /// `fields` say which to show.
    #[tokio::test]
    async fn a_form_lens_keeps_its_fixed_fields() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        std::fs::write(
            root.join("oxplow/extensions/acme/lenses/edit.yaml"),
            "title: Edit\nviz: form\nparams: [{ name: ref }]\nquery: SELECT title FROM v_work_item WHERE ref = :ref\nform: { command: oxplow.work_item.update, defaults: { ref: '{{param.ref}}' }, fixed: [ref], fields: [title] }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let params = BTreeMap::from([("ref".to_string(), SqlCell::Text(item.clone()))]);
        let start = form_start(
            &fx.svc,
            &root,
            "acme/edit",
            params.clone(),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.fixed, vec!["ref".to_string()]);
        assert_eq!(start.fields, Some(vec!["title".to_string()]));
        assert_eq!(start.values["ref"], json!(item));
        submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/edit".into(),
                input: json!({ "ref": "work_item:oxplow:tsk999999", "title": "Edited" }),
                params,
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::LensContext::default(),
        )
        .await
        .unwrap();
        let title: String = fx
            .svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT title FROM v_work_item WHERE ref = ?1",
                    [item],
                    |r| r.get(0),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(title, "Edited");
    }

    #[test]
    fn placeholders_bind_typed_or_spliced() {
        let params = BTreeMap::from([("n".to_string(), SqlCell::Int(3))]);
        let row = BTreeMap::from([("id".to_string(), SqlCell::Int(42))]);
        let bound = bind_input(
            &json!({ "a": "{{param.n}}", "b": "tsk{{row.id}}", "c": ["{{ row.id }}"], "d": 1 }),
            &params,
            Some(&row),
        )
        .unwrap();
        assert_eq!(bound, json!({ "a": 3, "b": "tsk42", "c": [42], "d": 1 }));
        let err = bind_input(&json!({ "a": "{{row.id}}" }), &params, None).unwrap_err();
        assert!(err.to_string().contains("row.id"), "{err}");
    }

    /// P6.B2: a form lens submits its command as the lens, its defaults
    /// under what the person filled in.
    #[tokio::test]
    async fn a_form_submits_its_command_as_the_lens() {
        let (fx, root) = fixture().await;
        std::fs::write(
            root.join("oxplow/extensions/acme/lenses/new-task.yaml"),
            "title: New Task\nviz: form\nparams: [{ name: body, default: from the form }]\nform: { command: oxplow.work_item.create, defaults: { body: '{{param.body}}' } }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let out = submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/new-task".into(),
                input: json!({ "title": "Made by a form" }),
                params: BTreeMap::new(),
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::LensContext::default(),
        )
        .await
        .unwrap();
        let item = out.result["ref"].as_str().unwrap().to_string();
        let (kind, _) = audit_actor(&fx.svc).await;
        assert_eq!(kind, "lens");
        let row: (String, String) = fx
            .svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT title, body FROM v_work_item WHERE ref = ?1",
                    [item],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(row, ("Made by a form".into(), "from the form".into()));
        // A lens that isn't a form can't be submitted.
        let err = submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/tasks".into(),
                input: json!({}),
                params: BTreeMap::new(),
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("isn't a form"), "{err}");
    }

    /// P6b.D2: a private extension with a component (`board`) that may
    /// query `acme/tasks` and invoke `oxplow.work_item.transition`.
    async fn component_fixture() -> (crate::test_fixtures::TaskEffortFixture, std::path::PathBuf) {
        let (fx, root) = fixture().await;
        let ext = root.join("oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\ncustom_components:\n  - { id: board, assets: [tasks], commands: [oxplow.work_item.transition] }\n",
        )
        .unwrap();
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        (fx, root)
    }

    /// Approve `acme/board` as it is now, as a person on Programs does.
    async fn approve_board(fx: &crate::test_fixtures::EffortFixture, root: &Path) {
        let ext = fx.svc.extension_catalog.named(root, "acme").unwrap();
        let program = crate::exec_consent::component_program(&ext, &ext.custom_components[0]);
        let config = fx.svc.config.read().unwrap().clone();
        crate::exec_consent::approve_program(
            &fx.svc.approvals,
            root,
            &config,
            std::slice::from_ref(&ext),
            crate::exec_consent::ProgramKind::Component,
            &program.name,
            &program.hash(root).unwrap(),
        )
        .unwrap();
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
    }

    /// Load `acme/view`'s bundle as its frame does: the version it runs.
    async fn load_board(fx: &crate::test_fixtures::EffortFixture, root: &Path) -> String {
        // Whatever was edited since is published first, as the owner of
        // generations does on the file change.
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        load_component(&fx.svc, root, "acme/view").unwrap()
    }

    /// An invoke of `command` from `acme/view`'s frame loaded at `version`.
    fn board_invoke(
        fx: &crate::test_fixtures::TaskEffortFixture,
        command: &str,
        version: &str,
    ) -> ComponentInvoke {
        ComponentInvoke {
            lens_id: "acme/view".into(),
            command: command.into(),
            input: serde_json::json!({ "ref": oxplow_tasks::work_item_ref(fx.task), "to": "done" }),
            on_behalf_of: Actor::Human,
            confirmed: false,
            version: version.into(),
        }
    }

    /// The programs a person is asked to approve, as Programs lists them.
    fn programs(
        fx: &crate::test_fixtures::EffortFixture,
        root: &Path,
    ) -> Vec<crate::exec_consent::ProjectProgram> {
        let config = fx.svc.config.read().unwrap().clone();
        crate::exec_consent::list(
            &fx.svc.approvals,
            root,
            &config,
            fx.svc.extension_catalog.get(root).as_ref(),
        )
    }

    /// P11 (tsk960): a component that declares commands acts with a
    /// person's rights, so it is a program a person approves — its bundle
    /// and the commands it may run. Until then its invoke is refused and
    /// nothing runs; approved as it is now, it runs; a changed bundle asks
    /// again.
    #[tokio::test]
    async fn an_unapproved_components_invoke_is_refused() {
        let (fx, root) = component_fixture().await;
        let listed = programs(&fx, &root);
        let board = listed
            .iter()
            .find(|p| p.kind == crate::exec_consent::ProgramKind::Component)
            .expect("listed on Programs");
        assert_eq!(board.name, "acme/board");
        assert_eq!(
            board.commands,
            vec!["oxplow.work_item.transition".to_string()]
        );
        assert!(!board.approved);
        let version = load_board(&fx, &root).await;
        assert_eq!(
            board.version.as_deref(),
            Some(version.as_str()),
            "the listing's version is the frame's"
        );
        let invoke = || board_invoke(&fx, "oxplow.work_item.transition", &version);
        refused(
            invoke_component_command(&fx.svc, &root, invoke())
                .await
                .unwrap_err(),
            "approval",
        );
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        approve_board(&fx, &root).await;
        invoke_component_command(&fx.svc, &root, invoke())
            .await
            .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
        // A changed bundle is another version: loaded again, it asks again.
        std::fs::write(
            root.join("oxplow/extensions/acme/components/board/index.html"),
            "<!doctype html><script src=other.js></script>",
        )
        .unwrap();
        let changed = load_board(&fx, &root).await;
        assert_ne!(changed, version);
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", &changed),
            )
            .await
            .unwrap_err(),
            "approval",
        );
    }

    /// `err` is a refusal saying `why`.
    fn refused(err: CommandError, why: &str) {
        assert!(
            matches!(&err, CommandError::Denied { reason }
                if reason.contains("component `acme/board`") && reason.contains(why)),
            "{err:?}"
        );
    }

    /// tsk984: what acts is what the frame loaded. A frame loaded while the
    /// bundle was edited can't act on the approval of what the disk says
    /// once it's put back; one loaded at the approved version keeps acting
    /// whatever the disk says later — it runs what was approved. A version
    /// that isn't loaded, or is another component's, is refused.
    #[tokio::test]
    async fn an_invoke_acts_only_for_the_version_its_frame_loaded() {
        let (fx, root) = component_fixture().await;
        let index = root.join("oxplow/extensions/acme/components/board/index.html");
        let approved = load_board(&fx, &root).await;
        approve_board(&fx, &root).await;
        std::fs::write(&index, "<!doctype html><script src=evil.js></script>").unwrap();
        let tampered = load_board(&fx, &root).await;
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", &tampered),
            )
            .await
            .unwrap_err(),
            "approval",
        );
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        // Put back, the next generation serves the approved bytes again.
        std::fs::write(&index, "<!doctype html>").unwrap();
        assert_eq!(load_board(&fx, &root).await, approved);
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", "0123abcd"),
            )
            .await
            .unwrap_err(),
            "isn't loaded",
        );
        std::fs::write(&index, "<!doctype html><p>later</p>").unwrap();
        invoke_component_command(
            &fx.svc,
            &root,
            board_invoke(&fx, "oxplow.work_item.transition", &approved),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A hot update: once a changed bundle is activated, a frame still on
    /// the old version can't act — it's told to reload — and the version
    /// it reloads at acts only once a person approves that code; the old
    /// approval doesn't carry over.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_activated_edit_needs_a_reload_and_its_own_approval() {
        let (fx, root) = component_fixture().await;
        let activate = || async {
            fx.svc
                .extension_activation
                .activate(fx.svc.clone())
                .await
                .expect("activates");
        };
        activate().await;
        let old = load_board(&fx, &root).await;
        approve_board(&fx, &root).await;

        std::fs::write(
            root.join("oxplow/extensions/acme/components/board/index.html"),
            "<!doctype html><p>2</p>",
        )
        .unwrap();
        activate().await;
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", &old),
            )
            .await
            .unwrap_err(),
            "reload its lens",
        );
        let new = load_board(&fx, &root).await;
        assert_ne!(new, old);
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", &new),
            )
            .await
            .unwrap_err(),
            "approval",
        );
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        approve_board(&fx, &root).await;
        invoke_component_command(
            &fx.svc,
            &root,
            board_invoke(&fx, "oxplow.work_item.transition", &new),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// tsk984: the bundle is read as it is served — the extension's folder
    /// joined with the manifest's `bundle` — so on a case-insensitive disk a
    /// spelling that differs only in case still covers every file it
    /// serves, and an edit is a new version.
    #[tokio::test]
    async fn a_bundle_spelled_in_another_case_is_hashed_as_served() {
        let (fx, root) = fixture().await;
        let ext = root.join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        if !ext.join("components/BOARD").exists() {
            // A case-sensitive disk: the manifest's spelling is a missing
            // folder there, a load error, never a silent hash.
            return;
        }
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\ncustom_components:\n  - { id: board, bundle: components/Board, assets: [tasks], commands: [oxplow.work_item.transition] }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        let first = load_board(&fx, &root).await;
        let served = fx.svc.component_bundles.get(&first).unwrap();
        assert_eq!(served.file(""), Some(&b"<!doctype html>"[..]));
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>2").unwrap();
        assert_ne!(load_board(&fx, &root).await, first);
        let program = programs(&fx, &root)
            .into_iter()
            .find(|p| p.kind == crate::exec_consent::ProgramKind::Component)
            .unwrap();
        assert_ne!(program.version.as_deref(), Some(first.as_str()));
    }

    /// P11 (tsk960): a component that declares no commands can only show
    /// and query — nothing to approve, and it isn't listed.
    #[tokio::test]
    async fn a_component_without_commands_needs_no_approval() {
        let (fx, root) = fixture().await;
        let ext = root.join("oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\ncustom_components:\n  - { id: board, assets: [tasks] }\n",
        )
        .unwrap();
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        assert!(programs(&fx, &root)
            .iter()
            .all(|p| p.kind != crate::exec_consent::ProgramKind::Component));
        let ctx = extensions::LensContext::default();
        run_component_query(
            &fx.svc,
            &root,
            "acme/view",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_component_queries_only_its_declared_lenses() {
        let (fx, root) = component_fixture().await;
        let ctx = extensions::LensContext::default();
        let run = run_component_query(
            &fx.svc,
            &root,
            "acme/view",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap();
        assert_eq!(run.lens.id, "acme/tasks");
        let err = run_component_query(
            &fx.svc,
            &root,
            "acme/view",
            "acme/view",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("isn't one of component `board`'s assets (acme/tasks)")),
            "{err:?}"
        );
        let err = run_component_query(
            &fx.svc,
            &root,
            "acme/tasks",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("isn't a custom component lens"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_component_invokes_only_its_declared_commands_as_the_lens() {
        let (fx, root) = component_fixture().await;
        let version = load_board(&fx, &root).await;
        let invoke = |command: &str| board_invoke(&fx, command, &version);
        let err = invoke_component_command(&fx.svc, &root, invoke("oxplow.config.set"))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("isn't one of component `board`'s commands"),
            "{err}"
        );
        approve_board(&fx, &root).await;
        let out = invoke_component_command(&fx.svc, &root, invoke("oxplow.work_item.transition"))
            .await
            .unwrap();
        let audit = fx
            .svc
            .commands
            .audit_store()
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit.actor_kind,
            oxplow_domain::events::schema::ActorKind::Lens
        );
        assert_eq!(audit.actor_id.as_deref(), Some("acme/view"));
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// The frame's input is checked like anyone's: an array or a scalar
    /// where the command takes an object is `Invalid`, and nothing runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_invoke_with_a_non_object_input_is_invalid() {
        let (fx, root) = component_fixture().await;
        let version = load_board(&fx, &root).await;
        approve_board(&fx, &root).await;
        for input in [
            serde_json::json!([1, 2]),
            serde_json::json!("done"),
            serde_json::json!(7),
        ] {
            let err = invoke_component_command(
                &fx.svc,
                &root,
                ComponentInvoke {
                    lens_id: "acme/view".into(),
                    command: "oxplow.work_item.transition".into(),
                    input: input.clone(),
                    on_behalf_of: Actor::Human,
                    confirmed: false,
                    version: version.clone(),
                },
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
        assert_ne!(state(&fx.svc, fx.task).await, "done");
    }
}
