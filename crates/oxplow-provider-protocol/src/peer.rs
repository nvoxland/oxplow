//! One side of a connection: send requests and await their replies (the
//! outstanding-id table), send notifications, answer the other side's
//! requests, and receive what it sends. Symmetric — the host and a
//! provider both use it.
//!
//! A request started with [`Peer::start_streaming`] also gets the
//! notifications the other side sends about it (`$/progress`,
//! `$/record`, `$/state`, by their `id`), in order, on a channel of its
//! own that closes when its reply arrives — so a `read`'s rows can't be
//! mistaken for another's or lost before the caller is listening.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
type Streams = Arc<std::sync::Mutex<HashMap<Id, mpsc::UnboundedSender<Incoming>>>>;

/// The notifications that are about an in-flight request (its `id`).
const ABOUT_A_REQUEST: [&str; 3] = [notify::PROGRESS, notify::RECORD, notify::STATE];
type Writer = Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>;

/// One side of a connection. Cheap to clone.
#[derive(Clone)]
pub struct Peer {
    writer: Writer,
    pending: Pending,
    streams: Streams,
    next_id: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
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
            streams: Arc::default(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let pending = peer.pending.clone();
        let streams = peer.streams.clone();
        let responder = peer.clone();
        // A reply ends its request's stream: everything the other side
        // sent about it came before the reply, so it is all queued.
        let end_stream = {
            let streams = streams.clone();
            move |id: &Id| {
                streams.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            }
        };
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match Message::from_line(&line) {
                    Ok(Message::Response { id, result }) => {
                        end_stream(&id);
                        if let Some(waiter) = pending.lock().await.remove(&id) {
                            let _ = waiter.send(Ok(result));
                        }
                    }
                    Ok(Message::Error {
                        id: Some(id),
                        error,
                    }) => {
                        end_stream(&id);
                        if let Some(waiter) = pending.lock().await.remove(&id) {
                            let _ = waiter.send(Err(error.into()));
                        }
                    }
                    Ok(Message::Error { id: None, .. }) => {}
                    Ok(Message::Request { id, method, params }) => {
                        let _ = tx.send(Incoming::Request { id, method, params });
                    }
                    Ok(Message::Notification { method, params }) => {
                        let stream = ABOUT_A_REQUEST
                            .contains(&method.as_str())
                            .then(|| params.get("id").and_then(Value::as_u64))
                            .flatten()
                            .and_then(|id| {
                                streams
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .get(&id)
                                    .cloned()
                            });
                        let message = Incoming::Notification { method, params };
                        match stream {
                            Some(stream) => {
                                let _ = stream.send(message);
                            }
                            None => {
                                let _ = tx.send(message);
                            }
                        }
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
            responder.closed.store(true, Ordering::SeqCst);
            streams.lock().unwrap_or_else(|e| e.into_inner()).clear();
            for (_, waiter) in pending.lock().await.drain() {
                let _ = waiter.send(Err(ProtocolError::Internal("the peer closed".into())));
            }
        });
        (peer, rx)
    }

    /// The other side's stream has ended: no reply will come.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
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
        self.start_with(method, params, None).await
    }

    /// [`Self::start`], with the notifications about this request
    /// (`$/progress`, `$/record`, `$/state` naming its id) on their own
    /// channel — registered before the request is sent, and closed when
    /// its reply arrives (or the stream ends).
    pub async fn start_streaming(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(Call, mpsc::UnboundedReceiver<Incoming>), ProtocolError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let call = self.start_with(method, params, Some(tx)).await?;
        Ok((call, rx))
    }

    async fn start_with(
        &self,
        method: &str,
        params: Value,
        stream: Option<mpsc::UnboundedSender<Incoming>>,
    ) -> Result<Call, ProtocolError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, reply) = oneshot::channel();
        {
            // Under the lock the reader drains `pending` with once it has
            // marked the stream closed: either this sees `closed`, or the
            // drain sees this waiter.
            let mut pending = self.pending.lock().await;
            if self.is_closed() {
                return Err(ProtocolError::Internal("the peer closed".into()));
            }
            pending.insert(id, tx);
            if let Some(stream) = stream {
                self.streams
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, stream);
            }
        }
        if let Err(e) = self
            .send(&Message::Request {
                id,
                method: method.into(),
                params,
            })
            .await
        {
            self.pending.lock().await.remove(&id);
            self.streams
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
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
