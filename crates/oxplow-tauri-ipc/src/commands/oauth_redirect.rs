//! The desktop shell catches a sign-in's redirect (P10,
//! `.context/providers.md` → "Signing in"): it listens on the person's
//! machine ([`oxplow_oauth_redirect`]), hands what came back to the
//! renderer — which gives it to the core (`complete_oauth_sign_in`) — and
//! answers the browser with how that went. So a sign-in works the same
//! with the core here or on a remote daemon. Shell-only: no daemon serves
//! these.

use std::sync::Arc;

use oxplow_app::providers::oauth::SIGN_IN_WAIT;
use oxplow_app::providers::SignInCompletion;
use oxplow_oauth_redirect::{Ended, ListenerId, RedirectListeners};

use crate::error::IpcError;

/// The shell's redirect listeners: each stops by itself once its
/// sign-in's [`SIGN_IN_WAIT`] is up, whether or not anyone waits on it.
pub struct OAuthRedirects(Arc<RedirectListeners>);

impl Default for OAuthRedirects {
    fn default() -> Self {
        OAuthRedirects(RedirectListeners::new(SIGN_IN_WAIT))
    }
}

/// A redirect listener: its id — what waits on it, answers it and stops
/// it (tsk905), never its port — and the port to sign in on.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OAuthListener {
    pub id: ListenerId,
    pub port: u16,
}

/// Listen on loopback for a sign-in's redirect — on `port` when the
/// service has one registered, any free port otherwise.
#[tauri::command]
#[specta::specta]
pub async fn listen_for_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    port: Option<u16>,
) -> Result<OAuthListener, IpcError> {
    let (id, port) = redirects.0.listen(port.unwrap_or(0)).await.map_err(|e| {
        IpcError::invalid(format!(
            "listening for the sign-in's redirect on port {}: {e}",
            port.map_or("any".to_string(), |p| p.to_string())
        ))
    })?;
    Ok(OAuthListener { id, port })
}

/// The next redirect to listener `listener`: the path and query the
/// browser asked for, to hand to the core. Its browser waits for
/// [`answer_oauth_redirect`]. Ends when the sign-in is over (answered,
/// stopped, or out of time).
#[tauri::command]
#[specta::specta]
pub async fn await_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    listener: ListenerId,
) -> Result<String, IpcError> {
    redirects
        .0
        .next(listener)
        .await
        .map_err(|ended| match ended {
            Ended::Unknown => IpcError::invalid("the sign-in isn't listening any more"),
            Ended::Stopped => IpcError::invalid("the sign-in was stopped"),
            Ended::Failed(e) => IpcError::internal(format!("the sign-in's redirect: {e}")),
        })
}

/// Answer the browser waiting on listener `listener` with how its
/// redirect went (the core's answer): a redirect that wasn't the
/// sign-in's is refused and the listener waits on; otherwise the sign-in
/// is over and it stops listening.
#[tauri::command]
#[specta::specta]
pub async fn answer_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    listener: ListenerId,
    outcome: SignInCompletion,
) -> Result<(), IpcError> {
    let (ok, page, over) = match &outcome {
        SignInCompletion::SignedIn => (
            true,
            "Signed in. You can close this tab and go back to oxplow.".to_string(),
            true,
        ),
        SignInCompletion::Failed { error } => (
            false,
            format!("oxplow couldn't finish the sign-in: {error}"),
            true,
        ),
        SignInCompletion::NotThisSignIn { reason } => (false, reason.clone(), false),
    };
    redirects.0.answer(listener, ok, &page, over).await;
    Ok(())
}

/// Stop listener `listener` (the sign-in was replaced, cancelled or
/// left): a browser it holds is told, its wait ends, and when this
/// returns its socket is closed — so the next sign-in can listen on the
/// same port at once.
#[tauri::command]
#[specta::specta]
pub async fn stop_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    listener: ListenerId,
) -> Result<(), IpcError> {
    redirects.0.stop(listener).await;
    Ok(())
}
