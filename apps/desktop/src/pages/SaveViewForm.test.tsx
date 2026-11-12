import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { RememberedContext, createRememberedStore } from "../prefs/remembered.js";
import { setKnowledgeStores } from "../knowledgeStores.js";
import { SaveTempViewForm, SaveViewCommandContext, SaveViewForm, SaveViewSourceContext } from "./SaveViewForm.js";

afterEach(cleanup);
const spec = "# Comment\ntitle: View\nloaders: { items: { rows: [{ title: Alpha }] } }\nview: { viz: table, from: items }\n";

function form(kind: "page" | "knowledge", command: (name: string, input: unknown) => Promise<unknown>) {
  const opened: string[] = [];
  let closed = 0;
  const prefs = createRememberedStore({ load: async () => [], save: async () => {} });
  const view = render(<RememberedContext.Provider value={prefs}><SaveViewCommandContext.Provider value={command}>
    <SaveViewForm kind={kind} specYaml={spec} initialTitle="Busy Tasks" onClose={() => { closed += 1; }} onSaved={(uri) => opened.push(uri)} />
  </SaveViewCommandContext.Provider></RememberedContext.Provider>);
  return { view, opened, closed: () => closed, prefs };
}

test("Save as Page writes and deploys the exact view and remembers its extension", async () => {
  const calls: [string, unknown][] = [];
  // The daemon says the page's id: its extension's namespace's, not its name.
  const f = form("page", async (name, input) => { calls.push([name, input]); return { result: { page: "my_personal/busy-tasks" } }; });
  fireEvent.change(f.view.getByTestId("save-view-extension"), { target: { value: "my-personal" } });
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/page?page=my_personal/busy-tasks"]));
  expect(calls[0][0]).toBe("oxplow.extension.write_page");
  expect(calls[0][1]).toMatchObject({ extension: "my-personal", page: "busy-tasks", deploy: true });
  expect((calls[0][1] as { spec: string }).spec).toContain("# Comment");
  expect(f.prefs.get("app", "pages.extension")).toBe("my-personal");
  expect(f.closed()).toBe(1);
});

test("Save as knowledge page writes the live block, lets the store derive the id and navigates to the page the command names", async () => {
  const calls: [string, unknown][] = [];
  const f = form("knowledge", async (name, input) => { calls.push([name, input]); return { result: { page: "oxplow:oxplow/knowledge?id=busy-tasks&store=oxwiki" } }; });
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/knowledge?id=busy-tasks&store=oxwiki"]));
  expect(calls[0][0]).toBe("oxplow.knowledge.write_page");
  expect((calls[0][1] as { body: string }).body).toContain("```oxplow-page\n# Comment");
  expect(calls[0][1]).toMatchObject({ title: "Busy Tasks" });
  expect(calls[0][1]).not.toHaveProperty("id");
  expect(calls[0][1]).not.toHaveProperty("store");
});

test("with several stores that edit, a store is chosen and sent; one that cannot edit is not offered", async () => {
  setKnowledgeStores([
    { store: "oxwiki", title: "oxwiki", kinds: [], edit: true },
    { store: "docs", title: "Docs", kinds: [], edit: true },
    { store: "ro", title: "Read only", kinds: [], edit: false },
  ]);
  try {
    const calls: [string, unknown][] = [];
    const f = form("knowledge", async (name, input) => { calls.push([name, input]); return { result: { page: "oxplow:oxplow/knowledge?id=busy-tasks&store=docs" } }; });
    const select = f.view.getByTestId("save-view-store") as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(["oxwiki", "docs"]);
    fireEvent.change(select, { target: { value: "docs" } });
    fireEvent.submit(f.view.getByTestId("save-view-form"));
    await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/knowledge?id=busy-tasks&store=docs"]));
    expect(calls[0][1]).toMatchObject({ store: "docs" });
  } finally {
    setKnowledgeStores([]);
  }
});

test("with one store that edits among several enabled, it is named without asking", async () => {
  setKnowledgeStores([
    { store: "oxwiki", title: "oxwiki", kinds: [], edit: true },
    { store: "ro", title: "Read only", kinds: [], edit: false },
  ]);
  try {
    const calls: [string, unknown][] = [];
    const f = form("knowledge", async (name, input) => { calls.push([name, input]); return { result: { page: "oxplow:oxplow/knowledge?id=busy-tasks&store=oxwiki" } }; });
    expect(f.view.queryByTestId("save-view-store")).toBeNull();
    fireEvent.submit(f.view.getByTestId("save-view-form"));
    await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/knowledge?id=busy-tasks&store=oxwiki"]));
    expect(calls[0][1]).toMatchObject({ store: "oxwiki" });
  } finally {
    setKnowledgeStores([]);
  }
});

test("a refused save keeps the form and source for retry; Escape cancels", async () => {
  const f = form("page", async () => { throw new Error("Only the main stream can deploy"); });
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  expect((await f.view.findByRole("alert")).textContent).toContain("main stream");
  expect(f.opened).toEqual([]);
  expect(f.closed()).toBe(0);
  fireEvent.keyDown(f.view.getByTestId("save-view-title"), { key: "Escape" });
  expect(f.closed()).toBe(1);
});

function tempForm(source: string | null) {
  const reads: [string | null, string][] = [];
  const calls: [string, unknown][] = [];
  const prefs = createRememberedStore({ load: async () => [], save: async () => {} });
  const view = render(<RememberedContext.Provider value={prefs}><SaveViewCommandContext.Provider value={async (name, input) => { calls.push([name, input]); return { result: { page: "oxplow:oxplow/knowledge?id=x&store=oxwiki" } }; }}>
    <SaveViewSourceContext.Provider value={async (streamId, file) => { reads.push([streamId, file]); return source; }}>
      <SaveTempViewForm kind="knowledge" file="v1.yaml" streamId="str1" initialTitle="Busy Tasks" onClose={() => {}} onSaved={() => {}} />
    </SaveViewSourceContext.Provider>
  </SaveViewCommandContext.Provider></RememberedContext.Provider>);
  return { view, reads, calls };
}

test("a temp view's YAML is read from its file on oxplow:temp, then saved as it is", async () => {
  const f = tempForm(spec);
  await f.view.findByTestId("save-view-title");
  expect(f.reads).toEqual([["str1", "v1.yaml"]]);
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await waitFor(() => expect(f.calls[0]?.[0]).toBe("oxplow.knowledge.write_page"));
  expect((f.calls[0]![1] as { body: string }).body).toContain("```oxplow-page\n# Comment");
});

test("a temp view that is gone can't be saved", async () => {
  const f = tempForm(null);
  expect((await f.view.findByRole("alert")).textContent).toContain("This view is gone");
  expect(f.view.queryByTestId("save-view-submit")).toBeNull();
});
