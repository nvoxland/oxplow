//! `oxplow-provider-mcp-notes`: the notes MCP server (see
//! [`oxplow_provider_mcp::notes`]) — over stdio, or with `--http
//! <addr:port>` over streamable HTTP at `/mcp` (its address goes to
//! stderr), behind the bearer token in `$NOTES_BEARER` when that is set.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let served = match args.iter().position(|a| a == "--http") {
        None => oxplow_provider_mcp::notes::serve_stdio().await,
        Some(at) => {
            let addr = args.get(at + 1).map_or("127.0.0.1:0", String::as_str);
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    if let Ok(local) = listener.local_addr() {
                        eprintln!("notes: http://{local}/mcp");
                    }
                    let tokens = std::env::var("NOTES_BEARER")
                        .ok()
                        .map(|t| oxplow_provider_mcp::notes::only(&t));
                    oxplow_provider_mcp::notes::serve_http(
                        listener,
                        tokens,
                        oxplow_provider_mcp::notes::Refusal::Challenge,
                    )
                    .await
                }
                Err(e) => Err(e),
            }
        }
    };
    if let Err(e) = served {
        eprintln!("oxplow-provider-mcp-notes: {e}");
        std::process::exit(1);
    }
}
