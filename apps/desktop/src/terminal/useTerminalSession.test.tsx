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
let answer: Promise<void> = Promise.resolve();
// What the open answers about the size: its grid, and who owns it.
let sized: { cols: number; rows: number; owner: string | null; you: string | null } = { cols: 120, rows: 40, owner: null, you: "con-me" };
// The session id the next open answers; null: a new one each time.
let sameSession: string | null = null;
let reconnect: () => void = () => {};
mock.module("../api.js", () => ({
  ...realApi,
  onRemoteReconnect: (cb: () => void) => {
    reconnect = cb;
    return () => {};
  },
  desktopBridge: () => ({
    openTerminalSession: async (uri: string, cols: number, rows: number) => {
      calls.push(`open ${uri} ${cols}x${rows}`);
      await answer;
      return { sessionId: sameSession ?? `term-${next++}`, replayB64: btoa("hello"), ...sized };
    },
    lookupTerminalSession: async (uri: string) => {
      calls.push(`lookup ${uri}`);
      return running;
    },
    closeTerminalSession: async (id: string) => void calls.push(`detach ${id}`),
    terminalInput: (uri: string, message: string) => void calls.push(`input ${uri} ${message}`),
    terminalSessionCwd: async () => null,
    onTerminalEvent: (cb: typeof listener) => {
      listener = cb;
      return () => {};
    },
  }),
}));
const { useTerminalSession, useTerminalLive } = await import("./useTerminalSession.js");

afterEach(() => {
  cleanup();
  calls.length = 0;
  running = null;
  next = 1;
  answer = Promise.resolve();
  sized = { cols: 120, rows: 40, owner: null, you: "con-me" };
  sameSession = null;
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

// A person typing a new thread's title while a session finishes opening keeps
// the focus: losing it would close that entry, which cancels on blur.
test("a session that finishes opening takes the focus, unless a field elsewhere has it", async () => {
  const open = async () => {
    let focused = 0;
    const { term, written } = fakeTerm();
    Object.assign(term, { element: document.createElement("div"), focus: () => void focused++ });
    const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
    act(() => void result.current.attach(term));
    await waitFor(() => expect(written).toContain("hello"));
    await waitFor(() => expect(calls.at(-1)).toBe(`open ${SHELL} 80x24`));
    return () => focused;
  };
  const free = await open();
  await waitFor(() => expect(free()).toBe(1));
  cleanup();

  const field = document.createElement("input");
  document.body.append(field);
  field.focus();
  try {
    const held = await open();
    // The replay is written just before the focus would be taken.
    await act(async () => {
      await answer;
    });
    expect(held()).toBe(0);
    expect(document.activeElement).toBe(field);
  } finally {
    field.remove();
  }
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

// Restart Terminal is offered only while a process runs in the pane: an
// ended one shows Start Again and takes no keys, so a restart there would
// start an agent nobody sees.
test("a session is live while its process runs, and not once it ends or the pane goes", async () => {
  const live = renderHook(() => useTerminalLive(AGENT));
  expect(live.result.current).toBe(false);
  const { result, unmount } = renderHook(() => useTerminalSession(AGENT, { shown: true, startOnActivate: true }));
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(live.result.current).toBe(true));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 0 }) }));
  expect(live.result.current).toBe(false);
  act(() => result.current.reopen());
  await waitFor(() => expect(live.result.current).toBe(true));
  unmount();
  expect(live.result.current).toBe(false);
});

test("an attached agent follows a restarted PTY", async () => {
  const { result } = renderHook(() => useTerminalSession(AGENT, { shown: true, startOnActivate: true }));
  const { term, written } = fakeTerm();
  act(() => void result.current.attach(term));
  await waitFor(() => expect(written).toContain("hello"));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "restart" }) }));
  await waitFor(() => expect(written.filter((chunk) => chunk === "hello")).toHaveLength(2));
  expect(written).toContain("<reset>");
  expect(result.current.ended).toBeNull();
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 1 }) }));
  expect(result.current.ended).toBeNull();
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

test("keys go to the terminal's URI at once, encoded, before its open answers too", async () => {
  let release: () => void = () => {};
  answer = new Promise<void>((r) => {
    release = r;
  });
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  act(() => result.current.input("é", true));
  act(() => result.current.binary("\x01"));
  act(() => result.current.resize(100, 30));
  expect(calls.filter((c) => c.startsWith("input"))).toEqual([
    `input ${SHELL} {"type":"input","bytes":"w6k=","typed":true}`,
    `input ${SHELL} {"type":"input-binary","bytes":"AQ=="}`,
    `input ${SHELL} {"type":"resize","cols":100,"rows":30}`,
  ]);
  release();
  await waitFor(() => expect(calls).toContain(`open ${SHELL} 80x24`));
});

test("an ended session takes no keys; unmounting detaches", async () => {
  const { result, unmount } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(result.current.ended).toBeNull());
  await waitFor(() => expect(calls.length).toBe(1));
  await new Promise((r) => setTimeout(r, 0));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "exit", exitCode: 1 }) }));
  const before = calls.length;
  act(() => result.current.input("x"));
  expect(calls.length).toBe(before);
  unmount();
  expect(calls).toContain("detach term-1");
});

// The terminal's grid is the one the client typing into it set: the open
// answers it, a size message changes it, and Fit to this device takes it.
test("a pane follows the terminal's grid, and Fit takes it", async () => {
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  const { term } = fakeTerm();
  act(() => void result.current.attach(term));
  await waitFor(() => expect(result.current.grid).toEqual({ cols: 120, rows: 40 }));
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "size", cols: 90, rows: 30 }) }));
  expect(result.current.grid).toEqual({ cols: 90, rows: 30 });
  act(() => result.current.fit(80, 24));
  const sent = calls.filter((c) => c.startsWith("input "));
  expect(JSON.parse(sent.at(-1)!.slice(`input ${SHELL} `.length))).toEqual({ type: "resize", cols: 80, rows: 24, take: true });
});

// The open answers who owns the size, and a size message says it again: a pane
// says "another device" only when the daemon does.
test("a pane is owned elsewhere only when the daemon says another page load owns it", async () => {
  sized = { cols: 120, rows: 40, owner: "con-other", you: "con-me" };
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(result.current.ownedElsewhere).toBe(true));
  // It types: the daemon's size event names it the owner.
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "size", cols: 80, rows: 24, owner: "con-me" }) }));
  expect(result.current.ownedElsewhere).toBe(false);
  expect(result.current.grid).toEqual({ cols: 80, rows: 24 });
  act(() => listener({ sessionId: "term-1", message: JSON.stringify({ type: "size", cols: 90, rows: 30, owner: "con-other" }) }));
  expect(result.current.ownedElsewhere).toBe(true);
});

// Only what the person typed takes the size: xterm's replies to queries are
// input too, and are sent without the mark.
test("input is marked typed only when the pane says so", async () => {
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  act(() => result.current.input("k", true));
  act(() => result.current.input("\x1b[?62c", false));
  act(() => result.current.binary("\x01"));
  const sent = calls.filter((c) => c.startsWith("input ")).map((c) => JSON.parse(c.slice(`input ${SHELL} `.length)));
  expect(sent.map((m) => m.typed === true)).toEqual([true, false, false]);
});

// After a reconnect the pane asks again; the answer's grid is the terminal's
// now, and the size it asks for is its room, not the grid it draws.
test("a reopen of the same session takes the answer's grid", async () => {
  sameSession = "term-7";
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  act(() => void result.current.attach(fakeTerm().term));
  await waitFor(() => expect(result.current.grid).toEqual({ cols: 120, rows: 40 }));
  // It missed a size event while the socket was down.
  sized = { cols: 100, rows: 30, owner: "con-other", you: "con-me" };
  act(() => reconnect());
  await waitFor(() => expect(result.current.grid).toEqual({ cols: 100, rows: 30 }));
  expect(result.current.ownedElsewhere).toBe(true);
});

test("a reopen asks for the pane's size, not the grid the terminal draws", async () => {
  const { result } = renderHook(() => useTerminalSession(SHELL, { shown: true }));
  const { term } = fakeTerm();
  act(() => void result.current.attach(term, { cols: 90, rows: 30 }));
  await waitFor(() => expect(calls).toContain(`open ${SHELL} 90x30`));
  // The surface now draws the owner's grid, and the pane is measured again.
  Object.assign(term, { cols: 120, rows: 40 });
  act(() => result.current.resize(100, 31));
  sized = { ...sized, owner: "con-other" };
  await waitFor(() => expect(result.current.grid).not.toBeNull());
  act(() => reconnect());
  await waitFor(() => expect(calls.filter((c) => c.startsWith("open"))).toHaveLength(2));
  expect(calls.filter((c) => c.startsWith("open")).at(-1)).toBe(`open ${SHELL} 100x31`);
});
