import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Extension } from "../api.js";

// Settings → Extensions: each extension's row says how its last run ended and
// when (`v_extension_run`, latest per contribution); failed, denied and stale
// in the warning color, and a denied run says which decision refused it.

const realApi = await import("../api.js");
// Taken now: `mock.module` is process-wide, and this file puts it back after.
const real = { ...realApi };
const realQuerySql = realApi.querySql as (...a: unknown[]) => Promise<unknown>;
const noReads = { models: [], tables: [], measures: [] };

const ext = (name: string): Extension =>
  ({
    name,
    description: "",
    path: `.oxplow/extensions/${name}`,
    errors: [],
    lenses: [],
    deployment: { kind: "deployed", hash: "0123456789abcdef" },
    collectors: [],
    origin: "project",
    ui: { slots: [], commands: [], decorators: [] },
    enabled: true, pages: [],
    advisories: [],
    measures: [],
    metrics: [],
    dimensions: [],
  }) as unknown as Extension;

let runs: unknown[][] = [];
/** The event listeners the section registered, and how often it read each part. */
const listeners = new Set<(event: Record<string, unknown>) => void>();
const emit = (event: Record<string, unknown>) => listeners.forEach((l) => l(event));
const calls = { extensions: 0, collectors: 0, checked: 0, runs: 0 };
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => (calls.extensions++, [ext("acme"), ext("beta"), ext("quiet")]),
  listCollectors: async () => (calls.collectors++, []),
  validateExtension: async () => (calls.checked++, { errors: [] }),
  // Bun's `mock.module` is process-wide: only the runs read is faked.
  querySql: (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_extension_run")) return realQuerySql(sql, ...rest);
    calls.runs++;
    return Promise.resolve({
      columns: ["extension", "contribution", "outcome", "error", "finished_at", "decisions"],
      rows: runs,
      truncated: false,
      reads: { models: ["v_extension_run"], tables: [], measures: [] },
      freshness: [],
    });
  },
  subscribeOxplowEvents: (handler: (event: Record<string, unknown>) => void) => {
    listeners.add(handler);
    return () => listeners.delete(handler);
  },
}));
const { ExtensionsSection } = await import("./ExtensionsSection.js");

afterAll(() => {
  mock.module("../api.js", () => real);
});
afterEach(() => {
  listeners.clear();
  Object.assign(calls, { extensions: 0, collectors: 0, checked: 0, runs: 0 });
  runs = [];
  cleanup();
});

test("each extension's row shows its last run, and a denied one says why", async () => {
  runs = [
    ["acme", "collector:acme/pull", "failed", "rate limited", "2026-10-10T09:00:00Z", null],
    ["acme", "effect:acme/note", "ok", null, "2026-10-10T08:00:00Z", null],
    [
      "beta",
      "collector:beta/sync",
      "denied",
      "denied",
      "2026-10-10T07:00:00Z",
      JSON.stringify([{ need: "host", name: "evil.example", decision: "denied", reason: "it isn't in the grant" }]),
    ],
  ];
  const view = render(<ExtensionsSection />);
  const acme = await view.findByTestId("extension-last-run-acme");
  expect(acme.textContent).toContain("Last run: failed · collector pull");
  expect(acme.textContent).toContain("rate limited");
  expect(acme.getAttribute("data-warn")).toBe("true");
  const beta = await view.findByTestId("extension-last-run-beta");
  expect(beta.textContent).toContain("host evil.example: it isn't in the grant");
  expect(beta.getAttribute("data-warn")).toBe("true");
  // An extension that hasn't run says nothing of it.
  expect(view.queryByTestId("extension-last-run-quiet")).toBeNull();
});

test("a healthy last run is quiet, not a warning", async () => {
  runs = [["acme", "effect:acme/note", "ok", null, "2026-10-10T08:00:00Z", null]];
  const view = render(<ExtensionsSection />);
  const acme = await waitFor(() => view.getByTestId("extension-last-run-acme"));
  expect(acme.getAttribute("data-warn")).toBe("false");
  expect(acme.textContent).toContain("Last run: ok");
});

// tsk1403: each part re-reads on the change that touches it. A collector run
// (a steady stream of them) re-lists the collectors; it neither lists the
// extensions again nor re-checks each one.
const quiet = () => new Promise((r) => setTimeout(r, 250));

test("a collector run re-lists the collectors and checks no extension", async () => {
  const view = render(<ExtensionsSection />);
  await view.findByTestId("extension-last-run-acme").catch(() => null);
  await waitFor(() => expect(calls.checked).toBe(3));
  const before = { ...calls };
  emit({ kind: "modelsChanged", models: ["v_collector_run"] });
  await waitFor(() => expect(calls.collectors).toBe(before.collectors + 1));
  await quiet();
  expect(calls.extensions).toBe(before.extensions);
  expect(calls.checked).toBe(before.checked);
  expect(calls.runs).toBe(before.runs);
});

test("an extension run re-reads the last runs alone", async () => {
  render(<ExtensionsSection />);
  await waitFor(() => expect(calls.runs).toBe(1));
  const before = { ...calls };
  emit({ kind: "modelsChanged", models: ["v_extension_run"] });
  await waitFor(() => expect(calls.runs).toBe(before.runs + 1));
  await quiet();
  expect(calls.extensions).toBe(before.extensions);
  expect(calls.collectors).toBe(before.collectors);
  expect(calls.checked).toBe(before.checked);
});

test("the config changing lists and checks the extensions again", async () => {
  render(<ExtensionsSection />);
  await waitFor(() => expect(calls.checked).toBe(3));
  emit({ kind: "configChanged" });
  await waitFor(() => expect(calls.checked).toBe(6));
  expect(calls.extensions).toBe(2);
});
