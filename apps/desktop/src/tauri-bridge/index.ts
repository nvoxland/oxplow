// Frontend facade over the backend IPC.
//
// Re-exports the tauri-specta-generated bindings (every Rust command
// becomes a typed async TS function) plus a small ergonomics layer
// for events. UI code imports from here, never directly from
// @tauri-apps/api — the actual transport (local Tauri invoke/listen
// vs a remote daemon over HTTP/WebSocket) is selected in
// ./transport.ts, so neither the bindings' call sites nor the UI
// change between modes.

import { listen } from "./transport";
import { commands } from "./generated/bindings";

export { commands };
export * as oxplow from "./generated/bindings";

// Re-export every type the renderer reaches for. The bindings file
// has ~99 types; we re-export them all so call sites can do
// `import type { X } from "../tauri-bridge"` without first checking
// whether X is on the export list.
export type * from "./generated/bindings";

/// Discriminant kinds for the cross-store event bus. Mirrors the
/// `OxplowEvent` enum on the Rust side.
export type OxplowEventKind =
  | "streamsChanged"
  | "currentStreamChanged"
  | "threadsChanged"
  | "selectedThreadChanged"
  | "tasksChanged"
  | "workNotesChanged"
  | "commentsChanged"
  | "wikiPagesChanged"
  | "followupsChanged"
  | "backgroundTasksChanged"
  | "hookEventsChanged"
  | "agentStatusChanged"
  | "agentTurnsChanged"
  | "pageVisitChanged"
  | "usageRecorded"
  | "fileSnapshotCreated"
  | "codeQualityScanned"
  | "effortObservationsChanged"
  | "configChanged"
  | "workspaceContextChanged"
  | "workspaceChanged"
  | "gitRefsChanged"
  | "streamOrphaned";

/// Subscribe to all oxplow events on the cross-store bus. Returns an
/// unlisten callback. Each event is the raw `OxplowEvent` payload —
/// the renderer normally branches on the `kind` field and refetches
/// the affected bucket via the matching `commands.*` call.
export function subscribeOxplowEvents(
  onEvent: (event: { kind: OxplowEventKind } & Record<string, unknown>) => void,
): () => Promise<void> {
  let cleanup: (() => void) | null = null;
  const promise = listen<{ kind: OxplowEventKind } & Record<string, unknown>>(
    "oxplow:event",
    (e) => {
      onEvent(e.payload);
    },
  ).then((un) => {
    cleanup = un;
  });
  return async () => {
    await promise;
    cleanup?.();
  };
}

/// Filtered helper: only fire `onEvent` for events matching `kinds`.
export function subscribeOxplowEventsOfKind(
  kinds: OxplowEventKind[],
  onEvent: (event: { kind: OxplowEventKind } & Record<string, unknown>) => void,
): () => Promise<void> {
  return subscribeOxplowEvents((event) => {
    if (kinds.includes(event.kind)) onEvent(event);
  });
}
