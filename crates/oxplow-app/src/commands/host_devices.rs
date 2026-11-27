//! Devices as commands: the host's paired devices, pairing, and "Allow
//! other devices", for foundation's Devices page (`oxplow/devices`). The
//! daemon talks to the host that started it as its owner, over the home's
//! owner socket (`oxplow_host_access::owner`): a daemon runs as the same user
//! with the same `OXPLOW_HOME`. A daemon no host started (one run alone)
//! reads `hosted: false`.
//!
//! - `oxplow.host.devices` (read): who is paired and connected, the code in
//!   force, the settings and whether the host runs with them, the port and
//!   where it came from, the addresses a device opens, this machine's
//!   private addresses and `.local` name, whether there is a root to trust.
//! - `oxplow.host.pair`, `rename_device`, `remove_device`, `set_settings`.
//!
//! A person's only (or a page a person pressed): the read carries the code
//! in force, and the writes let a device in. An agent, an effect or a job is
//! refused. Not undoable: the host's own state, outside oxplow's database.

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

pub const DEVICES: &str = "oxplow.host.devices";
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

/// What the Devices page reads (`oxplow.host.devices`).
async fn devices(home: &Path) -> Value {
    let settings = HostSettings::load(home);
    let (port, port_problem) = match settings.device_port() {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let open_on_device = port
        .map(|p| settings.open_on_device(p.port))
        .unwrap_or_default();
    let mut out = json!({
        "hosted": false,
        "problem": null,
        "devices": [],
        "pairing": null,
        "settings": settings,
        "applied": false,
        "port": port,
        "port_problem": port_problem,
        "open_on_device": open_on_device,
        "addresses": oxplow_host_access::network::private_addresses().iter().map(|ip| ip.to_string()).collect::<Vec<_>>(),
        "local_name": oxplow_host_access::network::local_name(),
        "root_certificate": settings.tls_cert.is_none() && oxplow_host_access::root_path(home).is_file(),
    });
    let status = match call(home.to_path_buf(), "GET", "/host/status".into(), None).await {
        Ok(status) => status,
        Err(OwnerCallError::NoHost(_)) => return out,
        Err(e) => {
            out["problem"] = json!(e.to_string());
            return out;
        }
    };
    out["hosted"] = json!(true);
    let connected: Vec<String> = status["connected"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|d| d["device"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let listed = call(home.to_path_buf(), "GET", "/host/devices".into(), None).await;
    let pairing = call(home.to_path_buf(), "GET", "/host/pairing".into(), None).await;
    match listed {
        Ok(list) => {
            out["devices"] = json!(list
                .as_array()
                .map(|l| l
                    .iter()
                    .map(|d| json!({
                        "id": d["id"],
                        "name": d["name"],
                        "kind": d["kind"],
                        "paired_at": d["paired_at"],
                        "connected": d["id"].as_str().is_some_and(|id| connected.iter().any(|c| c == id)),
                    }))
                    .collect::<Vec<_>>())
                .unwrap_or_default());
        }
        Err(e) => out["problem"] = json!(e.to_string()),
    }
    if let Ok(code) = pairing {
        out["pairing"] = code;
    }
    out["applied"] = json!(applied(&settings, port.map(|p| p.port), &status["public"]));
    out
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

fn output(result: Value) -> HandlerOutput {
    HandlerOutput {
        result,
        ..HandlerOutput::default()
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let home = |services: &ServicesSlot| -> Result<PathBuf, CommandError> {
        Ok(services.get()?.global_config_dir.clone())
    };
    let read = {
        let services = services.clone();
        Op::new(
            "host.read",
            "devices",
            schema::<NoInput>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    person_only(invocation.actor(), "Reading the host's devices")?;
                    let _: NoInput = parse(input)?;
                    Ok(output(devices(&home(&services)?).await))
                })
            })),
        )
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
                    Ok(output(code))
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
                    Ok(output(device))
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
                    Ok(output(device))
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
                        json!({ "saved": true, "applies": "at the next start" }),
                    ))
                })
            })),
        )
    };
    vec![read, pair, rename, remove, set_settings]
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

    /// The page's read: the paired devices (which are connected), the code
    /// in force, the settings; no host is `hosted: false`, not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_devices_read_from_the_host_and_no_host_is_said() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let home = fx.svc.global_config_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        let alone = run(&fx, &Actor::Human, super::DEVICES, json!({}))
            .await
            .unwrap();
        assert_eq!(alone["hosted"], false);
        assert_eq!(alone["devices"], json!([]));

        let seen = stand_in(&home, host_answers);
        let read = run(&fx, &Actor::Human, super::DEVICES, json!({}))
            .await
            .unwrap();
        assert_eq!(read["hosted"], true);
        assert_eq!(read["devices"][0]["name"], "Tab S9");
        assert_eq!(read["devices"][0]["connected"], true);
        assert_eq!(read["devices"][1]["connected"], false);
        assert_eq!(read["pairing"]["code"], "ABCD-EFGH");
        assert_eq!(read["applied"], true, "off and off");
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

    /// An agent (or a page it pressed) may not read the code or let a
    /// device in.
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
            for (name, input) in [(super::DEVICES, json!({})), (super::PAIR, json!({}))] {
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
