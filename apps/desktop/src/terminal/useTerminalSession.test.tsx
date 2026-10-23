import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";

// A terminal's session: opened at the surface's size and replayed into
// it, followed to its exit, opened again on Start Again or when what an
// agent is bound to changes; an agent's spawns nothing until it is shown,
// attaching at once to one that runs; unmounting detaches.

const realApi = { ...(await import("../api.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
const calls: string[] = [];
let running: string | null = null;
let listener: (e: { sessionId: string; message: string }) => void = () => {};
let next = 1;
mock.module("../api.js", () => ({
  ...realApi,
  onRemoteReconnect: () => () => {},
  desktopBridge: () => ({
    openTerminalSession: async (uri: string, cols: number, rows: number) => {
      calls.push(`open ${uri} ${cols}x${rows}`);
      return { sessionId: `term-${next++}`, replayB64: btoa("hello") };
    },
    lookupTerminalSession: async (uri: string) => {
      calls.push(`lookup ${uri}`);
      return running;
    },
    closeTerminalSession: async (id: string) => void calls.push(`detach ${id}`),
    forwardTerminalInput: async (id: string, message: string) => void calls.push(`input ${id} ${message}`),
    terminalSessionCwd: async () => null,
    onTerminalEvent: (cb: typeof listener) => {
      listener = cb;
      return () => {};
    },
  }),
}));
const { useTerminalSession } = await import("./useTerminalSession.js");

afterEach(() => {
  cleanup();
  calls.length = 0;
  running = null;
  next = 1;
});

function fakeTerm() {
  const written: string[] = [];
  const term = {
    cols: 80,
    rows: 24,
    write: (data: string | Uint8Array) => written.push(typeof data === "string" ? data : new TextDecoder().decode(data)),
    reset: () => written.push("<reset>"),
    focus: () => {},
  } as unknown as Terminal;
  return { term, written };
}

const SHELL = "oxplow:oxplow/terminal?id=trm1";
const AGENT = "oxplow:oxplow/agent_session?id=ses1";

test("a shell opens at the surface's size and its replay is written", async () => {
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  const { term, written } = fakeTerm();
  act(() => void result.current.attach(term));
  await waitFor(() => expect(written).toContain("hello"));
  expect(calls).toEqual([`open ${SHELL} 80x24`]);
});

test("an agent's hidden tab looks it up and opens only one that runs", async () => {
  const { result, rerender } = renderHook(({ shown }) => useTerminalSession(AGENT, { shown, startOnActivate: true }), {
    initialProps: { shown: false },
  });
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(calls).toEqual([`lookup ${AGENT}`]));
  rerender({ shown: true });
  await waitFor(() => expect(calls).toContain(`open ${AGENT} 80x24`));
});

test("an exit is said with its outcome, and Start Again opens it again under a notice", async () => {
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  const { term, written } = fakeTerm();
  act(() => void result.current.attach(term));
  await waitFor(() => expect(calls.length).toBe(1));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 0 }) }));
  expect(result.current.ended).toEqual({ exitCode: 0, outcome: { kind: "ended" } });
  act(() => result.current.reopen());
  await waitFor(() => expect(result.current.ended).toBeNull());
  expect(written).toContain("<reset>");
});

test("an agent moved to another conversation opens again once its process exits", async () => {
  const { result, rerender } = renderHook(({ conversation }) => useTerminalSession(AGENT, { shown: true, startOnActivate: true, conversation }), {
    initialProps: { conversation: "a" },
  });
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(calls.filter((c) => c.startsWith("open")).length).toBe(1));
  rerender({ conversation: "b" });
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 0 }) }));
  await waitFor(() => expect(calls.filter((c) => c.startsWith("open")).length).toBe(2));
  expect(result.current.ended).toBeNull();
});

test("keys go to the session; an ended one takes none; unmounting detaches", async () => {
  const { result, unmount } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(calls.length).toBe(1));
  act(() => result.current.input("ls"));
  await waitFor(() => expect(calls.some((c) => c.startsWith("input term-1"))).toBe(true));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 1 }) }));
  const before = calls.length;
  act(() => result.current.input("x"));
  await new Promise((r) => setTimeout(r, 10));
  expect(calls.length).toBe(before);
  unmount();
  expect(calls).toContain("detach term-1");
});
