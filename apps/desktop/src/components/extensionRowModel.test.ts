import { describe, expect, test } from "bun:test";
import type { CollectorListing } from "../tauri-bridge/generated/bindings.js";
import {
  collectorRan,
  collectorRowModel,
} from "./extensionRowModel.js";

describe("collectorRowModel", () => {
  const listing = (over: Partial<CollectorListing> = {}): CollectorListing => ({
    owner: "my-gh",
    spec: {
      id: "gh",
      doc: "",
      runtime: "exec",
      entry: "bin/sync.sh",
      provider: null,
      trigger: { kind: "every", minutes: 10 },
      after: [],
      input: null,
      report: null,
      sync: "replace",
      env: ["GITHUB_TOKEN"],
      network: [],
      credentials: [],
      entities: [],
      facts: [],
    },
    run: null,
    networkEnforced: true,
    credentials: [],
    ...over,
  });

  test("a collector shows its status and trigger, and syncs by hand", () => {
    const m = collectorRowModel(listing());
    expect(m.actionLabel).toBe("Sync Now");
    expect(m.actionTitle).toBe("Run bin/sync.sh now");
    expect(m.status).toBe("Never run");
    expect(m.trigger).toBe("every 10m");
  });

  // The project's own collectors (its report collectors among them), listed
  // beside the extensions': owner `project`.
  test("a report collector says when it runs, and syncs by hand", () => {
    const m = collectorRowModel(
      listing({
        owner: "project",
        spec: { ...listing().spec, id: "tests.cov", runtime: "jaq", entry: "oxplow:lcov", trigger: { kind: "onRun", run: "test" }, env: [] },
      }),
    );
    expect(m.trigger).toBe("after a test run");
    expect(m.actionLabel).toBe("Sync Now");
    const analysis = collectorRowModel(
      listing({ owner: "project", spec: { ...listing().spec, trigger: { kind: "onRun", run: "analysis" } } }),
    );
    expect(analysis.trigger).toBe("after an analysis run");
  });

  test("a collector failures disabled says why and offers Enable Again", () => {
    const m = collectorRowModel(listing({ disabled: "failed 3 times in a row: report not found" }));
    expect(m.disabled).toBe("Off on this machine: failed 3 times in a row: report not found");
    expect(m.canEnableAgain).toBe(true);
    expect(collectorRowModel(listing()).disabled).toBeNull();
    expect(collectorRowModel(listing()).canEnableAgain).toBe(false);
  });

  test("a collector summarizes its last run", () => {
    const m = collectorRowModel(
      listing({
        run: { owner: "my-gh", id: "gh", status: "ok", lastRunAt: "2026-09-27T01:00:00Z", error: null, rowCounts: { pr: 12, review: 3 }, cursor: null, lastEventId: null },
      }),
    );
    expect(m.actionLabel).toBe("Sync Now");
    expect(m.status).toBe("12 pr · 3 review");
    expect(m.error).toBeNull();
  });

  test("a failed run surfaces the error", () => {
    const m = collectorRowModel(
      listing({
        run: { owner: "my-gh", id: "gh", status: "error", lastRunAt: "2026-09-27T01:00:00Z", error: "boom", rowCounts: {}, cursor: null, lastEventId: null },
      }),
    );
    expect(m.status).toBe("Failed");
    expect(m.error).toBe("boom");
    expect(collectorRowModel(listing({ spec: { ...listing().spec, trigger: { kind: "manual" } } })).trigger).toBe("manual");
    expect(
      collectorRowModel(
        listing({ spec: { ...listing().spec, trigger: { kind: "on", events: ["snapshot.taken", "vcs.head.moved"], filter: {} } } }),
      ).trigger,
    ).toBe("on snapshot.taken, vcs.head.moved");
  });
  test("unset credentials are listed and flagged", () => {
    const m = collectorRowModel(
      listing({
        spec: { ...listing().spec, trigger: { kind: "manual" }, env: [], credentials: ["GH_PAT", "OTHER"] },
        credentials: [
          { name: "GH_PAT", set: true },
          { name: "OTHER", set: false },
        ],
      }),
    );
    expect(m.credentials).toEqual([
      { name: "GH_PAT", set: true },
      { name: "OTHER", set: false },
    ]);
    expect(m.missingCredentials).toBe("Needs OTHER (set it under Extensions).");
  });
});

test("a collector run is a v_collector_run change", () => {
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item", "v_collector_run"] })).toBe(true);
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item"] })).toBe(false);
  expect(collectorRan({ kind: "tasksChanged" })).toBe(false);
});
