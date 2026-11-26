//! A headless host prints its owner token once and keeps it nowhere: the CLI
//! reaches it over the owner socket (`host.sock`, mode 0600) to make a
//! pairing code, list devices and stop it.

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

    let (ok, said) = cli(home.path(), &["pair"]);
    assert!(ok, "{said}");
    let code = said
        .split_whitespace()
        .find(|w| w.len() == 9 && w.as_bytes()[4] == b'-')
        .unwrap()
        .to_string();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let paired: serde_json::Value = rt.block_on(async {
        reqwest::Client::new()
            .post(format!("{base}/host/pair"))
            .json(&serde_json::json!({ "code": code, "name": "Phone", "kind": "phone" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let id = paired["device"]["id"].as_str().unwrap();
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
