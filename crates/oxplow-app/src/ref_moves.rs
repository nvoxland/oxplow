//! Ref moves (P7.B6): a stream's HEAD or refs moved — a commit, a
//! checkout, a fetch — heard on the VCS watcher's own channel. The
//! backend's listeners (the commit indexer, the branch reconciler, a
//! stream's git-refs snapshot take) subscribe here; the renderer hears
//! the same move as `OxplowEvent::VcsRefsChanged` on the UI push bus.
//! Backend work never listens to the in-memory event bus.

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

    pub fn subscribe(&self) -> broadcast::Receiver<StreamId> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_reaches_its_listeners_and_the_ui() {
        let bus = EventBus::new();
        let mut ui = bus.subscribe_ui();
        let moves = RefMoves::new(bus);
        let mut rx = moves.subscribe();
        moves.moved(StreamId::new(3));
        assert_eq!(rx.try_recv().unwrap(), StreamId::new(3));
        assert!(matches!(
            ui.try_recv().unwrap(),
            OxplowEvent::VcsRefsChanged { stream_id } if stream_id == StreamId::new(3)
        ));
    }
}
