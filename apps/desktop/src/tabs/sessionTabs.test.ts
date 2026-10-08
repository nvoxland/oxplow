import { expect, test } from "bun:test";

import { agentSessionRef, newSessionRef, wikiPageRef } from "./pageRefs.js";
import { reconcileSessionTabs } from "./sessionTabs.js";

const wiki = wikiPageRef("a");

/** The thread's open sessions lead its tabs, in the order they opened; a
 *  closed one's tab goes; other tabs keep their order. */
test("open sessions lead the tabs, closed ones go", () => {
  const tabs = [agentSessionRef("ses1"), wiki, agentSessionRef("ses2")];
  expect(reconcileSessionTabs(tabs, [{ id: "ses2" }, { id: "ses3" }])).toEqual([
    agentSessionRef("ses2"),
    agentSessionRef("ses3"),
    wiki,
  ]);
});

/** Nothing changed: the same array, so the host doesn't re-render. */
test("unchanged tabs come back as the same array", () => {
  const tabs = [agentSessionRef("ses1"), wiki];
  expect(reconcileSessionTabs(tabs, [{ id: "ses1" }])).toBe(tabs);
});

/** A thread with no session shows the session picker first, once. */
test("with no session the picker leads", () => {
  expect(reconcileSessionTabs([wiki], [])).toEqual([newSessionRef(), wiki]);
  const withPicker = [newSessionRef(), wiki];
  expect(reconcileSessionTabs(withPicker, [])).toBe(withPicker);
});
