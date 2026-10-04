//! The browser suite's test extension declares the fake provider with a
//! checked-in `provider.json` (`tests-e2e/fixtures/extension/`). It must be
//! what this fake declares, or the suite approves and runs a provider whose
//! declarations lie. `OXPLOW_BLESS=1` rewrites it.

use oxplow_provider_protocol::model::InitializeResult;

#[test]
fn the_suite_fixture_declares_what_the_fake_does() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests-e2e/fixtures/extension/provider.json"
    );
    if std::env::var_os("OXPLOW_BLESS").is_some() {
        let json = serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap();
        std::fs::write(path, json + "\n").unwrap();
    }
    // Compared as declarations, not text: key order isn't the contract.
    let on_disk: InitializeResult =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(on_disk, oxplow_provider_fake::declarations());
}
