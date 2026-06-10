// Event channel registry — the renderer-side mirror of
// `oxplow_app::event_channels` (crates/oxplow-app/src/events.rs).
//
// Keys are the frame keys the daemon's `/events` WebSocket uses in
// `{"channel": <key>, "payload": …}`; values are the channel names the
// Tauri shell `app.emit`s on (and that `listen()` subscribes to in
// both modes). The surface-parity test (crates/oxplow-surface-parity/
// tests/parity.rs, `event_channels_match_typescript`) fails the build
// if this map and the Rust registry diverge — update both together.
export const EVENT_CHANNELS = {
  oxplow: "oxplow:event",
  lsp: "lsp:event",
  terminal: "terminal:event",
} as const;

export type EventChannelFrameKey = keyof typeof EVENT_CHANNELS;
export type EventChannelName = (typeof EVENT_CHANNELS)[EventChannelFrameKey];
