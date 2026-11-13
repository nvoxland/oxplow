import { describe, expect, test } from "bun:test";
import {
  extensionsChanged,
  MissedChanges,
  pageDefinitionChanged,
  readsChanged,
  unionReads,
  NO_READS,
} from "./rerun.js";

const task = { ...NO_READS, datasets: ["v_oxplow_task", "v_thread"] };
const grid = { ...NO_READS, datasets: [], measures: ["oxplow.coverage"] };

describe("a viz re-runs on what it read", () => {
  test("a dataset it read changed; another didn't", () => {
    expect(readsChanged({ kind: "datasetsChanged", datasets: ["v_oxplow_task"] }, task)).toBe(true);
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
    // the daemon says when the extension catalog changed; a file
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
    expect(extensionsChanged({ kind: "datasetsChanged", datasets: ["v_oxplow_task"] })).toBe(false);
  });
  test("several runs read the union", () => {
    const u = unionReads([task, grid, null]);
    expect(u.datasets.sort()).toEqual(["v_oxplow_task", "v_thread"]);
    expect(u.measures).toEqual(["oxplow.coverage"]);
  });
});

// A host remembers the changes that matched nothing it read yet. It once kept
// every event in a list copied on each one: a boot's burst of events, heard
// by every host, spun the window's thread for most of an hour.
describe("the changes a host missed", () => {
  test("are kept by what changed, never by how many events said so", () => {
    const missed = new MissedChanges();
    for (let i = 0; i < 10_000; i++) {
      missed.note({ kind: "datasetsChanged", datasets: [`v_${i % 3}`] }, i);
      missed.note({ kind: "pageRunChanged", runId: `r${i}` }, i);
      missed.note({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, i);
    }
    expect(missed.size).toBe(4);
  });
  test("match a run's reads as readsChanged does, within the window", () => {
    const missed = new MissedChanges();
    missed.note({ kind: "datasetsChanged", datasets: ["v_oxplow_task"] }, 1_000);
    expect(missed.matches(task, 2_000)).toBe(true);
    expect(missed.matches({ ...NO_READS, datasets: ["v_snapshot"] }, 2_000)).toBe(false);
    expect(missed.matches(task, 1_000 + 30_000)).toBe(false);
    missed.note({ kind: "metricSamplesChanged", measures: [] }, 1_000);
    expect(missed.matches(grid, 2_000)).toBe(true);
    expect(missed.matches(NO_READS, 2_000)).toBe(false);
  });
  test("a re-run forgets what changed before it started", () => {
    const missed = new MissedChanges();
    missed.note({ kind: "datasetsChanged", datasets: ["v_oxplow_task"] }, 1_000);
    missed.note({ kind: "metricSamplesChanged", measures: ["oxplow.coverage"] }, 3_000);
    missed.forgetBefore(2_000);
    expect(missed.matches(task, 3_000)).toBe(false);
    expect(missed.matches(grid, 3_000)).toBe(true);
  });
});
