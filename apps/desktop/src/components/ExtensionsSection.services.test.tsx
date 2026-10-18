import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { Extension } from "../api.js";

// Settings → Extensions: each extension's row says what its scripts are given
// (the services), in the service titles, beside its other lines.

const realApi = await import("../api.js");
// Taken now: `mock.module` is process-wide, and this file puts it back after.
const real = { ...realApi };

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
    enabled: true, pages: [],
    advisories: [],
    measures: [],
    metrics: [],
    dimensions: [],
  }) as unknown as Extension;

mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [ext("acme"), ext("quiet")],
  listCollectors: async () => [],
  listServices: async () => [
    { scope: "tree.read", title: "Read the captured tree", summary: "", approval: false },
    { scope: "ai.compute", title: "Ask a model", summary: "", approval: true },
  ],
  validateExtension: async (name: string) => ({
    errors: [],
    services:
      name === "acme"
        ? [
            {
              script: "collector `triage`",
              entry: "triage.star",
              services: [
                { scope: "tree.read", declared: "implied", used: true, approval: false },
                { scope: "ai.compute", declared: "declared", used: true, approval: true },
              ],
            },
          ]
        : [],
  }),
  subscribeOxplowEvents: () => () => {},
}));
const { ExtensionsSection } = await import("./ExtensionsSection.js");

afterAll(() => {
  mock.module("../api.js", () => real);
});
afterEach(cleanup);

test("a row names the services its scripts are given, and a script-less extension says nothing", async () => {
  const view = render(<ExtensionsSection />);
  const line = await view.findByTestId("extension-services-acme");
  expect(line.textContent).toBe("Collector triage — Read the captured tree · Ask a model (you approve it)");
  expect(view.queryByTestId("extension-services-quiet")).toBeNull();
});
