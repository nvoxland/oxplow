import { expect, test } from "bun:test";

import { CORE_PANELS, hidePanel, movePanel, resolveLayout, showPanel, toggleCollapsed } from "./panelLayout.js";

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
  layout = toggleCollapsed(avail, layout, "core:work");
  expect(resolveLayout(avail, layout).collapsed.has("core:work")).toBe(false);
});
