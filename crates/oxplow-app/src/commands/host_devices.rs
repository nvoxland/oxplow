//! Devices as commands: the host's paired devices, pairing, and "Allow
//! other devices", for foundation's Devices page (`oxplow/devices`). The
//! daemon talks to the host that started it as its owner, over the home's
//! owner socket (`oxplow_host_access::owner`): a daemon runs as the same user
//! with the same `OXPLOW_HOME`. A daemon no host started (one run alone)
//! reads `hosted: false`.
//!
//! - Read: the live datasets `v_host_device` (who is paired and connected)
//!   and `v_host_settings` (one row: the saved settings, the port in force
//!   and the flags of the host's run over them, the addresses a device
//!   opens, this machine's private addresses and `.local` name, whether
//!   there is a root to trust, who waits to pair), from [`read`].
//! - Write: `oxplow.host.pair` (the code a device shows: it is paired),
//!   `rename_device`, `remove_device`,
//!   `set_settings`; each posts `HostChanged`, so the datasets are read
//!   again. So does a change the host makes itself (a pairing finished on
//!   the device, a device connecting): [`watch_host`] waits on the host's
//!   `/host/changes`.
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
use oxplow_host_access::settings::{Effective, HostSettings};
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
pub struct PairInput {
    /// The code the device shows (`ABCD-EFGH`; any case, the dash or none).
    pub code: String,
}

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
    let mut saved = HostSettings::default();
    let mut in_force: Option<Effective> = None;
    let mut settings_problem: Option<String> = None;
    let mut hosted = false;
    let mut problem: Option<String> = None;
    let mut devices = Vec::new();
    let mut waiting = Value::Array(Vec::new());
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
            if let Ok(list) =
                call(home.to_path_buf(), "GET", "/host/pair/waiting".into(), None).await
            {
                waiting = list;
            }
            match call(home.to_path_buf(), "GET", "/host/settings".into(), None).await {
                Ok(view) => {
                    saved = serde_json::from_value(view["saved"].clone()).unwrap_or_default();
                    in_force = serde_json::from_value(view["effective"].clone()).ok();
                    settings_problem = view["problem"].as_str().map(str::to_string);
                }
                Err(e) => settings_problem = Some(e.to_string()),
            }
            // The listener for other devices could not start as they say.
            if public["state"].as_str() == Some("failed") {
                settings_problem = public["error"].as_str().map(str::to_string);
            }
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
        in_force
            .as_ref()
            .map(|e| SqlCell::Int(i64::from(e.port.port)))
            .unwrap_or(SqlCell::Null(())),
        opt_text(in_force.as_ref().map(|e| {
            serde_json::to_value(e.port.from)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        })),
        opt_text(settings_problem),
        opt_text(saved.tls_cert.as_ref().map(|p| p.display().to_string())),
        opt_text(saved.tls_key.as_ref().map(|p| p.display().to_string())),
        json_cell(
            &in_force
                .as_ref()
                .map(|e| e.overridden.clone())
                .unwrap_or_default(),
        ),
        json_cell(&open_on_device(&public)),
        json_cell(&addresses),
        opt_text(oxplow_host_access::network::local_name()),
        flag(
            in_force.as_ref().is_some_and(|e| e.tls_cert.is_none())
                && oxplow_host_access::root_path(home).is_file(),
        ),
        // The devices waiting for the owner to type their codes.
        json_cell(&waiting),
        // The host's own listener for other devices, as it runs now: its
        // address.
        opt_text(open_on_device(&public).into_iter().next()),
    ];
    HostRead { devices, settings }
}

/// What a device opens, from the host's listener for other devices as it
/// runs (`/host/status` `public`): its address, then each name it answers
/// to (this computer's `.local` name among them, which works where the
/// network passes it), on its port. None while it doesn't listen. Pure.
pub fn open_on_device(public: &Value) -> Vec<String> {
    if public["state"].as_str() != Some("listening") {
        return Vec::new();
    }
    let Some(url) = public["url"].as_str() else {
        return Vec::new();
    };
    let url = url.trim_end_matches('/');
    let port = url.rsplit_once(':').map(|(_, p)| p).unwrap_or_default();
    let names = public["names"].as_array().into_iter().flatten();
    std::iter::once(format!("{url}/"))
        .chain(
            names
                .filter_map(Value::as_str)
                .map(|n| format!("https://{n}:{port}/")),
        )
        .collect()
}

/// Read the host datasets again whenever the host says its devices or
/// settings changed: a pairing finished on the device, a code made by the
/// CLI, a device connecting, another window's change. One wait on the
/// host's `/host/changes` at a time (it answers when the count moves); with
/// no host (a daemon run alone, or before its host is up) it asks again
/// now and then. Stops with the services.
pub fn watch_host(state: &Arc<crate::Services>) {
    let services = Arc::downgrade(state);
    let stopping = state.stopping();
    tokio::spawn(async move {
        let mut seen: Option<u64> = None;
        loop {
            let Some(svc) = services.upgrade() else {
                return;
            };
            let home = svc.global_config_dir.clone();
            let events = svc.events.clone();
            drop(svc);
            let path = match seen {
                Some(n) => format!("/host/changes?after={n}"),
                None => "/host/changes".to_string(),
            };
            let wait = tokio::select! {
                _ = stopping.cancelled() => return,
                answer = call(home, "GET", path, None) => match answer {
                    Ok(answer) => {
                        let now = answer["n"].as_u64();
                        if now.is_some() && now != seen {
                            seen = now;
                            events.emit(OxplowEvent::HostChanged);
                        }
                        std::time::Duration::ZERO
                    }
                    Err(OwnerCallError::NoHost(_)) => {
                        seen = None;
                        std::time::Duration::from_secs(10)
                    }
                    Err(_) => std::time::Duration::from_secs(2),
                },
            };
            tokio::select! {
                _ = stopping.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
        }
    });
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
            schema::<PairInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Pairing a device")?;
                    let input: PairInput = parse(input)?;
                    let code = input.code.trim().to_string();
                    if code.is_empty() {
                        return Err(super::util::invalid(
                            "/code",
                            "type the code the device shows",
                        ));
                    }
                    let body = json!({ "code": code });
                    let device = call(
                        home(&services)?,
                        "POST",
                        "/host/pair/approve".into(),
                        Some(body),
                    )
                    .await
                    .map_err(|e| match e {
                        OwnerCallError::Refused(words) => super::util::invalid("/code", words),
                        other => host_error(other),
                    })?;
                    Ok(output(&services, device))
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
                    let wanted = HostSettings {
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
                    // What cannot work is said here, at its field; the host
                    // checks again, applies them at once and keeps them.
                    let kept = wanted.checked().map_err(|e| {
                        super::util::invalid(&format!("/{}", e.field()), e.to_string())
                    })?;
                    let body = serde_json::to_value(&kept).map_err(|e| CommandError::Failed {
                        message: e.to_string(),
                    })?;
                    let view = call(home(&services)?, "PUT", "/host/settings".into(), Some(body))
                        .await
                        .map_err(|e| match e {
                            OwnerCallError::Refused(words) => {
                                super::util::invalid("/address", words)
                            }
                            other => host_error(other),
                        })?;
                    Ok(output(&services, view))
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

    fn host_answers(method: &str, path: &str, body: &str) -> (u16, Value) {
        if path.starts_with("/host/changes") {
            // The host waits for a change before it answers the same count.
            if path.contains("after=") {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            return (200, json!({ "n": 3 }));
        }
        match (method, path) {
            ("GET", "/host/settings") => (
                200,
                json!({
                    "saved": { "allowOtherDevices": true, "address": "192.168.1.20", "names": [], "port": null, "tlsCert": null, "tlsKey": null },
                    "effective": {
                        "allowOtherDevices": true, "address": "192.168.1.20", "names": [],
                        "port": { "port": 7500, "from": "flag" }, "tlsCert": null, "tlsKey": null,
                        "overridden": ["--device-port"]
                    },
                    "problem": null
                }),
            ),
            ("PUT", "/host/settings") if body.contains("7999") => (
                409,
                json!({ "error": "bind 192.168.1.20:7999 failed: in use", "field": "address" }),
            ),
            ("PUT", "/host/settings") => (200, json!({})),
            ("GET", "/host/status") => (
                200,
                json!({
                    "connected": [{ "device": "dev-1", "name": "Tab S9" }],
                    "public": { "state": "failed", "error": "bind 192.168.1.20:7500 failed: in use" }
                }),
            ),
            ("GET", "/host/devices") => (
                200,
                json!([
                    { "id": "dev-1", "name": "Tab S9", "kind": "tablet", "paired_at": 100 },
                    { "id": "dev-2", "name": "Phone", "kind": "phone", "paired_at": 200 }
                ]),
            ),
            ("GET", "/host/pair/waiting") => {
                (200, json!([{ "name": "Garage tablet", "kind": "tablet" }]))
            }
            ("POST", "/host/pair/approve") if body.to_uppercase().contains("WXYZ-2345") => (
                200,
                json!({ "id": "dev-3", "name": "Garage tablet", "kind": "tablet", "paired_at": 3 }),
            ),
            ("POST", "/host/pair/approve") => (
                404,
                json!({ "error": "no device is waiting with that code; check the code its screen shows" }),
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
        let alone = rows(&fx, "SELECT hosted, waiting FROM v_host_settings").await;
        assert_eq!(alone, [json!({ "hosted": 0, "waiting": "[]" })]);
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
            "SELECT hosted, address, port, port_from, overrides, port_problem, waiting FROM v_host_settings",
        )
        .await;
        assert_eq!(
            standing,
            [json!({
                "hosted": 1, "address": "192.168.1.20", "port": 7500, "port_from": "flag",
                "overrides": "[\"--device-port\"]",
                "port_problem": "bind 192.168.1.20:7500 failed: in use",
                "waiting": "[{\"name\":\"Garage tablet\",\"kind\":\"tablet\"}]"
            })]
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
            run(&fx, &me, super::PAIR, json!({ "code": " wxyz-2345 " }))
                .await
                .unwrap()["name"],
            "Garage tablet"
        );
        let wrong = run(&fx, &me, super::PAIR, json!({ "code": "AAAA-AAAA" }))
            .await
            .unwrap_err();
        assert!(
            matches!(&wrong, CommandError::Invalid { field: Some(f), message } if f == "/code" && message.contains("no device is waiting")),
            "{wrong:?}"
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
                (super::PAIR, json!({ "code": "WXYZ-2345" })),
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

    /// The settings go to the host, which keeps and applies them; one that
    /// cannot work is refused here with its field, before the host is asked,
    /// and the host's own refusal (a port in use) is its words.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_settings_go_to_the_host_or_are_refused() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let me = Actor::Human;
        let none = run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "192.168.1.20" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(none, CommandError::Denied { .. }),
            "no host: {none:?}"
        );

        let seen = stand_in(&home, host_answers);
        run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "192.168.1.20", "port": 7401, "names": ["studio.local", " "] }),
        )
        .await
        .unwrap();
        let sent = seen
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.starts_with("PUT"))
            .cloned()
            .unwrap();
        let body: Value =
            serde_json::from_str(sent.trim_start_matches("PUT /host/settings ")).unwrap();
        assert_eq!(body["port"], 7401);
        assert_eq!(body["names"], json!(["studio.local"]));

        let refused = run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&refused, CommandError::Invalid { field: Some(f), .. } if f == "/address"),
            "{refused:?}"
        );
        let puts = || {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|s| s.starts_with("PUT"))
                .count()
        };
        assert_eq!(puts(), 1, "refused here, the host never asked");

        let in_use = run(
            &fx,
            &me,
            super::SET_SETTINGS,
            json!({ "allow_other_devices": true, "address": "192.168.1.20", "port": 7999 }),
        )
        .await
        .unwrap_err();
        assert!(format!("{in_use:?}").contains("in use"), "{in_use:?}");
    }

    /// A pairing finished on the device happens in the host: the daemon
    /// hears of it (`/host/changes`) and reads its host datasets again, so
    /// an open Devices page shows the new device.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_daemon_reads_the_host_again_when_its_devices_change() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let seen = stand_in(&home, host_answers);
        let mut events = fx.svc.events.subscribe_ui();
        super::watch_host(&fx.svc);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if matches!(
                    events.recv().await,
                    Ok(crate::events::OxplowEvent::HostChanged)
                ) {
                    return;
                }
            }
        })
        .await
        .expect("told to read the host again");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !seen
                .lock()
                .unwrap()
                .iter()
                .any(|s| s == "GET /host/changes?after=3")
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("then waits for the next change");
    }

    #[test]
    fn a_device_opens_the_address_first_then_each_name_on_its_port() {
        let listening = json!({
            "state": "listening",
            "url": "https://192.168.1.20:7400",
            "names": ["studio.local"],
        });
        assert_eq!(
            super::open_on_device(&listening),
            ["https://192.168.1.20:7400/", "https://studio.local:7400/"]
        );
        assert_eq!(
            super::open_on_device(
                &json!({ "state": "listening", "url": "https://[fd7a::1]:7401" })
            ),
            ["https://[fd7a::1]:7401/"]
        );
        assert!(super::open_on_device(&json!({ "state": "off" })).is_empty());
    }
}
