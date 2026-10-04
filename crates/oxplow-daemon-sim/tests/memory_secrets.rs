//! The browser suite's daemon keeps its secrets in memory (tsk948): it
//! boots the project as the shipped daemon does and a person's approval
//! works, but the approval key goes with the process — nothing reached a
//! keychain, so a restarted daemon no longer trusts the approval.

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};

const TOKEN: &str = "suite-token";

struct Daemon {
    child: Child,
    base: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(project: &std::path::Path, home: &std::path::Path) -> Daemon {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-daemon-sim"))
        .args(["--project"])
        .arg(project)
        .args(["--bind", "127.0.0.1:0", "--token-stdin"])
        .env("OXPLOW_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{TOKEN}").unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let base = loop {
        let line = lines
            .next()
            .expect("the daemon says where it listens")
            .unwrap();
        if let Some(url) = line.strip_prefix("oxplow-daemon-sim listening on ") {
            break url.trim().to_string();
        }
    };
    Daemon { child, base }
}

async fn ipc(daemon: &Daemon, name: &str, args: Value) -> Value {
    let reply: Value = reqwest::Client::new()
        .post(format!("{}/ipc/{name}", daemon.base))
        .bearer_auth(TOKEN)
        .json(&args)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reply["status"], "ok", "{name}: {reply}");
    reply["data"].clone()
}

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test]
async fn an_approval_lives_and_dies_with_the_memory_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let (project, home) = (dir.path().join("project"), dir.path().join("home"));
    std::fs::create_dir_all(project.join(".oxplow")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // A project is a git repo with a commit.
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
    std::fs::write(
        project.join(".oxplow/project.yaml"),
        "acpAgents:\n  - { name: mine, command: tools/agent, args: [--acp] }\n",
    )
    .unwrap();

    let first = start(&project, &home);
    let programs = ipc(&first, "list_project_programs", json!({})).await;
    let mine = programs
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "mine")
        .cloned()
        .expect("the project's ACP agent is a program");
    assert_eq!(mine["approved"], false);
    let approved = ipc(
        &first,
        "approve_project_program",
        json!({ "kind": mine["kind"], "name": "mine", "version": mine["version"] }),
    )
    .await;
    assert!(approved
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "mine" && p["approved"] == true));
    assert!(
        files_under(&home)
            .iter()
            .any(|f| f.to_string_lossy().contains("approvals")),
        "the approval is kept under OXPLOW_HOME, not the person's config: {:?}",
        files_under(&home)
    );
    drop(first);

    // A new process, a new key: the recorded approval no longer verifies.
    let second = start(&project, &home);
    let programs = ipc(&second, "list_project_programs", json!({})).await;
    assert!(programs
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "mine" && p["approved"] == false));
}
