//! The app's host is a child of the app: `serve --for-app` ends when its
//! stdin closes (the app quit, crashed or was killed). A headless host does
//! not: it runs until it is stopped.

// Terse unwraps are the assertion style in test crates.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn serve(home: &std::path::Path, for_app: bool) -> (Child, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxplow-host"));
    cmd.args(["serve", "--token-stdin"])
        .env("OXPLOW_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if for_app {
        cmd.arg("--for-app");
    }
    let mut child = cmd.spawn().unwrap();
    writeln!(child.stdin.as_mut().unwrap(), "tok").unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("listening on"), "{line}");
    (child, line)
}

fn ended_within(child: &mut Child, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn the_apps_host_ends_when_the_app_does() {
    let home = tempfile::tempdir().unwrap();
    let (mut child, _) = serve(home.path(), true);
    assert!(
        matches!(
            oxplow_host::db::HostDb::open(home.path()),
            Err(oxplow_host::db::DbError::Held(_))
        ),
        "the running host holds the home"
    );
    drop(child.stdin.take());
    assert!(
        ended_within(&mut child, Duration::from_secs(10)),
        "the host ended"
    );
    oxplow_host::db::HostDb::open(home.path()).expect("and let go of the home");
}

#[test]
fn a_headless_host_runs_on_when_its_stdin_closes() {
    let home = tempfile::tempdir().unwrap();
    let (mut child, _) = serve(home.path(), false);
    drop(child.stdin.take());
    assert!(
        !ended_within(&mut child, Duration::from_secs(1)),
        "still running"
    );
    child.kill().unwrap();
    child.wait().unwrap();
}

fn own_host(home: &std::path::Path) -> oxplow_host::client::OwnHost {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxplow-host"));
    cmd.env("OXPLOW_HOME", home);
    oxplow_host::client::start_own_host(cmd, &[]).unwrap()
}

fn answers(base: &str, token: &str) -> bool {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        reqwest::Client::new()
            .get(format!("{base}/host/projects"))
            .bearer_auth(token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    })
}

/// The app starts its own host and reaches it with this run's token; the
/// next start, right after a quit, waits for the old host to let go.
#[test]
fn the_app_starts_its_own_host_and_the_next_start_follows_a_quit() {
    let home = tempfile::tempdir().unwrap();
    let first = own_host(home.path());
    let base = first.client.base().to_string();
    assert!(answers(&base, first.client.token()));
    drop(first);
    let second = own_host(home.path());
    assert!(answers(second.client.base(), second.client.token()));
    assert!(!answers(&base, second.client.token()), "the first is gone");
}

/// A headless host has this home: the app's host cannot start, and says why.
#[test]
fn the_app_does_not_start_a_host_where_a_headless_one_runs() {
    let home = tempfile::tempdir().unwrap();
    let (mut headless, _) = serve(home.path(), false);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxplow-host"));
    cmd.env("OXPLOW_HOME", home.path());
    let started = Instant::now();
    let err = oxplow_host::client::start_own_host(cmd, &[])
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("a headless host"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a headless host is not waited for"
    );
    headless.kill().unwrap();
    headless.wait().unwrap();
}
