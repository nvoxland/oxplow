import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// R16: a provider's Approve waits for its declaration diff — disabled while
// it loads, and still disabled, with the reason shown, when it failed.

const realApi = await import("../api.js");
let answer: () => Promise<unknown> = () => new Promise(() => {});
const decided: string[] = [];
const ran: Array<[string, unknown, boolean]> = [];
/** What `effect.retry` answers. */
let retried: unknown = { effect: "acme/mark-done", event: "event:e1", attempt: 2, outcome: "ok" };
const letter = { id: 7, consumer: "change.analyze", event_seq: 41, error: "boom", attempts: 2, first_failed_at: "t", last_failed_at: "t" };
/** The models Settings → Data lists, and what counting each answers (none: it times out). */
let entities: Array<{ name: string; owner: string; kind: string; description: string }> = [];
let counts: Record<string, number> = {};
/** What `effect.backfill_plan` counts. */
let plannedCount = 3;
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) => {
    const counted = /^SELECT count\(\*\) FROM "(\w+)"$/.exec(sql)?.[1];
    if (counted) {
      const n = counts[counted];
      if (n === undefined) throw new Error("query_sql: timed out after 5s");
      return { columns: ["count(*)"], rows: [[n]], truncated: false, reads: { models: [counted], tables: [], measures: [] }, freshness: {} };
    }
    return sql.includes("FROM v_effect_run")
      ? {
          columns: ["effect", "event_id", "event_seq", "attempt", "reason", "event_type"],
          rows: [["acme/mark-done", "e1", 52, 1, "interrupted: a step outside oxplow may have run", "work_item.transitioned"]],
          truncated: false,
          reads: { models: ["v_effect_run", "v_event"], tables: [], measures: [] },
          freshness: {},
        }
      : {
          columns: ["id", "consumer", "event_seq", "event_type", "error", "attempts", "last_failed_at"],
          rows: sql.includes("FROM v_event_dead_letter") ? [[7, "change.analyze", 41, "snapshot.taken", "boom", 2, "t"]] : [],
          truncated: false,
          reads: { models: ["v_event_dead_letter"], tables: [], measures: [] },
          freshness: {},
        };
  },
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push([name, input, confirmed]);
    const result =
      name === "effect.backfill_plan"
        ? { effect: "acme/mark-done", planned: plannedCount, from_seq: 4, to_seq: 9, batch: 200 }
        : name === "effect.backfill"
          ? { effect: "acme/mark-done", planned: 3, ran: 3, skipped: 0, proposed: 0, failed: 0, remaining: 0 }
          : retried;
    return { result, audit_id: 1, event_id: null, inverse: null };
  },
  retryDeadLetter: async (id: number) => {
    decided.push(`retry ${id}`);
    return { ...letter, state: "retried" };
  },
  discardDeadLetter: async (id: number) => {
    decided.push(`discard ${id}`);
    return { ...letter, state: "discarded" };
  },
  listDataEntities: async () => entities,
  listCollectors: async () => [],
  listProjectPrograms: async () => [
    {
      kind: "provider",
      name: "tracker/fake",
      program: "oxplow/extensions/tracker/bin/provider",
      args: [],
      env: [],
      credentials: [],
      network: [],
      tree: "oxplow/extensions/tracker",
      approved: false,
      version: "h1",
    },
    {
      kind: "effect",
      name: "acme/mark-done",
      program: "oxplow/extensions/acme/mark.star",
      args: [],
      env: [],
      credentials: [],
      network: [],
      tree: "oxplow/extensions/acme",
      remote: false,
      approved: true,
      version: "h2",
    },
    {
      kind: "effect",
      name: "oxplow-bundled/verify-unchecked",
      program: "bundled:oxplow-bundled/verify_unchecked.star",
      args: [],
      env: [],
      credentials: [],
      network: [],
      tree: "bundled:oxplow-bundled",
      remote: false,
      approved: true,
      version: "h3",
    },
  ],
  programSource: async () => "def transform(x):\n    return {\"skip\": \"nothing\"}\n",
  providerDeclarationEffects: () => answer(),
  subscribeOxplowEvents: () => () => {},
}));
const { DataSection } = await import("./DataSection.js");

afterEach(() => {
  entities = [];
  counts = {};
  decided.length = 0;
  ran.length = 0;
  cleanup();
});

const approve = (view: ReturnType<typeof render>) =>
  view.container.querySelector('[data-testid^="program-approve-"]') as HTMLButtonElement;

test("Approve stays disabled while the diff loads", async () => {
  answer = () => new Promise(() => {});
  const view = render(<DataSection />);
  await waitFor(() => expect(view.container.textContent).toContain("Comparing its declarations"));
  expect(approve(view).disabled).toBe(true);
});

test("a failed diff is shown, and Approve stays disabled", async () => {
  answer = async () => {
    throw new Error("provider.json doesn't parse");
  };
  const view = render(<DataSection />);
  await waitFor(() => expect(view.container.textContent).toContain("provider.json doesn't parse"));
  expect(view.container.textContent).not.toContain("Comparing its declarations");
  expect(approve(view).disabled).toBe(true);
  expect(approve(view).title).toContain("couldn't compare");
});

// P7.C3: Delivery lists the events a consumer couldn't take; Retry runs
// at once, Discard arms first and runs on Confirm.
test("Delivery retries at once and discards only once confirmed", async () => {
  const view = render(<DataSection />);
  const row = await waitFor(() => view.getByTestId("delivery-row-7"));
  expect(row.textContent).toContain("change.analyze couldn't take snapshot.taken (event 41), 2 times");
  expect(row.textContent).toContain("boom");
  fireEvent.click(view.getByTestId("delivery-retry-7"));
  await waitFor(() => expect(decided).toEqual(["retry 7"]));
  await waitFor(() => expect((view.getByTestId("delivery-retry-7") as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(view.getByTestId("delivery-discard-7-trigger"));
  expect(decided).toEqual(["retry 7"]);
  fireEvent.click(view.getByTestId("delivery-discard-7-confirm"));
  await waitFor(() => expect(decided).toEqual(["retry 7", "discard 7"]));
});

// P9.D4: a failed reaction is listed with why, and Retry — armed first,
// since a step outside oxplow may already have run — runs `effect.retry`
// as the person, confirmed.
test("Delivery lists a failed reaction and retries it only once confirmed", async () => {
  const view = render(<DataSection />);
  const row = await waitFor(() => view.getByTestId("reaction-row-acme/mark-done-e1"));
  expect(row.textContent).toContain("acme/mark-done failed on work_item.transitioned (event 52)");
  expect(row.textContent).toContain("interrupted: a step outside oxplow may have run");
  fireEvent.click(view.getByTestId("reaction-retry-acme/mark-done-e1-trigger"));
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("reaction-retry-acme/mark-done-e1-confirm"));
  await waitFor(() => expect(ran).toEqual([["effect.retry", { effect: "acme/mark-done", event: "event:e1" }, true]]));
});

// P9.D5: an approved effect's row offers Backfill…: it says how many past
// events the effect never reacted to, and runs only on the second click.
test("Backfill… asks with the count and runs effect.backfill once confirmed", async () => {
  const view = render(<DataSection />);
  await waitFor(() => view.getByTestId("program-row-effect:acme/mark-done"));
  // Only an effect's row, and only an approved one, has it.
  expect(view.queryByTestId("effect-backfill-provider:tracker/fake")).toBeNull();
  fireEvent.click(view.getByTestId("effect-backfill-effect:acme/mark-done"));
  const ask = await waitFor(() => view.getByTestId("effect-backfill-ask-effect:acme/mark-done"));
  expect(ask.textContent).toContain("never reacted to 3 matching events");
  expect(ran).toEqual([["effect.backfill_plan", { effect: "acme/mark-done" }, false]]);
  expect(view.getByTestId("effect-backfill-run-effect:acme/mark-done").textContent).toBe("Run on 3 events");
  fireEvent.click(view.getByTestId("effect-backfill-run-effect:acme/mark-done"));
  // It runs on the range it showed (tsk849): what was logged since isn't in it.
  await waitFor(() => expect(ran[1]).toEqual(["effect.backfill", { effect: "acme/mark-done", to_seq: 9 }, true]));
  await waitFor(() => expect(view.queryByTestId("effect-backfill-ask-effect:acme/mark-done")).toBeNull());
});

// tsk850: with nothing to backfill only Close is there — focused, so
// Escape closes the note without a click into it first.
test("Escape closes Backfill's nothing-to-backfill note", async () => {
  plannedCount = 0;
  try {
    const view = render(<DataSection />);
    await waitFor(() => view.getByTestId("program-row-effect:acme/mark-done"));
    fireEvent.click(view.getByTestId("effect-backfill-effect:acme/mark-done"));
    const ask = await waitFor(() => view.getByTestId("effect-backfill-ask-effect:acme/mark-done"));
    expect(ask.textContent).toContain("nothing to backfill");
    const close = view.getByTestId("effect-backfill-cancel-effect:acme/mark-done");
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    await waitFor(() => expect(view.queryByTestId("effect-backfill-ask-effect:acme/mark-done")).toBeNull());
  } finally {
    plannedCount = 3;
  }
});


// tsk1009: a bundled program's "Read the script" says whether its source is
// shown, for a screen reader as for the eye.
test("a bundled program's script toggle says whether it's shown", async () => {
  answer = () => new Promise(() => {});
  const view = render(<DataSection />);
  const key = "effect:oxplow-bundled/verify-unchecked";
  const toggle = await waitFor(() => view.getByTestId(`program-source-toggle-${key}`));
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(toggle);
  await waitFor(() => expect(view.getByTestId(`program-source-${key}`).textContent).toContain("transform"));
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  expect(toggle.getAttribute("aria-controls")).toBe(view.getByTestId(`program-source-${key}`).id);
  fireEvent.click(toggle);
  await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("false"));
});

// tsk1065: the list shows without waiting on counts, and a model too big
// to count in time costs only its own cell.
test("a model that can't be counted in time leaves the rest of the list", async () => {
  entities = [
    { name: "v_file_metric", owner: "core", kind: "sql", description: "" },
    { name: "v_task", owner: "core", kind: "sql", description: "" },
  ];
  counts = { v_task: 3 };
  const view = render(<DataSection />);
  const cell = (name: string) => view.getByTestId(`data-entity-${name}`).querySelectorAll("td")[2]!;
  await waitFor(() => expect(cell("v_task").textContent).toBe("3"));
  expect(cell("v_file_metric").textContent).toBe("—");
  expect(cell("v_file_metric").getAttribute("title")).toContain("timed out");
});
