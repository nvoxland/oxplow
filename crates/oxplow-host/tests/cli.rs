//! A headless host prints its owner token once and keeps it nowhere: the CLI
//! reaches it over the owner socket (`host.sock`, mode 0600) to make a
//! device (the code it shows), list devices and stop it.

// Terse unwraps are the assertion style in test crates.
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cli(home: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args(args)
        .env("OXPLOW_HOME", home)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn the_cli_pairs_a_device_lists_it_and_stops_a_headless_host() {
    let home = tempfile::tempdir().unwrap();
    let (ok, said) = cli(home.path(), &["pair"]);
    assert!(!ok && said.contains("no host runs"), "{said}");

    let mut host = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args(["serve"])
        .env("OXPLOW_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(host.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let base = line
        .trim()
        .strip_prefix("oxplow-host listening on ")
        .unwrap()
        .to_string();
    let socket = home.path().join("host.sock");
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "only this user");

    // A phone asks and shows its code; the owner types it here.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let asked: serde_json::Value = rt.block_on(async {
        reqwest::Client::new()
            .post(format!("{base}/host/pair"))
            .json(&serde_json::json!({ "name": "Phone", "kind": "phone" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let (ok, said) = cli(home.path(), &["pair"]);
    assert!(ok && said.contains("Phone"), "who waits: {said}");
    let (ok, said) = cli(home.path(), &["pair", "AAAA-AAAA"]);
    assert!(!ok && said.contains("no device is waiting"), "{said}");
    let (ok, said) = cli(home.path(), &["pair", asked["code"].as_str().unwrap()]);
    assert!(ok && said.contains("Phone"), "{said}");
    let id = said
        .split_whitespace()
        .find(|w| w.starts_with("dev-"))
        .unwrap()
        .to_string();
    let id = id.as_str();
    let (ok, said) = cli(home.path(), &["devices"]);
    assert!(ok && said.contains(id) && said.contains("Phone"), "{said}");

    let (ok, said) = cli(home.path(), &["stop"]);
    assert!(ok, "{said}");
    let started = Instant::now();
    while host.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the host stopped"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!socket.exists(), "the socket went with it");
}

/// The CLI's projects: in `host.db` when no host runs, through the host
/// (which holds `host.db`) when one does; the same list either way.
#[test]
fn the_cli_adds_lists_and_removes_projects_with_or_without_a_host() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let folder = |name: &str| {
        let dir = work.path().join(name);
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        dir.to_string_lossy().into_owned()
    };
    let id_of = |said: &str| said.split_whitespace().next().unwrap().to_string();

    let (ok, said) = cli(home.path(), &["project", "add", &folder("alpha")]);
    assert!(ok && said.contains("alpha"), "{said}");
    let alpha = id_of(&said);

    let mut host = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args(["serve"])
        .env("OXPLOW_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(host.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("listening on"), "{line}");

    let (ok, said) = cli(home.path(), &["project", "add", &folder("beta")]);
    assert!(ok && said.contains("beta"), "{said}");
    let beta = id_of(&said);
    let (ok, said) = cli(home.path(), &["project", "list"]);
    assert!(
        ok && said.contains(&alpha) && said.contains(&beta),
        "{said}"
    );
    let (ok, said) = cli(home.path(), &["project", "remove", &alpha]);
    assert!(ok && said.contains(&alpha), "{said}");
    let (ok, said) = cli(home.path(), &["project", "remove", &alpha]);
    assert!(!ok && said.contains("no project"), "{said}");

    host.kill().unwrap();
    host.wait().unwrap();
    let (ok, said) = cli(home.path(), &["project", "list"]);
    assert!(
        ok && !said.contains(&alpha) && said.contains(&beta),
        "{said}"
    );
}

/// "Allow other devices" from a terminal: saved in `host.db` with no host
/// running, in force when a host starts, and changed live through it.
#[test]
fn the_cli_sets_other_devices_and_a_running_host_applies_them_at_once() {
    let home = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (ok, said) = cli(
        home.path(),
        &[
            "settings",
            "set",
            "allow=on",
            "address=127.0.0.1",
            &format!("port={port}"),
        ],
    );
    assert!(ok, "{said}");
    let (ok, said) = cli(home.path(), &["settings", "set", "address="]);
    assert!(!ok && said.contains("address"), "refused: {said}");

    let mut host = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args(["serve"])
        .env("OXPLOW_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(host.stdout.take().unwrap());
    let mut lines = String::new();
    while !lines.contains("for other devices") {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "{lines}");
        lines.push_str(&line);
    }
    assert!(
        lines.contains(&format!("https://127.0.0.1:{port}")),
        "{lines}"
    );

    let (ok, said) = cli(home.path(), &["settings", "set", "allow=off"]);
    assert!(ok, "{said}");
    let (ok, status) = cli(home.path(), &["status"]);
    assert!(ok && status.contains("\"off\""), "off at once: {status}");
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is free again");
    let (ok, said) = cli(home.path(), &["settings"]);
    assert!(
        ok && said.contains("\"allowOtherDevices\": false"),
        "{said}"
    );

    host.kill().unwrap();
    host.wait().unwrap();
}
