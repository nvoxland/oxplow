import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { RememberedContext, createRememberedStore } from "../prefs/remembered.js";
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

test("Save as knowledge page offers the store's own command at the knowledge.new point, not a save of its own", async () => {
  const calls: [string, unknown][] = [];
  const f = form("knowledge", async (name, input) => { calls.push([name, input]); return {}; });
  // The title is asked here; the writing is the point's command (a button or a menu).
  expect(f.view.getByTestId("save-view-title")).toBeTruthy();
  expect(f.view.queryByTestId("save-view-submit")).toBeNull();
  expect(f.view.queryByTestId("save-view-store")).toBeNull();
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await new Promise((r) => setTimeout(r, 10));
  expect(calls).toEqual([]);
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

test("a temp view's YAML is read from its file on oxplow:temp", async () => {
  const f = tempForm(spec);
  await f.view.findByTestId("save-view-title");
  expect(f.reads).toEqual([["str1", "v1.yaml"]]);
});

test("a temp view that is gone can't be saved", async () => {
  const f = tempForm(null);
  expect((await f.view.findByRole("alert")).textContent).toContain("This view is gone");
  expect(f.view.queryByTestId("save-view-submit")).toBeNull();
});
