import { describe, expect, test } from "bun:test";

import type { AcpEvent, AcpSnapshot, TranscriptItem } from "../../api.js";
import {
  applyEvent,
  contextPercent,
  fromSnapshot,
  initialState,
  isBusy,
  mergeSnapshot,
  openPermissions,
} from "./acpTranscript.js";

const agent = (id: number, seq: number, text: string): TranscriptItem => ({
  id,
  seq,
  type: "agent",
  text,
});

const snap = (items: TranscriptItem[], over: Partial<AcpSnapshot> = {}): AcpSnapshot => ({
  agent: "fake",
  status: "idle",
  directive: null,
  usage: null,
  headSeq: Math.max(0, ...items.map((i) => i.seq)),
  items,
  stderrTail: [],
  ...over,
});

const itemEvent = (item: TranscriptItem): AcpEvent => ({ threadId: "thr1", type: "item", item });

describe("acp transcript reducer", () => {
  test("a snapshot seeds the state in id order", () => {
    const s = fromSnapshot(snap([agent(2, 3, "b"), agent(1, 2, "a")]));
    expect(s.items.map((i) => i.id)).toEqual([1, 2]);
    expect(s.headSeq).toBe(3);
    expect(s.status).toBe("idle");
    expect(s.stale).toBe(false);
  });

  test("an item event upserts by id and advances the head", () => {
    let s = fromSnapshot(snap([agent(1, 1, "he")]));
    s = applyEvent(s, itemEvent(agent(1, 2, "hello")));
    s = applyEvent(s, itemEvent(agent(2, 3, "next")));
    expect(s.items.map((i) => (i.type === "agent" ? i.text : ""))).toEqual(["hello", "next"]);
    expect(s.headSeq).toBe(3);
    expect(s.stale).toBe(false);
  });

  test("an older copy of an item never overwrites a newer one", () => {
    let s = fromSnapshot(snap([agent(1, 5, "new")]));
    s = applyEvent(s, itemEvent(agent(1, 4, "old")));
    expect(s.items[0]).toEqual(agent(1, 5, "new"));
  });

  test("a gap in seqs marks the state stale until a since-fetch merges", () => {
    let s = fromSnapshot(snap([agent(1, 1, "a")]));
    s = applyEvent(s, itemEvent(agent(3, 4, "c")));
    expect(s.stale).toBe(true);
    // The head stays at the last seq seen without a gap, so the refetch
    // (\`since(headSeq)\`) asks for the missed items too.
    expect(s.headSeq).toBe(1);
    s = mergeSnapshot(s, snap([agent(2, 2, "b"), agent(3, 4, "c")], { headSeq: 4 }));
    expect(s.stale).toBe(false);
    expect(s.items.map((i) => i.id)).toEqual([1, 2, 3]);
    expect(s.headSeq).toBe(4);
  });

  test("status, directive, usage and closed events set session state", () => {
    let s = initialState();
    expect(s.status).toBe("starting");
    s = applyEvent(s, { threadId: "thr1", type: "status", status: "running" });
    expect(isBusy(s.status)).toBe(true);
    s = applyEvent(s, { threadId: "thr1", type: "directive", text: "Close the task." });
    expect(s.directive).toBe("Close the task.");
    s = applyEvent(s, { threadId: "thr1", type: "directive", text: null });
    expect(s.directive).toBeNull();
    s = applyEvent(s, {
      threadId: "thr1",
      type: "usage",
      usage: { used: 50, size: 200, costAmount: null, costCurrency: null },
    });
    expect(contextPercent(s.usage)).toBe(25);
    s = applyEvent(s, { threadId: "thr1", type: "closed", reason: "the agent exited" });
    expect(s.status).toBe("stopped");
    expect(s.closedReason).toBe("the agent exited");
    expect(isBusy(s.status)).toBe(false);
  });

  test("open permissions are the unanswered cards", () => {
    const card = (id: number, answered: boolean): TranscriptItem => ({
      id,
      seq: id,
      type: "permission",
      requestId: `perm-${id}`,
      toolCallId: "t",
      title: "Edit a.rs",
      options: [],
      answer: answered ? { type: "cancelled" } : null,
    });
    const s = fromSnapshot(snap([card(1, true), card(2, false)]));
    expect(openPermissions(s).map((i) => i.id)).toEqual([2]);
  });

  test("contextPercent is null without a size", () => {
    expect(contextPercent(null)).toBeNull();
    expect(contextPercent({ used: 1, size: 0, costAmount: null, costCurrency: null })).toBeNull();
  });
});

describe("session generations (Restart)", () => {
  test("a new generation's snapshot replaces the old transcript", () => {
    let s = fromSnapshot(snap([agent(1, 9, "old one"), agent(2, 10, "old two")], { generation: 1 }));
    s = mergeSnapshot(s, snap([agent(1, 1, "new")], { generation: 2 }));
    expect(s.items.map((i) => (i.type === "agent" ? i.text : ""))).toEqual(["new"]);
    expect(s.headSeq).toBe(1);
  });

  test("a new generation's event starts fresh instead of losing to old seqs", () => {
    let s = fromSnapshot(snap([agent(1, 9, "old")], { generation: 1 }));
    s = applyEvent(s, { threadId: "thr1", generation: 2, type: "item", item: agent(1, 1, "new") });
    expect(s.items.map((i) => (i.type === "agent" ? i.text : ""))).toEqual(["new"]);
    expect(s.generation).toBe(2);
    expect(s.stale).toBe(false);
  });
});
