import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { AcpEvent, AcpSnapshot, Thread } from "../../api.js";

// The no-automation contract at the component level: "Put in input" only
// fills the draft; only Enter (or Send) calls acpPrompt, exactly once.

const realApi = await import("../../api.js");
const prompts: Array<[string, string]> = [];
const responses: Array<[string, string, string | null]> = [];
let listener: ((e: AcpEvent) => void) | null = null;
let snapshot: AcpSnapshot;
/** `v_command_proposal`'s answer: the thread's proposals' decisions by id. */
let proposalDecisions: Record<number, string> = {};

mock.module("../../api.js", () => ({
  ...realApi,
  acpPrompt: async (threadId: string, text: string) => {
    prompts.push([threadId, text]);
  },
  acpTranscript: async () => snapshot,
  acpOpenSession: async () => snapshot,
  acpRespondPermission: async (t: string, r: string, o: string | null) => {
    responses.push([t, r, o]);
  },
  acpCancel: async () => {},
  acpDismissDirective: async () => {},
  subscribeAcpEvents: (l: (e: AcpEvent) => void) => {
    listener = l;
    return () => {
      listener = null;
    };
  },
  onRemoteReconnect: () => () => {},
  querySql: async (sql: string) =>
    sql.includes("v_command_proposal")
      ? {
          columns: ["id", "ref", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run", "decision", "decided_at"],
          rows: Object.entries(proposalDecisions).map(([id, decision]) => [
            Number(id),
            `proposal:${id}`,
            "2026-10-03T00:00:00Z",
            "work_item.delete",
            "{}",
            "agent",
            "thr1",
            1,
            "k",
            '{"command":"work_item.delete","summary":"Delete a work item","input":{},"destructive":false}',
            null,
            decision,
            null,
          ]),
          truncated: false,
          reads: { models: ["v_command_proposal"], tables: [], measures: [] },
          freshness: {},
        }
      : {
          columns: ["ref", "title", "lens", "kept_lens"],
          rows: [["answer:7", "Churn", null, null]],
          truncated: false,
          reads: { models: ["v_thread_answer"], tables: [], measures: [] },
          freshness: {},
        },
  runAnswer: async () => ({
    lens: { id: "answer/7", title: "Churn", viz: "table", columns: [], actions: [], children: [], params: [] },
    params: {},
    result: { columns: ["path"], rows: [["hot.rs"]], truncated: false, reads: { models: [], tables: [], measures: [] } },
  }),
}));

const { AcpAgentView } = await import("./AcpAgentView.js");

const thread = { id: "thr1", acp_agent: "fake", agent: "acp" } as unknown as Thread;

beforeEach(() => {
  prompts.length = 0;
  responses.length = 0;
  proposalDecisions = {};
  snapshot = {
    agent: "fake",
    status: "idle",
    directive: "Close the task before stopping.",
    usage: null,
    headSeq: 1,
    items: [{ id: 1, seq: 1, type: "agent", text: "done" }],
    stderrTail: [],
  };
});

afterEach(cleanup);

describe("AcpAgentView", () => {
  // Starting is a loading state, said plainly; only an empty transcript
  // is an EmptyState (what would be here, and prompts to start it).
  test("the starting state is plain text; an empty transcript is an EmptyState", async () => {
    snapshot = { ...snapshot, status: "starting", items: [] };
    const starting = render(<AcpAgentView thread={thread} visible={true} />);
    await waitFor(() => expect(starting.getByTestId("acp-transcript").textContent).toContain("Starting the agent"));
    expect(starting.container.querySelector("[data-empty-state]")).toBeNull();
    starting.unmount();

    snapshot = { ...snapshot, status: "idle", items: [] };
    const empty = render(<AcpAgentView thread={thread} visible={true} />);
    await waitFor(() => expect(empty.container.querySelector("[data-empty-state]")).not.toBeNull());
  });

  test("Put in input fills the draft and sends nothing; Enter sends once", async () => {
    const view = render(<AcpAgentView thread={thread} visible={true} />);
    await waitFor(() => view.getByTestId("acp-directive"));

    fireEvent.click(view.getByTestId("acp-directive-put"));
    const input = view.getByTestId("acp-prompt-input") as HTMLTextAreaElement;
    expect(input.value).toBe("Close the task before stopping.");
    await new Promise((r) => setTimeout(r, 20));
    expect(prompts).toEqual([]);

    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(prompts).toEqual([]);
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(prompts).toEqual([["thr1", "Close the task before stopping."]]));
    await waitFor(() => expect(input.value).toBe(""));
  });

  test("while a turn runs Enter sends nothing and Escape stops", async () => {
    snapshot = { ...snapshot, status: "running", directive: null };
    const view = render(<AcpAgentView thread={thread} visible={true} />);
    await waitFor(() => view.getByTestId("acp-prompt-stop"));
    const input = view.getByTestId("acp-prompt-input") as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "again" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await new Promise((r) => setTimeout(r, 20));
    expect(prompts).toEqual([]);
  });

  test("live events render and a permission card answers through the API", async () => {
    snapshot = { ...snapshot, directive: null };
    const view = render(<AcpAgentView thread={thread} visible={true} />);
    await waitFor(() => view.getByTestId("acp-item-1"));
    act(() => {
      listener?.({
        threadId: "thr1",
        type: "item",
        item: {
          id: 2,
          seq: 2,
          type: "permission",
          requestId: "perm-1",
          toolCallId: "t1",
          title: "Edit a.rs",
          options: [
            { id: "allow", name: "Allow", kind: "allow_once" },
            { id: "reject", name: "Reject", kind: "reject_once" },
          ],
          answer: null,
        },
      });
      listener?.({ threadId: "other", type: "status", status: "stopped" });
    });
    fireEvent.click(await waitFor(() => view.getByTestId("acp-permission-perm-1-allow")));
    await waitFor(() => expect(responses).toEqual([["thr1", "perm-1", "allow"]]));
    // Another thread's event is ignored.
    expect(view.getByTestId("acp-status").textContent).toBe("Ready");
    expect(prompts).toEqual([]);
  });

  // P9.A3: a run that became a proposal waits under its tool call, and a
  // decided one stays in the transcript as what happened.
  test("a proposal waits under the call that made it, then reads as decided", async () => {
    const proposing = (id: number, toolId: string, text: string[], status = "completed") => ({
      id,
      seq: id,
      type: "tool" as const,
      call: {
        id: toolId,
        title: "mcp__oxplow__run_command",
        name: "mcp__oxplow__run_command",
        kind: "other",
        status,
        locations: [],
        rawInput: null,
        rawOutput: null,
        diffs: [],
        text,
      },
    });
    snapshot = {
      ...snapshot,
      directive: null,
      headSeq: 3,
      items: [
        proposing(1, "t1", [JSON.stringify({ kind: "proposed", proposal: "proposal:7", message: "waits" })]),
        proposing(2, "t2", [JSON.stringify({ kind: "proposed", proposal: "proposal:8", message: "waits" })]),
        // Another thread's proposal, named in this transcript: no card.
        proposing(3, "t3", [JSON.stringify({ kind: "proposed", proposal: "proposal:99", message: "waits" })]),
      ] as never,
    };
    proposalDecisions = { 7: "pending", 8: "approved" };
    const view = render(<AcpAgentView thread={thread} visible={true} />);
    const card = await waitFor(() => view.getByTestId("proposal-7"));
    expect(card.textContent).toContain("Delete a work item");
    expect(view.getByTestId("proposal-approve-7")).toBeTruthy();
    const done = await waitFor(() => view.getByTestId("acp-proposal-8-decided"));
    expect(done.textContent).toContain("Approved");
    expect(view.queryByTestId("proposal-approve-8")).toBeNull();
    expect(view.queryByTestId("proposal-99")).toBeNull();
    expect(view.queryByTestId("acp-proposal-99-decided")).toBeNull();
  });

  test("an agent's show_lens answer renders inline where the call is (P6.C2)", async () => {
    const out = JSON.stringify({ answer: "answer:7", title: "Churn", text: "| path |" });
    snapshot = {
      ...snapshot,
      directive: null,
      headSeq: 2,
      items: [
        { id: 1, seq: 1, type: "agent", text: "Here:" },
        {
          id: 2,
          seq: 2,
          type: "tool",
          call: {
            id: "t9",
            title: "mcp__oxplow__show_lens",
            name: "mcp__oxplow__show_lens",
            kind: "other",
            status: "completed",
            locations: [],
            rawInput: null,
            rawOutput: null,
            diffs: [],
            text: [out],
          },
        },
      ],
    };
    const view = render(<AcpAgentView thread={thread} visible={true} />);
    const item = await waitFor(() => view.getByTestId("acp-item-2"));
    await waitFor(() => expect(item.querySelector('[data-testid="thread-answer"]')).not.toBeNull());
    await waitFor(() => expect(item.textContent).toContain("hot.rs"));
    expect(item.querySelector('[data-testid="thread-answer-keep"]')).not.toBeNull();
  });
});
