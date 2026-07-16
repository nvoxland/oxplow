import { describe, expect, it } from "bun:test";

import {
  allCollapsed,
  allExpanded,
  isExpanded,
  parseCollapsed,
  serializeCollapsed,
  toggleCollapsed,
} from "./sectionCollapse.js";

describe("isExpanded / toggleCollapsed", () => {
  it("defaults a never-seen section to expanded", () => {
    expect(isExpanded(new Set(), "testing")).toBe(true);
  });

  it("toggles one section without disturbing the others", () => {
    const collapsed = toggleCollapsed(new Set(["coverage"]), "testing");
    expect([...collapsed].sort()).toEqual(["coverage", "testing"]);
    expect(isExpanded(collapsed, "testing")).toBe(false);
    expect(isExpanded(collapsed, "coverage")).toBe(false);
    expect(isExpanded(collapsed, "operational")).toBe(true);
  });

  it("toggles a collapsed section back open", () => {
    const collapsed = toggleCollapsed(new Set(["testing", "coverage"]), "testing");
    expect([...collapsed]).toEqual(["coverage"]);
    expect(isExpanded(collapsed, "testing")).toBe(true);
  });

  it("does not mutate the incoming set", () => {
    const before = new Set(["testing"]);
    toggleCollapsed(before, "coverage");
    expect([...before]).toEqual(["testing"]);
  });
});

describe("allExpanded / allCollapsed", () => {
  const ids = ["testing", "coverage", "operational"];

  it("reads all-expanded when nothing rendered is collapsed", () => {
    expect(allExpanded(ids, new Set())).toBe(true);
    expect(allCollapsed(ids, new Set())).toBe(false);
  });

  it("reads all-collapsed only when every rendered section is collapsed", () => {
    expect(allCollapsed(ids, new Set(["testing", "coverage"]))).toBe(false);
    expect(allCollapsed(ids, new Set(ids))).toBe(true);
    expect(allExpanded(ids, new Set(ids))).toBe(false);
  });

  it("ignores collapsed ids that aren't currently rendered", () => {
    // A search can hide a section entirely; its remembered collapse must not
    // make the visible ones read as all-collapsed (which would wrongly disable
    // Collapse all).
    expect(allCollapsed(["testing"], new Set(["testing", "gone"]))).toBe(true);
    expect(allExpanded(["testing"], new Set(["gone"]))).toBe(true);
  });

  it("treats an empty section list as neither (nothing to act on)", () => {
    expect(allExpanded([], new Set())).toBe(false);
    expect(allCollapsed([], new Set())).toBe(false);
  });
});

describe("parseCollapsed / serializeCollapsed", () => {
  it("round-trips one page's collapsed ids", () => {
    const raw = serializeCollapsed(null, "metrics-recorded", new Set(["testing"]));
    expect(parseCollapsed(raw, "metrics-recorded")).toEqual(new Set(["testing"]));
  });

  it("keeps other pages' entries when writing one page", () => {
    const first = serializeCollapsed(null, "metrics-recorded", new Set(["testing"]));
    const both = serializeCollapsed(first, "metrics-catalog", new Set(["coverage"]));
    expect(parseCollapsed(both, "metrics-recorded")).toEqual(new Set(["testing"]));
    expect(parseCollapsed(both, "metrics-catalog")).toEqual(new Set(["coverage"]));
  });

  it("remembers an id that isn't currently rendered", () => {
    // Deliberately NOT reconciled against the known set (unlike the rail's
    // section ORDER): a section hidden by a search filter must come back
    // collapsed, so an unknown id survives a round-trip.
    const raw = serializeCollapsed(null, "p", new Set(["hidden-by-search"]));
    expect(parseCollapsed(raw, "p")).toEqual(new Set(["hidden-by-search"]));
  });

  it("reads missing / malformed / wrong-shaped storage as nothing collapsed", () => {
    expect(parseCollapsed(null, "p")).toEqual(new Set());
    expect(parseCollapsed("not json", "p")).toEqual(new Set());
    expect(parseCollapsed(JSON.stringify({ p: "nope" }), "p")).toEqual(new Set());
    expect(parseCollapsed(JSON.stringify({ other: ["x"] }), "p")).toEqual(new Set());
    expect(parseCollapsed(JSON.stringify(["x"]), "p")).toEqual(new Set());
  });

  it("drops non-string members rather than trusting storage", () => {
    expect(parseCollapsed(JSON.stringify({ p: ["ok", 3, null] }), "p")).toEqual(new Set(["ok"]));
  });
});
