import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

const realApi = await import("../api.js");
const decided: string[] = [];
/** What Settings → Data lists as extension collectors. */
let collectorListings: unknown[] = [];
const ran: Array<[string, unknown, boolean]> = [];
/** What `oxplow.effect.retry` answers. */
let retried: unknown = { effect: "acme/mark-done", event: "oxplow:oxplow/event?id=e1", attempt: 2, outcome: "ok" };
const letter = { id: 7, consumer: "change.analyze", event_seq: 41, error: "boom", attempts: 2, first_failed_at: "t", last_failed_at: "t" };
/** The models Settings → Data lists, and what counting each answers (none: it times out). */
let entities: Array<{ name: string; owner: string; kind: string; description: string }> = [];
let counts: Record<string, number> = {};
/** What `oxplow.effect.backfill_plan` counts. */
let plannedCount = 3;
/** The event listeners the section (and its `useRerunOnChange`) registered. */
const listeners = new Set<(event: Record<string, unknown>) => void>();
const emit = (event: Record<string, unknown>) => listeners.forEach((l) => l(event));
/** How often each list was read, and which models were counted. */
const reads = { entities: 0, collectors: 0, effects: 0, counted: [] as string[] };
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) => {
    const counted = /^SELECT count\(\*\) FROM "(\w+)"$/.exec(sql)?.[1];
    if (counted) {
      reads.counted.push(counted);
      const n = counts[counted];
      if (n === undefined) throw new Error("query_sql: timed out after 5s");
      return { columns: ["count(*)"], rows: [[n]], truncated: false, reads: { models: [counted], tables: [], measures: [] }, freshness: {} };
    }
    if (sql.includes("FROM v_contribution")) {
      reads.effects++;
      return { columns: ["effect"], rows: [["acme/mark-done"]], truncated: false, reads: { models: ["v_contribution"], tables: [], measures: [] }, freshness: {} };
    }
    return sql.includes("FROM v_effect_run")
      ? {
          columns: ["effect", "event_id", "event_seq", "attempt", "reason", "event_type"],
          rows: [["acme/mark-done", "e1", 52, 1, "interrupted: a step outside oxplow may have run", "work_item.state_changed"]],
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
      name === "oxplow.effect.backfill_plan"
        ? { effect: "acme/mark-done", planned: plannedCount, from_seq: 4, to_seq: 9, batch: 200 }
        : name === "oxplow.effect.backfill"
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
  listDataEntities: async () => {
    reads.entities++;
    return entities;
  },
  listCollectors: async () => {
    reads.collectors++;
    return collectorListings;
  },
  subscribeOxplowEvents: (handler: (event: Record<string, unknown>) => void) => {
    listeners.add(handler);
    return () => listeners.delete(handler);
  },
}));
const { DataSection } = await import("./DataSection.js");

afterEach(() => {
  listeners.clear();
  Object.assign(reads, { entities: 0, collectors: 0, effects: 0, counted: [] });
  entities = [];
  counts = {};
  decided.length = 0;
  collectorListings = [];
  ran.length = 0;
  cleanup();
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
// since a step outside oxplow may already have run — runs `oxplow.effect.retry`
// as the person, confirmed.
test("Delivery lists a failed reaction and retries it only once confirmed", async () => {
  const view = render(<DataSection />);
  const row = await waitFor(() => view.getByTestId("reaction-row-acme/mark-done-e1"));
  expect(row.textContent).toContain("acme/mark-done failed on work_item.state_changed (event 52)");
  expect(row.textContent).toContain("interrupted: a step outside oxplow may have run");
  fireEvent.click(view.getByTestId("reaction-retry-acme/mark-done-e1-trigger"));
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("reaction-retry-acme/mark-done-e1-confirm"));
  await waitFor(() => expect(ran).toEqual([["oxplow.effect.retry", { effect: "acme/mark-done", event: "oxplow:oxplow/event?id=e1" }, true]]));
});

// P9.D5: an effect's row offers Backfill…: it says how many past
// events the effect never reacted to, and runs only on the second click.
test("Backfill… asks with the count and runs effect.backfill once confirmed", async () => {
  const view = render(<DataSection />);
  await waitFor(() => view.getByTestId("effect-row-acme/mark-done"));
  fireEvent.click(view.getByTestId("effect-backfill-acme/mark-done"));
  const ask = await waitFor(() => view.getByTestId("effect-backfill-ask-acme/mark-done"));
  expect(ask.textContent).toContain("never reacted to 3 matching events");
  expect(ran).toEqual([["oxplow.effect.backfill_plan", { effect: "acme/mark-done" }, false]]);
  expect(view.getByTestId("effect-backfill-run-acme/mark-done").textContent).toBe("Run on 3 events");
  fireEvent.click(view.getByTestId("effect-backfill-run-acme/mark-done"));
  // It runs on the range it showed (tsk849): what was logged since isn't in it.
  await waitFor(() => expect(ran[1]).toEqual(["oxplow.effect.backfill", { effect: "acme/mark-done", to_seq: 9 }, true]));
  await waitFor(() => expect(view.queryByTestId("effect-backfill-ask-acme/mark-done")).toBeNull());
});

// tsk850: with nothing to backfill only Close is there — focused, so
// Escape closes the note without a click into it first.
test("Escape closes Backfill's nothing-to-backfill note", async () => {
  plannedCount = 0;
  try {
    const view = render(<DataSection />);
    await waitFor(() => view.getByTestId("effect-row-acme/mark-done"));
    fireEvent.click(view.getByTestId("effect-backfill-acme/mark-done"));
    const ask = await waitFor(() => view.getByTestId("effect-backfill-ask-acme/mark-done"));
    expect(ask.textContent).toContain("nothing to backfill");
    const close = view.getByTestId("effect-backfill-cancel-acme/mark-done");
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    await waitFor(() => expect(view.queryByTestId("effect-backfill-ask-acme/mark-done")).toBeNull());
  } finally {
    plannedCount = 3;
  }
});


// tsk1065: the list shows without waiting on counts, and a model too big
// to count in time costs only its own cell.
test("a model that can't be counted in time leaves the rest of the list", async () => {
  entities = [
    { name: "v_file_metric", owner: "core", kind: "sql", description: "" },
    { name: "v_work_item", owner: "core", kind: "sql", description: "" },
  ];
  counts = { v_work_item: 3 };
  const view = render(<DataSection />);
  const cell = (name: string) => view.getByTestId(`data-entity-${name}`).querySelectorAll("td")[2]!;
  await waitFor(() => expect(cell("v_work_item").textContent).toBe("3"));
  expect(cell("v_file_metric").textContent).toBe("—");
  expect(cell("v_file_metric").getAttribute("title")).toContain("timed out");
});

// The project's own collectors (report collectors among them) are listed
// beside the extensions': last run, health, Sync Now, Enable Again.
const projectCollector = (over: Record<string, unknown> = {}) => ({
  owner: "project",
  spec: { id: "tests.cov", runtime: "jaq", entry: "oxplow:lcov", trigger: { kind: "onRun", run: "test" }, env: [], network: [], credentials: [], entities: [], facts: [] },
  run: { owner: "project", id: "tests.cov", status: "ok", lastRunAt: "2026-10-10T08:00:00Z", error: null, rowCounts: {}, cursor: null },
  networkEnforced: false,
  credentials: [],
  disabled: null,
  ...over,
});

test("a project report collector is listed with its trigger and last run, and syncs by hand", async () => {
  collectorListings = [projectCollector()];
  const view = render(<DataSection />);
  const row = await view.findByTestId("collector-row-project/tests.cov");
  expect(row.textContent).toContain("after a test run");
  expect(row.textContent).toContain("last run");
  expect((view.getByTestId("collector-run-project/tests.cov") as HTMLButtonElement).textContent).toBe("Sync Now");
});

test("a collector failures turned off says why and Enable Again turns it back on", async () => {
  collectorListings = [projectCollector({ disabled: "failed 3 times in a row: report not found" })];
  const view = render(<DataSection />);
  const row = await view.findByTestId("collector-row-project/tests.cov");
  expect(row.textContent).toContain("Off on this machine: failed 3 times in a row: report not found");
  fireEvent.click(view.getByTestId("collector-enable-project/tests.cov"));
  await waitFor(() =>
    expect(ran).toContainEqual(["oxplow.contribution.enable", { extension: "project", kind: "collector", contribution: "tests.cov" }, false]),
  );
});

// tsk1403: each list re-reads on the change that touches what it shows.
const model = (name: string) => ({ name, owner: "core", kind: "sql", description: "" });
const modelsChanged = (...models: string[]) => ({ kind: "modelsChanged", models });
const quiet = () => new Promise((r) => setTimeout(r, 250));

test("a collector run re-reads the collectors, not the models or their counts", async () => {
  entities = [model("v_a")];
  counts = { v_a: 1 };
  const view = render(<DataSection />);
  await waitFor(() => expect(view.getByTestId("data-entity-v_a")).toBeTruthy());
  const before = { ...reads, counted: [...reads.counted] };
  emit(modelsChanged("v_collector_run"));
  await waitFor(() => expect(reads.collectors).toBe(before.collectors + 1));
  await quiet();
  expect(reads.entities).toBe(before.entities);
  expect(reads.effects).toBe(before.effects);
  expect(reads.counted).toEqual(before.counted);
});

test("a change to one counted model recounts that model alone", async () => {
  entities = [model("v_a"), model("v_b")];
  counts = { v_a: 1, v_b: 2 };
  const view = render(<DataSection />);
  await waitFor(() => expect(view.getByTestId("data-entity-v_b")).toBeTruthy());
  reads.counted.length = 0;
  emit(modelsChanged("v_a"));
  await waitFor(() => expect(reads.counted).toEqual(["v_a"]));
  await quiet();
  expect(reads.counted).toEqual(["v_a"]);
  expect(reads.entities).toBe(1);
});

test("the catalog changing re-reads the models, collectors and effects", async () => {
  entities = [model("v_a")];
  counts = { v_a: 1 };
  const view = render(<DataSection />);
  await waitFor(() => expect(view.getByTestId("data-entity-v_a")).toBeTruthy());
  expect(reads.entities).toBe(1);
  expect(reads.effects).toBe(1);
  emit({ kind: "extensionsChanged" });
  await waitFor(() => expect(reads.entities).toBe(2));
  await waitFor(() => expect(reads.effects).toBe(2));
  expect(reads.collectors).toBeGreaterThan(1);
});
