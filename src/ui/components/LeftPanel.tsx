import type { ReactNode } from "react";
import type { Stream } from "../api.js";

export function LeftPanel({ stream }: { stream: Stream | null }) {
  if (!stream) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>loading stream…</div>;
  }

  return (
    <div style={{ padding: 12, fontSize: 12, display: "flex", flexDirection: "column", gap: 12 }}>
      <Section title="Stream">
        <Row label="Name" value={stream.title} />
        <Row label="Branch" value={stream.branch} />
        <Row label="Source" value={stream.branch_source} />
        <Row label="Worktree" value={stream.worktree_path} />
      </Section>
      <Section title="Claude resume">
        <Row label="Working" value={stream.resume.working_session_id || "not started yet"} />
        <Row label="Talking" value={stream.resume.talking_session_id || "not started yet"} />
      </Section>
      <Section title="Panes">
        <Row label="Working" value={stream.panes.working} />
        <Row label="Talking" value={stream.panes.talking} />
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.6, fontSize: 11 }}>
        {title}
      </div>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ color: "var(--muted)" }}>{label}</div>
      <div style={{ wordBreak: "break-word" }}>{value}</div>
    </div>
  );
}
