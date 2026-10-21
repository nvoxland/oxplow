import { describe, expect, test } from "bun:test";
import { tabsToCloseOthers, tabsToCloseRight } from "./tabClose.js";

const tabs = [
  { id: "oxplow:oxplow/page?page=agent", closable: false },
  { id: "oxplow:oxplow/file?path=a.ts", closable: true },
  { id: "oxplow:oxplow/file?path=b.ts", closable: true },
  { id: "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk", closable: true },
];

describe("tabsToCloseOthers", () => {
  test("returns every closable tab except the anchor, in strip order", () => {
    expect(tabsToCloseOthers(tabs, "oxplow:oxplow/file?path=b.ts")).toEqual(["oxplow:oxplow/file?path=a.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
  });

  test("never includes pinned (non-closable) tabs", () => {
    // Anchored on the agent tab → all closables are 'others'.
    expect(tabsToCloseOthers(tabs, "oxplow:oxplow/page?page=agent")).toEqual(["oxplow:oxplow/file?path=a.ts", "oxplow:oxplow/file?path=b.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
  });

  test("empty when the anchor is the only closable tab", () => {
    expect(tabsToCloseOthers([{ id: "oxplow:oxplow/page?page=agent", closable: false }, { id: "x", closable: true }], "x")).toEqual([]);
  });
});

describe("tabsToCloseRight", () => {
  test("returns closable tabs after the anchor, in strip order", () => {
    expect(tabsToCloseRight(tabs, "oxplow:oxplow/file?path=a.ts")).toEqual(["oxplow:oxplow/file?path=b.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
  });

  test("skips pinned tabs to the right (none here) and respects order", () => {
    expect(tabsToCloseRight(tabs, "oxplow:oxplow/file?path=b.ts")).toEqual(["oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
  });

  test("empty when the anchor is the last tab", () => {
    expect(tabsToCloseRight(tabs, "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk")).toEqual([]);
  });

  test("empty for an unknown anchor", () => {
    expect(tabsToCloseRight(tabs, "nope")).toEqual([]);
  });
});
