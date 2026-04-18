import type { CSSProperties } from "react";
import { useState } from "react";
import type { Stream } from "../api.js";

interface Props {
  stream: Stream | null;
  error: string | null;
  onRename(title: string): Promise<void>;
}

export function StreamHeader({ stream, error, onRename }: Props) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);

  function start() {
    if (!stream) return;
    setDraft(stream.title);
    setRenameError(null);
    setEditing(true);
  }

  async function submit() {
    if (!draft.trim()) return;
    try {
      await onRename(draft.trim());
      setRenameError(null);
      setEditing(false);
    } catch (e) {
      setRenameError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "4px 12px 6px",
        borderBottom: "1px solid var(--border)",
        fontSize: 12,
        color: "var(--muted)",
        minHeight: 28,
      }}
    >
      {editing ? (
        <>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submit();
              if (e.key === "Escape") { setRenameError(null); setEditing(false); }
            }}
            autoFocus
            style={inputStyle}
          />
          <button onClick={() => void submit()} style={buttonStyle}>Save</button>
          <button onClick={() => { setRenameError(null); setEditing(false); }} style={buttonStyle}>Cancel</button>
          {renameError ? <span style={{ color: "#ff6b6b" }}>{renameError}</span> : null}
        </>
      ) : (
        <>
          <strong style={{ color: "var(--fg)" }}>{stream?.title ?? "…"}</strong>
          {stream ? <button onClick={start} style={buttonStyle}>Rename</button> : null}
          {stream ? <span>branch: {stream.branch}</span> : null}
          {stream ? <span title={stream.worktree_path} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, minWidth: 0 }}>{stream.worktree_path}</span> : null}
          {stream ? <span style={{ flexShrink: 0 }}>id: {stream.id}</span> : null}
          {error ? <span style={{ color: "#ff6b6b", flexShrink: 0 }}>{error}</span> : null}
        </>
      )}
    </div>
  );
}

const buttonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "2px 8px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
};

const inputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 8px",
  borderRadius: 4,
  fontFamily: "inherit",
  minWidth: 200,
};
