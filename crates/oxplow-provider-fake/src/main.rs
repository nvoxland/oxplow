//! `oxplow-provider-fake`: the fake work-items provider on stdio. Hooks
//! come from `OXPLOW_FAKE_HOOKS` (see the library). It exits when serving
//! ends — at once, since the runtime would otherwise wait on its blocked
//! stdin reader — with status 3 after a `crash`.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hooks = std::env::var("OXPLOW_FAKE_HOOKS").unwrap_or_default();
    let served = oxplow_provider_fake::serve(tokio::io::stdin(), tokio::io::stdout(), &hooks).await;
    std::process::exit(match served {
        oxplow_provider_fake::Served::Ended => 0,
        oxplow_provider_fake::Served::Crashed => 3,
    });
}
