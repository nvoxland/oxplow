// Frontend facade over the backend IPC.
//
// Re-exports the tauri-specta-generated bindings (every Rust command
// becomes a typed async TS function) plus a small ergonomics layer
// for events. UI code imports from here, never directly from
// @tauri-apps/api — the actual transport (local Tauri invoke/listen
// vs a remote daemon over HTTP/WebSocket) is selected in
// ./transport.ts, so neither the bindings' call sites nor the UI
// change between modes.

import { commands, type OxplowEvent } from "./generated/bindings";

export { commands };
export * as oxplow from "./generated/bindings";

// Re-export every type the renderer reaches for. The bindings file
// has ~99 types; we re-export them all so call sites can do
// `import type { X } from "../tauri-bridge"` without first checking
// whether X is on the export list.
export type * from "./generated/bindings";

/// Discriminant kinds for the cross-store event bus. Derived from the
/// specta-generated `OxplowEvent` union, so a new Rust variant lands
/// here automatically when the bindings regenerate — no hand-synced
/// list to forget (the old "camelcase trap").
export type OxplowEventKind = OxplowEvent["kind"];

// `subscribeOxplowEvents` lives in `api.ts` — deliberately ONE implementation
// (tsk221). A second copy used to sit here and differed in two ways that matter:
// it had no `stopped` guard, so a caller that didn't await its async teardown
// could still be delivered an event after unmount; and it typed the payload as
// `{ kind } & Record<string, unknown>`, discarding the discriminated union that
// lets consumers narrow on `kind` (tsk198 had to hand-narrow to read `measures`
// because of exactly that). Both are gone — add new bus helpers next to the
// `api.ts` one, per `.context/ipc-and-stores.md`.
