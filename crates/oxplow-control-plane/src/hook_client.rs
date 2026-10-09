//! `<oxplow> hook <event>`: what a harness's command hooks run (Codex's).
//! Its stdin — the hook's body — is posted to the hook route
//! (`OXPLOW_HOOK_BASE_URL/<event>`) with the session's bearer
//! (`OXPLOW_HOOK_TOKEN`), both from the agent's env; the route's answer
//! goes to its stdout, in the harness's shape. Both oxplow binaries run
//! it — the desktop shell and the daemon, whose executable is the one a
//! launch names (`LaunchInput::oxplow_executable`).

use std::io::{Read as _, Write as _};

/// The event `args` (after the program's name) name as `hook <event>`.
pub fn event_arg(mut args: impl Iterator<Item = String>) -> Option<String> {
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("hook"), Some(event), None) => Some(event),
        _ => None,
    }
}

/// Forward the hook on stdin for `event` and print the answer. A failure
/// is reported on stderr; the hook still exits cleanly, so the agent
/// isn't blocked by oxplow being away.
pub async fn run(event: &str) {
    let mut payload = Vec::new();
    if let Err(err) = std::io::stdin().read_to_end(&mut payload) {
        eprintln!("oxplow hook failed to read stdin: {err}");
        return;
    }
    if payload.is_empty() {
        payload.extend_from_slice(b"{}");
    }
    let base_url = std::env::var("OXPLOW_HOOK_BASE_URL").unwrap_or_default();
    let token = std::env::var("OXPLOW_HOOK_TOKEN").unwrap_or_default();
    match forward(&base_url, &token, event, payload).await {
        Ok(body) if !body.is_empty() => {
            if let Err(err) = std::io::stdout().write_all(&body) {
                eprintln!("oxplow hook failed to write response: {err}");
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("oxplow hook forwarding failed: {err}"),
    }
}

/// [`run`] for a caller with no async runtime of its own.
pub fn run_blocking(event: &str) {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run(event)),
        Err(err) => eprintln!("oxplow hook failed to start runtime: {err}"),
    }
}

/// Post `payload` to `<base_url>/<event>` as the bearer `token`; the
/// answer's body.
pub async fn forward(
    base_url: &str,
    token: &str,
    event: &str,
    payload: Vec<u8>,
) -> Result<Vec<u8>, reqwest::Error> {
    let url = format!("{}/{}", base_url.trim_end_matches('/'), event);
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await?;
    Ok(response.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_and_an_event_is_the_hook_command() {
        let args = |a: &[&str]| {
            a.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert_eq!(event_arg(args(&["hook", "Stop"])), Some("Stop".into()));
        assert_eq!(event_arg(args(&["hook"])), None);
        assert_eq!(event_arg(args(&["hook", "Stop", "extra"])), None);
        assert_eq!(event_arg(args(&["--project", "/p"])), None);
    }
}
