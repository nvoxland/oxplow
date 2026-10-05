import { expect, test } from "bun:test";

import { terminalSender } from "./terminalInput.js";

// tsk979: a terminal's messages reach the daemon one at a time, in the
// order they were made — each its own request would let a later keystroke
// overtake an earlier one — and keystrokes that wait together go as one.
// tsk992: a send that doesn't answer drops its session's backlog rather
// than delivering it late in a burst; a closed pane sends nothing more; a
// bare Escape keeps its own message; scrolls and resizes coalesce.

function harness(timeoutMs = 1000) {
  const sent: Array<[string, unknown]> = [];
  const replies: Array<(ok: boolean) => void> = [];
  const errors: string[] = [];
  const sender = terminalSender(
    (sessionId, message) => {
      sent.push([sessionId, JSON.parse(message)]);
      return new Promise<void>((resolve, reject) => replies.push((ok) => (ok ? resolve() : reject(new Error("lost")))));
    },
    (e) => errors.push(e instanceof Error ? e.message : String(e)),
    { timeoutMs },
  );
  /** Answer the call in flight and let the sender go on. */
  const answer = async (ok = true) => {
    replies.shift()!(ok);
    await new Promise((r) => setTimeout(r, 0));
  };
  return { send: sender.send, attach: sender.attach, close: sender.close, sent, answer, errors };
}

const decoded = (m: unknown) => new TextDecoder().decode(Uint8Array.from(atob((m as { bytes: string }).bytes), (c) => c.charCodeAt(0)));

test("a message waits for the one before it, and keystrokes waiting together go as one", async () => {
  const { send, sent, answer } = harness();
  send("s1", { type: "input", data: "6" });
  send("s1", { type: "input", data: "*" });
  send("s1", { type: "input", data: "7" });
  expect(sent.length).toBe(1);
  expect(decoded(sent[0]![1])).toBe("6");
  await answer();
  expect(sent.length).toBe(2);
  expect(sent[1]![1]).toMatchObject({ type: "input" });
  expect(decoded(sent[1]![1])).toBe("*7");
  await answer();
  expect(sent.length).toBe(2);
});

test("only neighbouring keystrokes of one session merge; everything else keeps its place", async () => {
  const { send, sent, answer } = harness();
  send("s1", { type: "history-exit" });
  send("s1", { type: "input", data: "a" });
  send("s1", { type: "resize", cols: 80, rows: 24 });
  send("s1", { type: "input", data: "é" });
  send("s2", { type: "input", data: "b" });
  send("s1", { type: "input-binary", data: "\x01" });
  for (let i = 0; i < 6; i++) await answer();
  expect(sent.map(([s, m]) => [s, (m as { type: string }).type])).toEqual([
    ["s1", "history-exit"],
    ["s1", "input"],
    ["s1", "resize"],
    ["s1", "input"],
    ["s2", "input"],
    ["s1", "input-binary"],
  ]);
  expect(decoded(sent[3]![1])).toBe("é");
  expect(sent[2]![1]).toEqual({ type: "resize", cols: 80, rows: 24 });
});

test("a failed send is reported and the next one still goes", async () => {
  const { send, sent, answer, errors } = harness();
  send("s1", { type: "input", data: "x" });
  send("s1", { type: "input", data: "y" });
  await answer(false);
  expect(errors.length).toBe(1);
  expect(sent.length).toBe(2);
  expect(decoded(sent[1]![1])).toBe("y");
});

test("a bare Escape isn't merged with the key after it", async () => {
  const { send, sent, answer } = harness();
  send("s1", { type: "input", data: "a" });
  send("s1", { type: "input", data: "\x1b" });
  send("s1", { type: "input", data: "\r" });
  for (let i = 0; i < 3; i++) await answer();
  expect(sent.map(([, m]) => decoded(m))).toEqual(["a", "\x1b", "\r"]);
});

test("scrolls waiting together sum, and a newer resize replaces a waiting one", async () => {
  const { send, sent, answer } = harness();
  send("s1", { type: "input", data: "x" });
  send("s1", { type: "history-scroll", lines: 3 });
  send("s1", { type: "history-scroll", lines: -1 });
  send("s1", { type: "history-scroll", lines: 2 });
  send("s1", { type: "resize", cols: 80, rows: 24 });
  send("s1", { type: "resize", cols: 100, rows: 30 });
  for (let i = 0; i < 3; i++) await answer();
  expect(sent.map(([, m]) => m)).toEqual([
    { type: "input", bytes: btoa("x") },
    { type: "history-scroll", lines: 4 },
    { type: "resize", cols: 100, rows: 30 },
  ]);
});

test("a send that doesn't answer drops its session's backlog, said so, and the next goes", async () => {
  const { send, sent, errors } = harness(20);
  send("s1", { type: "input", data: "ls" });
  send("s1", { type: "input", data: "\r" });
  send("s2", { type: "input", data: "y" });
  await new Promise((r) => setTimeout(r, 60));
  expect(errors.join()).toContain("didn't answer");
  // s1's waiting Enter is never sent late; s2's input goes.
  expect(sent.map(([s, m]) => [s, decoded(m)])).toEqual([
    ["s1", "ls"],
    ["s2", "y"],
  ]);
});

test("a closed sender sends nothing more", async () => {
  const { send, close, sent, answer } = harness();
  send("s1", { type: "input", data: "a" });
  send("s1", { type: "input", data: "b" });
  close();
  send("s1", { type: "input", data: "c" });
  await answer();
  expect(sent.map(([, m]) => decoded(m))).toEqual(["a"]);
});

// tsk993: keystrokes typed while the terminal's session is still opening
// are kept, and sent — in order, before anything after them — once it
// opens; only keystrokes are kept, and only so many.
test("keystrokes typed before the session opens are sent when it does", async () => {
  const { send, attach, sent, answer } = harness();
  send(null, { type: "input", data: "ec" });
  send(null, { type: "input", data: "ho" });
  send(null, { type: "resize", cols: 80, rows: 24 });
  expect(sent).toEqual([]);
  attach("s1");
  send("s1", { type: "input", data: " hi" });
  await answer();
  await answer();
  expect(sent.map(([s, m]) => [s, decoded(m)])).toEqual([
    ["s1", "echo"],
    ["s1", " hi"],
  ]);
});

test("a closed sender drops keystrokes held for a session that never opened", () => {
  const { send, attach, close, sent } = harness();
  send(null, { type: "input", data: "x" });
  close();
  attach("s1");
  expect(sent).toEqual([]);
});
