//! Devices as commands: the host's paired devices, pairing, and "Allow
//! other devices", for foundation's Devices page (`oxplow/devices`). The
//! daemon talks to the host that started it as its owner, over the home's
//! owner socket (`oxplow_host_access::owner`): a daemon runs as the same user
//! with the same `OXPLOW_HOME`. A daemon no host started (one run alone)
//! reads `hosted: false`.
//!
//! - Read: the live datasets `v_host_device` (who is paired and connected)
//!   and `v_host_settings` (one row: the settings and whether the host runs
//!   with them, the port and where it came from, the addresses a device
//!   opens, this machine's private addresses and `.local` name, whether
//!   there is a root to trust, the code in force), from [`read`].
//! - Write: `oxplow.host.pair`, `rename_device`, `remove_device`,
//!   `set_settings`; each posts `HostChanged`, so the datasets are read
//!   again.
//!
//! The writes are a person's only (or a page a person pressed): they let a
//! device in. An agent, an effect or a job is refused. The datasets are as
//! readable as any (an agent on this machine could reach the owner socket
//! from its shell anyway: owner and device are roles, not a wall). Not
//! undoable: the host's own state, outside oxplow's database.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::commands::Confirm;
use oxplow_domain::{Actor, CommandError};
use oxplow_host_access::owner::{owner_call, OwnerCallError};
use oxplow_host_access::settings::HostSettings;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::events::OxplowEvent;
use oxplow_db::SqlCell;

pub const PAIR: &str = "oxplow.host.pair";
pub const RENAME_DEVICE: &str = "oxplow.host.rename_device";
pub const REMOVE_DEVICE: &str = "oxplow.host.remove_device";
pub const SET_SETTINGS: &str = "oxplow.host.set_settings";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The paired device (`dev-…`).
    pub id: String,
    /// Its new name.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceInput {
    /// The paired device (`dev-…`).
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    /// Other devices may reach this computer.
    #[serde(default)]
    pub allow_other_devices: bool,
    /// The IP address of this computer's they use (one of its private ones).
    #[serde(default)]
    pub address: String,
    /// The port they come in on; none is 7400.
    #[serde(default)]
    pub port: Option<u16>,
    /// Names a device may use besides the address (`studio.local`).
    #[serde(default)]
    pub names: Vec<String>,
    /// A certificate of your own (a PEM file) in place of the host's.
    #[serde(default)]
    pub tls_cert: Option<String>,
    /// Its key (a PEM file).
    #[serde(default)]
    pub tls_key: Option<String>,
}

/// The person's only: a person, or a page or component a person pressed.
fn person_only(actor: &Actor, what: &str) -> Result<(), CommandError> {
    if actor.may_confirm() {
        Ok(())
    } else {
        Err(CommandError::Denied {
            reason: format!("{what} is the person's: it lets a device into this computer"),
        })
    }
}

fn host_error(e: OwnerCallError) -> CommandError {
    match e {
        OwnerCallError::NoHost(_) => CommandError::Denied {
            reason: "no host runs for this oxplow: its devices are the host's".into(),
        },
        OwnerCallError::Refused(words) => CommandError::Denied { reason: words },
        OwnerCallError::Failed(words) => CommandError::Failed { message: words },
    }
}

/// A blocking owner call, off the async workers.
async fn call(
    home: PathBuf,
    method: &'static str,
    path: String,
    body: Option<Value>,
) -> Result<Value, OwnerCallError> {
    tokio::task::spawn_blocking(move || owner_call(&home, method, &path, body.as_ref()))
        .await
        .unwrap_or_else(|e| Err(OwnerCallError::Failed(e.to_string())))
}

/// What the host datasets hold (`v_host_device`, `v_host_settings`, read
/// live): the paired devices, and one row of this computer's standing.
pub struct HostRead {
    pub devices: Vec<Vec<SqlCell>>,
    pub settings: Vec<SqlCell>,
}

fn text(v: impl Into<String>) -> SqlCell {
    SqlCell::Text(v.into())
}
fn opt_text(v: Option<String>) -> SqlCell {
    v.map(SqlCell::Text).unwrap_or(SqlCell::Null(()))
}
fn flag(v: bool) -> SqlCell {
    SqlCell::Int(i64::from(v))
}
fn json_cell(v: &impl serde::Serialize) -> SqlCell {
    SqlCell::Text(serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
}

/// Read the host of `home`: as its owner over its socket; a home with no
/// host (a daemon run alone) is `hosted = 0`, and one that does not answer
/// says why (`problem`).
pub async fn read(home: &Path) -> HostRead {
    let saved = HostSettings::load(home);
    let (port, port_problem) = match saved.device_port() {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let mut hosted = false;
    let mut problem: Option<String> = None;
    let mut devices = Vec::new();
    let mut pairing: Option<Value> = None;
    let mut public = Value::Null;
    match call(home.to_path_buf(), "GET", "/host/status".into(), None).await {
        Ok(status) => {
            hosted = true;
            public = status["public"].clone();
            let connected: Vec<String> = status["connected"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .filter_map(|d| d["device"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            match call(home.to_path_buf(), "GET", "/host/devices".into(), None).await {
                Ok(list) => {
                    devices = list
                        .as_array()
                        .map(|l| {
                            l.iter()
                                .map(|d| {
                                    let id = d["id"].as_str().unwrap_or_default().to_string();
                                    vec![
                                        text(id.clone()),
                                        text(d["name"].as_str().unwrap_or_default()),
                                        text(d["kind"].as_str().unwrap_or_default()),
                                        d["paired_at"]
                                            .as_i64()
                                            .map(SqlCell::Int)
                                            .unwrap_or(SqlCell::Null(())),
                                        flag(connected.contains(&id)),
                                    ]
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Err(e) => problem = Some(e.to_string()),
            }
            pairing = call(home.to_path_buf(), "GET", "/host/pairing".into(), None)
                .await
                .ok()
                .filter(|v| !v.is_null());
        }
        Err(OwnerCallError::NoHost(_)) => {}
        Err(e) => problem = Some(e.to_string()),
    }
    let addresses: Vec<String> = oxplow_host_access::network::private_addresses()
        .iter()
        .map(|ip| ip.to_string())
        .collect();
    let settings = vec![
        flag(hosted),
        opt_text(problem),
        flag(saved.allow_other_devices),
        text(saved.address.clone()),
        json_cell(&saved.names),
        saved
            .port
            .map(|p| SqlCell::Int(i64::from(p)))
            .unwrap_or(SqlCell::Null(())),
        port.map(|p| SqlCell::Int(i64::from(p.port)))
            .unwrap_or(SqlCell::Null(())),
        opt_text(port.map(|p| {
            serde_json::to_value(p.from)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        })),
        opt_text(port_problem),
        opt_text(saved.tls_cert.as_ref().map(|p| p.display().to_string())),
        opt_text(saved.tls_key.as_ref().map(|p| p.display().to_string())),
        flag(hosted && applied(&saved, port.map(|p| p.port), &public)),
        json_cell(
            &port
                .map(|p| saved.open_on_device(p.port))
                .unwrap_or_default(),
        ),
        json_cell(&addresses),
        opt_text(oxplow_host_access::network::local_name()),
        flag(saved.tls_cert.is_none() && oxplow_host_access::root_path(home).is_file()),
        opt_text(
            pairing
                .as_ref()
                .and_then(|p| p["code"].as_str().map(str::to_string)),
        ),
        pairing
            .as_ref()
            .and_then(|p| p["expires_at"].as_i64())
            .map(SqlCell::Int)
            .unwrap_or(SqlCell::Null(())),
    ];
    HostRead { devices, settings }
}

/// Whether the host runs with `settings` now: its listener for other
/// devices is on the saved address and port when they are allowed, off
/// when not. Pure.
pub fn applied(settings: &HostSettings, port: Option<u16>, public: &Value) -> bool {
    let state = public["state"].as_str().unwrap_or("off");
    if !settings.allow_other_devices {
        return state == "off";
    }
    let (Some(port), "listening") = (port, state) else {
        return false;
    };
    let address = settings.address.trim();
    public["url"].as_str().is_some_and(|url| {
        let host = if address.contains(':') {
            format!("[{address}]")
        } else {
            address.to_string()
        };
        url.contains(&format!("{host}:{port}"))
    })
}

/// The command's answer; once it has run, the host datasets are read again
/// (a Devices page open shows it).
fn output(services: &ServicesSlot, result: Value) -> HandlerOutput {
    let events = services.get().ok().map(|svc| svc.events.clone());
    HandlerOutput {
        result,
        after_commit: events.map(|events| {
            Box::new(move || events.emit(OxplowEvent::HostChanged))
                as Box<dyn FnOnce() + Send + Sync>
        }),
        ..HandlerOutput::default()
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let home = |services: &ServicesSlot| -> Result<PathBuf, CommandError> {
        Ok(services.get()?.global_config_dir.clone())
    };
    let pair = {
        let services = services.clone();
        Op::new(
            "host.manage",
            "pair",
            schema::<NoInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Pairing a device")?;
                    let _: NoInput = parse(input)?;
                    let code = call(home(&services)?, "POST", "/host/pairing".into(), None)
                        .await
                        .map_err(host_error)?;
                    Ok(output(&services, code))
                })
            })),
        )
    };
    let rename = {
        let services = services.clone();
        Op::new(
            "host.manage",
            "rename_device",
            schema::<RenameInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Renaming a device")?;
                    let input: RenameInput = parse(input)?;
                    let name = input.name.trim().to_string();
                    if name.is_empty() {
                        return Err(super::util::invalid("/name", "a device needs a name"));
                    }
                    let device = call(
                        home(&services)?,
                        "PATCH",
                        format!("/host/devices/{}", input.id),
                        Some(json!({ "name": name })),
                    )
                    .await
                    .map_err(host_error)?;
                    Ok(output(&services, device))
                })
            })),
        )
    };
    let remove = {
        let services = services.clone();
        Op::new(
            "host.manage",
            "remove_device",
            schema::<DeviceInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Removing a device")?;
                    let input: DeviceInput = parse(input)?;
                    let device = call(
                        home(&services)?,
                        "DELETE",
                        format!("/host/devices/{}", input.id),
                        None,
                    )
                    .await
                    .map_err(host_error)?;
                    Ok(output(&services, device))
                })
            })),
        )
        .confirm_at_least(Confirm::Destructive)
    };
    let set_settings = {
        let services = services.clone();
        Op::new(
            "host.manage",
            "set_settings",
            schema::<SettingsInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Letting other devices in")?;
                    let input: SettingsInput = parse(input)?;
                    let path = |p: Option<String>| {
                        p.map(|p| p.trim().to_string())
                            .filter(|p| !p.is_empty())
                            .map(PathBuf::from)
                    };
                    let settings = HostSettings {
                        allow_other_devices: input.allow_other_devices,
                        address: input.address.trim().to_string(),
                        names: input
                            .names
                            .iter()
                            .map(|n| n.trim().to_string())
                            .filter(|n| !n.is_empty())
                            .collect(),
                        tls_cert: path(input.tls_cert),
                        tls_key: path(input.tls_key),
                        port: input.port,
                    };
                    settings
                        .save(&home(&services)?)
                        .map_err(|words| super::util::invalid("/address", words))?;
                    // They apply when the host starts next.
                    Ok(output(
                        &services,
                        json!({ "saved": true, "applies": "at the next start" }),
                    ))
                })
            })),
        )
    };
    vec![pair, rename, remove, set_settings]
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::Path;

    use oxplow_domain::{Actor, CommandError};
    use serde_json::{json, Value};

    /// A stand-in host on the home's owner socket: each request gets the
    /// answer `answer(method, path, body)` gives; it keeps what it was sent.
    fn stand_in(
        home: &Path,
        answer: impl Fn(&str, &str, &str) -> (u16, Value) + Send + 'static,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        let listener = std::os::unix::net::UnixListener::bind(
            oxplow_host_access::owner::owner_socket_path(home),
        )
        .unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let body = String::from_utf8(body).unwrap();
                let mut parts = first.split_whitespace();
                let (method, path) = (
                    parts.next().unwrap().to_string(),
                    parts.next().unwrap().to_string(),
                );
                kept.lock()
                    .unwrap()
                    .push(format!("{method} {path} {body}").trim().to_string());
                let (status, value) = answer(&method, &path, &body);
                let text = value.to_string();
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n{text}",
                    text.len()
                )
                .unwrap();
            }
        });
        seen
    }

    fn host_answers(method: &str, path: &str, _body: &str) -> (u16, Value) {
        match (method, path) {
            ("GET", "/host/status") => (
                200,
                json!({ "connected": [{ "device": "dev-1", "name": "Tab S9" }], "public": { "state": "off" } }),
            ),
            ("GET", "/host/devices") => (
                200,
                json!([
                    { "id": "dev-1", "name": "Tab S9", "kind": "tablet", "paired_at": 100 },
                    { "id": "dev-2", "name": "Phone", "kind": "phone", "paired_at": 200 }
                ]),
            ),
            ("GET", "/host/pairing") => (
                200,
                json!({ "code": "ABCD-EFGH", "expires_in": 600, "expires_at": 1 }),
            ),
            ("POST", "/host/pairing") => (
                200,
                json!({ "code": "WXYZ-2345", "expires_in": 600, "expires_at": 2 }),
            ),
            ("PATCH", "/host/devices/dev-1") => (200, json!({ "id": "dev-1", "name": "Kitchen" })),
            ("DELETE", "/host/devices/dev-9") => (404, json!({ "error": "no device dev-9" })),
            ("DELETE", "/host/devices/dev-2") => (200, json!({ "id": "dev-2" })),
            _ => (404, json!({ "error": "no such route" })),
        }
    }

    async fn run(
        fx: &crate::test_fixtures::ThreadFixture,
        actor: &Actor,
        name: &str,
        input: Value,
    ) -> Result<Value, CommandError> {
        fx.svc
            .commands
            .run(actor, name, input, true)
            .await
            .map(|o| o.result)
    }

    /// The rows of `sql` as JSON objects.
    async fn rows(fx: &crate::test_fixtures::ThreadFixture, sql: &str) -> Vec<Value> {
        let result = fx.svc.sql.query_sql(sql, vec![], None).await.unwrap();
        result
            .rows
            .into_iter()
            .map(|row| {
                let mut o = serde_json::Map::new();
                for (c, v) in result.columns.iter().zip(row) {
                    o.insert(c.clone(), serde_json::to_value(v).unwrap());
                }
                Value::Object(o)
            })
            .collect()
    }

    /// The datasets read the host: the paired devices (which are
    /// connected), the code in force; no host is `hosted = 0`, not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_datasets_read_the_host_and_no_host_is_said() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let alone = rows(&fx, "SELECT hosted, pairing_code FROM v_host_settings").await;
        assert_eq!(alone, [json!({ "hosted": 0, "pairing_code": null })]);
        assert!(rows(&fx, "SELECT * FROM v_host_device").await.is_empty());

        let seen = stand_in(&home, host_answers);
        let devices = rows(
            &fx,
            "SELECT id, name, connected FROM v_host_device ORDER BY id",
        )
        .await;
        assert_eq!(
            devices,
            [
                json!({ "id": "dev-1", "name": "Tab S9", "connected": 1 }),
                json!({ "id": "dev-2", "name": "Phone", "connected": 0 })
            ]
        );
        let standing = rows(
            &fx,
            "SELECT hosted, applied, pairing_code, pairing_until FROM v_host_settings",
        )
        .await;
        assert_eq!(
            standing,
            [json!({ "hosted": 1, "applied": 1, "pairing_code": "ABCD-EFGH", "pairing_until": 1 })]
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == "GET /host/devices"));
    }

    /// Pairing, renaming and removing go to the host as its owner; the
    /// host's refusal is its own words.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_pairs_renames_and_removes_through_the_host() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let seen = stand_in(&home, host_answers);
        let me = Actor::Human;
        assert_eq!(
            run(&fx, &me, super::PAIR, json!({})).await.unwrap()["code"],
            "WXYZ-2345"
        );
        assert_eq!(
            run(
                &fx,
                &me,
                super::RENAME_DEVICE,
                json!({ "id": "dev-1", "name": " Kitchen " })
            )
            .await
            .unwrap()["name"],
            "Kitchen"
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == r#"PATCH /host/devices/dev-1 {"name":"Kitchen"}"#));
        run(&fx, &me, super::REMOVE_DEVICE, json!({ "id": "dev-2" }))
            .await
            .unwrap();
        let refused = run(&fx, &me, super::REMOVE_DEVICE, json!({ "id": "dev-9" }))
            .await
            .unwrap_err();
        assert!(
            format!("{refused:?}").contains("no device dev-9"),
            "{refused:?}"
        );
    }

    /// An agent (or a page it pressed) may not let a device in.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_is_refused() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let page = Actor::Page {
            page_id: "oxplow/devices".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        for actor in [&agent, &page] {
            for (name, input) in [
                (super::PAIR, json!({})),
                (super::REMOVE_DEVICE, json!({ "id": "dev-1" })),
            ] {
                let refused = run(&fx, actor, name, input).await.unwrap_err();
                assert!(
                    matches!(refused, CommandError::Denied { .. }),
                    "{name}: {refused:?}"
                );
            }
        }
    }

    /// The settings are kept in the home's `host-settings.json`; one that
    /// cannot work is refused and nothing is kept.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_settings_are_kept_or_refused() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let me = Actor::Human;
        run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "192.168.1.20", "port": 7401, "names": ["studio.local", " "] }),
        )
        .await
        .unwrap();
        let kept = oxplow_host_access::settings::HostSettings::load(&home);
        assert!(kept.allow_other_devices);
        assert_eq!(kept.port, Some(7401));
        assert_eq!(kept.names, ["studio.local"]);
        let refused = run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(refused, CommandError::Invalid { .. }),
            "{refused:?}"
        );
        assert_eq!(
            oxplow_host_access::settings::HostSettings::load(&home).port,
            Some(7401),
            "unchanged"
        );
    }

    #[test]
    fn applied_is_the_hosts_listener_matching_the_settings() {
        use oxplow_host_access::settings::HostSettings;
        let on = HostSettings {
            allow_other_devices: true,
            address: "192.168.1.20".into(),
            ..Default::default()
        };
        let listening = json!({ "state": "listening", "url": "https://192.168.1.20:7400/" });
        assert!(super::applied(&on, Some(7400), &listening));
        assert!(!super::applied(&on, Some(7401), &listening), "another port");
        assert!(!super::applied(&on, Some(7400), &json!({ "state": "off" })));
        assert!(super::applied(
            &HostSettings::default(),
            Some(7400),
            &json!({ "state": "off" })
        ));
        assert!(!super::applied(
            &HostSettings::default(),
            Some(7400),
            &listening
        ));
    }
}
