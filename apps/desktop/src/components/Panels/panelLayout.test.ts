import { expect, test } from "bun:test";

import type { PanelPlacement, ScopedPanelLayout } from "../../tauri-bridge/generated/bindings.js";
import {
  hidePanel,
  layoutSync,
  movePanelBeside,
  movePanelToGroup,
  resolveLayout,
  revealPanel,
  setCollapsed,
  showPanel,
} from "./panelLayout.js";

/** Only the project's list. */
const proj = (project: PanelPlacement[]): ScopedPanelLayout => ({ thread: [], stream: [], project });
const EMPTY = proj([]);
const shown = (panel: string, collapsed = false): PanelPlacement => ({ panel, hidden: false, collapsed });

test("no stored layout: every panel in default order, all open", () => {
  const out = resolveLayout(["ext:acme/inbox", "ext:gh/prs"], EMPTY);
  expect(out.order).toEqual(["ext:acme/inbox", "ext:gh/prs"]);
  expect(out.hidden).toEqual([]);
  // Work too: collapsed, it hid the task a person had just added.
  expect([...out.collapsed]).toEqual([]);
});

test("the stored layout orders, hides and collapses; unknown ids drop, new ones append", () => {
  const out = resolveLayout(["ext:acme/inbox", "ext:bundled/go-to", "ext:gh/prs"], proj([
    { panel: "ext:gh/prs", hidden: false, collapsed: true },
    { panel: "ext:bundled/go-to", hidden: true, collapsed: false },
    { panel: "ext:gone/x", hidden: false, collapsed: false },
  ]));
  expect(out.order).toEqual(["ext:gh/prs", "ext:acme/inbox"]);
  expect(out.hidden).toEqual(["ext:bundled/go-to"]);
  expect(out.collapsed.has("ext:gh/prs")).toBe(true);
  expect(out.collapsed.has("ext:acme/inbox")).toBe(false);
});

test("moving, hiding, showing and collapsing produce the next stored layout", () => {
  const avail = ["core:alerts", "ext:acme/inbox", "ext:gh/prs"];
  let layout = movePanelBeside(avail, EMPTY, "ext:gh/prs", "core:alerts", "before");
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
  let layout = hidePanel(avail, EMPTY, "ext:acme/inbox");
  layout = setCollapsed(avail, layout, "ext:acme/inbox", true);
  const out = resolveLayout(avail, revealPanel(avail, layout, "ext:acme/inbox"));
  expect(out.order).toContain("ext:acme/inbox");
  expect(out.collapsed.has("ext:acme/inbox")).toBe(false);
});

// an edit made before an extension's panels are available keeps
// where the person put them.
test("an edit keeps the placements of panels not available yet", () => {
  const stored = proj([
    { panel: "ext:gh/prs", hidden: false, collapsed: true },
    { panel: "core:alerts", hidden: false, collapsed: false },
    { panel: "ext:acme/inbox", hidden: false, collapsed: false },
  ]);
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
  const stored = proj([
    { panel: "ext:acme/inbox", hidden: false, collapsed: false },
    { panel: "core:alerts", hidden: false, collapsed: false },
    { panel: "ext:gh/prs", hidden: false, collapsed: false },
  ]);
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
function fakeSync(loads: Array<ScopedPanelLayout | Error>) {
  const saved: ScopedPanelLayout[] = [];
  const pendingSaves: Array<() => void> = [];
  const shown: ScopedPanelLayout[] = [];
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

const hideAlerts = (base: ScopedPanelLayout) => hidePanel(["core:alerts", "ext:acme/inbox"], base, "core:alerts");
const collapseWork = (base: ScopedPanelLayout) => setCollapsed(["core:alerts", "ext:acme/inbox"], base, "ext:acme/inbox", true);
const stored = proj([
  { panel: "ext:acme/inbox", hidden: false, collapsed: false },
  { panel: "core:alerts", hidden: false, collapsed: false },
]);

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

// The groups: this thread's, this stream's, then the project's.
const AVAIL = ["ext:a/go", "ext:a/git", "ext:a/work", "ext:a/prs"];

test("the rail is the thread's group, then the stream's, then the project's", () => {
  const out = resolveLayout(AVAIL, {
    thread: [shown("ext:a/prs")],
    stream: [shown("ext:a/git", true)],
    project: [shown("ext:a/go"), { panel: "ext:a/prs", hidden: true, collapsed: false }, { panel: "ext:a/git", hidden: true, collapsed: false }],
  });
  expect(out.groups).toEqual({ thread: ["ext:a/prs"], stream: ["ext:a/git"], project: ["ext:a/go", "ext:a/work"] });
  expect(out.order).toEqual(["ext:a/prs", "ext:a/git", "ext:a/go", "ext:a/work"]);
  expect(out.groupOf.get("ext:a/git")).toBe("stream");
  expect(out.collapsed.has("ext:a/git")).toBe(true);
  expect(out.hidden).toEqual([]);
});

test("a panel placed on another thread is hidden here, not new", () => {
  const out = resolveLayout(AVAIL, proj([shown("ext:a/go"), { panel: "ext:a/prs", hidden: true, collapsed: false }]));
  expect(out.hidden).toEqual(["ext:a/prs"]);
  expect(out.groups.project).toEqual(["ext:a/go", "ext:a/git", "ext:a/work"]);
});

test("a move beside a panel puts it in that panel's group, once", () => {
  const base: ScopedPanelLayout = { thread: [shown("ext:a/prs")], stream: [], project: [shown("ext:a/go"), shown("ext:a/git")] };
  const next = movePanelBeside(AVAIL, base, "ext:a/git", "ext:a/prs", "after");
  const out = resolveLayout(AVAIL, next);
  expect(out.groups.thread).toEqual(["ext:a/prs", "ext:a/git"]);
  expect(out.groups.project).toEqual(["ext:a/go", "ext:a/work"]);
  // Its project row stays, hidden: placed, not new elsewhere.
  expect(next.project.find((p) => p.panel === "ext:a/git")?.hidden).toBe(true);
});

test("a move to a group puts it at that group's end, even an empty one", () => {
  let next = movePanelToGroup(AVAIL, EMPTY, "ext:a/work", "stream");
  expect(resolveLayout(AVAIL, next).groups.stream).toEqual(["ext:a/work"]);
  next = movePanelToGroup(AVAIL, next, "ext:a/work", "project");
  const out = resolveLayout(AVAIL, next);
  expect(out.groups.stream).toEqual([]);
  expect(out.groups.project).toEqual(["ext:a/go", "ext:a/git", "ext:a/prs", "ext:a/work"]);
});

test("hiding takes it out of its group; showing puts it at the project's end", () => {
  let next = hidePanel(AVAIL, { thread: [shown("ext:a/prs")], stream: [], project: [] }, "ext:a/prs");
  let out = resolveLayout(AVAIL, next);
  expect(out.groups.thread).toEqual([]);
  expect(out.hidden).toEqual(["ext:a/prs"]);
  next = showPanel(AVAIL, next, "ext:a/prs");
  out = resolveLayout(AVAIL, next);
  expect(out.order.at(-1)).toBe("ext:a/prs");
  expect(out.groupOf.get("ext:a/prs")).toBe("project");
});

test("collapsing a thread's panel collapses its row there", () => {
  const next = setCollapsed(AVAIL, { thread: [shown("ext:a/prs")], stream: [], project: [] }, "ext:a/prs", true);
  expect(next.thread).toEqual([shown("ext:a/prs", true)]);
});
