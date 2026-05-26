import { desktopBridge } from "./api.js";

export type UiLogLevel = "debug" | "info" | "warn" | "error";

const CLIENT_ID_KEY = "oxplow-ui-client-id";

let uninstall: (() => void) | null = null;

// --- Main-thread stall watchdog ---------------------------------------
// WKWebView (macOS Tauri) doesn't support PerformanceObserver's
// 'longtask' entry type, so we detect freezes by timer drift instead:
// schedule a tick every STALL_TICK_MS and compare the actual elapsed
// time against what we asked for. A large positive delta means the event
// loop was blocked for ~that long — the "felt frozen, nothing thrown"
// case that otherwise leaves no trace in the log.
const STALL_TICK_MS = 1000;
const STALL_THRESHOLD_MS = 1000;

// Coarse "what was on screen" context, pushed from the app so a stall
// WARN can name the active page + open file without coupling the logger
// to React state. Replaced wholesale on each update; read at tick time.
let uiLogContext: Record<string, unknown> = {};

/** Set the context attached to stall-watchdog warnings (active page id,
 *  open file path, …). Called by the app when navigation changes. */
export function setUiLogContext(context: Record<string, unknown>): void {
  uiLogContext = context;
}

export interface StallEval {
  /** True when the gap is a real stall worth logging. */
  stalled: boolean;
  /** Milliseconds the main thread was blocked beyond the scheduled tick. */
  blockedMs: number;
}

/**
 * Pure stall decision for one watchdog tick. `blockedMs` is how much
 * longer than expected the tick took; it's a stall only when that
 * exceeds the threshold AND the document wasn't hidden during the gap
 * (backgrounded windows throttle timers, which would otherwise look like
 * a multi-second freeze).
 */
export function evaluateStall(opts: {
  actualElapsedMs: number;
  expectedMs: number;
  thresholdMs: number;
  wasHidden: boolean;
}): StallEval {
  const blockedMs = opts.actualElapsedMs - opts.expectedMs;
  return { stalled: !opts.wasHidden && blockedMs >= opts.thresholdMs, blockedMs };
}

function getDoc(): Document | undefined {
  return (globalThis as { document?: Document }).document;
}

function docHidden(): boolean {
  const d = getDoc();
  return !!d && d.visibilityState !== undefined && d.visibilityState !== "visible";
}

/** Start the timer-drift watchdog. Returns a stop function. */
function startStallWatchdog(): () => void {
  let lastTick = Date.now();
  let wasHidden = docHidden();
  let timer: ReturnType<typeof setTimeout> | null = null;

  const onVisibility = () => {
    // Latch hidden-ness so a gap that spanned any hidden interval is
    // discounted even if the doc is visible again by the next tick.
    if (docHidden()) wasHidden = true;
  };
  const doc = getDoc();
  doc?.addEventListener?.("visibilitychange", onVisibility);

  const tick = () => {
    const now = Date.now();
    const { stalled, blockedMs } = evaluateStall({
      actualElapsedMs: now - lastTick,
      expectedMs: STALL_TICK_MS,
      thresholdMs: STALL_THRESHOLD_MS,
      wasHidden,
    });
    lastTick = now;
    wasHidden = docHidden();
    if (stalled) {
      void sendUiLog("warn", "main thread stalled", { blockedMs, ...uiLogContext });
    }
    timer = globalThis.setTimeout(tick, STALL_TICK_MS);
  };
  timer = globalThis.setTimeout(tick, STALL_TICK_MS);

  return () => {
    if (timer != null) globalThis.clearTimeout(timer);
    timer = null;
    doc?.removeEventListener?.("visibilitychange", onVisibility);
  };
}

/**
 * Patch `console.*` and register global error listeners so UI logs reach
 * the backend. Idempotent: a second call while already installed is a
 * no-op that returns the existing uninstall fn. Returns an uninstall
 * function that restores the original console methods and removes the
 * window listeners — call it on teardown (the app installs once for its
 * lifetime, but tests mount/unmount and must not leak listeners or a
 * patched console between cases).
 */
export function installUiLogging(): () => void {
  if (uninstall) return uninstall;

  const original = {
    log: console.log.bind(console),
    info: console.info.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
  };

  console.log = (...args: unknown[]) => {
    original.log(...args);
    void sendUiLog("info", "console.log", { args: args.map(serializeValue) });
  };
  console.info = (...args: unknown[]) => {
    original.info(...args);
    void sendUiLog("info", "console.info", { args: args.map(serializeValue) });
  };
  console.warn = (...args: unknown[]) => {
    original.warn(...args);
    void sendUiLog("warn", "console.warn", { args: args.map(serializeValue) });
  };
  console.error = (...args: unknown[]) => {
    original.error(...args);
    void sendUiLog("error", "console.error", { args: args.map(serializeValue) });
  };

  const onError = (event: ErrorEvent) => {
    void sendUiLog("error", "window.error", {
      message: event.message,
      filename: event.filename,
      lineno: event.lineno,
      colno: event.colno,
    });
  };

  const onRejection = (event: PromiseRejectionEvent) => {
    if (isMonacoCancellation(event.reason)) {
      event.preventDefault();
      return;
    }
    void sendUiLog("error", "window.unhandledrejection", {
      reason: serializeValue(event.reason),
    });
  };

  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);

  const stopStallWatchdog = startStallWatchdog();

  uninstall = () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
    stopStallWatchdog();
    console.log = original.log;
    console.info = original.info;
    console.warn = original.warn;
    console.error = original.error;
    uninstall = null;
  };

  void sendUiLog("info", "ui logging installed", {
    clientId: getUiClientId(),
    href: location.href,
  });

  return uninstall;
}

/** Tear down whatever `installUiLogging` set up. No-op if not installed. */
export function uninstallUiLogging(): void {
  uninstall?.();
}

export function logUi(level: UiLogLevel, message: string, context?: Record<string, unknown>): void {
  void sendUiLog(level, message, context);
}

export function getUiClientId(): string {
  try {
    const existing = sessionStorage.getItem(CLIENT_ID_KEY);
    if (existing) return existing;
    const id = globalThis.crypto?.randomUUID?.() ?? `client-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    sessionStorage.setItem(CLIENT_ID_KEY, id);
    return id;
  } catch {
    return `client-${Date.now()}`;
  }
}

async function sendUiLog(level: UiLogLevel, message: string, context?: Record<string, unknown>): Promise<void> {
  try {
    await desktopBridge().logUi({
      clientId: getUiClientId(),
      level,
      message,
      context,
      timestamp: new Date().toISOString(),
    });
  } catch {}
}

// Monaco aborts in-flight model/tokenizer/code-lens work by rejecting
// internal promises with a `Canceled` error when an editor or model
// gets disposed. Those rejections aren't bugs — they're just lifecycle
// noise — but they bubble up to `unhandledrejection`, polluting the
// error log. Filter them out at the listener.
function isMonacoCancellation(reason: unknown): boolean {
  if (!reason || typeof reason !== "object") return false;
  const r = reason as { name?: unknown; message?: unknown };
  return r.name === "Canceled" || r.name === "CancellationError" || r.message === "Canceled";
}

function serializeValue(value: unknown): unknown {
  if (value instanceof Error) {
    return { name: value.name, message: value.message, stack: value.stack };
  }
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || value === null) {
    return value;
  }
  if (value === undefined) return "undefined";
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    return String(value);
  }
}
