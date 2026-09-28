//! Live smoke test against a real ACP adapter (tsk340). Ignored by default:
//! it needs the adapter installed and its login. Run with
//!
//! ```sh
//! OXPLOW_ACP_LIVE_CMD="bunx @zed-industries/claude-code-acp" \
//!   cargo test -p oxplow-app --test acp_live -- --ignored
//! ```
//!
//! It opens a session in a temp project, sends one prompt, and expects a
//! reply and a clean turn end.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use oxplow_app::acp::host::ServicesAcpHost;
use oxplow_app::acp::manager::Launch;
use oxplow_app::acp::session::{AcpEventBody, AcpStatus, SessionSpec};
use oxplow_app::acp::transcript::ItemBody;
use oxplow_app::Services;

#[tokio::test]
#[ignore = "needs a real ACP adapter: set OXPLOW_ACP_LIVE_CMD"]
async fn a_real_adapter_answers_a_prompt() {
    let Ok(cmd) = std::env::var("OXPLOW_ACP_LIVE_CMD") else {
        panic!("set OXPLOW_ACP_LIVE_CMD to the adapter command");
    };
    let mut words = cmd.split_whitespace();
    let bin = words.next().unwrap();
    let program = oxplow_app::agent_path::resolve_program(bin)
        .unwrap_or_else(|| panic!("{bin} is not installed"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let svc = Arc::new(Services::in_memory(&root).unwrap());
    let stream = svc.streams.ensure_primary().await.unwrap();
    let thread = svc
        .threads
        .create_with_acp(
            &stream.id,
            "live",
            "working",
            oxplow_domain::AgentKind::Acp,
            Some("live".into()),
        )
        .await
        .unwrap()
        .id;

    let host = Arc::new(ServicesAcpHost::new(&svc, Some(stream.id)));
    svc.acp
        .open(
            host,
            SessionSpec {
                thread_id: thread,
                agent: "live".into(),
                cwd: root.clone(),
                mcp: vec![],
                resume_session_id: None,
                system_prompt: None,
                system_prompt_via_meta: false,
            },
            Launch {
                program: program.into(),
                args: words.map(str::to_string).collect(),
                env: vec![],
            },
        )
        .await
        .unwrap();
    let mut rx = svc.acp.subscribe();
    svc.acp
        .submit_human_prompt(&thread, "Reply with the single word: pong".into())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            if let AcpEventBody::Status {
                status: AcpStatus::Idle,
            } = rx.recv().await.unwrap().body
            {
                return;
            }
        }
    })
    .await
    .expect("the turn ended");
    let items = svc.acp.transcript(&thread, 0).unwrap().items;
    assert!(
        items
            .iter()
            .any(|i| matches!(&i.body, ItemBody::Agent { text } if !text.trim().is_empty())),
        "no reply: {items:?}"
    );
    svc.acp.close(&thread).unwrap();
}
