import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// The fallback renderer: what a URI no page renders opens on. A URI of an
// undeclared resource, and one whose `resolve` dataset has no row, both show as
// a dead link (the raw URI, muted); a resolvable one shows its params.

const realApi = await import("../api.js");
const realQuerySql = realApi.querySql;
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_acme_prs")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    const found = (rest[0] as string[])[0] === "oxplow:acme/pr?id=12";
    return {
      columns: ["title"],
      rows: found ? [["Fix the thing"]] : [],
      truncated: false,
      reads: { datasets: ["v_acme_prs"], tables: [], measures: [] },
      freshness: {},
    };
  },
}));
const { ResourcePage } = await import("./ResourcePage.js");
const { setResources } = await import("../resources.js");

afterEach(() => {
  cleanup();
  setResources([]);
});

const PR = {
  resource: "acme/pr",
  extension: "acme",
  label: "Pull request",
  params: { id: { required: true, pattern: null } },
  additional: false,
  resolve: "v_acme_prs",
  title: null,
  searchable: null,
  icon: null,
  wikilink: null,
};

test("a URI of a resource nothing declares is a dead link, never an error", () => {
  const view = render(<ResourcePage uri="oxplow:gone/kind?id=1" />);
  const dead = view.getByTestId("resource-dead-link");
  expect(dead.textContent).toContain("oxplow:gone/kind?id=1");
  expect(view.queryByTestId("resource-params")).toBeNull();
});

test("a URI whose dataset has no row is a dead link", async () => {
  setResources([PR]);
  const view = render(<ResourcePage uri="oxplow:acme/pr?id=99" />);
  await waitFor(() => expect(view.getByTestId("resource-dead-link")).toBeTruthy());
  expect(view.queryByTestId("resource-params")).toBeNull();
});

test("a URI its dataset resolves shows its title and params", async () => {
  setResources([PR]);
  const view = render(<ResourcePage uri="oxplow:acme/pr?id=12" />);
  await waitFor(() => expect(view.getByTestId("resource-params").textContent).toContain("12"));
  expect(view.queryByTestId("resource-dead-link")).toBeNull();
  expect(view.container.textContent).toContain("Fix the thing");
});

test("a URI with a view (#part=output) resolves as the resource it names", async () => {
  setResources([PR]);
  const view = render(<ResourcePage uri="oxplow:acme/pr?id=12#part=output" />);
  await waitFor(() => expect(view.getByTestId("resource-params").textContent).toContain("12"));
  expect(view.queryByTestId("resource-dead-link")).toBeNull();
});
