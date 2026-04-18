import type { CSSProperties } from "react";
import type { Stream } from "../api.js";

interface Props {
  stream: Stream | null;
  error: string | null;
}

export function StreamHeader({ stream, error }: Props) {
  return (
    <div style={rowStyle}>
      {stream ? <span>{stream.branch}</span> : null}
      {stream ? (
        <span
          title={stream.worktree_path}
          style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, minWidth: 0 }}
        >
          · {stream.worktree_path}
        </span>
      ) : null}
      {error ? <span style={{ color: "#ff6b6b", flexShrink: 0 }}>{error}</span> : null}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 10,
  padding: "2px 12px",
  borderBottom: "1px solid var(--border)",
  fontSize: 11,
  color: "var(--muted)",
  minHeight: 22,
  background: "var(--bg-2)",
};
