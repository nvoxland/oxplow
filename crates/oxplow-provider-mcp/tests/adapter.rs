//! P7.A6: the MCP adapter over real stdio, in a copy of the `notes`
//! fixture extension in front of the notes MCP server — the tool pin, the
//! mapping's tool calls and outcomes, its read, and what it may not
//! return. `OXPLOW_BLESS=1` re-pins `mcp/tools.json` from the server.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Stdio;

use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Incoming, Peer, ProtocolError};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio::process::{Child, Command};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the fixture with its server entry running the notes server.
fn extension() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture_dir(), dir.path());
    let server = dir.path().join("bin/notes-server");
    std::fs::write(
        &server,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_oxplow-provider-mcp-notes")
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn spawn(dir: &Path) -> (Child, Peer) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-mcp"))
        .args([
            "--declarations",
            "provider.json",
            "--mapping",
            "mcp/notes.star",
            "--tools",
            "mcp/tools.json",
            "--",
            "bin/notes-server",
        ])
        .current_dir(dir)
        .env("OXPLOW_PROVIDER_ID", "notes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (stdout, stdin) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let (peer, _incoming) = Peer::spawn(stdout, stdin);
    (child, peer)
}

async fn check(peer: &Peer) -> CheckResult {
    let declared: InitializeResult = peer
        .call(
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
        .unwrap();
    let on_disk: InitializeResult = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir().join("provider.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(declared, on_disk);
    peer.call(
        method::CHECK,
        &CheckParams {
            config: json!({}),
            credentials: vec![],
        },
    )
    .await
    .unwrap()
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

async fn read(peer: &Peer, handle: &Handle, state: Option<Value>) -> (Vec<(String, Value)>, Value) {
    let params = serde_json::to_value(ReadParams {
        handle: handle.clone(),
        collector: "items".into(),
        state,
    })
    .unwrap();
    let (call, mut rx) = peer.start_streaming(method::READ, params).await.unwrap();
    let result = call.reply().await.unwrap();
    let mut streamed = Vec::new();
    while let Ok(Incoming::Notification { method, params }) = rx.try_recv() {
        streamed.push((method, params));
    }
    (streamed, result)
}

#[tokio::test]
async fn the_pinned_tools_are_the_servers() {
    let transport = rmcp::transport::TokioChildProcess::new(Command::new(env!(
        "CARGO_BIN_EXE_oxplow-provider-mcp-notes"
    )))
    .unwrap();
    let client = ().serve(transport).await.unwrap();
    let live: Vec<Value> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(oxplow_provider_mcp::pinned)
        .collect();
    client.cancel().await.unwrap();
    let path = fixture_dir().join("mcp/tools.json");
    if std::env::var_os("OXPLOW_BLESS").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&live).unwrap() + "\n").unwrap();
    }
    let pins: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(oxplow_provider_mcp::pin_difference(&pins, &live), None);
}

#[tokio::test]
async fn a_server_whose_tools_differ_from_the_pin_is_refused() {
    let ext = extension();
    let (_child, peer) = spawn(ext.path());
    let ok = check(&peer).await;
    assert_eq!((ok.problems, ok.handle.is_some()), (vec![], true));

    let tools = ext.path().join("mcp/tools.json");
    let mut pins: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&tools).unwrap()).unwrap();
    let at = pins
        .iter()
        .position(|t| t["name"] == "create_item")
        .unwrap();
    pins[at]["description"] = json!("Create a note (as it was).");
    std::fs::write(&tools, serde_json::to_string(&pins).unwrap()).unwrap();
    let (_child, peer) = spawn(ext.path());
    let refused = check(&peer).await;
    assert_eq!(refused.handle, None);
    assert!(
        refused.problems[0]
            .message
            .contains("`create_item` description"),
        "{:?}",
        refused.problems
    );
}

#[tokio::test]
async fn the_mapping_turns_verbs_into_tool_calls_and_output_into_outcomes() {
    let ext = extension();
    let (_child, peer) = spawn(ext.path());
    let handle = check(&peer).await.handle.unwrap();

    let created = invoke(&peer, &handle, "create", json!({ "title": "First" }))
        .await
        .unwrap();
    assert_eq!(created.result, json!({ "ref": "work_item:notes:N-1" }));
    let row = &created.events[0].payload["item"];
    assert_eq!(
        (row["state"].clone(), row["native_state"].clone()),
        (json!("todo"), json!("open"))
    );
    assert_eq!(created.events[0].subject, ["work_item:notes:N-1"]);

    let child = invoke(
        &peer,
        &handle,
        "create",
        json!({ "title": "Child", "parent_ref": "work_item:notes:N-1" }),
    )
    .await
    .unwrap();
    assert_eq!(
        child.events[0].payload["item"]["parent_ref"],
        "work_item:notes:N-1"
    );

    let moved = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:notes:N-1", "to": "blocked" }),
    )
    .await
    .unwrap();
    assert_eq!(moved.events[0].payload["item"]["native_state"], "stuck");
    let wrong = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:notes:N-1", "to": "done", "native_state": "dropped" }),
    )
    .await;
    assert!(
        matches!(&wrong, Err(ProtocolError::InvalidInput { field, .. }) if field == "/native_state"),
        "{wrong:?}"
    );
    let missing = invoke(
        &peer,
        &handle,
        "update",
        json!({ "ref": "work_item:notes:N-9", "title": "x" }),
    )
    .await;
    assert!(
        matches!(&missing, Err(ProtocolError::InvalidInput { field, message })
            if field == "/ref" && message.contains("no note `N-9`")),
        "a tool error reaches the mapping, which refuses it: {missing:?}"
    );

    let (streamed, result) = read(&peer, &handle, None).await;
    assert_eq!(result, json!({ "records": 2 }));
    let refs: Vec<&str> = streamed
        .iter()
        .filter_map(|(_, p)| p["row"]["ref"].as_str())
        .collect();
    assert_eq!(refs, ["work_item:notes:N-2", "work_item:notes:N-1"]);
    let (method, last) = streamed.last().unwrap();
    assert_eq!(
        (method.as_str(), last["state"].clone()),
        (notify::STATE, json!({ "cursor": 3 }))
    );
    let (_, again) = read(&peer, &handle, Some(json!({ "cursor": 3 }))).await;
    assert_eq!(again, json!({ "records": 0 }));
}

/// A mapping changed to return `from` as `to`, in a fresh extension copy.
async fn with_mapping(from: &str, to: &str) -> (tempfile::TempDir, Child, Peer, Handle) {
    let ext = extension();
    let mapping = ext.path().join("mcp/notes.star");
    let text = std::fs::read_to_string(&mapping).unwrap();
    assert!(text.contains(from));
    std::fs::write(&mapping, text.replace(from, to)).unwrap();
    let (child, peer) = spawn(ext.path());
    let handle = check(&peer).await.handle.unwrap();
    (ext, child, peer, handle)
}

#[tokio::test]
async fn a_mapping_may_not_return_an_undeclared_event_or_a_foreign_ref() {
    let (_ext, _child, peer, handle) = with_mapping(
        "\"type\": \"work_item.recorded\"",
        "\"type\": \"work_item.created\"",
    )
    .await;
    let undeclared = invoke(&peer, &handle, "create", json!({ "title": "x" })).await;
    assert!(
        matches!(&undeclared, Err(ProtocolError::Internal(m)) if m.contains("`work_item.created@1`")),
        "{undeclared:?}"
    );

    let (_ext, _child, peer, handle) = with_mapping(
        "return \"work_item:\" + x[\"provider\"]",
        "return \"work_item:oxplow:\" + \"\"",
    )
    .await;
    let foreign = invoke(&peer, &handle, "create", json!({ "title": "x" })).await;
    assert!(
        matches!(&foreign, Err(ProtocolError::Internal(m)) if m.contains("isn't one of this provider's")),
        "{foreign:?}"
    );
    let (_, read_back) = (
        (),
        peer.request(
            method::READ,
            serde_json::to_value(ReadParams {
                handle,
                collector: "items".into(),
                state: None,
            })
            .unwrap(),
        )
        .await,
    );
    assert!(
        read_back.is_err(),
        "a foreign record is refused too: {read_back:?}"
    );
}

/// P7 review (tsk719): a tool error the mapping doesn't refuse is still a
/// refusal of the input — never a provider failure that counts.
#[tokio::test]
async fn a_tool_error_the_mapping_passes_over_is_an_invalid_input() {
    let (_ext, _child, peer, handle) =
        with_mapping("    if x.get(\"error\"):\n", "    if False:\n").await;
    let missing = invoke(
        &peer,
        &handle,
        "update",
        json!({ "ref": "work_item:notes:N-9", "title": "x" }),
    )
    .await;
    assert!(
        matches!(&missing, Err(ProtocolError::InvalidInput { message, .. })
            if message.contains("no note `N-9`")),
        "{missing:?}"
    );
}

/// P7 review (tsk719): a read answer without a `state` is the mapping's
/// error, never a `null` checkpoint that restarts every read.
#[tokio::test]
async fn a_read_answer_without_a_state_is_refused() {
    let (_ext, _child, peer, handle) =
        with_mapping("\"state\": {\"cursor\": x[\"output\"][\"cursor\"]},", "").await;
    let params = serde_json::to_value(ReadParams {
        handle,
        collector: "items".into(),
        state: None,
    })
    .unwrap();
    let read = peer.request(method::READ, params).await;
    assert!(
        matches!(&read, Err(e) if e.to_string().contains("`state`")),
        "{read:?}"
    );
}

/// The pinned tools, changed by `edit`, in a fresh extension copy: the
/// `check` problems.
async fn check_with_pins(edit: impl FnOnce(&mut Vec<Value>)) -> Vec<Problem> {
    let ext = extension();
    let tools = ext.path().join("mcp/tools.json");
    let mut pins: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&tools).unwrap()).unwrap();
    edit(&mut pins);
    std::fs::write(&tools, serde_json::to_string(&pins).unwrap()).unwrap();
    let (_child, peer) = spawn(ext.path());
    let checked = check(&peer).await;
    assert_eq!(checked.handle, None, "{:?}", checked.problems);
    checked.problems
}

/// P7 review (tsk719): the pin is the whole tool — its annotations, title
/// and output schema too — and a tool pinned twice is refused.
#[tokio::test]
async fn the_pin_covers_the_whole_tool_and_each_name_once() {
    let at = |pins: &Vec<Value>| {
        pins.iter()
            .position(|t| t["name"] == "create_item")
            .unwrap()
    };
    let hinted = check_with_pins(|pins| {
        let i = at(pins);
        pins[i]["annotations"] = json!({ "destructiveHint": true });
    })
    .await;
    assert!(
        hinted[0].message.contains("`create_item` annotations"),
        "{hinted:?}"
    );
    let twice = check_with_pins(|pins| {
        let i = at(pins);
        let copy = pins[i].clone();
        pins.push(copy);
    })
    .await;
    assert!(
        twice[0].message.contains("`create_item` twice"),
        "{twice:?}"
    );
}

/// A copy of the fixture whose server never answers MCP's `initialize`,
/// writing its pid to `server.pid`.
fn hanging_extension() -> tempfile::TempDir {
    let ext = extension();
    let server = ext.path().join("bin/notes-server");
    std::fs::write(&server, "#!/bin/sh\necho $$ > server.pid\nexec sleep 60\n").unwrap();
    ext
}

/// P7 review (tsk720): a server stuck starting holds nothing the adapter
/// needs to answer `$/cancel`.
#[tokio::test]
async fn a_check_is_cancelled_while_the_server_hangs_starting() {
    let ext = hanging_extension();
    let (_child, peer) = spawn(ext.path());
    let params = serde_json::to_value(CheckParams {
        config: json!({}),
        credentials: vec![],
    })
    .unwrap();
    let call = peer.start(method::CHECK, params).await.unwrap();
    let pid_file = ext.path().join("server.pid");
    while !std::fs::read_to_string(&pid_file).is_ok_and(|p| p.ends_with('\n')) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    peer.cancel(call.id).await.unwrap();
    let reply = tokio::time::timeout(std::time::Duration::from_secs(5), call.reply())
        .await
        .expect("the cancel is answered");
    assert!(matches!(reply, Err(ProtocolError::Cancelled)), "{reply:?}");
}

/// P7 review (tsk720): `shutdown` with a call in flight stops the call and
/// the server, and the adapter exits.
#[tokio::test]
async fn shutdown_with_a_call_in_flight_stops_the_server() {
    let ext = hanging_extension();
    let (mut child, peer) = spawn(ext.path());
    let params = serde_json::to_value(CheckParams {
        config: json!({}),
        credentials: vec![],
    })
    .unwrap();
    let _call = peer.start(method::CHECK, params).await.unwrap();
    let pid_file = ext.path().join("server.pid");
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .filter(|p| p.ends_with('\n'))
        {
            break pid.trim().to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    peer.request(method::SHUTDOWN, Value::Null).await.unwrap();
    let exited = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
    assert!(exited.is_ok(), "the adapter exits");
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    for _ in 0..40 {
        if !alive() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the server {pid} is still running");
}

/// The notes server over HTTP on loopback (at `port`, or any), behind
/// `bearer`: its url, and the task serving it.
async fn http_notes(port: u16, bearer: Option<&str>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let bearer = bearer.map(str::to_string);
    let task = tokio::spawn(async move {
        let _ = oxplow_provider_mcp::notes::serve_http(listener, bearer).await;
    });
    (url, task)
}

/// The adapter in front of the server at `url`, holding `token` (or
/// nothing) as the credential `NOTES_TOKEN` its bearer is.
fn spawn_by_url(dir: &Path, url: &str, token: Option<&str>) -> (Child, Peer) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_oxplow-provider-mcp"));
    command
        .args([
            "--declarations",
            "provider.json",
            "--mapping",
            "mcp/notes.star",
            "--tools",
            "mcp/tools.json",
            "--url",
            url,
            "--auth-env",
            "NOTES_TOKEN",
        ])
        .current_dir(dir)
        .env("OXPLOW_PROVIDER_ID", "notes")
        .env_remove("NOTES_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    if let Some(token) = token {
        command.env("NOTES_TOKEN", token);
    }
    let mut child = command.spawn().unwrap();
    let (stdout, stdin) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let (peer, _incoming) = Peer::spawn(stdout, stdin);
    (child, peer)
}

async fn initialize(peer: &Peer) {
    let _: InitializeResult = peer
        .call(
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
        .unwrap();
}

async fn try_check(peer: &Peer) -> Result<CheckResult, ProtocolError> {
    initialize(peer).await;
    peer.call(
        method::CHECK,
        &CheckParams {
            config: json!({}),
            credentials: vec!["NOTES_TOKEN".into()],
        },
    )
    .await
}

/// P9.B4: an MCP server reached by url — over streamable HTTP, its bearer
/// the credential the adapter is told the name of — is pinned and called
/// like one it runs: the same check, the same mapping.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_by_url_is_pinned_and_called() {
    let ext = extension();
    let (url, server) = http_notes(0, Some("s3cret")).await;
    let port: u16 = url::Url::parse(&url).unwrap().port().unwrap();

    let (_child, peer) = spawn_by_url(ext.path(), &url, Some("s3cret"));
    let ok = try_check(&peer).await.unwrap();
    assert_eq!(ok.problems, vec![]);
    let handle = ok.handle.unwrap();
    let created = invoke(&peer, &handle, "create", json!({ "title": "First" }))
        .await
        .unwrap();
    assert_eq!(created.result, json!({ "ref": "work_item:notes:N-1" }));
    let moved = invoke(
        &peer,
        &handle,
        "transition",
        json!({ "ref": "work_item:notes:N-1", "to": "blocked" }),
    )
    .await
    .unwrap();
    assert_eq!(moved.events[0].payload["item"]["native_state"], "stuck");
    let (streamed, result) = read(&peer, &handle, None).await;
    assert_eq!(result["records"], 1);
    assert_eq!(streamed[0].1["row"]["ref"], "work_item:notes:N-1");

    // Its service stops taking the token mid-session (the server comes
    // back wanting another): the call answers `Auth`, which is what has
    // the host renew a signed-in credential and try again.
    server.abort();
    let _ = server.await;
    let (_, server) = http_notes(port, Some("rotated")).await;
    let refused = invoke(&peer, &handle, "create", json!({ "title": "Second" })).await;
    assert!(
        matches!(refused, Err(ProtocolError::Auth(_))),
        "{refused:?}"
    );

    // A bearer the server refuses is `Auth` at check; none to send is the
    // credential's problem.
    let (_child, wrong) = spawn_by_url(ext.path(), &url, Some("s3cret"));
    let refused = try_check(&wrong).await;
    assert!(
        matches!(refused, Err(ProtocolError::Auth(_))),
        "{refused:?}"
    );
    let (_child, without) = spawn_by_url(ext.path(), &url, None);
    let unset = try_check(&without).await.unwrap();
    assert_eq!(unset.handle, None);
    assert_eq!(unset.problems[0].path, "/credentials/NOTES_TOKEN");

    // The pin holds over HTTP too: a server whose tools aren't the pinned
    // ones is refused.
    let tools = ext.path().join("mcp/tools.json");
    let mut pins: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&tools).unwrap()).unwrap();
    pins[0]["description"] = json!("Not what the server says.");
    std::fs::write(&tools, serde_json::to_string(&pins).unwrap()).unwrap();
    let (_child, pinned) = spawn_by_url(ext.path(), &url, Some("rotated"));
    let refused = try_check(&pinned).await.unwrap();
    assert_eq!(refused.handle, None);
    assert!(
        refused.problems[0].message.contains("isn't the pinned one"),
        "{:?}",
        refused.problems
    );
    server.abort();
}
