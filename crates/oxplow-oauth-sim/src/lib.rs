//! A stand-in authorization server (P9.B3, made runnable in P10; there is
//! no real OAuth provider yet, by decision): `GET /authorize` redirects
//! straight back with a code, `POST /token` exchanges a code — checking
//! PKCE's verifier against the challenge the authorize request carried —
//! or a refresh token. It models what oxplow's client relies on (RFC 6749
//! §4.1, §6; RFC 7636), not any service.
//!
//! Beside it, at `/mcp`, the notes MCP server
//! ([`oxplow_provider_mcp::notes`]) takes exactly the access tokens it
//! issued and still holds live, so a signed-in bearer can be run end to
//! end. `POST /sim/expire` lapses every access token it issued and `POST
//! /sim/revoke` revokes the grant (refresh tokens and access tokens
//! alike), for the in-app walk; the tests reach the same through
//! [`OAuthSim`]'s methods.
//!
//! Development only: the tests' dev-dependency and the
//! `oxplow-oauth-sim` binary, never part of anything that ships.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::json;
use sha2::{Digest, Sha256};

/// What a code was issued for: a token request must match it.
struct Issued {
    challenge: String,
    client_id: String,
    redirect_uri: String,
}

#[derive(Default)]
struct World {
    /// Issued codes → what their authorize request carried.
    codes: HashMap<String, Issued>,
    /// Live refresh tokens.
    refresh: Vec<String>,
    /// Issued access tokens → when each lapses (unix seconds).
    access: HashMap<String, i64>,
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

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl World {
    /// Whether `token` is an access token it issued that hasn't lapsed
    /// and whose grant isn't revoked.
    fn takes(&self, token: &str) -> bool {
        !self.revoked && self.access.get(token).is_some_and(|at| now_secs() < *at)
    }

    /// Lapse every access token it issued.
    fn expire(&mut self) {
        let now = now_secs();
        for at in self.access.values_mut() {
            *at = now;
        }
    }
}

type Shared = Arc<Mutex<World>>;

/// A running authorization server; it stops when dropped.
pub struct OAuthSim {
    pub authorize_url: String,
    pub token_url: String,
    /// The notes MCP server, behind the access tokens it issues.
    pub mcp_url: String,
    /// `POST` here lapses every access token it issued.
    pub expire_url: String,
    /// `POST` here revokes the grant.
    pub revoke_url: String,
    world: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for OAuthSim {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl OAuthSim {
    /// Serve on a free loopback port.
    pub async fn start() -> OAuthSim {
        OAuthSim::bind("127.0.0.1:0")
            .await
            .expect("binding a loopback port")
    }

    /// Serve on `addr`.
    pub async fn bind(addr: &str) -> std::io::Result<OAuthSim> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let world: Shared = Arc::new(Mutex::new(World {
            ttl_secs: 3600,
            ..World::default()
        }));
        let task = tokio::spawn(serve(listener, world.clone()));
        let base = format!("http://{local}");
        Ok(OAuthSim {
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            mcp_url: format!("{base}/mcp"),
            expire_url: format!("{base}/sim/expire"),
            revoke_url: format!("{base}/sim/revoke"),
            world,
            task,
        })
    }

    /// Until it stops serving.
    pub async fn served(mut self) {
        let _ = (&mut self.task).await;
    }

    fn world(&self) -> parking_lot::MutexGuard<'_, World> {
        self.world.lock()
    }

    /// Access tokens issued from now on live `secs` (may be negative:
    /// already expired).
    pub fn set_ttl(&self, secs: i64) {
        self.world().ttl_secs = secs;
    }

    /// Every access token issued so far lapses now.
    pub fn expire(&self) {
        self.world().expire();
    }

    /// The grant is revoked: its access tokens stop working and the next
    /// refresh is `invalid_grant`.
    pub fn revoke(&self) {
        self.world().revoked = true;
    }

    /// How each token request authenticated the client.
    pub fn client_auths(&self) -> Vec<&'static str> {
        self.world().client_auths.clone()
    }

    /// Token requests are sent on to `url` with a `307`.
    pub fn redirect_token_requests(&self, url: &str) {
        self.world().redirect_to = Some(url.to_string());
    }

    /// Tokens are issued as `kind` rather than `Bearer`.
    pub fn set_token_type(&self, kind: &str) {
        self.world().token_type = Some(kind.to_string());
    }

    /// `expires_in` is written as a string, as some services do.
    pub fn expires_in_as_string(&self) {
        self.world().expires_as_string = true;
    }

    /// Token requests are answered after `ms` (a slow service).
    pub fn delay_token_requests(&self, ms: u64) {
        self.world().delay_ms = ms;
    }

    /// A refresh issues a new refresh token (and retires the old one).
    pub fn rotate_refresh_tokens(&self) {
        self.world().rotate = true;
    }

    /// The `grant_type` of every token request so far.
    pub fn grants(&self) -> Vec<String> {
        self.world().grants.clone()
    }

    /// The `client_secret` each token request carried.
    pub fn secrets(&self) -> Vec<Option<String>> {
        self.world().secrets.clone()
    }
}

async fn serve(listener: tokio::net::TcpListener, world: Shared) {
    let takes = world.clone();
    let notes = oxplow_provider_mcp::notes::http_router(
        Some(Arc::new(move |token: &str| takes.lock().takes(token))),
        oxplow_provider_mcp::notes::Refusal::Challenge,
    );
    let router = axum::Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route(
            "/sim/expire",
            post(|State(w): State<Shared>| async move {
                w.lock().expire();
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/sim/revoke",
            post(|State(w): State<Shared>| async move {
                w.lock().revoked = true;
                StatusCode::NO_CONTENT
            }),
        )
        .with_state(world)
        .merge(notes);
    let _ = axum::serve(listener, router).await;
}

fn params(text: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

fn redirect(status: StatusCode, to: &str) -> Response {
    (status, [(header::LOCATION, to.to_string())]).into_response()
}

async fn authorize(
    State(world): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let ok = q.get("response_type").map(String::as_str) == Some("code")
        && q.get("code_challenge_method").map(String::as_str) == Some("S256");
    let (Some(redirect_uri), Some(state), Some(challenge), Some(client_id), true) = (
        q.get("redirect_uri"),
        q.get("state"),
        q.get("code_challenge"),
        q.get("client_id"),
        ok,
    ) else {
        return (StatusCode::BAD_REQUEST, "bad authorize request").into_response();
    };
    let code = {
        let mut w = world.lock();
        w.issued += 1;
        let code = format!("code-{}", w.issued);
        w.codes.insert(
            code.clone(),
            Issued {
                challenge: challenge.clone(),
                client_id: client_id.clone(),
                redirect_uri: redirect_uri.clone(),
            },
        );
        code
    };
    let mut to = match url::Url::parse(redirect_uri) {
        Ok(to) => to,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad redirect_uri").into_response(),
    };
    to.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", state);
    redirect(StatusCode::FOUND, to.as_str())
}

async fn token(State(world): State<Shared>, headers: HeaderMap, body: String) -> Response {
    let delay = world.lock().delay_ms;
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
    let redirect_to = world.lock().redirect_to.clone();
    if let Some(to) = redirect_to {
        return redirect(StatusCode::TEMPORARY_REDIRECT, &to);
    }
    let form = params(&body);
    // The client's secret: in a Basic header (`id:secret`, each
    // form-encoded), or in the form.
    let decoded = |s: &str| params(&format!("s={s}")).remove("s").unwrap_or_default();
    let basic: Option<(String, String)> = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
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
        let mut w = world.lock();
        let grant = form.get("grant_type").cloned().unwrap_or_default();
        w.grants.push(grant.clone());
        let (how, secret) = match (&basic, form.get("client_secret")) {
            (Some((_, s)), _) => ("basic", Some(s.clone())),
            (None, Some(s)) => ("post", Some(s.clone())),
            (None, None) => ("none", None),
        };
        w.client_auths.push(how);
        w.secrets.push(secret);
        match grant.as_str() {
            "authorization_code" => {
                // The same client, redirect and verifier the code was
                // issued for (RFC 6749 §4.1.3, RFC 7636 §4.6).
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
                    Ok(issue(&mut w, Some(format!("rt-{n}"))))
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
                    Ok(issue(&mut w, Some(format!("rt-{n}"))))
                } else {
                    Ok(issue(&mut w, None))
                }
            }
            _ => Err("unsupported_grant_type"),
        }
    };
    let (status, body) = match answer {
        Ok(body) => (StatusCode::OK, body),
        Err(code) => (
            StatusCode::BAD_REQUEST,
            json!({ "error": code, "error_description": "the grant is no longer valid" }),
        ),
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Issue an access token (and `refresh`, when given), remembering when it
/// lapses: the token response's body.
fn issue(w: &mut World, refresh: Option<String>) -> serde_json::Value {
    w.issued += 1;
    let access = format!("at-{}", w.issued);
    w.access.insert(access.clone(), now_secs() + w.ttl_secs);
    let expires = if w.expires_as_string {
        json!(w.ttl_secs.to_string())
    } else {
        json!(w.ttl_secs)
    };
    let mut body = json!({
        "access_token": access,
        "token_type": w.token_type.clone().unwrap_or_else(|| "Bearer".into()),
        "expires_in": expires,
        "scope": "read write",
    });
    if let Some(r) = refresh {
        w.refresh.push(r.clone());
        body["refresh_token"] = json!(r);
    }
    body
}
