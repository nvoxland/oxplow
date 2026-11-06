import { describe, expect, test } from "bun:test";
import {
  invokeRoute,
  ipcRequest,
  listenRoute,
  onRemoteReconnect,
  resolveBase,
  resolveToken,
  handleSocketOpen,
  scheduleResync,
  parseRemoteInput,
  createLanes,
  BULK_IN_FLIGHT,
  PRIORITY_IN_FLIGHT,
  PRIORITY_COMMANDS,
  triggerRemoteResync,
  writeFrame,
  flushFrames,
  eventsUrl,
} from "./transport";
import { EVENT_CHANNELS } from "./channels";
import { SHELL_COMMANDS } from "./generated/shellCommands";

const MULTIPLEXED = Object.values(EVENT_CHANNELS)[0];
const DAEMON = "http://127.0.0.1:60331";
/// A command only the shell can serve, and one any daemon serves.
const SHELL_COMMAND = "set_native_menu";
const PROJECT_COMMAND = "list_streams";

test("the event socket carries the stable client identity beside its token", () => {
  const url = new URL(eventsUrl("https://daemon.example", "private & token", "window/one"));
  expect(url.protocol).toBe("wss:");
  expect(url.pathname).toBe("/events");
  expect(url.searchParams.get("token")).toBe("private & token");
  expect(url.searchParams.get("client")).toBe("window/one");
  const local = new URL(eventsUrl(DAEMON, null, "window-one"));
  expect(local.protocol).toBe("ws:");
  expect(local.searchParams.has("token")).toBe(false);
  expect(local.searchParams.get("client")).toBe("window-one");
});

describe("resolveBase", () => {
  test("no source at all means no daemon — talk to the shell", () => {
    expect(resolveBase({})).toBeNull();
    expect(resolveBase({ stored: null, injected: null, env: null })).toBeNull();
  });

  test("the shell's injected base is what a normal window uses", () => {
    expect(resolveBase({ injected: DAEMON })).toBe(DAEMON);
  });

  test("a manual connect overrides the window's own daemon", () => {
    // "Connect to remote daemon" in the launcher is an explicit user
    // action; it outranks whatever the shell injected.
    expect(resolveBase({ stored: "http://elsewhere:7420", injected: DAEMON })).toBe(
      "http://elsewhere:7420",
    );
  });

  test("the build-time override applies only when nothing else is set", () => {
    expect(resolveBase({ env: DAEMON })).toBe(DAEMON);
    expect(resolveBase({ injected: "http://injected:1", env: DAEMON })).toBe("http://injected:1");
  });

  test("blank values are ignored and trailing slashes trimmed", () => {
    expect(resolveBase({ stored: "   ", injected: DAEMON })).toBe(DAEMON);
    expect(resolveBase({ injected: `${DAEMON}///` })).toBe(DAEMON);
    expect(resolveBase({ injected: `  ${DAEMON}  ` })).toBe(DAEMON);
  });
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
    // A plain browser driving a daemon (tsk996): no daemon serves a shell
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

describe("resolveToken", () => {
  const DAEMON = "http://127.0.0.1:7420";
  test("pairs with the source the base came from", () => {
    expect(
      resolveToken({
        stored: { base: "http://manual:1", token: "m" },
        injected: { base: DAEMON, token: "i" },
      }),
    ).toBe("m");
    expect(resolveToken({ injected: { base: DAEMON, token: "i" }, env: { base: DAEMON, token: "e" } })).toBe(
      "i",
    );
    expect(resolveToken({ env: { base: DAEMON, token: "e" } })).toBe("e");
    expect(resolveToken({})).toBeNull();
  });

  test("a token in the URL fragment wins (a person pasted it)", () => {
    expect(resolveToken({ injected: { base: DAEMON, token: "i" }, fragment: "#oxplow-token=f" })).toBe("f");
    expect(resolveToken({ injected: { base: DAEMON, token: "i" }, fragment: "#other=1" })).toBe("i");
  });
});

describe("parseRemoteInput", () => {
  test("splits a pasted URL into base and token", () => {
    expect(parseRemoteInput("http://127.0.0.1:7420/#oxplow-token=abc ")).toEqual({
      base: "http://127.0.0.1:7420",
      token: "abc",
    });
    expect(parseRemoteInput("http://127.0.0.1:7420")).toEqual({ base: "http://127.0.0.1:7420", token: null });
  });
});

describe("a window closing", () => {
  test("hands its client host back with a request that outlives the page", () => {
    expect(ipcRequest("unregister_client_host", { client: "c" }, null).keepalive).toBe(true);
    expect(ipcRequest(PROJECT_COMMAND, undefined, "t").keepalive).toBe(false);
    expect(ipcRequest(PROJECT_COMMAND, undefined, "t").headers).toMatchObject({ Authorization: "Bearer t" });
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
