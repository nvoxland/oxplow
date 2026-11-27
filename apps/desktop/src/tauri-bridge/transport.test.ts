import { describe, expect, test } from "bun:test";
import {
  createWatchdog,
  dropSocket,
  isReplacedError,
  REPLACED_CLOSE_CODE,
  DEVICE_REMOVED_CLOSE_CODE,
  HOST_ENDING_CLOSE_CODE,
  OTHER_DEVICES_OFF_CLOSE_CODE,
  socketCloseMeaning,
  invokeRoute,
  ipcRequest,
  listenRoute,
  onRemoteReconnect,
  resolveEndpoint,
  filesLocalFrom,
  handleSocketOpen,
  scheduleResync,
  createLanes,
  BULK_IN_FLIGHT,
  PRIORITY_IN_FLIGHT,
  PRIORITY_COMMANDS,
  triggerRemoteResync,
  writeFrame,
  flushFrames,
  eventsUrl,
} from "./transport";
import { getUiClientId } from "../uiClientId";
import { EVENT_CHANNELS } from "./channels";
import { SHELL_COMMANDS } from "./generated/shellCommands";

const MULTIPLEXED = Object.values(EVENT_CHANNELS)[0];
const DAEMON = "http://127.0.0.1:60331";
/// A command only the shell can serve, and one any daemon serves.
const SHELL_COMMAND = "set_native_menu";
const PROJECT_COMMAND = "list_streams";

const TABLET = { id: "dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427", name: "Tab S9", kind: "tablet" as const };
const LOAD = "con-6fa459ea-ee8a-3ca4-894e-db77e160355e";

test("the event socket says which device and page load it is, beside its token", () => {
  const url = new URL(eventsUrl("https://daemon.example", "private & token", TABLET, LOAD));
  expect(url.protocol).toBe("wss:");
  expect(url.pathname).toBe("/events");
  expect(url.searchParams.get("token")).toBe("private & token");
  expect(url.searchParams.get("device")).toBe(TABLET.id);
  expect(url.searchParams.get("connection")).toBe(LOAD);
  expect(url.searchParams.get("name")).toBe("Tab S9");
  expect(url.searchParams.get("kind")).toBe("tablet");
  expect(url.searchParams.has("client")).toBe(false);
  const local = new URL(eventsUrl(DAEMON, null, TABLET, LOAD));
  expect(local.protocol).toBe("ws:");
  expect(local.searchParams.has("token")).toBe(false);
});

describe("invokeRoute", () => {
  test("with no daemon everything is the shell's", () => {
    expect(invokeRoute(SHELL_COMMAND, null, true)).toBe("tauri");
    expect(invokeRoute(PROJECT_COMMAND, null, true)).toBe("tauri");
  });

  test("a window with a daemon still runs shell commands in the shell", () => {
    // No daemon serves these — routing them over HTTP is a guaranteed
    // "unknown command".
    expect(invokeRoute(SHELL_COMMAND, DAEMON, true)).toBe("tauri");
  });

  test("a window with a daemon sends project commands to it", () => {
    expect(invokeRoute(PROJECT_COMMAND, DAEMON, true)).toBe("http");
  });

  test("with no Tauri host, a shell command has nowhere to go", () => {
    // A plain browser driving a daemon: no daemon serves a shell
    // command, so it's refused here, not sent to fail there.
    expect(invokeRoute(SHELL_COMMAND, DAEMON, false)).toBe("none");
    expect(invokeRoute(PROJECT_COMMAND, DAEMON, false)).toBe("http");
  });
});

test("the generated shell-command table names the shell surface", () => {
  const names: readonly string[] = SHELL_COMMANDS;
  expect(names).toContain(SHELL_COMMAND);
  expect(names).toContain("open_project");
  expect(names).not.toContain(PROJECT_COMMAND);
});

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

// A host mounted before the socket opened ran against a snapshot that may
// predate events the socket was not yet there to hear (a write made while
// the window loaded), so the first open re-hydrates it as a reconnect does.
test("the first open of the socket fires the reconnect handlers, as a reconnect does", () => {
  let count = 0;
  const off = onRemoteReconnect(() => (count += 1));
  handleSocketOpen();
  expect(count).toBe(1);
  handleSocketOpen();
  expect(count).toBe(2);
  off();
});

// The daemon's socket fell behind and lost frames: everything is read again,
// once for a burst of those.
test("lost frames re-hydrate as a reconnect does, once for a burst", async () => {
  let count = 0;
  const off = onRemoteReconnect(() => (count += 1));
  scheduleResync();
  scheduleResync();
  scheduleResync();
  expect(count).toBe(0);
  await Bun.sleep(1100);
  expect(count).toBe(1);
  off();
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

describe("a window closing", () => {
  test("hands its client host back with a request that outlives the page", () => {
    expect(ipcRequest("unregister_client_host", { client: "c" }, null).keepalive).toBe(true);
    expect(ipcRequest(PROJECT_COMMAND, undefined, "t").keepalive).toBe(false);
    expect(ipcRequest(PROJECT_COMMAND, undefined, "t").headers).toMatchObject({ Authorization: "Bearer t" });
  });

  test("every request says who asks: the device and the connection", () => {
    const headers = ipcRequest(PROJECT_COMMAND, undefined, null).headers as Record<string, string>;
    expect(headers["Oxplow-Device"]).toMatch(/^dev-[0-9a-f-]{36}$/);
    expect(headers["Oxplow-Connection"]).toMatch(/^con-[0-9a-f-]{36}$/);
    expect(headers["Oxplow-Connection"]).toBe(getUiClientId());
  });

  test("a shell-only command still goes to the shell, not over HTTP", () => {
    expect(invokeRoute(SHELL_COMMAND, DAEMON, true)).toBe("tauri");
  });
});

describe("request lanes", () => {
  /// A task that stays in flight until `finish` is called.
  function pending() {
    let finish: () => void = () => {};
    const done = new Promise<void>((resolve) => {
      finish = resolve;
    });
    return { run: () => done, finish };
  }

  test("a burst of reads fills the bulk lane and a priority call still goes at once", async () => {
    const lanes = createLanes({ bulk: BULK_IN_FLIGHT, priority: PRIORITY_IN_FLIGHT });
    let started = 0;
    const reads = Array.from({ length: 20 }, () => {
      const p = pending();
      void lanes(false, () => {
        started += 1;
        return p.run();
      });
      return p;
    });
    expect(started).toBe(BULK_IN_FLIGHT);
    let sent = false;
    const key = lanes(true, async () => {
      sent = true;
    });
    await key;
    expect(sent).toBe(true);
    expect(started).toBe(BULK_IN_FLIGHT);
    // Reads finishing let the waiting ones through, in order.
    reads[0].finish();
    await Promise.resolve();
    await Promise.resolve();
    expect(started).toBe(BULK_IN_FLIGHT + 1);
    reads.forEach((r) => r.finish());
  });

  test("the lanes together stay within the webview's six connections, and a stream switch is a priority call", () => {
    expect(BULK_IN_FLIGHT + PRIORITY_IN_FLIGHT).toBeLessThanOrEqual(6);
    expect(PRIORITY_COMMANDS.has("switch_stream")).toBe(true);
    expect(PRIORITY_COMMANDS.has("list_streams")).toBe(false);
  });
});

describe("frames the window writes on the socket", () => {
  function fakeSocket(readyState: number) {
    const sent: string[] = [];
    return { sent, socket: { readyState, send: (f: string) => void sent.push(f) } };
  }

  test("an open socket sends each frame at once, in order", () => {
    const { sent, socket } = fakeSocket(WebSocket.OPEN);
    const waiting: string[] = [];
    writeFrame(socket, waiting, "a");
    writeFrame(socket, waiting, "b");
    expect(sent).toEqual(["a", "b"]);
    expect(waiting).toEqual([]);
  });

  test("a socket still opening holds them until it opens, then sends them first", () => {
    const { sent, socket } = fakeSocket(WebSocket.CONNECTING);
    const waiting: string[] = [];
    writeFrame(socket, waiting, "a");
    writeFrame(socket, waiting, "b");
    expect(sent).toEqual([]);
    socket.readyState = WebSocket.OPEN;
    flushFrames(socket, waiting);
    writeFrame(socket, waiting, "c");
    expect(sent).toEqual(["a", "b", "c"]);
  });

  test("with no socket, or a closed one, a frame is dropped", () => {
    const waiting: string[] = [];
    writeFrame(null, waiting, "a");
    const { sent, socket } = fakeSocket(WebSocket.CLOSED);
    writeFrame(socket, waiting, "b");
    expect(sent).toEqual([]);
    expect(waiting).toEqual([]);
  });
});

// Open In ▸ Reveal / Default App act on this machine's files: only when
// the window drives the daemon its shell started (on this machine).
describe("filesLocalFrom", () => {
  // A window the shell opened on a project folder here has its files; the
  // launcher's projects are here; a page a host served, or a window on
  // another machine's host, has none.
  test("only the shell's own project windows and the launcher", () => {
    expect(filesLocalFrom({ shell: true, kind: "project", projectDir: "/Users/me/src/a" })).toBe(true);
    expect(filesLocalFrom({ shell: true, kind: "project", projectDir: null })).toBe(false);
    expect(filesLocalFrom({ shell: false, kind: "project", projectDir: "/x" })).toBe(false);
    expect(filesLocalFrom({ shell: true, kind: "launcher", projectDir: null })).toBe(true);
  });
});

describe("resolveEndpoint", () => {
  const DAEMON = "http://127.0.0.1:7420";
  // One read path: what started the page says where its daemon is.
  test("the window's context is the base and the token", () => {
    expect(resolveEndpoint({ base: `${DAEMON}///`, token: " t " }, null)).toEqual({ base: DAEMON, token: "t" });
    expect(resolveEndpoint(null, "p")).toEqual({ base: null, token: null });
    expect(resolveEndpoint({ base: "  ", token: "t" }, null)).toEqual({ base: null, token: null });
  });

  // A page a host served carries no token: a paired device uses its own.
  test("a host's page uses the device's own token", () => {
    const page = { base: `${DAEMON}/p/prj-1`, token: null, host: DAEMON };
    expect(resolveEndpoint(page, "dev-token").token).toBe("dev-token");
    expect(resolveEndpoint(page, null).token).toBeNull();
    expect(resolveEndpoint({ ...page, host: undefined }, "dev-token").token).toBeNull();
    expect(resolveEndpoint({ ...page, token: "owner" }, "dev-token").token).toBe("owner");
  });
});

// A socket the daemon closes as replaced (another page load of this device
// took over) is not a drop: the window stops and says so. A refused token
// stays refused; anything else is a drop to retry.
test("what a socket's close means", () => {
  expect(socketCloseMeaning(REPLACED_CLOSE_CODE, false)).toBe("replaced");
  expect(socketCloseMeaning(1006, false)).toBe("retry");
  expect(socketCloseMeaning(1006, true)).toBe("refused");
  // The host removed this device: pair again. The host is ending (the app
  // quit): say so, and keep trying slowly.
  expect(socketCloseMeaning(DEVICE_REMOVED_CLOSE_CODE, false)).toBe("removed");
  expect(socketCloseMeaning(HOST_ENDING_CLOSE_CODE, false)).toBe("ended");
  // Other devices were turned off there (or moved): say so, keep trying
  // slowly.
  expect(socketCloseMeaning(OTHER_DEVICES_OFF_CLOSE_CODE, false)).toBe("closed");
});

test("a REPLACED answer is the replaced page load's", () => {
  expect(isReplacedError({ code: "REPLACED", message: "replaced" })).toBe(true);
  expect(isReplacedError({ code: "INVALID", message: "no" })).toBe(false);
  expect(isReplacedError(null)).toBe(false);
});

// The page's watchdog: every frame feeds it; silence for its time runs
// `silent` once, and a fed watchdog waits again.
test("a watchdog runs once nothing came for its time, and every frame puts it off", () => {
  const timers: Array<{ f: () => void; ms: number; live: boolean }> = [];
  const fake = {
    set: (f: () => void, ms: number) => {
      const t = {
        f: () => {
          t.live = false;
          f();
        },
        ms,
        live: true,
      };
      timers.push(t);
      return t;
    },
    clear: (t: unknown) => {
      (t as { live: boolean }).live = false;
    },
  };
  let silent = 0;
  const dog = createWatchdog(50, () => (silent += 1), fake);
  dog.feed();
  dog.feed();
  const live = () => timers.filter((t) => t.live);
  expect(live()).toHaveLength(1);
  live()[0]!.f();
  expect(silent).toBe(1);
  dog.feed();
  dog.stop();
  expect(live()).toHaveLength(0);
});

// The watchdog takes a silent socket out of service at once: it doesn't wait
// for the close handshake, which a dead path never finishes.
test("a silent socket is dropped at once and its late close is not heard", () => {
  let closed = 0;
  let gone = 0;
  const ws = { onmessage: () => {}, onopen: () => {}, onclose: () => (gone += 100), onerror: () => {}, close: () => (closed += 1) };
  dropSocket(ws, () => (gone += 1));
  expect(gone).toBe(1);
  expect(closed).toBe(1);
  expect([ws.onmessage, ws.onopen, ws.onclose, ws.onerror]).toEqual([null, null, null, null]);
});
