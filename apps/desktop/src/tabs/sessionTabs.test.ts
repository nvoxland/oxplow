import { expect, test } from "bun:test";

import { agentSessionRef, newSessionRef, wikiPageRef } from "./pageRefs.js";
import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { newThreadTabs, reconcileSessionTabs } from "./sessionTabs.js";

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

/** A session opened later still goes ahead of every other tab. */
test("a session opened later leads the tabs opened before it", () => {
  const tabs = [agentSessionRef("ses1"), wiki, wikiPageRef("b")];
  expect(reconcileSessionTabs(tabs, [{ id: "ses1" }, { id: "ses2" }])).toEqual([
    agentSessionRef("ses1"),
    agentSessionRef("ses2"),
    wiki,
    wikiPageRef("b"),
  ]);
});

/** The picker is an ordinary tab: reconciling never adds one, so a thread
 *  whose picker was closed keeps it closed. */
test("reconciling never brings the picker back", () => {
  const tabs = [wiki];
  expect(reconcileSessionTabs(tabs, [])).toBe(tabs);
  expect(reconcileSessionTabs([], [])).toEqual([]);
  const withPicker = [newSessionRef(), wiki];
  expect(reconcileSessionTabs(withPicker, [])).toBe(withPicker);
});

const harness = (id: string, over: Partial<HarnessListing> = {}): HarnessListing => ({
  id,
  title: id,
  chat: false,
  enabled: true,
  settings: [],
  ...over,
});
const harnesses = [harness("claude"), harness("acp", { chat: true }), harness("codex", { enabled: false })];

/** A new thread follows the person's `newThreadSession`: ask opens the
 *  picker, none opens nothing, an agent starts it. */
test("a new thread asks, starts nothing, or starts the chosen agent", () => {
  expect(newThreadTabs("ask", harnesses)).toEqual({ tabs: [newSessionRef()], start: null });
  expect(newThreadTabs("none", harnesses)).toEqual({ tabs: [], start: null });
  expect(newThreadTabs("claude", harnesses)).toEqual({ tabs: [], start: { harness: "claude", acpAgent: null } });
  expect(newThreadTabs("acp:gemini", harnesses)).toEqual({
    tabs: [],
    start: { harness: "acp", acpAgent: "gemini" },
  });
});

/** An agent that can't start any more — disabled, gone, or a chat harness
 *  without its agent — asks instead. */
test("an agent that isn't available asks instead", () => {
  for (const choice of ["codex", "gone", "acp"]) {
    expect(newThreadTabs(choice, harnesses)).toEqual({ tabs: [newSessionRef()], start: null });
  }
});
