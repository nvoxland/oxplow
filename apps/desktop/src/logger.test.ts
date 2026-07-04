import { afterEach, expect, mock, test } from "bun:test";

// logger.ts calls desktopBridge().logUi(...) inside sendUiLog. Stub the
// bridge so install/uninstall don't depend on a real Tauri runtime, and
// record the calls so tests can assert what got logged. logUi pushes
// synchronously (no await before the push), so the record is populated by
// the time the caller returns.
const logged: Array<{ level: string; message: string; context?: Record<string, unknown> }> = [];
mock.module("./api.js", () => ({
  desktopBridge: () => ({
    logUi: async (r: { level: string; message: string; context?: Record<string, unknown> }) => {
      logged.push(r);
    },
  }),
}));

// The logger only touches window.add/removeEventListener and
// location.href. We swap in a fake window that records listeners so we
// can assert install/uninstall counts precisely (happy-dom's real
// window doesn't expose its listener registry). Originals are captured
// up front and restored after each case so the fake never leaks into
// other test files sharing this process.
type Listener = (event: unknown) => void;
const originalWindow = (globalThis as unknown as { window: unknown }).window;
const originalLocation = (globalThis as unknown as { location: unknown }).location;

function installFakeWindow(): Map<string, Set<Listener>> {
  const listeners = new Map<string, Set<Listener>>();
  (globalThis as unknown as { window: unknown }).window = {
    addEventListener(type: string, fn: Listener) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(fn);
    },
    removeEventListener(type: string, fn: Listener) {
      listeners.get(type)?.delete(fn);
    },
  };
  (globalThis as unknown as { location: unknown }).location = { href: "test://app" };
  return listeners;
}

const { installUiLogging, uninstallUiLogging, evaluateStall, isSlowOperation, timed, describeLoafScripts } =
  await import("./logger.js");

afterEach(() => {
  // Reset module singleton state, then restore the real globals.
  uninstallUiLogging();
  (globalThis as unknown as { window: unknown }).window = originalWindow;
  (globalThis as unknown as { location: unknown }).location = originalLocation;
});

test("registers global error listeners on install", () => {
  const listeners = installFakeWindow();
  installUiLogging();
  expect(listeners.get("error")?.size).toBe(1);
  expect(listeners.get("unhandledrejection")?.size).toBe(1);
});

test("uninstall removes listeners and restores console", () => {
  const listeners = installFakeWindow();
  const beforeInstall = console.error;
  installUiLogging();
  const patched = console.error;
  expect(patched).not.toBe(beforeInstall);

  uninstallUiLogging();
  expect(listeners.get("error")?.size).toBe(0);
  expect(listeners.get("unhandledrejection")?.size).toBe(0);
  // Console is no longer the patched wrapper (restored to original behavior).
  expect(console.error).not.toBe(patched);
});

test("re-install is idempotent (no duplicate listeners)", () => {
  const listeners = installFakeWindow();
  const uninstallA = installUiLogging();
  const uninstallB = installUiLogging();
  expect(uninstallB).toBe(uninstallA);
  expect(listeners.get("error")?.size).toBe(1);
});

test("uninstall before install is a safe no-op", () => {
  installFakeWindow();
  expect(() => uninstallUiLogging()).not.toThrow();
});

test("evaluateStall: a normal tick (elapsed ≈ expected) is not a stall", () => {
  const r = evaluateStall({ actualElapsedMs: 1010, expectedMs: 1000, thresholdMs: 1000, wasHidden: false });
  expect(r.stalled).toBe(false);
  expect(r.blockedMs).toBe(10);
});

test("evaluateStall: a long drift while visible is a stall, blockedMs = drift", () => {
  const r = evaluateStall({ actualElapsedMs: 2840, expectedMs: 1000, thresholdMs: 1000, wasHidden: false });
  expect(r.stalled).toBe(true);
  expect(r.blockedMs).toBe(1840);
});

test("evaluateStall: drift below the threshold is not a stall", () => {
  const r = evaluateStall({ actualElapsedMs: 1900, expectedMs: 1000, thresholdMs: 1000, wasHidden: false });
  expect(r.stalled).toBe(false);
});

test("evaluateStall: a hidden window never counts as a stall (timer throttling)", () => {
  const r = evaluateStall({ actualElapsedMs: 60_000, expectedMs: 1000, thresholdMs: 1000, wasHidden: true });
  expect(r.stalled).toBe(false);
  expect(r.blockedMs).toBe(59_000);
});

test("isSlowOperation: at or above the threshold is slow, below is not", () => {
  expect(isSlowOperation(49, 50)).toBe(false);
  expect(isSlowOperation(50, 50)).toBe(true);
  expect(isSlowOperation(1000, 50)).toBe(true);
});

test("timed: returns the fn result and logs a slow operation when it exceeds the threshold", () => {
  logged.length = 0;
  let i = 0;
  const clock = [1000, 1080]; // 80ms elapsed
  const result = timed("terminal-repaint", () => 42, {
    thresholdMs: 50,
    now: () => clock[i++]!,
    context: () => ({ bufferLines: 5000 }),
  });
  expect(result).toBe(42);
  const slow = logged.find((l) => l.message === "slow operation");
  expect(slow).toBeDefined();
  expect(slow!.level).toBe("warn");
  expect(slow!.context).toMatchObject({ label: "terminal-repaint", durMs: 80, bufferLines: 5000 });
});

test("timed: a fast fn returns its result and logs nothing", () => {
  logged.length = 0;
  let i = 0;
  const clock = [1000, 1005]; // 5ms elapsed, under threshold
  const result = timed("fast", () => "ok", { thresholdMs: 50, now: () => clock[i++]! });
  expect(result).toBe("ok");
  expect(logged.find((l) => l.message === "slow operation")).toBeUndefined();
});

test("timed: still times (and returns) when fn throws, then rethrows", () => {
  logged.length = 0;
  let i = 0;
  const clock = [1000, 1200]; // 200ms before it threw
  expect(() =>
    timed("boom", () => {
      throw new Error("nope");
    }, { thresholdMs: 50, now: () => clock[i++]! }),
  ).toThrow("nope");
  expect(logged.find((l) => l.message === "slow operation")?.context).toMatchObject({ label: "boom", durMs: 200 });
});

test("describeLoafScripts: no scripts field yields an empty attribution list", () => {
  expect(describeLoafScripts({})).toEqual([]);
  expect(describeLoafScripts({ scripts: [] })).toEqual([]);
});

test("describeLoafScripts: maps script fields and rounds durations", () => {
  const out = describeLoafScripts({
    scripts: [
      { sourceURL: "app.js", sourceFunctionName: "repaint", duration: 912.7, invoker: "requestAnimationFrame" },
      { duration: 40.2 },
    ],
  });
  expect(out).toEqual([
    { src: "app.js", fn: "repaint", durMs: 913, invoker: "requestAnimationFrame" },
    { src: undefined, fn: undefined, durMs: 40, invoker: undefined },
  ]);
});
