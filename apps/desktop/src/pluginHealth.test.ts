import { expect, test } from "bun:test";

import type { PersonCommands } from "./personCommands.js";
import {
  enableAgain,
  healthLine,
  healthOf,
  pluginHealthFromResult,
  repairMention,
  repairWithAgent,
  type PluginHealth,
} from "./pluginHealth.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

const COLUMNS = [
  "plugin",
  "contribution",
  "kind",
  "state",
  "reason",
  "consecutive_failures",
  "last_error",
  "mean_ms",
  "dead_letters",
  "fresh",
  "repair_item",
];

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: COLUMNS,
    rows,
    truncated: false,
    reads: { models: ["v_plugin_health"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

const health = (over: Partial<PluginHealth>): PluginHealth => ({
  plugin: "tracker",
  contribution: "issues",
  kind: "collector",
  state: "ok",
  reason: null,
  failures: 0,
  lastError: null,
  meanMs: null,
  deadLetters: 0,
  fresh: true,
  repairItem: null,
  ...over,
});

test("rows read as health", () => {
  const [h] = pluginHealthFromResult(
    result([["tracker", "issues", "collector", "disabled", "3 failures", 3, "boom", 12.5, 2, 0, "work_item:oxplow:tsk9"]]),
  );
  expect(h).toEqual(
    health({
      state: "disabled",
      reason: "3 failures",
      failures: 3,
      lastError: "boom",
      meanMs: 12.5,
      deadLetters: 2,
      fresh: false,
      repairItem: "work_item:oxplow:tsk9",
    }),
  );
});

test("an extension's health is its contributions'", () => {
  const rows = [health({}), health({ plugin: "other" }), health({ contribution: "prs" })];
  expect(healthOf(rows, "tracker").map((h) => h.contribution)).toEqual(["issues", "prs"]);
});

test("a disabled contribution shows its reason and can be enabled and repaired", () => {
  const line = healthLine(health({ state: "disabled", reason: "3 failures in a row; the last: boom", repairItem: "work_item:oxplow:tsk9" }));
  expect(line.text).toBe("Disabled: 3 failures in a row; the last: boom");
  expect(line.tone).toBe("error");
  expect(line.canEnable).toBe(true);
  expect(line.repairItem).toBe("work_item:oxplow:tsk9");
});

test("a disabled contribution without an open repair item has nothing to repair from", () => {
  const line = healthLine(health({ state: "disabled", reason: "gone" }));
  expect(line.canEnable).toBe(true);
  expect(line.repairItem).toBeNull();
});

test("a failing contribution says how often and why, and isn't enabled again", () => {
  const line = healthLine(health({ state: "failing", failures: 2, lastError: "timeout" }));
  expect(line.text).toBe("Failing (2 in a row): timeout");
  expect(line.tone).toBe("warn");
  expect(line.canEnable).toBe(false);
});

test("a healthy contribution says so, with its average, missed schedule and undelivered events", () => {
  expect(healthLine(health({ meanMs: 41.6 })).text).toBe("OK · 42 ms on average");
  expect(healthLine(health({})).tone).toBe("ok");
  const late = healthLine(health({ fresh: false, deadLetters: 1 }));
  expect(late.text).toBe("OK · missed its schedule · 1 event undelivered");
  expect(late.tone).toBe("warn");
  expect(healthLine(health({ deadLetters: 3 })).text).toBe("OK · 3 events undelivered");
});

test("the repair mention names the repair item as a ref, on one line", () => {
  expect(repairMention("work_item:oxplow:tsk9")).toBe(
    "Repair the extension described in [oxplow ref work_item:oxplow:tsk9] — read it first.",
  );
});

test("Repair with the Agent fills the agent input with the mention, and only that", () => {
  const inserted: string[] = [];
  repairWithAgent("work_item:linear:ENG-12", (t) => inserted.push(t));
  expect(inserted).toEqual(["Repair the extension described in [oxplow ref work_item:linear:ENG-12] — read it first."]);
});

test("Enable Again runs plugin.enable as the person", async () => {
  const runs: unknown[][] = [];
  const commands = {
    run: async (label: string, command: string, input: unknown) => {
      runs.push([label, command, input]);
      return true;
    },
  } as unknown as PersonCommands;
  await enableAgain(health({ state: "disabled" }), commands);
  expect(runs).toEqual([["Enable tracker/issues", "plugin.enable", { plugin: "tracker", contribution: "issues" }]]);
});
