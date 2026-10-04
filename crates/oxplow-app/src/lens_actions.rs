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
    let LensActionCall {
        lens_id,
        action_id,
        params,
        row,
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
    let input = bind_input(&action.input, &params, row)?;
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

/// What a form lens shows: its command (the fields come from its input
/// schema) and the values the fields start from — the form's `defaults`
/// (placeholders bound) under the query's first row, if it has a query.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct FormStart {
    pub command: oxplow_domain::CommandSpec,
    #[specta(type = oxplow_domain::Json)]
    pub values: Value,
}

pub async fn form_start(
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
    let Value::Object(given) = input else {
        return Err(CommandError::Invalid {
            field: Some("/input".into()),
            message: "a form's input is its fields, a map".into(),
        });
    };
    let params = extensions::resolve_params(&lens, &params, ctx).map_err(CommandError::from)?;
    let mut merged = match form.defaults.as_ref() {
        Some(d) => match bind_input(d, &params, None)? {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        },
        None => serde_json::Map::new(),
    };
    merged.extend(given);
    svc.commands
        .run(
            &Actor::Lens {
                lens_id: lens_id.to_string(),
                on_behalf_of: Box::new(on_behalf_of),
            },
            form.command.as_deref().unwrap_or_default(),
            Value::Object(merged),
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
) -> Result<extensions::LensRun, CommandError> {
    let (_, component) = component_of(svc, lens_root, lens_id)?;
    let asset = if asset.contains('/') {
        asset.to_string()
    } else {
        format!("{}/{asset}", component.extension)
    };
    if !component.assets.contains(&asset) {
        return Err(CommandError::Invalid {
            field: Some("/asset".into()),
            message: format!(
                "`{asset}` isn't one of component `{}`'s assets ({})",
                component.id,
                component.assets.join(", ")
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
}

/// Run `call`'s command as the lens acting for its viewer, so every
/// policy applies as if they ran it — a component can offer an action but
/// never grant a power — when the component declares it, and a person
/// approved the component as it is now (tsk960): its bundle runs that
/// command with their rights.
pub async fn invoke_component_command(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentInvoke,
) -> Result<CommandOutcome, CommandError> {
    let (ext, component) = component_of(svc, lens_root, &call.lens_id)?;
    if !component.commands.contains(&call.command) {
        return Err(CommandError::Invalid {
            field: Some("/command".into()),
            message: format!(
                "`{}` isn't one of component `{}`'s commands ({})",
                call.command,
                component.id,
                component.commands.join(", ")
            ),
        });
    }
    if let Some(program) = crate::exec_consent::component_program(&ext, &component) {
        if !crate::exec_consent::may_run_program(&svc.approvals, lens_root, &program) {
            return Err(CommandError::Denied {
                reason: crate::exec_consent::needs_approval(
                    program.kind,
                    &program.name,
                    &program.program,
                ),
            });
        }
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
        Value::String(s) => match crate::extensions::whole_placeholder(s) {
            Some(p) => serde_json::to_value(lookup(&p.scope, &p.name)?).unwrap_or(Value::Null),
            None => {
                let mut out = String::with_capacity(s.len());
                let mut at = 0;
                for p in crate::extensions::placeholders(s) {
                    out.push_str(&s[at..p.start]);
                    out.push_str(&text(&lookup(&p.scope, &p.name)?));
                    at = p.end;
                }
                out.push_str(&s[at..]);
                Value::String(out)
            }
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
    use oxplow_domain::refs::build::work_item_ref;
    use serde_json::json;

    /// A task, and a lens over it whose actions run commands.
    async fn fixture() -> (crate::test_fixtures::EffortFixture, std::path::PathBuf) {
        let fx = crate::test_fixtures::services_with_effort().await;
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
query: "SELECT id, title FROM v_task"
params: [{ name: item, default: "" }]
actions:
  - { id: finish, label: Finish, command: work_item.transition, input: { ref: "{{param.item}}", to: done } }
  - { id: finish-row, label: Finish, command: work_item.transition, row: true, input: { ref: "work_item:oxplow:tsk{{row.id}}", to: done } }
  - { id: commit, label: Commit, command: vcs.commit, input: { stream: str1, message: "x" } }
  - { id: prompt, label: Prompt, command: config.set, input: { key: agentPromptAppend, value: be brief } }
"#,
        )
        .unwrap();
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

    async fn state(svc: &crate::Services, task: oxplow_domain::TaskId) -> String {
        use oxplow_domain::stores::TaskStore as _;
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
        let row = BTreeMap::from([("id".to_string(), SqlCell::Int(fx.task.value()))]);
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
            "title: New Task\nviz: form\nparams: [{ name: body, default: from the form }]\nform: { command: work_item.create, defaults: { body: '{{param.body}}' } }\n",
        )
        .unwrap();
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
    /// query `acme/tasks` and invoke `work_item.transition`.
    async fn component_fixture() -> (crate::test_fixtures::EffortFixture, std::path::PathBuf) {
        let (fx, root) = fixture().await;
        let ext = root.join("oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\ncustom_components:\n  - { id: board, assets: [tasks], commands: [work_item.transition] }\n",
        )
        .unwrap();
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        (fx, root)
    }

    /// Approve `acme/board` as it is now, as a person on Programs does.
    fn approve_board(fx: &crate::test_fixtures::EffortFixture, root: &Path) {
        let ext = fx.svc.extension_catalog.named(root, "acme").unwrap();
        let program =
            crate::exec_consent::component_program(&ext, &ext.custom_components[0]).unwrap();
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
        let invoke = || ComponentInvoke {
            lens_id: "acme/view".into(),
            command: "work_item.transition".into(),
            input: serde_json::json!({ "ref": oxplow_domain::refs::build::work_item_ref(fx.task), "to": "done" }),
            on_behalf_of: Actor::Human,
            confirmed: false,
        };
        let listed = programs(&fx, &root);
        let board = listed
            .iter()
            .find(|p| p.kind == crate::exec_consent::ProgramKind::Component)
            .expect("listed on Programs");
        assert_eq!(board.name, "acme/board");
        assert_eq!(board.commands, vec!["work_item.transition".to_string()]);
        assert!(!board.approved);
        let refused = |err: CommandError| {
            assert!(
                matches!(&err, CommandError::Denied { reason }
                    if reason.contains("component `acme/board`") && reason.contains("approval")),
                "{err:?}"
            );
        };
        refused(
            invoke_component_command(&fx.svc, &root, invoke())
                .await
                .unwrap_err(),
        );
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        approve_board(&fx, &root);
        invoke_component_command(&fx.svc, &root, invoke())
            .await
            .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
        std::fs::write(
            root.join("oxplow/extensions/acme/components/board/index.html"),
            "<!doctype html><script src=other.js></script>",
        )
        .unwrap();
        refused(
            invoke_component_command(&fx.svc, &root, invoke())
                .await
                .unwrap_err(),
        );
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
        run_component_query(&fx.svc, &root, "acme/view", "tasks", BTreeMap::new(), &ctx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_component_queries_only_its_declared_lenses() {
        let (fx, root) = component_fixture().await;
        let ctx = extensions::LensContext::default();
        let run = run_component_query(&fx.svc, &root, "acme/view", "tasks", BTreeMap::new(), &ctx)
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
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("isn't one of component `board`'s assets (acme/tasks)")),
            "{err:?}"
        );
        let err = run_component_query(&fx.svc, &root, "acme/tasks", "tasks", BTreeMap::new(), &ctx)
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
        let invoke = |command: &str| ComponentInvoke {
            lens_id: "acme/view".into(),
            command: command.into(),
            input: serde_json::json!({ "ref": oxplow_domain::refs::build::work_item_ref(fx.task), "to": "done" }),
            on_behalf_of: Actor::Human,
            confirmed: false,
        };
        let err = invoke_component_command(&fx.svc, &root, invoke("config.set"))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("isn't one of component `board`'s commands"),
            "{err}"
        );
        approve_board(&fx, &root);
        let out = invoke_component_command(&fx.svc, &root, invoke("work_item.transition"))
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
        approve_board(&fx, &root);
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
                    command: "work_item.transition".into(),
                    input: input.clone(),
                    on_behalf_of: Actor::Human,
                    confirmed: false,
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
