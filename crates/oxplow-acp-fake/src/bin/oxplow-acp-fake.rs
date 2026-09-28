//! The fake ACP agent on stdio. `OXPLOW_ACP_FAKE_STATE=<file>` keeps the
//! session history across runs (for `session/load`);
//! `OXPLOW_ACP_FAKE_NO_LOAD=1` / `OXPLOW_ACP_FAKE_NO_MCP_HTTP=1` turn
//! those capabilities off. Exits 1 after a `crash` step.

use std::sync::{Arc, Mutex};

use oxplow_acp_fake::{serve, Ended, FakeOptions, FakeState};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let state_path = std::env::var_os("OXPLOW_ACP_FAKE_STATE").map(std::path::PathBuf::from);
    let state = state_path
        .as_deref()
        .map(FakeState::load)
        .unwrap_or_default();
    let shared = Arc::new(Mutex::new(state));
    let opts = FakeOptions {
        load_session: std::env::var_os("OXPLOW_ACP_FAKE_NO_LOAD").is_none(),
        mcp_http: std::env::var_os("OXPLOW_ACP_FAKE_NO_MCP_HTTP").is_none(),
        state_file: state_path,
    };
    let ended = serve(tokio::io::stdin(), tokio::io::stdout(), shared, opts).await;
    match ended {
        Ok(Ended::Eof) => {}
        Ok(Ended::Crashed) => std::process::exit(1),
        Err(err) => {
            eprintln!("oxplow-acp-fake: {err}");
            std::process::exit(2);
        }
    }
}
