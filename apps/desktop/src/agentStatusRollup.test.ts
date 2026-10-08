import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { collapseAgentStatusState, type AgentStatus, type AgentStatusEntry } from "./api.js";
import { rollUpAgentStatus, sessionStatusKey, threadStatuses } from "./agentStatusRollup.js";

interface Case {
  states: string[];
  expect: string | null;
}

/** The roll-up the backend follows (`oxplow_domain::agent::roll_up_status`):
 *  rolled up over the dot's alphabet, every case agrees with it. */
test("statuses roll up as the shared fixture says", () => {
  const fixture = JSON.parse(
    readFileSync(join(import.meta.dir, "../../../crates/oxplow-domain/fixtures/agent_status_rollup.json"), "utf8"),
  ) as { cases: Case[] };
  for (const c of fixture.cases) {
    const rolled = rollUpAgentStatus(c.states.map(collapseAgentStatusState));
    expect([c.states, rolled]).toEqual([c.states, c.expect === null ? null : collapseAgentStatusState(c.expect)]);
  }
});

/** A thread's dot is its sessions' roll-up, and its question the one the
 *  awaiting session asks. */
test("each thread rolls its sessions up", () => {
  const entry = (threadId: string, sessionId: string | null, status: AgentStatus, question?: string): AgentStatusEntry => ({
    streamId: "",
    threadId,
    sessionId,
    status,
    question,
  });
  expect(
    threadStatuses([
      entry("thr1", "ses1", "working"),
      entry("thr1", "ses2", "awaiting", "Which DB?"),
      entry("thr2", "ses3", "stalled"),
      entry("thr2", null, "working"),
    ]),
  ).toEqual({
    statuses: { thr1: "awaiting", thr2: "stalled" },
    questions: { thr1: "Which DB?", thr2: undefined },
  });
});

test("a session's status is kept under its own key", () => {
  expect(sessionStatusKey({ streamId: "", threadId: "thr1", sessionId: "ses4", status: "working" })).toBe("ses4");
  expect(sessionStatusKey({ streamId: "", threadId: "thr1", sessionId: null, status: "working" })).toBe("thr1:unclaimed");
});
