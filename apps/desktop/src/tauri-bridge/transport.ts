// Transport switch: the desktop shell (Tauri IPC) vs a daemon over
// HTTP/WS.
//
// Every command in the generated bindings funnels through `invoke`
// below (the bindings' import of `@tauri-apps/api/core` is rewritten
// to this module at export time), and every event subscription funnels
// through `listen`. With no daemon base both delegate straight to
// `@tauri-apps/api`. With one, `invoke` POSTs to the daemon's
// `/ipc/:name` and `listen` reads the multiplexed `/events` WebSocket.
//
// **Which daemon is a per-window question.** The shell injects
// `window.__OXPLOW__ = { base, kind }` into each window it creates
// (`src-tauri/src/windows.rs`), so two project windows in one shell
// process can drive two different daemons. localStorage and
// VITE_OXPLOW_REMOTE remain as overrides — see `resolveBase`. All three
// are read once at module load; switching is a reload-level decision.
//
// **Routing is per command, not all-or-nothing.** Some commands only
// exist in the shell (windowing, native menus, clipboard, project
// lifecycle) and no daemon serves them; `SHELL_COMMANDS` is generated
// from the Rust definition so the two can't drift. Everything else goes
// to this window's daemon when it has one.
//
// Channels that are inherently local to the shell (the native menu's
// "menu:command") use Tauri listen even with a daemon — except in a
// plain-browser session (no Tauri host at all), where they're inert.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { CHANNEL_ROUTING, EVENT_CHANNELS, type ListenChannel } from "./channels";
import { SHELL_COMMANDS } from "./generated/shellCommands";

/// Channels multiplexed over the daemon's /events WebSocket, keyed by
/// the wire `channel` value in each frame. Sourced from the shared
/// channel registry (see channels.ts) so the demux table can't drift
/// from what the daemon frames.
const REMOTE_CHANNELS: Record<string, string> = EVENT_CHANNELS;

/// Commands the desktop shell serves itself. Generated from
/// `SHELL_ONLY_COMMANDS` in crates/oxplow-tauri-ipc, so this table
/// can't drift from the Rust definition (the surface-parity test
/// asserts the generated file still matches).
const SHELL_COMMAND_SET: ReadonlySet<string> = new Set<string>(SHELL_COMMANDS);

/// What the shell injects into every window it creates, before any page
/// script runs. Absent in a plain-browser session.
export type WindowContext = {
  base: string | null;
  kind: string;
  projectDir: string | null;
};

function readWindowContext(): WindowContext | null {
  try {
    const injected = (globalThis as { __OXPLOW__?: WindowContext }).__OXPLOW__;
    if (injected && typeof injected === "object") return injected;
  } catch {
    // No window object (bun tests) — fall through.
  }
  return null;
}

/// Which kind of window this renderer is in ("project" | "launcher" |
/// "setup"), or null when the shell didn't create it (a plain browser
/// driving a daemon).
export function windowKind(): string | null {
  return readWindowContext()?.kind ?? null;
}

/// The project this window is for: the one it has open, or — on the
/// setup screen — the one being offered for creation.
export function windowProjectDir(): string | null {
  return readWindowContext()?.projectDir ?? null;
}

function cleanBase(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim().replace(/\/+$/, "");
  return trimmed.length > 0 ? trimmed : null;
}

/// The daemon base for this window, in precedence order:
///
/// 1. **localStorage** — a manual "connect to remote daemon" from the
///    launcher. An explicit user action outranks the default.
/// 2. **The shell's injected base** — the normal path once windows own
///    daemons.
/// 3. **VITE_OXPLOW_REMOTE** — build/dev-time developer override.
///
/// null means "no daemon": talk to the shell over Tauri IPC.
/// Pure; exported for tests.
export function resolveBase(sources: {
  stored?: string | null;
  injected?: string | null;
  env?: string | null;
}): string | null {
  return cleanBase(sources.stored) ?? cleanBase(sources.injected) ?? cleanBase(sources.env) ?? null;
}

function readRemoteBase(): string | null {
  let stored: string | null = null;
  try {
    stored = window.localStorage.getItem("oxplow.remoteBase");
  } catch {
    // localStorage unavailable (tests) — leave it null.
  }
  const env = (import.meta as { env?: Record<string, string> }).env?.VITE_OXPLOW_REMOTE ?? null;
  return resolveBase({ stored, injected: readWindowContext()?.base ?? null, env });
}

const remoteBase: string | null = readRemoteBase();

/// True when this renderer drives a remote daemon instead of the
/// in-process backend.
export function isRemote(): boolean {
  return remoteBase !== null;
}

/// The remote daemon's HTTP base (e.g. "http://127.0.0.1:7420"), or
/// null in local mode.
export function remoteBaseUrl(): string | null {
  return remoteBase;
}

/// Persist a remote daemon base and reload into remote mode. The
/// transport reads the key once at module load, so a reload is the
/// mode switch.
export function connectRemote(base: string): void {
  window.localStorage.setItem("oxplow.remoteBase", base.trim().replace(/\/+$/, ""));
  window.location.reload();
}

/// Drop the remote base and reload back into local mode.
export function disconnectRemote(): void {
  window.localStorage.removeItem("oxplow.remoteBase");
  window.location.reload();
}

/// Probe a daemon base before committing to it: POST /ipc/ping and
/// expect the ok envelope. Throws with a human-readable message on
/// any failure (unreachable, non-JSON, wrong service).
export async function probeRemoteDaemon(base: string, timeoutMs = 4000): Promise<void> {
  const cleaned = base.trim().replace(/\/+$/, "");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const resp = await fetch(`${cleaned}/ipc/ping`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "null",
      signal: controller.signal,
    });
    if (!resp.ok) throw new Error(`daemon replied HTTP ${resp.status}`);
    const envelope = (await resp.json()) as { status?: string; data?: unknown };
    if (envelope.status !== "ok" || envelope.data !== "pong") {
      throw new Error("endpoint is not an oxplow daemon");
    }
  } catch (e) {
    if (e instanceof DOMException && e.name === "AbortError") {
      throw new Error(`no response within ${timeoutMs / 1000}s — is the tunnel up?`);
    }
    throw e instanceof Error ? e : new Error(String(e));
  } finally {
    clearTimeout(timer);
  }
}

/// Where an `invoke(name)` goes: the desktop shell's Tauri IPC
/// ("tauri") or this window's daemon ("http").
///
/// With no daemon everything is the shell's. With one, shell commands
/// stay in the shell — unless there is no Tauri host at all (a plain
/// browser driving a daemon), where the only reachable backend is the
/// daemon; it answers with a structured "unknown command" rather than
/// the renderer throwing on a missing `__TAURI_INTERNALS__`.
/// Pure; exported for tests.
export function invokeRoute(
  name: string,
  base: string | null,
  tauriAvailable: boolean,
): "tauri" | "http" {
  if (base === null) return "tauri";
  if (SHELL_COMMAND_SET.has(name) && tauriAvailable) return "tauri";
  return "http";
}

/// Tauri-invoke-compatible entry point. The generated bindings import
/// this as `__TAURI_INVOKE`. Resolves with the command's data and
/// REJECTS with the IpcError object on failure — the same semantics
/// as `@tauri-apps/api/core`'s invoke, which the bindings' typedError
/// wrapper converts into the `{status, data|error}` envelope.
export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (invokeRoute(name, remoteBase, tauriHostAvailable()) === "tauri") {
    return tauriInvoke<T>(name, args);
  }
  const resp = await fetch(`${remoteBase}/ipc/${name}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? null),
  });
  if (!resp.ok) {
    // Transport-level failure (tunnel down, daemon restarting).
    throw { code: "TRANSPORT", message: `daemon http ${resp.status}`, cause: null };
  }
  const envelope = (await resp.json()) as
    | { status: "ok"; data: T }
    | { status: "error"; error: unknown };
  if (envelope.status === "ok") return envelope.data;
  throw envelope.error;
}

// ---------------------------------------------------------------------------
// Remote event stream: one shared WebSocket, demuxed by channel.

type Handler = (event: { payload: unknown }) => void;

const channelHandlers = new Map<string, Set<Handler>>();
let socket: WebSocket | null = null;
let reconnectDelayMs = 500;

/// Remote connection lifecycle, for the reconnect banner. "up" fires
/// on every successful (re)connect; "down" on every drop. Local mode
/// never fires either.
export type RemoteConnectionState = "up" | "down";
const connectionStateHandlers = new Set<(s: RemoteConnectionState) => void>();

export function onRemoteConnectionState(
  handler: (state: RemoteConnectionState) => void,
): () => void {
  connectionStateHandlers.add(handler);
  return () => connectionStateHandlers.delete(handler);
}

function notifyConnectionState(state: RemoteConnectionState): void {
  for (const h of connectionStateHandlers) h(state);
}

/// Reconnect/resync handlers. Fired when the WS comes back *after* a
/// drop (not on the initial connect). Consumers re-hydrate the stores
/// they hold here so state goes live again with no manual page reload —
/// the WS itself auto-re-subscribes (channelHandlers persist across the
/// drop), this covers the events missed while it was down. Local mode
/// never fires.
const reconnectHandlers = new Set<() => void>();

/// Register a resync callback for WS reconnect (and manual
/// `triggerRemoteResync`). Returns an unsubscribe.
export function onRemoteReconnect(handler: () => void): () => void {
  reconnectHandlers.add(handler);
  return () => reconnectHandlers.delete(handler);
}

/// Fire every reconnect handler now. Used by the daemon health-probe
/// recovery path (App.tsx) so an HTTP-level recovery resyncs the same
/// stores a WS reconnect would, instead of forcing a full page reload.
export function triggerRemoteResync(): void {
  for (const h of reconnectHandlers) h();
}

/// True once the socket has dropped; reset after the next open fires the
/// reconnect handlers. Distinguishes a reconnect from the first connect.
let wasDown = false;

function ensureSocket(): void {
  if (remoteBase === null || socket !== null) return;
  const wsUrl = `${remoteBase.replace(/^http/, "ws")}/events`;
  const ws = new WebSocket(wsUrl);
  socket = ws;
  ws.onmessage = (msg) => {
    let frame: { channel?: string; payload?: unknown };
    try {
      frame = JSON.parse(String(msg.data));
    } catch {
      return;
    }
    const local = frame.channel ? REMOTE_CHANNELS[frame.channel] : undefined;
    if (!local) return;
    const handlers = channelHandlers.get(local);
    if (!handlers) return;
    for (const h of handlers) h({ payload: frame.payload });
  };
  ws.onopen = () => {
    reconnectDelayMs = 500;
    notifyConnectionState("up");
    // A reconnect after a drop — re-hydrate the stores to catch up on
    // events missed while the socket was down. Not fired on the first
    // connect (App's mount loader already does the initial hydration).
    if (wasDown) {
      wasDown = false;
      for (const h of reconnectHandlers) h();
    }
  };
  ws.onclose = () => {
    socket = null;
    wasDown = true;
    notifyConnectionState("down");
    // Backoff reconnect; the RemoteConnectionBanner drives the
    // user-facing state off the notifications above.
    if (channelHandlers.size > 0) {
      setTimeout(ensureSocket, reconnectDelayMs);
      reconnectDelayMs = Math.min(reconnectDelayMs * 2, 10_000);
    }
  };
  ws.onerror = () => {
    ws.close();
  };
}

/// Where a `listen(channel)` subscription routes: the daemon
/// WebSocket ("ws"), the local Tauri event bus ("tauri"), or nowhere
/// ("none" — a shell-local channel in a plain-browser session, where
/// no Tauri host exists and the event can never fire). Switches on the
/// channel's classification in CHANNEL_ROUTING (channels.ts), not on a
/// runtime membership check. Pure; exported for tests.
export function listenRoute(
  channel: ListenChannel,
  base: string | null,
  tauriAvailable: boolean,
): "ws" | "tauri" | "none" {
  if (base !== null && CHANNEL_ROUTING[channel] === "multiplexed") return "ws";
  return tauriAvailable ? "tauri" : "none";
}

function tauriHostAvailable(): boolean {
  try {
    return "__TAURI_INTERNALS__" in window;
  } catch {
    return false;
  }
}

/// Tauri-listen-compatible entry point. In remote mode the three
/// daemon-multiplexed channels read from the shared WebSocket; any
/// other channel (e.g. the native menu) still listens on the local
/// Tauri event bus — unless the page isn't hosted by the shell at
/// all (plain browser), where the subscription is inert.
export async function listen<T>(
  channel: ListenChannel,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  const route = listenRoute(channel, remoteBase, tauriHostAvailable());
  if (route === "none") return () => {};
  if (route === "tauri") {
    return tauriListen<T>(channel, handler);
  }
  let handlers = channelHandlers.get(channel);
  if (!handlers) {
    handlers = new Set();
    channelHandlers.set(channel, handlers);
  }
  const h = handler as Handler;
  handlers.add(h);
  ensureSocket();
  return () => {
    handlers.delete(h);
    if (handlers.size === 0) channelHandlers.delete(channel);
  };
}
