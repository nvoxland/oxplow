import { describe, expect, test } from "bun:test";
import { extensionsChanged, pageDefinitionChanged, readsChanged, unionReads, NO_READS } from "./rerun.js";

const task = { ...NO_READS, datasets: ["v_work_item", "v_thread"] };
const grid = { ...NO_READS, datasets: [], measures: ["oxplow.coverage"] };

describe("a viz re-runs on what it read", () => {
  test("a dataset it read changed; another didn't", () => {
    expect(readsChanged({ kind: "datasetsChanged", datasets: ["v_work_item"] }, task)).toBe(true);
    expect(readsChanged({ kind: "datasetsChanged", datasets: ["v_snapshot"] }, task)).toBe(false);
  });
  test("a metric grid re-runs on its own measures only", () => {
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, grid)).toBe(true);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.tokens"] }, grid)).toBe(false);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: [] }, grid)).toBe(true);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, task)).toBe(false);
  });
  test("nothing else re-runs it but its own definition", () => {
    expect(readsChanged({ kind: "tasksChanged" }, task)).toBe(false);
    // tsk1030: the daemon says when the extension catalog changed; a file
    // path is not the signal.
    expect(pageDefinitionChanged({ kind: "extensionsChanged" })).toBe(true);
    expect(pageDefinitionChanged({ kind: "workspaceChanged", path: ".oxplow/extensions/x/pages/a.yaml" })).toBe(false);
    expect(pageDefinitionChanged({ kind: "configChanged" })).toBe(false);
  });
  // What an extension contributes (its panels, pages, viz nodes) changes when
  // the catalog changes — and when the person enables or disables one,
  // which is a config change.
  test("the set of enabled extensions' contributions changes with the catalog or the config", () => {
    expect(extensionsChanged({ kind: "extensionsChanged" })).toBe(true);
    expect(extensionsChanged({ kind: "configChanged" })).toBe(true);
    expect(extensionsChanged({ kind: "workspaceChanged", path: "src/a.ts" })).toBe(false);
    expect(extensionsChanged({ kind: "datasetsChanged", datasets: ["v_work_item"] })).toBe(false);
  });
  test("several runs read the union", () => {
    const u = unionReads([task, grid, null]);
    expect(u.datasets.sort()).toEqual(["v_thread", "v_work_item"]);
    expect(u.measures).toEqual(["oxplow.coverage"]);
  });
});
