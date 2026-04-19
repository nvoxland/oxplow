import type { BatchFileChange } from "../../api.js";
import { fileChangeKindColor, netFileChanges } from "../../file-change-net.js";

interface Props {
  batchFileChanges: BatchFileChange[] | null;
  onOpenFile(path: string): void;
  onOpenTurnDiff?(turnId: string, path: string): void;
}

export function BatchChanges({ batchFileChanges, onOpenFile, onOpenTurnDiff }: Props) {
  if (batchFileChanges === null) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Loading…</div>;
  }
  const net = netFileChanges(batchFileChanges);
  if (net.length === 0) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No file changes recorded for this batch.</div>;
  }

  // Map path -> count of underlying raw events for the "× N" summary.
  const rawCountByPath = new Map<string, number>();
  for (const change of batchFileChanges) {
    rawCountByPath.set(change.path, (rawCountByPath.get(change.path) ?? 0) + 1);
  }

  return (
    <div style={{ flex: 1, overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6, marginBottom: 4 }}>
        {net.length} file{net.length === 1 ? "" : "s"} touched
      </div>
      {net.map((change) => {
        const rawCount = rawCountByPath.get(change.path) ?? 1;
        return (
          <button
            key={change.id}
            onClick={() => onOpenFile(change.path)}
            style={{
              background: "transparent",
              border: "1px solid transparent",
              borderRadius: 4,
              padding: "4px 6px",
              display: "flex",
              gap: 6,
              alignItems: "baseline",
              cursor: "pointer",
              textAlign: "left",
              color: "inherit",
              fontFamily: "inherit",
              fontSize: 12,
            }}
            title={`Open ${change.path}`}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-2)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
          >
            <span
              style={{
                fontSize: 9,
                textTransform: "uppercase",
                padding: "0 4px",
                borderRadius: 3,
                background: "var(--bg-2)",
                border: "1px solid var(--border)",
                color: fileChangeKindColor(change.change_kind),
                minWidth: 52,
                textAlign: "center",
              }}
            >
              {change.change_kind}
            </span>
            <span style={{ fontFamily: "ui-monospace, monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{change.path}</span>
            {onOpenTurnDiff && change.turn_id ? (
              <span
                role="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onOpenTurnDiff(change.turn_id!, change.path);
                }}
                title="Open turn diff"
                style={{ fontSize: 10, padding: "0 4px", border: "1px solid var(--border)", borderRadius: 3, cursor: "pointer", color: "var(--muted)" }}
              >
                diff
              </span>
            ) : null}
            <span style={{ color: "var(--muted)", marginLeft: "auto", fontSize: 11 }}>
              {rawCount > 1 ? `×${rawCount} · ` : ""}
              {change.source}
              {change.tool_name ? ` · ${change.tool_name}` : ""}
            </span>
          </button>
        );
      })}
    </div>
  );
}
