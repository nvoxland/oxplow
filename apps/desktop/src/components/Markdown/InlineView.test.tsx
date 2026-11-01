import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { createPageRuns, PageRunsContext, type PageKey, type PageRunsTransport } from "../../pageRuns.js";
import { fakePage, fakePageRuns, fakeRun, node } from "../../pages/testing/fakeRun.js";
import { PageViewContext } from "../../pages/PageViewContext.js";
import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { selectedThreadId, setSelectedThread } from "../../contextBasket.js";
import { Markdown } from "./Markdown.js";
import { InlineView, INLINE_PAGE_MAX_BYTES } from "./InlineView.js";
import { getMarkdown } from "./richTextExtensions.js";
import type { Editor } from "@tiptap/core";

afterEach(cleanup);

const SPEC = "title: Release\nview: { markdown: Healthy }";
const FENCE = `Before\n\n\`\`\`oxplow-page\n${SPEC}\n\`\`\`\n\nAfter`;
const run = () => fakeRun(fakePage(node.markdown("status", "Healthy")));

for (const mode of ["view", "edit"] as const) {
  test(`${mode}: a page fence opens a live inline-spec run and renders its tree without page chrome`, async () => {
    const { runs, calls } = fakePageRuns(run());
    const view = render(
      <PageRunsContext.Provider value={runs}>
        <Markdown value={FENCE} {...(mode === "edit" ? { onChange: () => {} } : {})} />
      </PageRunsContext.Provider>,
    );
    await waitFor(() => expect(view.getByTestId("inline-page").textContent).toContain("Healthy"));
    expect(calls.open).toHaveLength(1);
    expect(calls.open[0]?.specYaml?.trim()).toBe(SPEC);
    expect(view.queryByTestId("markdown-code-copy") === null).toBe(true);
    expect(view.container.querySelector('[data-node="status"]')?.textContent).toBe("Healthy");
    expect(view.container.textContent).toContain("Before");
    expect(view.container.textContent).toContain("After");
  });
}

test("a rejected inline spec shows its source and daemon's validation error", async () => {
  const transport: PageRunsTransport = {
    open: async () => { throw new Error("line 3: inline pages cannot run command loaders"); },
    close: async () => {}, fetch: async () => ({}), setParams: async () => run(), rerun: async () => run(), setVisible: async () => false,
    onChange: () => () => {}, onReconnect: () => () => {},
  };
  const view = render(<PageRunsContext.Provider value={createPageRuns(transport)}><InlineView specYaml={SPEC} /></PageRunsContext.Provider>);
  await waitFor(() => expect(view.getByTestId("inline-page-error").textContent).toContain("line 3"));
  expect(view.getByTestId("inline-page-error").textContent).toContain(SPEC);
});

test("a restored wiki view binds the inline page's declared picks without leaking host view keys", async () => {
  const { runs, calls } = fakePageRuns(run());
  const source = "title: Picked\nparams: [{ name: selected, role: selection }]\nview: { markdown: Ready }";
  const nav = { viewParams: { selected: "kept-row", comment: "cmt3" } } as unknown as PageNavigation;
  render(<PageNavigationContext.Provider value={nav}><PageRunsContext.Provider value={runs}>
    <InlineView specYaml={source} />
  </PageRunsContext.Provider></PageNavigationContext.Provider>);
  await waitFor(() => expect(calls.open).toHaveLength(1));
  expect(calls.open[0]?.params).toEqual({ selected: "kept-row" });
});

test("an inline block inherits explicit scope and releases its run when removed", async () => {
  const opened: PageKey[] = [];
  const closed: string[] = [];
  const transport: PageRunsTransport = {
    open: async (key) => { opened.push(key); return { runId: "inline", run: run() }; },
    close: async (id) => { closed.push(id); }, fetch: async () => ({}), setParams: async () => run(), rerun: async () => run(), setVisible: async () => false,
    onChange: () => () => {}, onReconnect: () => () => {},
  };
  const view = render(<PageRunsContext.Provider value={createPageRuns(transport)}><InlineView specYaml={SPEC} streamId="str2" threadId="thr5" /></PageRunsContext.Provider>);
  await waitFor(() => expect(opened).toHaveLength(1));
  expect(opened[0]).toMatchObject({ streamId: "str2", threadId: "thr5", specYaml: SPEC });
  view.unmount();
  await waitFor(() => expect(closed).toEqual(["inline"]));
});

test("an inline block keeps its containing page's scope when another thread is selected", async () => {
  const previous = selectedThreadId();
  const { runs, calls } = fakePageRuns(run());
  try {
    setSelectedThread("thr9");
    const view = render(
      <PageRunsContext.Provider value={runs}>
        <PageViewContext.Provider value={{ streamId: "str2", threadId: "thr5" }}>
          <InlineView specYaml={SPEC} />
        </PageViewContext.Provider>
      </PageRunsContext.Provider>,
    );
    await waitFor(() => expect(view.getByTestId("inline-page").textContent).toContain("Healthy"));
    await act(async () => setSelectedThread("thr10"));
    expect(calls.open).toHaveLength(1);
    expect(calls.open[0]).toMatchObject({ streamId: "str2", threadId: "thr5" });
  } finally {
    await act(async () => setSelectedThread(previous));
  }
});

test("oversized and recursively nested blocks stop before opening another run", async () => {
  const recursive = fakeRun(fakePage(node.markdown("nested", `\`\`\`oxplow-page\n${SPEC}\n\`\`\``)));
  const { runs, calls } = fakePageRuns(recursive);
  const view = render(<PageRunsContext.Provider value={runs}><InlineView specYaml={`${SPEC}\n`} /></PageRunsContext.Provider>);
  await waitFor(() => expect(view.getByTestId("inline-page-error").textContent).toContain("nesting"));
  // Identical nested specs share one open run; depth still stays bounded.
  expect(calls.open).toHaveLength(1);
  cleanup();
  const large = render(<PageRunsContext.Provider value={runs}><InlineView specYaml={"x".repeat(INLINE_PAGE_MAX_BYTES + 1)} /></PageRunsContext.Provider>);
  expect(large.getByTestId("inline-page-error").textContent).toContain("64 KiB");
  expect(calls.open).toHaveLength(1);
});

test("the editor opens YAML with its pencil, Escape returns to the live view and the fence round-trips", async () => {
  const { runs } = fakePageRuns(run());
  const view = render(<PageRunsContext.Provider value={runs}><Markdown value={FENCE} onChange={() => {}} testId="md" /></PageRunsContext.Provider>);
  await waitFor(() => expect(view.getByTestId("inline-page").textContent).toContain("Healthy"));
  const editable = view.getByTestId("md") as HTMLElement & { editor?: Editor };
  expect(getMarkdown(editable.editor!).trim()).toBe(FENCE);
  await act(async () => { fireEvent.mouseDown(view.getByTitle("Edit YAML")); });
  const block = view.container.querySelector(".oxplow-rt-page")!;
  expect(block.getAttribute("data-editing")).toBe("true");
  expect(view.queryByTestId("inline-page") === null).toBe(true);
  await act(async () => { fireEvent.keyDown(block, { key: "Escape" }); });
  await waitFor(() => expect(block.getAttribute("data-editing")).toBe("false"));
  expect(getMarkdown(editable.editor!).trim()).toBe(FENCE);
});
