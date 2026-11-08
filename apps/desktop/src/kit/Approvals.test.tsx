import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { sourceOf, type Proposal } from "../proposals.js";
import { createFailureStore, createInlineClaims } from "../commands/actionFailures.js";
import { createRunner } from "../commands/runForPerson.js";
import { RunnerContext } from "../commands/RunnerContext.js";
import { narrow, ProposalCard, ProposalList } from "./Approvals.js";

afterEach(cleanup);

const proposal: Proposal = {
  id: 7,
  uri: "oxplow:oxplow/proposal?id=7",
  createdAt: "2026-10-01T00:00:00Z",
  command: "oxplow.config.set",
  input: { key: "agentPromptAppend", value: "be brief" },
  actorKind: "agent",
  actorId: "thr3",
  threadId: 3,
  threadTitle: "Fix the cart",
  key: "oxplow:oxplow/config?key=agentPromptAppend",
  preview: { command: "oxplow.config.set", summary: "Set one key.", input: {}, destructive: false },
  dryRun: { key: "agentPromptAppend", before: null, after: "be brief", changed: true },
};

// P6b.A4: approving is the confirmation — no modal; each button decides.
test("a card shows what the agent proposed and decides it", async () => {
  const decided: [number, boolean][] = [];
  const view = render(
    <ProposalCard
      proposal={proposal}
      onDecide={async (p, approve) => {
        decided.push([p.id, approve]);
      }}
    />,
  );
  const card = view.getByTestId("proposal-7");
  expect(card.textContent).toContain("Set agentPromptAppend");
  // Who asked is the source chip every alert uses: the agent's thread.
  const chip = view.getByTestId("source-chip");
  expect([chip.getAttribute("data-kind"), chip.textContent]).toEqual(["agent", "Fix the cart"]);
  expect(card.textContent).not.toContain("proposed this");
  expect(view.getByTestId("proposal-change-7").textContent).toContain("be brief");
  fireEvent.click(view.getByTestId("proposal-approve-7"));
  await waitFor(() => expect(decided).toEqual([[7, true]]));
  fireEvent.click(view.getByTestId("proposal-decline-7"));
  await waitFor(() => expect(decided).toEqual([[7, true], [7, false]]));
});

// A destructive proposal asks inline (InlineConfirm) before Approve runs it.
test("approving a destructive proposal asks first", async () => {
  const decided: [number, boolean][] = [];
  const view = render(
    <ProposalCard
      proposal={{ ...proposal, preview: { ...proposal.preview, destructive: true } }}
      onDecide={async (p, approve) => {
        decided.push([p.id, approve]);
      }}
    />,
  );
  fireEvent.click(view.getByTestId("proposal-approve-7-trigger"));
  await new Promise((r) => setTimeout(r, 20));
  expect(decided).toEqual([]);
  fireEvent.click(view.getByTestId("proposal-approve-7-confirm"));
  await waitFor(() => expect(decided).toEqual([[7, true]]));
});

// A decision that fails says so next to the buttons.
test("a failed decision shows its error on the card", async () => {
  const view = render(
    <ProposalCard
      proposal={proposal}
      onDecide={async () => {
        throw new Error("proposal 7 is already declined");
      }}
    />,
  );
  fireEvent.click(view.getByTestId("proposal-approve-7"));
  await waitFor(() => expect(view.getByTestId("proposal-error-7").textContent).toContain("already declined"));
});

// The list re-reads as the run touches the proposals: the card is drawn
// again, and the failure is still on it (it is kept by the proposal, not by
// the card). It is recorded once, as the person's failed operation.
test("a failed approval stays on the card when the list draws it again, recorded once", async () => {
  const reports: { label: string; command?: string }[] = [];
  const runner = createRunner({
    report: async (e) => (reports.push({ label: e.label, command: e.command }), "ev1"),
    failures: createFailureStore(),
    claims: createInlineClaims(),
    now: () => 1,
  });
  const draw = () =>
    render(
      <RunnerContext.Provider value={runner}>
        <ProposalList
          proposals={[proposal]}
          onDecide={async () => {
            throw new Error("the schema of `oxplow_bundled.accepted@1` changed");
          }}
        />
      </RunnerContext.Provider>,
    );
  const first = draw();
  fireEvent.click(first.getByTestId("proposal-approve-7"));
  await waitFor(() => expect(first.getByTestId("proposal-error-7").textContent).toContain("changed"));
  cleanup();
  const again = draw();
  expect(again.getByTestId("proposal-error-7").textContent).toContain("changed");
  expect(reports).toEqual([{ label: "Approve: Set agentPromptAppend", command: "oxplow.config.set" }]);
});

// An effect's step that waits for a person is the extension's, not an agent's.
test("who asked: the agent's thread, a page acting for it, or an extension's effect", () => {
  expect(sourceOf(proposal)).toEqual({ kind: "agent", label: "Fix the cart", uri: "oxplow:oxplow/page?page=thread-work&thread=thr3" });
  expect(sourceOf({ ...proposal, actorKind: "effect", actorId: "e2e/delete-flagged", threadId: null, threadTitle: null })).toEqual({
    kind: "extension",
    label: "e2e",
    uri: "oxplow:oxplow/extension?name=e2e",
  });
  expect(sourceOf({ ...proposal, actorKind: "page", actorId: "acme/prs" })).toEqual({ kind: "page", label: "acme/prs" });
});

test("a list narrows to a thread and to a setting, and draws nothing when empty", () => {
  const other: Proposal = { ...proposal, id: 8, threadId: 9, key: "oxplow:oxplow/config?key=other" };
  const all = [proposal, other];
  expect(narrow(all).map((p) => p.id)).toEqual([7, 8]);
  expect(narrow(all, "oxplow:oxplow/thread?id=thr3").map((p) => p.id)).toEqual([7]);
  expect(narrow(all, "thr9").map((p) => p.id)).toEqual([8]);
  expect(narrow(all, null, "other").map((p) => p.id)).toEqual([8]);
  expect(narrow(all, "thr3", "other")).toEqual([]);

  const empty = render(<ProposalList proposals={[]} />);
  expect(empty.queryByTestId("approvals")).toBeNull();
  cleanup();
  const decided: [number, boolean][] = [];
  const view = render(
    <ProposalList
      proposals={all}
      onDecide={async (p, approve) => {
        decided.push([p.id, approve]);
      }}
    />,
  );
  expect(view.getAllByTestId(/^proposal-\d+$/).length).toBe(2);
  fireEvent.click(view.getByTestId("proposal-decline-8"));
  return waitFor(() => expect(decided).toEqual([[8, false]]));
});
