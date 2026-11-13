//! A `Peer` drives the fake provider over real stdio — initialize, check,
//! invoke, read (records and state), cancel, and the hooks.

use std::process::Stdio;
use std::time::Duration;

use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::schemas::{for_message, validate};
use oxplow_provider_protocol::{Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::Receiver;

/// The fake in its default mode (snapshots) with `hooks`.
fn spawn(hooks: &str) -> (Child, Peer, Receiver<Incoming>) {
    spawn_as("snapshots", "fake", hooks, None)
}

/// The fake of `capability`, as instance `id`, with `hooks`; `token` is the
/// credential `FAKE_TOKEN` it holds.
fn spawn_as(
    capability: &str,
    id: &str,
    hooks: &str,
    token: Option<&str>,
) -> (Child, Peer, Receiver<Incoming>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"));
    command
        .env("OXPLOW_FAKE_CAPABILITY", capability)
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
            context: None,
        },
    )
    .await
}

/// A snapshots `mark` of `dir` (the fake marks whatever is in it).
fn mark_of(dir: &std::path::Path, stream: &str) -> Value {
    json!({ "stream": stream, "worktree": dir, "trigger": "turn_end" })
}

/// An idempotency key makes a write happen once: sent again it is answered
/// as the first was (so the next `mark` is the second, not the third). A
/// key reused for another write is refused. Without a key, every send is a
/// write.
#[tokio::test]
async fn the_same_key_writes_once_and_answers_alike() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    let (_child, peer, _incoming) = spawn("");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let input = mark_of(dir.path(), "s1");
    let first = keyed(&peer, &handle, "mark", input.clone(), Some("k-1"))
        .await
        .unwrap();
    let again = keyed(&peer, &handle, "mark", input.clone(), Some("k-1"))
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(first.result["handle"], "m1");
    let reused = keyed(
        &peer,
        &handle,
        "mark",
        mark_of(dir.path(), "other"),
        Some("k-1"),
    )
    .await;
    assert!(
        matches!(&reused, Err(ProtocolError::InvalidInput { field, .. }) if field == "/idempotency_key"),
        "{reused:?}"
    );
    // Without a key, every send is a write: m1 was made once, so this is m2.
    let next = invoke(&peer, &handle, "mark", input.clone()).await.unwrap();
    assert_eq!(next.result["handle"], "m2");
    let last = invoke(&peer, &handle, "mark", input).await.unwrap();
    assert_eq!(last.result["handle"], "m3");
}

/// `lose-reply` — the next write lands but its answer never comes (a reply
/// lost on the way); sent again with its key, it is answered as the first
/// would have been, and it was done once.
#[tokio::test]
async fn a_lost_reply_is_answered_when_its_write_is_sent_again() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, peer, _incoming) = spawn("lose-reply");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let lost = tokio::time::timeout(
        Duration::from_millis(300),
        keyed(
            &peer,
            &handle,
            "mark",
            mark_of(dir.path(), "s1"),
            Some("k-1"),
        ),
    )
    .await;
    assert!(lost.is_err(), "no answer came: {lost:?}");
    let answered = keyed(
        &peer,
        &handle,
        "mark",
        mark_of(dir.path(), "s1"),
        Some("k-1"),
    )
    .await
    .unwrap();
    assert_eq!(answered.result["handle"], "m1");
    let next = invoke(&peer, &handle, "mark", mark_of(dir.path(), "s1"))
        .await
        .unwrap();
    assert_eq!(next.result["handle"], "m2", "the write was done once");
}

#[tokio::test]
async fn a_peer_drives_the_fake_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    let (_child, peer, _incoming) = spawn("");
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
    let result: CheckResult = peer
        .call(
            method::CHECK,
            &CheckParams {
                config: json!({ "team": "core" }),
                credentials: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(result.handle, Some(Handle("fake:core".into())));
    let handle = check(&peer).await;

    // invoke: the result, and no event.
    let marked = invoke(&peer, &handle, "mark", mark_of(dir.path(), "s1"))
        .await
        .unwrap();
    assert_eq!(
        marked.result,
        json!({ "handle": "m1", "unchanged": false, "file_count": 1 })
    );
    assert!(marked.events.is_empty());
    let refused = invoke(&peer, &handle, "nope", json!({})).await.unwrap_err();
    assert!(
        matches!(refused, ProtocolError::InvalidInput { ref field, .. } if field == "/command"),
        "{refused:?}"
    );

    // cancel: a slow invoke comes back Cancelled.
    peer.notify("fake/hooks", json!({ "hooks": "slow:5000" }))
        .await
        .unwrap();
    let call = peer
        .start(
            method::INVOKE,
            json!({ "handle": handle, "command": "mark", "input": mark_of(dir.path(), "s1") }),
        )
        .await
        .unwrap();
    peer.cancel(call.id).await.unwrap();
    assert_eq!(call.reply().await, Err(ProtocolError::Cancelled));
}

#[tokio::test]
async fn the_hooks_script_failures_and_crashes() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, peer, _incoming) = spawn("fail-next:2,bad-declarations");
    let declared = initialize(&peer).await;
    assert_ne!(
        declared,
        oxplow_provider_fake::declarations(),
        "bad-declarations"
    );
    assert!(declared.commands.iter().any(|c| c.name == "undeclared"));
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
    assert!(invoke(&peer, &handle, "mark", mark_of(dir.path(), "s1"))
        .await
        .is_ok());

    let (mut child, peer, _incoming) = spawn("crash");
    let died: Result<Value, _> = peer.request(method::INITIALIZE, json!({})).await;
    assert!(died.is_err());
    assert_eq!(child.wait().await.unwrap().code(), Some(3));
}

/// A `rate-limit:<ms>` refuses the next write once, saying when to retry.
#[tokio::test]
async fn a_rate_limit_refuses_one_write() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, peer, _incoming) = spawn("rate-limit:250");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let limited = invoke(&peer, &handle, "mark", mark_of(dir.path(), "s1")).await;
    assert!(
        matches!(
            &limited,
            Err(ProtocolError::RateLimited {
                retry_after_ms: Some(250),
                ..
            })
        ),
        "{limited:?}"
    );
    assert!(invoke(&peer, &handle, "mark", mark_of(dir.path(), "s1"))
        .await
        .is_ok());
}

/// The read hooks, on the snapshots mode's collector of marks — `progress` (each
/// `$/progress` fits its golden) and `read-fail-after:<n>` (checkpointed
/// records, then a failure).
#[tokio::test]
async fn the_read_hooks_stream_progress_and_fail_midway() {
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
                    collector: "marks".into(),
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
    let dir = tempfile::tempdir().unwrap();
    let (_child, peer, mut incoming) = spawn_as("snapshots", "fake", "progress", None);
    initialize(&peer).await;
    let handle = check(&peer).await;
    for stream in ["a", "b", "c"] {
        invoke(&peer, &handle, "mark", mark_of(dir.path(), stream))
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

    // Only a snapshots provider has a collector to read.
    let (_child, peer, _incoming) = spawn_as("agent_harness", "fake", "", None);
    initialize(&peer).await;
    let handle = check(&peer).await;
    let refused: Result<ReadResult, _> = peer
        .call(
            method::READ,
            &ReadParams {
                handle,
                collector: "marks".into(),
                state: None,
            },
        )
        .await;
    assert!(
        matches!(refused, Err(ProtocolError::InvalidInput { .. })),
        "{refused:?}"
    );
}

/// Under `needs:<NAME>` the fake's `check` says when the credential
/// `<NAME>` didn't reach it.
#[tokio::test]
async fn the_fake_reports_a_missing_credential() {
    let dir = tempfile::tempdir().unwrap();
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
    let (_c1, without, _i1) = spawn_as("snapshots", "fake", "needs:FAKE_TOKEN", None);
    initialize(&without).await;
    let result = checked(without).await;
    assert!(result.handle.is_none());
    assert_eq!(result.problems[0].path, "/credentials/FAKE_TOKEN");
    let (_c2, with, _i2) = spawn_as("snapshots", "fake", "needs:FAKE_TOKEN", Some("s3cret"));
    initialize(&with).await;
    assert!(checked(with).await.handle.is_some());

    // Under `accepts:<NAME>=<value>` its service takes that token and no
    // other — `check` and `invoke` answer `Auth` otherwise.
    let (_c3, stale, _i3) = spawn_as("snapshots", "fake", "accepts:FAKE_TOKEN=at-2", Some("at-1"));
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
    let (_c4, fresh, _i4) = spawn_as("snapshots", "fake", "accepts:FAKE_TOKEN=at-2", Some("at-2"));
    initialize(&fresh).await;
    let handle = check(&fresh).await;
    assert!(invoke(&fresh, &handle, "mark", mark_of(dir.path(), "s1"))
        .await
        .is_ok());
    // Told mid-session that the service moved on, the same process is refused.
    fresh
        .notify("fake/hooks", json!({ "hooks": "accepts:FAKE_TOKEN=at-3" }))
        .await
        .unwrap();
    let refused = invoke(&fresh, &handle, "mark", mark_of(dir.path(), "s1")).await;
    assert!(
        matches!(
            &refused,
            Err(ProtocolError::Auth { credential: Some(c), .. }) if c == "FAKE_TOKEN"
        ),
        "{refused:?}"
    );
}

/// Its snapshots mode (`OXPLOW_FAKE_CAPABILITY=snapshots`, the default): a fake that
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
    std::fs::write(root.join(".git/HEAD"), "uri").unwrap();
    std::fs::write(root.join("a.txt"), "one").unwrap();
    std::fs::write(root.join("src/b.txt"), "two").unwrap();
    let (_child, peer) = spawn_snapshots("contents");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let mark = |parent: Value| {
        json!({ "stream": "str1", "worktree": root, "trigger": "turn_end",
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
        json!({ "stream": "str1", "from": null, "to": "m1" }),
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
        json!({ "stream": "str1", "from": "m2", "to": "m3" }),
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
            json!({ "stream": "str1", "from": "m9", "to": "m1" }),
        ),
        (
            "changed",
            json!({ "stream": "str1", "from": null, "to": "m9" }),
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

/// Its filesystem mode (`OXPLOW_FAKE_CAPABILITY=filesystem`): an
/// in-memory tree seeded from `OXPLOW_FAKE_FILES`, written only with the
/// feature `writable`.
fn spawn_filesystem(features: &str) -> (Child, Peer) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-fake"))
        .env("OXPLOW_FAKE_CAPABILITY", "filesystem")
        .env("OXPLOW_FAKE_FEATURES", features)
        // a/b.txt = "hello", c.bin = 0x00 0x01
        .env(
            "OXPLOW_FAKE_FILES",
            r#"{"a/b.txt":"aGVsbG8=","c.bin":"AAE="}"#,
        )
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
async fn as_a_filesystem_provider_it_declares_write_only_when_writable() {
    for (features, writable, verbs) in [
        ("writable", true, vec!["stat", "read", "list", "write"]),
        ("", false, vec!["stat", "read", "list"]),
    ] {
        let (_child, peer) = spawn_filesystem(features);
        let declared = initialize(&peer).await;
        assert_eq!(
            declared,
            oxplow_provider_fake::filesystem_declarations(writable)
        );
        assert_eq!(declared.capabilities[0].capability, "filesystem");
        assert_eq!(
            declared.capabilities[0].features,
            json!({ "writable": writable })
        );
        assert_eq!(
            declared
                .commands
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            verbs
        );
    }
}

#[tokio::test]
async fn as_a_filesystem_provider_it_serves_and_changes_its_tree() {
    let (_child, peer) = spawn_filesystem("writable");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let call = |verb: &'static str, input: Value| {
        let (peer, handle) = (&peer, &handle);
        async move { invoke(peer, handle, verb, input).await.map(|r| r.result) }
    };
    assert_eq!(
        call("stat", json!({ "path": "a/b.txt" })).await.unwrap(),
        json!({ "entry": { "kind": "file", "size": 5, "modified_at": 1_700_000_000_000_i64,
                           "is_text": true } })
    );
    assert_eq!(
        call("stat", json!({ "path": "a" })).await.unwrap(),
        json!({ "entry": { "kind": "dir" } })
    );
    assert_eq!(
        call("stat", json!({ "path": "nope" })).await.unwrap(),
        json!({ "entry": null })
    );
    assert_eq!(
        call("stat", json!({ "path": "c.bin" })).await.unwrap()["entry"]["is_text"],
        json!(false)
    );
    assert_eq!(
        call("read", json!({ "path": "a/b.txt", "offset": 1, "len": 3 }))
            .await
            .unwrap(),
        json!({ "bytes": "ZWxs" })
    );
    assert_eq!(
        call("read", json!({ "path": "nope" })).await.unwrap(),
        json!({ "bytes": null })
    );
    assert_eq!(
        call("list", json!({ "parent": "" })).await.unwrap(),
        json!({ "entries": [{ "path": "a", "kind": "dir" }, { "path": "c.bin", "kind": "file" }] })
    );
    assert_eq!(
        call("list", json!({})).await.unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    call("write", json!({ "path": "a/new.txt", "content": "eA==" }))
        .await
        .unwrap();
    assert_eq!(
        call("read", json!({ "path": "a/new.txt" })).await.unwrap(),
        json!({ "bytes": "eA==" })
    );
    for refused in [
        call("stat", json!({ "path": "../x" })).await,
        call("read", json!({ "path": "a/b.txt", "rev": "v9" })).await,
    ] {
        assert!(
            matches!(refused, Err(ProtocolError::InvalidInput { .. })),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn a_read_only_filesystem_provider_has_no_write() {
    let (_child, peer) = spawn_filesystem("");
    initialize(&peer).await;
    let handle = check(&peer).await;
    let refused = invoke(
        &peer,
        &handle,
        "write",
        json!({ "path": "x", "content": "eA==" }),
    )
    .await;
    assert!(
        matches!(refused, Err(ProtocolError::InvalidInput { .. })),
        "{refused:?}"
    );
}
