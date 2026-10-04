//! The loopback listeners a sign-in's redirect comes back to (RFC 8252
//! §7.3) — the **desktop shell's**, never the core's (P10,
//! `.context/providers.md` → "Signing in"). The shell listens where the
//! person's browser is, hands what came back to the core
//! (`complete_oauth_sign_in`, which checks it is that sign-in's and
//! exchanges the code), and answers the browser with how it went. So a
//! sign-in works the same whether the core runs here or on a remote
//! daemon. Guard (in the core): `the_core_never_binds_a_socket_for_a_sign_in`.
//!
//! [`RedirectListeners`] keeps the shell's listeners: each stops by itself
//! once its sign-in's time is up, whether or not anyone is waiting on it
//! (tsk904) — a held redirect is answered, and the socket closed, so a
//! declared port is free again.

mod listener;

pub use listener::{Redirect, RedirectListener};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// One listener, and the browser it is answering.
struct Listening {
    /// Taken by the one waiting on it ([`RedirectListeners::next`]); taken
    /// out for good, closing the socket, when it stops.
    listener: tokio::sync::Mutex<Option<RedirectListener>>,
    /// The redirect handed over, awaiting its answer.
    held: parking_lot::Mutex<Option<Redirect>>,
    /// Set when the sign-in is over: a wait on it ends.
    stopped: tokio::sync::watch::Sender<bool>,
    /// Stops it when its time is up.
    deadline: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Why waiting on a listener ended without a redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Nothing listens on that port.
    Unknown,
    /// The sign-in is over: answered, replaced, or out of time.
    Stopped,
    /// The socket failed.
    Failed(String),
}

/// The shell's redirect listeners, by port.
pub struct RedirectListeners {
    wait: Duration,
    listening: parking_lot::Mutex<HashMap<u16, Arc<Listening>>>,
}

impl RedirectListeners {
    /// Listeners that stop by themselves `wait` after they start (the
    /// core's `SIGN_IN_WAIT`).
    pub fn new(wait: Duration) -> Arc<Self> {
        Arc::new(RedirectListeners {
            wait,
            listening: parking_lot::Mutex::new(HashMap::new()),
        })
    }

    fn get(&self, port: u16) -> Option<Arc<Listening>> {
        self.listening.lock().get(&port).cloned()
    }

    /// Listen on `port` (any free port for `0`): the port. It stops by
    /// itself when its time is up.
    pub async fn listen(self: &Arc<Self>, port: u16) -> std::io::Result<u16> {
        let listener = RedirectListener::bind(port).await?;
        let port = listener.port();
        let listening = Arc::new(Listening {
            listener: tokio::sync::Mutex::new(Some(listener)),
            held: parking_lot::Mutex::new(None),
            stopped: tokio::sync::watch::Sender::new(false),
            deadline: parking_lot::Mutex::new(None),
        });
        self.listening.lock().insert(port, listening.clone());
        let (me, wait) = (Arc::downgrade(self), self.wait);
        let deadline = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            if let Some(me) = me.upgrade() {
                let page = format!(
                    "The sign-in wasn't finished within {} minutes. Start it again in oxplow.",
                    wait.as_secs() / 60
                );
                me.answer(port, false, &page, true).await;
            }
        });
        *listening.deadline.lock() = Some(deadline);
        Ok(port)
    }

    /// The next redirect to `port`: what the browser asked for, to hand to
    /// the core. Its browser waits for [`Self::answer`].
    pub async fn next(&self, port: u16) -> Result<String, Ended> {
        let listening = self.get(port).ok_or(Ended::Unknown)?;
        let mut stopped = listening.stopped.subscribe();
        let mut slot = listening.listener.lock().await;
        let Some(listener) = slot.as_mut() else {
            return Err(Ended::Stopped);
        };
        tokio::select! {
            next = listener.next() => match next {
                Ok(redirect) => {
                    let target = redirect.target.clone();
                    *listening.held.lock() = Some(redirect);
                    Ok(target)
                }
                Err(e) => {
                    drop(slot);
                    self.stop(port).await;
                    Err(Ended::Failed(e.to_string()))
                }
            },
            () = async {
                // The guard it returns isn't `Send`: let it go here.
                let _ = stopped.wait_for(|s| *s).await;
            } => Err(Ended::Stopped),
        }
    }

    /// Answer the browser waiting on `port` with `page` (`ok`: signed in);
    /// when the sign-in is `over`, stop listening. With nothing listening
    /// (already over), nothing to answer.
    pub async fn answer(&self, port: u16, ok: bool, page: &str, over: bool) {
        let Some(listening) = self.get(port) else {
            return;
        };
        let held = listening.held.lock().take();
        if let Some(redirect) = held {
            redirect.answer(ok, page).await;
        }
        if over {
            self.stop(port).await;
        }
    }

    /// Stop listening on `port`: a wait on it ends, and once it has, the
    /// socket is closed — when this returns, the port is free.
    pub async fn stop(&self, port: u16) {
        let Some(listening) = self.listening.lock().remove(&port) else {
            return;
        };
        listening.stopped.send_replace(true);
        if let Some(deadline) = listening.deadline.lock().take() {
            deadline.abort();
        }
        listening.listener.lock().await.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(port: u16, target: &str) -> (u16, String) {
        let resp = reqwest::get(format!("http://127.0.0.1:{port}{target}"))
            .await
            .unwrap();
        (resp.status().as_u16(), resp.text().await.unwrap())
    }

    /// tsk904: with nobody waiting on it (the window closed, the renderer
    /// reloaded), a listener still stops when its time is up: a held
    /// redirect is answered and the port is free for the next sign-in.
    #[tokio::test]
    async fn a_listener_stops_when_its_time_is_up_with_nobody_waiting() {
        let listeners = RedirectListeners::new(Duration::from_millis(300));
        let port = listeners.listen(0).await.unwrap();
        let browser = tokio::spawn(async move { get(port, "/callback?state=s").await });
        assert_eq!(
            listeners.next(port).await.unwrap(),
            "/callback?state=s",
            "handed over, then nobody answers"
        );
        let (status, page) = browser.await.unwrap();
        assert_eq!(status, 400);
        assert!(page.contains("wasn't finished"), "{page}");
        assert_eq!(
            listeners.listen(port).await.unwrap(),
            port,
            "the port is free"
        );
    }

    /// tsk904: a stop ends a wait, and when it returns the socket is
    /// closed — listening on the same port at once works.
    #[tokio::test]
    async fn a_stop_ends_the_wait_and_frees_the_port() {
        let listeners = RedirectListeners::new(Duration::from_secs(300));
        let port = listeners.listen(0).await.unwrap();
        let waiting = {
            let listeners = listeners.clone();
            tokio::spawn(async move { listeners.next(port).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        listeners.stop(port).await;
        assert_eq!(waiting.await.unwrap(), Err(Ended::Stopped));
        assert_eq!(listeners.listen(port).await.unwrap(), port);
    }

    /// Two listens on one port: the second fails while the first holds it.
    #[tokio::test]
    async fn two_listens_on_one_port_conflict() {
        let listeners = RedirectListeners::new(Duration::from_secs(300));
        let port = listeners.listen(0).await.unwrap();
        assert!(listeners.listen(port).await.is_err());
    }

    /// An answer reaches the browser; one that isn't the end keeps the
    /// listener waiting, one that is stops it.
    #[tokio::test]
    async fn an_answer_reaches_the_browser_and_ends_the_sign_in_when_over() {
        let listeners = RedirectListeners::new(Duration::from_secs(300));
        let port = listeners.listen(0).await.unwrap();
        let first = tokio::spawn(async move { get(port, "/callback?state=old").await });
        listeners.next(port).await.unwrap();
        listeners.answer(port, false, "Not this one.", false).await;
        assert_eq!(first.await.unwrap(), (400, "Not this one.".to_string()));
        let second = tokio::spawn(async move { get(port, "/callback?state=s").await });
        listeners.next(port).await.unwrap();
        listeners.answer(port, true, "Signed in.", true).await;
        assert_eq!(second.await.unwrap(), (200, "Signed in.".to_string()));
        assert_eq!(listeners.next(port).await, Err(Ended::Unknown));
    }
}
