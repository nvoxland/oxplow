//! The host's log: stderr and `<config dir>/logs/host.<date>.log`, daily, the
//! last week kept (the daemon's setup, `oxplow-daemon`'s `init_logging`).
//! Written synchronously, so a crash loses no line.

use std::path::Path;

pub fn init(config_dir: &Path) {
    use tracing_subscriber::prelude::*;
    // The appender reads the folder as it opens, so it must exist.
    let _ = std::fs::create_dir_all(config_dir.join("logs"));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("host")
        .filename_suffix("log")
        .max_log_files(7)
        .build(config_dir.join("logs"));
    let file_error = file.as_ref().err().map(ToString::to_string);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file.ok().map(|f| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(f)
        }))
        .init();
    if let Some(e) = file_error {
        tracing::warn!(error = %e, "no log file; logging to stderr only");
    }
}
