import { describe, expect, test } from "bun:test";
import {
  invokeRoute,
  listenRoute,
  onRemoteReconnect,
  resolveBase,
  resolveToken,
  parseRemoteInput,
  triggerRemoteResync,
} from "./transport";
import { EVENT_CHANNELS } from "./channels";
import { SHELL_COMMANDS } from "./generated/shellCommands";

const MULTIPLEXED = Object.values(EVENT_CHANNELS)[0];
const DAEMON = "http://127.0.0.1:60331";
/// A command only the shell can serve, and one any daemon serves.
const SHELL_COMMAND = "set_native_menu";
const PROJECT_COMMAND = "list_streams";

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
