import { useEffect, useRef, useState } from "react";
import type { StoredEvent, NormalizedEvent } from "../api.js";

const MAX_ROWS = 200;

export function BottomPanel({ streamId }: { streamId: string | null }) {
  const [events, setEvents] = useState<StoredEvent[]>([]);
  const scrollerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!streamId) return;
    setEvents([]);
    const es = new EventSource(`/api/hooks/stream?stream=${encodeURIComponent(streamId)}`);
    es.onmessage = (msg) => {
      try {
        const evt = JSON.parse(msg.data) as StoredEvent;
        setEvents((prev) => {
          const next = [...prev, evt];
          return next.length > MAX_ROWS ? next.slice(next.length - MAX_ROWS) : next;
        });
      } catch {}
    };
    es.onerror = () => {
      // EventSource auto-reconnects; nothing to do here.
    };
    return () => es.close();
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
        fontFamily: "ui-monospace, Menlo, monospace",
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
        <span>hook events</span>
        <span>{events.length} / {MAX_ROWS}</span>
      </div>
      <div ref={scrollerRef} style={{ flex: 1, overflowY: "auto", padding: "4px 8px" }}>
        {events.length === 0 ? (
          <div style={{ color: "var(--muted)" }}>waiting for events…</div>
        ) : (
          events.map((e) => <EventRow key={e.id} evt={e} />)
        )}
      </div>
    </div>
  );
}

function EventRow({ evt }: { evt: StoredEvent }) {
  const { normalized: n } = evt;
  const time = formatTime(n.t);
  return (
    <div style={{ display: "flex", gap: 8, whiteSpace: "nowrap" }}>
      <span style={{ color: "var(--muted)" }}>{time}</span>
      <span style={{ color: kindColor(n.kind), width: 110, flexShrink: 0 }}>{n.kind}</span>
      <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{detail(n)}</span>
    </div>
  );
}

function detail(n: NormalizedEvent): string {
  switch (n.kind) {
    case "session-start":
      return n.cwd ?? "";
    case "session-end":
      return n.reason ?? "";
    case "user-prompt":
      return truncate(n.prompt, 120);
    case "tool-use-start":
      return `${n.toolName}${n.target ? " · " + n.target : ""}`;
    case "tool-use-end":
      return `${n.toolName} · ${n.status}`;
    case "stop":
      return "";
    case "notification":
      return n.message;
    case "meta":
      return n.hookEventName;
  }
}

function kindColor(kind: string): string {
  switch (kind) {
    case "user-prompt":
      return "#7dd3fc";
    case "tool-use-start":
      return "#a5b4fc";
    case "tool-use-end":
      return "#86efac";
    case "session-start":
      return "#fcd34d";
    case "session-end":
      return "#fca5a5";
    case "stop":
      return "#fda4af";
    case "notification":
      return "#e0e7ff";
    default:
      return "var(--muted)";
  }
}

function formatTime(t: number): string {
  const d = new Date(t);
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
