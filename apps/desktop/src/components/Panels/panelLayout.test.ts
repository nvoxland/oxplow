import { expect, test } from "bun:test";

import { CORE_PANELS, hidePanel, movePanel, resolveLayout, revealPanel, setCollapsed, showPanel } from "./panelLayout.js";

const core = CORE_PANELS.map((p) => p.id);

test("no stored layout: every panel in default order, Work collapsed", () => {
  const out = resolveLayout([...core, "ext:gh/prs"], []);
  expect(out.order).toEqual([...core, "ext:gh/prs"]);
  expect(out.hidden).toEqual([]);
  expect([...out.collapsed]).toEqual(["core:work"]);
});

test("the stored layout orders, hides and collapses; unknown ids drop, new ones append", () => {
  const out = resolveLayout(["core:work", "core:comments", "ext:gh/prs"], [
    { panel: "ext:gh/prs", hidden: false, collapsed: true },
    { panel: "core:comments", hidden: true, collapsed: false },
    { panel: "ext:gone/x", hidden: false, collapsed: false },
  ]);
  expect(out.order).toEqual(["ext:gh/prs", "core:work"]);
  expect(out.hidden).toEqual(["core:comments"]);
  expect(out.collapsed.has("ext:gh/prs")).toBe(true);
  expect(out.collapsed.has("core:work")).toBe(true);
});

test("moving, hiding, showing and collapsing produce the next stored layout", () => {
  const avail = ["core:alerts", "core:work", "ext:gh/prs"];
  let layout = movePanel(avail, [], "ext:gh/prs", 0);
  expect(resolveLayout(avail, layout).order).toEqual(["ext:gh/prs", "core:alerts", "core:work"]);
  layout = hidePanel(avail, layout, "core:alerts");
  expect(resolveLayout(avail, layout).hidden).toEqual(["core:alerts"]);
  layout = showPanel(avail, layout, "core:alerts");
  expect(resolveLayout(avail, layout).order).toEqual(["ext:gh/prs", "core:work", "core:alerts"]);
  layout = setCollapsed(avail, layout, "core:work", false);
  expect(resolveLayout(avail, layout).collapsed.has("core:work")).toBe(false);
});

// P6b.A4: proposals wait in their own core panel, right after Alerts.
test("Approvals is a core panel after Alerts, appended to a layout stored before it existed", () => {
  expect(core.slice(0, 2)).toEqual(["core:alerts", "core:approvals"]);
  const out = resolveLayout(core, [
    { panel: "core:work", hidden: false, collapsed: false },
    { panel: "core:alerts", hidden: false, collapsed: false },
  ]);
  expect(out.order[out.order.length - 1]).toBe("core:bookmarks");
  expect(out.order).toContain("core:approvals");
});

test("revealing a panel shows it when hidden and expands it when collapsed", () => {
  const avail = ["core:alerts", "core:approvals"];
  let layout = hidePanel(avail, [], "core:approvals");
  layout = setCollapsed(avail, layout, "core:approvals", true);
  const out = resolveLayout(avail, revealPanel(avail, layout, "core:approvals"));
  expect(out.order).toContain("core:approvals");
  expect(out.collapsed.has("core:approvals")).toBe(false);
});
