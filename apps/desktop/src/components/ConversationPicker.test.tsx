import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// The conversation picker lists a session's saved conversations; ↓ and
// Enter resume the one chosen, a taken one can't be picked, and Escape
// cancels.

const resumed: [string, string][] = [];
const realApi = { ...(await import("../api.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
mock.module("../api.js", () => ({
  ...realApi,
  listNativeConversations: async () => [
    { conversation: { id: "c1", title: "Current one", first_prompt: null, started_at: null, updated_at: null, size_bytes: null, parent: null }, current: true, open_in: null },
    { conversation: { id: "c2", title: null, first_prompt: "Speed up the build", started_at: null, updated_at: null, size_bytes: null, parent: null }, current: false, open_in: null },
    { conversation: { id: "c3", title: "Taken", first_prompt: null, started_at: null, updated_at: null, size_bytes: null, parent: null }, current: false, open_in: "agent_session:ses4" },
  ],
  resumeConversation: async (session: string, conversation: string) => {
    resumed.push([session, conversation]);
  },
}));
const { ConversationPicker } = await import("./ConversationPicker.js");

afterEach(() => {
  cleanup();
  resumed.length = 0;
});

test("the keyboard resumes the chosen conversation", async () => {
  let chosen = 0;
  const view = render(<ConversationPicker sessionId="ses1" mode="resume" onChosen={() => chosen++} onCancel={() => {}} />);
  const row = await view.findByTestId("conversation-row-c2");
  expect(row.textContent).toContain("Speed up the build");
  expect((view.getByTestId("conversation-row-c3") as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByTestId("conversation-row-c1") as HTMLButtonElement).disabled).toBe(true);
  // The cursor starts on the first takeable row: Enter takes it.
  fireEvent.keyDown(view.getByRole("listbox"), { key: "Enter" });
  await waitFor(() => expect(resumed).toEqual([["ses1", "c2"]]));
  expect(chosen).toBe(1);
});

test("Escape cancels", async () => {
  let cancelled = 0;
  const view = render(<ConversationPicker sessionId="ses1" mode="resume" onChosen={() => {}} onCancel={() => cancelled++} />);
  await view.findByTestId("conversation-row-c2");
  fireEvent.keyDown(window, { key: "Escape" });
  expect(cancelled).toBe(1);
  expect(resumed).toEqual([]);
});
