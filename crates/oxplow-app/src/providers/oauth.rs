//! OAuth for provider credentials (P9.B3, `.context/providers.md` →
//! "Credentials and sign-in"): the authorization-code flow with PKCE
//! (OAuth 2.1; RFC 6749 §4.1, RFC 7636) and a loopback redirect (RFC
//! 8252 §7.3), run by oxplow for a credential a provider declares with
//! `oauth:`. The token — access, refresh, expiry — is one keychain
//! secret; the provider is handed the access token alone, by the
//! credential's name.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use oxplow_ai::secrets::SecretStore;

use super::spec::OAuthDecl;

/// What a sign-in yields, as kept in the keychain (JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    /// What gets a new access token without the person; none means the
    /// next expiry needs them.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC 3339; none means the service didn't say (it lasts until refused).
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl OAuthToken {
    /// Whether it lapses within `margin_ms` of now (never, when the
    /// service gave no expiry).
    fn lapses_within(&self, margin_ms: i64) -> bool {
        self.expires_at
            .as_deref()
            .and_then(|t| oxplow_domain::Timestamp::parse(t).ok())
            .is_some_and(|t| t.unix_ms() <= oxplow_domain::Timestamp::now().unix_ms() + margin_ms)
    }

    /// This token as one that needs the person: no way to renew, lapsed
    /// now. Kept (not deleted) so its row says "sign in again", not "not
    /// signed in".
    fn lapsed(&self) -> OAuthToken {
        OAuthToken {
            refresh_token: None,
            expires_at: Some(oxplow_domain::Timestamp::now().to_string()),
            ..self.clone()
        }
    }

    /// Where it stands for the person.
    pub fn state(&self) -> SignInState {
        match (&self.refresh_token, self.lapses_within(0)) {
            // It renews itself; its access token's expiry is nobody's concern.
            (Some(_), _) => SignInState::SignedIn { until: None },
            (None, false) => SignInState::SignedIn {
                until: self.expires_at.clone(),
            },
            (None, true) => SignInState::SignInAgain,
        }
    }
}

/// Where a signed-in credential stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SignInState {
    NotSignedIn,
    /// `until` (RFC 3339): when it lapses, for one that can't renew
    /// itself; none, it lasts until the service refuses it.
    SignedIn {
        until: Option<String>,
    },
    /// Its token lapsed or was revoked, and it can't renew itself.
    SignInAgain,
}

/// Why a signed-in credential has no access token to give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialProblem {
    NotSignedIn,
    /// The person signs in again; why.
    SignInAgain(String),
    /// Reading the keychain or reaching the token endpoint failed: worth
    /// trying again as it is.
    Failed(String),
}

/// How near its expiry a token is renewed before it is handed out.
const RENEW_MARGIN_MS: i64 = 60_000;

/// The token kept under `account`, if the person signed in.
pub fn stored(secrets: &dyn SecretStore, account: &str) -> Result<Option<OAuthToken>, String> {
    let Some(text) = secrets.get(account).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("what the keychain holds isn't a sign-in's token: {e}"))
}

/// Keep `token` under `account`.
pub fn store(secrets: &dyn SecretStore, account: &str, token: &OAuthToken) -> Result<(), String> {
    let text = serde_json::to_string(token).map_err(|e| e.to_string())?;
    secrets.set(account, &text).map_err(|e| e.to_string())
}

/// Where the credential kept under `account` stands.
pub fn state(secrets: &dyn SecretStore, account: &str) -> SignInState {
    match stored(secrets, account) {
        Ok(Some(token)) => token.state(),
        Ok(None) => SignInState::NotSignedIn,
        // Unreadable: nothing to start on; signing in replaces it.
        Err(_) => SignInState::SignInAgain,
    }
}

/// The access token for the credential kept under `account`, renewed
/// first (and kept) when it is about to lapse — or, with `renew`, because
/// the service refused it. A renewal the service refuses for good
/// (`invalid_grant`) leaves the token marked as needing the person.
pub async fn access_token(
    secrets: &dyn SecretStore,
    account: &str,
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    renew: bool,
) -> Result<String, CredentialProblem> {
    let token = stored(secrets, account)
        .map_err(CredentialProblem::Failed)?
        .ok_or(CredentialProblem::NotSignedIn)?;
    if !renew && !token.lapses_within(RENEW_MARGIN_MS) {
        return Ok(token.access_token);
    }
    let again = |token: &OAuthToken, why: String| match store(secrets, account, &token.lapsed()) {
        Ok(()) => CredentialProblem::SignInAgain(why),
        Err(e) => CredentialProblem::Failed(e),
    };
    if token.refresh_token.is_none() {
        let why = if token.lapses_within(0) {
            "it expired and can't renew itself"
        } else if renew {
            "its service refused it and it can't renew itself"
        } else {
            // Inside the margin but still good, with no way to renew.
            return Ok(token.access_token);
        };
        return Err(again(&token, why.to_string()));
    }
    match refresh(decl, client_secret, &token).await {
        Ok(renewed) => {
            store(secrets, account, &renewed).map_err(CredentialProblem::Failed)?;
            Ok(renewed.access_token)
        }
        Err(OAuthError::Revoked(why)) => Err(again(&token, why)),
        // The endpoint couldn't be reached: a token that's still good is
        // still good.
        Err(OAuthError::Failed(_)) if !renew && !token.lapses_within(0) => Ok(token.access_token),
        Err(OAuthError::Failed(why)) => Err(CredentialProblem::Failed(why)),
    }
}

/// Why a token request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    /// The grant is no longer valid (`invalid_grant`): the refresh token
    /// was revoked or expired — the person signs in again.
    Revoked(String),
    Failed(String),
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OAuthError::Revoked(m) | OAuthError::Failed(m) => f.write_str(m),
        }
    }
}

/// An HTTP request as far as the loopback listener (and the tests'
/// authorization server) read one.
pub struct HttpRequest {
    pub method: String,
    /// The path and query.
    pub target: String,
    pub body: String,
}

/// Read one HTTP/1.1 request: its line, headers and `Content-Length` body.
pub async fn read_request(conn: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or_default().to_string(),
        parts.next().unwrap_or_default().to_string(),
    );
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length.min(64 * 1024)];
    reader.read_exact(&mut body).await?;
    Ok(HttpRequest {
        method,
        target,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// PKCE's S256 challenge for `verifier`.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 64 unguessable characters from PKCE's unreserved set: a verifier, or a
/// `state`.
fn unguessable() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// How long a sign-in waits for the person to finish in their browser.
pub const SIGN_IN_WAIT: Duration = Duration::from_secs(300);

/// How long a token request may take.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

/// A sign-in under way: where the person goes, and what it comes to.
pub struct SignIn {
    pub authorize_url: String,
    /// The token, once the redirect came back and its code was exchanged;
    /// or why not (the person never finished, the service refused). It
    /// owns the loopback listener: the redirect is answered only while
    /// this is awaited, and dropping it stops listening at once.
    pub done:
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<OAuthToken, String>> + Send>>,
}

/// Start a sign-in for `decl`: listen on loopback for the redirect and
/// return where the person signs in. `client_secret` is the value of the
/// credential `decl.client_secret` names, when it names one.
///
/// The listener answers only `GET /callback` carrying this sign-in's
/// `state` — the redirect's authentication: another page sending the
/// browser there is refused and the wait goes on — then exchanges the
/// code (with the PKCE verifier only this process holds) and closes.
pub async fn begin(decl: &OAuthDecl, client_secret: Option<String>) -> Result<SignIn, String> {
    let listener = TcpListener::bind(("127.0.0.1", decl.redirect_port.unwrap_or(0)))
        .await
        .map_err(|e| format!("listening for the sign-in's redirect: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("listening for the sign-in's redirect: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let (verifier, state) = (unguessable(), unguessable());
    let mut url = url::Url::parse(&decl.authorize_url)
        .map_err(|e| format!("`authorize_url` isn't a URL: {e}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &decl.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &pkce_challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    if !decl.scopes.is_empty() {
        url.query_pairs_mut()
            .append_pair("scope", &decl.scopes.join(" "));
    }
    let decl = decl.clone();
    let done = async move {
        tokio::time::timeout(
            SIGN_IN_WAIT,
            await_redirect(
                listener,
                &decl,
                client_secret.as_deref(),
                &state,
                &verifier,
                &redirect_uri,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "the sign-in wasn't finished within {} minutes",
                SIGN_IN_WAIT.as_secs() / 60
            ))
        })
    };
    Ok(SignIn {
        authorize_url: url.to_string(),
        done: Box::pin(done),
    })
}

/// Answer connections until the redirect with our `state` arrives; then
/// exchange its code and tell the browser how it went.
async fn await_redirect(
    listener: TcpListener,
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthToken, String> {
    loop {
        let (mut conn, _) = listener
            .accept()
            .await
            .map_err(|e| format!("the sign-in's redirect: {e}"))?;
        let Ok(request) = read_request(&mut conn).await else {
            continue;
        };
        let (path, query) = request
            .target
            .split_once('?')
            .unwrap_or((&request.target, ""));
        if request.method != "GET" || path != "/callback" {
            let _ = respond(&mut conn, "404 Not Found", "Nothing here.").await;
            continue;
        }
        let query: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
        if query.get("state").map(String::as_str) != Some(state) {
            let _ = respond(
                &mut conn,
                "400 Bad Request",
                "This isn't the sign-in oxplow started. Nothing was done.",
            )
            .await;
            continue;
        }
        let outcome = match (query.get("code"), query.get("error")) {
            (Some(code), _) => exchange(decl, client_secret, code, verifier, redirect_uri)
                .await
                .map_err(|e| e.to_string()),
            (None, Some(error)) => Err(format!(
                "the sign-in was refused: {}",
                query.get("error_description").unwrap_or(error)
            )),
            (None, None) => Err("the redirect carried no code".to_string()),
        };
        let page = match &outcome {
            Ok(_) => "Signed in. You can close this tab and go back to oxplow.".to_string(),
            Err(why) => format!("oxplow couldn't finish the sign-in: {why}"),
        };
        let _ = respond(&mut conn, "200 OK", &page).await;
        return outcome;
    }
}

/// A plain-text page for the person's browser.
async fn respond(conn: &mut TcpStream, status: &str, text: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    conn.write_all(head.as_bytes()).await?;
    conn.write_all(text.as_bytes()).await?;
    conn.shutdown().await
}

/// The token for an authorization `code` (RFC 6749 §4.1.3, with PKCE's
/// verifier).
async fn exchange(
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthToken, OAuthError> {
    token_request(
        decl,
        client_secret,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await
}

/// A new access token from `token`'s refresh token (RFC 6749 §6). A
/// service that answers without a new refresh token keeps the old one.
pub async fn refresh(
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    token: &OAuthToken,
) -> Result<OAuthToken, OAuthError> {
    let Some(refresh_token) = token.refresh_token.as_deref() else {
        return Err(OAuthError::Revoked(
            "it has expired and can't be renewed without signing in".into(),
        ));
    };
    let mut renewed = token_request(
        decl,
        client_secret,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )
    .await?;
    if renewed.refresh_token.is_none() {
        renewed.refresh_token = token.refresh_token.clone();
    }
    Ok(renewed)
}

/// What a token endpoint answers (RFC 6749 §5.1, §5.2).
#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn token_request(
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    grant: &[(&str, &str)],
) -> Result<OAuthToken, OAuthError> {
    let failed = |why: String| OAuthError::Failed(format!("the token request failed: {why}"));
    // Built before any await: the serializer isn't `Send`.
    let body = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in grant {
            form.append_pair(name, value);
        }
        form.append_pair("client_id", &decl.client_id);
        if let Some(secret) = client_secret {
            form.append_pair("client_secret", secret);
        }
        form.finish()
    };
    let client = reqwest::Client::builder()
        .timeout(TOKEN_TIMEOUT)
        .build()
        .map_err(|e| failed(e.to_string()))?;
    let response = client
        .post(&decl.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| failed(e.to_string()))?;
    let answer: TokenResponse = serde_json::from_str(&text)
        .map_err(|_| failed(format!("{status}: the answer isn't a token response")))?;
    if let Some(error) = answer.error {
        let said = answer.error_description.unwrap_or_else(|| error.clone());
        return Err(if error == "invalid_grant" {
            OAuthError::Revoked(said)
        } else {
            failed(format!("{error}: {said}"))
        });
    }
    let Some(access_token) = answer.access_token.filter(|_| status.is_success()) else {
        return Err(failed(format!("{status}: no access token in the answer")));
    };
    let now = oxplow_domain::Timestamp::now().unix_ms();
    Ok(OAuthToken {
        access_token,
        refresh_token: answer.refresh_token,
        expires_at: answer.expires_in.map(|secs| {
            oxplow_domain::Timestamp::from_unix_ms(now.saturating_add(secs.saturating_mul(1000)))
                .to_string()
        }),
        scope: answer.scope,
    })
}

#[cfg(test)]
mod tests {
    use super::super::oauth_sim::OAuthSim;
    use super::*;

    fn decl(sim: &OAuthSim) -> OAuthDecl {
        OAuthDecl {
            authorize_url: sim.authorize_url.clone(),
            token_url: sim.token_url.clone(),
            client_id: "oxplow-test".into(),
            scopes: vec!["read".into(), "write".into()],
            client_secret: None,
            redirect_port: None,
        }
    }

    /// A sign-in being waited for, as the registry waits for one.
    struct Waited {
        authorize_url: String,
        done: tokio::task::JoinHandle<Result<OAuthToken, String>>,
    }

    async fn begin(decl: &OAuthDecl, client_secret: Option<String>) -> Result<Waited, String> {
        let sign_in = super::begin(decl, client_secret).await?;
        Ok(Waited {
            authorize_url: sign_in.authorize_url,
            done: tokio::spawn(sign_in.done),
        })
    }

    /// What the person's browser does: open the page, follow the redirect
    /// back to oxplow's loopback listener.
    async fn browse(url: &str) -> (u16, String) {
        let resp = reqwest::get(url).await.unwrap();
        (resp.status().as_u16(), resp.text().await.unwrap())
    }

    /// RFC 7636 appendix B.
    #[test]
    fn pkce_challenge_is_s256_of_the_verifier() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[tokio::test]
    async fn signing_in_exchanges_the_code_with_its_verifier() {
        let sim = OAuthSim::start().await;
        let sign_in = begin(&decl(&sim), None).await.unwrap();
        assert!(sign_in.authorize_url.starts_with(&sim.authorize_url));
        for wanted in [
            "response_type=code",
            "client_id=oxplow-test",
            "code_challenge_method=S256",
            "scope=read+write",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A",
        ] {
            assert!(
                sign_in.authorize_url.contains(wanted),
                "{wanted}: {}",
                sign_in.authorize_url
            );
        }
        let (status, page) = browse(&sign_in.authorize_url).await;
        assert_eq!(status, 200);
        assert!(
            page.contains("oxplow"),
            "the page says where to go back: {page}"
        );
        let token = sign_in.done.await.unwrap().unwrap();
        // The simulator only issues a token for the verifier matching the
        // challenge the authorize request carried.
        assert!(token.access_token.starts_with("at-"));
        assert!(token
            .refresh_token
            .as_deref()
            .is_some_and(|t| t.starts_with("rt-")));
        assert!(token.expires_at.is_some());
        assert_eq!(sim.grants(), vec!["authorization_code"]);
    }

    #[tokio::test]
    async fn a_callback_with_the_wrong_state_is_refused() {
        let sim = OAuthSim::start().await;
        let sign_in = begin(&decl(&sim), None).await.unwrap();
        let url = url::Url::parse(&sign_in.authorize_url).unwrap();
        let redirect = url
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .unwrap()
            .1
            .into_owned();
        // Someone else's page sends the browser to the listener.
        let (status, _) = browse(&format!("{redirect}?code=stolen&state=not-ours")).await;
        assert_eq!(status, 400);
        assert_eq!(sim.grants(), Vec::<String>::new(), "no code was exchanged");
        // The real redirect still works.
        let (status, _) = browse(&sign_in.authorize_url).await;
        assert_eq!(status, 200);
        assert!(sign_in.done.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn a_refresh_keeps_or_rotates_the_refresh_token_and_a_revoked_one_says_so() {
        let sim = OAuthSim::start().await;
        let d = decl(&sim);
        let sign_in = begin(&d, None).await.unwrap();
        browse(&sign_in.authorize_url).await;
        let first = sign_in.done.await.unwrap().unwrap();

        // The service answers a refresh without a new refresh token: the
        // old one is kept.
        let second = refresh(&d, None, &first).await.unwrap();
        assert_ne!(second.access_token, first.access_token);
        assert_eq!(second.refresh_token, first.refresh_token);

        sim.rotate_refresh_tokens();
        let third = refresh(&d, None, &second).await.unwrap();
        assert_ne!(third.refresh_token, second.refresh_token);
        // The retired one no longer works.
        assert!(matches!(
            refresh(&d, None, &second).await,
            Err(OAuthError::Revoked(_))
        ));

        sim.revoke();
        assert!(matches!(
            refresh(&d, None, &third).await,
            Err(OAuthError::Revoked(_))
        ));
        let no_refresh = OAuthToken {
            refresh_token: None,
            ..third
        };
        assert!(matches!(
            refresh(&d, None, &no_refresh).await,
            Err(OAuthError::Revoked(_))
        ));
    }

    #[tokio::test]
    async fn a_client_secret_goes_with_each_token_request() {
        let sim = OAuthSim::start().await;
        let d = decl(&sim);
        let sign_in = begin(&d, Some("s3cret".into())).await.unwrap();
        browse(&sign_in.authorize_url).await;
        let token = sign_in.done.await.unwrap().unwrap();
        refresh(&d, Some("s3cret"), &token).await.unwrap();
        assert_eq!(
            sim.secrets(),
            vec![Some("s3cret".to_string()), Some("s3cret".to_string())]
        );
        // And never in the URL the browser opens.
        assert!(!sign_in.authorize_url.contains("s3cret"));
    }

    /// Sign in against `sim` and keep the token under `account`.
    async fn signed_in(sim: &OAuthSim, secrets: &dyn SecretStore, account: &str) -> OAuthToken {
        let sign_in = begin(&decl(sim), None).await.unwrap();
        browse(&sign_in.authorize_url).await;
        let token = sign_in.done.await.unwrap().unwrap();
        store(secrets, account, &token).unwrap();
        token
    }

    #[tokio::test]
    async fn an_access_token_about_to_lapse_is_renewed_and_kept() {
        let sim = OAuthSim::start().await;
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let d = decl(&sim);
        assert_eq!(
            access_token(&secrets, "acct", &d, None, false).await,
            Err(CredentialProblem::NotSignedIn)
        );
        assert_eq!(state(&secrets, "acct"), SignInState::NotSignedIn);
        // Thirty seconds left: inside the margin.
        sim.set_ttl(30);
        let first = signed_in(&sim, &secrets, "acct").await;
        assert_eq!(
            state(&secrets, "acct"),
            SignInState::SignedIn { until: None }
        );
        sim.set_ttl(3600);
        let renewed = access_token(&secrets, "acct", &d, None, false)
            .await
            .unwrap();
        assert_ne!(renewed, first.access_token);
        assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
        // Kept: the next one asks nobody.
        assert_eq!(
            access_token(&secrets, "acct", &d, None, false)
                .await
                .unwrap(),
            renewed
        );
        assert_eq!(sim.grants().len(), 2);
        // Its service refused it: renewed although it looks good.
        let forced = access_token(&secrets, "acct", &d, None, true)
            .await
            .unwrap();
        assert_ne!(forced, renewed);
        assert_eq!(sim.grants().len(), 3);
    }

    #[tokio::test]
    async fn a_refused_renewal_asks_the_person_to_sign_in_again() {
        let sim = OAuthSim::start().await;
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let d = decl(&sim);
        sim.set_ttl(-10);
        signed_in(&sim, &secrets, "acct").await;
        sim.revoke();
        let refused = access_token(&secrets, "acct", &d, None, false).await;
        assert!(
            matches!(refused, Err(CredentialProblem::SignInAgain(_))),
            "{refused:?}"
        );
        // Kept, marked: the row says "sign in again", and nothing asks the
        // service again.
        assert_eq!(state(&secrets, "acct"), SignInState::SignInAgain);
        assert!(stored(&secrets, "acct").unwrap().is_some());
        let grants = sim.grants().len();
        assert!(matches!(
            access_token(&secrets, "acct", &d, None, false).await,
            Err(CredentialProblem::SignInAgain(_))
        ));
        assert_eq!(sim.grants().len(), grants);
    }

    #[tokio::test]
    async fn a_token_that_cannot_renew_itself_is_good_until_it_lapses() {
        let sim = OAuthSim::start().await;
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let d = decl(&sim);
        let token = signed_in(&sim, &secrets, "acct").await;
        let alone = OAuthToken {
            refresh_token: None,
            ..token
        };
        store(&secrets, "acct", &alone).unwrap();
        assert_eq!(
            state(&secrets, "acct"),
            SignInState::SignedIn {
                until: alone.expires_at.clone()
            }
        );
        assert_eq!(
            access_token(&secrets, "acct", &d, None, false)
                .await
                .unwrap(),
            alone.access_token
        );
        // Refused by its service with nothing to renew it: the person's.
        assert!(matches!(
            access_token(&secrets, "acct", &d, None, true).await,
            Err(CredentialProblem::SignInAgain(_))
        ));
        assert_eq!(state(&secrets, "acct"), SignInState::SignInAgain);
        assert_eq!(sim.grants(), vec!["authorization_code"]);
    }

    /// A sign-in nobody waits for stops listening: the next one (with a
    /// fixed `redirect_port`) gets the port.
    #[tokio::test]
    async fn an_abandoned_sign_in_frees_its_port() {
        let sim = OAuthSim::start().await;
        let port = {
            let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            l.local_addr().unwrap().port()
        };
        let d = OAuthDecl {
            redirect_port: Some(port),
            ..decl(&sim)
        };
        let first = super::begin(&d, None).await.unwrap();
        assert!(super::begin(&d, None).await.is_err(), "the port is taken");
        drop(first);
        let again = begin(&d, None).await.unwrap();
        browse(&again.authorize_url).await;
        assert!(again.done.await.unwrap().is_ok());
    }
}
