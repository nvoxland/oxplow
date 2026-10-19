import { expect, test } from "bun:test";

import { pageTabRef } from "./pageRefs.js";
import { navigatedHistory } from "./tabHistory.js";

const form = pageTabRef("new-task");
const list = pageTabRef("tasks");
const item = pageTabRef("board");
const old = { back: [{ ref: list, siblings: null }], forward: [{ ref: item, siblings: null }], siblings: null };

test("navigating in a tab puts the page it left on Back and clears Forward", () => {
  expect(navigatedHistory(old, form, item, undefined, false)).toEqual({
    back: [{ ref: list, siblings: null }, { ref: form, siblings: null }],
    forward: [],
    siblings: null,
  });
});

test("replacing leaves the page out of history: Back skips it", () => {
  expect(navigatedHistory(old, form, item, undefined, true)).toEqual({
    back: [{ ref: list, siblings: null }],
    forward: [],
    siblings: null,
  });
});

test("the destination's siblings snap to where it is in the list", () => {
  const siblings = { entries: [{ ref: list, label: "a" }, { ref: item, label: "b" }], index: 0 };
  expect(navigatedHistory(old, form, item, siblings, false).siblings).toEqual({ entries: siblings.entries, index: 1 });
});
