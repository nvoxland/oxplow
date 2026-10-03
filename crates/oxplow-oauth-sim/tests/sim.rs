//! The stand-in signs in, and its notes server at `/mcp` takes exactly the
//! access tokens it issued and still holds live.

#![allow(clippy::unwrap_used)]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use oxplow_oauth_sim::OAuthSim;
use sha2::{Digest, Sha256};

const VERIFIER: &str = "a-verifier-long-enough-to-be-one-of-pkces-0123456789";
const REDIRECT: &str = "http://127.0.0.1:9/callback";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// A token request's answer.
async fn token(sim: &OAuthSim, form: String) -> serde_json::Value {
    let answer = client()
        .post(&sim.token_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 200);
    serde_json::from_str(&answer.text().await.unwrap()).unwrap()
}

/// Sign in: authorize (PKCE), take the code from the redirect, exchange it.
async fn sign_in(sim: &OAuthSim) -> serde_json::Value {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let mut authorize = url::Url::parse(&sim.authorize_url).unwrap();
    authorize
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", "oxplow-test")
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("state", "st-1")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let redirected = client().get(authorize).send().await.unwrap();
    assert_eq!(redirected.status(), 302);
    let location = url::Url::parse(redirected.headers()["location"].to_str().unwrap()).unwrap();
    let code = location
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("client_id", "oxplow-test")
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("code_verifier", VERIFIER)
        .finish();
    token(sim, form).await
}

/// An MCP `initialize` at `/mcp` under `bearer`: its HTTP status.
async fn initialize(sim: &OAuthSim, bearer: &str) -> u16 {
    client()
        .post(&sim.mcp_url)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "sim-test", "version": "0" }
                }
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_access_token_is_refused_at_mcp() {
    let sim = OAuthSim::start().await;
    let tokens = sign_in(&sim).await;
    let access = tokens["access_token"].as_str().unwrap();
    assert_eq!(initialize(&sim, access).await, 200);
    assert_eq!(initialize(&sim, "never-issued").await, 401);

    // Expired over HTTP, as the in-app walk does it.
    let expired = client().post(&sim.expire_url).send().await.unwrap();
    assert!(expired.status().is_success(), "{}", expired.status());
    assert_eq!(initialize(&sim, access).await, 401);

    // A refresh issues a live one.
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", tokens["refresh_token"].as_str().unwrap())
        .append_pair("client_id", "oxplow-test")
        .finish();
    let renewed = token(&sim, form).await;
    assert_eq!(
        initialize(&sim, renewed["access_token"].as_str().unwrap()).await,
        200
    );
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
}

/// A revoked grant's access tokens stop working too, and its refresh is
/// `invalid_grant`.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_grant_is_refused_everywhere() {
    let sim = OAuthSim::start().await;
    let tokens = sign_in(&sim).await;
    let access = tokens["access_token"].as_str().unwrap();
    let revoked = client().post(&sim.revoke_url).send().await.unwrap();
    assert!(revoked.status().is_success(), "{}", revoked.status());
    assert_eq!(initialize(&sim, access).await, 401);
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", tokens["refresh_token"].as_str().unwrap())
        .append_pair("client_id", "oxplow-test")
        .finish();
    let refused = client()
        .post(&sim.token_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 400);
    assert!(refused.text().await.unwrap().contains("invalid_grant"));
}
