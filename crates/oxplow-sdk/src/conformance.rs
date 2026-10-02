//! The provider conformance kit (P5.D5, `.context/providers.md` "The
//! conformance kit"): a [`ReferenceClient`] drives a provider over stdio
//! — spawned the way the host spawns it — validating every message
//! against the protocol's schema goldens and recording a transcript,
//! which [`normalize`] makes comparable (ids renumbered, volatile values
//! `$any`) and [`first_mismatch`] compares against a golden.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oxplow_app::providers::host::{self, HostError, Launch, Spawned};
use oxplow_provider_protocol::codec::{notify, Message};
use oxplow_provider_protocol::schemas::{for_message, validate};
use oxplow_provider_protocol::Peer;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// A golden transcript value that matches anything.
pub const ANY: &str = "$any";

/// Who sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Host,
    Provider,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Host => "host",
            Side::Provider => "provider",
        }
    }
}

/// What the tap saw: every message in order, and every one that broke
/// the protocol.
#[derive(Default)]
struct Monitor {
    transcript: Vec<(Side, Value)>,
    violations: Vec<String>,
    /// The method of each request still awaiting its reply, by sender and id.
    pending: HashMap<(Side, u64), String>,
}

impl Monitor {
    fn saw(&mut self, from: Side, line: &str) {
        // An error reply is checked against the `error` golden as sent
        // (the codec would only say it isn't JSON-RPC).
        if let Ok(raw) = serde_json::from_str::<Value>(line) {
            if let (Some(error), None) = (raw.get("error"), raw.get("method")) {
                if let Err(errors) = validate("error", error) {
                    self.violations.push(format!(
                        "the {}'s error doesn't match the protocol: {}",
                        from.name(),
                        errors.join("; ")
                    ));
                    self.transcript.push((from, raw));
                    return;
                }
            }
        }
        let message = match Message::from_line(line) {
            Ok(m) => m,
            Err(e) => {
                self.violations.push(format!(
                    "the {} sent a line that isn't JSON-RPC: {e}",
                    from.name()
                ));
                self.transcript
                    .push((from, Value::String(line.trim_end().to_string())));
                return;
            }
        };
        let other = match from {
            Side::Host => Side::Provider,
            Side::Provider => Side::Host,
        };
        let checked = match &message {
            Message::Request { id, method, params } => {
                self.pending.insert((from, *id), method.clone());
                Some((method.clone(), false, params))
            }
            Message::Notification { method, .. }
                if from == Side::Provider && HOST_ONLY.contains(&method.as_str()) =>
            {
                self.violations.push(format!(
                    "the provider sent `{method}`, which only the host sends"
                ));
                None
            }
            Message::Notification { method, params } => Some((method.clone(), false, params)),
            Message::Response { id, result } => match self.pending.remove(&(other, *id)) {
                Some(method) => Some((method, true, result)),
                None => {
                    self.unasked(from, other, *id);
                    None
                }
            },
            Message::Error { id, .. } => {
                if let Some(id) = id {
                    if self.pending.remove(&(other, *id)).is_none() {
                        self.unasked(from, other, *id);
                    }
                }
                None
            }
        };
        if let Some((method, is_result, payload)) = checked {
            match for_message(&method, is_result) {
                Some(schema) => {
                    if let Err(errors) = validate(schema, payload) {
                        self.violations.push(format!(
                            "the {}'s `{method}` {} doesn't match the protocol: {}",
                            from.name(),
                            if is_result { "result" } else { "params" },
                            errors.join("; ")
                        ));
                    }
                }
                // `shutdown` carries nothing to check. A provider may send
                // only the protocol's notifications (a host request it
                // sends is answered MethodNotFound, not a violation).
                None if from == Side::Provider
                    && !is_result
                    && !matches!(message, Message::Request { .. }) =>
                {
                    self.violations.push(format!(
                        "the provider sent `{method}`, which isn't in the protocol"
                    ))
                }
                None => {}
            }
        }
        self.transcript.push((from, message.to_value()));
    }
}

/// The notifications only the host sends.
const HOST_ONLY: &[&str] = &[notify::CANCEL];

impl Monitor {
    /// A reply to an id the other side never asked (or already had
    /// answered).
    fn unasked(&mut self, from: Side, other: Side, id: u64) {
        self.violations.push(format!(
            "the {} answered id {id}, which no request of the {} is waiting on",
            from.name(),
            other.name()
        ));
    }
}

/// Copy NDJSON lines from `from` to `to`, showing each to the monitor.
fn pump<R, W>(side: Side, from: R, mut to: W, monitor: Arc<Mutex<Monitor>>)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(from).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            monitor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .saw(side, &line);
            let written = to.write_all(line.as_bytes()).await;
            if written.is_err() || to.write_all(b"\n").await.is_err() || to.flush().await.is_err() {
                break;
            }
        }
        let _ = to.shutdown().await;
    });
}

/// A provider under test: the host's spawn, with every message between
/// the two tapped.
pub struct ReferenceClient {
    pub peer: Peer,
    child: tokio::process::Child,
    _proxy: Option<oxplow_app::net_sandbox::EgressProxy>,
    monitor: Arc<Mutex<Monitor>>,
}

/// What a session recorded.
pub struct Session {
    /// Every message, in order, as sent.
    pub transcript: Vec<(Side, Value)>,
    /// Messages that broke the protocol.
    pub violations: Vec<String>,
}

impl ReferenceClient {
    /// Spawn the provider as the host would (no consent check: the
    /// person running the kit is running their own provider).
    pub async fn start(launch: &Launch) -> Result<Self, HostError> {
        let Spawned {
            child,
            stdin,
            stdout,
            proxy,
        } = host::spawn(launch).await?;
        let monitor = Arc::new(Mutex::new(Monitor::default()));
        let (peer_end, tap_end) = tokio::io::duplex(1 << 16);
        let (peer_read, peer_write) = tokio::io::split(peer_end);
        let (tap_read, tap_write) = tokio::io::split(tap_end);
        pump(Side::Provider, stdout, tap_write, monitor.clone());
        pump(Side::Host, tap_read, stdin, monitor.clone());
        let (peer, incoming) = Peer::spawn(peer_read, peer_write);
        host::serve_incoming(peer.clone(), incoming);
        Ok(Self {
            peer,
            child,
            _proxy: proxy,
            monitor,
        })
    }

    /// The `params` of every `method` notification the provider has sent
    /// so far, in order (`$/record`, `$/state`, `$/progress`).
    pub fn provider_notifications(&self, method: &str) -> Vec<Value> {
        let m = self.monitor.lock().unwrap_or_else(|e| e.into_inner());
        m.transcript
            .iter()
            .filter(|(from, msg)| {
                *from == Side::Provider
                    && msg.get("id").is_none()
                    && msg.get("method").and_then(Value::as_str) == Some(method)
            })
            .map(|(_, msg)| msg.get("params").cloned().unwrap_or(Value::Null))
            .collect()
    }

    /// Ask it to shut down, wait for it to go, and hand back what was
    /// recorded.
    pub async fn finish(mut self) -> Session {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.peer.request(
                oxplow_provider_protocol::model::method::SHUTDOWN,
                Value::Null,
            ),
        )
        .await;
        drop(self.peer);
        if tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        // Let the pumps see the last lines.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let m = std::mem::take(&mut *self.monitor.lock().unwrap_or_else(|e| e.into_inner()));
        Session {
            transcript: m.transcript,
            violations: m.violations,
        }
    }
}

/// The transcript as golden lines: ids renumbered from 1 per sender in
/// the order its requests appear (replies and `$/…` notifications
/// follow), and the host's version `$any`. One JSON object per line.
pub fn normalize(transcript: &[(Side, Value)]) -> Vec<String> {
    let mut ids: HashMap<(Side, u64), u64> = HashMap::new();
    let mut next: HashMap<Side, u64> = HashMap::new();
    let mut renumber = |side: Side, id: u64, fresh: bool| -> u64 {
        if let Some(n) = ids.get(&(side, id)) {
            return *n;
        }
        if !fresh {
            return id;
        }
        let n = next.entry(side).or_insert(0);
        *n += 1;
        ids.insert((side, id), *n);
        *n
    };
    transcript
        .iter()
        .map(|(from, message)| {
            let mut m = message.clone();
            let other = match from {
                Side::Host => Side::Provider,
                Side::Provider => Side::Host,
            };
            let is_request = m.get("method").is_some() && m.get("id").is_some();
            if let Some(id) = m.get("id").and_then(Value::as_u64) {
                let n = if is_request {
                    renumber(*from, id, true)
                } else {
                    renumber(other, id, false)
                };
                m["id"] = json!(n);
            }
            // Notifications about an in-flight request name the host's id.
            let about_request = matches!(
                m.get("method").and_then(Value::as_str),
                Some(notify::CANCEL | notify::PROGRESS | notify::RECORD | notify::STATE)
            );
            if about_request {
                if let Some(id) = m.pointer("/params/id").and_then(Value::as_u64) {
                    m["params"]["id"] = json!(renumber(Side::Host, id, false));
                }
            }
            if m.get("method").and_then(Value::as_str)
                == Some(oxplow_provider_protocol::model::method::INITIALIZE)
            {
                if let Some(v) = m.pointer_mut("/params/host/version") {
                    *v = json!(ANY);
                }
            }
            json!({ "from": from.name(), "message": m }).to_string()
        })
        .collect()
}

/// Where `actual` first differs from `golden` (a JSON pointer), with the
/// two values; `"$any"` in the golden matches anything.
pub fn first_mismatch(golden: &Value, actual: &Value) -> Option<(String, Value, Value)> {
    fn walk(path: String, g: &Value, a: &Value) -> Option<(String, Value, Value)> {
        if g.as_str() == Some(ANY) {
            return None;
        }
        match (g, a) {
            (Value::Object(x), Value::Object(y)) => {
                let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
                keys.into_iter().find_map(|k| {
                    walk(
                        format!("{path}/{k}"),
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    )
                })
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| walk(format!("{path}/{i}"), p, q)),
            _ if g == a => None,
            _ => Some((
                if path.is_empty() { "/".into() } else { path },
                g.clone(),
                a.clone(),
            )),
        }
    }
    walk(String::new(), golden, actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_renumber_per_sender_and_any_matches_anything() {
        let transcript = vec![
            (
                Side::Host,
                json!({ "jsonrpc": "2.0", "id": 7, "method": "initialize", "params": { "protocol_version": "1", "host": { "name": "oxplow", "version": "0.7.0" } } }),
            ),
            (
                Side::Provider,
                json!({ "jsonrpc": "2.0", "id": 7, "result": {} }),
            ),
            (
                Side::Host,
                json!({ "jsonrpc": "2.0", "id": 9, "method": "read", "params": {} }),
            ),
            (
                Side::Provider,
                json!({ "jsonrpc": "2.0", "method": "$/state", "params": { "id": 9, "state": 1 } }),
            ),
            (
                Side::Provider,
                json!({ "jsonrpc": "2.0", "id": 9, "result": { "records": 0 } }),
            ),
        ];
        let lines = normalize(&transcript);
        let parsed: Vec<Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(parsed[0]["message"]["id"], 1);
        assert_eq!(parsed[0]["message"]["params"]["host"]["version"], ANY);
        assert_eq!(parsed[1]["message"]["id"], 1);
        assert_eq!(parsed[2]["message"]["id"], 2);
        assert_eq!(parsed[3]["message"]["params"]["id"], 2);
        assert_eq!(parsed[4]["from"], "provider");

        assert_eq!(
            first_mismatch(&json!({ "a": ANY }), &json!({ "a": [1] })),
            None
        );
        assert_eq!(
            first_mismatch(&json!({ "a": { "b": 1 } }), &json!({ "a": { "b": 2 } })),
            Some(("/a/b".into(), json!(1), json!(2)))
        );
        assert!(first_mismatch(&json!([1]), &json!([1, 2])).is_some());
    }

    #[test]
    fn the_monitor_flags_what_breaks_the_protocol() {
        let mut m = Monitor::default();
        m.saw(
            Side::Host,
            r#"{"jsonrpc":"2.0","id":1,"method":"check","params":{"config":{},"credentials":[]}}"#,
        );
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","id":1,"result":{"problems":"none"}}"#,
        );
        m.saw(Side::Provider, "not json");
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","method":"made/up","params":{}}"#,
        );
        assert_eq!(m.violations.len(), 3, "{:?}", m.violations);
        assert!(
            m.violations[0].contains("`check` result"),
            "{:?}",
            m.violations
        );
        assert_eq!(m.transcript.len(), 4);
    }

    /// tsk570: an error reply is checked against the `error` golden, a
    /// reply must answer a request the other side sent, and a provider
    /// may not send the host's notifications.
    #[test]
    fn the_monitor_checks_errors_replies_and_who_notifies() {
        let mut m = Monitor::default();
        let ask = |m: &mut Monitor, id: u64| {
            m.saw(
                Side::Host,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"method":"check","params":{{"config":{{}},"credentials":[]}}}}"#
                ),
            )
        };
        ask(&mut m, 1);
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"x","why":"extra"}}"#,
        );
        ask(&mut m, 2);
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"fine"}}"#,
        );
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","id":99,"result":{"problems":[]}}"#,
        );
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","id":98,"error":{"code":-32603,"message":"late"}}"#,
        );
        m.saw(
            Side::Provider,
            r#"{"jsonrpc":"2.0","method":"$/cancel","params":{"id":1}}"#,
        );
        let v = &m.violations;
        assert_eq!(v.len(), 4, "{v:?}");
        assert!(v[0].contains("error") && v[0].contains("why"), "{v:?}");
        assert!(v[1].contains("99"), "{v:?}");
        assert!(v[2].contains("98"), "{v:?}");
        assert!(v[3].contains("$/cancel") && v[3].contains("host"), "{v:?}");
    }
}
