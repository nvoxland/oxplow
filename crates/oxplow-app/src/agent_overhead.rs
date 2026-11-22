//! Content-free measurements for agent transport overhead.

use std::time::Instant;

pub(crate) struct Timer(Option<(Instant, Instant)>);

impl Timer {
    pub(crate) fn start() -> Self {
        Self(
            tracing::enabled!(target: "oxplow::agent_overhead", tracing::Level::DEBUG).then(|| {
                let now = Instant::now();
                (now, now)
            }),
        )
    }

    pub(crate) fn enabled(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn elapsed_us(&self) -> u64 {
        self.0
            .map_or(0, |(start, _)| start.elapsed().as_micros() as u64)
    }

    pub(crate) fn lap_us(&mut self) -> u64 {
        self.0.as_mut().map_or(0, |(_, previous)| {
            let now = Instant::now();
            let elapsed = now.duration_since(*previous).as_micros() as u64;
            *previous = now;
            elapsed
        })
    }
}
