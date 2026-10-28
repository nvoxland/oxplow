import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { RememberedContext, createRememberedStore } from "../prefs/remembered.js";
import { SaveViewCommandContext, SaveViewForm } from "./SaveViewForm.js";

afterEach(cleanup);
const spec = "# Comment\ntitle: View\nloaders: { items: { rows: [{ title: Alpha }] } }\nview: { viz: table, from: items }\n";

function form(kind: "page" | "wiki", command: (name: string, input: unknown) => Promise<unknown>) {
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
  const f = form("page", async (name, input) => { calls.push([name, input]); });
  fireEvent.change(f.view.getByTestId("save-view-extension"), { target: { value: "personal" } });
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/page?page=personal/busy-tasks"]));
  expect(calls[0][0]).toBe("oxplow.extension.write_page");
  expect(calls[0][1]).toMatchObject({ extension: "personal", page: "busy-tasks", deploy: true });
  expect((calls[0][1] as { spec: string }).spec).toContain("# Comment");
  expect(f.prefs.get("app", "pages.extension")).toBe("personal");
  expect(f.closed()).toBe(1);
});

test("Save to wiki writes the live block and navigates only after success", async () => {
  const calls: [string, unknown][] = [];
  const f = form("wiki", async (name, input) => { calls.push([name, input]); });
  fireEvent.submit(f.view.getByTestId("save-view-form"));
  await waitFor(() => expect(f.opened).toEqual(["oxplow:oxplow/wiki?slug=busy-tasks"]));
  expect(calls[0][0]).toBe("oxplow.knowledge.write_page");
  expect((calls[0][1] as { body: string }).body).toContain("```oxplow-page\n# Comment");
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
