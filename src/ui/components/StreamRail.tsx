import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createStream, listBranches, type AgentStatus, type BranchRef, type Stream } from "../api.js";
import { logUi } from "../logger.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { ContextMenu } from "./ContextMenu.js";
import { WORK_ITEM_DRAG_MIME } from "./BatchRail.js";
import type { MenuItem } from "../menu.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  streamStatuses: Record<string, AgentStatus>;
  streamActiveBatchIds?: Record<string, string | null>;
  gitEnabled: boolean;
  onSwitch(id: string): void;
  onStreamCreated(stream: Stream): void;
  onRenameStream?(streamId: string, currentTitle: string): void;
  onRequestCreateBatch?(): void;
  onOpenSettings?(): void;
  onDropWorkItemOnStream?(targetStreamId: string, itemId: string, fromBatchId: string | null): void;
  /** Bumping this number opens the inline "new stream" form. */
  createRequest?: number;
}

export function StreamRail({ stream, streams, streamStatuses, streamActiveBatchIds, gitEnabled, onSwitch, onStreamCreated, onRenameStream, onRequestCreateBatch, onOpenSettings, onDropWorkItemOnStream, createRequest }: Props) {
  const [dragOverStreamId, setDragOverStreamId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; stream: Stream } | null>(null);
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
  const nameInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!showCreate) return;
    const t = setTimeout(() => nameInputRef.current?.focus(), 0);
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setShowCreate(false);
    }
    window.addEventListener("keydown", onKey);
    return () => {
      clearTimeout(t);
      window.removeEventListener("keydown", onKey);
    };
  }, [showCreate]);

  const selectableStartPoints = useMemo(
    () => branches.filter((branch) => branch.kind === "local" || !branch.name.endsWith("/HEAD")),
    [branches],
  );

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    void openCreate();
  }, [createRequest]);

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
    <div style={{ display: "flex", flexDirection: "column", background: "var(--bg-3)", borderBottom: "1px solid var(--border-strong)" }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 0, padding: "0 10px", overflow: "hidden" }}>
        <div className="newde-rail-scroll" style={{ display: "flex", gap: 2, overflowX: "auto", flex: 1, minWidth: 0, alignItems: "flex-end" }}>
          {streams.map((candidate) => {
            const active = candidate.id === stream?.id;
            const status = streamStatuses[candidate.id] ?? "idle";
            const canDrop = !!onDropWorkItemOnStream && !!streamActiveBatchIds?.[candidate.id];
            const isDragOver = dragOverStreamId === candidate.id;
            return (
              <button
                key={candidate.id}
                onClick={() => onSwitch(candidate.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  setContextMenu({ x: event.clientX, y: event.clientY, stream: candidate });
                }}
                onDragOver={canDrop ? (event) => {
                  const types = event.dataTransfer.types;
                  if (!types || !Array.from(types).includes(WORK_ITEM_DRAG_MIME)) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  if (dragOverStreamId !== candidate.id) setDragOverStreamId(candidate.id);
                } : undefined}
                onDragLeave={canDrop ? () => {
                  if (dragOverStreamId === candidate.id) setDragOverStreamId(null);
                } : undefined}
                onDrop={canDrop ? (event) => {
                  const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
                  if (!raw) return;
                  event.preventDefault();
                  setDragOverStreamId(null);
                  try {
                    const payload = JSON.parse(raw) as { itemId?: string; fromBatchId?: string | null };
                    if (!payload.itemId) return;
                    onDropWorkItemOnStream?.(candidate.id, payload.itemId, payload.fromBatchId ?? null);
                  } catch {
                    // ignore malformed payload
                  }
                } : undefined}
                style={{
                  ...tabStyle,
                  background: active ? "var(--bg-2)" : "transparent",
                  color: active ? "var(--fg)" : "var(--muted)",
                  fontWeight: active ? 600 : 400,
                  borderBottom: active
                    ? "2px solid var(--accent)"
                    : "2px solid transparent",
                  boxShadow: isDragOver ? "inset 0 0 0 2px var(--accent)" : undefined,
                }}
                title={candidate.title}
              >
                <AgentStatusDot status={status} />
                <span>{candidate.title}</span>
              </button>
            );
          })}
        </div>
        <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, padding: "6px 0 6px 8px" }}>
          <button
            onClick={openCreate}
            title={gitEnabled ? "Create a new stream" : "Disabled: this workspace root does not contain its own .git directory"}
            style={{ ...buttonStyle, opacity: gitEnabled ? 1 : 0.6, cursor: gitEnabled ? "pointer" : "not-allowed" }}
            disabled={!gitEnabled}
          >
            + New stream
          </button>
          {onOpenSettings ? (
            <button
              onClick={onOpenSettings}
              title="Settings"
              aria-label="Settings"
              style={iconButtonStyle}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ display: "block" }}>
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
            </button>
          ) : null}
        </span>
      </div>
      {showCreate ? (
        <div style={backdropStyle} onMouseDown={() => setShowCreate(false)}>
          <form
            onSubmit={(e) => { e.preventDefault(); void handleCreate(); }}
            onMouseDown={(e) => e.stopPropagation()}
            style={modalStyle}
          >
            <div style={modalHeaderStyle}>
              <span>New stream</span>
              <button type="button" onClick={() => setShowCreate(false)} style={closeBtnStyle} aria-label="Close">×</button>
            </div>
            <div style={modalBodyStyle}>
          <label style={labelStyle}>
            <span>Name</span>
            <input ref={nameInputRef} value={title} onChange={(e) => setTitle(e.target.value)} style={inputStyle} />
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
            <button type="button" onClick={() => setShowCreate(false)} style={buttonStyle}>Cancel</button>
            <button type="submit" style={buttonStyle} disabled={creating || loadingBranches}>
              {creating ? "Creating…" : "Create stream"}
            </button>
          </div>
            </div>
          </form>
        </div>
      ) : null}
      {contextMenu ? (
        <ContextMenu
          items={[
            {
              id: "stream.rename",
              label: "Rename…",
              enabled: !!onRenameStream,
              run: () => onRenameStream?.(contextMenu.stream.id, contextMenu.stream.title),
            },
            {
              id: "stream.add-stream",
              label: "Add stream",
              enabled: gitEnabled,
              run: () => void openCreate(),
            },
            {
              id: "stream.add-batch",
              label: "Add batch",
              enabled: !!onRequestCreateBatch,
              run: () => onRequestCreateBatch?.(),
            },
          ]}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={180}
        />
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

const iconButtonStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 6px",
  cursor: "pointer",
  fontFamily: "inherit",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
};

const tabStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 8,
  border: "none",
  // The bottom border is the accent underline for the active tab — set
  // via the inline style override on the element itself.
  padding: "8px 14px 6px 14px",
  cursor: "pointer",
  fontFamily: "inherit",
  whiteSpace: "nowrap",
  flexShrink: 0,
  borderTopLeftRadius: 6,
  borderTopRightRadius: 6,
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

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1100,
};

const modalStyle: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  boxShadow: "0 24px 60px rgba(0,0,0,0.5)",
  minWidth: 520,
  maxWidth: 720,
  maxHeight: "80vh",
  display: "flex",
  flexDirection: "column",
};

const modalHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "10px 14px",
  borderBottom: "1px solid var(--border)",
  fontSize: 13,
  fontWeight: 600,
  background: "var(--bg-1, var(--bg-2))",
};

const modalBodyStyle: CSSProperties = {
  padding: 14,
  display: "grid",
  gridTemplateColumns: "1fr 1fr",
  gap: 8,
  overflow: "auto",
};

const closeBtnStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--muted)",
  fontSize: 20,
  lineHeight: 1,
  cursor: "pointer",
};

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: 12 };
