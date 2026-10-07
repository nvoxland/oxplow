import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Proposal } from "../../proposals.js";
import { ProposalCard } from "./ProposalCard.js";
import { SettingRow } from "../../pages/SettingsPage.js";
import type { EffectiveSetting } from "../../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const proposal: Proposal = {
  id: 7,
  ref: "proposal:7",
  createdAt: "2026-10-01T00:00:00Z",
  command: "oxplow.config.set",
  input: { key: "agentPromptAppend", value: "be brief" },
  actorKind: "agent",
  actorId: "thr3",
  threadId: 3,
  threadTitle: "Fix the cart",
  key: "config:agentPromptAppend",
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
  expect(card.textContent).toContain("The agent in “Fix the cart”");
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
        throw new Error("proposal:7 is already declined");
      }}
    />,
  );
  fireEvent.click(view.getByTestId("proposal-approve-7"));
  await waitFor(() => expect(view.getByTestId("proposal-error-7").textContent).toContain("already declined"));
});

// A Settings row with the agent's pending change shows it with the same two
// buttons.
test("a setting's row shows its pending proposal", async () => {
  const setting = {
    key: "agentPromptAppend",
    value: null,
    origin: "default",
    extension: null,
    humanOnly: true,
    doc: "Appended to every agent's prompt.",
  } as unknown as EffectiveSetting;
  const decided: boolean[] = [];
  const view = render(
    <SettingRow
      setting={setting}
      proposal={proposal}
      onDecide={async (_p, approve) => {
        decided.push(approve);
      }}
    />,
  );
  expect(view.getByTestId("setting-agentPromptAppend").textContent).toContain("be brief");
  fireEvent.click(view.getByTestId("proposal-decline-7"));
  await waitFor(() => expect(decided).toEqual([false]));
  cleanup();
  const none = render(<SettingRow setting={setting} proposal={undefined} onDecide={async () => {}} />);
  expect(none.queryByTestId("proposal-7")).toBeNull();
});
