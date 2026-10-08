use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::errors::{ErrorObject, ProtocolError, AUTH, INVALID_INPUT};
use oxplow_provider_protocol::{Incoming, Message, Peer};
use serde_json::json;

/// Every message shape survives its NDJSON line, `$/cancel` included.
#[test]
fn messages_round_trip_through_their_lines() {
    let messages = [
        Message::Request {
            id: 1,
            method: "invoke".into(),
            params: json!({ "handle": "h", "command": "create", "input": {} }),
        },
        Message::Response {
            id: 1,
            result: json!({ "result": null }),
        },
        Message::Error {
            id: Some(2),
            error: (&ProtocolError::InvalidInput {
                field: "/title".into(),
                message: "required".into(),
            })
                .into(),
        },
        Message::Notification {
            method: notify::CANCEL.into(),
            params: serde_json::to_value(notify::Cancel { id: 3 }).unwrap(),
        },
        Message::Notification {
            method: notify::RECORD.into(),
            params: json!({ "id": 4, "entity": "issue", "row": { "id": "W-1" } }),
        },
    ];
    for m in messages {
        let line = m.to_line();
        assert!(
            line.ends_with('\n') && !line[..line.len() - 1].contains('\n'),
            "{line}"
        );
        assert_eq!(Message::from_line(&line).unwrap(), m);
    }
    assert!(
        Message::from_line(r#"{"id":1,"result":{}}"#).is_err(),
        "no jsonrpc"
    );
    assert!(
        Message::from_line(r#"{"jsonrpc":"2.0","id":1}"#).is_err(),
        "no body"
    );
    assert!(Message::from_line("not json").is_err());
}

/// Typed errors keep their meaning across the wire.
#[test]
fn errors_keep_their_meaning() {
    let wire = ErrorObject::from(&ProtocolError::InvalidInput {
        field: "/title".into(),
        message: "required".into(),
    });
    assert_eq!(wire.code, INVALID_INPUT);
    assert_eq!(
        ProtocolError::from(wire),
        ProtocolError::InvalidInput {
            field: "/title".into(),
            message: "required".into()
        }
    );
    let limited = ProtocolError::from(ErrorObject::from(&ProtocolError::RateLimited {
        message: "slow down".into(),
        retry_after_ms: Some(500),
    }));
    assert_eq!(
        limited,
        ProtocolError::RateLimited {
            message: "slow down".into(),
            retry_after_ms: Some(500)
        }
    );
    assert_eq!(
        ProtocolError::from(ErrorObject::from(&ProtocolError::Cancelled)),
        ProtocolError::Cancelled
    );
}

/// tsk821: an `Auth` may say which credential was refused (`data.credential`)
/// — a service with two tokens refuses one — and the host renews only it.
#[test]
fn auth_carries_its_credential() {
    let named = ProtocolError::Auth {
        message: "the tracker refused it".into(),
        credential: Some("TRACKER_TOKEN".into()),
    };
    let wire = ErrorObject::from(&named);
    assert_eq!(wire.code, AUTH);
    assert_eq!(
        wire.data,
        Some(serde_json::json!({ "credential": "TRACKER_TOKEN" }))
    );
    assert_eq!(ProtocolError::from(wire), named);
    let unnamed = ProtocolError::Auth {
        message: "refused".into(),
        credential: None,
    };
    let wire = ErrorObject::from(&unnamed);
    assert_eq!(wire.data, None);
    assert_eq!(ProtocolError::from(wire), unnamed);
}

/// Two peers over a pipe: a request gets its reply, and a cancelled one
/// comes back `Cancelled` after the other side saw the `$/cancel`.
#[tokio::test]
async fn peers_answer_requests_and_cancellations() {
    let (a_io, b_io) = tokio::io::duplex(4096);
    let (a_read, a_write) = tokio::io::split(a_io);
    let (b_read, b_write) = tokio::io::split(b_io);
    let (host, _host_in) = Peer::spawn(a_read, a_write);
    let (provider, mut provider_in) = Peer::spawn(b_read, b_write);

    let answering = tokio::spawn(async move {
        let mut slow = None;
        while let Some(incoming) = provider_in.recv().await {
            match incoming {
                Incoming::Request { id, method, params } if method == "echo" => {
                    provider.respond(id, Ok(params)).await.unwrap();
                }
                Incoming::Request { id, method, .. } if method == "slow" => slow = Some(id),
                Incoming::Notification { method, params } if method == notify::CANCEL => {
                    let cancel: notify::Cancel = serde_json::from_value(params).unwrap();
                    assert_eq!(Some(cancel.id), slow);
                    provider
                        .respond(cancel.id, Err(ProtocolError::Cancelled))
                        .await
                        .unwrap();
                    return;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    });

    assert_eq!(
        host.request("echo", json!({ "hello": 1 })).await.unwrap(),
        json!({ "hello": 1 })
    );
    let call = host.start("slow", json!({})).await.unwrap();
    host.cancel(call.id).await.unwrap();
    assert_eq!(call.reply().await, Err(ProtocolError::Cancelled));
    answering.await.unwrap();
}

/// tsk549: once the other side's stream has ended, a new request fails at
/// once instead of waiting for a reply that can't come.
#[tokio::test]
async fn a_request_after_the_other_side_closed_fails_at_once() {
    let (host_end, provider_end) = tokio::io::duplex(1024);
    let (hr, hw) = tokio::io::split(host_end);
    let (host, _incoming) = Peer::spawn(hr, hw);
    // The provider closes its output but stays alive (still reading), so
    // the host's writes succeed and only the reply can never come.
    let (_provider_reads, mut provider_writes) = tokio::io::split(provider_end);
    tokio::io::AsyncWriteExt::shutdown(&mut provider_writes)
        .await
        .unwrap();
    for _ in 0..100 {
        if host.is_closed() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(host.is_closed());
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        host.request("echo", json!({})),
    )
    .await
    .expect("answered without waiting");
    assert!(reply.is_err());
}

/// P7.A3: a streaming request gets the notifications about it — in order,
/// on its own channel, which closes with its reply; a notification naming
/// another id (or none in flight) arrives on the general channel.
#[tokio::test]
async fn a_streaming_request_gets_its_own_notifications_until_its_reply() {
    let (a_io, b_io) = tokio::io::duplex(4096);
    let (a_read, a_write) = tokio::io::split(a_io);
    let (b_read, b_write) = tokio::io::split(b_io);
    let (host, mut host_in) = Peer::spawn(a_read, a_write);
    let (provider, mut provider_in) = Peer::spawn(b_read, b_write);

    tokio::spawn(async move {
        while let Some(incoming) = provider_in.recv().await {
            if let Incoming::Request { id, .. } = incoming {
                for (method, params) in [
                    (notify::PROGRESS, json!({ "id": id, "message": "page 1" })),
                    (
                        notify::RECORD,
                        json!({ "id": id, "entity": "work_item", "row": { "n": 1 } }),
                    ),
                    (notify::STATE, json!({ "id": id, "state": { "after": 1 } })),
                    (
                        notify::RECORD,
                        json!({ "id": 999, "entity": "work_item", "row": {} }),
                    ),
                ] {
                    provider.notify(method, params).await.unwrap();
                }
                provider
                    .respond(id, Ok(json!({ "records": 1 })))
                    .await
                    .unwrap();
            }
        }
    });

    let (call, mut stream) = host.start_streaming("read", json!({})).await.unwrap();
    let mut methods = Vec::new();
    while let Some(Incoming::Notification { method, .. }) = stream.recv().await {
        methods.push(method);
    }
    assert_eq!(
        methods,
        vec![notify::PROGRESS, notify::RECORD, notify::STATE]
    );
    assert_eq!(call.reply().await.unwrap(), json!({ "records": 1 }));
    let Some(Incoming::Notification { params, .. }) = host_in.recv().await else {
        panic!("the stray record arrives on the general channel");
    };
    assert_eq!(params["id"], 999);
}

/// A message is one line, and a line has a length limit: a peer that
/// sends a longer one is cut off — what was awaited fails saying why —
/// instead of the reader buffering it without end.
#[tokio::test]
async fn a_line_over_the_limit_ends_the_connection() {
    use oxplow_provider_protocol::peer::PeerLimits;
    let (host_end, provider_end) = tokio::io::duplex(1 << 16);
    let (hr, hw) = tokio::io::split(host_end);
    let limits = PeerLimits {
        max_line_bytes: 1024,
        ..PeerLimits::default()
    };
    let (host, _incoming) = Peer::spawn_with(hr, hw, limits);
    let (_provider_reads, mut provider_writes) = tokio::io::split(provider_end);
    let call = host.start("echo", json!({})).await.unwrap();
    let long = format!("{}\n", "x".repeat(4096));
    tokio::io::AsyncWriteExt::write_all(&mut provider_writes, long.as_bytes())
        .await
        .unwrap();
    let reply = tokio::time::timeout(std::time::Duration::from_secs(1), call.reply())
        .await
        .expect("answered without waiting");
    let err = reply.unwrap_err().to_string();
    assert!(err.contains("1024 bytes"), "{err}");
    assert!(host.is_closed());
}

/// `closed` resolves once the other side's stream has ended, saying how
/// many awaited replies the end failed.
#[tokio::test]
async fn closed_says_how_many_calls_the_end_cut_off() {
    let (host_end, provider_end) = tokio::io::duplex(1024);
    let (hr, hw) = tokio::io::split(host_end);
    let (host, _incoming) = Peer::spawn(hr, hw);
    let call = host.start("echo", json!({})).await.unwrap();
    drop(provider_end);
    tokio::time::timeout(std::time::Duration::from_secs(1), host.closed())
        .await
        .expect("it ends");
    assert_eq!(host.calls_cut_off(), 1);
    assert!(call.reply().await.is_err());
}
