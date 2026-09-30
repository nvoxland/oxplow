import { describe, expect, test } from "bun:test";
import { lensDefinitionChanged, readsChanged, unionReads, NO_READS } from "./lensRerun.js";

const task = { ...NO_READS, models: ["v_task", "v_thread"] };
const grid = { ...NO_READS, models: [], measures: ["oxplow.coverage"] };

describe("a lens re-runs on what it read", () => {
  test("a model it read changed; another didn't", () => {
    expect(readsChanged({ kind: "modelsChanged", models: ["v_task"] }, task)).toBe(true);
    expect(readsChanged({ kind: "modelsChanged", models: ["v_snapshot"] }, task)).toBe(false);
  });
  test("a metric grid re-runs on its own measures only", () => {
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, grid)).toBe(true);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.tokens"] }, grid)).toBe(false);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: [] }, grid)).toBe(true);
    expect(readsChanged({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, task)).toBe(false);
  });
  test("nothing else re-runs it but its own definition", () => {
    expect(readsChanged({ kind: "tasksChanged" }, task)).toBe(false);
    expect(lensDefinitionChanged({ kind: "workspaceChanged", path: "oxplow/extensions/x/lenses/a.yaml" })).toBe(true);
    expect(lensDefinitionChanged({ kind: "workspaceChanged", path: "src/a.ts" })).toBe(false);
  });
  test("several runs read the union", () => {
    const u = unionReads([task, grid, null]);
    expect(u.models.sort()).toEqual(["v_task", "v_thread"]);
    expect(u.measures).toEqual(["oxplow.coverage"]);
  });
});
