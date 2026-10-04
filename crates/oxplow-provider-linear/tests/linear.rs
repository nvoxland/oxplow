//! P7.A5: the Linear provider over real stdio, against the simulator —
//! its declarations, `check`, what each verb sends and records, the
//! paging read and its cursor, and a rate limit.

#![allow(clippy::unwrap_used)]

use std::process::Stdio;

use oxplow_provider_linear::sim::{LinearSim, Request, PROJECT_ID, TEAM_ID};
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::process::{Child, Command};

const KEY: &str = "lin_api_test";

fn spawn(sim: &LinearSim, key: Option<&str>) -> (Child, Peer) {
    spawn_as(sim, key, "linear")
}

/// As the provider its manifest calls `id` (`OXPLOW_PROVIDER_ID`).
fn spawn_as(sim: &LinearSim, key: Option<&str>, id: &str) -> (Child, Peer) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-linear"));
    cmd.env_clear()
        .env("LINEAR_API_URL", &sim.url)
        .env("OXPLOW_PROVIDER_ID", id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    if let Some(key) = key {
        cmd.env("LINEAR_API_KEY", key);
    }
    let mut child = cmd.spawn().expect("spawn the provider");
    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");
    let (peer, _incoming) = Peer::spawn(stdout, stdin);
    (child, peer)
}

async fn initialize(peer: &Peer) -> InitializeResult {
    peer.call(
        method::INITIALIZE,
        &InitializeParams {
            protocol_version: PROTOCOL_VERSION.into(),
            host: Party {
                name: "test".into(),
                version: "0".into(),
            },
        },
    )
    .await
    .expect("initialize")
}

async fn check_with(peer: &Peer, config: Value, credentials: &[&str]) -> CheckResult {
    peer.call(
        method::CHECK,
        &CheckParams {
            config,
            credentials: credentials.iter().map(|c| c.to_string()).collect(),
        },
    )
    .await
    .expect("check answers")
}

async fn checked(peer: &Peer) -> Handle {
    let r = check_with(peer, json!({ "team": "ENG" }), &["LINEAR_API_KEY"]).await;
    assert_eq!(r.problems, vec![]);
    r.handle.expect("a handle")
}

async fn invoke(
    peer: &Peer,
    handle: &Handle,
    command: &str,
    input: Value,
) -> Result<InvokeResult, ProtocolError> {
    keyed(peer, handle, command, input, None).await
}

/// [`invoke`], sent with an idempotency key.
async fn keyed(
    peer: &Peer,
    handle: &Handle,
    command: &str,
    input: Value,
    key: Option<&str>,
) -> Result<InvokeResult, ProtocolError> {
    peer.call(
        method::INVOKE,
        &InvokeParams {
            handle: handle.clone(),
            command: command.into(),
            input,
            idempotency_key: key.map(str::to_string),
        },
    )
    .await
}

/// The record a write's one event carries.
fn row(result: &InvokeResult) -> Value {
    assert_eq!(result.events.len(), 1, "{result:?}");
    result.events[0].payload["item"].clone()
}

fn ops(sim: &LinearSim) -> Vec<String> {
    sim.requests().into_iter().map(|r| r.operation).collect()
}

/// The streamed notifications of a `read`, and its result.
async fn read(peer: &Peer, handle: &Handle, state: Option<Value>) -> (Vec<(String, Value)>, Value) {
    let (call, mut rx) = peer
        .start_streaming(
            method::READ,
            serde_json::to_value(ReadParams {
                handle: handle.clone(),
                collector: "issues".into(),
                state,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let result = call.reply().await.expect("read");
    let mut streamed = Vec::new();
    while let Ok(Incoming::Notification { method, params }) = rx.try_recv() {
        streamed.push((method, params));
    }
    (streamed, result)
}

#[test]
fn the_examples_provider_json_is_its_declarations() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/linear/provider.json"
    );
    if std::env::var_os("OXPLOW_BLESS").is_some() {
        let json = serde_json::to_string_pretty(&oxplow_provider_linear::declarations()).unwrap();
        std::fs::write(path, json + "\n").unwrap();
    }
    let on_disk: InitializeResult =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(on_disk, oxplow_provider_linear::declarations());
}

#[tokio::test]
async fn check_needs_the_key_a_team_and_its_blocked_state() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    assert_eq!(
        initialize(&peer).await,
        oxplow_provider_linear::declarations()
    );
    let problem = |r: CheckResult| {
        assert!(r.handle.is_none(), "{r:?}");
        r.problems.into_iter().map(|p| p.path).collect::<Vec<_>>()
    };
    let key = ["LINEAR_API_KEY"];
    assert_eq!(
        problem(check_with(&peer, json!({ "team": "ENG" }), &[]).await),
        [""]
    );
    assert_eq!(problem(check_with(&peer, json!({}), &key).await), ["/team"]);
    assert_eq!(
        problem(check_with(&peer, json!({ "team": "OPS" }), &key).await),
        ["/team"]
    );
    assert_eq!(
        problem(
            check_with(
                &peer,
                json!({ "team": "ENG", "blocked_state": "Stuck" }),
                &key
            )
            .await
        ),
        ["/blocked_state"]
    );
    assert_eq!(
        problem(check_with(&peer, json!({ "team": "ENG", "project": "Nope" }), &key).await),
        ["/project"]
    );
    assert_eq!(
        problem(check_with(&peer, json!({ "team": "ENG", "token": "x" }), &key).await),
        ["/token"],
        "a secret never rides in config"
    );
    let ok = check_with(&peer, json!({ "team": "ENG", "project": "Roadmap" }), &key).await;
    assert_eq!(ok.handle, Some(Handle("linear:ENG/Roadmap".into())));

    // A key Linear refuses is a problem, not a crash.
    let (_child, wrong) = spawn(&sim, Some("lin_api_wrong"));
    initialize(&wrong).await;
    let refused = check_with(&wrong, json!({ "team": "ENG" }), &key).await;
    assert_eq!(problem(refused), [""]);
}

#[tokio::test]
async fn each_verb_sends_its_graphql_and_records_the_issue() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    initialize(&peer).await;
    let handle = checked(&peer).await;
    sim.clear_requests();

    // create → issueCreate in the team, the body as its description.
    let created = invoke(
        &peer,
        &handle,
        "create",
        json!({ "title": "First", "body": "why", "native": { "priority": 2 } }),
    )
    .await
    .unwrap();
    assert_eq!(created.result, json!({ "ref": "work_item:linear:ENG-1" }));
    assert_eq!(
        sim.requests(),
        [Request {
            operation: "IssueCreate".into(),
            variables: json!({ "input": { "teamId": TEAM_ID, "title": "First",
                                          "description": "why", "priority": 2 } }),
        }]
    );
    let first = row(&created);
    assert_eq!(
        (
            first["state"].clone(),
            first["native_state"].clone(),
            first["body"].clone()
        ),
        (json!("todo"), json!("Backlog"), json!("why"))
    );
    assert_eq!(
        first["native"]["id"],
        "00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(first["native"]["priority"], 2);
    invoke(
        &peer,
        &handle,
        "create",
        json!({ "title": "Second", "parent_ref": "work_item:linear:ENG-1" }),
    )
    .await
    .unwrap();
    // A parent is an input-object field: Linear wants its uuid, looked up.
    assert_eq!(ops(&sim)[ops(&sim).len() - 2..], ["Issue", "IssueCreate"]);
    assert_eq!(
        sim.requests().last().unwrap().variables["input"]["parentId"],
        "00000000-0000-4000-8000-000000000001"
    );

    // update → issueUpdate; "" detaches the parent.
    sim.clear_requests();
    let updated = invoke(
        &peer,
        &handle,
        "update",
        json!({ "ref": "work_item:linear:ENG-2", "title": "Renamed", "parent_ref": "" }),
    )
    .await
    .unwrap();
    assert_eq!(
        sim.requests(),
        [Request {
            operation: "IssueUpdate".into(),
            variables: json!({ "id": "ENG-2", "input": { "title": "Renamed", "parentId": null } }),
        }]
    );
    assert_eq!(row(&updated)["parent_ref"], Value::Null);

    // transition → the issue's state first (the inverse), then issueUpdate.
    sim.clear_requests();
    let moved = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:linear:ENG-1", "to": "blocked" }),
    )
    .await
    .unwrap();
    assert_eq!(ops(&sim), ["Issue", "IssueUpdate"]);
    assert_eq!(
        sim.requests()[1].variables,
        json!({ "id": "ENG-1", "input": { "stateId": "state-4" } })
    );
    assert_eq!(
        (
            row(&moved)["state"].clone(),
            row(&moved)["native_state"].clone()
        ),
        (json!("blocked"), json!("Blocked"))
    );
    assert_eq!(
        moved.inverse,
        Some(CommandCall {
            command: "transition".into(),
            input: json!({ "ref": "work_item:linear:ENG-1", "to": "todo", "native_state": "Backlog" }),
        })
    );
    let wrong = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:linear:ENG-1", "to": "done", "native_state": "Duplicate" }),
    )
    .await;
    assert!(
        matches!(&wrong, Err(ProtocolError::InvalidInput { field, .. }) if field == "/native_state"),
        "{wrong:?}"
    );

    // link → issueRelationCreate, oxplow's link types as Linear's.
    sim.clear_requests();
    invoke(
        &peer,
        &handle,
        "link",
        json!({ "ref": "work_item:linear:ENG-1",
        "target": "work_item:linear:ENG-2", "link_type": "relates_to" }),
    )
    .await
    .unwrap();
    assert_eq!(
        sim.requests()[2].variables,
        json!({ "input": { "issueId": "00000000-0000-4000-8000-000000000001",
                           "relatedIssueId": "00000000-0000-4000-8000-000000000002",
                           "type": "related" } })
    );
    assert_eq!(ops(&sim), ["Issue", "Issue", "IssueRelationCreate"]);
    assert_eq!(
        sim.relations(),
        [(
            "ENG-1".to_string(),
            "ENG-2".to_string(),
            "related".to_string()
        )]
    );

    // comment → commentCreate.
    sim.clear_requests();
    invoke(
        &peer,
        &handle,
        "comment",
        json!({ "ref": "work_item:linear:ENG-1", "body": "noted" }),
    )
    .await
    .unwrap();
    assert_eq!(ops(&sim), ["Issue", "CommentCreate"]);
    assert_eq!(
        sim.requests()[1].variables["input"]["issueId"],
        "00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(sim.comments("ENG-1"), ["noted"]);

    // delete → the issue, then issueDelete; recorded deleted.
    sim.clear_requests();
    let deleted = invoke(
        &peer,
        &handle,
        "delete",
        json!({ "ref": "work_item:linear:ENG-2" }),
    )
    .await
    .unwrap();
    assert_eq!(ops(&sim), ["Issue", "IssueDelete"]);
    assert_eq!(row(&deleted)["deleted"], true);

    // A ref that isn't Linear's, or an issue that isn't there, is the input's fault.
    let foreign = invoke(
        &peer,
        &handle,
        "comment",
        json!({ "ref": "work_item:oxplow:tsk1", "body": "x" }),
    )
    .await;
    assert!(
        matches!(&foreign, Err(ProtocolError::InvalidInput { field, .. }) if field == "/ref"),
        "{foreign:?}"
    );
    let gone = invoke(
        &peer,
        &handle,
        "comment",
        json!({ "ref": "work_item:linear:ENG-9", "body": "x" }),
    )
    .await;
    assert!(
        matches!(&gone, Err(ProtocolError::InvalidInput { field, .. }) if field == "/ref"),
        "{gone:?}"
    );
}

#[tokio::test]
async fn a_read_pages_by_update_and_checkpoints_after_each_page() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    initialize(&peer).await;
    let handle = checked(&peer).await;
    for title in ["a", "b", "c"] {
        invoke(&peer, &handle, "create", json!({ "title": title }))
            .await
            .unwrap();
    }
    sim.set_max_page(2);
    sim.clear_requests();

    let (streamed, result) = read(&peer, &handle, None).await;
    assert_eq!(result, json!({ "records": 3 }));
    let shape: Vec<String> = streamed
        .iter()
        .map(|(m, p)| match m.as_str() {
            notify::RECORD => format!("record {}", p["row"]["ref"].as_str().unwrap()),
            notify::STATE => format!("state {}", p["state"]),
            notify::PROGRESS => format!("progress {}", p["message"].as_str().unwrap()),
            other => other.to_string(),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "progress issues: page 1",
            "record work_item:linear:ENG-1",
            "record work_item:linear:ENG-2",
            r#"state {"since":null,"after":"ENG-2","until":"2026-01-01T00:00:02.000Z"}"#,
            "progress issues: page 2",
            "record work_item:linear:ENG-3",
            r#"state {"since":"2026-01-01T00:00:03.000Z"}"#,
        ]
    );
    assert_eq!(
        sim.requests()[0].variables,
        json!({ "filter": { "team": { "id": { "eq": TEAM_ID } } }, "first": 50, "after": null })
    );

    // From its last checkpoint a read finds nothing new, then only what changed.
    let cursor = streamed.last().unwrap().1["state"].clone();
    let (_, again) = read(&peer, &handle, Some(cursor.clone())).await;
    assert_eq!(again, json!({ "records": 0 }));
    invoke(
        &peer,
        &handle,
        "update",
        json!({ "ref": "work_item:linear:ENG-1", "title": "a2" }),
    )
    .await
    .unwrap();
    let (changed, _) = read(&peer, &handle, Some(cursor)).await;
    let refs: Vec<&str> = changed
        .iter()
        .filter_map(|(_, p)| p["row"]["ref"].as_str())
        .collect();
    assert_eq!(refs, ["work_item:linear:ENG-1"]);

    // A project instance reads only the project's issues.
    let r = check_with(
        &peer,
        json!({ "team": "ENG", "project": "Roadmap" }),
        &["LINEAR_API_KEY"],
    )
    .await;
    let project = r.handle.unwrap();
    invoke(&peer, &project, "create", json!({ "title": "planned" }))
        .await
        .unwrap();
    let (planned, _) = read(&peer, &project, None).await;
    let refs: Vec<&str> = planned
        .iter()
        .filter_map(|(_, p)| p["row"]["ref"].as_str())
        .collect();
    assert_eq!(refs, ["work_item:linear:ENG-4"]);
    assert!(sim
        .requests()
        .iter()
        .any(|r| r.variables["filter"]["project"]["id"]["eq"] == PROJECT_ID));
}

/// Two `providers:` entries are two instances: each one's refs carry
/// its own id.
#[tokio::test]
async fn its_refs_carry_the_id_its_manifest_gives_it() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn_as(&sim, Some(KEY), "linear_ops");
    initialize(&peer).await;
    let handle = checked(&peer).await;
    let created = invoke(&peer, &handle, "create", json!({ "title": "x" }))
        .await
        .unwrap();
    assert_eq!(
        created.result,
        json!({ "ref": "work_item:linear_ops:ENG-1" })
    );
    let other = invoke(
        &peer,
        &handle,
        "comment",
        json!({ "ref": "work_item:linear:ENG-1", "body": "x" }),
    )
    .await;
    assert!(
        matches!(&other, Err(ProtocolError::InvalidInput { field, .. }) if field == "/ref"),
        "{other:?}"
    );
}

/// P7 review (tsk718): an issue the provider can't map is skipped — said
/// in `$/progress` — and the read streams the rest and checkpoints past
/// it, instead of failing every read at the same page.
#[tokio::test]
async fn a_read_skips_an_issue_it_cant_map_and_moves_past_it() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    initialize(&peer).await;
    let handle = checked(&peer).await;
    for title in ["a", "b", "c"] {
        invoke(&peer, &handle, "create", json!({ "title": title }))
            .await
            .unwrap();
    }
    sim.set_state_type("ENG-2", "paused");
    let (streamed, result) = read(&peer, &handle, None).await;
    assert_eq!(result, json!({ "records": 2 }));
    let refs: Vec<&str> = streamed
        .iter()
        .filter_map(|(_, p)| p["row"]["ref"].as_str())
        .collect();
    assert_eq!(refs, ["work_item:linear:ENG-1", "work_item:linear:ENG-3"]);
    assert!(
        streamed.iter().any(|(m, p)| m == notify::PROGRESS
            && p["message"].as_str().is_some_and(|t| t.contains("ENG-2"))),
        "the skip is said: {streamed:?}"
    );
    let cursor = streamed.last().unwrap().1["state"].clone();
    let (_, again) = read(&peer, &handle, Some(cursor)).await;
    assert_eq!(again, json!({ "records": 0 }), "the checkpoint is past it");
}

#[tokio::test]
async fn a_rate_limited_reply_is_rate_limited_with_its_retry_after() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    initialize(&peer).await;
    let handle = checked(&peer).await;
    sim.rate_limit_next(2);
    let limited = invoke(&peer, &handle, "create", json!({ "title": "x" })).await;
    assert!(
        matches!(
            limited,
            Err(ProtocolError::RateLimited {
                retry_after_ms: Some(2000),
                ..
            })
        ),
        "{limited:?}"
    );
    assert!(invoke(&peer, &handle, "create", json!({ "title": "x" }))
        .await
        .is_ok());
}

/// P10: a create sent with an idempotency key carries an id derived from
/// it, so sending it again is one issue (one comment, one relation): the
/// service refuses the repeated id and the provider answers with what the
/// first send made. It still doesn't declare `idempotent_writes` — that
/// waits on a live run confirming Linear keeps client ids.
#[tokio::test]
async fn a_repeated_create_is_one_issue() {
    a_repeated_create_whose_lookup_fails_says_why().await;
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    let declared = initialize(&peer).await;
    assert_ne!(
        declared.capabilities[0].features["idempotent_writes"],
        json!(true)
    );
    let handle = checked(&peer).await;
    let create = || {
        keyed(
            &peer,
            &handle,
            "create",
            json!({ "title": "Once" }),
            Some("k-1"),
        )
    };
    let first = create().await.unwrap();
    let again = create().await.unwrap();
    assert_eq!(row(&first)["ref"], row(&again)["ref"]);
    assert_eq!(sim.live_issues().len(), 1);
    let item = row(&first)["ref"].as_str().unwrap().to_string();
    let other = invoke(&peer, &handle, "create", json!({ "title": "Other" }))
        .await
        .unwrap();
    let other = row(&other)["ref"].as_str().unwrap().to_string();

    for _ in 0..2 {
        keyed(
            &peer,
            &handle,
            "comment",
            json!({ "ref": item, "body": "once" }),
            Some("c-1"),
        )
        .await
        .unwrap();
        keyed(
            &peer,
            &handle,
            "link",
            json!({ "ref": item, "target": other, "link_type": "blocks" }),
            Some("l-1"),
        )
        .await
        .unwrap();
    }
    let identifier = item.rsplit(':').next().unwrap();
    assert_eq!(sim.comments(identifier), vec!["once"]);
    assert_eq!(sim.relations().len(), 1);
}

/// tsk931: a create sent again is refused as taken, and the lookup of what
/// it made is rate limited: the caller is told it is rate limited — the
/// write may well have landed — not the refusal of the repeat.
async fn a_repeated_create_whose_lookup_fails_says_why() {
    let sim = LinearSim::start(KEY).await.unwrap();
    let (_child, peer) = spawn(&sim, Some(KEY));
    initialize(&peer).await;
    let handle = checked(&peer).await;
    let create = || {
        keyed(
            &peer,
            &handle,
            "create",
            json!({ "title": "Once" }),
            Some("k-lookup"),
        )
    };
    create().await.unwrap();
    sim.rate_limit_next_of("Issue", 3);
    let again = create().await.unwrap_err();
    assert!(
        matches!(again, ProtocolError::RateLimited { .. }),
        "{again:?}"
    );
    assert_eq!(sim.live_issues().len(), 1);
}
