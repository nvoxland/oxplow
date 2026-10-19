import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// The section keeps only what the Extensions lens lacks: health rows and
// credentials. Nothing of either shows one muted line.

const realApi = await import("../api.js");
const realHealth = await import("../contributionHealth.js");
let extensions: unknown[] = [];
let collectors: unknown[] = [];
let healthRows: unknown[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  listCollectors: async () => collectors,
  subscribeOxplowEvents: () => () => {},
}));
mock.module("../contributionHealth.js", () => ({
  ...realHealth,
  useContributionHealth: () => healthRows,
}));
const { ExtensionsSection } = await import("./ExtensionsSection.js");

afterEach(() => {
  cleanup();
  extensions = [];
  collectors = [];
  healthRows = [];
});

const ext = { name: "tracker", providers: [] };

test("shows a disabled contribution's health row and an unset credential", async () => {
  extensions = [ext];
  healthRows = [
    {
      extension: "tracker",
      contribution: "issues",
      kind: "collector",
      state: "disabled",
      reason: "3 failures in a row",
      failures: 3,
      lastError: "boom",
      meanMs: null,
      deadLetters: 0,
      fresh: true,
      repairItem: null,
    },
  ];
  collectors = [{ owner: "tracker", credentials: [{ name: "TOKEN", set: false }] }];
  const view = render(<ExtensionsSection />);
  await waitFor(() => expect(view.getByTestId("extension-health-tracker-issues")).toBeTruthy());
  expect(view.getByTestId("credential-tracker-TOKEN")).toBeTruthy();
  expect(view.queryByTestId("extensions-health-empty")).toBeNull();
});

test("shows the empty line when nothing needs attention", async () => {
  extensions = [ext];
  const view = render(<ExtensionsSection />);
  await waitFor(() => expect(view.getByTestId("extensions-health-empty")).toBeTruthy());
});
