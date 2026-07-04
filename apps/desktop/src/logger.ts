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

// --- Self-timed operations --------------------------------------------
// The stall watchdog above fires only *after* a freeze ends, so the
// blocking code is already off the stack — it can name a duration, never a
// culprit. `timed()` closes that gap: wrap a synchronous operation and it
// logs a `slow operation` warn naming the op + its inputs the moment it
// runs long. Works everywhere (unlike PerformanceObserver longtask/LoAF,
// which WKWebView doesn't support).

/** A synchronous operation slower than this warrants a `slow operation` warn. */
export const SLOW_OP_THRESHOLD_MS = 50;

function nowMs(): number {
  return globalThis.performance?.now?.() ?? Date.now();
}

/** Whether a completed operation of `durMs` is slow enough to warn about. */
export function isSlowOperation(durMs: number, thresholdMs: number = SLOW_OP_THRESHOLD_MS): boolean {
  return durMs >= thresholdMs;
}

/**
 * Time a synchronous `fn`; emit a `slow operation` warn when it runs longer
 * than `thresholdMs`. Returns `fn`'s result (and rethrows on throw, timing
 * the partial run) so it wraps an expression transparently:
 * `const x = timed("label", () => expensive())`. `context` is evaluated only
 * on the slow path so attaching diagnostics (buffer size, item counts) costs
 * nothing on the fast path. `now` is injectable for tests.
 */
export function timed<T>(
  label: string,
  fn: () => T,
  opts?: { thresholdMs?: number; context?: () => Record<string, unknown>; now?: () => number },
): T {
  const clock = opts?.now ?? nowMs;
  const start = clock();
  try {
    return fn();
  } finally {
    const durMs = Math.round(clock() - start);
    if (isSlowOperation(durMs, opts?.thresholdMs ?? SLOW_OP_THRESHOLD_MS)) {
      void sendUiLog("warn", "slow operation", {
        label,
        durMs,
        ...(opts?.context?.() ?? {}),
        ...uiLogContext,
      });
    }
  }
}

interface LoafScript {
  sourceURL?: string;
  sourceFunctionName?: string;
  duration?: number;
  invoker?: string;
}

/**
 * Flatten a `long-animation-frame` entry's script attribution into a compact
 * loggable shape: absent fields become `undefined`, durations round to whole
 * ms. LoAF `scripts[]` is what makes a long frame actionable — it names the
 * source URL + function that ran long.
 */
export function describeLoafScripts(entry: { scripts?: LoafScript[] }): Array<{
  src: string | undefined;
  fn: string | undefined;
  durMs: number;
  invoker: string | undefined;
}> {
  return (entry.scripts ?? []).map((s) => ({
    src: s.sourceURL || undefined,
    fn: s.sourceFunctionName || undefined,
    durMs: Math.round(s.duration ?? 0),
    invoker: s.invoker || undefined,
  }));
}

function getDoc(): Document | undefined {
  return (globalThis as { document?: Document }).document;
}

function docHidden(): boolean {
  const d = getDoc();
  return !!d && d.visibilityState !== undefined && d.visibilityState !== "visible";
}

/** Total element count in the document — a cheap DOM-bloat signal attached
 *  to stall warnings (a huge tree makes layout/paint the likely blocker). */
function countDomNodes(): number | null {
  const d = getDoc();
  try {
    return d?.getElementsByTagName?.("*").length ?? null;
  } catch {
    return null;
  }
}

/** Start the timer-drift watchdog. Returns a stop function. */
function startStallWatchdog(): () => void {
  let lastTick = Date.now();
  let wasHidden = docHidden();
  let consecutiveStalls = 0;
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
      consecutiveStalls += 1;
      // consecutiveStalls separates a one-off hitch from sustained churn
      // (e.g. a per-frame render looping); domNodes flags DOM bloat.
      void sendUiLog("warn", "main thread stalled", {
        blockedMs,
        consecutiveStalls,
        domNodes: countDomNodes(),
        ...uiLogContext,
      });
    } else {
      consecutiveStalls = 0;
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
 * Attach `PerformanceObserver`s for long tasks and long animation frames
 * (LoAF) when the platform supports them. WKWebView (the macOS Tauri
 * target) supports neither, so this is a no-op there and the drift watchdog
 * remains the only signal — but in a dev browser (or a future WebKit) LoAF
 * `scripts[]` gives real per-script attribution for a long frame, the one
 * thing the after-the-fact watchdog can't. Feature-detected + per-type
 * try/catch so an unsupported type is silently skipped. Returns a teardown.
 */
function startPerfObservers(): () => void {
  const PO = (globalThis as { PerformanceObserver?: typeof PerformanceObserver }).PerformanceObserver;
  if (!PO) return () => {};
  const observers: PerformanceObserver[] = [];
  const observe = (type: string, handle: (entry: PerformanceEntry) => void) => {
    try {
      const po = new PO((list) => {
        for (const entry of list.getEntries()) handle(entry);
      });
      po.observe({ type, buffered: false } as PerformanceObserverInit);
      observers.push(po);
    } catch {
      /* entry type unsupported on this platform — skip */
    }
  };
  observe("longtask", (entry) => {
    void sendUiLog("warn", "long task", { durationMs: Math.round(entry.duration), ...uiLogContext });
  });
  observe("long-animation-frame", (entry) => {
    void sendUiLog("warn", "long animation frame", {
      durationMs: Math.round(entry.duration),
      scripts: describeLoafScripts(entry as { scripts?: LoafScript[] }),
      ...uiLogContext,
    });
  });
  return () => {
    for (const po of observers) {
      try {
        po.disconnect();
      } catch {
        /* ignore */
      }
    }
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
  const stopPerfObservers = startPerfObservers();

  uninstall = () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
    stopStallWatchdog();
    stopPerfObservers();
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
