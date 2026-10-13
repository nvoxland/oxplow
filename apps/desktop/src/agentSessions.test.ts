import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { agentSessionsFromResult } from "./agentSessions.js";

test("a v_agent_session row reads as a session under the UI's ids", () => {
  const result = {
    columns: ["ref", "thread_ref", "kind", "harness", "acp_agent", "title", "opened_at", "stopped_by_user_at"],
    rows: [
      ["agent_session:ses3", "thread:thr1", "terminal", "claude", null, "", "2026-10-08T00:00:00Z", null],
      ["agent_session:ses4", "thread:thr1", "chat", "acp", "gemini", "review", "2026-10-08T00:01:00Z", "2026-10-08T00:02:00Z"],
    ],
  } as unknown as SqlQueryResult;
  expect(agentSessionsFromResult(result)).toEqual([
    { id: "ses3", threadId: "thr1", kind: "terminal", harness: "claude", acpAgent: null, title: "", openedAt: "2026-10-08T00:00:00Z", stoppedByUser: false },
    { id: "ses4", threadId: "thr1", kind: "chat", harness: "acp", acpAgent: "gemini", title: "review", openedAt: "2026-10-08T00:01:00Z", stoppedByUser: true },
  ]);
});
