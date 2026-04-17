import type { StoredEvent } from "./hook-ingest.js";

export type AgentStatus = "idle" | "working" | "waiting" | "done";

export function deriveBatchAgentStatus(events: readonly StoredEvent[]): AgentStatus {
  // Fold chronological events into a running status. We only care about the
  // last status-relevant event; any event of a given kind subsumes earlier
  // ones. `meta` events never change status.
  let status: AgentStatus = "idle";
  for (const stored of events) {
    const kind = stored.normalized.kind;
    switch (kind) {
      case "session-end":
        status = "idle";
        break;
      case "session-start":
      case "stop":
        status = "done";
        break;
      case "notification":
        status = "waiting";
        break;
      case "user-prompt":
      case "tool-use-start":
      case "tool-use-end":
        status = "working";
        break;
      case "meta":
        break;
    }
  }
  return status;
}
