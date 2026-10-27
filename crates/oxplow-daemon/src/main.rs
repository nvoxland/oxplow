//! `oxplow-daemon --project <dir> [--bind 127.0.0.1:7420] [--init]`
//! `oxplow-daemon extension <new|check|test> …`
//!
//! Headless backend entrypoint. See lib.rs for the HTTP surface and
//! the crate docs for the SSH-tunnel deployment model. Its secrets are
//! the OS keychain's, always.
//!
//! `extension …` is the extension SDK's CLI, as `oxplow extension …` has
//! it: the daemon launches the agent sessions, so it is the binary their
//! `$OXPLOW_BIN` names, on a remote host too. It runs before any runtime
//! starts — the SDK runs its own.

use std::sync::Arc;

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("extension") {
        std::process::exit(oxplow_sdk::cli::run(&args.collect::<Vec<_>>()));
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(oxplow_daemon::run_main(
            "oxplow-daemon",
            Arc::new(oxplow_ai::secrets::KeychainSecrets),
        ));
}
