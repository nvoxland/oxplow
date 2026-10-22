import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { keyOf, proposalUri } from "../uri/uri.js";
import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";

// `viz: approvals` draws the kit's Approvals: it reads the pending
// proposals itself (the lens's rows are ignored), its `thread` and
// `setting` params narrow them, and Approve / Decline decide through the
// person-only path.

const realApi = await import("../api.js");
const decided: [number, boolean][] = [];
const columns = ["id", "uri", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "thread_title", "key", "preview", "dry_run"];
const row = (id: number, threadId: number, key: string) => [
  id,
  proposalUri(id),
  "2026-10-01T00:00:00Z",
  "oxplow.config.set",
  JSON.stringify({ key: keyOf(key), value: "x" }),
  "agent",
  `thr${threadId}`,
  threadId,
  `Thread ${threadId}`,
  key,
  JSON.stringify({ command: "oxplow.config.set", summary: "Set one key.", input: {}, destructive: false }),
  null,
];
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    sql.includes("v_command_proposal")
      ? {
          columns,
          rows: [row(7, 3, "oxplow:oxplow/config?key=agentPromptAppend"), row(8, 9, "oxplow:oxplow/config?key=other")],
          truncated: false,
          reads: { models: [], tables: [], measures: [] },
          freshness: {},
        }
      : { columns: ["n"], rows: [[1]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
  decideProposal: async (id: number, approve: boolean) => {
    decided.push([id, approve]);
    return null;
  },
}));

const { LensResultView } = await import("./LensResultView.js");

afterEach(() => {
  cleanup();
  decided.length = 0;
});

const lens = {
  id: "x/approvals",
  extension: "x",
  slug: "approvals",
  title: "Approvals",
  description: "",
  query: "SELECT 1",
  viz: "approvals",
  params: [],
  columns: [],
  empty: null,
  parts: [],
  actions: [],
  alert: null,
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
} as unknown as Lens;
const runWith = (params: Record<string, string>): LensRun =>
  ({ lens, params, result: { columns: ["1"], rows: [[1]], truncated: false } }) as unknown as LensRun;

test("an approvals lens draws every pending proposal and decides through the person's path", async () => {
  const view = render(<LensResultView run={runWith({})} onOpenPage={() => {}} />);
  await waitFor(() => view.getByTestId("proposal-7"));
  view.getByTestId("proposal-8");
  fireEvent.click(view.getByTestId("proposal-approve-7"));
  await waitFor(() => expect(decided).toEqual([[7, true]]));
});

test("its thread and setting params narrow the list", async () => {
  const byThread = render(<LensResultView run={runWith({ thread: "oxplow:oxplow/thread?id=thr9" })} onOpenPage={() => {}} />);
  await waitFor(() => byThread.getByTestId("proposal-8"));
  expect(byThread.queryByTestId("proposal-7")).toBeNull();
  cleanup();
  const bySetting = render(<LensResultView run={runWith({ setting: "agentPromptAppend" })} onOpenPage={() => {}} />);
  await waitFor(() => bySetting.getByTestId("proposal-7"));
  expect(bySetting.queryByTestId("proposal-8")).toBeNull();
});
