import { describe, expect, test } from "bun:test";
import type { CollectorListing, Extension } from "../tauri-bridge/generated/bindings.js";
import {
  collectorRan,
  collectorRowModel,
  extensionCredentials,
  secretCredentials,
  secretRowModel,
} from "./extensionRowModel.js";

const ext = (over: Partial<Extension> = {}): Extension => ({
  name: "review",
  description: "Review lenses",
  path: ".oxplow/runtime/extensions/review/0123456789abcdef",
  errors: [],
  lenses: [],
  collectors: [],
  origin: "project",
  deployment: { kind: "deployed", hash: "0123456789abcdef" },
  ui: { slots: [], commands: [], decorators: [] },
  enabled: true,
  ...over,
});

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

test("extensionCredentials lists each declared credential once per extension", () => {
  const l = (extension: string, creds: { name: string; set: boolean }[]) =>
    ({ owner: extension, credentials: creds }) as unknown as CollectorListing;
  expect(
    extensionCredentials(
      [
        l("gh", [{ name: "TOKEN", set: true }]),
        l("gh", [{ name: "TOKEN", set: true }, { name: "APP", set: false }]),
        l("other", [{ name: "X", set: false }]),
      ],
      "gh",
    ),
  ).toEqual([
    { name: "APP", set: false },
    { name: "TOKEN", set: true },
  ]);
});

test("a collector run is a v_collector_run change", () => {
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item", "v_collector_run"] })).toBe(true);
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item"] })).toBe(false);
  expect(collectorRan({ kind: "tasksChanged" })).toBe(false);
});

describe("secretCredentials", () => {
  const ref = { schemaVersion: 1, id: "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b" } as const;
  const provider = (credentials: unknown[]) =>
    ({ id: "remote", credentials }) as unknown as Extension["providers"][number];

  test("an extension's credentials with a secret reference are listed once per provider", () => {
    const list = secretCredentials(
      ext({
        providers: [
          provider([{ name: "OXPLOW_MCP_BEARER", oauth: null, secretRef: ref }, { name: "PLAIN", oauth: null, secretRef: null }]),
        ],
      }),
    );
    expect(list).toEqual([{ provider: "remote", name: "OXPLOW_MCP_BEARER", reference: ref }]);
  });

  test("an extension with no secret references lists none", () => {
    expect(secretCredentials(ext())).toEqual([]);
  });

  test("the row says whether a value is set, and offers Rotate only once it is", () => {
    expect(secretRowModel(null)).toEqual({ status: "Checking…", setLabel: "Set", canRevoke: false });
    expect(secretRowModel(false)).toEqual({ status: "Not set", setLabel: "Set", canRevoke: false });
    expect(secretRowModel(true)).toEqual({ status: "Set", setLabel: "Rotate", canRevoke: true });
  });
});
