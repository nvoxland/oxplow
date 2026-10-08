/**
 * One agent status for many: a thread's dot from its sessions', a stream's
 * from its threads'. What the person owes ranks first — an answer
 * (`awaiting`), then the next move after a dead turn (`stalled`) — then
 * work in flight (`working`), then `waiting`. The backend's
 * `oxplow_domain::agent::roll_up_status` follows the same rule
 * (`crates/oxplow-domain/fixtures/agent_status_rollup.json`).
 */
import type { AgentStatus, AgentStatusEntry } from "./api.js";

const ATTENTION: Record<AgentStatus, number> = { awaiting: 3, stalled: 2, working: 1, waiting: 0 };

/** The roll-up of `states`; `null` when there are none. */
export function rollUpAgentStatus(states: Iterable<AgentStatus>): AgentStatus | null {
  let top: AgentStatus | null = null;
  for (const s of states) {
    if (top === null || ATTENTION[s] > ATTENTION[top]) top = s;
  }
  return top;
}

/** Where a session's status is kept: its session id, or its thread's
 *  activity no session claims. */
export function sessionStatusKey(entry: AgentStatusEntry): string {
  return entry.sessionId ?? `${entry.threadId}:unclaimed`;
}

/** Each thread's status, rolled up from its sessions', and the question
 *  the session it took that status from is asking. */
export function threadStatuses(entries: Iterable<AgentStatusEntry>): {
  statuses: Record<string, AgentStatus>;
  questions: Record<string, string | undefined>;
} {
  const byThread = new Map<string, AgentStatusEntry[]>();
  for (const e of entries) {
    const list = byThread.get(e.threadId) ?? [];
    list.push(e);
    byThread.set(e.threadId, list);
  }
  const statuses: Record<string, AgentStatus> = {};
  const questions: Record<string, string | undefined> = {};
  for (const [threadId, list] of byThread) {
    const status = rollUpAgentStatus(list.map((e) => e.status));
    if (status === null) continue;
    statuses[threadId] = status;
    questions[threadId] = list.find((e) => e.status === status)?.question;
  }
  return { statuses, questions };
}
