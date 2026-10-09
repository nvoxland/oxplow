//! `oxplow-provider-fake`: the fake work-items provider on stdio. Hooks
//! come from `OXPLOW_FAKE_HOOKS` (see the library), and the instance it
//! is from `OXPLOW_PROVIDER_ID` (`fake` when the host doesn't say), the
//! capability it implements from `OXPLOW_FAKE_CAPABILITY` (`work_items`,
//! the default, `effort_policy`, `agent_harness`, `snapshots` or `knowledge`; a
//! snapshots provider's features, `contents`, from `OXPLOW_FAKE_FEATURES`). It exits when serving
//! ends — at once, since the runtime would otherwise wait on its blocked
//! stdin reader — with status 3 after a `crash`.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hooks = std::env::var("OXPLOW_FAKE_HOOKS").unwrap_or_default();
    let id = std::env::var("OXPLOW_PROVIDER_ID").unwrap_or_else(|_| "fake".into());
    let state = std::env::var_os("OXPLOW_FAKE_STATE").map(std::path::PathBuf::from);
    let capability = match oxplow_provider_fake::Capability::named(
        std::env::var("OXPLOW_FAKE_CAPABILITY").ok().as_deref(),
    )
    .and_then(|c| c.with_features(std::env::var("OXPLOW_FAKE_FEATURES").ok().as_deref()))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fake: {e}");
            std::process::exit(2);
        }
    };
    let served = oxplow_provider_fake::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        &hooks,
        &id,
        state,
        capability,
    )
    .await;
    std::process::exit(match served {
        oxplow_provider_fake::Served::Ended => 0,
        oxplow_provider_fake::Served::Crashed => 3,
    });
}
