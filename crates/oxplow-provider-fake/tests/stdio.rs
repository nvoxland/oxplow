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
use tokio::sync::mpsc::Receiver;

fn spawn(hooks: &str) -> (Child, Peer, Receiver<Incoming>) {
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

/// The refs of every item it has, by a read from the start.
async fn refs(peer: &Peer, handle: &Handle, incoming: &mut Receiver<Incoming>) -> Vec<String> {
    let result: ReadResult = peer
        .call(
            method::READ,
            &ReadParams {
                handle: handle.clone(),
                collector: "work_items".into(),
                state: None,
            },
        )
        .await
        .expect("read");
    let mut refs = Vec::new();
    while refs.len() < result.records as usize {
        if let Some(Incoming::Notification { method, params }) = incoming.recv().await {
            if method == notify::RECORD {
                refs.push(
                    params["row"]["ref"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }
    }
    refs
}

/// P10: the fake declares `idempotent_writes` and keeps it: a write sent
/// twice with one key is done once, and both answers are the same. A key
/// reused for another write is refused. `forget-keys` breaks the promise
/// (what the kit must catch); `plain-writes` doesn't make it.
#[tokio::test]
async fn the_same_key_writes_once_and_answers_alike() {
    let (_child, peer, mut incoming) = spawn("");
    let declared = initialize(&peer).await;
    assert_eq!(
        declared.capabilities[0].features["idempotent_writes"],
        json!(true)
    );
    let handle = check(&peer).await;
    let input = json!({ "title": "Once" });
    let first = keyed(&peer, &handle, "create", input.clone(), Some("k-1"))
        .await
        .unwrap();
    let again = keyed(&peer, &handle, "create", input, Some("k-1"))
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(refs(&peer, &handle, &mut incoming).await.len(), 1);
    let reused = keyed(
        &peer,
        &handle,
        "create",
        json!({ "title": "Other" }),
        Some("k-1"),
    )
    .await;
    assert!(
        matches!(&reused, Err(ProtocolError::InvalidInput { field, .. }) if field == "/idempotency_key"),
        "{reused:?}"
    );
    // Without a key, every send is a write.
    invoke(&peer, &handle, "create", json!({ "title": "Twice" }))
        .await
        .unwrap();
    invoke(&peer, &handle, "create", json!({ "title": "Twice" }))
        .await
        .unwrap();
    assert_eq!(refs(&peer, &handle, &mut incoming).await.len(), 3);

    // `forget-keys`: it declares the promise and doesn't keep it.
    let (_child, forgetful, mut incoming) = spawn("forget-keys");
    initialize(&forgetful).await;
    let handle = check(&forgetful).await;
    for _ in 0..2 {
        keyed(
            &forgetful,
            &handle,
            "create",
            json!({ "title": "Once" }),
            Some("k-1"),
        )
        .await
        .unwrap();
    }
    assert_eq!(refs(&forgetful, &handle, &mut incoming).await.len(), 2);

    // `plain-writes`: it doesn't declare it.
    let (_child, plain, _incoming) = spawn("plain-writes");
    let declared = initialize(&plain).await;
    assert_eq!(
        declared.capabilities[0].features["idempotent_writes"],
        json!(false)
    );
    assert_eq!(declared, oxplow_provider_fake::plain_declarations());
}

/// P10: `lose-reply` — the next write lands but its answer never comes (a
/// reply lost on the way); sent again with its key, it is answered as the
/// first would have been, and it was done once.
#[tokio::test]
async fn a_lost_reply_is_answered_when_its_write_is_sent_again() {
    let (_child, peer, mut incoming) = spawn("lose-reply");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let lost = tokio::time::timeout(
        Duration::from_millis(300),
        keyed(
            &peer,
            &handle,
            "create",
            json!({ "title": "Lost" }),
            Some("k-1"),
        ),
    )
    .await;
    assert!(lost.is_err(), "no answer came: {lost:?}");
    let answered = keyed(
        &peer,
        &handle,
        "create",
        json!({ "title": "Lost" }),
        Some("k-1"),
    )
    .await
    .unwrap();
    assert_eq!(answered.events.len(), 1);
    assert_eq!(refs(&peer, &handle, &mut incoming).await.len(), 1);
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

    // read: each changed item (in the order it changed) then a
    // checkpoint, before the result; resuming skips them.
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
    // W-1 changed last (its transition), so it comes after W-2.
    assert_eq!(
        streamed,
        vec![json!("work_item:fake:W-2"), json!("work_item:fake:W-1")]
    );
    assert_eq!(state, Some(json!({ "cursor": 3, "seen": 2 })));
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

/// P7.A3: the read hooks — `progress` (each `$/progress` fits its
/// golden), `read-fail-after:<n>` (checkpointed records, then a failure)
/// and `bad-record` (another provider's item).
#[tokio::test]
async fn the_read_hooks_stream_progress_fail_midway_and_misreport() {
    async fn read(
        peer: &Peer,
        handle: &Handle,
        incoming: &mut Receiver<Incoming>,
    ) -> (Result<ReadResult, ProtocolError>, Vec<(String, Value)>) {
        let result = peer
            .call(
                method::READ,
                &ReadParams {
                    handle: handle.clone(),
                    collector: "work_items".into(),
                    state: None,
                },
            )
            .await;
        let mut seen = Vec::new();
        while let Ok(Some(Incoming::Notification { method, params })) =
            tokio::time::timeout(Duration::from_millis(200), incoming.recv()).await
        {
            assert_eq!(
                validate(for_message(&method, false).unwrap(), &params),
                Ok(())
            );
            seen.push((method, params));
        }
        (result, seen)
    }
    let (_child, peer, mut incoming) = spawn("progress");
    initialize(&peer).await;
    let handle = check(&peer).await;
    for title in ["a", "b", "c"] {
        invoke(&peer, &handle, "create", json!({ "title": title }))
            .await
            .unwrap();
    }
    let (result, seen) = read(&peer, &handle, &mut incoming).await;
    assert_eq!(result.unwrap().records, 3);
    let methods: Vec<&str> = seen.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        &methods[..3],
        &[notify::PROGRESS, notify::RECORD, notify::STATE]
    );
    assert_eq!(seen[0].1["fraction"], json!(1.0 / 3.0));

    peer.notify("fake/hooks", json!({ "hooks": "read-fail-after:2" }))
        .await
        .unwrap();
    let (result, seen) = read(&peer, &handle, &mut incoming).await;
    assert!(
        matches!(result, Err(ProtocolError::Internal(_))),
        "{result:?}"
    );
    let states: Vec<&Value> = seen
        .iter()
        .filter(|(m, _)| m == notify::STATE)
        .map(|(_, p)| &p["state"])
        .collect();
    assert_eq!(states.len(), 2, "checkpointed before failing");

    let (_child, peer, mut incoming) = spawn("bad-record");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let (_, seen) = read(&peer, &handle, &mut incoming).await;
    assert_eq!(seen[0].1["row"]["ref"], "work_item:other:X-1");
}

/// P9.B1: the fake is whichever instance the host says it is — its refs
/// carry `OXPLOW_PROVIDER_ID` — and, under `needs:<NAME>`, its `check`
/// says when the credential `<NAME>` didn't reach it.
#[tokio::test]
async fn the_fake_takes_its_id_from_the_host_and_reports_a_missing_credential() {
    let spawn_as = |id: &str, hooks: &str, token: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"));
        command
            .env("OXPLOW_FAKE_HOOKS", hooks)
            .env("OXPLOW_PROVIDER_ID", id)
            .env_remove("FAKE_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        if let Some(token) = token {
            command.env("FAKE_TOKEN", token);
        }
        let mut child = command.spawn().expect("spawn the fake");
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take().expect("piped stdin");
        let (peer, incoming) = Peer::spawn(stdout, stdin);
        (child, peer, incoming)
    };
    let (_child, peer, _incoming) = spawn_as("fake_second", "", None);
    initialize(&peer).await;
    let handle = check(&peer).await;
    let created = invoke(&peer, &handle, "create", json!({ "title": "First" }))
        .await
        .unwrap();
    assert_eq!(created.result["ref"], "work_item:fake_second:W-1");
    let moved = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:fake_second:W-1", "to": "done" }),
    )
    .await;
    assert!(moved.is_ok(), "{moved:?}");
    // Another instance's ref isn't one of its items.
    let foreign = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:fake:W-1", "to": "done" }),
    )
    .await;
    assert!(
        matches!(foreign, Err(ProtocolError::InvalidInput { .. })),
        "{foreign:?}"
    );

    let checked = |peer: Peer| async move {
        let result: CheckResult = peer
            .call(
                method::CHECK,
                &CheckParams {
                    config: json!({ "team": "core" }),
                    credentials: vec!["FAKE_TOKEN".into()],
                },
            )
            .await
            .expect("check");
        result
    };
    let (_c1, without, _i1) = spawn_as("fake", "needs:FAKE_TOKEN", None);
    initialize(&without).await;
    let result = checked(without).await;
    assert!(result.handle.is_none());
    assert_eq!(result.problems[0].path, "/credentials/FAKE_TOKEN");
    let (_c2, with, _i2) = spawn_as("fake", "needs:FAKE_TOKEN", Some("s3cret"));
    initialize(&with).await;
    assert!(checked(with).await.handle.is_some());

    // P9.B3: under `accepts:<NAME>=<value>` its service takes that token
    // and no other — `check` and `invoke` answer `Auth` otherwise.
    let (_c3, stale, _i3) = spawn_as("fake", "accepts:FAKE_TOKEN=at-2", Some("at-1"));
    initialize(&stale).await;
    let refused = stale
        .call::<_, CheckResult>(
            method::CHECK,
            &CheckParams {
                config: json!({ "team": "core" }),
                credentials: vec!["FAKE_TOKEN".into()],
            },
        )
        .await;
    assert!(
        matches!(
            &refused,
            Err(ProtocolError::Auth { credential: Some(c), .. }) if c == "FAKE_TOKEN"
        ),
        "{refused:?}"
    );
    let (_c4, fresh, _i4) = spawn_as("fake", "accepts:FAKE_TOKEN=at-2", Some("at-2"));
    initialize(&fresh).await;
    let handle = check(&fresh).await;
    assert!(invoke(&fresh, &handle, "create", json!({ "title": "x" }))
        .await
        .is_ok());
    // Told mid-session that the service moved on, the same process is refused.
    fresh
        .notify("fake/hooks", json!({ "hooks": "accepts:FAKE_TOKEN=at-3" }))
        .await
        .unwrap();
    let refused = invoke(&fresh, &handle, "create", json!({ "title": "y" })).await;
    assert!(
        matches!(
            &refused,
            Err(ProtocolError::Auth { credential: Some(c), .. }) if c == "FAKE_TOKEN"
        ),
        "{refused:?}"
    );
}

/// Its effort-policy mode (`OXPLOW_FAKE_CAPABILITY=effort_policy`): it
/// declares the one verb `react` and composes efforts for an item's
/// moves — open on `in_progress` with a thread, close the item's open
/// efforts (read through `host/call`) on `done` — and skips the rest.
#[tokio::test]
async fn as_an_effort_policy_it_composes_efforts() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"))
        .env("OXPLOW_FAKE_CAPABILITY", "effort_policy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the fake");
    let stdout = child.stdout.take().unwrap();
    let stdin = child.stdin.take().unwrap();
    let (peer, mut incoming) = Peer::spawn(stdout, stdin);
    let declared = initialize(&peer).await;
    assert_eq!(declared, oxplow_provider_fake::policy_declarations());
    assert_eq!(declared.capabilities[0].capability, "effort_policy");
    assert_eq!(
        declared
            .commands
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["react"]
    );
    assert!(declared.event_types.is_empty() && declared.collectors.is_empty());
    let handle = check(&peer).await;
    // The host answers its reads: the item's one open effort.
    let answering = peer.clone();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = asked.clone();
    tokio::spawn(async move {
        while let Some(Incoming::Request { id, method, params }) = incoming.recv().await {
            assert_eq!(method, method::HOST_CALL);
            seen.lock().unwrap().push(params);
            let _ = answering.respond(id, Ok(json!([{ "id": 7 }]))).await;
        }
    });
    let event = |to: &str, thread: Option<&str>| {
        json!({ "event": {
            "id": format!("e-{to}"), "type": "work_item.state_changed", "v": 1, "seq": 1,
            "source": "human", "subject": ["work_item:oxplow:tsk1"],
            "payload": { "work_item": "work_item:oxplow:tsk1", "to": to },
            "anchors": { "thread_id": thread },
        }})
    };
    let started = keyed(
        &peer,
        &handle,
        "react",
        event("in_progress", Some("thr3")),
        Some("react:e1"),
    )
    .await
    .unwrap();
    assert_eq!(
        started.result,
        json!({ "commands": [{ "name": "oxplow.effort.open",
                               "input": { "thread": "thread:thr3", "work_item": "work_item:oxplow:tsk1" } }] })
    );
    let done = keyed(
        &peer,
        &handle,
        "react",
        event("done", None),
        Some("react:e2"),
    )
    .await
    .unwrap();
    assert_eq!(
        done.result,
        json!({ "commands": [{ "name": "oxplow.effort.close",
                               "input": { "effort": "effort:eff7", "reason": "switch" } }] })
    );
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["key"], "react:e2");
    assert_eq!(asked[0]["scope"], "sql.read");
    assert_eq!(
        asked[0]["args"]["params"]["work_item"],
        "work_item:oxplow:tsk1"
    );
    let skipped = keyed(
        &peer,
        &handle,
        "react",
        event("blocked", Some("thr3")),
        Some("react:e3"),
    )
    .await
    .unwrap();
    assert!(skipped.result.get("skip").is_some(), "{}", skipped.result);
    // A start with no thread has nowhere to open an effort.
    let nowhere = keyed(
        &peer,
        &handle,
        "react",
        event("in_progress", None),
        Some("react:e4"),
    )
    .await
    .unwrap();
    assert!(nowhere.result.get("skip").is_some(), "{}", nowhere.result);
}

/// Its snapshots mode (`OXPLOW_FAKE_CAPABILITY=snapshots`): a fake that
/// marks a directory, tells what changed between marks, and (with
/// `OXPLOW_FAKE_FEATURES=contents`) gives a file's bytes back.
fn spawn_snapshots(features: &str) -> (Child, Peer) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"))
        .env("OXPLOW_FAKE_CAPABILITY", "snapshots")
        .env("OXPLOW_FAKE_FEATURES", features)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the fake");
    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");
    let (peer, _incoming) = Peer::spawn(stdout, stdin);
    (child, peer)
}

#[tokio::test]
async fn as_a_snapshots_provider_it_declares_its_verbs_by_feature() {
    for (features, contents, verbs) in [
        ("contents", true, vec!["mark", "changed", "read_at"]),
        ("", false, vec!["mark", "changed"]),
    ] {
        let (_child, peer) = spawn_snapshots(features);
        let declared = initialize(&peer).await;
        assert_eq!(
            declared,
            oxplow_provider_fake::snapshots_declarations(contents)
        );
        let capability = &declared.capabilities[0];
        assert_eq!(capability.capability, "snapshots");
        assert_eq!(capability.features, json!({ "contents": contents }));
        assert_eq!(capability.data, Value::Null);
        assert_eq!(
            declared
                .commands
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            verbs
        );
        for c in &declared.commands {
            assert_eq!((c.confirm.as_str(), c.access.as_str()), ("never", "record"));
            assert_eq!(
                c.input_schema["additionalProperties"],
                json!(false),
                "{}",
                c.name
            );
        }
        assert!(declared.event_types.is_empty() && declared.collectors.is_empty());
    }
}

#[tokio::test]
async fn as_a_snapshots_provider_it_marks_tells_what_changed_and_gives_bytes_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref").unwrap();
    std::fs::write(root.join("a.txt"), "one").unwrap();
    std::fs::write(root.join("src/b.txt"), "two").unwrap();
    let (_child, peer) = spawn_snapshots("contents");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let mark = |parent: Value| {
        json!({ "stream": "stream:str1", "worktree": root, "trigger": "turn_end",
                "parent": parent })
    };

    let first = invoke(&peer, &handle, "mark", mark(Value::Null))
        .await
        .unwrap()
        .result;
    assert_eq!(
        first,
        json!({ "handle": "m1", "unchanged": false, "file_count": 2 })
    );

    // xxh3-128 of "one", as core formats a content hash.
    let all = invoke(
        &peer,
        &handle,
        "changed",
        json!({ "stream": "stream:str1", "from": null, "to": "m1" }),
    )
    .await
    .unwrap()
    .result;
    assert_eq!(
        all["changes"],
        json!([
            { "path": "a.txt", "kind": "added", "size": 3,
              "identity": format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"one")) },
            { "path": "src/b.txt", "kind": "added", "size": 3,
              "identity": format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"two")) },
        ])
    );

    let same = invoke(&peer, &handle, "mark", mark(json!("m1")))
        .await
        .unwrap()
        .result;
    assert_eq!(
        same,
        json!({ "handle": "m2", "unchanged": true, "file_count": 2 })
    );

    std::fs::write(root.join("a.txt"), "uno").unwrap();
    std::fs::write(root.join("c.txt"), "three").unwrap();
    std::fs::remove_file(root.join("src/b.txt")).unwrap();
    let third = invoke(&peer, &handle, "mark", mark(json!("m2")))
        .await
        .unwrap()
        .result;
    assert_eq!(
        third,
        json!({ "handle": "m3", "unchanged": false, "file_count": 2 })
    );
    let diff = invoke(
        &peer,
        &handle,
        "changed",
        json!({ "stream": "stream:str1", "from": "m2", "to": "m3" }),
    )
    .await
    .unwrap()
    .result;
    let kinds: Vec<_> = diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["path"].as_str().unwrap(),
                c["kind"].as_str().unwrap(),
                c.get("identity").is_some(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("a.txt", "modified", true),
            ("c.txt", "added", true),
            ("src/b.txt", "deleted", false)
        ]
    );

    // The bytes of an earlier state are still there, and base64.
    let bytes = invoke(
        &peer,
        &handle,
        "read_at",
        json!({ "handle": "m1", "path": "src/b.txt" }),
    )
    .await
    .unwrap()
    .result;
    assert_eq!(bytes, json!({ "bytes": "dHdv" }));

    for (verb, input) in [
        ("read_at", json!({ "handle": "m9", "path": "a.txt" })),
        ("read_at", json!({ "handle": "m1", "path": "nope.txt" })),
        (
            "changed",
            json!({ "stream": "stream:str1", "from": "m9", "to": "m1" }),
        ),
        (
            "changed",
            json!({ "stream": "stream:str1", "from": null, "to": "m9" }),
        ),
    ] {
        let refused = invoke(&peer, &handle, verb, input).await;
        assert!(
            matches!(refused, Err(ProtocolError::InvalidInput { .. })),
            "{verb}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn a_snapshots_provider_without_contents_has_no_read_at() {
    let (_child, peer) = spawn_snapshots("");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let refused = invoke(
        &peer,
        &handle,
        "read_at",
        json!({ "handle": "m1", "path": "a" }),
    )
    .await;
    assert!(
        matches!(refused, Err(ProtocolError::InvalidInput { .. })),
        "{refused:?}"
    );
}

/// Its knowledge mode (`OXPLOW_FAKE_CAPABILITY=knowledge`): pages that
/// answer their `knowledge.page.recorded` event, and a collector of them.
fn spawn_knowledge(state: Option<&std::path::Path>) -> (Child, Peer, Receiver<Incoming>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"));
    cmd.env("OXPLOW_FAKE_CAPABILITY", "knowledge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    if let Some(state) = state {
        cmd.env("OXPLOW_FAKE_STATE", state);
    }
    let mut child = cmd.spawn().expect("spawn the fake");
    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");
    let (peer, incoming) = Peer::spawn(stdout, stdin);
    (child, peer, incoming)
}

fn page_write(slug: &str, body: &str) -> Value {
    json!({ "slug": slug, "body": body, "verified_refs": [], "removed_refs": [] })
}

#[tokio::test]
async fn as_a_knowledge_provider_it_declares_its_verbs_event_and_collector() {
    let (_child, peer, _incoming) = spawn_knowledge(None);
    let declared = initialize(&peer).await;
    assert_eq!(declared, oxplow_provider_fake::knowledge_declarations());
    let capability = &declared.capabilities[0];
    assert_eq!(capability.capability, "knowledge");
    assert_eq!(capability.features, json!({}));
    assert_eq!(
        declared
            .commands
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["write_page", "delete_page", "link"]
    );
    for c in &declared.commands {
        assert_eq!((c.confirm.as_str(), c.access.as_str()), ("never", "record"));
        assert_eq!(
            c.input_schema["additionalProperties"],
            json!(false),
            "{}",
            c.name
        );
    }
    assert_eq!(
        (
            declared.event_types[0].event_type.as_str(),
            declared.event_types[0].v
        ),
        ("knowledge.page.recorded", 1)
    );
    assert_eq!(
        (
            declared.collectors[0].name.as_str(),
            declared.collectors[0].entity.as_str()
        ),
        ("knowledge_pages", "knowledge_page")
    );
}

#[tokio::test]
async fn as_a_knowledge_provider_a_write_answers_the_page_and_its_record() {
    let (_child, peer, _incoming) = spawn_knowledge(None);
    initialize(&peer).await;
    let handle = check(&peer).await;
    let wrote = invoke(
        &peer,
        &handle,
        "write_page",
        page_write(
            "design",
            "See [[src/lib.rs]] and [[other-page]], [[src/lib.rs]] again.",
        ),
    )
    .await
    .unwrap();
    assert_eq!(wrote.result, json!({ "page": "wiki:design" }));
    assert_eq!(wrote.events.len(), 1);
    let event = &wrote.events[0];
    assert_eq!(event.event_type, "knowledge.page.recorded");
    assert_eq!(event.v, 1);
    assert_eq!(event.subject, vec!["wiki:design".to_string()]);
    let page = &event.payload["page"];
    assert_eq!(page["ref"], "wiki:design");
    assert_eq!(page["title"], "design");
    assert_eq!(page["refs"], json!(["file:src/lib.rs", "wiki:other-page"]));
    assert!(page.get("deleted").is_none());
    assert!(page["updated_at"].as_str().unwrap().ends_with('Z'));

    // A link goes under Related; the record states the new ref.
    let linked = invoke(
        &peer,
        &handle,
        "link",
        json!({ "page": "wiki:design", "target": "wiki:third" }),
    )
    .await
    .unwrap();
    assert_eq!(linked.result, json!({}));
    let page = &linked.events[0].payload["page"];
    assert!(
        page["body"]
            .as_str()
            .unwrap()
            .contains("## Related\n\n[[third]]"),
        "{page}"
    );
    assert_eq!(
        page["refs"],
        json!(["file:src/lib.rs", "wiki:other-page", "wiki:third"])
    );

    let deleted = invoke(&peer, &handle, "delete_page", json!({ "slug": "design" }))
        .await
        .unwrap();
    assert_eq!(deleted.result, json!({}));
    assert_eq!(deleted.events[0].payload["page"]["deleted"], json!(true));

    for (verb, input) in [
        ("delete_page", json!({ "slug": "design" })),
        ("delete_page", json!({ "slug": "nope" })),
        ("link", json!({ "page": "wiki:nope", "target": "x" })),
    ] {
        let refused = invoke(&peer, &handle, verb, input).await;
        assert!(
            matches!(refused, Err(ProtocolError::InvalidInput { .. })),
            "{verb}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn as_a_knowledge_provider_it_streams_changed_pages_and_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    {
        let (_child, peer, _incoming) = spawn_knowledge(Some(&state));
        initialize(&peer).await;
        let handle = check(&peer).await;
        invoke(&peer, &handle, "write_page", page_write("a", "one"))
            .await
            .unwrap();
        invoke(&peer, &handle, "write_page", page_write("b", "two"))
            .await
            .unwrap();
    }
    // A new process reads back what the old one kept.
    let (_child, peer, mut incoming) = spawn_knowledge(Some(&state));
    initialize(&peer).await;
    let handle = check(&peer).await;
    invoke(&peer, &handle, "write_page", page_write("a", "uno"))
        .await
        .unwrap();
    let read = |state: Value| {
        let peer = peer.clone();
        let handle = handle.clone();
        async move {
            peer.call::<_, Value>(
                method::READ,
                &ReadParams {
                    handle,
                    collector: "knowledge_pages".into(),
                    state: Some(state),
                },
            )
            .await
        }
    };
    let reader = tokio::spawn(async move {
        let mut rows = Vec::new();
        let mut checkpoints = Vec::new();
        while let Some(Incoming::Notification { method, params }) = incoming.recv().await {
            match method.as_str() {
                notify::RECORD => rows.push(params),
                notify::STATE => checkpoints.push(params),
                _ => {}
            }
            if rows.len() == 2 && checkpoints.len() == 2 {
                break;
            }
        }
        (rows, checkpoints)
    });
    // Everything, then — after its cursor — only what changed since.
    read(json!({})).await.expect("read");
    let (rows, checkpoints) = reader.await.unwrap();
    assert!(rows.iter().all(|r| r["entity"] == "knowledge_page"));
    let slugs: Vec<_> = rows
        .iter()
        .map(|r| r["row"]["ref"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        slugs,
        vec!["wiki:b", "wiki:a"],
        "in revision order, a's rewrite last"
    );
    assert_eq!(rows[1]["row"]["body"], "uno");
    assert_eq!(checkpoints[1]["state"]["cursor"], json!(3));
}
