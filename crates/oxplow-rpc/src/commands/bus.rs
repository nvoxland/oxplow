//! The desktop's way onto the command bus (P5.A1, `.context/commands.md`
//! "Callers"): run any command as the person, and undo a run. A typed
//! IPC setter is a convenience over one command; anything new the UI
//! writes goes through here, so the bus's validation, policy,
//! confirmation, audit and undo apply once.

use oxplow_app::Services;
use oxplow_domain::{Actor, CommandOutcome, CommandSpec, Json};
use oxplow_domain::{AuditId, ProposalId};

use crate::error::IpcError;

/// Run command `id` with `input` as the person. `confirmed` says the person
/// confirmed this exact call (a destructive command, a human-only config
/// key); an unconfirmed call that needs one comes back
/// `NEEDS_CONFIRMATION`, so the UI asks and calls again.
pub async fn run_command(
    svc: &Services,
    id: String,
    input: Json,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = svc
        .commands
        .run(&Actor::Human, &id, input.0, confirmed)
        .await?;
    // A write may have opened or closed an effort: let its snapshot pin
    // land before the UI reads what follows.
    if outcome.audit_id.is_some() {
        svc.efforts.settle_lifecycle().await;
    }
    Ok(outcome)
}

/// A read the person makes through the bus: command `id` run as the
/// person, its result decoded. A typed IPC read the agent has too is
/// this, so both front doors validate and answer alike.
pub(crate) async fn read_as_person<T: serde::de::DeserializeOwned>(
    svc: &Services,
    id: &str,
    input: serde_json::Value,
) -> Result<T, IpcError> {
    let outcome = svc.commands.run(&Actor::Human, id, input, false).await?;
    serde_json::from_value(outcome.result)
        .map_err(|e| IpcError::internal(format!("`{id}` answered an unexpected shape: {e}")))
}

/// The page a person's run of command `id` opens next: its
/// `ui.open_after` bound from the run's `result` by the one tokenizer
/// (`CommandUi::open_after_for`); `None` when it says none. A field the
/// result lacks is `INVALID`, naming it.
pub async fn command_open_after(
    svc: &Services,
    id: String,
    result: Json,
) -> Result<Option<String>, IpcError> {
    let spec = svc
        .commands
        .list(&Actor::Human)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| IpcError::invalid(format!("no command `{id}`")))?;
    spec.ui
        .as_ref()
        .map_or(Ok(None), |ui| ui.open_after_for(&result.0))
        .map_err(|e| IpcError::invalid(format!("`{id}`: {e}")))
}

/// The commands a person is offered: those active now (their needs
/// met) that say how a person meets them (`ui`) — what search,
/// menus and pages list (`.context/commands.md` "Offering a command to a
/// person").
pub async fn list_person_commands(svc: &Services) -> Result<Vec<CommandSpec>, IpcError> {
    let mut specs: Vec<CommandSpec> = svc
        .commands
        .list(&Actor::Human)
        .into_iter()
        .filter(|s| s.ui.is_some())
        .collect();
    specs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(specs)
}

/// The commands about the ref `uri` a person is offered: those whose kind
/// (`ui.about`) it is, with each `ui.about_when` held on its row
/// (`oxplow_app::commands::ref_commands`). What every menu of a ref's commands asks,
/// the page's ☰ and a row's right-click alike.
pub async fn ref_commands(svc: &Services, uri: String) -> Result<Vec<String>, IpcError> {
    oxplow_app::commands::ref_commands::ref_commands(svc, &uri)
        .await
        .map_err(IpcError::invalid)
}

/// Window `client` (an id it mints when it starts) is open and hosts
/// `scopes` (`tabs.write`, …): a command the daemon runs over one
/// of them comes to it as a `clientCall` event addressed to it
/// (`oxplow_app::client_host`). Said when the window starts and again
/// when it reconnects.
pub async fn register_client_host(
    svc: &Services,
    client: String,
    scopes: Vec<String>,
) -> Result<(), IpcError> {
    svc.client_host.register(&client, scopes);
    Ok(())
}

/// Window `client` is closing: no more calls go to it, and those it
/// hadn't answered are refused now.
pub async fn unregister_client_host(svc: &Services, client: String) -> Result<(), IpcError> {
    svc.client_host.unregister(&client);
    Ok(())
}

/// Window `client`'s answer to client call `id`: its `result`, or the
/// `error` it couldn't do it for.
pub async fn answer_client_call(
    svc: &Services,
    client: String,
    id: String,
    result: Option<Json>,
    error: Option<String>,
) -> Result<(), IpcError> {
    let answer = match error {
        Some(message) => Err(message),
        None => Ok(result.map(|r| r.0).unwrap_or(serde_json::Value::Null)),
    };
    svc.client_host.answer(&client, &id, answer);
    Ok(())
}

/// Run command `id` while window `client` answers client call `call`, as
/// whoever ran that call — an agent's Save writes the file as the agent
/// (its write gate and its stream), never as the person at the window.
/// Not confirmed: a run that asks is the caller's to settle.
pub async fn run_command_for_call(
    svc: &Services,
    client: String,
    call: String,
    id: String,
    input: Json,
) -> Result<CommandOutcome, IpcError> {
    let actor = svc.client_host.caller(&client, &call)?;
    let outcome = svc.commands.run(&actor, &id, input.0, false).await?;
    if outcome.audit_id.is_some() {
        svc.efforts.settle_lifecycle().await;
    }
    Ok(outcome)
}

/// A command's spec — what a form renders from its `input_schema`, and
/// what a confirmation says (its summary, whether it's destructive).
pub async fn get_command(svc: &Services, name: String) -> Result<CommandSpec, IpcError> {
    svc.commands
        .spec(&name)
        .ok_or_else(|| IpcError::from(oxplow_domain::CommandError::Unknown { name }))
}

/// Apply the inverse recorded for audit row `audit_id`, as the person,
/// through the same pipeline.
pub async fn undo_command(
    svc: &Services,
    audit_id: AuditId,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = svc
        .commands
        .undo(&Actor::Human, audit_id.value(), confirmed)
        .await?;
    if outcome.audit_id.is_some() {
        svc.efforts.settle_lifecycle().await;
    }
    Ok(outcome)
}

/// A person decides proposal `proposal` (an agent's run that needed their
/// confirmation): approving runs it as them — confirmed, audited to them,
/// the outcome returned — declining records the decision and runs nothing
/// (`None`).
pub async fn decide_proposal(
    svc: &Services,
    proposal: ProposalId,
    approve: bool,
) -> Result<Option<CommandOutcome>, IpcError> {
    if !approve {
        svc.commands
            .decline(&Actor::Human, proposal.value())
            .await?;
        return Ok(None);
    }
    let outcome = svc
        .commands
        .approve(&Actor::Human, proposal.value())
        .await?;
    if outcome.audit_id.is_some() {
        svc.efforts.settle_lifecycle().await;
    }
    Ok(Some(outcome))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    /// P5.A1 (tsk519): the person runs a command over IPC. A human-only
    /// key needs their confirmation — refused without it, done with it —
    /// and the run is audited to a human.
    /// What a person can run now and is offered: every command with
    /// person-facing metadata the person may invoke — Pull, New Task —
    /// and none without (a step, an agent's tool).
    /// What a person's run opens next is bound here, by the bus's own
    /// tokenizer, from the run's result: a field the result lacks opens
    /// nothing and says why, rather than a page with an empty id.
    /// A person's run that fails says which audit row recorded it, so the
    /// window's record of the failure links the run.
    #[tokio::test]
    async fn a_failed_run_says_its_audit_row() {
        let (svc, _dir) = services().await;
        let err = crate::dispatch(
            "run_command",
            json!({
                "id": "oxplow.work_item.transition",
                "input": { "uri": "oxplow:oxplow/work_item?id=tsk999&provider=oxplow", "to": "done" },
                "confirmed": true,
            }),
            &svc,
        )
        .await
        .unwrap_err();
        let audit = err.audit_id.expect("the failed run's audit row").get();
        let row = svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT command, outcome FROM command_audit WHERE id = ?1",
                    [audit],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(row.0, "oxplow.work_item.transition");
        assert_ne!(row.1, "ok");
        // A run that succeeds, or fails before any run, names none.
        let unknown = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.nope", "input": {}, "confirmed": true }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(unknown.audit_id, None);
    }

    #[tokio::test]
    async fn the_page_a_run_opens_is_bound_from_its_result() {
        let (svc, _dir) = services().await;
        let open = |result: serde_json::Value| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "command_open_after",
                    json!({ "id": "oxplow.extension.new_dashboard", "result": result }),
                    &svc,
                )
                .await
            }
        };
        assert_eq!(
            open(json!({ "page": "my-pages/health" })).await.unwrap(),
            json!("oxplow:oxplow/page?page=my-pages/health")
        );
        let err = open(json!({})).await.unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(err.message.contains("`page`"), "{}", err.message);
        let none = crate::dispatch(
            "command_open_after",
            json!({ "id": "oxplow.vcs.pull", "result": {} }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(none, serde_json::Value::Null);
    }

    /// A command about a kind is offered on a ref of it only while its
    /// `about_when` column holds on the ref's row: Reopen Session on a
    /// session a person closed, not on an open one; a command without a
    /// condition (Hand Off Work on a thread) on every ref of its kind.
    #[tokio::test]
    async fn an_about_command_is_offered_while_its_condition_holds() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .selected_or_first(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a thread");
        let run = |id: &str, input: serde_json::Value| {
            let svc = svc.clone();
            let id = id.to_string();
            async move {
                crate::dispatch(
                    "run_command",
                    json!({ "id": id, "input": input, "confirmed": false }),
                    &svc,
                )
                .await
                .unwrap()
            }
        };
        let offered = |uri: String| {
            let svc = svc.clone();
            async move {
                let ids = crate::dispatch("ref_commands", json!({ "uri": uri }), &svc)
                    .await
                    .unwrap();
                ids.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<String>>()
            }
        };
        let opened = run(
            "oxplow.agent_session.open",
            json!({ "thread": thread.to_string() }),
        )
        .await;
        let session = opened["result"]["id"].as_str().unwrap().to_string();
        let uri = oxplow_domain::uri::build::agent_session_uri(session.parse().unwrap());
        let reopen = "oxplow.agent_session.reopen".to_string();

        assert!(!offered(uri.clone()).await.contains(&reopen), "open");
        run("oxplow.agent_session.close", json!({ "session": session })).await;
        assert!(
            offered(uri.clone()).await.contains(&reopen),
            "closed by a person"
        );
        run("oxplow.agent_session.reopen", json!({ "session": session })).await;
        assert!(!offered(uri).await.contains(&reopen), "open again");

        assert_eq!(
            offered("https://example.com".into()).await,
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn a_person_is_offered_the_commands_with_a_label() {
        let (svc, _dir) = services().await;
        let listed = crate::dispatch("list_person_commands", json!({}), &svc)
            .await
            .unwrap();
        let by_id = |id: &str| {
            listed
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .cloned()
        };
        let pull = by_id("oxplow.vcs.pull").expect("pull is offered");
        assert_eq!(pull["ui"]["label"], "Pull Changes");
        assert_eq!(pull["ui"]["group"], "Git");
        assert_eq!(pull["ui"]["input"], json!({ "stream": "{{stream}}" }));
        assert_eq!(pull["ui"]["background"], true);
        let dashboard = by_id("oxplow.extension.new_dashboard").expect("New Dashboard is offered");
        assert_eq!(
            dashboard["ui"]["open_after"],
            "oxplow:oxplow/page?page={{result.page}}"
        );
        assert!(
            by_id("oxplow.config.get").is_none(),
            "no label: not offered"
        );
        assert!(
            listed
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["ui"].is_object()),
            "{listed}"
        );
    }

    #[tokio::test]
    async fn a_person_runs_a_command_confirming_where_asked() {
        let (svc, dir) = services().await;
        let set = |confirmed: bool| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "run_command",
                    json!({
                        "id": "oxplow.config.set",
                        "input": { "key": "agentPromptAppend", "value": "Be brief." },
                        "confirmed": confirmed,
                    }),
                    &svc,
                )
                .await
            }
        };
        // A command that asks asks the person over IPC: a work item's delete.
        let made = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.work_item.create", "input": { "title": "T" }, "confirmed": false }),
            &svc,
        )
        .await
        .unwrap();
        let delete = |confirmed: bool| {
            let (svc, uri) = (svc.clone(), made["result"]["uri"].clone());
            async move {
                crate::dispatch(
                    "run_command",
                    json!({ "id": "oxplow.work_item.delete", "input": { "uri": uri }, "confirmed": confirmed }),
                    &svc,
                )
                .await
            }
        };
        let err = delete(false).await.unwrap_err();
        assert_eq!(err.code, "NEEDS_CONFIRMATION", "{}", err.message);
        delete(true).await.unwrap();
        // One that doesn't ask runs at once.
        let out = set(false).await.unwrap();
        assert!(out["audit_id"].is_number(), "{out}");
        let file = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        assert!(file.contains("Be brief."), "{file}");
        let audit = oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
            .get(out["audit_id"].as_i64().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(audit.actor_kind).unwrap(),
            json!("human")
        );

        // Undo runs the recorded inverse through the same pipeline.
        let undone = crate::dispatch(
            "undo_command",
            json!({ "auditId": out["audit_id"], "confirmed": true }),
            &svc,
        )
        .await
        .unwrap();
        assert!(undone["audit_id"].is_number(), "{undone}");
        let file = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        assert!(!file.contains("Be brief."), "{file}");
    }

    /// P6b.A3: an effect's run that asks waits as a proposal; the person
    /// approves it over IPC (it runs as them) or declines it.
    #[tokio::test]
    async fn a_person_decides_an_effects_proposal() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (svc, _dir) = services().await;
        let stream = svc.stream_store.list().await.unwrap().pop().unwrap();
        let thread = svc
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let effect = oxplow_domain::Actor::Effect {
            effect: "acme/notify".into(),
            thread_id: Some(thread.id),
            stream_id: Some(stream.id),
        };
        let made = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                json!({ "title": "Proposed away" }),
                false,
            )
            .await
            .unwrap();
        let propose = || {
            let (svc, effect) = (svc.clone(), effect.clone());
            let uri = made.result["uri"].clone();
            async move {
                let err = svc
                    .commands
                    .run(
                        &effect,
                        "oxplow.work_item.delete",
                        json!({ "uri": uri }),
                        false,
                    )
                    .await
                    .unwrap_err();
                let oxplow_domain::CommandError::Proposed { proposal, .. } = err else {
                    panic!("{err:?}");
                };
                proposal
            }
        };
        let decide = |id: i64, approve: bool| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "decide_proposal",
                    json!({ "proposal": id, "approve": approve }),
                    &svc,
                )
                .await
            }
        };
        let exists = || {
            let svc = svc.clone();
            async move {
                svc.commands
                    .run(
                        &oxplow_domain::Actor::Human,
                        "oxplow.sql.query",
                        json!({ "sql": "SELECT count(*) FROM v_work_item WHERE title = 'Proposed away'" }),
                        false,
                    )
                    .await
                    .unwrap()
                    .result["rows"][0][0]
                    .clone()
            }
        };

        let declined = propose().await;
        assert_eq!(decide(declined, false).await.unwrap(), json!(null));
        assert_eq!(exists().await, json!(1), "declined: still there");

        let approved = propose().await;
        let out = decide(approved, true).await.unwrap();
        assert!(out["audit_id"].is_number(), "{out}");
        assert_eq!(exists().await, json!(0), "approved: deleted as the person");
        // Decided once.
        let err = decide(approved, false).await.unwrap_err();
        assert_eq!(err.code, "INVALID", "{}", err.message);
    }
}
