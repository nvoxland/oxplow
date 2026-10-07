import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// Settings → Pieces lists each choosable capability's implementations, says
// which is active and why, and writes a choice to the project or, "just for
// me", to the person's own layer.

const COLUMNS = [
  "capability", "provider", "extension", "features", "active", "title", "source",
  "available", "chosen_by", "capability_title", "choosable", "optional",
];
const ran: [string, unknown][] = [];
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: () => () => {},
  listExtensions: async () => [
    { enabled: true, lenses: [{ title: "Ready Tasks", needs: ["work_items"] }], advisories: [] },
  ],
  effectiveConfig: async () => [
    { key: "activeProviders", origin: "default", value: {} },
    { key: "personal.activeProviders", origin: "personal", value: { work_items: "none" } },
  ],
  querySql: async () => ({
    columns: COLUMNS,
    rows: [
      ["work_items", "none", null, "{}", 1, "None", "none", 1, "personal", "Work list", 1, 1],
      ["work_items", "oxplow", "oxplow-bundled", '{"hierarchy":true}', 0, "oxplow's tasks", "builtin", 1, null, "Work list", 1, 1],
      ["vcs", "git", null, "{}", 1, "git", "core", 1, "default", "Version control", 0, 0],
    ],
    truncated: false,
    reads: { models: ["v_capability_provider"], tables: [], measures: [] },
    freshness: [],
  }),
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: "e", inverse: null };
  },
}));
const { PiecesSection } = await import("./PiecesSection.js");

afterEach(() => {
  cleanup();
  ran.length = 0;
});

test("a capability shows its choices, what's active and why, and what none turns off", async () => {
  const view = render(<PiecesSection />);
  await waitFor(() => expect(view.getByTestId("pieces-work_items")).toBeTruthy());
  expect(view.queryByTestId("pieces-vcs")).toBeNull();
  expect(view.getByTestId("pieces-work_items-status").textContent).toContain("None — Your own choice.");
  expect(view.getByTestId("pieces-work_items-off").textContent).toContain("Ready Tasks");
  expect((view.getByTestId("pieces-work_items-project-default") as HTMLInputElement).checked).toBe(true);
  expect((view.getByTestId("pieces-work_items-personal") as HTMLSelectElement).value).toBe("none");
});

test("a project choice writes activeProviders; just-for-me writes the personal layer", async () => {
  const view = render(<PiecesSection />);
  await waitFor(() => expect(view.getByTestId("pieces-work_items")).toBeTruthy());
  fireEvent.click(view.getByTestId("pieces-work_items-project-oxplow"));
  await waitFor(() =>
    expect(ran).toEqual([["config.set", { key: "activeProviders", value: { work_items: "oxplow" } }]]),
  );
  fireEvent.change(view.getByTestId("pieces-work_items-personal"), { target: { value: "" } });
  await waitFor(() => expect(ran[1]).toEqual(["config.unset", { key: "activeProviders", layer: "personal" }]));
});
