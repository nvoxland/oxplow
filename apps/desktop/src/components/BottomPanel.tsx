import { useEffect, useRef, useState } from "react";
import { listAgentEvents, subscribeAgentEvents, type AgentEvent } from "../api.js";
import { reportUiError } from "../ui-error.js";

const MAX_ROWS = 200;

/** The agent activity log: the stream's logged `agent.*` events (P3.9 —
 *  it survives a restart; it used to read an in-memory hook ring). */
export function BottomPanel({ streamId }: { streamId: string | null }) {
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const scrollerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!streamId) return;
    setEvents([]);
    // Newest first from the backend; shown oldest first, like a log.
    const show = (newest: AgentEvent[]) => setEvents(newest.slice(0, MAX_ROWS).reverse());
    void listAgentEvents(streamId, MAX_ROWS)
      .then(show)
      .catch((err) => reportUiError("Load agent activity", err));
    return subscribeAgentEvents(streamId, show);
  }, [streamId]);

  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [events]);

  return (
    <div
      style={{
        height: 160,
        display: "flex",
        flexDirection: "column",
        fontSize: 11,
        fontFamily: "var(--font-mono)",
      }}
    >
      <div
        style={{
          padding: "4px 8px",
          color: "var(--muted)",
          borderBottom: "1px solid var(--border)",
          display: "flex",
          justifyContent: "space-between",
        }}
      >
        <span>agent activity</span>
        <span>{events.length} / {MAX_ROWS}</span>
      </div>
      <div ref={scrollerRef} style={{ flex: 1, overflowY: "auto", padding: "4px 8px" }}>
        {events.length === 0 ? (
          <div style={{ color: "var(--muted)" }}>waiting for activity…</div>
        ) : (
          events.map((e) => <EventRow key={e.seq} evt={e} />)
        )}
      </div>
    </div>
  );
}

function EventRow({ evt }: { evt: AgentEvent }) {
  const kind = evt.type.replace(/^agent\./, "");
  return (
    <div style={{ display: "flex", gap: 8, whiteSpace: "nowrap" }}>
      <span style={{ color: "var(--muted)" }}>{formatTime(evt.at)}</span>
      <span style={{ color: kindColor(kind), width: 130, flexShrink: 0 }}>{kind}</span>
      <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{detail(kind, evt.payload)}</span>
    </div>
  );
}

type Payload = Record<string, unknown>;

function str(p: Payload, key: string): string {
  const v = p[key];
  return typeof v === "string" ? v : "";
}

/** What a row says about its event, from the payload. */
export function detail(kind: string, raw: unknown): string {
  const p = (raw && typeof raw === "object" ? raw : {}) as Payload;
  // Retention replaced the payload (every live agent payload has fields).
  if (raw && typeof raw === "object" && Object.keys(p).length === 0) return "(details expired)";
  switch (kind) {
    case "tool.requested": {
      const target = str(p, "path") || str(p, "detail");
      const denied = p.decision === "denied" ? ` · denied: ${str(p, "reason")}` : "";
      return `${str(p, "tool")}${target ? " · " + truncate(target, 80) : ""}${denied}`;
    }
    case "tool.finished": {
      const target = str(p, "path") || str(p, "detail");
      const outcome = p.ok === false ? " · failed" : p.ok === true ? " · ok" : "";
      return `${str(p, "tool")}${target ? " · " + truncate(target, 80) : ""}${outcome}`;
    }
    case "turn.ended":
      return str(p, "outcome");
    case "prompt.submitted":
      return p.reprompt ? "re-prompt" : "";
    case "status.changed":
      return `${str(p, "state")}${str(p, "detail") ? " · " + truncate(str(p, "detail"), 80) : ""}`;
    case "session.started":
      return `${str(p, "harness")}${p.resumed ? " (resumed)" : ""}`;
    case "session.ended":
      return str(p, "reason");
    default:
      return "";
  }
}

function kindColor(kind: string): string {
  switch (kind) {
    case "prompt.submitted":
    case "turn.started":
      return "#7dd3fc";
    case "tool.requested":
      return "#a5b4fc";
    case "tool.finished":
      return "#86efac";
    case "session.started":
      return "#fcd34d";
    case "session.ended":
      return "#fca5a5";
    case "turn.ended":
      return "#fda4af";
    case "status.changed":
      return "#e0e7ff";
    default:
      return "var(--muted)";
  }
}

function formatTime(at: string): string {
  const d = new Date(at);
  return (
    String(d.getHours()).padStart(2, "0") +
    ":" +
    String(d.getMinutes()).padStart(2, "0") +
    ":" +
    String(d.getSeconds()).padStart(2, "0")
  );
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1) + "…";
}
