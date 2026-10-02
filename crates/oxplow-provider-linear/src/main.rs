//! `oxplow-provider-linear`: the Linear work-items provider on stdio.
//! `LINEAR_API_KEY` is its key (the instance's credential),
//! `LINEAR_API_URL` overrides Linear's endpoint. It exits when serving
//! ends — at once, since the runtime would otherwise wait on its blocked
//! stdin reader.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    oxplow_provider_linear::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        oxplow_provider_linear::Env::from_process(),
    )
    .await;
    std::process::exit(0);
}
