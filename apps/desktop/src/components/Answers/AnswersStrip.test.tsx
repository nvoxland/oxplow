import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun } from "../../tauri-bridge/generated/bindings.js";

// The Answers strip (P6.C2): an agent's `show_lens` answers, newest first,
// each live, with Keep This running `lens.keep`.

const realApi = await import("../../api.js");
const commands: Array<[string, unknown]> = [];
let answerRows: (string | null)[][] = [];
/** `v_command_proposal` rows: id, thread_id, decision. */
let proposalRows: Array<{ id: number; thread: number; decision: string }> = [];
const decided: Array<[number, boolean]> = [];
const PROPOSAL_COLUMNS = ["id", "ref", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run", "decision", "decided_at"];

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
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  path: "",
});

mock.module("../../api.js", () => ({
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
              "work_item.delete",
              '{"ref":"work_item:oxplow:tsk1"}',
              "agent",
              `thr${p.thread}`,
              p.thread,
              "k",
              '{"command":"work_item.delete","summary":"Delete a work item","input":{},"destructive":false}',
              null,
              p.decision,
              null,
            ]),
          truncated: false,
          reads: { models: ["v_command_proposal"], tables: [], measures: [] },
          freshness: {},
        }
      : {
          columns: ["ref", "title", "lens", "kept_lens"],
          rows: answerRows,
          truncated: false,
          reads: { models: ["v_thread_answer"], tables: [], measures: [] },
          freshness: {},
        },
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

const { AnswersStrip } = await import("./AnswersStrip.js");
const { ThreadAnswer } = await import("./ThreadAnswer.js");

beforeEach(() => {
  commands.length = 0;
  decided.length = 0;
  proposalRows = [];
  answerRows = [
    ["answer:2", "Churn", null, null],
    ["answer:1", "Open tasks", "review/waiting", "review/waiting"],
  ];
});
afterEach(cleanup);

test("the strip lists a thread's answers newest first, each rendered live", async () => {
  const view = render(<AnswersStrip threadId="thr1" onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getAllByTestId("thread-answer").length).toBe(2));
  const titles = view.getAllByTestId("thread-answer-title").map((e) => e.textContent);
  expect(titles).toEqual(["Churn", "Open tasks"]);
  await waitFor(() => expect(view.container.textContent).toContain("answer:2.rs"));
});

test("no answers, no strip", async () => {
  answerRows = [];
  const view = render(<AnswersStrip threadId="thr1" onOpenPage={() => {}} />);
  await waitFor(() => expect(view.queryByTestId("answers-strip")).toBeNull());
});

test("the strip collapses, and Escape inside it collapses it", async () => {
  const view = render(<AnswersStrip threadId="thr1" onOpenPage={() => {}} />);
  await waitFor(() => view.getByTestId("answers-strip"));
  fireEvent.keyDown(view.getByTestId("answers-strip"), { key: "Escape" });
  expect(view.queryAllByTestId("thread-answer").length).toBe(0);
  fireEvent.click(view.getByTestId("answers-strip-toggle"));
  await waitFor(() => expect(view.getAllByTestId("thread-answer").length).toBe(2));
});

test("Keep This asks for a slug inline; Enter keeps it with lens.keep, Escape cancels", async () => {
  const answer = { ref: "answer:2", title: "Churn", lens: null, keptLens: null };
  const view = render(<ThreadAnswer answer={answer} onOpenPage={() => {}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("thread-answer-keep")));
  const input = view.getByTestId("thread-answer-slug") as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  fireEvent.keyDown(input, { key: "Escape" });
  expect(view.queryByTestId("thread-answer-slug")).toBeNull();
  expect(commands).toEqual([]);

  fireEvent.click(view.getByTestId("thread-answer-keep"));
  fireEvent.change(view.getByTestId("thread-answer-slug"), { target: { value: "churn" } });
  fireEvent.submit(view.getByTestId("thread-answer-slug"));
  await waitFor(() => expect(commands).toEqual([["lens.keep", { answer: "answer:2", slug: "churn" }]]));
  // Kept: a link to the lens replaces Keep This.
  await waitFor(() => expect(view.getByTestId("thread-answer-lens").textContent).toContain("my-lenses/churn"));
  expect(view.queryByTestId("thread-answer-keep")).toBeNull();
});

test("a kept answer links to its lens", async () => {
  const opened: string[] = [];
  const answer = { ref: "answer:1", title: "Open tasks", lens: "review/waiting", keptLens: "review/waiting" };
  const view = render(<ThreadAnswer answer={answer} onOpenPage={(r) => opened.push(r.id)} />);
  fireEvent.click(await waitFor(() => view.getByTestId("thread-answer-lens")));
  expect(opened).toEqual(["lens:review/waiting"]);
});

// P9.A3: what the agent asked for waits where the conversation is.
test("a thread's pending proposals wait above its answers; Approve decides as the person", async () => {
  answerRows = [];
  proposalRows = [
    { id: 1, thread: 1, decision: "pending" },
    { id: 2, thread: 2, decision: "pending" },
    { id: 3, thread: 1, decision: "declined" },
  ];
  const view = render(<AnswersStrip threadId="thr1" onOpenPage={() => {}} />);
  const group = await waitFor(() => view.getByTestId("thread-proposals"));
  expect(group.textContent).toContain("Waiting for you");
  expect(view.getByTestId("proposal-1").textContent).toContain("Delete a work item");
  // Another thread's, and one already decided, aren't waiting here.
  expect(view.queryByTestId("proposal-2")).toBeNull();
  expect(view.queryByTestId("proposal-3")).toBeNull();
  fireEvent.click(view.getByTestId("proposal-approve-1"));
  await waitFor(() => expect(decided).toEqual([[1, true]]));
});

test("no answers and nothing waiting, no strip", async () => {
  answerRows = [];
  proposalRows = [{ id: 2, thread: 2, decision: "pending" }];
  const view = render(<AnswersStrip threadId="thr1" onOpenPage={() => {}} />);
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("answers-strip")).toBeNull();
});

