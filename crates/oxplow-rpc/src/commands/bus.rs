//! The desktop's way onto the command bus (P5.A1, `.context/commands.md`
//! "Callers"): run any command as the person, and undo a run. A typed
//! IPC setter is a convenience over one command; anything new the UI
//! writes goes through here, so the bus's validation, policy,
//! confirmation, audit and undo apply once.

use oxplow_app::Services;
use oxplow_domain::{Actor, CommandOutcome, CommandSpec, Json};

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

/// The commands a person is offered: those they may run now (invokers,
/// needs active) that say how a person meets them (`ui`) — what search,
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

/// The window is open and hosts `capabilities` (`tabs.write`, …): a
/// command the daemon runs over one of them comes to it as a
/// `clientCall` event (`oxplow_app::client_host`). Said when the window
/// starts and again when it reconnects.
pub async fn register_client_host(
    svc: &Services,
    capabilities: Vec<String>,
) -> Result<(), IpcError> {
    svc.client_host.register(capabilities);
    Ok(())
}

/// The window's answer to client call `id`: its `result`, or the `error`
/// it couldn't do it for.
pub async fn answer_client_call(
    svc: &Services,
    id: String,
    result: Option<Json>,
    error: Option<String>,
) -> Result<(), IpcError> {
    let answer = match error {
        Some(message) => Err(message),
        None => Ok(result.map(|r| r.0).unwrap_or(serde_json::Value::Null)),
    };
    svc.client_host.answer(&id, answer);
    Ok(())
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
    audit_id: i64,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = svc
        .commands
        .undo(&Actor::Human, audit_id, confirmed)
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
    proposal: i64,
    approve: bool,
) -> Result<Option<CommandOutcome>, IpcError> {
    if !approve {
        svc.commands.decline(&Actor::Human, proposal).await?;
        return Ok(None);
    }
    let outcome = svc.commands.approve(&Actor::Human, proposal).await?;
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
    #[tokio::test]
    async fn a_person_is_offered_the_commands_with_a_label() {
        let (svc, _dir) = services();
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
        let new_task = by_id("oxplow.work_item.create").expect("New Task is offered");
        assert_eq!(new_task["ui"]["form"], "page:new-task");
        let dashboard = by_id("oxplow.dashboard.create").expect("New Dashboard is offered");
        assert_eq!(
            dashboard["ui"]["open_after"],
            "page:custom-dashboard?id={{result.id}}"
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
        let (svc, dir) = services();
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
        let err = set(false).await.unwrap_err();
        assert_eq!(err.code, "NEEDS_CONFIRMATION", "{}", err.message);
        let out = set(true).await.unwrap();
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

    /// P6b.A3: an agent's change to a person-only key waits as a proposal;
    /// the person approves it over IPC (it runs as them) or declines it.
    #[tokio::test]
    async fn a_person_decides_an_agents_proposal() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (svc, dir) = services();
        let stream = svc.stream_store.list().await.unwrap().pop().unwrap();
        let thread = svc
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(thread.id),
            stream_id: Some(stream.id),
        };
        let propose = |value: &'static str| {
            let (svc, agent) = (svc.clone(), agent.clone());
            async move {
                let err = svc
                    .commands
                    .run(
                        &agent,
                        "oxplow.config.set",
                        json!({ "key": "agentPromptAppend", "value": value }),
                        false,
                    )
                    .await
                    .unwrap_err();
                let oxplow_domain::CommandError::Proposed { proposal, .. } = err else {
                    panic!("{err:?}");
                };
                proposal
                    .strip_prefix("proposal:")
                    .unwrap()
                    .parse::<i64>()
                    .unwrap()
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
        let file =
            || std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap_or_default();

        let declined = propose("No.").await;
        assert_eq!(decide(declined, false).await.unwrap(), json!(null));
        assert!(!file().contains("No."), "{}", file());

        let approved = propose("Be brief.").await;
        let out = decide(approved, true).await.unwrap();
        assert!(out["audit_id"].is_number(), "{out}");
        assert!(file().contains("Be brief."), "{}", file());
        // Decided once.
        let err = decide(approved, false).await.unwrap_err();
        assert_eq!(err.code, "INVALID", "{}", err.message);
    }
}
