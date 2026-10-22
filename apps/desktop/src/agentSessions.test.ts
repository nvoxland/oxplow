import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { agentSessionsFromResult } from "./agentSessions.js";

test("a v_agent_session row reads as a session under the UI's ids", () => {
  const result = {
    columns: ["uri", "thread_uri", "kind", "harness", "title", "native_title", "display_name", "opened_at", "stopped_by_user_at"],
    rows: [
      ["oxplow:oxplow/agent_session?id=ses3", "oxplow:oxplow/thread?id=thr1", "terminal", "claude", "", "", "claude 2026-10-08", "2026-10-08T00:00:00Z", null],
      ["oxplow:oxplow/agent_session?id=ses4", "oxplow:oxplow/thread?id=thr1", "terminal", "codex", "review", "Login fix", "review", "2026-10-08T00:01:00Z", "2026-10-08T00:02:00Z"],
    ],
  } as unknown as SqlQueryResult;
  expect(agentSessionsFromResult(result)).toEqual([
    { id: "ses3", threadId: "thr1", kind: "terminal", harness: "claude", title: "", nativeTitle: "", displayName: "claude 2026-10-08", openedAt: "2026-10-08T00:00:00Z", stoppedByUser: false },
    { id: "ses4", threadId: "thr1", kind: "terminal", harness: "codex", title: "review", nativeTitle: "Login fix", displayName: "review", openedAt: "2026-10-08T00:01:00Z", stoppedByUser: true },
  ]);
});
