import { expect, test } from "bun:test";

import type { MenuItem } from "../menu.js";
import { barActions, pageMenuItems } from "./pageMenuItems.js";

const ids = (items: MenuItem[]) => items.map((i) => (i.separator ? "—" : i.id));
const noop = () => {};

test("standard actions come first in their own order, then the custom ones sorted, then Open With and the panels", () => {
  const items = pageMenuItems({
    actions: [
      { id: "save", label: "Save", run: noop, primary: true },
      { id: "new", label: "New", run: noop, prominent: true },
      { id: "refresh", label: "Refresh", run: noop, standard: true, icon: "refresh-cw" },
      { id: "zip", label: "zip it", run: noop },
      { id: "export", label: "Export", run: noop, busy: true },
      { id: "copy", label: "Copy", run: noop, standard: true },
      { id: "csv", label: "As CSV", run: noop, submenu: "Download" },
      { id: "json", label: "As JSON", run: noop, submenu: "Download" },
    ],
    ask: { askAssistant: noop },
    context: { add: noop },
    commands: [
      { id: "tracker.sync", label: "Sync", group: "Tracker", run: noop },
      { id: "tracker.archive", label: "Archive", group: "Tracker", run: noop },
      { id: "dl.pdf", label: "As PDF", group: "Download", run: noop },
    ],
    openWith: { options: [{ name: "a/text", title: "Text" }, { name: "a/rich", title: "Formatted" }], current: "a/rich", pick: noop },
    backlinks: 2,
    outbound: 0,
    pin: { disabledReason: null },
    openPanel: noop,
  });
  // The primary and prominent actions stay buttons in the bar, never in the menu.
  expect(ids(items)).toEqual([
    "page-ask-assistant",
    "page-add-to-context",
    "action-refresh",
    "action-copy",
    "—",
    "page-submenu-Download",
    "action-export",
    "page-submenu-Tracker",
    "action-zip",
    "—",
    "page-open-with",
    "—",
    "page-backlinks",
    "page-outbound",
    "page-pin",
  ]);
  const export_ = items.find((i) => i.id === "action-export")!;
  expect(export_.label).toBe("Export…");
  expect(export_.enabled).toBe(false);
  expect(items.find((i) => i.id === "action-refresh")!.icon).toBe("refresh-cw");
  // A page action and a command naming the same submenu share it, sorted.
  expect(items.find((i) => i.id === "page-submenu-Download")!.submenu!.map((i) => i.label)).toEqual(["As CSV", "As JSON", "As PDF"]);
  expect(items.find((i) => i.id === "page-submenu-Tracker")!.submenu!.map((i) => i.id)).toEqual([
    "ref-command-tracker.archive",
    "ref-command-tracker.sync",
  ]);
  expect(items.find((i) => i.id === "page-backlinks")!.label).toBe("Backlinks (2)…");
  expect(items.find((i) => i.id === "page-outbound")!.enabled).toBe(false);
  const openWith = items.find((i) => i.id === "page-open-with")!;
  expect(openWith.submenu!.map((i) => [i.label, i.checked])).toEqual([
    ["Text", false],
    ["Formatted", true],
  ]);
  const ask = items.find((i) => i.id === "page-ask-assistant")!;
  expect([ask.label, ask.icon]).toEqual(["Ask the Assistant", "sparkles"]);
});

test("the items run what they stand for", () => {
  const ran: string[] = [];
  const items = pageMenuItems({
    ask: { askAssistant: () => ran.push("ask") },
    commands: [{ id: "x.y", label: "Y", group: "X", run: () => ran.push("command") }],
    openWith: { options: [{ name: "a/b", title: "B" }, { name: "a/c", title: "C" }], current: "a/b", pick: (n) => ran.push(`pick:${n}`) },
    backlinks: 1,
    pin: {},
    openPanel: (p) => ran.push(`panel:${p}`),
  });
  const find = (id: string, list: MenuItem[] = items): MenuItem =>
    list.flatMap((i) => [i, ...(i.submenu ?? [])]).find((i) => i.id === id)!;
  for (const id of ["page-ask-assistant", "ref-command-x.y", "page-open-with-a/c", "page-backlinks", "page-pin"]) {
    void find(id).run?.();
  }
  expect(ran).toEqual(["ask", "command", "pick:a/c", "panel:backlinks", "panel:pin"]);
});

test("a page that offers nothing gets no menu, and Open With needs two renderers", () => {
  expect(pageMenuItems({ openPanel: noop })).toEqual([]);
  expect(
    pageMenuItems({ openWith: { options: [{ name: "a/b", title: "B" }], current: "a/b", pick: noop }, openPanel: noop }),
  ).toEqual([]);
  expect(pageMenuItems({ actions: [{ id: "save", label: "Save", run: noop, primary: true }], openPanel: noop })).toEqual([]);
});

test("a pin the page can't make says why and doesn't open", () => {
  const [pin] = pageMenuItems({ pin: { disabledReason: "Run the query first" }, openPanel: noop });
  expect(pin.enabled).toBe(false);
  expect(pin.label).toBe("Pin to Dashboard… (Run the query first)");
});

test("the primary and prominent actions stay in the bar", () => {
  expect(
    barActions([
      { id: "a", label: "A", run: noop },
      { id: "b", label: "B", run: noop, primary: true },
      { id: "c", label: "C", run: noop, prominent: true },
      { id: "d", label: "D", run: noop, prominent: true },
    ]).map((a) => a.id),
  ).toEqual(["b", "c", "d"]);
});

// Add to Context is oxplow's own entry, next to Ask: the page goes in the
// thread's basket; with no thread selected it says why it can't.
test("Add to Context follows Ask and is disabled with a reason when no thread is selected", () => {
  const ran: string[] = [];
  const on = pageMenuItems({ ask: { askAssistant: noop }, context: { add: () => ran.push("add") }, openPanel: noop });
  expect(ids(on)).toEqual(["page-ask-assistant", "page-add-to-context"]);
  const item = on.find((i) => i.id === "page-add-to-context")!;
  expect(item.label).toBe("Add to Context");
  expect(item.icon).toBe("at-sign");
  item.run!();
  expect(ran).toEqual(["add"]);
  const off = pageMenuItems({ context: { add: noop, disabledReason: "Select a thread first" }, openPanel: noop });
  expect(off[0]).toMatchObject({ id: "page-add-to-context", enabled: false, label: "Add to Context (Select a thread first)" });
});
