import type { Stream } from "../api.js";

export function TopBar({ stream, error }: { stream: Stream | null; error: string | null }) {
  return (
    <div style={{ display: "flex", alignItems: "center", padding: "8px 12px", gap: 16 }}>
      <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
        <div style={{ fontWeight: 600, fontSize: 14 }}>
          {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : stream?.title ?? "…"}
        </div>
        <div style={{ color: "var(--muted)", fontSize: 12 }}>
          {stream?.summary || (stream ? `branch: ${stream.branch}` : "")}
        </div>
      </div>
      <div style={{ color: "var(--muted)", fontSize: 12 }}>
        {stream ? `id: ${stream.id}` : ""}
      </div>
      <button
        onClick={() => console.log("new stream clicked")}
        style={{
          background: "var(--bg-2)",
          color: "var(--fg)",
          border: "1px solid var(--border)",
          padding: "6px 12px",
          borderRadius: 4,
          cursor: "pointer",
        }}
      >
        + New stream
      </button>
    </div>
  );
}
