//! `oxplow-provider-mcp`: oxplow's MCP adapter on stdio (see the library).
//! The host runs it in the extension folder with its manifest's
//! `adapter:` as arguments; `OXPLOW_PROVIDER_ID` names the provider.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::env::current_dir().unwrap_or_default();
    let id = std::env::var("OXPLOW_PROVIDER_ID").unwrap_or_default();
    let adapter = match oxplow_provider_mcp::Adapter::from_args(&dir, &args, &id) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxplow-provider-mcp: {e}");
            std::process::exit(2);
        }
    };
    oxplow_provider_mcp::serve(tokio::io::stdin(), tokio::io::stdout(), adapter).await;
    std::process::exit(0);
}
