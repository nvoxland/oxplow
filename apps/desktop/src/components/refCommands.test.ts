import { expect, test } from "bun:test";

import { groupOffers, refCommandMenuItems } from "./refCommands.js";
import type { CommandEntry } from "./quickOpenResults.js";

const entry = (id: string, group: string, enabled = true): CommandEntry => ({
  id,
  group,
  label: id,
  searchKey: id,
  enabled,
  run: () => {},
});

test("a ref's commands group by their ui.group into a row menu's submenus", () => {
  const entries = [entry("a.x.one", "Review"), entry("b.y.two", "Tracker", false), entry("a.x.three", "Review")];
  expect(groupOffers(entries).map((g) => [g.group, g.entries.map((e) => e.id)])).toEqual([
    ["Review", ["a.x.one", "a.x.three"]],
    ["Tracker", ["b.y.two"]],
  ]);
  const items = refCommandMenuItems(entries);
  expect(items[0].separator).toBe(true);
  expect(items.slice(1).map((i) => [i.label, i.submenu?.map((s) => [s.id, s.enabled])])).toEqual([
    ["Review", [["ref-command-a.x.one", true], ["ref-command-a.x.three", true]]],
    ["Tracker", [["ref-command-b.y.two", false]]],
  ]);
  expect(refCommandMenuItems([])).toEqual([]);
});
