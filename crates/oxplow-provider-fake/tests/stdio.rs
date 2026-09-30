//! P5.D2: a `Peer` drives the fake provider over real stdio — initialize,
//! check, invoke, read (records and state), cancel, and the hooks.

use std::process::Stdio;
use std::time::Duration;

use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::schemas::{for_message, validate};
use oxplow_provider_protocol::{Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedReceiver;

fn spawn(hooks: &str) -> (Child, Peer, UnboundedReceiver<Incoming>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"))
        .env("OXPLOW_FAKE_HOOKS", hooks)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the fake");
    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");
    let (peer, incoming) = Peer::spawn(stdout, stdin);
    (child, peer, incoming)
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

async fn check(peer: &Peer) -> Handle {
    let result: CheckResult = peer
        .call(
            method::CHECK,
            &CheckParams {
                config: json!({ "team": "core" }),
                credentials: vec![],
            },
        )
        .await
        .expect("check");
    result.handle.expect("a clean check returns a handle")
}

async fn invoke(
    peer: &Peer,
    handle: &Handle,
    command: &str,
    input: Value,
) -> Result<InvokeResult, ProtocolError> {
    peer.call(
        method::INVOKE,
        &InvokeParams {
            handle: handle.clone(),
            command: command.into(),
            input,
        },
    )
    .await
}

#[tokio::test]
async fn a_peer_drives_the_fake_over_stdio() {
    let (_child, peer, mut incoming) = spawn("");
    let declared = initialize(&peer).await;
    assert_eq!(declared, oxplow_provider_fake::declarations());
    assert_eq!(
        validate(
            for_message("initialize", true).unwrap(),
            &serde_json::to_value(&declared).unwrap()
        ),
        Ok(())
    );

    // A config without a team is a problem at /team, and no handle.
    let bad: CheckResult = peer
        .call(
            method::CHECK,
            &CheckParams {
                config: json!({}),
                credentials: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(bad.problems[0].path, "/team");
    assert!(bad.handle.is_none());
    let handle = check(&peer).await;

    // invoke: the result and the event the host logs.
    let created = invoke(&peer, &handle, "create", json!({ "title": "First" }))
        .await
        .unwrap();
    assert_eq!(created.result, json!({ "ref": "work_item:fake:W-1" }));
    assert_eq!(created.events[0].event_type, "work_item.recorded");
    assert_eq!(created.events[0].payload["item"]["native_state"], "Backlog");
    invoke(&peer, &handle, "create", json!({ "title": "Second" }))
        .await
        .unwrap();
    let moved = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:fake:W-1", "to": "in_progress" }),
    )
    .await
    .unwrap();
    assert_eq!(moved.events[0].payload["item"]["native_state"], "Doing");
    let refused = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:fake:W-9", "to": "done" }),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(refused, ProtocolError::InvalidInput { ref field, .. } if field == "/ref"),
        "{refused:?}"
    );

    // read: records then state, before the result; resuming skips them.
    let read: ReadResult = peer
        .call(
            method::READ,
            &ReadParams {
                handle: handle.clone(),
                collector: "work_items".into(),
                state: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(read.records, 2);
    let mut streamed = Vec::new();
    let mut state = None;
    while let Ok(Some(Incoming::Notification { method, params })) =
        tokio::time::timeout(Duration::from_millis(200), incoming.recv()).await
    {
        assert_eq!(
            validate(for_message(&method, false).unwrap(), &params),
            Ok(())
        );
        match method.as_str() {
            notify::RECORD => streamed.push(params["row"]["ref"].clone()),
            notify::STATE => state = Some(params["state"].clone()),
            other => panic!("unexpected {other}"),
        }
    }
    assert_eq!(
        streamed,
        vec![json!("work_item:fake:W-1"), json!("work_item:fake:W-2")]
    );
    let resumed: ReadResult = peer
        .call(
            method::READ,
            &ReadParams {
                handle: handle.clone(),
                collector: "work_items".into(),
                state,
            },
        )
        .await
        .unwrap();
    assert_eq!(resumed.records, 0);

    // cancel: a slow invoke comes back Cancelled.
    peer.notify("fake/hooks", json!({ "hooks": "slow:5000" }))
        .await
        .unwrap();
    let call = peer
        .start(
            method::INVOKE,
            json!({ "handle": handle, "command": "create", "input": { "title": "Slow" } }),
        )
        .await
        .unwrap();
    peer.cancel(call.id).await.unwrap();
    assert_eq!(call.reply().await, Err(ProtocolError::Cancelled));
}

#[tokio::test]
async fn the_hooks_script_failures_and_crashes() {
    let (_child, peer, _incoming) = spawn("fail-next:2,bad-declarations");
    let declared = initialize(&peer).await;
    assert_ne!(
        declared,
        oxplow_provider_fake::declarations(),
        "bad-declarations"
    );
    for _ in 0..2 {
        let failed: Result<CheckResult, _> = peer
            .call(
                method::CHECK,
                &CheckParams {
                    config: json!({ "team": "t" }),
                    credentials: vec![],
                },
            )
            .await;
        assert!(
            matches!(failed, Err(ProtocolError::Internal(_))),
            "{failed:?}"
        );
    }
    let handle = check(&peer).await;
    assert!(invoke(&peer, &handle, "create", json!({ "title": "ok" }))
        .await
        .is_ok());

    let (mut child, peer, _incoming) = spawn("crash");
    let died: Result<Value, _> = peer.request(method::INITIALIZE, json!({})).await;
    assert!(died.is_err());
    assert_eq!(child.wait().await.unwrap().code(), Some(3));
}
