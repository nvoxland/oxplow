//! One side of a connection: send requests and await their replies (the
//! outstanding-id table), send notifications, answer the other side's
//! requests, and receive what it sends. Symmetric — the host and a
//! provider both use it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::codec::{notify, Id, Message};
use crate::errors::{ErrorObject, ProtocolError};

/// What the other side sent that isn't a reply.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request {
        id: Id,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

type Pending = Arc<Mutex<HashMap<Id, oneshot::Sender<Result<Value, ProtocolError>>>>>;
type Writer = Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>;

/// One side of a connection. Cheap to clone.
#[derive(Clone)]
pub struct Peer {
    writer: Writer,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

/// A request in flight: its id (to `$/cancel` it) and its reply.
pub struct Call {
    pub id: Id,
    reply: oneshot::Receiver<Result<Value, ProtocolError>>,
}

impl Call {
    /// The reply, or the error it came back with.
    pub async fn reply(self) -> Result<Value, ProtocolError> {
        self.reply
            .await
            .unwrap_or_else(|_| Err(ProtocolError::Internal("the peer closed".into())))
    }
}

impl Peer {
    /// Start reading `reader` (NDJSON) in the background; what isn't a
    /// reply arrives on the returned channel, which closes with the
    /// stream. Replies still awaited when it ends fail.
    pub fn spawn<R, W>(reader: R, writer: W) -> (Peer, mpsc::UnboundedReceiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let peer = Peer {
            writer: Arc::new(Mutex::new(Box::new(writer))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let pending = peer.pending.clone();
        let responder = peer.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match Message::from_line(&line) {
                    Ok(Message::Response { id, result }) => {
                        if let Some(waiter) = pending.lock().await.remove(&id) {
                            let _ = waiter.send(Ok(result));
                        }
                    }
                    Ok(Message::Error {
                        id: Some(id),
                        error,
                    }) => {
                        if let Some(waiter) = pending.lock().await.remove(&id) {
                            let _ = waiter.send(Err(error.into()));
                        }
                    }
                    Ok(Message::Error { id: None, .. }) => {}
                    Ok(Message::Request { id, method, params }) => {
                        let _ = tx.send(Incoming::Request { id, method, params });
                    }
                    Ok(Message::Notification { method, params }) => {
                        let _ = tx.send(Incoming::Notification { method, params });
                    }
                    Err(e) => {
                        let _ = responder
                            .send(&Message::Error {
                                id: None,
                                error: (&ProtocolError::Parse(e.to_string())).into(),
                            })
                            .await;
                    }
                }
            }
            for (_, waiter) in pending.lock().await.drain() {
                let _ = waiter.send(Err(ProtocolError::Internal("the peer closed".into())));
            }
        });
        (peer, rx)
    }

    async fn send(&self, message: &Message) -> Result<(), ProtocolError> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(message.to_line().as_bytes())
            .await
            .and(writer.flush().await)
            .map_err(|e| ProtocolError::Internal(format!("write: {e}")))
    }

    /// Send a request; its [`Call`] carries the id and the reply.
    pub async fn start(&self, method: &str, params: Value) -> Result<Call, ProtocolError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, reply) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if let Err(e) = self
            .send(&Message::Request {
                id,
                method: method.into(),
                params,
            })
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        Ok(Call { id, reply })
    }

    /// Send a request and await its reply.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, ProtocolError> {
        self.start(method, params).await?.reply().await
    }

    /// [`Self::request`] with typed params and result.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<R, ProtocolError> {
        let params = serde_json::to_value(params)
            .map_err(|e| ProtocolError::InvalidParams(e.to_string()))?;
        let result = self.request(method, params).await?;
        serde_json::from_value(result)
            .map_err(|e| ProtocolError::Internal(format!("`{method}` result: {e}")))
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), ProtocolError> {
        self.send(&Message::Notification {
            method: method.into(),
            params,
        })
        .await
    }

    /// Ask the other side to stop working on request `id`.
    pub async fn cancel(&self, id: Id) -> Result<(), ProtocolError> {
        self.notify(
            notify::CANCEL,
            serde_json::to_value(notify::Cancel { id }).expect("cancel serializes"),
        )
        .await
    }

    /// Answer the other side's request `id`.
    pub async fn respond(
        &self,
        id: Id,
        result: Result<Value, ProtocolError>,
    ) -> Result<(), ProtocolError> {
        let message = match result {
            Ok(result) => Message::Response { id, result },
            Err(e) => Message::Error {
                id: Some(id),
                error: ErrorObject::from(&e),
            },
        };
        self.send(&message).await
    }
}
