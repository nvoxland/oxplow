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
  - { id: agents, label: Agents, command: config.set, input: { key: agents, value: [] } }
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
            call("agents", BTreeMap::new(), None, agent, true),
            &extensions::LensContext::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                CommandError::NeedsConfirmation { .. } | CommandError::Denied { .. }
            ),
            "an agent's lens can't set a human-only key: {err:?}"
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
            "title: New Task\nviz: form\nparams: [{ name: body, default: from the form }]\nform: { command: work_item.create, defaults: { description: '{{param.body}}' } }\n",
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
}
