//! The observe conformance suite, live over the fake harness: its launch
//! runs its binary, which posts a scripted session to this control plane's
//! hook route and OTLP receiver, and `oxplow_app::observe_conformance`
//! checks what core recorded. The fake renders its own answer shape and
//! fails on any other, so a pass also shows every answer was the harness's
//! to render (`.context/agent-model.md` "Observe conformance").

#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]
#![allow(clippy::unwrap_used)]

mod common;

use std::process::Command as Proc;
use std::sync::Arc;

use common::boot;

use oxplow_app::exec_consent;
use oxplow_app::observe_conformance::{suite, Expect};
use oxplow_domain::agent::harness::{Endpoints, LaunchInput, LaunchSpec, SessionIds};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent_session::NewAgentSession;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};
use oxplow_harness_fake::{FakeHarness, BIN, EDITED, TOKENS};

/// The fake's binary, built on demand into this test's target dir.
fn fake_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    let bin = dir.join(BIN);
    if !bin.exists() {
        let ok = Proc::new(env!("CARGO"))
            .args(["build", "-q", "-p", "oxplow-harness-fake", "--bin", BIN])
            .env("CARGO_TARGET_DIR", dir.parent().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok, "building {BIN} failed");
    }
    bin
}

/// The fake provider's binary, built on demand into this test's target
/// dir: run in harness mode, it answers the fake harness's verbs.
fn provider_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    let bin = dir.join("oxplow-provider-fake");
    if !bin.exists() {
        let ok = Proc::new(env!("CARGO"))
            .args(["build", "-q", "-p", "oxplow-provider-fake"])
            .env("CARGO_TARGET_DIR", dir.parent().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok, "building oxplow-provider-fake failed");
    }
    bin
}

/// A session of `harness` (a registered harness's key) on a fresh thread:
/// launched, its process run to the end, and the suite over what core
/// recorded.
async fn a_session_of(
    cp: &oxplow_control_plane::ControlPlane,
    svc: &oxplow_app::Services,
    root: &std::path::Path,
    harness_id: &str,
) {
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
        worktree_path: root.to_string_lossy().into(),
        working_pane: String::new(),
        talking_pane: String::new(),
        working_session_id: String::new(),
        talking_session_id: String::new(),
        host: oxplow_domain::HostId::LOCAL,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    svc.stream_store.upsert(&stream).await.unwrap();
    let thread = Thread {
        id: ThreadId::new(1),
        stream_id: stream.id,
        title: "t".into(),
        status: ThreadStatus::Active,
        sort_index: 0,
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    svc.thread_store.upsert(&thread).await.unwrap();

    let session = svc
        .agent_session_store
        .open(&NewAgentSession::terminal(thread.id, harness_id))
        .await
        .unwrap();
    let harness = svc.harnesses.get(harness_id).unwrap();
    // The fake's binary is found where cargo built it.
    let bin_dir = fake_bin().parent().unwrap().to_path_buf();
    let token = common::bearer_for(svc, session.id).await;
    let identity = vec![
        ("OXPLOW_HOOK_TOKEN".to_string(), token.clone()),
        ("OXPLOW_HOOK_BASE_URL".to_string(), cp.hook_base_url()),
        ("OXPLOW_STREAM_ID".to_string(), stream.id.to_string()),
        ("OXPLOW_THREAD_ID".to_string(), thread.id.to_string()),
        ("OXPLOW_SESSION".to_string(), session.id.to_string()),
    ];
    let launch = harness
        .launch(&LaunchInput {
            session: SessionIds {
                stream: stream.id,
                thread: thread.id,
                session: session.id,
            },
            workspace: root.to_path_buf(),
            project_dir: root.to_path_buf(),
            endpoints: Endpoints {
                hook_base_url: cp.hook_base_url(),
                mcp_endpoint_url: cp.mcp_endpoint_url(),
                otlp_base_url: cp.otlp_base_url(),
                hook_token: token.clone(),
            },
            identity_env: identity,
            system_prompt: None,
            resume: None,
            text: AgentText::default(),
            config: serde_json::json!({}),
            oxplow_executable: "/bin/false".into(),
            home: None,
            search_path: vec![bin_dir],
        })
        .await
        .unwrap();
    let LaunchSpec::Pty { command, env } = launch.spec else {
        panic!("the fake runs in a terminal");
    };
    // Its identity rides the env, as a PTY spawn sets it; none of it is in
    // the command.
    assert!(!command.contains(&token), "{command}");
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .envs(env)
            .output(),
    )
    .await
    .expect("the fake's session ends")
    .unwrap();
    assert!(
        out.status.success(),
        "the fake failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let findings = suite(
        svc,
        &Expect {
            harness: harness_id,
            thread: thread.id,
            edited: EDITED,
            tokens: (TOKENS.0 as u64, TOKENS.1 as u64),
        },
    )
    .await;
    assert!(findings.is_empty(), "{findings:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fake_harness_session_is_recorded_canonically() {
    let (cp, svc, root, _dir) = boot().await;
    // The fake registered as any harness is.
    svc.harnesses.register(Arc::new(FakeHarness));
    a_session_of(&cp, &svc, &root, oxplow_harness_fake::ID).await;
}

/// The same session, its harness a provider's process: the fake provider
/// in harness mode, approved and enabled in the project as a person would,
/// registered under its instance id. Its launch, every tool hook's mapping
/// and every answer's rendering go through that process — and core records
/// it canonically.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_launched_session_is_recorded_canonically() {
    let (cp, svc, root, _dir) = boot().await;
    let ext_dir = root.join("oxplow/extensions/relay");
    std::fs::create_dir_all(ext_dir.join("bin")).unwrap();
    std::fs::write(
        ext_dir.join("extension.yaml"),
        "manifest: 2\nname: relay\nsharing: private\nintent:\n  purpose: a harness over the protocol\n  examples: [{ name: a }]\nproviders:\n  - id: relay\n    capability: agent_harness\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = ext_dir.join("bin/provider");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=agent_harness exec '{}' \"$@\"\n",
            provider_bin().display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        ext_dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::harness_declarations()).unwrap(),
    )
    .unwrap();

    let extensions = oxplow_app::extensions::load_extensions(&root);
    let ext = extensions
        .iter()
        .find(|e| e.name == "relay")
        .cloned()
        .expect("the harness extension loads");
    assert_eq!(ext.errors, Vec::<String>::new());
    let config = oxplow_app::config_service::read_config(&svc.config);
    let program = exec_consent::list(&svc.approvals, &root, &config, &extensions)
        .into_iter()
        .find(|p| p.kind == exec_consent::ProgramKind::Provider)
        .expect("the provider is a program");
    exec_consent::approve_program(
        &svc.approvals,
        &root,
        &config,
        &extensions,
        program.kind,
        &program.name,
        program.version.as_deref().unwrap(),
    )
    .unwrap();
    svc.providers
        .enable(
            &ext,
            &ext.providers[0],
            serde_json::json!({ "team": "core" }),
        )
        .await
        .unwrap();
    assert!(
        svc.harnesses.get(oxplow_harness_fake::ID).is_err(),
        "only the provider's instance stands for the fake"
    );

    a_session_of(&cp, &svc, &root, "relay").await;
    svc.providers.stop("relay/relay").await;
}
