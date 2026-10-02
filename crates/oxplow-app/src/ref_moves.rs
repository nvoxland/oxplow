//! Ref moves (P7.B6): a stream's HEAD or refs moved — a commit, a
//! checkout, a fetch — heard on the VCS watcher's own channel. The
//! backend's listeners (the commit indexer, the branch reconciler, a
//! stream's git-refs snapshot take) subscribe here; the renderer hears
//! the same move as `OxplowEvent::VcsRefsChanged` on the UI push bus.
//! Backend work never listens to the in-memory event bus.
//!
//! The channel is bounded: a listener that falls behind loses moves, and
//! hears [`Moved::Missed`] instead — any stream may have moved. Every
//! listener's work is idempotent, so it does it as if its stream(s) did.

use oxplow_domain::StreamId;
use tokio::sync::broadcast;

use crate::events::{EventBus, OxplowEvent};

/// The channel. A cheap handle: clone it.
#[derive(Clone)]
pub struct RefMoves {
    tx: broadcast::Sender<StreamId>,
    events: EventBus,
}

impl RefMoves {
    pub fn new(events: EventBus) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx, events }
    }

    /// `stream`'s refs moved: the backend hears it here, the UI as
    /// `VcsRefsChanged`.
    pub fn moved(&self, stream: StreamId) {
        let _ = self.tx.send(stream);
        self.events
            .emit(OxplowEvent::VcsRefsChanged { stream_id: stream });
    }

    pub fn subscribe(&self) -> Listener {
        Listener {
            rx: self.tx.subscribe(),
        }
    }
}

/// What a [`Listener`] hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// This stream's refs moved.
    Stream(StreamId),
    /// The listener fell behind and moves were dropped: any stream's refs
    /// may have moved.
    Missed,
}

impl Moved {
    /// Whether `stream`'s refs may have moved.
    pub fn may_concern(self, stream: StreamId) -> bool {
        match self {
            Moved::Stream(s) => s == stream,
            Moved::Missed => true,
        }
    }
}

/// One listener's end of the channel.
pub struct Listener {
    rx: broadcast::Receiver<StreamId>,
}

impl Listener {
    /// The next move; `None` once the channel closes.
    pub async fn recv(&mut self) -> Option<Moved> {
        match self.rx.recv().await {
            Ok(stream) => Some(Moved::Stream(stream)),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "a ref-moves listener fell behind");
                Some(Moved::Missed)
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_move_reaches_its_listeners_and_the_ui() {
        let bus = EventBus::new();
        let mut ui = bus.subscribe_ui();
        let moves = RefMoves::new(bus);
        let mut rx = moves.subscribe();
        moves.moved(StreamId::new(3));
        assert_eq!(rx.recv().await, Some(Moved::Stream(StreamId::new(3))));
        assert!(matches!(
            ui.try_recv().unwrap(),
            OxplowEvent::VcsRefsChanged { stream_id } if stream_id == StreamId::new(3)
        ));
    }

    /// P7 review (tsk724): a listener that fell behind hears that it
    /// missed moves — which may concern any stream — then the rest.
    #[tokio::test]
    async fn a_listener_that_fell_behind_hears_it_missed_moves() {
        let moves = RefMoves::new(EventBus::new());
        let mut rx = moves.subscribe();
        for _ in 0..300 {
            moves.moved(StreamId::new(1));
        }
        let missed = rx.recv().await.unwrap();
        assert_eq!(missed, Moved::Missed);
        assert!(missed.may_concern(StreamId::new(7)));
        assert_eq!(rx.recv().await, Some(Moved::Stream(StreamId::new(1))));
    }
}
