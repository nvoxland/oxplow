import { expect, test } from "bun:test";

import type { ListedConversation } from "../tauri-bridge/generated/bindings.js";
import { conversationRows } from "./conversationRows.js";

const listed = (id: string, over: Partial<ListedConversation["conversation"]> = {}, rest: Partial<ListedConversation> = {}): ListedConversation => ({
  conversation: { id, title: null, first_prompt: null, started_at: null, updated_at: null, size_bytes: null, parent: null, ...over },
  current: false,
  open_in: null,
  ...rest,
});

// Each row names its conversation by its title, else the first thing the
// person said, else its id; the one the session is on and one open in
// another session say so, and only a free one other than the current can
// be resumed (any can be forked).
test("rows name each conversation and say which can be taken", () => {
  const rows = conversationRows(
    [
      listed("c1", { title: "Login fix", size_bytes: 2048, updated_at: "2026-10-15T10:00:00.000000Z" }, { current: true }),
      listed("c2", { first_prompt: "Speed up the build" }),
      listed("c3", {}, { open_in: "agent_session:ses4" }),
    ],
    "resume",
  );
  expect(rows.map((r) => [r.id, r.label, r.note, r.takeable])).toEqual([
    ["c1", "Login fix", "this session's", false],
    ["c2", "Speed up the build", null, true],
    ["c3", "c3", "open in another session", false],
  ]);
  expect(rows[0]!.size).toBe("2 KB");
  expect(conversationRows([listed("c1", {}, { current: true })], "fork")[0]!.takeable).toBe(true);
});
