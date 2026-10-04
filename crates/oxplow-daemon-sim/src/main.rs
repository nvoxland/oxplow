//! `oxplow-daemon-sim --project <dir> [--bind …] [--init] [--token-stdin]`
//!
//! The daemon for oxplow's browser suite (P11, tsk948): the same server,
//! boot and arguments as `oxplow-daemon`, with its secrets — keys, tokens,
//! the approval key — in memory, so a headless runner needs no keychain
//! and a local run writes no test key into the person's. They go with the
//! process. Development only; isolate its global config with
//! `OXPLOW_HOME`.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    eprintln!("oxplow-daemon-sim: secrets are kept in memory — for tests only");
    oxplow_daemon::run_main(
        "oxplow-daemon-sim",
        Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
    )
    .await;
}
