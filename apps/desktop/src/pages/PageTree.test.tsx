import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageTree, NodeRegistryContext, createNodeRegistry, useNodeFocus, type NodeRegistry } from "./PageTree.js";
import { PageActionContext, type RunPageAction } from "./useNodeActions.js";
import { fakeAction, fakePage, fakeRun, gridPage, loader, node, stackPage, switchPage, tablePage } from "./testing/fakeRun.js";
import type { PageRun } from "../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const nodeIds = (root: ParentNode) => [...root.querySelectorAll("[data-node]")].map((e) => e.getAttribute("data-node"));

test("every node kind draws with its id; an embed's nodes are prefixed with the embed's", () => {
  const run = stackPage();
  const { container, getByText } = render(<PageTree run={run} runId="run-1" />);
  expect(nodeIds(container)).toEqual(["stack", "intro", "items", "notes", "notes/notes", "more", "chart"]);
  // A stack's children are sections, titled.
  const intro = container.querySelector('[data-node="intro"]')!;
  expect(intro.querySelector(".ox-section-title")?.textContent).toBe("Intro");
  // Markdown is the Markdown component's.
  expect(getByText("there").tagName).toBe("STRONG");
  // The viz shows its loader's rows; the embed its own page's.
  expect(container.querySelector('[data-node="items"]')!.textContent).toContain("a");
  expect(container.querySelector('[data-node="notes/notes"]')!.textContent).toContain("inner note");
  // An empty slot draws nothing; a component with no daemon to serve it
  // says so in its place.
  expect(container.querySelector('[data-node="more"]')!.textContent).toBe("");
  expect(container.querySelector('[data-node="chart"] [data-testid="custom-component-fallback"]')).not.toBeNull();
});

// A section's `description` is the ⓘ after its heading: the sentence is
// its hover and its accessible name, and the page stays quiet.
test("a section's description is a ⓘ after its heading with the text as its hover", () => {
  const page = fakePage(
    node.stack("s", [
      node.viz("items", "items", { columns: [{ key: "id" }] }, { title: "Items", description: "What is there." }),
      node.viz("more", "items", { columns: [{ key: "id" }] }, { title: "More" }),
    ]),
  );
  const run = fakeRun(page, { loaders: { items: loader(["id"], [["a"]]) } });
  const { container } = render(<PageTree run={run} runId="run-1" />);
  const help = container.querySelector('[data-node="items"] [data-testid="section-help"]')!;
  expect(help.getAttribute("title")).toBe("What is there.");
  expect(help.getAttribute("aria-label")).toBe("What is there.");
  expect(help.closest(".ox-section-head")).not.toBeNull();
  expect(container.querySelector('[data-node="more"] [data-testid="section-help"]')).toBeNull();
});

test("a switch draws only the child the server put in view", () => {
  const list = render(<PageTree run={switchPage("list")} runId="run-1" />);
  expect(nodeIds(list.container)).toEqual(["modes", "list"]);
  cleanup();
  const board = render(<PageTree run={switchPage("board")} runId="run-1" />);
  expect(nodeIds(board.container)).toEqual(["modes", "board"]);
});

test("a grid's children are tiles at their size; a failed loader says why in its tile", () => {
  const { container } = render(<PageTree run={gridPage()} runId="run-1" />);
  expect(nodeIds(container)).toEqual(["tiles", "count", "broken"]);
  const broken = container.querySelector('[data-node="broken"]')!;
  expect(broken.closest(".ox-tile-slot")!.className).toContain("ox-tile-wide");
  expect(broken.querySelector(".ox-tile-title")?.textContent).toBe("Broken");
  expect(broken.querySelector(".ox-error")?.textContent).toContain("no such table: v_nope");
  expect(container.querySelector('[data-node="count"]')!.textContent).toContain("3");
});

test("a node the server couldn't run shows its error inline", () => {
  const run = stackPage();
  const tree = { ...run.tree, children: run.tree.children.map((c) => (c.id === "notes" ? { ...c, embed: null, error: "page acme/notes is gone" } : c)) };
  const { container } = render(<PageTree run={{ ...run, tree }} runId="run-1" />);
  expect(container.querySelector('[data-node="notes"] .ox-error')?.textContent).toContain("page acme/notes is gone");
});

function recorder() {
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(args);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  return { calls, press };
}

test("a row action runs on the open run, naming the node and the row", async () => {
  const { calls, press } = recorder();
  const run = tablePage({ rowActions: [fakeAction({ id: "finish", label: "Finish", row: true })] });
  const { getByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageTree run={run} runId="run-1" />
    </PageActionContext.Provider>,
  );
  fireEvent.contextMenu(getByTestId("viz-row-1"));
  fireEvent.click(getByTestId("menu-item-viz-action-finish"));
  await waitFor(() => expect(calls).toEqual([["run-1", "items", "finish", { id: "b", title: "Beta" }, null, null, false]]));
});

test("an embedded node's action names it by its path", async () => {
  const { calls, press } = recorder();
  const inner = fakeRun(
    fakePage(node.viz("notes", "notes", { columns: [{ key: "note" }], actions: [fakeAction({ id: "drop", row: true })] }), { id: "acme/notes" }),
    { loaders: { notes: loader(["note"], [["n1"]]) } },
  );
  const run = fakeRun(fakePage(node.stack("s", [node.embed("side", "acme/notes")])), { embeds: { side: inner } });
  const { getByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageTree run={run} runId="run-1" />
    </PageActionContext.Provider>,
  );
  fireEvent.contextMenu(getByTestId("viz-row-0"));
  fireEvent.click(getByTestId("menu-item-viz-action-drop"));
  await waitFor(() => expect(calls).toEqual([["run-1", "side/notes", "drop", { note: "n1" }, null, null, false]]));
});

function Focused({ run, registry, focus, ms }: { run: PageRun; registry: NodeRegistry; focus: string | null; ms?: number }) {
  useNodeFocus(registry, focus, ms);
  return (
    <NodeRegistryContext.Provider value={registry}>
      <PageTree run={run} runId="run-1" />
    </NodeRegistryContext.Provider>
  );
}

test("`#node=<id>` scrolls to the node and marks it a moment", async () => {
  const scrolled: string[] = [];
  // An own property on HTMLElement's prototype, taken away after: the real
  // one (Element's) shows through again.
  const proto = HTMLElement.prototype as unknown as { scrollIntoView: (o?: unknown) => void };
  const owned = Object.getOwnPropertyDescriptor(proto, "scrollIntoView");
  proto.scrollIntoView = function (this: HTMLElement) {
    scrolled.push(this.getAttribute("data-node") ?? "");
  };
  try {
    const registry = createNodeRegistry();
    const { container } = render(<Focused run={stackPage()} registry={registry} focus="notes/notes" ms={40} />);
    const el = container.querySelector<HTMLElement>('[data-node="notes/notes"]')!;
    expect(scrolled).toEqual(["notes/notes"]);
    expect(el.getAttribute("data-focused")).toBe("true");
    await act(() => new Promise((r) => setTimeout(r, 80)));
    expect(el.hasAttribute("data-focused")).toBe(false);
  } finally {
    if (owned) Object.defineProperty(proto, "scrollIntoView", owned);
    else delete (proto as { scrollIntoView?: unknown }).scrollIntoView;
  }
});

test("a focus on a node not drawn yet lands once it registers", () => {
  const registry = createNodeRegistry();
  const scrolled: string[] = [];
  function Late() {
    useNodeFocus(registry, "late");
    return null;
  }
  render(<Late />);
  const el = document.createElement("div");
  el.scrollIntoView = () => scrolled.push("late");
  registry.register("late", el);
  expect(scrolled).toEqual(["late"]);
  expect(el.getAttribute("data-focused")).toBe("true");
  // Registering again (a re-render) doesn't land again.
  registry.register("late", el);
  expect(scrolled).toEqual(["late"]);
});

// A stack's titled sections with nothing to show fold into one "Nothing
// found" line, as a page document's parts do: no rows, no error, not a
// form, nothing to press above its rows.
test("a stack folds its empty titled sections into one Nothing found line", () => {
  const page = fakePage(
    node.stack("stack", [
      node.viz("full", "full", { columns: [{ key: "id" }] }, { title: "Full" }),
      node.viz("none", "none", { columns: [{ key: "id" }] }, { title: "Providers" }),
      node.viz("also", "also", { columns: [{ key: "id" }] }, { title: "Roles" }),
      node.viz("add", "none", { columns: [{ key: "id" }], actions: [fakeAction({ id: "add" })] }, { title: "With Add" }),
      node.viz("form", null, { viz: "form" }, { title: "Form" }),
    ]),
  );
  const run = fakeRun(page, { loaders: { full: loader(["id"], [["a"]]), none: loader(["id"], []), also: loader(["id"], []) } });
  const { container, getByTestId } = render(<PageTree run={run} runId="run-1" />);
  expect(nodeIds(container)).toEqual(["stack", "full", "add", "form"]);
  expect(getByTestId("page-nothing-found").textContent).toBe("Nothing found: Providers, Roles.");
});

test("a page a slot mounted is a section titled by that page", () => {
  const run = stackPage();
  const mounted = fakeRun(
    fakePage(node.viz("review", "review", { columns: [{ key: "note" }] }), { id: "beta/review", title: "Change Analysis" }),
    { loaders: { review: loader(["note"], [["look here"]]) } },
  );
  const tree = {
    ...run.tree,
    children: run.tree.children.map((c) =>
      c.id === "more" ? { ...c, children: [{ id: "beta.review", children: [], embed: mounted, error: null }] } : c,
    ),
  };
  const { container } = render(<PageTree run={{ ...run, tree } as PageRun} runId="run-1" />);
  const slot = container.querySelector('[data-node="beta.review"]')!;
  expect(slot.querySelector(".ox-section-title")?.textContent).toBe("Change Analysis");
  expect(slot.textContent).toContain("look here");
});

test("a page a slot mounted with nothing to show is left out", () => {
  const run = stackPage();
  const mounted = fakeRun(
    fakePage(node.viz("review", "review", { columns: [{ key: "note" }] }), { id: "beta/review", title: "Change Analysis" }),
    { loaders: { review: loader(["note"], []) } },
  );
  const tree = {
    ...run.tree,
    children: run.tree.children.map((c) =>
      c.id === "more" ? { ...c, children: [{ id: "beta.review", children: [], embed: mounted, error: null }] } : c,
    ),
  };
  const { container } = render(<PageTree run={{ ...run, tree } as PageRun} runId="run-1" />);
  expect(container.querySelector('[data-node="beta.review"]')).toBeNull();
});

test("an embedded page's own actions are buttons above it, pressed naming the embed's node", async () => {
  const { calls, press } = recorder();
  const inner = fakeRun(
    fakePage(node.viz("notes", "notes", { columns: [{ key: "note" }] }), {
      id: "acme/notes",
      title: "Notes",
      actions: [fakeAction({ id: "tidy", label: "Tidy" })],
    }),
    { loaders: { notes: loader(["note"], [["n"]]) } },
  );
  const host = fakeRun(fakePage(node.stack("s", [node.embed("notes", "acme/notes")])), { embeds: { notes: inner } });
  const { getByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageTree run={host} runId="run-1" />
    </PageActionContext.Provider>,
  );
  fireEvent.click(getByTestId("viz-action-tidy"));
  await waitFor(() => expect(calls).toEqual([["run-1", "notes", "tidy", null, null, null, false]]));
});

// A section's title says what it is; `hide_title` only keeps it from being
// drawn: the heading stays in the page, visually hidden.
test("a hidden-title section keeps its heading in the page, visually hidden", () => {
  const page = fakePage(
    node.stack("stack", [
      node.viz("facts", "items", { columns: [{ key: "id" }] }, { title: "Facts", hideTitle: true }),
      node.viz("more", "items", { columns: [{ key: "id" }] }, { title: "More" }),
    ]),
  );
  const run = fakeRun(page, { loaders: { items: loader(["id"], [["a"]]) } });
  const { container } = render(<PageTree run={run} runId="run-1" />);
  const facts = container.querySelector('[data-node="facts"] section.ox-section')!;
  const heading = facts.querySelector("h2.ox-section-title")!;
  expect(heading.textContent).toBe("Facts");
  expect(heading.className).toContain("ox-sr-only");
  expect(facts.querySelector(".ox-section-head")!.className).toContain("ox-sr-only");
  const more = container.querySelector('[data-node="more"] h2.ox-section-title')!;
  expect(more.className).not.toContain("ox-sr-only");
});
