import { expect, test } from "bun:test";

import type { EarlierSession, EarlierSessions, NativeConversation, PastSession } from "../tauri-bridge/generated/bindings.js";
import { earlierSessionRows, filterEarlierRows, takeable } from "./earlierSessionRows.js";

const conversation = (over: Partial<NativeConversation> = {}): NativeConversation =>
  ({ id: "c1", title: null, first_prompt: null, started_at: null, updated_at: "2026-10-10T10:05:00Z", size_bytes: null, parent: null, ...over }) as NativeConversation;
const past = (over: Partial<PastSession> = {}): PastSession =>
  ({
    session: "agent_session:ses3",
    name: null,
    conversation: null,
    display_name: "claude 2026-10-10",
    thread: "thread:thr1",
    thread_title: "Main",
    here: true,
    open: false,
    closed_at: "2026-10-10T11:00:00Z",
    turns: 0,
    last_prompt: null,
    ...over,
  }) as PastSession;
const listing = (...rows: EarlierSession[]): EarlierSessions => ({ rows, unavailable: [] });

test("a row is labelled by its title, else a prompt, else its id", () => {
  const rows = earlierSessionRows(
    listing(
      { harness: "claude", session: null, conversation: conversation({ id: "a", title: "Fix login" }), action: "resume" },
      { harness: "claude", session: null, conversation: conversation({ id: "b", first_prompt: "why is it slow" }), action: "resume" },
      { harness: "claude", session: past({ last_prompt: "ship it" }), conversation: null, action: "reopen" },
      { harness: "claude", session: null, conversation: conversation({ id: "d" }), action: "resume" },
    ),
  );
  expect(rows.map((r) => r.label)).toEqual(["Fix login", "why is it slow", "ship it", "d"]);
});

test("a person's own name for a session labels it, the default name doesn't", () => {
  const [named, unnamed] = earlierSessionRows(
    listing(
      { harness: "claude", session: past({ name: "The parser", display_name: "The parser" }), conversation: conversation({ title: null }), action: "reopen" },
      { harness: "claude", session: past({ last_prompt: "hello" }), conversation: null, action: "reopen" },
    ),
  );
  expect(named!.label).toBe("The parser");
  expect(unnamed!.label).toBe("hello");
});

test("a note says why a row stands out, and only an open-elsewhere row is not takeable", () => {
  const rows = earlierSessionRows(
    listing(
      { harness: "claude", session: past(), conversation: conversation(), action: "reopen" },
      { harness: "claude", session: past({ here: false, thread_title: "Elsewhere" }), conversation: conversation({ id: "x" }), action: "resume" },
      { harness: "codex", session: null, conversation: conversation({ id: "y" }), action: "resume" },
      { harness: "claude", session: past({ open: true, closed_at: null }), conversation: conversation({ id: "z" }), action: "switch" },
      { harness: "claude", session: past({ open: true, here: false, display_name: "Helper", thread_title: "Other" }), conversation: conversation({ id: "w" }), action: "none" },
    ),
  );
  expect(rows.map((r) => r.note)).toEqual([
    "closed here",
    "from Elsewhere",
    "not started in oxplow",
    "open in this thread",
    "open in Helper (Other)",
  ]);
  expect(rows.map((r) => r.status)).toEqual(["closed_here", "elsewhere", "outside", "open_here", "open_elsewhere"]);
  expect(rows.map(takeable)).toEqual([true, true, true, true, false]);
});

test("a row shows its turns, its size and when it was last active, each on its own", () => {
  const [a, b] = earlierSessionRows(
    listing(
      { harness: "claude", session: past({ turns: 3 }), conversation: conversation({ size_bytes: 2048 }), action: "reopen" },
      { harness: "claude", session: null, conversation: conversation({ id: "s", size_bytes: 2048 }), action: "resume" },
    ),
  );
  expect([a!.turns, a!.size]).toEqual(["3", "2 KB"]);
  expect([b!.turns, b!.size]).toEqual([null, "2 KB"]);
  expect(a!.when).toMatch(/^2026-10-10 \d\d:\d\d$/);
});

test("the filter matches every word against title, prompts, thread and harness", () => {
  const rows = earlierSessionRows(
    listing(
      { harness: "claude", session: null, conversation: conversation({ id: "a", title: "Fix login", first_prompt: "the redirect loops" }), action: "resume" },
      { harness: "codex", session: past({ thread_title: "Billing", last_prompt: "refund flow" }), conversation: null, action: "reopen" },
    ),
  );
  expect(filterEarlierRows(rows, "").length).toBe(2);
  expect(filterEarlierRows(rows, "LOGIN redirect").map((r) => r.label)).toEqual(["Fix login"]);
  expect(filterEarlierRows(rows, "billing").map((r) => r.harness)).toEqual(["codex"]);
  expect(filterEarlierRows(rows, "codex refund").length).toBe(1);
  expect(filterEarlierRows(rows, "nothing like it")).toEqual([]);
});

// A session whose conversation the harness didn't list resumes on its own
// native id; a session ref is never a conversation id.
test("a resume row's conversation is a native id, never a session ref", () => {
  const rows = earlierSessionRows(
    listing(
      { harness: "claude", session: past({ here: false, conversation: "native-9", last_prompt: "hi" }), conversation: null, action: "resume" },
      { harness: "claude", session: null, conversation: conversation({ id: "c7" }), action: "resume" },
    ),
  );
  expect(rows.map((r) => r.conversation)).toEqual(["native-9", "c7"]);
  for (const r of rows) expect(r.conversation).not.toMatch(/^agent_session:/);
});
