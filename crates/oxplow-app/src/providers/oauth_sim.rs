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

/// What a code was issued for: a token request must match it.
struct Issued {
    challenge: String,
    client_id: String,
    redirect_uri: String,
}

#[derive(Default)]
struct World {
    /// Issued codes → the challenge their authorize request carried.
    codes: HashMap<String, Issued>,
    /// Live refresh tokens.
    refresh: Vec<String>,
    issued: u32,
    /// The lifetime of the access tokens it issues.
    ttl_secs: i64,
    revoked: bool,
    /// Whether a refresh also issues a new refresh token.
    rotate: bool,
    /// How long a token request waits before it is answered.
    delay_ms: u64,
    /// How each token request authenticated the client: `basic`, `post`
    /// or `none`.
    client_auths: Vec<&'static str>,
    /// Token requests are sent on here (`307`) instead of answered.
    redirect_to: Option<String>,
    /// The `token_type` it answers with.
    token_type: Option<String>,
    /// It writes `expires_in` as a string.
    expires_as_string: bool,
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

    /// How each token request authenticated the client.
    pub fn client_auths(&self) -> Vec<&'static str> {
        self.world.lock().unwrap().client_auths.clone()
    }

    /// Token requests are sent on to `url` with a `307`.
    pub fn redirect_token_requests(&self, url: &str) {
        self.world.lock().unwrap().redirect_to = Some(url.to_string());
    }

    /// Tokens are issued as `kind` rather than `Bearer`.
    pub fn set_token_type(&self, kind: &str) {
        self.world.lock().unwrap().token_type = Some(kind.to_string());
    }

    /// `expires_in` is written as a string, as some services do.
    pub fn expires_in_as_string(&self) {
        self.world.lock().unwrap().expires_as_string = true;
    }

    /// Token requests are answered after `ms` (a slow service).
    pub fn delay_token_requests(&self, ms: u64) {
        self.world.lock().unwrap().delay_ms = ms;
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
            let (Some(redirect), Some(state), Some(challenge), Some(client_id), true) = (
                q.get("redirect_uri"),
                q.get("state"),
                q.get("code_challenge"),
                q.get("client_id"),
                ok,
            ) else {
                return respond(&mut conn, "400 Bad Request", "", "bad authorize request").await;
            };
            let code = {
                let mut w = world.lock().unwrap();
                w.issued += 1;
                let code = format!("code-{}", w.issued);
                w.codes.insert(
                    code.clone(),
                    Issued {
                        challenge: challenge.clone(),
                        client_id: client_id.clone(),
                        redirect_uri: redirect.clone(),
                    },
                );
                code
            };
            let to = format!("{redirect}?code={code}&state={state}");
            respond(&mut conn, "302 Found", &format!("Location: {to}\r\n"), "").await
        }
        ("POST", "/token") => {
            let delay = world.lock().unwrap().delay_ms;
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            let redirect_to = world.lock().unwrap().redirect_to.clone();
            if let Some(to) = redirect_to {
                return respond(
                    &mut conn,
                    "307 Temporary Redirect",
                    &format!("Location: {to}\r\n"),
                    "",
                )
                .await;
            }
            let form = params(&request.body);
            // The client's secret: in a Basic header (`id:secret`, each
            // form-encoded), or in the form.
            let decoded = |s: &str| params(&format!("s={s}")).remove("s").unwrap_or_default();
            let basic: Option<(String, String)> = request
                .header("authorization")
                .and_then(|h| h.strip_prefix("Basic "))
                .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                .and_then(|raw| String::from_utf8(raw).ok())
                .and_then(|pair| {
                    pair.split_once(':')
                        .map(|(id, secret)| (decoded(id), decoded(secret)))
                });
            // Which client asks: the Basic header's, else the form's.
            let client_id = basic
                .as_ref()
                .map(|(id, _)| id.clone())
                .or_else(|| form.get("client_id").cloned());
            let answer = {
                let mut w = world.lock().unwrap();
                let grant = form.get("grant_type").cloned().unwrap_or_default();
                w.grants.push(grant.clone());
                let (how, secret) = match (&basic, form.get("client_secret")) {
                    (Some((_, s)), _) => ("basic", Some(s.clone())),
                    (None, Some(s)) => ("post", Some(s.clone())),
                    (None, None) => ("none", None),
                };
                w.client_auths.push(how);
                w.secrets.push(secret);
                let issue = |w: &mut World, refresh: Option<String>| {
                    w.issued += 1;
                    let expires = if w.expires_as_string {
                        json!(w.ttl_secs.to_string())
                    } else {
                        json!(w.ttl_secs)
                    };
                    let mut body = json!({
                        "access_token": format!("at-{}", w.issued),
                        "token_type": w.token_type.clone().unwrap_or_else(|| "Bearer".into()),
                        "expires_in": expires,
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
                        // The same client, redirect and verifier the code
                        // was issued for (RFC 6749 §4.1.3, RFC 7636 §4.6).
                        let verified = form
                            .get("code")
                            .and_then(|c| w.codes.remove(c))
                            .is_some_and(|issued| {
                                client_id.as_deref() == Some(issued.client_id.as_str())
                                    && form.get("redirect_uri") == Some(&issued.redirect_uri)
                                    && form.get("code_verifier").is_some_and(|verifier| {
                                        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
                                            == issued.challenge
                                    })
                            });
                        if verified {
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
