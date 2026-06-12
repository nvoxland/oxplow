// Transport switch: local Tauri IPC vs remote daemon over HTTP/WS.
//
// Every command in the generated bindings funnels through `invoke`
// below (the bindings' import of `@tauri-apps/api/core` is rewritten
// to this module at export time), and every event subscription funnels
// through `listen`. In local mode both delegate straight to
// `@tauri-apps/api` — byte-for-byte today's behavior. In remote mode
// `invoke` POSTs to the daemon's `/ipc/:name` and `listen` reads the
// multiplexed `/events` WebSocket.
//
// Remote mode is selected by a base URL in either:
//   - localStorage "oxplow.remoteBase" (set by the connect UI), or
//   - VITE_OXPLOW_REMOTE at build/dev time (developer override).
// The value is read once at module load — switching modes is a
// reload-level decision, matching the process-per-project model.
//
// Channels that are inherently local to the shell (the native menu's
// "menu:command") use Tauri listen even in remote mode — except in a
// plain-browser session (no Tauri host at all), where they're inert.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { EVENT_CHANNELS } from "./channels";

/// Channels multiplexed over the daemon's /events WebSocket, keyed by
/// the wire `channel` value in each frame. Sourced from the shared
/// channel registry (see channels.ts) so the demux table can't drift
/// from what the daemon frames.
const REMOTE_CHANNELS: Record<string, string> = EVENT_CHANNELS;

function readRemoteBase(): string | null {
  try {
    const stored = window.localStorage.getItem("oxplow.remoteBase");
    if (stored && stored.trim().length > 0) return stored.trim().replace(/\/+$/, "");
  } catch {
    // localStorage unavailable (tests) — fall through.
  }
  const env = (import.meta as { env?: Record<string, string> }).env?.VITE_OXPLOW_REMOTE;
  if (env && env.trim().length > 0) return env.trim().replace(/\/+$/, "");
  return null;
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

/// Tauri-invoke-compatible entry point. The generated bindings import
/// this as `__TAURI_INVOKE`. Resolves with the command's data and
/// REJECTS with the IpcError object on failure — the same semantics
/// as `@tauri-apps/api/core`'s invoke, which the bindings' typedError
/// wrapper converts into the `{status, data|error}` envelope.
export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (remoteBase === null) {
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
  };
  ws.onclose = () => {
    socket = null;
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
/// no Tauri host exists and the event can never fire). Pure; exported
/// for tests.
export function listenRoute(
  channel: string,
  base: string | null,
  tauriAvailable: boolean,
): "ws" | "tauri" | "none" {
  const isMultiplexed = Object.values(REMOTE_CHANNELS).includes(channel);
  if (base !== null && isMultiplexed) return "ws";
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
  channel: string,
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
