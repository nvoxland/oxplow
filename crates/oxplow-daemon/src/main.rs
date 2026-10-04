//! `oxplow-daemon --project <dir> [--bind 127.0.0.1:7420] [--init]`
//!
//! Headless backend entrypoint. See lib.rs for the HTTP surface and
//! the crate docs for the SSH-tunnel deployment model. Its secrets are
//! the OS keychain's, always.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    oxplow_daemon::run_main(
        "oxplow-daemon",
        Arc::new(oxplow_ai::secrets::KeychainSecrets),
    )
    .await;
}
