//! P7.A5: the example extension (`examples/extensions/linear`) passes
//! `oxplow plugin test` — its handshake, check, `create` example, the
//! read-back and the work-items conformance suite through a throwaway
//! host — with the built provider against the simulator.
//! `OXPLOW_BLESS=1` rewrites its golden transcript.

#![allow(clippy::unwrap_used)]

mod common;

use oxplow_provider_linear::sim::LinearSim;
use oxplow_sdk::plugin_test::test_extension_in;

#[tokio::test(flavor = "multi_thread")]
async fn the_example_extension_passes_plugin_test() {
    let sim = LinearSim::start("lin_api_kit").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ext = common::install_example(dir.path()).await;
    let example = common::example_dir();
    // The simulator listens on localhost: let the provider's egress reach it.
    common::rewrite(
        &ext,
        "extension.yaml",
        "network: [api.linear.app]",
        "network: [api.linear.app, localhost]",
    );

    let bless = std::env::var_os("OXPLOW_BLESS").is_some();
    let env = common::linear_env("lin_api_kit", Some(&sim.url));
    let report = test_extension_in(dir.path(), "linear", bless, env)
        .await
        .unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
    if bless {
        std::fs::create_dir_all(example.join("fixtures/transcripts")).unwrap();
        std::fs::copy(
            ext.join("fixtures/transcripts/linear.jsonl"),
            example.join("fixtures/transcripts/linear.jsonl"),
        )
        .unwrap();
    }
    // P9.A1: the example replaces the Board with its own lens, checked
    // above with the rest of the extension.
    let loaded = oxplow_app::extensions::load_extensions(dir.path())
        .into_iter()
        .find(|e| e.name == "linear")
        .unwrap();
    let replaced: Vec<(&str, &str)> = loaded
        .ui
        .replacements
        .iter()
        .map(|r| (r.target.as_str(), r.lens_id.as_str()))
        .collect();
    assert_eq!(
        replaced,
        vec![("work_item.board", "linear/board")],
        "{:?}",
        loaded.errors
    );
    for ran in ["work_items suite", "read issues", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
}
