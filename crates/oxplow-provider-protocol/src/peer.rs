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
//!
//! Everything is bounded ([`PeerLimits`]): a line longer than
//! `max_line_bytes` ends the connection (what was awaited fails saying
//! why), and the channels hold `channel_capacity` messages — a reader that
//! falls behind slows the other side down (its writes block on the pipe)
//! rather than growing a queue without end.

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
type Streams = Arc<std::sync::Mutex<HashMap<Id, mpsc::Sender<Incoming>>>>;

/// How much one connection holds at most.
#[derive(Debug, Clone, Copy)]
pub struct PeerLimits {
    /// The longest line (one message) read; a longer one ends the
    /// connection.
    pub max_line_bytes: usize,
    /// The messages each channel (what isn't a reply, and each streaming
    /// request's notifications) holds before the reader waits.
    pub channel_capacity: usize,
}

impl Default for PeerLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 16 * 1024 * 1024,
            channel_capacity: 1024,
        }
    }
}

/// One line read, at most `max` bytes before its newline.
enum Line {
    Text(String),
    TooLong,
    End,
}

async fn read_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buf: &mut Vec<u8>,
    max: usize,
) -> Line {
    buf.clear();
    loop {
        let available = match reader.fill_buf().await {
            Ok(b) => b,
            Err(_) => return Line::End,
        };
        if available.is_empty() {
            // The stream ended; a last line without its newline still counts.
            return match buf.is_empty() {
                true => Line::End,
                false => Line::Text(String::from_utf8_lossy(buf).into_owned()),
            };
        }
        let (taken, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let content = if done { taken - 1 } else { taken };
        if buf.len() + content > max {
            return Line::TooLong;
        }
        buf.extend_from_slice(&available[..content]);
        reader.consume(taken);
        if done {
            return Line::Text(String::from_utf8_lossy(buf).into_owned());
        }
    }
}

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
    /// Woken when the other side's stream ends.
    ended: Arc<tokio::sync::Notify>,
    /// The replies still awaited when it ended (each failed).
    cut_off: Arc<AtomicU64>,
    /// Its end is fully handled: every waiter failed, `cut_off` final.
    finished: Arc<AtomicBool>,
    capacity: usize,
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
    /// Start reading `reader` (NDJSON) in the background, within the
    /// default [`PeerLimits`]; what isn't a reply arrives on the returned
    /// channel, which closes with the stream. Replies still awaited when
    /// it ends fail.
    pub fn spawn<R, W>(reader: R, writer: W) -> (Peer, mpsc::Receiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::spawn_with(reader, writer, PeerLimits::default())
    }

    /// [`Self::spawn`] within `limits`.
    pub fn spawn_with<R, W>(
        reader: R,
        writer: W,
        limits: PeerLimits,
    ) -> (Peer, mpsc::Receiver<Incoming>)
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
            ended: Arc::new(tokio::sync::Notify::new()),
            cut_off: Arc::new(AtomicU64::new(0)),
            finished: Arc::new(AtomicBool::new(false)),
            capacity: limits.channel_capacity,
        };
        let (tx, rx) = mpsc::channel(limits.channel_capacity);
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
            let mut reader = BufReader::new(reader);
            let mut buf = Vec::new();
            let mut why = "the peer closed".to_string();
            loop {
                let line = match read_line(&mut reader, &mut buf, limits.max_line_bytes).await {
                    Line::Text(line) => line,
                    Line::End => break,
                    Line::TooLong => {
                        why = format!(
                            "the peer sent a message over {} bytes; the connection is closed",
                            limits.max_line_bytes
                        );
                        break;
                    }
                };
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
                        let _ = tx.send(Incoming::Request { id, method, params }).await;
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
                                let _ = stream.send(message).await;
                            }
                            None => {
                                let _ = tx.send(message).await;
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
                responder.cut_off.fetch_add(1, Ordering::SeqCst);
                let _ = waiter.send(Err(ProtocolError::Internal(why.clone())));
            }
            // After the drain: who waits on the end sees what it cut off.
            responder.finished.store(true, Ordering::SeqCst);
            responder.ended.notify_waiters();
        });
        (peer, rx)
    }

    /// Once the other side's stream has ended and every reply it still
    /// owed has failed ([`Self::calls_cut_off`] is final).
    pub async fn closed(&self) {
        let ended = self.ended.notified();
        tokio::pin!(ended);
        ended.as_mut().enable();
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        ended.await;
    }

    /// How many replies still awaited when the other side's stream ended
    /// it failed: each caller had its own error.
    pub fn calls_cut_off(&self) -> u64 {
        self.cut_off.load(Ordering::SeqCst)
    }

    /// Whether `other` is a clone of this one: the same connection.
    pub fn same_connection(&self, other: &Peer) -> bool {
        Arc::ptr_eq(&self.closed, &other.closed)
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
    ) -> Result<(Call, mpsc::Receiver<Incoming>), ProtocolError> {
        let (tx, rx) = mpsc::channel(self.capacity);
        let call = self.start_with(method, params, Some(tx)).await?;
        Ok((call, rx))
    }

    async fn start_with(
        &self,
        method: &str,
        params: Value,
        stream: Option<mpsc::Sender<Incoming>>,
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
