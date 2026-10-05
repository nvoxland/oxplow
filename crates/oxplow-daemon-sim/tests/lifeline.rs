//! A daemon started with `--token-stdin` keeps its stdin as a lifeline to
//! the app that started it (tsk1073): when the app goes — quit, crash or
//! SIGKILL — the pipe closes and the daemon stops, instead of running on
//! with its agents until the project is next opened.

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn a_daemon_stops_when_its_app_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (project, home) = (dir.path().join("project"), dir.path().join("home"));
    std::fs::create_dir_all(project.join(".oxplow")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    for args in [
        &["init", "-q"][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&project)
            .status()
            .unwrap()
            .success());
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-daemon-sim"))
        .arg("--project")
        .arg(&project)
        .args(["--bind", "127.0.0.1:0", "--token-stdin"])
        .env("OXPLOW_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "lifeline-token").unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    while !lines
        .next()
        .expect("the daemon says where it listens")
        .unwrap()
        .starts_with("oxplow-daemon-sim listening on ")
    {}
    // Serving, with the app still there.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "it serves while the app lives"
    );

    // The app goes: its end of the pipe closes.
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(15);
    let exited = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(exited, "the daemon outlived its app");
}
