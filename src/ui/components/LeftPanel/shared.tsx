import type { CSSProperties, ReactNode } from "react";
import type { Thread, GitFileStatus } from "../../api.js";

export interface ContextMenuTarget {
  path: string;
  kind: "file" | "directory";
  name: string;
  x: number;
  y: number;
}

export const smallButtonStyle = {
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "inherit",
  borderRadius: 6,
  padding: "6px 10px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
} satisfies CSSProperties;

export const threadInputStyle = {
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  borderRadius: 6,
  padding: "8px 10px",
  fontFamily: "inherit",
  fontSize: 12,
} satisfies CSSProperties;

export const iconButtonStyle = {
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  borderRadius: 6,
  width: 24,
  height: 24,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: 0,
} satisfies CSSProperties;

export function SidebarButton({ active, onClick, children }: { active: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button type="button"
      onClick={onClick}
      style={{
        flex: 1,
        padding: "8px 12px",
        border: "none",
        borderBottom: active ? "2px solid var(--accent)" : "2px solid transparent",
        background: active ? "var(--bg)" : "transparent",
        color: active ? "var(--fg)" : "var(--muted)",
        cursor: "pointer",
        fontFamily: "inherit",
      }}
    >
      {children}
    </button>
  );
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.6, fontSize: 11 }}>
        {title}
      </div>
      {children}
    </div>
  );
}

export function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ color: "var(--muted)" }}>{label}</div>
      <div style={{ wordBreak: "break-word" }}>{value}</div>
    </div>
  );
}

export function InlineBadge({ children }: { children: ReactNode }) {
  return (
    <span
      style={{
        borderRadius: 999,
        border: "1px solid var(--border)",
        color: "var(--muted)",
        fontSize: 10,
        padding: "2px 6px",
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {children}
    </span>
  );
}

export function StatusBadge({ status }: { status: GitFileStatus | null }) {
  const color = statusColor(status);
  return (
    <span style={{ color, fontSize: 11, flexShrink: 0 }}>
      {status === null ? "●" : shortStatus(status)}
    </span>
  );
}

export function shortStatus(status: GitFileStatus): string {
  switch (status) {
    case "modified": return "M";
    case "added": return "A";
    case "deleted": return "D";
    case "renamed": return "R";
    case "untracked": return "U";
  }
}

export function statusColor(status: GitFileStatus | null): string {
  switch (status) {
    case "modified": return "#fcd34d";
    case "added": return "#86efac";
    case "deleted": return "#fca5a5";
    case "renamed": return "#c4b5fd";
    case "untracked": return "#7dd3fc";
    default: return "#fcd34d";
  }
}

export function threadStatusColor(status: Thread["status"]) {
  switch (status) {
    case "active": return "#86efac";
    case "queued": return "#7dd3fc";
  }
}

export function dirname(path: string): string {
  if (!path) return "";
  const slash = path.lastIndexOf("/");
  return slash >= 0 ? path.slice(0, slash) : "";
}

export function basename(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash >= 0 ? path.slice(slash + 1) : path;
}

export function joinChildPath(parent: string, child: string): string {
  return parent ? `${parent}/${child}` : child;
}

export async function copyText(text: string): Promise<void> {
  await navigator.clipboard.writeText(text);
}
