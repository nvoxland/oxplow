import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

// One title cache for a link to any resource's row: the title comes from
// the resource's `resolve` dataset and title column (`v_resource`), whatever
// the resource — a made-up `acme/ticket` here — and a burst of changes is
// one re-read.

import { resourceUri } from "./uri/uri.js";
import { setResources, type Resource } from "./resourceStore.js";

const realApi = { ...(await import("./api.js")) }; // a copy: `mock.module` patches a loaded module in place
const asked: Array<{ sql: string; params: unknown[] }> = [];
let titles: Record<string, string> = { "AC-7": "Fix the parser" };
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string, params: unknown[]) => {
    asked.push({ sql, params });
    const id = new URL(String(params[0]).replace(/^oxplow:/, "http://x/")).searchParams.get("id") ?? "";
    return {
      columns: ["name"],
      rows: titles[id] ? [[titles[id]]] : [],
      truncated: false,
      reads: { datasets: ["v_acme_ticket"], tables: [], measures: [] },
    };
  },
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.push(l);
    return () => listeners.splice(listeners.indexOf(l), 1);
  },
}));
const { useRefTitle, clearRefTitles } = (await import("./refTitleCache.js?fresh")) as typeof import("./refTitleCache.js");

const resource = (name: string, resolve: string | null, title: string | null): Resource => ({
  resource: name,
  extension: name.split("/")[0]!,
  label: "Ticket",
  params: { id: { required: true, pattern: null } },
  additional: false,
  resolve,
  title,
  searchable: null,
  icon: null,
  wikilink: null,
});

afterEach(cleanup);
// A test that changes a title doesn't change the next one's.
beforeEach(() => {
  titles = { "AC-7": "Fix the parser" };
});

const TICKET = resourceUri("acme/ticket", { id: "AC-7" });

function Title({ uri }: { uri: string }) {
  const { title, status } = useRefTitle(uri);
  return <span data-testid="t">{`${status}:${title ?? "-"}`}</span>;
}

test("a row's title comes from its resource's dataset and title column, with no code for the resource", async () => {
  clearRefTitles();
  setResources([resource("acme/ticket", "v_acme_ticket", "name")]);
  const view = render(<Title uri={TICKET} />);
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("found:Fix the parser"));
  expect(asked[0]!.sql).toBe("SELECT name FROM v_acme_ticket WHERE uri = ?1 LIMIT 1");
});

test("a row the dataset lacks is missing; a resource with no dataset is untitled", async () => {
  clearRefTitles();
  setResources([resource("acme/ticket", "v_acme_ticket", "name"), resource("acme/note", null, null)]);
  const gone = render(<Title uri={resourceUri("acme/ticket", { id: "AC-0" })} />);
  await waitFor(() => expect(gone.container.textContent).toBe("missing:-"));
  const note = render(<Title uri={resourceUri("acme/note", { id: "1" })} />);
  await waitFor(() => expect(note.container.textContent).toBe("untitled:-"));
});

test("a ref asked for before its resource loads waits, then reads", async () => {
  clearRefTitles();
  setResources([]);
  const view = render(<Title uri={TICKET} />);
  expect(view.getByTestId("t").textContent).toBe("loading:-");
  act(() => setResources([resource("acme/ticket", "v_acme_ticket", "name")]));
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("found:Fix the parser"));
});

test("a burst of changes to the dataset is one re-read, and the last change is seen", async () => {
  clearRefTitles();
  setResources([resource("acme/ticket", "v_acme_ticket", "name")]);
  titles = { "AC-7": "Fix the parser" };
  const view = render(<Title uri={TICKET} />);
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("found:Fix the parser"));
  const before = asked.length;
  titles = { "AC-7": "Fix the lexer" };
  for (let i = 0; i < 10; i++) {
    act(() => listeners.forEach((l) => l({ kind: "datasetsChanged", datasets: ["v_acme_ticket"] })));
    await new Promise((r) => setTimeout(r, 20));
  }
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("found:Fix the lexer"));
  await new Promise((r) => setTimeout(r, 250));
  expect(asked.length - before).toBe(1);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("./api.js", () => realApi);
  setResources([]);
});
