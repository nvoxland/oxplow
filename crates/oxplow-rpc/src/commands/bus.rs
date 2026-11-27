//! The desktop's way onto the command bus (P5.A1, `.context/commands.md`
//! "Callers"): run any command as the person, and undo a run. A typed
//! IPC setter is a convenience over one command; anything new the UI
//! writes goes through here, so the bus's validation, policy,
//! confirmation, the command run record and undo apply once.

use oxplow_app::Services;
use oxplow_domain::{Actor, CommandOutcome, CommandSpec, Json};
use oxplow_domain::{CommandRunId, ProposalId};

use crate::error::IpcError;
use crate::RpcRequest;

/// Run command `id` with `input` as the person. `confirmed` says the person
/// confirmed this exact call (a destructive command, a human-only config
/// key); an unconfirmed call that needs one comes back
/// `NEEDS_CONFIRMATION`, so the UI asks and calls again.
pub async fn run_command(
    req: &RpcRequest<'_>,
    id: String,
    input: Json,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = req
        .commands
        .run_in(&persons_run(req), &id, input.0, confirmed)
        .await?;
    Ok(outcome)
}

/// A person's run from `req`: its command run keeps the device they asked
/// from (none for a request with no caller).
fn persons_run(req: &RpcRequest<'_>) -> oxplow_domain::run::RunContext {
    oxplow_domain::run::RunContext::core(Actor::Human, oxplow_domain::run::Mode::External)
        .with_caller(req.caller.cloned())
}

/// A read the person makes through the bus: command `id` run as the
/// person, its result decoded. A typed IPC read the agent has too is
/// this, so both front doors validate and answer alike. Its record keeps
/// the device the person asked from.
pub(crate) async fn read_as_person<T: serde::de::DeserializeOwned>(
    req: &RpcRequest<'_>,
    id: &str,
    input: serde_json::Value,
) -> Result<T, IpcError> {
    let outcome = req
        .commands
        .run_in(&persons_run(req), id, input, false)
        .await?;
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
    // An extension's command answers `{ result, children }` (the composed
    // run's shape): the page binds from its own answer.
    let answer = match &result.0 {
        serde_json::Value::Object(o) if o.contains_key("children") => {
            o.get("result").unwrap_or(&result.0)
        }
        other => other,
    };
    spec.ui
        .as_ref()
        .map_or(Ok(None), |ui| ui.open_after_for(answer))
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

/// What action point `point` offers for `context`: the commands that
/// joined it, each with the context as its input, and how to draw them
/// (`oxplow_app::commands::action_point_offers`).
pub async fn action_point_commands(
    svc: &Services,
    point: String,
    context: Json,
) -> Result<oxplow_app::commands::action_point_offers::PointOffer, IpcError> {
    oxplow_app::commands::action_point_offers::action_point_offer(svc, &point, &context.0)
        .await
        .map_err(IpcError::invalid)
}

/// Window `client` (an id it mints when it starts) is open and hosts
/// `scopes` (`tabs.write`, …): a command the daemon runs over one
/// of them comes to it as a `clientCall` event addressed to it
/// (`oxplow_app::client_host`). Said when the window starts and again
/// when it reconnects.
pub async fn register_client_host(
    req: &RpcRequest<'_>,
    scopes: Vec<String>,
) -> Result<(), IpcError> {
    req.client_host.register(req.connection()?.as_str(), scopes);
    Ok(())
}

/// Window `client` is closing: no more calls go to it, and those it
/// hadn't answered are refused now.
pub async fn unregister_client_host(req: &RpcRequest<'_>) -> Result<(), IpcError> {
    req.client_host.unregister(req.connection()?.as_str());
    Ok(())
}

/// Window `client`'s answer to client call `id`: its `result`, or the
/// `error` it couldn't do it for.
pub async fn answer_client_call(
    req: &RpcRequest<'_>,
    id: String,
    result: Option<Json>,
    error: Option<String>,
) -> Result<(), IpcError> {
    let answer = match error {
        Some(message) => Err(message),
        None => Ok(result.map(|r| r.0).unwrap_or(serde_json::Value::Null)),
    };
    req.client_host
        .answer(req.connection()?.as_str(), &id, answer);
    Ok(())
}

/// Run command `id` while window `client` answers client call `call`, as
/// whoever ran that call — an agent's Save writes the file as the agent
/// (its write gate and its stream), never as the person at the window.
/// Not confirmed: a run that asks is the caller's to settle.
pub async fn run_command_for_call(
    req: &RpcRequest<'_>,
    call: String,
    id: String,
    input: Json,
) -> Result<CommandOutcome, IpcError> {
    // As the call's actor, and with the device its run was asked from.
    let run = req.client_host.run_for(req.connection()?.as_str(), &call)?;
    let outcome = req.commands.run_in(&run, &id, input.0, false).await?;
    Ok(outcome)
}

/// A command's spec — what a form renders from its `input_schema`, and
/// what a confirmation says (its summary, whether it's destructive).
pub async fn get_command(svc: &Services, name: String) -> Result<CommandSpec, IpcError> {
    svc.commands
        .spec(&name)
        .ok_or_else(|| IpcError::from(oxplow_domain::CommandError::Unknown { name }))
}

/// Apply the inverse recorded for command run `command_run_id`, as the person,
/// through the same pipeline.
pub async fn undo_command(
    req: &RpcRequest<'_>,
    command_run_id: CommandRunId,
    confirmed: bool,
) -> Result<CommandOutcome, IpcError> {
    let outcome = req
        .commands
        .undo_in(&persons_run(req), command_run_id.value(), confirmed)
        .await?;
    Ok(outcome)
}

/// A person decides proposal `proposal` (an agent's run that needed their
/// confirmation): approving runs it as them — confirmed, audited to them,
/// the outcome returned — declining records the decision and runs nothing
/// (`None`).
pub async fn decide_proposal(
    req: &RpcRequest<'_>,
    proposal: ProposalId,
    approve: bool,
) -> Result<Option<CommandOutcome>, IpcError> {
    if !approve {
        req.commands
            .decline(&Actor::Human, proposal.value())
            .await?;
        return Ok(None);
    }
    let outcome = req
        .commands
        .approve_in(&persons_run(req), proposal.value())
        .await?;
    Ok(Some(outcome))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    /// P5.A1: the person runs a command over IPC. A human-only
    /// key needs their confirmation — refused without it, done with it —
    /// and the run is audited to a human.
    /// What a person can run now and is offered: every command with
    /// person-facing metadata the person may invoke — Pull, New Task —
    /// and none without (a step, an agent's tool).
    /// What a person's run opens next is bound here, by the bus's own
    /// tokenizer, from the run's result: a field the result lacks opens
    /// nothing and says why, rather than a page with an empty id.
    /// A person's run keeps the device it was asked from, and so does its
    /// undo; a run with no caller keeps none.
    #[tokio::test]
    async fn a_persons_run_and_its_undo_keep_the_device_that_asked() {
        let (svc, _dir) = services().await;
        let caller = crate::test_support::caller();
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .selected_or_first(None, &stream.id)
            .await
            .unwrap()
            .unwrap();
        let mark = |mark: &str| {
            json!({
                "id": "oxplow.thread.mark",
                "input": { "thread": thread.to_string(), "mark": mark },
                "confirmed": true,
            })
        };
        let marked = crate::dispatch_as("run_command", mark("amber"), &svc, Some(&caller))
            .await
            .unwrap();
        let run_id = marked["command_run_id"].as_i64().expect("an audited run");
        crate::dispatch_as(
            "undo_command",
            json!({ "commandRunId": run_id, "confirmed": true }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        let anonymous = crate::dispatch("run_command", mark("red"), &svc)
            .await
            .unwrap();
        let rows = crate::dispatch(
            "query_sql",
            json!({ "sql": format!(
                "SELECT id, device_id FROM v_command_run WHERE id >= {run_id} AND command = 'oxplow.thread.mark' ORDER BY id"
            ) }),
            &svc,
        )
        .await
        .unwrap();
        let device = caller.device.to_string();
        let rows = rows["rows"].as_array().unwrap().clone();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0][1], json!(device), "the run");
        assert_eq!(rows[1][1], json!(device), "its undo");
        assert_eq!(rows[2][0], anonymous["command_run_id"]);
        assert_eq!(rows[2][1], serde_json::Value::Null, "no caller, no device");
    }

    /// A person's typed read (search) is recorded with the device it came
    /// from, as their writes are.
    #[tokio::test]
    async fn a_persons_read_keeps_the_device_that_asked() {
        let (svc, _dir) = services().await;
        let caller = crate::test_support::caller();
        crate::dispatch_as(
            "search",
            json!({ "query": "anything" }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        let rows = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT device_id FROM v_command_run WHERE command = 'oxplow.search.find'" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(rows["rows"], json!([[caller.device.to_string()]]));
    }

    /// A person's run that fails says which command run record recorded it, so the
    /// window's record of the failure links the run.
    #[tokio::test]
    async fn a_failed_run_says_its_command_run() {
        let (svc, _dir) = services().await;
        let err = crate::dispatch(
            "run_command",
            json!({
                "id": "oxplow.thread.mark",
                "input": { "thread": "thr999", "mark": "amber" },
                "confirmed": true,
            }),
            &svc,
        )
        .await
        .unwrap_err();
        let audit = err
            .command_run_id
            .expect("the failed run's command run record")
            .get();
        let row = svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT command, outcome FROM v_command_run WHERE id = ?1",
                    [audit],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(row.0, "oxplow.thread.mark");
        assert_ne!(row.1, "ok");
        // A run that succeeds, or fails before any run, names none.
        let unknown = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.nope", "input": {}, "confirmed": true }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(unknown.command_run_id, None);
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
        // An extension's command answers `{ result, children }`: the page
        // is bound from its own answer, not the envelope.
        assert_eq!(
            open(json!({ "result": { "page": "my-pages/health" }, "children": [] }))
                .await
                .unwrap(),
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
            .selected_or_first(None, &stream.id)
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
            json!({ "thread": thread.to_string(), "harness": "claude" }),
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
                        "input": { "key": "oxplow.agentPromptAppend", "value": "Be brief.", "level": "project" },
                        "confirmed": confirmed,
                    }),
                    &svc,
                )
                .await
            }
        };
        // A command that asks asks the person over IPC: a file's delete.
        std::fs::write(dir.path().join("old.txt"), "old").unwrap();
        let delete = |confirmed: bool| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "run_command",
                    json!({ "id": "oxplow.file.delete", "input": { "stream": "str1", "path": "old.txt" }, "confirmed": confirmed }),
                    &svc,
                )
                .await
            }
        };
        let err = delete(false).await.unwrap_err();
        assert_eq!(err.code, "NEEDS_CONFIRMATION", "{}", err.message);
        assert!(dir.path().join("old.txt").exists(), "asked, so not deleted");
        delete(true).await.unwrap();
        assert!(!dir.path().join("old.txt").exists());
        // One that doesn't ask runs at once.
        let out = set(false).await.unwrap();
        assert!(out["command_run_id"].is_number(), "{out}");
        let file = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        assert!(file.contains("Be brief."), "{file}");
        let audit = oxplow_db::SqliteCommandRunStore::new(svc.db.clone())
            .get(out["command_run_id"].as_i64().unwrap())
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
            json!({ "commandRunId": out["command_run_id"], "confirmed": true }),
            &svc,
        )
        .await
        .unwrap();
        assert!(undone["command_run_id"].is_number(), "{undone}");
        let file = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        assert!(!file.contains("Be brief."), "{file}");
    }

    /// P6b.A3: an effect's run that asks waits as a proposal; the person
    /// approves it over IPC (it runs as them) or declines it.
    #[tokio::test]
    async fn a_person_decides_an_effects_proposal() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (svc, dir) = services().await;
        std::fs::write(dir.path().join("proposed.txt"), "x").unwrap();
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
        let propose = || {
            let (svc, effect) = (svc.clone(), effect.clone());
            async move {
                let err = svc
                    .commands
                    .run(
                        &effect,
                        "oxplow.file.delete",
                        json!({ "stream": "str1", "path": "proposed.txt" }),
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
        let exists = || dir.path().join("proposed.txt").exists();

        let declined = propose().await;
        assert_eq!(decide(declined, false).await.unwrap(), json!(null));
        assert!(exists(), "declined: still there");

        let approved = propose().await;
        let out = decide(approved, true).await.unwrap();
        assert!(out["command_run_id"].is_number(), "{out}");
        assert!(!exists(), "approved: deleted as the person");
        // Decided once.
        let err = decide(approved, false).await.unwrap_err();
        assert_eq!(err.code, "INVALID", "{}", err.message);
    }
}
