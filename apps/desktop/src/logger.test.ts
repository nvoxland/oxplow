import { afterEach, expect, mock, test } from "bun:test";

// logger.ts calls desktopBridge().logUi(...) inside sendUiLog. Stub the
// bridge so install/uninstall don't depend on a real Tauri runtime.
mock.module("./api.js", () => ({
  desktopBridge: () => ({ logUi: async () => {} }),
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

const { installUiLogging, uninstallUiLogging, evaluateStall } = await import("./logger.js");

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
