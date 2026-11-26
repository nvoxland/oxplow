import { expect, test } from "bun:test";

import { continueItems, continueRefs } from "./continueFrom.js";

const session = (tabs: { id: string; pinned?: boolean }[], active: string | null) =>
  JSON.stringify({
    tabs: tabs.map((t, i) => ({ slot: `t${i}`, entries: [{ ref: { kind: "page", id: t.id } }], at: 0, ...(t.pinned ? { pinned: true } : {}) })),
    active,
    mru: [],
    next: tabs.length,
  });

// Continue from a device: its page tabs in its order, the one it shows last
// (so it is shown here); its agent sessions are everyone's already.
test("the pages another device has open, its shown one last, no sessions", () => {
  const refs = continueRefs(session([{ id: "agent", pinned: true }, { id: "a" }, { id: "b" }, { id: "c" }], "t2"));
  expect(refs.map((r) => r.id)).toEqual(["a", "c", "b"]);
  expect(continueRefs("not json")).toEqual([]);
});

test("one item per device with pages, named by the device", () => {
  const opened: string[] = [];
  const items = continueItems(
    [
      { device: "dev-1", name: "Mac", sessionJson: session([{ id: "a" }], "t0") },
      { device: "dev-2", name: null, sessionJson: session([{ id: "agent", pinned: true }], "t0") },
      { device: "dev-3", name: null, sessionJson: session([{ id: "x" }], null) },
    ],
    (ref) => opened.push(ref.id),
  );
  expect(items.map((i) => i.label)).toEqual(["Mac", "Another device"]);
  void items[0]!.run!();
  expect(opened).toEqual(["a"]);
});
