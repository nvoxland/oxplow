import type { StoredEvent } from "./hook-ingest.js";

/**
 * Two-state agent status. `working` = the agent is actively burning
 * cycles (a turn is in flight). `waiting` = the agent isn't doing
 * anything; the user owes the next move. Brand-new threads, finished
 * turns, exited processes, and permission prompts all collapse to
 * `waiting` — the actionable signal in each is identical.
 */
export type AgentStatus = "working" | "waiting";

export function deriveThreadAgentStatus(events: readonly StoredEvent[]): AgentStatus {
  // Subagent-in-flight carve-out: when the parent has dispatched one or
  // more `Task` subagents that haven't returned, a `stop` event must NOT
  // flip status back to `waiting` — the subagent is still doing work.
  // While the count is >0, `stop` keeps the status as `working`.
  let status: AgentStatus = "waiting";
  let pendingTasks = 0;
  for (const stored of events) {
    const ev = stored.normalized;
    switch (ev.kind) {
      case "session-end":
        status = "waiting";
        pendingTasks = 0;
        break;
      case "session-start":
        status = "waiting";
        break;
      case "stop":
        status = pendingTasks > 0 ? "working" : "waiting";
        break;
      case "notification":
        status = "waiting";
        break;
      case "user-prompt":
        status = "working";
        break;
      case "tool-use-start":
        status = "working";
        if (ev.toolName === "Task") pendingTasks += 1;
        break;
      case "tool-use-end":
        status = "working";
        if (ev.toolName === "Task" && pendingTasks > 0) pendingTasks -= 1;
        break;
      case "meta":
        // Terminal-synthesized `Interrupt` event: user pressed Escape
        // mid-turn. Claude Code doesn't reliably fire Stop on
        // user-initiated interrupts, so without this carve-out the tab
        // icon stays `working` until the next prompt. Reset
        // pendingTasks too — any in-flight Task is also dead.
        if (ev.hookEventName === "Interrupt" && status === "working") {
          status = "waiting";
          pendingTasks = 0;
        }
        break;
    }
  }
  return status;
}
