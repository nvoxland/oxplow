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
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    sql.includes("FROM v_effect_run")
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
        },
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push([name, input, confirmed]);
    return { result: retried, audit_id: 1, event_id: null, inverse: null };
  },
  retryDeadLetter: async (id: number) => {
    decided.push(`retry ${id}`);
    return { ...letter, state: "retried" };
  },
  discardDeadLetter: async (id: number) => {
    decided.push(`discard ${id}`);
    return { ...letter, state: "discarded" };
  },
  listDataEntities: async () => [],
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
  ],
  providerDeclarationEffects: () => answer(),
  subscribeOxplowEvents: () => () => {},
}));
const { DataSection } = await import("./DataSection.js");

afterEach(() => {
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
