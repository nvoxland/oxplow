import { expect, test } from "bun:test";

import { agentSessionRef, newSessionRef, knowledgePageRef } from "./pageRefs.js";
import { emptySession, open, setStrip } from "./session.js";
import { newThreadTabs, reconcileSessionTabs, storedSessionIsStale } from "./sessionTabs.js";

const knowledgeTab = knowledgePageRef("oxwiki", "a");

/** The thread's open sessions lead its tabs, in the order they opened; a
 *  closed one's tab goes; other tabs keep their order. */
test("open sessions lead the tabs, closed ones go", () => {
  const tabs = [agentSessionRef("ses1"), knowledgeTab, agentSessionRef("ses2")];
  expect(reconcileSessionTabs(tabs, [{ id: "ses2" }, { id: "ses3" }])).toEqual([
    agentSessionRef("ses2"),
    agentSessionRef("ses3"),
    knowledgeTab,
  ]);
});

/** Nothing changed: the same array, so the host doesn't re-render. */
test("unchanged tabs come back as the same array", () => {
  const tabs = [agentSessionRef("ses1"), knowledgeTab];
  expect(reconcileSessionTabs(tabs, [{ id: "ses1" }])).toBe(tabs);
});

/** A session opened later still goes ahead of every other tab. */
test("a session opened later leads the tabs opened before it", () => {
  const tabs = [agentSessionRef("ses1"), knowledgeTab, knowledgePageRef("oxwiki", "b")];
  expect(reconcileSessionTabs(tabs, [{ id: "ses1" }, { id: "ses2" }])).toEqual([
    agentSessionRef("ses1"),
    agentSessionRef("ses2"),
    knowledgeTab,
    knowledgePageRef("oxwiki", "b"),
  ]);
});

/** The picker is an ordinary tab: reconciling never adds one, so a thread
 *  whose picker was closed keeps it closed. */
test("reconciling never brings the picker back", () => {
  const tabs = [knowledgeTab];
  expect(reconcileSessionTabs(tabs, [])).toBe(tabs);
  expect(reconcileSessionTabs([], [])).toEqual([]);
  const withPicker = [newSessionRef(), knowledgeTab];
  expect(reconcileSessionTabs(withPicker, [])).toBe(withPicker);
});

test("a new thread shows the picker without starting an agent", () => {
  expect(newThreadTabs()).toEqual({ tabs: [newSessionRef()], start: null });
});

/** A project reset leaves the browser's stored tabs behind, and a new thread
 *  can get an old thread's id: an entry whose agent-session tabs name sessions
 *  the thread no longer has (none of them is open) belongs to a thread that is
 *  gone, so the thread opens as a new one rather than on tabs that can't load. */
test("a stored entry whose sessions are all gone is stale", () => {
  const stored = setStrip(emptySession(), [agentSessionRef("ses1"), knowledgeTab]);
  expect(storedSessionIsStale(stored, [])).toBe(true);
  expect(storedSessionIsStale(stored, [{ id: "ses9" }])).toBe(true);
});

/** Any of its sessions still open, or none ever stored (a thread whose
 *  sessions were all closed keeps its page tabs): not stale. */
test("a stored entry that still has a session, or never had one, is not stale", () => {
  const stored = setStrip(emptySession(), [agentSessionRef("ses1"), knowledgeTab]);
  expect(storedSessionIsStale(stored, [{ id: "ses1" }])).toBe(false);
  expect(storedSessionIsStale(stored, [{ id: "ses9" }, { id: "ses1" }])).toBe(false);
  expect(storedSessionIsStale(open(emptySession(), { ref: knowledgeTab }), [])).toBe(false);
  expect(storedSessionIsStale(emptySession(), [])).toBe(false);
});
