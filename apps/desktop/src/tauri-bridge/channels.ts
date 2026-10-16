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

// Routing classification for every channel `listen()` accepts. This is
// the single source of truth `listenRoute` switches on — adding a
// channel here (with its routing) is what makes `listen()` accept it;
// an unclassified channel name is a compile error at the call site.
//
//  - "multiplexed": bridged over the daemon's /events WebSocket in
//    remote mode, and the local Tauri event bus otherwise. These are
//    exactly the EVENT_CHANNELS values (asserted in channels.test.ts).
//  - "shellLocal": the Tauri event bus only — native shell features
//    like the menu. Never multiplexed; resolves to "none" (inert) in a
//    plain-browser session where no Tauri host exists, instead of
//    throwing on __TAURI_INTERNALS__.
export type ChannelRouting = "multiplexed" | "shellLocal";

export const CHANNEL_ROUTING = {
  [EVENT_CHANNELS.oxplow]: "multiplexed",
  [EVENT_CHANNELS.lsp]: "multiplexed",
  [EVENT_CHANNELS.terminal]: "multiplexed",
  "menu:command": "shellLocal",
} as const satisfies Record<string, ChannelRouting>;

/// The union of every channel `listen()` accepts. Derived from the
/// registry keys, so a channel can't be listened on without first being
/// classified above.
export type ListenChannel = keyof typeof CHANNEL_ROUTING;
