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
}));

const { AcpAgentView } = await import("./AcpAgentView.js");

const thread = { id: "thr1", acp_agent: "fake", agent: "acp" } as unknown as Thread;

beforeEach(() => {
  prompts.length = 0;
  responses.length = 0;
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
});
