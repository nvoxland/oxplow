//! `oxplow-oauth-sim --http <addr:port>`: the stand-in authorization
//! server (see [`oxplow_oauth_sim`]) on `addr` (a free loopback port when
//! omitted). Its endpoints go to stdout; it serves until stopped.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let addr = match args.iter().position(|a| a == "--http") {
        Some(at) => args.get(at + 1).map_or("127.0.0.1:0", String::as_str),
        None if args.is_empty() => "127.0.0.1:0",
        None => {
            eprintln!("usage: oxplow-oauth-sim [--http <addr:port>]");
            std::process::exit(2);
        }
    };
    let sim = match oxplow_oauth_sim::OAuthSim::bind(addr).await {
        Ok(sim) => sim,
        Err(e) => {
            eprintln!("oxplow-oauth-sim: {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("authorize_url: {}", sim.authorize_url);
    println!("token_url:     {}", sim.token_url);
    println!("mcp url:       {}", sim.mcp_url);
    println!("expire tokens: POST {}", sim.expire_url);
    println!("revoke grant:  POST {}", sim.revoke_url);
    sim.served().await;
}
