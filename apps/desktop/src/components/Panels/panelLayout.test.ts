import { expect, test } from "bun:test";

import type { PanelPlacement } from "../../tauri-bridge/generated/bindings.js";
import {
  hidePanel,
  layoutSync,
  movePanelBeside,
  resolveLayout,
  revealPanel,
  setCollapsed,
  showPanel,
} from "./panelLayout.js";

test("no stored layout: every panel in default order, all open", () => {
  const out = resolveLayout(["ext:acme/inbox", "ext:gh/prs"], []);
  expect(out.order).toEqual(["ext:acme/inbox", "ext:gh/prs"]);
  expect(out.hidden).toEqual([]);
  // Work too (tsk1045): collapsed, it hid the task a person had just added.
  expect([...out.collapsed]).toEqual([]);
});

test("the stored layout orders, hides and collapses; unknown ids drop, new ones append", () => {
  const out = resolveLayout(["ext:acme/inbox", "ext:bundled/go-to", "ext:gh/prs"], [
    { panel: "ext:gh/prs", hidden: false, collapsed: true },
    { panel: "ext:bundled/go-to", hidden: true, collapsed: false },
    { panel: "ext:gone/x", hidden: false, collapsed: false },
  ]);
  expect(out.order).toEqual(["ext:gh/prs", "ext:acme/inbox"]);
  expect(out.hidden).toEqual(["ext:bundled/go-to"]);
  expect(out.collapsed.has("ext:gh/prs")).toBe(true);
  expect(out.collapsed.has("ext:acme/inbox")).toBe(false);
});

test("moving, hiding, showing and collapsing produce the next stored layout", () => {
  const avail = ["core:alerts", "ext:acme/inbox", "ext:gh/prs"];
  let layout = movePanelBeside(avail, [], "ext:gh/prs", "core:alerts", "before");
  expect(resolveLayout(avail, layout).order).toEqual(["ext:gh/prs", "core:alerts", "ext:acme/inbox"]);
  layout = hidePanel(avail, layout, "core:alerts");
  expect(resolveLayout(avail, layout).hidden).toEqual(["core:alerts"]);
  layout = showPanel(avail, layout, "core:alerts");
  expect(resolveLayout(avail, layout).order).toEqual(["ext:gh/prs", "ext:acme/inbox", "core:alerts"]);
  layout = setCollapsed(avail, layout, "ext:acme/inbox", false);
  expect(resolveLayout(avail, layout).collapsed.has("ext:acme/inbox")).toBe(false);
});

// P6b.A4: proposals wait in their own core panel, right after Alerts.
test("revealing a panel shows it when hidden and expands it when collapsed", () => {
  const avail = ["core:alerts", "ext:acme/inbox"];
  let layout = hidePanel(avail, [], "ext:acme/inbox");
  layout = setCollapsed(avail, layout, "ext:acme/inbox", true);
  const out = resolveLayout(avail, revealPanel(avail, layout, "ext:acme/inbox"));
  expect(out.order).toContain("ext:acme/inbox");
  expect(out.collapsed.has("ext:acme/inbox")).toBe(false);
});

// tsk998: an edit made before an extension's panels are available keeps
// where the person put them.
test("an edit keeps the placements of panels not available yet", () => {
  const stored = [
    { panel: "ext:gh/prs", hidden: false, collapsed: true },
    { panel: "core:alerts", hidden: false, collapsed: false },
    { panel: "ext:acme/inbox", hidden: false, collapsed: false },
  ];
  const next = hidePanel(["core:alerts", "ext:acme/inbox"], stored, "core:alerts");
  const all = resolveLayout(["core:alerts", "ext:acme/inbox", "ext:gh/prs"], next);
  expect(all.order).toEqual(["ext:gh/prs", "ext:acme/inbox"]);
  expect(all.hidden).toEqual(["core:alerts"]);
  expect(all.collapsed.has("ext:gh/prs")).toBe(true);
});

test("a move means beside its target, wherever the layout it's replayed on puts that", () => {
  const avail = ["core:alerts", "ext:acme/inbox", "ext:gh/prs"];
  // Dragged on the defaults (alerts, work, prs): prs before work. Replayed
  // on the stored layout, it still lands right before work.
  const stored = [
    { panel: "ext:acme/inbox", hidden: false, collapsed: false },
    { panel: "core:alerts", hidden: false, collapsed: false },
    { panel: "ext:gh/prs", hidden: false, collapsed: false },
  ];
  expect(resolveLayout(avail, movePanelBeside(avail, stored, "ext:gh/prs", "ext:acme/inbox", "before")).order).toEqual([
    "ext:gh/prs",
    "ext:acme/inbox",
    "core:alerts",
  ]);
  expect(resolveLayout(avail, movePanelBeside(avail, stored, "ext:acme/inbox", "ext:gh/prs", "after")).order).toEqual([
    "core:alerts",
    "ext:gh/prs",
    "ext:acme/inbox",
  ]);
});

/** A sync over fakes: `loads` answer in turn (an Error rejects); each save
 *  waits until `settle()`. */
function fakeSync(loads: Array<PanelPlacement[] | Error>) {
  const saved: PanelPlacement[][] = [];
  const pendingSaves: Array<() => void> = [];
  const shown: PanelPlacement[][] = [];
  const errors: string[] = [];
  let ready = 0;
  let loadCalls = 0;
  const sync = layoutSync({
    load: async () => {
      const next = loads[loadCalls++];
      if (next instanceof Error || next === undefined) throw next ?? new Error("no more loads");
      return next;
    },
    save: (layout) => {
      saved.push(layout);
      return new Promise<void>((r) => pendingSaves.push(r));
    },
    show: (layout) => shown.push(layout),
    ready: () => {
      ready += 1;
    },
    failed: (action) => errors.push(action),
  });
  const tick = () => new Promise((r) => setTimeout(r, 0));
  const settle = async () => {
    pendingSaves.shift()?.();
    await tick();
  };
  return { sync, saved, shown, errors, tick, settle, ready: () => ready, loadCalls: () => loadCalls };
}

const hideAlerts = (base: PanelPlacement[]) => hidePanel(["core:alerts", "ext:acme/inbox"], base, "core:alerts");
const collapseWork = (base: PanelPlacement[]) => setCollapsed(["core:alerts", "ext:acme/inbox"], base, "ext:acme/inbox", true);
const stored = [
  { panel: "ext:acme/inbox", hidden: false, collapsed: false },
  { panel: "core:alerts", hidden: false, collapsed: false },
];

test("an edit made before the layout loads is shown at once, then replayed on it and saved", async () => {
  const f = fakeSync([stored]);
  f.sync.edit(hideAlerts);
  expect(f.saved).toEqual([]);
  expect(resolveLayout(["core:alerts", "ext:acme/inbox"], f.shown.at(-1)!).hidden).toEqual(["core:alerts"]);
  await f.tick();
  expect(f.ready()).toBe(1);
  expect(f.saved).toEqual([hideAlerts(stored)]);
});

test("a failed load saves nothing over the layout it couldn't read, and the next edit loads again", async () => {
  const f = fakeSync([new Error("busy"), stored]);
  await f.tick();
  expect(f.errors).toEqual(["Load the panel layout"]);
  expect(f.ready()).toBe(1);
  f.sync.edit(hideAlerts);
  await f.tick();
  expect(f.loadCalls()).toBe(2);
  // Replayed on what was stored, not on nothing.
  expect(f.saved).toEqual([hideAlerts(stored)]);
});

test("saves go one at a time, and the latest layout is the last saved", async () => {
  const f = fakeSync([stored]);
  await f.tick();
  f.sync.edit(hideAlerts);
  f.sync.edit(collapseWork);
  f.sync.edit((base) => showPanel(["core:alerts", "ext:acme/inbox"], base, "core:alerts"));
  expect(f.saved.length).toBe(1);
  await f.settle();
  expect(f.saved.length).toBe(2);
  expect(f.saved[1]).toEqual(f.shown.at(-1)!);
  await f.settle();
  expect(f.saved.length).toBe(2);
});
