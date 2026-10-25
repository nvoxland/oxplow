import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { createPageRuns, type PageKey } from "../pageRuns.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { extPageRef, viewParamsOf } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { LensParam, Page, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageFileView } from "./PageFileView.js";
import { decl, fakePage, fakePageRuns, fakeRun, loader, node } from "./testing/fakeRun.js";

afterEach(cleanup);

const param = (name: string, role: LensParam["role"], over: Partial<LensParam> = {}): LensParam =>
  ({ name, label: null, default: null, options: [], type: "text", role, presets: [], ...over }) as LensParam;

const listPage: Page = fakePage(
  node.viz("items", "items", { columns: [{ key: "id", label: "Id", select: "item" } as never, { key: "title" } as never] }),
  {
    id: "acme/list",
    slug: "list",
    title: "List",
    loaders: [decl("items")],
    params: [param("project", "resource"), param("q", "filter", { default: "", label: "Search" }), param("item", "selection")],
  },
);
const listRun = (params: Record<string, SqlCell> = {}, rows: SqlCell[][] = [["a", "Alpha"], ["b", "Beta"]]): PageRun =>
  fakeRun(listPage, { params, loaders: { items: loader(["id", "title"], rows) } });

interface Moves {
  sets: { view: Record<string, SqlCell>; push: boolean }[];
  navs: string[];
}

function inTab(ref: TabRef, moves: Moves, child: React.ReactNode) {
  const nav = {
    navigate: (r: TabRef) => moves.navs.push(r.id),
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref,
    viewParams: viewParamsOf(ref),
    setViewParams: (view: Record<string, SqlCell>, opts?: { push?: boolean }) => moves.sets.push({ view, push: !!opts?.push }),
    setTitle: () => {},
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{child}</PageNavigationContext.Provider>;
}

const moves = (): Moves => ({ sets: [], navs: [] });

test("its run's params are the URI's `?` and the filters and picks of its `#`", async () => {
  const { runs, calls } = fakePageRuns(listRun());
  const ref = extPageRef("acme", "list", { project: "p1" }, { q: "al", item: "a", node: "items" });
  render(inTab(ref, moves(), <PageFileView page={listPage} resource={{ project: "p1" }} stream={{ id: "str1" } as never} threadId="thr1" runs={runs} />));
  await waitFor(() => expect(calls.open.length).toBe(1));
  expect(calls.open[0]).toEqual({ page: "acme/list", params: { project: "p1", q: "al", item: "a" }, streamId: "str1", threadId: "thr1" } as PageKey);
});

test("a filter replaces the entry's view; the run is never told directly", async () => {
  const m = moves();
  const { runs, calls } = fakePageRuns(listRun());
  const view = render(inTab(extPageRef("acme", "list", { project: "p1" }), m, <PageFileView page={listPage} resource={{ project: "p1" }} stream={null} threadId={null} runs={runs} />));
  fireEvent.click(await view.findByTestId("lens-param-q-value"));
  const input = view.getByTestId("lens-param-q-edit") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "be" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(m.sets).toEqual([{ view: { q: "be" }, push: false }]));
  expect(calls.setParams).toEqual([]);
  expect(m.navs).toEqual([]);
});

test("a row's pick pushes an entry, and the rail shows what it picked", async () => {
  const m = moves();
  const { runs } = fakePageRuns(listRun());
  const view = render(inTab(extPageRef("acme", "list", { project: "p1" }), m, <PageFileView page={listPage} resource={{ project: "p1" }} stream={null} threadId={null} runs={runs} />));
  await view.findByTestId("lens-row-0");
  fireEvent.click(view.getByTestId("lens-row-0"));
  await waitFor(() => expect(m.sets).toEqual([{ view: { item: "a" }, push: true }]));
});

test("another resource navigates to a new entry, keeping the view", async () => {
  const m = moves();
  const resourcePage: Page = fakePage(node.viz("items", "items", { columns: [{ key: "id", label: "Id", select: "project" } as never] }), {
    id: "acme/list",
    slug: "list",
    params: listPage.params,
    loaders: [decl("items")],
  });
  const { runs } = fakePageRuns(fakeRun(resourcePage, { params: { project: "p1" }, loaders: { items: loader(["id"], [["p2"]]) } }));
  const ref = extPageRef("acme", "list", { project: "p1" }, { q: "x" });
  const view = render(inTab(ref, m, <PageFileView page={resourcePage} resource={{ project: "p1" }} stream={null} threadId={null} runs={runs} />));
  fireEvent.click(await view.findByTestId("lens-row-0"));
  await waitFor(() => expect(m.navs).toEqual([extPageRef("acme", "list", { project: "p2" }, { q: "x" }).id]));
  expect(m.sets).toEqual([]);
});

test("a renderer's other resource is that resource's URI", async () => {
  const m = moves();
  const commitPage: Page = fakePage(node.viz("parents", "parents", { columns: [{ key: "sha", label: "Parent", select: "sha" } as never] }), {
    id: "oxplow-foundation/commit",
    extension: "oxplow-foundation",
    slug: "commit",
    for: ["oxplow/commit"],
    params: [param("uri", "resource"), param("sha", "resource")],
    loaders: [decl("parents")],
  });
  const uri = "oxplow:oxplow/commit?sha=abc";
  const { runs, calls } = fakePageRuns(fakeRun(commitPage, { loaders: { parents: loader(["sha"], [["def"]]) } }));
  const view = render(
    inTab({ id: uri }, m, <PageFileView page={commitPage} resource={{ uri, sha: "abc" }} renders={uri} stream={null} threadId={null} runs={runs} />),
  );
  await waitFor(() => expect(calls.open[0]?.params).toEqual({ uri, sha: "abc" }));
  fireEvent.click(await view.findByTestId("lens-row-0"));
  await waitFor(() => expect(m.navs).toEqual(["oxplow:oxplow/commit?sha=def"]));
});

test("a view change re-keys the run; the old rows stay, marked, until the new run answers", async () => {
  let answer: (r: { runId: string; run: PageRun }) => void = () => {};
  const opened: PageKey[] = [];
  const runs = createPageRuns({
    open: (key) => {
      opened.push(key);
      if (opened.length === 1) return Promise.resolve({ runId: "run-1", run: listRun() });
      return new Promise((r) => (answer = r));
    },
    close: async () => {},
    fetch: async () => ({}),
    setParams: async () => listRun(),
    rerun: async () => listRun(),
    onChange: () => () => {},
    onReconnect: () => () => {},
  });
  const m = moves();
  const draw = (ref: TabRef) => inTab(ref, m, <PageFileView page={listPage} resource={{ project: "p1" }} stream={null} threadId={null} runs={runs} />);
  const view = render(draw(extPageRef("acme", "list", { project: "p1" })));
  await view.findByText("Alpha");
  view.rerender(draw(extPageRef("acme", "list", { project: "p1" }, { q: "be" })));
  await waitFor(() => expect(opened.length).toBe(2));
  expect(opened[1]!.params).toEqual({ project: "p1", q: "be" });
  await act(() => new Promise<void>((r) => setTimeout(r, 200)));
  expect(view.getByText("Alpha")).toBeTruthy();
  expect(view.getByTestId("page-file").getAttribute("data-stale")).toBe("true");
  await act(async () => answer({ runId: "run-2", run: listRun({ q: "be" }, [["b", "Beta"]]) }));
  await waitFor(() => expect(view.getByText("Beta")).toBeTruthy());
  expect(view.queryByText("Alpha")).toBeNull();
  expect(view.getByTestId("page-file").hasAttribute("data-stale")).toBe(false);
});

test("`#node=` lands on the node, and the open page publishes its run and focus", async () => {
  const { runs } = fakePageRuns(listRun());
  const ref = extPageRef("acme", "list", { project: "p1" }, { node: "items" });
  const view = render(inTab(ref, moves(), <PageFileView page={listPage} resource={{ project: "p1" }} stream={null} threadId={null} runs={runs} />));
  await waitFor(() => expect(view.container.querySelector('[data-node="items"]')?.getAttribute("data-focused")).toBe("true"));
  await waitFor(() => expect(getPageDetailStore().get(ref.id)).toEqual({ runId: "run-1", focus: { node: "items" } }));
  cleanup();
  expect(getPageDetailStore().get(ref.id)).toBeNull();
});
