//! `oxplow-provider-fake`: the fake work-items provider on stdio. Hooks
//! come from `OXPLOW_FAKE_HOOKS` (see the library); a `crash` exits 3.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hooks = std::env::var("OXPLOW_FAKE_HOOKS").unwrap_or_default();
    let served = oxplow_provider_fake::serve(tokio::io::stdin(), tokio::io::stdout(), &hooks).await;
    if served == oxplow_provider_fake::Served::Crashed {
        std::process::exit(3);
    }
}
