import { expect, test } from "bun:test";

import { planCloseOrGoBack, type HistoryEntry } from "./closeOrGoBack.js";
import type { TabRef } from "./tabState.js";

const refA: TabRef = { id: "wiki:a", kind: "wiki", payload: { slug: "a" } };
const refB: TabRef = { id: "file:b.ts", kind: "file", payload: null };
const refC: TabRef = { id: "task:c", kind: "task", payload: { itemId: "c" } };

function entry(over: Partial<HistoryEntry>): HistoryEntry {
  return { back: [], forward: [], siblings: null, ...over };
}

test("closes when the tab has no history entry", () => {
  expect(planCloseOrGoBack(undefined, true)).toEqual({ action: "close" });
});

test("closes when the back stack is empty", () => {
  expect(planCloseOrGoBack(entry({ back: [] }), true)).toEqual({ action: "close" });
});

test("closes when the tab is not present in the list", () => {
  expect(planCloseOrGoBack(entry({ back: [{ ref: refB, siblings: null }] }), false)).toEqual({
    action: "close",
  });
});

test("goes back to the previous page when back-history exists", () => {
  const plan = planCloseOrGoBack(
    entry({ back: [{ ref: refB, siblings: null }] }),
    true,
  );
  expect(plan).toEqual({
    action: "back",
    target: refB,
    nextEntry: { back: [], forward: [], siblings: null },
  });
});

test("does NOT push the deleted page onto forward; preserves the page's own forward stack", () => {
  const plan = planCloseOrGoBack(
    entry({
      back: [{ ref: refA, siblings: null }, { ref: refB, siblings: null }],
      forward: [{ ref: refC, siblings: null }],
    }),
    true,
  );
  // Target is the top of the back stack (refB); refB is popped off back.
  // The deleted page is gone from history entirely — only its existing
  // forward stack (refC) carries over so pages ahead stay reachable.
  expect(plan).toEqual({
    action: "back",
    target: refB,
    nextEntry: {
      back: [{ ref: refA, siblings: null }],
      forward: [{ ref: refC, siblings: null }],
      siblings: null,
    },
  });
});

test("restores the back-target's siblings", () => {
  const sibs = { entries: [{ ref: refB, label: "b" }], index: 0 };
  const plan = planCloseOrGoBack(
    entry({ back: [{ ref: refB, siblings: sibs }] }),
    true,
  );
  expect(plan).toEqual({
    action: "back",
    target: refB,
    nextEntry: { back: [], forward: [], siblings: sibs },
  });
});
