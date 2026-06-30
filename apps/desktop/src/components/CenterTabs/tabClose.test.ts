import { describe, expect, test } from "bun:test";
import { tabsToCloseOthers, tabsToCloseRight } from "./tabClose.js";

const tabs = [
  { id: "agent", closable: false },
  { id: "file:a.ts", closable: true },
  { id: "file:b.ts", closable: true },
  { id: "diff:c", closable: true },
];

describe("tabsToCloseOthers", () => {
  test("returns every closable tab except the anchor, in strip order", () => {
    expect(tabsToCloseOthers(tabs, "file:b.ts")).toEqual(["file:a.ts", "diff:c"]);
  });

  test("never includes pinned (non-closable) tabs", () => {
    // Anchored on the agent tab → all closables are 'others'.
    expect(tabsToCloseOthers(tabs, "agent")).toEqual(["file:a.ts", "file:b.ts", "diff:c"]);
  });

  test("empty when the anchor is the only closable tab", () => {
    expect(tabsToCloseOthers([{ id: "agent", closable: false }, { id: "x", closable: true }], "x")).toEqual([]);
  });
});

describe("tabsToCloseRight", () => {
  test("returns closable tabs after the anchor, in strip order", () => {
    expect(tabsToCloseRight(tabs, "file:a.ts")).toEqual(["file:b.ts", "diff:c"]);
  });

  test("skips pinned tabs to the right (none here) and respects order", () => {
    expect(tabsToCloseRight(tabs, "file:b.ts")).toEqual(["diff:c"]);
  });

  test("empty when the anchor is the last tab", () => {
    expect(tabsToCloseRight(tabs, "diff:c")).toEqual([]);
  });

  test("empty for an unknown anchor", () => {
    expect(tabsToCloseRight(tabs, "nope")).toEqual([]);
  });
});
