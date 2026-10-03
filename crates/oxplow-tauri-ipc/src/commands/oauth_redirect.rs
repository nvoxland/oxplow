//! The desktop shell catches a sign-in's redirect (P10,
//! `.context/providers.md` → "Signing in"): it listens on the person's
//! machine ([`oxplow_app::oauth_redirect`]), hands what came back to the
//! renderer — which gives it to the core (`complete_oauth_sign_in`) — and
//! answers the browser with how that went. So a sign-in works the same
//! with the core here or on a remote daemon. Shell-only: no daemon serves
//! these.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use oxplow_app::oauth_redirect::{Redirect, RedirectListener};
use oxplow_app::providers::oauth::SIGN_IN_WAIT;
use oxplow_app::providers::SignInCompletion;

use crate::error::IpcError;

/// One redirect listener, and the browser it is answering.
struct Listening {
    /// Held by the one [`await_oauth_redirect`] waiting on it.
    listener: tokio::sync::Mutex<RedirectListener>,
    /// The redirect handed to the renderer, awaiting its answer.
    held: parking_lot::Mutex<Option<Redirect>>,
    /// Set when the sign-in is over: a wait on it ends.
    stopped: tokio::sync::watch::Sender<bool>,
    since: Instant,
}

/// The shell's redirect listeners, by port.
#[derive(Default)]
pub struct OAuthRedirects(parking_lot::Mutex<HashMap<u16, Arc<Listening>>>);

impl OAuthRedirects {
    fn get(&self, port: u16) -> Result<Arc<Listening>, IpcError> {
        self.0
            .lock()
            .get(&port)
            .cloned()
            .ok_or_else(|| IpcError::invalid(format!("nothing listens for a sign-in on {port}")))
    }

    /// Stop listening on `port`: a wait on it ends.
    fn stop(&self, port: u16) {
        if let Some(listening) = self.0.lock().remove(&port) {
            listening.stopped.send_replace(true);
        }
    }
}

/// Listen on loopback for a sign-in's redirect — on `port` when the
/// service has one registered, any free port otherwise: the port.
#[tauri::command]
#[specta::specta]
pub async fn listen_for_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    port: Option<u16>,
) -> Result<u16, IpcError> {
    let listener = RedirectListener::bind(port.unwrap_or(0))
        .await
        .map_err(|e| {
            IpcError::invalid(format!(
                "listening for the sign-in's redirect on port {}: {e}",
                port.map_or("any".to_string(), |p| p.to_string())
            ))
        })?;
    let port = listener.port();
    redirects.0.lock().insert(
        port,
        Arc::new(Listening {
            listener: tokio::sync::Mutex::new(listener),
            held: parking_lot::Mutex::new(None),
            stopped: tokio::sync::watch::Sender::new(false),
            since: Instant::now(),
        }),
    );
    Ok(port)
}

/// The next redirect to `port`: the path and query the browser asked
/// for, to hand to the core. Its browser waits for
/// [`answer_oauth_redirect`]. It stops listening when the sign-in is
/// over (answered, or replaced) or not finished within
/// [`SIGN_IN_WAIT`].
#[tauri::command]
#[specta::specta]
pub async fn await_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    port: u16,
) -> Result<String, IpcError> {
    let listening = redirects.get(port)?;
    let mut stopped = listening.stopped.subscribe();
    let mut listener = listening.listener.lock().await;
    let left = SIGN_IN_WAIT.saturating_sub(listening.since.elapsed());
    tokio::select! {
        next = listener.next() => match next {
            Ok(redirect) => {
                let target = redirect.target.clone();
                *listening.held.lock() = Some(redirect);
                Ok(target)
            }
            Err(e) => {
                redirects.stop(port);
                Err(IpcError::internal(format!("the sign-in's redirect: {e}")))
            }
        },
        _ = stopped.wait_for(|s| *s) => Err(IpcError::invalid("the sign-in was stopped")),
        () = tokio::time::sleep(left) => {
            redirects.stop(port);
            Err(IpcError::invalid(format!(
                "the sign-in wasn't finished within {} minutes",
                SIGN_IN_WAIT.as_secs() / 60
            )))
        }
    }
}

/// Answer the browser waiting on `port` with how its redirect went (the
/// core's answer): a redirect that wasn't the sign-in's is refused and
/// the listener waits on; otherwise the sign-in is over and it stops
/// listening — with no browser waiting (the renderer gave up), it just
/// stops.
#[tauri::command]
#[specta::specta]
pub async fn answer_oauth_redirect(
    redirects: tauri::State<'_, OAuthRedirects>,
    port: u16,
    outcome: SignInCompletion,
) -> Result<(), IpcError> {
    let Ok(listening) = redirects.get(port) else {
        // Already over (timed out, replaced): nothing to answer.
        return Ok(());
    };
    let held = listening.held.lock().take();
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
    if let Some(redirect) = held {
        redirect.answer(ok, &page).await;
    }
    if over {
        redirects.stop(port);
    }
    Ok(())
}
