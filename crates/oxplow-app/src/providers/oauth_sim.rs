//! A stand-in authorization server for the OAuth tests (P9.B3; the user
//! decided there is no real OAuth provider yet): `GET /authorize`
//! redirects straight back with a code, `POST /token` exchanges a code —
//! checking PKCE's verifier against the challenge the authorize request
//! carried — or a refresh token. It models what oxplow's client relies
//! on (RFC 6749 §4.1, §6; RFC 7636), not any service.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use super::oauth::read_request;

#[derive(Default)]
struct World {
    /// Issued codes → the challenge their authorize request carried.
    codes: HashMap<String, String>,
    /// Live refresh tokens.
    refresh: Vec<String>,
    issued: u32,
    /// The lifetime of the access tokens it issues.
    ttl_secs: i64,
    revoked: bool,
    /// Whether a refresh also issues a new refresh token.
    rotate: bool,
    /// Every token request's `grant_type`, in order.
    grants: Vec<String>,
    /// The `client_secret` each token request carried.
    secrets: Vec<Option<String>>,
}

/// A running authorization server; it stops when dropped.
pub struct OAuthSim {
    pub authorize_url: String,
    pub token_url: String,
    world: Arc<Mutex<World>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for OAuthSim {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl OAuthSim {
    pub async fn start() -> OAuthSim {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let world = Arc::new(Mutex::new(World {
            ttl_secs: 3600,
            ..World::default()
        }));
        let served = world.clone();
        let task = tokio::spawn(async move {
            while let Ok((conn, _)) = listener.accept().await {
                let world = served.clone();
                tokio::spawn(async move {
                    let _ = serve(conn, world).await;
                });
            }
        });
        OAuthSim {
            authorize_url: format!("http://127.0.0.1:{port}/authorize"),
            token_url: format!("http://127.0.0.1:{port}/token"),
            world,
            task,
        }
    }

    /// Access tokens issued from now on live `secs` (may be negative:
    /// already expired).
    pub fn set_ttl(&self, secs: i64) {
        self.world.lock().unwrap().ttl_secs = secs;
    }

    /// Every refresh token stops working: the next refresh is
    /// `invalid_grant`.
    pub fn revoke(&self) {
        self.world.lock().unwrap().revoked = true;
    }

    /// A refresh issues a new refresh token (and retires the old one).
    pub fn rotate_refresh_tokens(&self) {
        self.world.lock().unwrap().rotate = true;
    }

    /// The `grant_type` of every token request so far.
    pub fn grants(&self) -> Vec<String> {
        self.world.lock().unwrap().grants.clone()
    }

    /// The `client_secret` each token request carried.
    pub fn secrets(&self) -> Vec<Option<String>> {
        self.world.lock().unwrap().secrets.clone()
    }
}

fn params(text: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

async fn respond(
    conn: &mut TcpStream,
    status: &str,
    headers: &str,
    body: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    );
    conn.write_all(head.as_bytes()).await?;
    conn.write_all(body.as_bytes()).await?;
    conn.shutdown().await
}

async fn serve(mut conn: TcpStream, world: Arc<Mutex<World>>) -> std::io::Result<()> {
    let request = read_request(&mut conn).await?;
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    match (request.method.as_str(), path) {
        ("GET", "/authorize") => {
            let q = params(query);
            let ok = q.get("response_type").map(String::as_str) == Some("code")
                && q.get("code_challenge_method").map(String::as_str) == Some("S256")
                && q.contains_key("client_id");
            let (Some(redirect), Some(state), Some(challenge), true) = (
                q.get("redirect_uri"),
                q.get("state"),
                q.get("code_challenge"),
                ok,
            ) else {
                return respond(&mut conn, "400 Bad Request", "", "bad authorize request").await;
            };
            let code = {
                let mut w = world.lock().unwrap();
                w.issued += 1;
                let code = format!("code-{}", w.issued);
                w.codes.insert(code.clone(), challenge.clone());
                code
            };
            let to = format!("{redirect}?code={code}&state={state}");
            respond(&mut conn, "302 Found", &format!("Location: {to}\r\n"), "").await
        }
        ("POST", "/token") => {
            let form = params(&request.body);
            let answer = {
                let mut w = world.lock().unwrap();
                let grant = form.get("grant_type").cloned().unwrap_or_default();
                w.grants.push(grant.clone());
                w.secrets.push(form.get("client_secret").cloned());
                let issue = |w: &mut World, refresh: Option<String>| {
                    w.issued += 1;
                    let mut body = json!({
                        "access_token": format!("at-{}", w.issued),
                        "token_type": "Bearer",
                        "expires_in": w.ttl_secs,
                        "scope": "read write",
                    });
                    if let Some(r) = refresh {
                        w.refresh.push(r.clone());
                        body["refresh_token"] = json!(r);
                    }
                    Ok(body)
                };
                match grant.as_str() {
                    "authorization_code" => {
                        let verified = form
                            .get("code")
                            .and_then(|c| w.codes.remove(c))
                            .zip(form.get("code_verifier"))
                            .is_some_and(|(challenge, verifier)| {
                                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
                                    == challenge
                            });
                        if verified && form.contains_key("redirect_uri") {
                            let n = w.issued + 1;
                            issue(&mut w, Some(format!("rt-{n}")))
                        } else {
                            Err("invalid_grant")
                        }
                    }
                    "refresh_token" => {
                        let known = form
                            .get("refresh_token")
                            .is_some_and(|t| w.refresh.contains(t));
                        if w.revoked || !known {
                            Err("invalid_grant")
                        } else if w.rotate {
                            let old = form.get("refresh_token").cloned().unwrap_or_default();
                            w.refresh.retain(|t| *t != old);
                            let n = w.issued + 1;
                            issue(&mut w, Some(format!("rt-{n}")))
                        } else {
                            issue(&mut w, None)
                        }
                    }
                    _ => Err("unsupported_grant_type"),
                }
            };
            match answer {
                Ok(body) => {
                    respond(
                        &mut conn,
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        &body.to_string(),
                    )
                    .await
                }
                Err(code) => respond(
                    &mut conn,
                    "400 Bad Request",
                    "Content-Type: application/json\r\n",
                    &json!({ "error": code, "error_description": "the grant is no longer valid" })
                        .to_string(),
                )
                .await,
            }
        }
        _ => respond(&mut conn, "404 Not Found", "", "").await,
    }
}
