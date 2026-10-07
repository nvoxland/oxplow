import { expect, test } from "bun:test";

import type { EffectiveSetting } from "../tauri-bridge/generated/bindings.js";
import { askToChange, groupSettings, matchesSearch } from "./settingsModel.js";

const row = (key: string, over: Partial<EffectiveSetting> = {}): EffectiveSetting => ({
  key,
  doc: `${key} doc`,
  value: null,
  origin: "default",
  extension: null,
  humanOnly: false,
  schema: null,
  ...over,
});

test("settings group by what they're about, in a fixed order", () => {
  const groups = groupSettings([
    row("snapshotRetentionDays"),
    row("ai.roles.main", { humanOnly: true }),
    row("agents", { humanOnly: true }),
    row("metrics.gh.prs"),
    row("zones"),
  ]);
  expect(groups.map((g) => [g.title, g.settings.map((s) => s.key)])).toEqual([
    ["Project", ["zones"]],
    ["Agents", ["agents"]],
    ["AI", ["ai.roles.main"]],
    ["Snapshots", ["snapshotRetentionDays"]],
    ["Metrics & Data", ["metrics.gh.prs"]],
  ]);
});

test("search matches the key, its doc or its value", () => {
  expect(matchesSearch(row("zones", { doc: "Code areas" }), "code")).toBe(true);
  expect(matchesSearch(row("snapshotRetentionDays", { value: 14 }), "14")).toBe(true);
  expect(matchesSearch(row("zones"), "nope")).toBe(false);
  // The whole value, not its compact display: a term past the 80th
  // character still finds the row.
  const zones = [{ name: "core", paths: ["crates/oxplow-app/**", "crates/oxplow-db/**"] }, { name: "ui", paths: ["apps/desktop/src/components/**"] }];
  expect(matchesSearch(row("zones", { value: zones }), "components")).toBe(true);
});

test("Ask the Agent to Change This names the key, its doc and its value", () => {
  const text = askToChange(row("snapshotRetentionDays", { doc: "Days to keep snapshots.", value: 7, origin: "default" }));
  expect(text).toContain("`snapshotRetentionDays`");
  expect(text).toContain("Days to keep snapshots.");
  expect(text).toContain("7 (the default)");
  expect(text).toContain("oxplow.config.set");
});
