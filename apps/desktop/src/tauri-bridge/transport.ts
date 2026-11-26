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
// **Which daemon is a per-window question**, with one answer:
// `window.__OXPLOW__` (`resolveEndpoint`). The shell injects it into each
// window it creates (`src-tauri/src/windows.rs`), so two project windows in
// one shell process can drive two different daemons; the host injects it
// into each page it serves; the Vite dev server (`VITE_OXPLOW_REMOTE`) and
// the e2e harness inject it the same way. Read once at module load.
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
import { shellStorage } from "../shellStorage.js";
import { getUiClientId } from "../uiClientId.js";
import { getDevice } from "../device.js";
import { readWindowContext, type WindowContext } from "../windowContext.js";

/// Channels multiplexed over the daemon's /events WebSocket, keyed by
/// the wire `channel` value in each frame. Sourced from the shared
/// channel registry (see channels.ts) so the demux table can't drift
/// from what the daemon frames.
const REMOTE_CHANNELS: Record<string, string> = EVENT_CHANNELS;
const RESYNC_AFTER_MS = 1000;

/// Commands the desktop shell serves itself. Generated from
/// `SHELL_ONLY_COMMANDS` in crates/oxplow-tauri-ipc, so this table
/// can't drift from the Rust definition (the surface-parity test
/// asserts the generated file still matches).
const SHELL_COMMAND_SET: ReadonlySet<string> = new Set<string>(SHELL_COMMANDS);

export type { WindowContext };

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

/// The daemon base and token for this window. One read path: what started
/// the page says where its daemon is, in `window.__OXPLOW__` (the Mac shell
/// injects it into each window, the host into each page it serves, and the
/// Vite dev server and the e2e harness the same way). A page the host served
/// carries no token: a paired device uses its own (`paired`). No context, or
/// no base: no daemon (the launcher; Tauri IPC only). Pure; exported for
/// tests.
export function resolveEndpoint(
  ctx: Pick<WindowContext, "base" | "token" | "host"> | null,
  paired: string | null,
): { base: string | null; token: string | null } {
  const base = cleanBase(ctx?.base);
  if (base === null) return { base: null, token: null };
  const injected = ctx?.token?.trim() || null;
  const token = injected ?? (ctx?.host ? paired?.trim() || null : null);
  return { base, token };
}

/// Whether this window's files are on this machine: the shell names the
/// folder of a project it opened here (`projectDir`), and the launcher's
/// projects are here. A page a host served, or a window on another
/// machine's host, has none. Pure; exported for tests.
export function filesLocalFrom(sources: { shell: boolean; kind: string | null; projectDir: string | null }): boolean {
  if (!sources.shell) return false;
  if (sources.kind === "launcher") return true;
  return sources.kind === "project" && sources.projectDir !== null;
}

const endpoint = resolveEndpoint(
  readWindowContext(),
  readWindowContext()?.host ? shellStorage.getItem("oxplow.hostToken") : null,
);
const remoteBase: string | null = endpoint.base;
const remoteToken: string | null = endpoint.token;

/// The host's origin, when a host served this page (`window.__OXPLOW__.host`):
/// its API is there, and the page uses the device's own token.
export function hostOrigin(): string | null {
  return readWindowContext()?.host ?? null;
}

/// Whether this page must pair before anything else: a host served it and
/// this browser has no token for that host yet.
export function needsPairing(): boolean {
  return hostOrigin() !== null && !readWindowContext()?.token && hostToken() === null;
}

/// The token this device got at pairing with the host that served this
/// page, if any.
export function hostToken(): string | null {
  return shellStorage.getItem("oxplow.hostToken")?.trim() || null;
}

/// The token this device got at pairing, kept for the host's origin; the
/// page loads again to use it.
export function keepHostToken(token: string): void {
  shellStorage.setItem("oxplow.hostToken", token);
}

/// This device's token no longer works (it was removed, or the host forgot
/// it): forget it and load again, which pairs.
export function forgetHostToken(): void {
  shellStorage.removeItem("oxplow.hostToken");
  window.location.reload();
}

/// The served UI's build when this page loaded, from `/host/me`.
let loadedBuild: string | null | undefined;

// A page the host served holds its project from the start (`checkHost`).
if (hostOrigin() !== null && remoteToken !== null) queueMicrotask(() => void checkHost());

/// Ask the host who this device is, and hold this page's project for it. A
/// token it no longer knows means pair again (a browser cannot see the 401
/// on a socket's upgrade, so this is how a removed device finds out); a
/// build that changed since this page loaded means load again (the app
/// restarted with a new build). The hold is taken at load and before each
/// reconnect: a host that started again (the Mac app quit and opened)
/// holds only what its devices ask for, and holding twice is one hold.
async function checkHost(): Promise<void> {
  const host = hostOrigin();
  if (host === null || remoteToken === null) return;
  let answer: Response;
  try {
    answer = await fetch(`${host}/host/me`, { headers: authHeaders(remoteToken) });
  } catch {
    return; // the host is away; the socket keeps trying
  }
  const project = readWindowContext()?.project;
  if (answer.ok && project) {
    await fetch(`${host}/host/holds`, {
      method: "POST",
      headers: { ...authHeaders(remoteToken), "Content-Type": "application/json" },
      body: JSON.stringify({ project }),
    }).catch(() => undefined);
  }
  if (answer.status === 401) {
    forgetHostToken();
    return;
  }
  if (!answer.ok) return;
  const me = (await answer.json().catch(() => null)) as { build?: string | null } | null;
  const build = me?.build ?? null;
  if (loadedBuild === undefined) loadedBuild = build;
  else if (build !== null && build !== loadedBuild) window.location.reload();
}

/** The event socket says which device and page load it is (a browser
 *  cannot set headers on a WebSocket, so they ride in the query); it owns
 *  this page load's open runs until it closes. */
export function eventsUrl(
  base: string,
  token: string | null,
  device: { id: string; name: string; kind: string },
  connection: string,
): string {
  const url = new URL(`${base.replace(/^http/, "ws")}/events`);
  if (token) url.searchParams.set("token", token);
  url.searchParams.set("device", device.id);
  url.searchParams.set("connection", connection);
  url.searchParams.set("name", device.name);
  url.searchParams.set("kind", device.kind);
  return url.toString();
}

/// Headers every daemon request carries.
function authHeaders(token: string | null): Record<string, string> {
  return token ? { Authorization: `Bearer ${token}` } : {};
}

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

/// Where an `invoke(name)` goes: the desktop shell's Tauri IPC
/// ("tauri"), this window's daemon ("http"), or nowhere ("none").
///
/// With no daemon everything is the shell's. With one, shell commands
/// stay in the shell — and with no Tauri host at all (a plain browser
/// driving a daemon) a shell command has nowhere to go: no daemon serves
/// one, so it's refused here as the daemon would refuse it, with no
/// request made.
/// Pure; exported for tests.
export function invokeRoute(
  name: string,
  base: string | null,
  tauriAvailable: boolean,
): "tauri" | "http" | "none" {
  if (base === null) return "tauri";
  if (SHELL_COMMAND_SET.has(name)) return tauriAvailable ? "tauri" : "none";
  return "http";
}

/// Tauri-invoke-compatible entry point. The generated bindings import
/// this as `__TAURI_INVOKE`. Resolves with the command's data and
/// REJECTS with the IpcError object on failure — the same semantics
/// as `@tauri-apps/api/core`'s invoke, which the bindings' typedError
/// wrapper converts into the `{status, data|error}` envelope.
/// Calls a closing page makes: sent `keepalive`, so the browser finishes
/// them after the page is gone instead of cancelling them (WebKit reports
/// a cancelled fetch as an error the page threw).
const UNLOAD_CALLS = new Set(["unregister_client_host"]);

/// The POST that carries `name(args)` to the daemon. Pure; exported for tests.
export function ipcRequest(name: string, args: Record<string, unknown> | undefined, token: string | null): RequestInit {
  return {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...authHeaders(token),
      // Who is asking: the daemon reads both or neither.
      "Oxplow-Device": getDevice().id,
      "Oxplow-Connection": getUiClientId(),
    },
    body: JSON.stringify(args ?? null),
    keepalive: UNLOAD_CALLS.has(name),
  };
}

/// Calls whose latency a person feels: the switch of a stream. They have a
/// lane of their own (see `createLanes`). Keystrokes to a terminal aren't
/// calls: they ride the socket (`sendSocketFrame`).
export const PRIORITY_COMMANDS: ReadonlySet<string> = new Set(["switch_stream"]);

/// How many `/ipc` requests each lane keeps in flight. The webview holds
/// six HTTP/1.1 connections to the daemon; the lanes add up to that, so a
/// burst of reads (a refetch storm) can fill the bulk lane and never the
/// connections the priority lane has.
export const BULK_IN_FLIGHT = 4;
export const PRIORITY_IN_FLIGHT = 2;

/// Two first-in first-out lanes, each running at most its limit at once:
/// whatever waits in one never delays the other. Pure; exported for tests.
export function createLanes(limits: { bulk: number; priority: number }) {
  type Lane = { limit: number; running: number; waiting: Array<() => void> };
  const lanes: Record<"bulk" | "priority", Lane> = {
    bulk: { limit: limits.bulk, running: 0, waiting: [] },
    priority: { limit: limits.priority, running: 0, waiting: [] },
  };
  return function run<T>(priority: boolean, task: () => Promise<T>): Promise<T> {
    const lane = lanes[priority ? "priority" : "bulk"];
    return new Promise<T>((resolve, reject) => {
      const start = () => {
        lane.running += 1;
        task()
          .then(resolve, reject)
          .finally(() => {
            lane.running -= 1;
            lane.waiting.shift()?.();
          });
      };
      if (lane.running < lane.limit) start();
      else lane.waiting.push(start);
    });
  };
}

const inLane = createLanes({ bulk: BULK_IN_FLIGHT, priority: PRIORITY_IN_FLIGHT });

export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  const route = invokeRoute(name, remoteBase, shellAvailable());
  if (route === "tauri") return tauriInvoke<T>(name, args);
  if (route === "none") {
    throw { code: "NOT_FOUND", message: `${name} is the desktop shell's, and this window has none`, cause: null };
  }
  const base = remoteBase;
  const token = remoteToken;
  const resp = await inLane(PRIORITY_COMMANDS.has(name), () => fetch(`${base}/ipc/${name}`, ipcRequest(name, args, token)));
  if (resp.status === 401) {
    // The daemon refused this window's token: no retry fixes that.
    markTokenRefused();
    throw { code: "UNAUTHORIZED", message: TOKEN_REFUSED, cause: null };
  }
  if (!resp.ok) {
    // Transport-level failure (tunnel down, daemon restarting).
    throw { code: "TRANSPORT", message: `daemon http ${resp.status}`, cause: null };
  }
  const envelope = (await resp.json()) as
    | { status: "ok"; data: T }
    | { status: "error"; error: unknown };
  if (envelope.status === "ok") return envelope.data;
  // Another page load of this device took over: this window stops.
  if (isReplacedError(envelope.error)) markReplaced();
  throw envelope.error;
}

// ---------------------------------------------------------------------------
// Remote event stream: one shared WebSocket, demuxed by channel.

type Handler = (event: { payload: unknown }) => void;

const channelHandlers = new Map<string, Set<Handler>>();
let socket: WebSocket | null = null;
let reconnectDelayMs = 500;

/// Remote connection lifecycle, for the reconnect banner. "up" fires
/// on every successful (re)connect; "down" on every drop; "refused" once,
/// when the daemon refuses this window's token — after which the
/// socket isn't opened or retried, since no retry fixes a wrong token;
/// "replaced" once, when another page load of this device took over (the
/// daemon keeps one per device, `client_reports::devices`) — after which
/// the socket isn't opened or retried either.
/// Local mode never fires any.
export type RemoteConnectionState = "up" | "down" | "refused" | "replaced" | "ended";

/// The close code the daemon gives a replaced page load's socket
/// (`client_reports::devices::REPLACED_CLOSE_CODE`).
export const REPLACED_CLOSE_CODE = 4001;
/// The host is ending: the app that runs it quit (`oxplow_host::access`).
export const HOST_ENDING_CLOSE_CODE = 4002;
/// The host's owner removed this device: its token is gone.
export const DEVICE_REMOVED_CLOSE_CODE = 4003;

/// What a socket's close means: this page load was replaced, the daemon
/// refused the token, this device was removed (pair again), the host is
/// ending (say so, try again slowly), or a drop to wait out and retry.
/// Pure; exported for tests.
export function socketCloseMeaning(
  code: number,
  tokenRefused: boolean,
): "replaced" | "refused" | "removed" | "ended" | "retry" {
  if (code === REPLACED_CLOSE_CODE) return "replaced";
  if (code === DEVICE_REMOVED_CLOSE_CODE) return "removed";
  if (code === HOST_ENDING_CLOSE_CODE) return "ended";
  if (tokenRefused) return "refused";
  return "retry";
}

/// Whether a call's error says this page load was replaced.
export function isReplacedError(error: unknown): boolean {
  return typeof error === "object" && error !== null && (error as { code?: unknown }).code === "REPLACED";
}

let replaced = false;

/// Whether another page load of this device took over: this window asks
/// nothing more until it is loaded again.
export function isReplaced(): boolean {
  return replaced;
}

function markReplaced(): void {
  if (replaced) return;
  replaced = true;
  socket?.close();
  notifyConnectionState("replaced");
}

/// What a call says when the daemon refused this window's token.
export const TOKEN_REFUSED = "this window's token was refused; open it again";

let tokenRefused = false;

/// Whether the daemon refused this window's token: every call fails the
/// same way until the window reconnects with the right one.
export function isTokenRefused(): boolean {
  return tokenRefused;
}

function markTokenRefused(): void {
  // On a page a host served, a refused token is this device's, and it no
  // longer works (its owner removed it): pair again. A page with no token
  // yet (the pair screen is up) has nothing to forget.
  if (hostOrigin() !== null && !readWindowContext()?.token && remoteToken !== null) {
    forgetHostToken();
    return;
  }
  if (tokenRefused) return;
  tokenRefused = true;
  socket?.close();
  notifyConnectionState("refused");
}
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

/// Reconnect/resync handlers. Fired whenever the WS opens — the first
/// connect too, for hosts that loaded before it was there to hear events. Consumers re-hydrate the stores
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

/// The daemon lost frames on their way here: read everything again, once for
/// a burst of them. Exported for tests.
let resyncTimer: ReturnType<typeof setTimeout> | null = null;
export function scheduleResync(): void {
  if (resyncTimer !== null) return;
  resyncTimer = setTimeout(() => {
    resyncTimer = null;
    triggerRemoteResync();
  }, RESYNC_AFTER_MS);
}

/// The socket opened. Every open re-hydrates the hosts that registered
/// (`onRemoteReconnect`) — after a drop, for the events missed while it was
/// down, and on the first connect too: a host mounted before the socket was
/// open loaded a snapshot that may predate a write the socket wasn't yet
/// there to announce. Exported for tests.
export function handleSocketOpen(): void {
  reconnectDelayMs = 500;
  notifyConnectionState("up");
  for (const h of reconnectHandlers) h();
  void checkHost();
}

function ensureSocket(): void {
  if (remoteBase === null || socket !== null || tokenRefused || replaced) return;
  // Browsers can't set headers on a WebSocket; the token rides the query.
  const wsUrl = eventsUrl(remoteBase, remoteToken, getDevice(), getUiClientId());
  const ws = new WebSocket(wsUrl);
  socket = ws;
  // The daemon beats every 20 s; a socket that hears nothing for longer is
  // gone (a tablet that slept, a tunnel that dropped). It is dropped at
  // once, not closed and waited for: on a dead path the close handshake
  // takes a minute.
  const watchdog = createWatchdog(SOCKET_SILENCE_MS, () => dropSocket(ws, () => down(1006)));
  watchdog.feed();
  ws.onmessage = (msg) => {
    watchdog.feed();
    let frame: { channel?: string; payload?: unknown };
    try {
      frame = JSON.parse(String(msg.data));
    } catch {
      return;
    }
    if (frame.channel === "resync") {
      scheduleResync();
      return;
    }
    if (frame.channel === "beat") return;
    const local = frame.channel ? REMOTE_CHANNELS[frame.channel] : undefined;
    if (!local) return;
    const handlers = channelHandlers.get(local);
    if (!handlers) return;
    for (const h of handlers) h({ payload: frame.payload });
  };
  ws.onopen = () => {
    flushFrames(ws, waitingFrames);
    handleSocketOpen();
  };
  const down = (code: number) => {
    watchdog.stop();
    socket = null;
    waitingFrames.length = 0;
    // A refused token or a replaced page load isn't a drop: nothing to
    // wait out or retry.
    const meaning = socketCloseMeaning(code, tokenRefused);
    if (meaning === "replaced") {
      markReplaced();
      return;
    }
    if (meaning === "refused" || replaced) return;
    if (meaning === "removed") {
      forgetHostToken();
      return;
    }
    // The app that runs the host quit: say so, and wait for it longer.
    if (meaning === "ended") reconnectDelayMs = Math.max(reconnectDelayMs, 5_000);
    notifyConnectionState(meaning === "ended" ? "ended" : "down");
    if (meaning === "retry") void checkHost();
    // Backoff reconnect; the RemoteConnectionBanner drives the
    // user-facing state off the notifications above.
    if (channelHandlers.size > 0) {
      setTimeout(ensureSocket, reconnectDelayMs);
      reconnectDelayMs = Math.min(reconnectDelayMs * 2, meaning === "ended" ? 30_000 : 10_000);
    }
  };
  ws.onclose = (event) => down(event.code);
  ws.onerror = () => {
    ws.close();
  };
}

/// Take `ws` out of service now: nothing it says or does is heard again, it
/// closes in the background, and `gone` runs once (the reconnect). Exported
/// for tests.
export function dropSocket(
  ws: { onmessage: unknown; onopen: unknown; onclose: unknown; onerror: unknown; close(): void },
  gone: () => void,
): void {
  ws.onmessage = ws.onopen = ws.onclose = ws.onerror = null;
  try {
    ws.close();
  } catch {
    // Already gone.
  }
  gone();
}

/// How long the socket may hear nothing before the page takes it for gone
/// and reconnects: a little over two of the daemon's 20-second beats
/// (`oxplow-daemon` `Liveness`).
export const SOCKET_SILENCE_MS = 50_000;

/// A watchdog: `feed` it on every frame; `silent` runs once nothing came for
/// `ms`. `stop` ends it. Exported for tests (`timers` is theirs).
export function createWatchdog(
  ms: number,
  silent: () => void,
  timers: { set: (f: () => void, ms: number) => unknown; clear: (t: unknown) => void } = {
    set: (f, delay) => setTimeout(f, delay),
    clear: (t) => clearTimeout(t as ReturnType<typeof setTimeout>),
  },
) {
  let timer: unknown = null;
  const arm = () => {
    if (timer !== null) timers.clear(timer);
    timer = timers.set(() => {
      timer = null;
      silent();
    }, ms);
  };
  return {
    feed: arm,
    stop() {
      if (timer !== null) timers.clear(timer);
      timer = null;
    },
  };
}

/// What a frame is written on: the shared socket, or a test's stand-in.
type FrameSocket = { readyState: number; send(frame: string): void };

/// Frames written while the socket was still opening, sent once it opens.
const waitingFrames: string[] = [];

/// Write `frame` on `ws` now when it is open, hold it in `waiting` while it
/// opens, and drop it when there is no socket or it closed: the daemon is
/// gone, and reconnecting opens the terminal again. Pure; exported for
/// tests.
export function writeFrame(ws: FrameSocket | null, waiting: string[], frame: string): void {
  if (ws?.readyState === WebSocket.OPEN) ws.send(frame);
  else if (ws?.readyState === WebSocket.CONNECTING) waiting.push(frame);
}

/// The socket opened: send the frames held for it, in order. Exported for
/// tests.
export function flushFrames(ws: FrameSocket, waiting: string[]): void {
  for (const frame of waiting.splice(0)) ws.send(frame);
}

/// Write one frame to this window's daemon on the `/events` socket, after
/// every frame written before it: the person's terminal input
/// (`{"channel":"terminal","uri","message"}`), which must reach the PTY in
/// the order it was typed — separate `/ipc` requests could overtake one
/// another. Fire and forget: no answer comes back.
export function sendSocketFrame(frame: string): void {
  ensureSocket();
  writeFrame(socket, waitingFrames, frame);
}

/// The person used this window now (a press, a key): an agent's call with no
/// device of its own goes to the device used last. Sent on the open socket
/// only; one lost to a drop matters not, the next use tells it again.
export function reportUse(): void {
  if (socket?.readyState === 1) socket.send('{"channel":"used"}');
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

/// Whether this window is in the desktop shell (a Tauri host): false in a
/// plain browser driving a daemon. The one check every bridge uses.
export function shellAvailable(): boolean {
  try {
    return "__TAURI_INTERNALS__" in window;
  } catch {
    return false;
  }
}

/// Whether this window's files are on this machine (`filesLocalFrom`):
/// Open In ▸ Reveal and Default App need them. Read once.
export function filesAreLocal(): boolean {
  if (localFilesAtLoad === null) {
    const ctx = readWindowContext();
    localFilesAtLoad = filesLocalFrom({
      shell: shellAvailable(),
      kind: ctx?.kind ?? null,
      projectDir: ctx?.projectDir ?? null,
    });
  }
  return localFilesAtLoad;
}
let localFilesAtLoad: boolean | null = null;

/// Tauri-listen-compatible entry point. In remote mode the three
/// daemon-multiplexed channels read from the shared WebSocket; any
/// other channel (e.g. the native menu) still listens on the local
/// Tauri event bus — unless the page isn't hosted by the shell at
/// all (plain browser), where the subscription is inert.
export async function listen<T>(
  channel: ListenChannel,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  const route = listenRoute(channel, remoteBase, shellAvailable());
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
