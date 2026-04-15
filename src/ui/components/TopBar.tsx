import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import { createStream, listBranches, type BranchRef, type StoredEvent, type Stream } from "../api.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  error: string | null;
  onSwitch(id: string): void;
  onRename(title: string): Promise<void>;
  onStreamCreated(stream: Stream): void;
}

export function TopBar({ stream, streams, error, onSwitch, onRename, onStreamCreated }: Props) {
  const [showCreate, setShowCreate] = useState(false);
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [isEditingName, setIsEditingName] = useState(false);
  const [editName, setEditName] = useState("");
  const [streamStatuses, setStreamStatuses] = useState<Record<string, StoredEvent["normalized"]>>({});
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [mode, setMode] = useState<"existing" | "new">("existing");
  const [selectedRef, setSelectedRef] = useState("");
  const [newBranch, setNewBranch] = useState("");
  const [startPointRef, setStartPointRef] = useState("");

  const selectableStartPoints = useMemo(
    () => branches.filter((branch) => branch.kind === "local" || !branch.name.endsWith("/HEAD")),
    [branches],
  );

  useEffect(() => {
    const es = new EventSource("/api/hooks/stream?stream=all");
    es.onmessage = (msg) => {
      try {
        const evt = JSON.parse(msg.data) as StoredEvent;
        setStreamStatuses((prev) => ({ ...prev, [evt.streamId]: evt.normalized }));
      } catch {}
    };
    return () => es.close();
  }, []);

  async function openCreate() {
    setShowCreate(true);
    setFormError(null);
    if (branches.length > 0) return;
    try {
      setLoadingBranches(true);
      const nextBranches = await listBranches();
      setBranches(nextBranches);
      setSelectedRef((prev) => prev || nextBranches[0]?.ref || "");
      setStartPointRef((prev) => prev || nextBranches[0]?.ref || "");
      setTitle((prev) => prev || `Stream ${streams.length + 1}`);
    } catch (e) {
      setFormError(String(e));
    } finally {
      setLoadingBranches(false);
    }
  }

  async function handleCreate() {
    if (!title.trim()) {
      setFormError("Name is required");
      return;
    }
    if (mode === "existing" && !selectedRef) {
      setFormError("Select an existing branch");
      return;
    }
    if (mode === "new" && !newBranch.trim()) {
      setFormError("Enter a new branch name");
      return;
    }
    if (mode === "new" && !startPointRef) {
      setFormError("Choose a starting branch");
      return;
    }
    try {
      setCreating(true);
      setFormError(null);
      const created = mode === "existing"
        ? await createStream({ title: title.trim(), summary: summary.trim(), source: "existing", ref: selectedRef })
        : await createStream({
            title: title.trim(),
            summary: summary.trim(),
            source: "new",
            branch: newBranch.trim(),
            startPointRef,
          });
      onStreamCreated(created);
      setShowCreate(false);
    } catch (e) {
      setFormError(String(e));
    } finally {
      setCreating(false);
    }
  }

  function startRename() {
    if (!stream) return;
    setEditName(stream.title);
    setIsEditingName(true);
  }

  function cancelRename() {
    setIsEditingName(false);
    setEditName("");
  }

  async function submitRename() {
    if (!editName.trim()) {
      return;
    }
    await onRename(editName.trim());
    setIsEditingName(false);
  }

  return (
    <div style={{ display: "flex", alignItems: "flex-start", padding: "8px 12px", gap: 16 }}>
      <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1, gap: 6 }}>
        <div style={{ display: "flex", gap: 6, overflowX: "auto", paddingBottom: 4 }}>
          {streams.map((candidate) => {
            const active = candidate.id === stream?.id;
            const status = statusForEvent(streamStatuses[candidate.id]);
            return (
              <button
                key={candidate.id}
                onClick={() => onSwitch(candidate.id)}
                style={{
                  ...tabStyle,
                  background: active ? "var(--bg)" : "var(--bg-2)",
                  borderBottomColor: active ? "var(--accent)" : "transparent",
                  color: active ? "var(--fg)" : "var(--muted)",
                }}
                title={status.label}
              >
                <span aria-hidden="true">{status.icon}</span>
                <span>{candidate.title}</span>
              </button>
            );
          })}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          {isEditingName ? (
            <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
              <input
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void submitRename();
                  if (e.key === "Escape") cancelRename();
                }}
                style={{ ...inputStyle, minWidth: 180 }}
                autoFocus
              />
              <button onClick={() => void submitRename()} style={buttonStyle}>
                Save
              </button>
              <button onClick={cancelRename} style={buttonStyle}>
                Cancel
              </button>
            </div>
          ) : (
            <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
              <div style={{ fontWeight: 600, fontSize: 14, minWidth: 0 }}>
                {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : stream?.title ?? "…"}
              </div>
              {stream ? (
                <button onClick={startRename} style={secondaryButtonStyle}>
                  Rename
                </button>
              ) : null}
            </div>
          )}
        </div>
        <div style={{ color: "var(--muted)", fontSize: 12 }}>
          {stream
            ? `${stream.summary || `branch: ${stream.branch}`} · ${stream.worktree_path}`
            : ""}
        </div>
        {showCreate ? (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "1fr 1fr",
              gap: 8,
              padding: 10,
              border: "1px solid var(--border)",
              borderRadius: 6,
              background: "var(--bg-2)",
              maxWidth: 760,
            }}
          >
            <label style={labelStyle}>
              <span>Name</span>
              <input value={title} onChange={(e) => setTitle(e.target.value)} style={inputStyle} />
            </label>
            <label style={labelStyle}>
              <span>Summary</span>
              <input value={summary} onChange={(e) => setSummary(e.target.value)} style={inputStyle} />
            </label>
            <label style={labelStyle}>
              <span>Branch source</span>
              <select value={mode} onChange={(e) => setMode(e.target.value as "existing" | "new")} style={selectStyle}>
                <option value="existing">Existing branch</option>
                <option value="new">Create new branch</option>
              </select>
            </label>
            {mode === "existing" ? (
              <label style={labelStyle}>
                <span>Existing branch</span>
                <select
                  value={selectedRef}
                  onChange={(e) => setSelectedRef(e.target.value)}
                  style={selectStyle}
                  disabled={loadingBranches}
                >
                  {branches.map((branch) => (
                    <option key={branch.ref} value={branch.ref}>
                      [{branch.kind}] {branch.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : (
              <>
                <label style={labelStyle}>
                  <span>New branch</span>
                  <input value={newBranch} onChange={(e) => setNewBranch(e.target.value)} style={inputStyle} />
                </label>
                <label style={labelStyle}>
                  <span>Start point</span>
                  <select
                    value={startPointRef}
                    onChange={(e) => setStartPointRef(e.target.value)}
                    style={selectStyle}
                    disabled={loadingBranches}
                  >
                    {selectableStartPoints.map((branch) => (
                      <option key={branch.ref} value={branch.ref}>
                        [{branch.kind}] {branch.name}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}
            <div style={{ gridColumn: "1 / 3", color: formError ? "#ff6b6b" : "var(--muted)", fontSize: 12 }}>
              {formError ?? "Each stream gets its own worktree and Claude resume metadata."}
            </div>
            <div style={{ gridColumn: "1 / 3", display: "flex", justifyContent: "flex-end", gap: 8 }}>
              <button onClick={() => setShowCreate(false)} style={buttonStyle}>
                Cancel
              </button>
              <button onClick={handleCreate} style={buttonStyle} disabled={creating || loadingBranches}>
                {creating ? "Creating…" : "Create stream"}
              </button>
            </div>
          </div>
        ) : null}
      </div>
      <div style={{ color: "var(--muted)", fontSize: 12, paddingTop: 8 }}>
        {stream ? `id: ${stream.id}` : ""}
      </div>
      <button onClick={openCreate} style={buttonStyle}>
        + New stream
      </button>
    </div>
  );
}

const buttonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "6px 12px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
};

const secondaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  padding: "4px 8px",
  fontSize: 12,
};

const tabStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 8,
  border: "1px solid var(--border)",
  borderBottom: "2px solid transparent",
  borderRadius: 6,
  padding: "8px 12px",
  cursor: "pointer",
  fontFamily: "inherit",
  whiteSpace: "nowrap",
  flexShrink: 0,
};

const inputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "6px 8px",
  borderRadius: 4,
  fontFamily: "inherit",
};

const selectStyle: CSSProperties = {
  ...inputStyle,
  minWidth: 220,
};

const labelStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  fontSize: 12,
};

function statusForEvent(event: StoredEvent["normalized"] | undefined): { icon: string; label: string } {
  if (!event) return { icon: "○", label: "No recent activity" };
  switch (event.kind) {
    case "user-prompt":
    case "tool-use-start":
      return { icon: "◔", label: "Thinking" };
    case "tool-use-end":
      return { icon: event.status === "error" ? "⚠" : "●", label: event.status === "error" ? "Tool error" : "Active" };
    case "session-start":
      return { icon: "▶", label: "Session started" };
    case "session-end":
      return { icon: "■", label: "Session ended" };
    case "stop":
      return { icon: "■", label: "Stopped" };
    case "notification":
      return { icon: "✦", label: "Notification" };
    case "meta":
      return { icon: "○", label: event.hookEventName };
  }
}
