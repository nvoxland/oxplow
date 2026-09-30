//! The desktop's way onto the command bus (P5.A1, `.context/commands.md`
//! "Callers"): run any command as the person, and undo a run. A typed
//! IPC setter is a convenience over one command; anything new the UI
//! writes goes through here, so the bus's validation, policy,
//! confirmation, audit and undo apply once.

use oxplow_app::Services;
use oxplow_domain::{Actor, CommandOutcome, CommandSpec, Json};

use crate::error::IpcError;

/// Run `name` with `input` as the person. `confirmed` says the person
/// confirmed this exact call (a destructive command, a human-only config
/// key); an unconfirmed call that needs one comes back
/// `NEEDS_CONFIRMATION`, so the UI asks and calls again.
pub async fn run_command(
    svc: &Services,
    name: String,
    input: Json,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = svc
        .commands
        .run(&Actor::Human, &name, input.0, confirmed)
        .await?;
    // A write may have opened or closed an effort: let its snapshot pin
    // land before the UI reads what follows.
    if outcome.audit_id.is_some() {
        svc.tasks.settle_lifecycle().await;
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
    audit_id: i64,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = svc
        .commands
        .undo(&Actor::Human, audit_id, confirmed)
        .await?;
    if outcome.audit_id.is_some() {
        svc.tasks.settle_lifecycle().await;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    /// P5.A1 (tsk519): the person runs a command over IPC. A human-only
    /// key needs their confirmation — refused without it, done with it —
    /// and the run is audited to a human.
    #[tokio::test]
    async fn a_person_runs_a_command_confirming_where_asked() {
        let (svc, dir) = services();
        let set = |confirmed: bool| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "run_command",
                    json!({
                        "name": "config.set",
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
}
