import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";

import type { ReactElement, ReactNode } from "react";
import type { VizConfig, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { Section } from "../kit/Section.js";

// `viz: sections`: a subsection per row — a heading (a link when the row
// names a page), the row's facts, its markdown body — each a real section
// of the page, with the viz's row actions in its right-click menu.

const realApi = await import("../api.js");
const actionRuns: { action: string; row: unknown }[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  refCommands: async () => [],
  listPersonCommands: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

const press: VizActionRunner = async (_id, action, _params, row) => {
  actionRuns.push({ action, row });
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

afterEach(cleanup);

const base = {
  id: "x/pages#all",
  extension: "x",
  slug: "all",
  title: "All",
  viz: "sections",
  params: [],
  columns: [{ key: "kind", label: "Kind", link: null, unit: null, icon: null, tone: null }],
  empty: null,
  sections: { title: "name", id: "id", link: "uri", body: "note" },
  group: null,
  emphasis: null,
  depth: null,
  fills: false,
  actions: [],
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
} as unknown as VizConfig;

const columns = ["id", "name", "uri", "kind", "note", "broken"];
const rows: SqlCell[][] = [
  ["a", "Alpha", "oxplow:oxplow/page?page=alpha", "ships with oxplow", "Reads **bold**.", 0],
  ["b", "Beta", null, "yours", null, 1],
];
const runOf = (over: Partial<VizConfig> = {}, data: SqlCell[][] = rows): VizRun =>
  ({ config: { ...base, ...over }, params: {}, result: { columns, rows: data, truncated: false } }) as unknown as VizRun;

test("one subsection per row, in row order, each a section whose heading is a link when the row names a page", () => {
  const view = render(<VizView run={runOf()} onOpenPage={() => {}} />);
  const sections = view.container.querySelectorAll("section.ox-section");
  expect(sections.length).toBe(2);
  const titles = Array.from(view.container.querySelectorAll(".ox-section-title")).map((h) => h.textContent);
  expect(titles).toEqual(["Alpha", "Beta"]);
  // Alpha's heading links; Beta has no link column value: plain text.
  expect(view.getByTestId("viz-section-a").querySelector(".ox-section-title a, .ox-section-title button")?.textContent).toBe("Alpha");
  expect(view.getByTestId("viz-section-b").querySelector(".ox-section-title a, .ox-section-title button")).toBeNull();
});

test("a subsection draws the displayed columns as facts, never the heading, link, id or body columns, and its body as markdown", () => {
  const view = render(<VizView run={runOf()} onOpenPage={() => {}} />);
  const facts = view.getByTestId("viz-section-facts-a");
  expect(facts.textContent).toBe("Kindships with oxplow");
  expect(view.getByTestId("viz-section-body-a").querySelector("strong")?.textContent).toBe("bold");
  // No body, no body element.
  expect(view.queryByTestId("viz-section-body-b")).toBeNull();
  expect(view.getByTestId("viz-section-b").textContent).not.toContain("Alpha");
});

test("with no columns declared the facts are every column but the role ones", () => {
  const view = render(<VizView run={runOf({ columns: [] })} onOpenPage={() => {}} />);
  const labels = Array.from(view.getByTestId("viz-section-facts-a").querySelectorAll("dt")).map((d) => d.textContent);
  expect(labels).toEqual(["Kind", "Broken"]);
});

test("a subsection's id is stable per row: <node>/<id column>, whatever the order", () => {
  const first = render(<VizView run={runOf()} onOpenPage={() => {}} />);
  expect(Array.from(first.container.querySelectorAll("[data-node]")).map((n) => n.getAttribute("data-node"))).toEqual(["all/a", "all/b"]);
  cleanup();
  const reversed = render(<VizView run={runOf({}, [rows[1]!, rows[0]!])} onOpenPage={() => {}} />);
  expect(Array.from(reversed.container.querySelectorAll("[data-node]")).map((n) => n.getAttribute("data-node"))).toEqual(["all/b", "all/a"]);
  expect(reversed.container.querySelector('[data-node="all/a"] .ox-section-title')?.textContent).toBe("Alpha");
});

test("with no id column the title keys the row; a repeated title is told apart", () => {
  const view = render(
    <VizView
      run={
        {
          config: { ...base, sections: { title: "name", id: null, link: null, body: null }, columns: [] },
          params: {},
          result: { columns: ["name"], rows: [["Same"], ["Same"]], truncated: false },
        } as unknown as VizRun
      }
      onOpenPage={() => {}}
    />,
  );
  expect(Array.from(view.container.querySelectorAll("[data-node]")).map((n) => n.getAttribute("data-node"))).toEqual(["all/Same", "all/Same-2"]);
});

test("a subsection's heading is one level below the section the viz is in", () => {
  const nested = render(
    <Section title="Extensions">
      <VizView run={runOf()} onOpenPage={() => {}} />
    </Section>,
  );
  expect(nested.container.querySelector("h2")?.textContent).toBe("Extensions");
  expect(Array.from(nested.container.querySelectorAll("h3")).map((h) => h.textContent)).toEqual(["Alpha", "Beta"]);
  cleanup();
  // Not inside a section: the top of the page's outline.
  const bare = render(<VizView run={runOf()} onOpenPage={() => {}} />);
  expect(bare.container.querySelectorAll("h2").length).toBe(2);
});

test("a subsection with nothing but its heading is still drawn", () => {
  const view = render(
    <VizView
      run={
        {
          config: { ...base, sections: { title: "name", id: null, link: null, body: null }, columns: [] },
          params: {},
          result: { columns: ["name"], rows: [["Only a name"]], truncated: false },
        } as unknown as VizRun
      }
      onOpenPage={() => {}}
    />,
  );
  expect(view.getByTestId("viz-section-Only a name").getAttribute("data-keep")).toBe("true");
});

test("a row action is in the subsection's right-click menu, offered by `when`, and runs on its row", async () => {
  actionRuns.length = 0;
  const view = render(
    <VizView
      run={runOf({ actions: [{ id: "fix", label: "Repair", command: "x.fix", input: {}, row: true, when: "broken" }] as unknown as VizConfig["actions"] })}
      onOpenPage={() => {}}
    />,
  );
  // Alpha's `broken` is 0: not offered. Beta's is 1.
  fireEvent.contextMenu(view.getByTestId("viz-section-a"));
  expect(view.queryByTestId("menu-item-viz-action-fix")).toBeNull();
  fireEvent.keyDown(document.body, { key: "Escape" });
  fireEvent.contextMenu(view.getByTestId("viz-section-b"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-viz-action-fix")));
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["fix"]));
  expect((actionRuns[0]!.row as Record<string, unknown>).id).toBe("b");
});

test("following a heading's link opens its page", () => {
  const opened: string[] = [];
  const nav = { navigate: () => {}, openRef: (ref: string) => void opened.push(ref) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={runOf()} />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.getByText("Alpha"));
  expect(opened).toEqual(["oxplow:oxplow/page?page=alpha"]);
});

test("no rows says `empty`, else the default", () => {
  const custom = render(<VizView run={runOf({ empty: "Nothing installed." }, [])} onOpenPage={() => {}} />);
  expect(custom.getByTestId("viz-empty").textContent).toBe("Nothing installed.");
  cleanup();
  const plain = render(<VizView run={runOf({}, [])} onOpenPage={() => {}} />);
  expect(plain.getByTestId("viz-empty").textContent).toBe("No rows.");
});

test("a long list of subsections is drawn in the reading column: no grid, no widening", () => {
  const many = Array.from({ length: 30 }, (_, i): SqlCell[] => [`p${i}`, `Page ${i}`, null, "page", `Description ${i}`, 0]);
  const view = render(<VizView run={runOf({}, many)} onOpenPage={() => {}} />);
  expect(view.container.querySelectorAll("section.ox-section").length).toBe(30);
  expect(view.container.querySelector("table, [class*=grid]")).toBeNull();
});
