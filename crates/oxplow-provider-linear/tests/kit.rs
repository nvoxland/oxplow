//! P7.A5: the example extension (`examples/extensions/linear`) passes
//! `oxplow plugin test` — its handshake, check, `create` example, the
//! read-back and the work-items conformance suite through a throwaway
//! host — with the built provider against the simulator.
//! `OXPLOW_BLESS=1` rewrites its golden transcript.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use oxplow_provider_linear::sim::LinearSim;
use oxplow_sdk::plugin_test::test_extension;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_example_extension_passes_plugin_test() {
    let sim = LinearSim::start("lin_api_kit").await.unwrap();
    // `plugin test` takes credentials and declared env from its own
    // environment, as a person running it would.
    std::env::set_var("LINEAR_API_KEY", "lin_api_kit");
    std::env::set_var("LINEAR_API_URL", &sim.url);
    let dir = tempfile::tempdir().unwrap();
    oxplow_app::vcs::GitProvider
        .init_repository(dir.path())
        .await
        .unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/linear");
    let ext = dir.path().join("oxplow/extensions/linear");
    copy_dir(&example, &ext);
    let entry = ext.join("bin/oxplow-provider-linear");
    std::fs::write(
        &entry,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_oxplow-provider-linear")
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The simulator listens on localhost: let the provider's egress reach it.
    let manifest = ext.join("extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "network: [api.linear.app]",
            "network: [api.linear.app, localhost]",
        ),
    )
    .unwrap();

    let bless = std::env::var_os("OXPLOW_BLESS").is_some();
    let report = test_extension(dir.path(), "linear", bless).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
    if bless {
        std::fs::create_dir_all(example.join("fixtures/transcripts")).unwrap();
        std::fs::copy(
            ext.join("fixtures/transcripts/linear.jsonl"),
            example.join("fixtures/transcripts/linear.jsonl"),
        )
        .unwrap();
    }
    for ran in ["work_items suite", "read issues", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
}
