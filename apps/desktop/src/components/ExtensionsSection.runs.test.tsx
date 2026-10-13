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
    path: `oxplow/extensions/${name}`,
    errors: [],
    lenses: [],
    source: null,
    collectors: [],
    origin: "project",
    ui: { slots: [], commands: [], decorators: [] },
    enabled: true,
    advisories: [],
    measures: [],
    metrics: [],
    dimensions: [],
  }) as unknown as Extension;

let runs: unknown[][] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [ext("acme"), ext("beta"), ext("quiet")],
  listCollectors: async () => [],
  validateExtension: async () => ({ errors: [] }),
  // Bun's `mock.module` is process-wide: only the runs read is faked.
  querySql: (sql: string, ...rest: unknown[]) =>
    sql.includes("FROM v_extension_run")
      ? Promise.resolve({
          columns: ["extension", "contribution", "outcome", "error", "finished_at", "decisions"],
          rows: runs,
          truncated: false,
          reads: noReads,
          freshness: [],
        })
      : realQuerySql(sql, ...rest),
  subscribeOxplowEvents: () => () => {},
}));
const { ExtensionsSection } = await import("./ExtensionsSection.js");

afterAll(() => {
  mock.module("../api.js", () => real);
});
afterEach(() => {
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
