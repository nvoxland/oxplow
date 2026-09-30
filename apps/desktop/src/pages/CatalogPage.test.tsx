import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { subscribeAgentInput } from "../agent-input-bus.js";

// The catalog page (P6.D2): what can be asked (by capability and
// extension), what data exists, and what can be configured.

const realApi = await import("../api.js");
const { MODELS_SQL } = await import("./exploreData.js");
// Bun's module mocks are process-wide: fake only this page's calls and
// hand anything else to the real functions, so later test files still work.
const realQuerySql = realApi.querySql;
const realRunCommand = realApi.runCommand;
mock.module("../api.js", () => ({
  ...realApi,
  promptCatalog: async () => [
    { prompt: "Which branches exist?", about: null, source: { kind: "capability", name: "vcs" } },
    { prompt: "Which PRs wait on me?", about: null, source: { kind: "extension", name: "gh" } },
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (sql !== MODELS_SQL) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    return {
    columns: ["view", "owner", "kind", "description"],
    rows: [["v_commit", "core", "sql", "Commits."], ["v_gh_pr", "gh", "entity", "Pull requests."]],
    truncated: false,
    reads: { models: ["v_model"], tables: [], measures: [] },
    freshness: {},
    };
  },
  runCommand: async (name: string, ...rest: unknown[]) => {
    if (name !== "config.list_keys") return (realRunCommand as (...a: unknown[]) => unknown)(name, ...rest);
    return { result: [{ key: "zones", doc: "Code areas.", human_only: false, schema: {}, set: true, value: [] }], audit_id: null, event_id: null, undo: null };
  },
}));
const { CatalogPage } = await import("./CatalogPage.js");

afterEach(cleanup);

test("the catalog lists prompts by source, data by owner and config keys", async () => {
  const inserted: string[] = [];
  const off = subscribeAgentInput((t) => inserted.push(t));
  const opened: string[] = [];
  const view = render(<CatalogPage streamId={null} onOpenPage={(r) => opened.push(r.id)} />);
  await waitFor(() => view.getByText("Which PRs wait on me?"));
  expect(view.getByTestId("catalog-prompts-capability-vcs").textContent).toContain("Which branches exist?");
  expect(view.getByTestId("catalog-prompts-extension-gh").textContent).toContain("Which PRs wait on me?");
  fireEvent.click(view.getByText("Which PRs wait on me?"));
  expect(inserted).toEqual(["Which PRs wait on me?"]);
  await waitFor(() => expect(view.getByTestId("catalog-data-gh").textContent).toContain("v_gh_pr"));
  const zones = await waitFor(() => view.getByTestId("catalog-config-zones"));
  expect(zones.textContent).toContain("Code areas.");
  fireEvent.click(view.getByTestId("catalog-open-settings"));
  expect(opened).toEqual(["page:settings"]);
  off();
});
