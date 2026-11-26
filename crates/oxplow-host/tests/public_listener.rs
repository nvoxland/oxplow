//! `serve --public-bind` listens a second time, for other devices, over
//! HTTPS when it has a certificate; `--bind` stays plain for this machine,
//! and only there is the owner's token good.

// Terse unwraps are the assertion style in test crates.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn testdata(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

#[test]
fn other_devices_reach_the_host_over_https_and_this_machine_over_http() {
    // A 401 over HTTPS shows the TLS listener answered; the owner's 200 on
    // loopback, that loopback stays plain.
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args([
            "serve",
            "--token-stdin",
            "--public-bind",
            "127.0.0.1:0",
            "--tls-cert",
        ])
        .arg(testdata("localhost-cert.pem"))
        .arg("--tls-key")
        .arg(testdata("localhost-key.pem"))
        .env("OXPLOW_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(child.stdin.as_mut().unwrap(), "tok").unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let local = lines.next().unwrap().unwrap();
    let public = lines.next().unwrap().unwrap();
    let local = local
        .strip_prefix("oxplow-host listening on ")
        .unwrap()
        .to_string();
    let public = public
        .strip_prefix("oxplow-host listening for other devices on ")
        .unwrap()
        .to_string();
    assert!(local.starts_with("http://"), "{local}");
    assert!(public.starts_with("https://"), "{public}");

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (secure, plain) = rt.block_on(async {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let get = |url: String| {
            let http = http.clone();
            async move {
                http.get(url)
                    .bearer_auth("tok")
                    .send()
                    .await
                    .map(|r| r.status().as_u16())
            }
        };
        (
            get(format!("{public}/host/projects")).await,
            get(format!("{local}/host/projects")).await,
        )
    });
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        secure.unwrap(),
        401,
        "the owner's token is for this machine only"
    );
    assert_eq!(plain.unwrap(), 200);
}

/// A certificate that cannot be read leaves the listener for other devices
/// out, never the host: this machine's windows still work, and the status
/// says why (the Devices window shows it).
#[test]
fn a_bad_certificate_leaves_other_devices_out_and_says_why() {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-host"))
        .args(["serve", "--token-stdin", "--public-bind", "127.0.0.1:0"])
        .args([
            "--tls-cert",
            "/nowhere/cert.pem",
            "--tls-key",
            "/nowhere/key.pem",
        ])
        .env("OXPLOW_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(child.stdin.as_mut().unwrap(), "tok").unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let local = lines.next().unwrap().unwrap();
    let local = local
        .strip_prefix("oxplow-host listening on ")
        .unwrap()
        .to_string();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let status: serde_json::Value = rt.block_on(async {
        reqwest::Client::new()
            .get(format!("{local}/host/status"))
            .bearer_auth("tok")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(status["public"]["state"], "failed", "{status}");
    assert!(
        status["public"]["error"]
            .as_str()
            .unwrap()
            .contains("cert.pem"),
        "{status}"
    );
}
