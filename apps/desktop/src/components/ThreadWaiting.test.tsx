import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";

// What waits for the person above an agent's terminal: its thread's
// pending proposals (P9.A3).

const realApi = { ...(await import("../api.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
const commands: Array<[string, unknown]> = [];
let answerRows: (string | null)[][] = [];
/** `v_command_proposal` rows: id, thread_id, decision. */
let proposalRows: Array<{ id: number; thread: number; decision: string }> = [];
const decided: Array<[number, boolean]> = [];
const PROPOSAL_COLUMNS = ["id", "uri", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run", "decision", "decided_at"];

const lens = (title: string): Lens => ({
  id: "answer/1",
  extension: "",
  slug: "",
  title,
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
});

mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string, params: unknown[] = []) =>
    sql.includes("v_command_proposal")
      ? {
          columns: PROPOSAL_COLUMNS,
          // The view's own filter: one thread's proposals.
          rows: proposalRows
            .filter((p) => p.thread === params[0])
            .map((p) => [
              p.id,
              `proposal:${p.id}`,
              "2026-10-03T00:00:00Z",
              "oxplow.work_item.delete",
              '{"uri":"oxplow:oxplow/work_item?id=tsk1&provider=oxplow"}',
              "agent",
              `thr${p.thread}`,
              p.thread,
              "k",
              '{"command":"oxplow.work_item.delete","summary":"Delete a work item","input":{},"destructive":false}',
              null,
              p.decision,
              null,
            ]),
          truncated: false,
          reads: { models: ["v_command_proposal"], tables: [], measures: [] },
          freshness: {},
        }
      : {
          columns: ["uri", "title", "lens", "kept_lens"],
          rows: answerRows,
          truncated: false,
          reads: { models: ["v_thread_answer"], tables: [], measures: [] },
          freshness: {},
        },
  // A result view reads the extensions (row commands, badges).
  listExtensions: async () => [],
  decideProposal: async (id: number, approve: boolean) => {
    decided.push([id, approve]);
    return null;
  },
  runAnswer: async (answer: string): Promise<LensRun> => ({
    lens: lens(answer),
    params: {},
    result: {
      columns: ["path"],
      rows: [[`${answer}.rs`]],
      truncated: false,
      reads: { models: [], tables: [], measures: [] },
    },
  } as unknown as LensRun),
  runCommand: async (name: string, input: unknown) => {
    commands.push([name, input]);
    return { result: { lens: "my-lenses/churn" }, audit_id: 1, event_id: null, undo: null };
  },
}));

const { ThreadWaiting } = await import("./ThreadWaiting.js");

beforeEach(() => {
  decided.length = 0;
  proposalRows = [];
});
afterEach(cleanup);

test("a thread's pending proposals wait; Approve decides as the person", async () => {
  proposalRows = [
    { id: 1, thread: 1, decision: "pending" },
    { id: 2, thread: 2, decision: "pending" },
    { id: 3, thread: 1, decision: "declined" },
  ];
  const view = render(<ThreadWaiting threadId="thr1" />);
  const group = await waitFor(() => view.getByTestId("thread-proposals"));
  expect(group.textContent).toContain("Waiting for you");
  expect(view.getByTestId("proposal-1").textContent).toContain("Delete a work item");
  // Another thread's, and one already decided, aren't waiting here.
  expect(view.queryByTestId("proposal-2")).toBeNull();
  expect(view.queryByTestId("proposal-3")).toBeNull();
  fireEvent.click(view.getByTestId("proposal-approve-1"));
  await waitFor(() => expect(decided).toEqual([[1, true]]));
});

test("nothing waiting, nothing shown", async () => {
  proposalRows = [{ id: 2, thread: 2, decision: "pending" }];
  const view = render(<ThreadWaiting threadId="thr1" />);
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("thread-proposals")).toBeNull();
});
