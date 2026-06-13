import { describe, expect, test } from "bun:test";
import { listenRoute, onRemoteReconnect, triggerRemoteResync } from "./transport";
import { EVENT_CHANNELS } from "./channels";

const MULTIPLEXED = Object.values(EVENT_CHANNELS)[0];

describe("listenRoute", () => {
  test("local mode with Tauri uses the Tauri event bus", () => {
    expect(listenRoute(MULTIPLEXED, null, true)).toBe("tauri");
    expect(listenRoute("menu:command", null, true)).toBe("tauri");
  });

  test("remote mode multiplexed channels read the daemon WebSocket", () => {
    expect(listenRoute(MULTIPLEXED, "http://127.0.0.1:7420", true)).toBe("ws");
    expect(listenRoute(MULTIPLEXED, "http://127.0.0.1:7420", false)).toBe("ws");
  });

  test("remote mode shell-local channels still use Tauri when hosted by the shell", () => {
    expect(listenRoute("menu:command", "http://127.0.0.1:7420", true)).toBe("tauri");
  });

  test("plain-browser session routes shell-local channels nowhere", () => {
    // No Tauri host (Playwright / served dist/): subscribing to the
    // local event bus would throw on __TAURI_INTERNALS__.
    expect(listenRoute("menu:command", "http://127.0.0.1:7420", false)).toBe("none");
  });
});

test("triggerRemoteResync fires every registered reconnect handler", () => {
  let a = 0;
  let b = 0;
  const offA = onRemoteReconnect(() => (a += 1));
  const offB = onRemoteReconnect(() => (b += 1));

  triggerRemoteResync();
  expect(a).toBe(1);
  expect(b).toBe(1);

  triggerRemoteResync();
  expect(a).toBe(2);
  expect(b).toBe(2);

  offA();
  offB();
});

test("an unsubscribed handler no longer fires", () => {
  let count = 0;
  const off = onRemoteReconnect(() => (count += 1));
  triggerRemoteResync();
  expect(count).toBe(1);

  off();
  triggerRemoteResync();
  expect(count).toBe(1);
});
