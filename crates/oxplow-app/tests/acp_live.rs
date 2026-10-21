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

use oxplow_app::chat::acp::AcpOpen;
use oxplow_app::chat::host::ServicesChatHost;
use oxplow_app::chat::manager::Launch;
use oxplow_app::chat::session::{ChatEventBody, ChatStatus, SessionSpec};
use oxplow_app::chat::transcript::ItemBody;
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
    let svc = Services::in_memory(&root).await.unwrap();
    let stream = svc.streams.ensure_primary().await.unwrap();
    let created = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            oxplow_app::commands::thread::CREATE,
            serde_json::json!({
                "stream": oxplow_domain::uri::build::stream_uri(stream.id),
                "title": "live",
            }),
            false,
        )
        .await
        .unwrap();
    let thread: oxplow_domain::ThreadId =
        serde_json::from_value(created.result["id"].clone()).unwrap();

    let session = svc
        .agent_session_store
        .newest_for_thread(thread)
        .await
        .unwrap()
        .expect("the thread has its session")
        .id;
    let host = Arc::new(ServicesChatHost::new(&svc, Some(stream.id), session));
    svc.chat
        .open(
            host,
            SessionSpec {
                session_id: session,
                thread_id: thread,
                agent: "live".into(),
                cwd: root.clone(),
            },
            Launch {
                program: program.into(),
                args: words.map(str::to_string).collect(),
                env: vec![],
                env_remove: vec![],
                open: AcpOpen::default(),
            },
        )
        .await
        .unwrap();
    let mut rx = svc.chat.subscribe();
    svc.chat
        .submit_human_prompt(&session, "Reply with the single word: pong".into())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            if let ChatEventBody::Status {
                status: ChatStatus::Idle,
            } = rx.recv().await.unwrap().body
            {
                return;
            }
        }
    })
    .await
    .expect("the turn ended");
    let items = svc.chat.transcript(&session, 0).unwrap().items;
    assert!(
        items
            .iter()
            .any(|i| matches!(&i.body, ItemBody::Agent { text } if !text.trim().is_empty())),
        "no reply: {items:?}"
    );
    svc.chat.close(&session).unwrap();
}
