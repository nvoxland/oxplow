import { expect, test } from "bun:test";

import type { MenuItem } from "../menu.js";
import { barActions, pageMenuItems } from "./pageMenuItems.js";

const ids = (items: MenuItem[]) => items.map((i) => (i.separator ? "—" : i.id));
const noop = () => {};

test("the menu lists the page's actions, then Ask, the commands, Open With and the panels, split by separators", () => {
  const items = pageMenuItems({
    actions: [
      { id: "save", label: "Save", run: noop, primary: true },
      { id: "new", label: "New", run: noop, prominent: true },
      { id: "refresh", label: "Refresh", run: noop },
      { id: "export", label: "Export", run: noop, busy: true },
    ],
    ask: { prompts: ["What changed?"], askThis: noop, askPrompt: noop },
    commands: [{ id: "tracker.sync", label: "Sync", group: "Tracker", run: noop }],
    openWith: { options: [{ name: "a/text", title: "Text" }, { name: "a/rich", title: "Formatted" }], current: "a/rich", pick: noop },
    backlinks: 2,
    outbound: 0,
    pin: { disabledReason: null },
    openPanel: noop,
  });
  // The primary and prominent actions stay buttons in the bar, never in the menu.
  expect(ids(items)).toEqual([
    "action-refresh",
    "action-export",
    "—",
    "page-ask",
    "ref-commands-Tracker",
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
  expect(items.find((i) => i.id === "page-backlinks")!.label).toBe("Backlinks (2)…");
  expect(items.find((i) => i.id === "page-outbound")!.enabled).toBe(false);
  const openWith = items.find((i) => i.id === "page-open-with")!;
  expect(openWith.submenu!.map((i) => [i.label, i.checked])).toEqual([
    ["Text", false],
    ["Formatted", true],
  ]);
  const ask = items.find((i) => i.id === "page-ask")!;
  expect(ask.submenu!.map((i) => i.label)).toEqual(["Ask About This", "What changed?"]);
});

test("the items run what they stand for", () => {
  const ran: string[] = [];
  const items = pageMenuItems({
    ask: { prompts: ["Why?"], askThis: () => ran.push("ask"), askPrompt: (p) => ran.push(`prompt:${p}`) },
    commands: [{ id: "x.y", label: "Y", group: "X", run: () => ran.push("command") }],
    openWith: { options: [{ name: "a/b", title: "B" }, { name: "a/c", title: "C" }], current: "a/b", pick: (n) => ran.push(`pick:${n}`) },
    backlinks: 1,
    pin: {},
    openPanel: (p) => ran.push(`panel:${p}`),
  });
  const find = (id: string, list: MenuItem[] = items): MenuItem =>
    list.flatMap((i) => [i, ...(i.submenu ?? [])]).find((i) => i.id === id)!;
  for (const id of ["page-ask-this", "page-ask-prompt-0", "ref-command-x.y", "page-open-with-a/c", "page-backlinks", "page-pin"]) {
    void find(id).run?.();
  }
  expect(ran).toEqual(["ask", "prompt:Why?", "command", "pick:a/c", "panel:backlinks", "panel:pin"]);
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
