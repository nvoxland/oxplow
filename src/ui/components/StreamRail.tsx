import type { CSSProperties } from "react";
import { useMemo, useState } from "react";
import { createStream, listBranches, type AgentStatus, type BranchRef, type Stream } from "../api.js";
import { logUi } from "../logger.js";
import { AgentStatusDot } from "./AgentStatusDot.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  streamStatuses: Record<string, AgentStatus>;
  gitEnabled: boolean;
  onSwitch(id: string): void;
  onStreamCreated(stream: Stream): void;
}

export function StreamRail({ stream, streams, streamStatuses, gitEnabled, onSwitch, onStreamCreated }: Props) {
  const [showCreate, setShowCreate] = useState(false);
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
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

  async function openCreate() {
    if (!gitEnabled) return;
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
      logUi("info", "loaded branch list", { branchCount: nextBranches.length });
    } catch (e) {
      setFormError(String(e));
      logUi("error", "failed to load branch list", { error: String(e) });
    } finally {
      setLoadingBranches(false);
    }
  }

  async function handleCreate() {
    if (!title.trim()) return setFormError("Name is required");
    if (mode === "existing" && !selectedRef) return setFormError("Select an existing branch");
    if (mode === "new" && !newBranch.trim()) return setFormError("Enter a new branch name");
    if (mode === "new" && !startPointRef) return setFormError("Choose a starting branch");
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

  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 12px", overflow: "hidden" }}>
        <div style={{ display: "flex", gap: 6, overflowX: "auto", flex: 1, minWidth: 0 }}>
          {streams.map((candidate) => {
            const active = candidate.id === stream?.id;
            const status = streamStatuses[candidate.id] ?? "idle";
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
                title={candidate.title}
              >
                <AgentStatusDot status={status} />
                <span>{candidate.title}</span>
              </button>
            );
          })}
        </div>
        <span
          title={gitEnabled ? "Create a new stream" : "Disabled: this workspace root does not contain its own .git directory"}
          style={{ display: "inline-flex", flexShrink: 0 }}
        >
          <button
            onClick={openCreate}
            style={{ ...buttonStyle, opacity: gitEnabled ? 1 : 0.6, cursor: gitEnabled ? "pointer" : "not-allowed" }}
            disabled={!gitEnabled}
          >
            + New stream
          </button>
        </span>
      </div>
      {showCreate ? (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: 8,
            padding: 10,
            margin: "0 12px 8px",
            border: "1px solid var(--border)",
            borderRadius: 6,
            background: "var(--bg-2)",
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
              <select value={selectedRef} onChange={(e) => setSelectedRef(e.target.value)} style={selectStyle} disabled={loadingBranches}>
                {branches.map((branch) => (
                  <option key={branch.ref} value={branch.ref}>[{branch.kind}] {branch.name}</option>
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
                <select value={startPointRef} onChange={(e) => setStartPointRef(e.target.value)} style={selectStyle} disabled={loadingBranches}>
                  {selectableStartPoints.map((branch) => (
                    <option key={branch.ref} value={branch.ref}>[{branch.kind}] {branch.name}</option>
                  ))}
                </select>
              </label>
            </>
          )}
          <div style={{ gridColumn: "1 / 3", color: formError ? "#ff6b6b" : "var(--muted)", fontSize: 12 }}>
            {formError ?? "Each stream gets its own worktree and Claude resume metadata."}
          </div>
          <div style={{ gridColumn: "1 / 3", display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <button onClick={() => setShowCreate(false)} style={buttonStyle}>Cancel</button>
            <button onClick={handleCreate} style={buttonStyle} disabled={creating || loadingBranches}>
              {creating ? "Creating…" : "Create stream"}
            </button>
          </div>
        </div>
      ) : null}
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

const tabStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 8,
  border: "1px solid var(--border)",
  borderBottom: "2px solid transparent",
  borderRadius: 6,
  padding: "6px 12px",
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

const selectStyle: CSSProperties = { ...inputStyle, minWidth: 220 };

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: 12 };
