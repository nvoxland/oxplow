import type { StoredEvent } from "./hook-ingest.js";

export type AgentStatus = "idle" | "working" | "waiting" | "done";

export function deriveThreadAgentStatus(events: readonly StoredEvent[]): AgentStatus {
  // Fold chronological events into a running status. We only care about the
  // last status-relevant event; any event of a given kind subsumes earlier
  // ones. `meta` events never change status.
  //
  // Subagent-in-flight carve-out: when the parent has dispatched one or
  // more `Task` subagents that haven't returned, a `stop` event must NOT
  // flip status to "done" — the subagent is still doing work, and the
  // tab icon needs to reflect that. We track the count of unreturned
  // `Task` PreToolUse events; while >0, `stop` keeps the status as
  // "working" instead of transitioning to "done".
  let status: AgentStatus = "idle";
  let pendingTasks = 0;
  for (const stored of events) {
    const ev = stored.normalized;
    switch (ev.kind) {
      case "session-end":
        status = "idle";
        pendingTasks = 0;
        break;
      case "session-start":
        status = "done";
        break;
      case "stop":
        status = pendingTasks > 0 ? "working" : "done";
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
        break;
    }
  }
  return status;
}
