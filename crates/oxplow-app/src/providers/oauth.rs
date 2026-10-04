//! OAuth for provider credentials (P9.B3, `.context/providers.md` →
//! "Credentials and sign-in"): the authorization-code flow with PKCE
//! (OAuth 2.1; RFC 6749 §4.1, RFC 7636) and a loopback redirect (RFC
//! 8252 §7.3), run by oxplow for a credential a provider declares with
//! `oauth:`. The core starts a sign-in and finishes it; the redirect is
//! caught by the desktop shell (`oxplow-oauth-redirect`) and handed over,
//! so the core never listens. The token — access, refresh, expiry — is one keychain
//! secret; the provider is handed the access token alone, by the
//! credential's name.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use oxplow_ai::secrets::SecretStore;

use super::spec::{ClientAuth, OAuthDecl};

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

/// One renewal of an account at a time in this process: every instance
/// built for it (a running one, a Settings Check, a replacement during a
/// reconcile) and every project open in the process shares the account.
fn renewal_lock(account: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::LazyLock<
        parking_lot::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::LazyLock::new(Default::default);
    LOCKS.lock().entry(account.to_string()).or_default().clone()
}

/// The access token for the credential kept under `account`, renewed
/// first (and kept) when it is about to lapse — or because its service
/// refused it: `refused` is the access token that was refused, and it is
/// renewed only while it is still the one stored (tsk928). A caller that
/// waited on another's renewal, or a refusal of a token another instance
/// has already replaced, gets what is stored and spends no refresh. A
/// renewal the service refuses for good (`invalid_grant`) leaves the
/// token marked as needing the person.
///
/// Renewals of one account are serialized in this process (a second
/// finds the token the first stored), and every write first re-reads what
/// is stored (tsk827): a renewal never writes over a sign-out, and a
/// refusal for a token another process already replaced uses the
/// replacement rather than marking the account lapsed. Across processes
/// the keychain offers no compare-and-swap, so that check narrows the
/// window rather than closing it.
pub async fn access_token(
    secrets: &dyn SecretStore,
    account: &str,
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    refused: Option<&str>,
) -> Result<String, CredentialProblem> {
    let lock = renewal_lock(account);
    let _one = lock.lock().await;
    let token = stored(secrets, account)
        .map_err(CredentialProblem::Failed)?
        .ok_or(CredentialProblem::NotSignedIn)?;
    // Refused, and still the one stored: renew it. Already replaced: the
    // replacement is used like any stored token.
    let renew = refused.is_some_and(|r| r == token.access_token);
    if !renew && !token.lapses_within(RENEW_MARGIN_MS) {
        return Ok(token.access_token);
    }
    // What is stored now, against the token this renewal began with.
    let now = || stored(secrets, account).map_err(CredentialProblem::Failed);
    // Mark it needing the person — unless it changed meanwhile.
    let again = |why: String| -> Result<String, CredentialProblem> {
        match now()? {
            None => Err(CredentialProblem::NotSignedIn),
            Some(current) if current != token => usable(current, why),
            Some(_) => match store(secrets, account, &token.lapsed()) {
                Ok(()) => Err(CredentialProblem::SignInAgain(why)),
                Err(e) => Err(CredentialProblem::Failed(e)),
            },
        }
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
        return again(why.to_string());
    }
    match refresh(decl, client_secret, &token).await {
        Ok(renewed) => match now()? {
            // Signed out while it renewed: stays signed out.
            None => Err(CredentialProblem::NotSignedIn),
            // Replaced meanwhile (another process, a new sign-in): theirs.
            Some(current) if current != token => usable(current, "replaced".into()),
            Some(_) => {
                store(secrets, account, &renewed).map_err(CredentialProblem::Failed)?;
                Ok(renewed.access_token)
            }
        },
        Err(OAuthError::Revoked(why)) => again(why),
        // The endpoint couldn't be reached: a token that's still good is
        // still good.
        Err(OAuthError::Failed(_)) if !renew && !token.lapses_within(0) => Ok(token.access_token),
        Err(OAuthError::Failed(why)) => Err(CredentialProblem::Failed(why)),
    }
}

/// A token that replaced the one a renewal began with: its access token,
/// unless it has already lapsed with no way to renew.
fn usable(current: OAuthToken, why: String) -> Result<String, CredentialProblem> {
    match current.state() {
        SignInState::SignedIn { .. } => Ok(current.access_token),
        SignInState::SignInAgain => Err(CredentialProblem::SignInAgain(why)),
        SignInState::NotSignedIn => Err(CredentialProblem::NotSignedIn),
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

/// A sign-in started: where the person signs in, and what finishing it
/// needs.
pub struct SignIn {
    pub authorize_url: String,
    pub pending: PendingSignIn,
}

/// What finishing a sign-in needs, kept by the core until its redirect
/// comes back: the `state` that authenticates the redirect and the PKCE
/// verifier only this process holds.
pub struct PendingSignIn {
    decl: OAuthDecl,
    client_secret: Option<String>,
    state: String,
    verifier: String,
    redirect_uri: String,
}

/// What a redirect to a sign-in brought back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirected {
    /// A code to exchange for the token.
    Code(String),
    /// Why the service, or the person, said no.
    Refused(String),
}

/// Start a sign-in for `decl` whose redirect comes back to
/// `127.0.0.1:<redirect_port>`, where the desktop shell listens
/// (`oxplow-oauth-redirect`): where the person signs in, and what
/// finishing it needs. `client_secret` is the value of the credential
/// `decl.client_secret` names, when it names one. A provider that
/// declares its `redirect_port` (a service with the port registered) is
/// signed in for on that port only.
pub fn begin(
    decl: &OAuthDecl,
    client_secret: Option<String>,
    redirect_port: u16,
) -> Result<SignIn, String> {
    if redirect_port == 0 {
        return Err("a sign-in needs the port its redirect comes back on".into());
    }
    if let Some(declared) = decl.redirect_port {
        if declared != redirect_port {
            return Err(format!(
                "its service takes the redirect on port {declared} only, not {redirect_port}"
            ));
        }
    }
    let redirect_uri = format!("http://127.0.0.1:{redirect_port}/callback");
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
    Ok(SignIn {
        authorize_url: url.to_string(),
        pending: PendingSignIn {
            decl: decl.clone(),
            client_secret,
            state,
            verifier,
            redirect_uri,
        },
    })
}

impl PendingSignIn {
    /// The declaration it was started with (the one checked as approved).
    pub fn decl(&self) -> &OAuthDecl {
        &self.decl
    }

    /// What `redirect` — the path and query the browser was sent to, or
    /// the whole URL — brought back to this sign-in. `Err(why)` when it
    /// isn't this sign-in's (another `state` — the redirect's
    /// authentication — or another path): nothing is done, and it still
    /// waits.
    pub fn redirected(&self, redirect: &str) -> Result<Redirected, String> {
        let url = url::Url::parse("http://127.0.0.1/")
            .and_then(|base| base.join(redirect))
            .map_err(|e| format!("that isn't a redirect: {e}"))?;
        if url.path() != "/callback" {
            return Err("that isn't a sign-in's redirect".into());
        }
        let query: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        if query.get("state") != Some(&self.state) {
            return Err("this isn't the sign-in oxplow started; nothing was done".into());
        }
        Ok(match (query.get("code"), query.get("error")) {
            (Some(code), _) => Redirected::Code(code.clone()),
            (None, Some(error)) => Redirected::Refused(format!(
                "the sign-in was refused: {}",
                query.get("error_description").unwrap_or(error)
            )),
            (None, None) => Redirected::Refused("the redirect carried no code".into()),
        })
    }

    /// The token for this sign-in's `code`.
    pub async fn exchange(&self, code: &str) -> Result<OAuthToken, String> {
        exchange(
            &self.decl,
            self.client_secret.as_deref(),
            code,
            &self.verifier,
            &self.redirect_uri,
        )
        .await
        .map_err(|e| e.to_string())
    }
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
    /// What kind of token: the only kind oxplow sends is a bearer token.
    /// Absent is read as bearer (some services leave it out).
    token_type: Option<String>,
    refresh_token: Option<String>,
    /// Seconds; some services write it as a string.
    #[serde(default, deserialize_with = "seconds")]
    expires_in: Option<i64>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// A number of seconds written as a number or a string of digits.
fn seconds<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seconds {
        Number(i64),
        Text(String),
    }
    match Option::<Seconds>::deserialize(d)? {
        None => Ok(None),
        Some(Seconds::Number(n)) => Ok(Some(n)),
        Some(Seconds::Text(s)) => s.trim().parse().map(Some).map_err(|_| {
            serde::de::Error::custom(format!("`expires_in` `{s}` isn't a number of seconds"))
        }),
    }
}

async fn token_request(
    decl: &OAuthDecl,
    client_secret: Option<&str>,
    grant: &[(&str, &str)],
) -> Result<OAuthToken, OAuthError> {
    let failed = |why: String| OAuthError::Failed(format!("the token request failed: {why}"));
    // The client authenticates one way (RFC 6749 §2.3.1): its id and
    // secret in an `Authorization: Basic` header — what every service must
    // take — or, for a service that says so, in the form.
    let basic = match (client_secret, decl.client_auth) {
        (Some(secret), ClientAuth::Basic) => {
            let encode =
                |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
            Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!(
                    "{}:{}",
                    encode(&decl.client_id),
                    encode(secret)
                ))
            ))
        }
        _ => None,
    };
    // Built before any await: the serializer isn't `Send`.
    let body = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in grant {
            form.append_pair(name, value);
        }
        if basic.is_none() {
            form.append_pair("client_id", &decl.client_id);
            if let Some(secret) = client_secret {
                form.append_pair("client_secret", secret);
            }
        }
        form.finish()
    };
    // A token request is never sent on: a redirect would re-send the code,
    // the verifier, the refresh token or the secret somewhere the approval
    // doesn't name (tsk829).
    let client = reqwest::Client::builder()
        .timeout(TOKEN_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| failed(e.to_string()))?;
    let mut request = client
        .post(&decl.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json");
    if let Some(basic) = basic {
        request = request.header("Authorization", basic);
    }
    let response = request
        .body(body)
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    if response.status().is_redirection() {
        return Err(failed(format!(
            "{}: the token endpoint redirected, and a token request is never sent on",
            response.status()
        )));
    }
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
    if let Some(kind) = answer
        .token_type
        .filter(|t| !t.eq_ignore_ascii_case("bearer"))
    {
        return Err(failed(format!(
            "the service issued a `{kind}` token; oxplow sends bearer tokens only"
        )));
    }
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
    use super::*;
    use oxplow_oauth_sim::OAuthSim;

    fn decl(sim: &OAuthSim) -> OAuthDecl {
        OAuthDecl {
            authorize_url: sim.authorize_url.clone(),
            token_url: sim.token_url.clone(),
            client_id: "oxplow-test".into(),
            scopes: vec!["read".into(), "write".into()],
            client_secret: None,
            redirect_port: None,
            client_auth: ClientAuth::Basic,
        }
    }

    /// The port the shell would catch the redirect on. Nothing listens:
    /// the redirect is read off the authorization server's answer, as the
    /// shell hands it over.
    const PORT: u16 = 8124;

    /// What the person's browser does: open the page; the redirect it is
    /// sent back with, as the shell catches it.
    async fn browse(url: &str) -> String {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resp = client.get(url).send().await.unwrap();
        resp.headers()["location"].to_str().unwrap().to_string()
    }

    /// A sign-in at `decl` finished as the person would: the page they
    /// signed in on, and what it came to.
    async fn sign_in(
        decl: &OAuthDecl,
        client_secret: Option<String>,
    ) -> (String, Result<OAuthToken, String>) {
        let started = begin(decl, client_secret, PORT).unwrap();
        let redirect = browse(&started.authorize_url).await;
        let token = match started.pending.redirected(&redirect).unwrap() {
            Redirected::Code(code) => started.pending.exchange(&code).await,
            Redirected::Refused(why) => Err(why),
        };
        (started.authorize_url, token)
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
        let (page, token) = sign_in(&decl(&sim), None).await;
        assert!(page.starts_with(&sim.authorize_url));
        for wanted in [
            "response_type=code",
            "client_id=oxplow-test",
            "code_challenge_method=S256",
            "scope=read+write",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8124%2Fcallback",
        ] {
            assert!(page.contains(wanted), "{wanted}: {page}");
        }
        // The simulator only issues a token for the verifier matching the
        // challenge the authorize request carried.
        let token = token.unwrap();
        assert!(token.access_token.starts_with("at-"));
        assert!(token
            .refresh_token
            .as_deref()
            .is_some_and(|t| t.starts_with("rt-")));
        assert!(token.expires_at.is_some());
        assert_eq!(sim.grants(), vec!["authorization_code"]);
    }

    /// A redirect is this sign-in's only with its `state` — the
    /// redirect's authentication — at `/callback`: anything else does
    /// nothing, and the real one still works.
    #[tokio::test]
    async fn a_redirect_with_another_state_isnt_this_sign_ins() {
        let sim = OAuthSim::start().await;
        let started = begin(&decl(&sim), None, PORT).unwrap();
        let state = url::Url::parse(&started.authorize_url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        // Someone else's page sends the browser to the listener.
        assert!(started
            .pending
            .redirected("/callback?code=stolen&state=not-ours")
            .is_err());
        assert!(started
            .pending
            .redirected(&format!("/elsewhere?code=c&state={state}"))
            .is_err());
        assert_eq!(sim.grants(), Vec::<String>::new(), "no code was exchanged");
        // The service saying no is this sign-in's answer.
        assert_eq!(
            started
                .pending
                .redirected(&format!("/callback?error=access_denied&state={state}")),
            Ok(Redirected::Refused(
                "the sign-in was refused: access_denied".into()
            ))
        );
        // The real redirect still works, as a path or a whole URL.
        let redirect = browse(&started.authorize_url).await;
        assert!(redirect.starts_with("http://127.0.0.1:8124/callback?"));
        let Ok(Redirected::Code(code)) = started.pending.redirected(&redirect) else {
            panic!("{redirect}");
        };
        assert!(started.pending.exchange(&code).await.is_ok());
    }

    #[tokio::test]
    async fn a_refresh_keeps_or_rotates_the_refresh_token_and_a_revoked_one_says_so() {
        let sim = OAuthSim::start().await;
        let d = decl(&sim);
        let (_, first) = sign_in(&d, None).await;
        let first = first.unwrap();

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
        let (page, token) = sign_in(&d, Some("s3cret".into())).await;
        let token = token.unwrap();
        refresh(&d, Some("s3cret"), &token).await.unwrap();
        assert_eq!(
            sim.secrets(),
            vec![Some("s3cret".to_string()), Some("s3cret".to_string())]
        );
        // tsk829: in the `Authorization` header (`client_secret_basic`,
        // what every service must take), unless the service says `post`.
        assert_eq!(sim.client_auths(), vec!["basic", "basic"]);
        // And never in the URL the browser opens.
        assert!(!page.contains("s3cret"));

        let post = OAuthDecl {
            client_auth: ClientAuth::Post,
            ..d
        };
        refresh(&post, Some("s3cret"), &token).await.unwrap();
        assert_eq!(sim.client_auths(), vec!["basic", "basic", "post"]);
    }

    /// tsk829: a token request is never sent on: a redirect would re-send
    /// the code, the verifier or the refresh token (and the client secret)
    /// somewhere the approval doesn't name.
    #[tokio::test]
    async fn a_token_endpoint_that_redirects_is_refused() {
        let sim = OAuthSim::start().await;
        let elsewhere = OAuthSim::start().await;
        let d = decl(&sim);
        let (_, token) = sign_in(&d, None).await;
        let token = token.unwrap();
        sim.redirect_token_requests(&elsewhere.token_url);
        assert!(matches!(
            refresh(&d, None, &token).await,
            Err(OAuthError::Failed(_))
        ));
        assert!(elsewhere.grants().is_empty(), "nothing was sent on");
    }

    /// tsk829: the answer's `token_type` must be one oxplow sends (bearer);
    /// an `expires_in` written as a string is still a lifetime.
    #[tokio::test]
    async fn a_token_answer_is_read_as_services_write_it() {
        let sim = OAuthSim::start().await;
        let d = decl(&sim);
        let (_, token) = sign_in(&d, None).await;
        let token = token.unwrap();
        sim.expires_in_as_string();
        let renewed = refresh(&d, None, &token).await.unwrap();
        assert!(renewed.expires_at.is_some(), "{renewed:?}");
        sim.set_token_type("mac");
        let refused = refresh(&d, None, &token).await;
        assert!(
            matches!(&refused, Err(OAuthError::Failed(why)) if why.contains("mac")),
            "{refused:?}"
        );
    }

    /// Sign in against `sim` and keep the token under `account`.
    async fn signed_in(sim: &OAuthSim, secrets: &dyn SecretStore, account: &str) -> OAuthToken {
        let (_, token) = sign_in(&decl(sim), None).await;
        let token = token.unwrap();
        store(secrets, account, &token).unwrap();
        token
    }

    #[tokio::test]
    async fn an_access_token_about_to_lapse_is_renewed_and_kept() {
        let sim = OAuthSim::start().await;
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let d = decl(&sim);
        assert_eq!(
            access_token(&secrets, "acct", &d, None, None).await,
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
        let renewed = access_token(&secrets, "acct", &d, None, None)
            .await
            .unwrap();
        assert_ne!(renewed, first.access_token);
        assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
        // Kept: the next one asks nobody.
        assert_eq!(
            access_token(&secrets, "acct", &d, None, None)
                .await
                .unwrap(),
            renewed
        );
        assert_eq!(sim.grants().len(), 2);
        // Its service refused it: renewed although it looks good.
        let forced = access_token(&secrets, "acct", &d, None, Some(&renewed))
            .await
            .unwrap();
        assert_ne!(forced, renewed);
        assert_eq!(sim.grants().len(), 3);
        // tsk928: another caller refused the same token, arriving after the
        // renewal: it gets the renewed one, and spends no refresh.
        assert_eq!(
            access_token(&secrets, "acct", &d, None, Some(&renewed))
                .await
                .unwrap(),
            forced
        );
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
        let refused = access_token(&secrets, "acct", &d, None, None).await;
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
            access_token(&secrets, "acct", &d, None, None).await,
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
            access_token(&secrets, "acct", &d, None, None)
                .await
                .unwrap(),
            alone.access_token
        );
        // Refused by its service with nothing to renew it: the person's.
        assert!(matches!(
            access_token(&secrets, "acct", &d, None, Some(&alone.access_token)).await,
            Err(CredentialProblem::SignInAgain(_))
        ));
        assert_eq!(state(&secrets, "acct"), SignInState::SignInAgain);
        assert_eq!(sim.grants(), vec!["authorization_code"]);
    }

    /// A sign-in comes back where the shell listens; a service with its
    /// port registered (`redirect_port`) is signed in for on that port
    /// only.
    #[tokio::test]
    async fn a_declared_redirect_port_is_the_only_one() {
        let sim = OAuthSim::start().await;
        assert!(begin(&decl(&sim), None, 0).is_err(), "a port is needed");
        let d = OAuthDecl {
            redirect_port: Some(PORT),
            ..decl(&sim)
        };
        let refused = begin(&d, None, PORT + 1).err().unwrap();
        assert!(refused.contains(&format!("port {PORT} only")), "{refused}");
        let (_, token) = sign_in(&d, None).await;
        assert!(token.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_renewals_of_one_account_renew_once() {
        let sim = OAuthSim::start().await;
        let secrets = std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let d = decl(&sim);
        sim.set_ttl(30);
        signed_in(&sim, secrets.as_ref(), "acct").await;
        sim.set_ttl(3600);
        sim.rotate_refresh_tokens();
        sim.delay_token_requests(100);
        let run = || {
            let (secrets, d) = (secrets.clone(), d.clone());
            tokio::spawn(
                async move { access_token(secrets.as_ref(), "acct", &d, None, None).await },
            )
        };
        let (a, b) = (run(), run());
        let (a, b) = (a.await.unwrap(), b.await.unwrap());
        assert!(a.is_ok() && b.is_ok(), "{a:?} {b:?}");
        assert_eq!(a, b, "both have the one renewed token");
        assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
        assert_eq!(
            state(secrets.as_ref(), "acct"),
            SignInState::SignedIn { until: None }
        );
    }

    /// tsk827: signing out while a renewal is in flight stays signed out:
    /// the renewal doesn't write its token back over the deletion.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_sign_out_during_a_renewal_stays_signed_out() {
        let sim = OAuthSim::start().await;
        let secrets = std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let d = decl(&sim);
        let refused = signed_in(&sim, secrets.as_ref(), "acct").await.access_token;
        sim.delay_token_requests(300);
        let renewing = {
            let (secrets, d) = (secrets.clone(), d.clone());
            tokio::spawn(async move {
                access_token(secrets.as_ref(), "acct", &d, None, Some(&refused)).await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        secrets.delete("acct").unwrap();
        assert_eq!(renewing.await.unwrap(), Err(CredentialProblem::NotSignedIn));
        assert_eq!(state(secrets.as_ref(), "acct"), SignInState::NotSignedIn);
    }

    /// tsk827: a refusal for a token another process has already replaced
    /// (its renewal rotated the refresh token) uses the replacement rather
    /// than marking the account lapsed.
    #[tokio::test]
    async fn a_refusal_of_a_token_already_replaced_uses_the_replacement() {
        let sim = OAuthSim::start().await;
        let secrets = std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let d = decl(&sim);
        let first = signed_in(&sim, secrets.as_ref(), "acct").await;
        sim.rotate_refresh_tokens();
        sim.delay_token_requests(300);
        // Another process (no lock shared with this one) renews first...
        let other = {
            let (secrets, d, first) = (secrets.clone(), d.clone(), first.clone());
            tokio::spawn(async move {
                let theirs = refresh(&d, None, &first).await.unwrap();
                store(secrets.as_ref(), "acct", &theirs).unwrap();
                theirs
            })
        };
        // ...and this one, reading the first token meanwhile, is refused.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let ours = access_token(
            secrets.as_ref(),
            "acct",
            &d,
            None,
            Some(&first.access_token),
        )
        .await;
        let theirs = other.await.unwrap();
        assert_eq!(ours, Ok(theirs.access_token.clone()));
        assert_eq!(stored(secrets.as_ref(), "acct").unwrap(), Some(theirs));
    }

    /// A code from `sim` for `client_id`, redirected to `redirect_uri`,
    /// bound to `verifier` — as the person's browser would bring it back.
    async fn code_for(
        sim: &OAuthSim,
        client_id: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> String {
        let mut url = url::Url::parse(&sim.authorize_url).unwrap();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", "s")
            .append_pair("code_challenge", &pkce_challenge(verifier))
            .append_pair("code_challenge_method", "S256");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resp = client.get(url).send().await.unwrap();
        let to = resp.headers()["location"].to_str().unwrap().to_string();
        url::Url::parse(&to)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "code")
            .unwrap()
            .1
            .into_owned()
    }

    /// tsk830: the stand-in server holds a code to what it was issued for
    /// (RFC 6749 §4.1.3): the same client, the same redirect, the same
    /// verifier — so a client that mixed them up would fail here.
    #[tokio::test]
    async fn a_code_is_exchanged_only_as_it_was_issued() {
        let sim = OAuthSim::start().await;
        let d = decl(&sim);
        let verifier = "v".repeat(43);
        let other = "w".repeat(43);
        let redirect = "http://127.0.0.1:1/callback";
        for (client, to, with, says) in [
            (
                "someone-else",
                redirect,
                verifier.as_str(),
                "another client",
            ),
            (
                "oxplow-test",
                "http://127.0.0.1:2/callback",
                verifier.as_str(),
                "another redirect",
            ),
            ("oxplow-test", redirect, other.as_str(), "another verifier"),
        ] {
            let code = code_for(&sim, client, redirect, &verifier).await;
            let refused = exchange(&d, None, &code, with, to).await;
            assert!(
                matches!(refused, Err(OAuthError::Revoked(_))),
                "{says}: {refused:?}"
            );
        }
        let code = code_for(&sim, "oxplow-test", redirect, &verifier).await;
        assert!(exchange(&d, None, &code, &verifier, redirect).await.is_ok());
    }
}
