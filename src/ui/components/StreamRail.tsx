import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { checkoutStreamBranch, createStream, listAdoptableWorktrees, listBranches, setStreamPrompt, type AgentStatus, type BranchRef, type GitWorktreeEntry, type Stream } from "../api.js";
import { logUi } from "../logger.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { BranchPicker, type PickedRef } from "./BranchPicker.js";
import { ContextMenu } from "./ContextMenu.js";
import { WORK_ITEM_DRAG_MIME, THREAD_DRAG_MIME } from "./ThreadRail.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  streamStatuses: Record<string, AgentStatus>;
  streamActiveThreadIds?: Record<string, string | null>;
  gitEnabled: boolean;
  onSwitch(id: string): void;
  onStreamCreated(stream: Stream): void;
  onRenameStream?(streamId: string, currentTitle: string): void;
  onRequestCreateThread?(): void;
  onOpenSettings?(): void;
  onDropWorkItemOnStream?(targetStreamId: string, itemId: string, fromThreadId: string | null): void;
  onReorderStreams?(orderedStreamIds: string[]): Promise<void> | void;
  /** Bumping this number opens the inline "new stream" form. */
  createRequest?: number;
}

export const STREAM_DRAG_MIME = "application/x-oxplow-stream";

export function StreamRail({ stream, streams, streamStatuses, streamActiveThreadIds, gitEnabled, onSwitch, onStreamCreated, onRenameStream, onRequestCreateThread, onOpenSettings, onDropWorkItemOnStream, onReorderStreams, createRequest }: Props) {
  const [dragOverStreamId, setDragOverStreamId] = useState<string | null>(null);
  const [draggingStreamId, setDraggingStreamId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; stream: Stream } | null>(null);
  const [settingsStream, setSettingsStream] = useState<Stream | null>(null);
  const [settingsPrompt, setSettingsPrompt] = useState("");
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [switchBranchStream, setSwitchBranchStream] = useState<Stream | null>(null);
  const [switchBranchRef, setSwitchBranchRef] = useState<string>("");
  const [switchBranchError, setSwitchBranchError] = useState<string | null>(null);
  const [switchBranchBusy, setSwitchBranchBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [mode, setMode] = useState<"existing" | "new" | "worktree">("existing");
  const [worktrees, setWorktrees] = useState<GitWorktreeEntry[]>([]);
  const [selectedWorktreePath, setSelectedWorktreePath] = useState("");
  const [selectedRef, setSelectedRef] = useState("");
  const [selectedRefLabel, setSelectedRefLabel] = useState("");
  const [newBranch, setNewBranch] = useState("");
  const [startPointRef, setStartPointRef] = useState("");
  const [startPointLabel, setStartPointLabel] = useState("");
  const nameInputRef = useRef<HTMLInputElement | null>(null);

  function openStreamSettings(candidate: Stream) {
    setSettingsStream(candidate);
    setSettingsPrompt(candidate.custom_prompt ?? "");
    setSettingsSaving(false);
  }

  async function saveStreamSettings() {
    if (!settingsStream) return;
    setSettingsSaving(true);
    try {
      await setStreamPrompt(settingsStream.id, settingsPrompt.trim() || null);
      setSettingsStream(null);
    } catch (e) {
      logUi("error", "failed to save stream prompt", { streamId: settingsStream.id, error: String(e) });
    } finally {
      setSettingsSaving(false);
    }
  }

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

  // Primary is always the leftmost tab regardless of persisted sort_index.
  const orderedStreams = useMemo(() => {
    const primary = streams.filter((s) => s.kind === "primary");
    const rest = streams.filter((s) => s.kind !== "primary");
    return [...primary, ...rest];
  }, [streams]);

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    void openCreate();
  }, [createRequest]);

  async function openCreate() {
    if (!gitEnabled) return;
    setShowCreate(true);
    setFormError(null);
    try {
      setLoadingBranches(true);
      const [nextBranches, nextWorktrees] = await Promise.all([
        branches.length > 0 ? Promise.resolve(branches) : listBranches(),
        listAdoptableWorktrees(),
      ]);
      if (branches.length === 0) setBranches(nextBranches);
      setWorktrees(nextWorktrees);
      setSelectedRef((prev) => prev || nextBranches[0]?.ref || "");
      setSelectedRefLabel((prev) => prev || nextBranches[0]?.name || "");
      setStartPointRef((prev) => prev || nextBranches[0]?.ref || "");
      setStartPointLabel((prev) => prev || nextBranches[0]?.name || "");
      setSelectedWorktreePath((prev) => prev || nextWorktrees[0]?.path || "");
      setTitle((prev) => prev || `Stream ${streams.length + 1}`);
      logUi("info", "loaded branch list", { branchCount: nextBranches.length, worktreeCount: nextWorktrees.length });
    } catch (e) {
      setFormError(String(e));
      logUi("error", "failed to load branch list", { error: String(e) });
    } finally {
      setLoadingBranches(false);
    }
  }

  async function openSwitchBranch(target: Stream) {
    setSwitchBranchStream(target);
    setSwitchBranchRef(`refs/heads/${target.branch}`);
    setSwitchBranchError(null);
    if (branches.length > 0) return;
    try {
      setLoadingBranches(true);
      const nextBranches = await listBranches();
      setBranches(nextBranches);
    } catch (e) {
      setSwitchBranchError(String(e));
    } finally {
      setLoadingBranches(false);
    }
  }

  async function handleSwitchBranch() {
    if (!switchBranchStream || !switchBranchRef) return;
    const branch = branches.find((b) => b.ref === switchBranchRef);
    if (!branch) {
      setSwitchBranchError("Select a branch");
      return;
    }
    const localName = branch.kind === "local" ? branch.name : branch.name.split("/").slice(1).join("/");
    try {
      setSwitchBranchBusy(true);
      setSwitchBranchError(null);
      await checkoutStreamBranch(switchBranchStream.id, localName);
      setSwitchBranchStream(null);
    } catch (e) {
      setSwitchBranchError(String(e));
    } finally {
      setSwitchBranchBusy(false);
    }
  }

  async function handleCreate() {
    if (!title.trim()) return setFormError("Name is required");
    if (mode === "existing" && !selectedRef) return setFormError("Select an existing branch");
    if (mode === "new" && !newBranch.trim()) return setFormError("Enter a new branch name");
    if (mode === "new" && !startPointRef) return setFormError("Choose a starting branch");
    if (mode === "worktree" && !selectedWorktreePath) return setFormError("Select a worktree");
    try {
      setCreating(true);
      setFormError(null);
      const created = mode === "existing"
        ? await createStream({ title: title.trim(), summary: summary.trim(), source: "existing", ref: selectedRef })
        : mode === "new"
        ? await createStream({
            title: title.trim(),
            summary: summary.trim(),
            source: "new",
            branch: newBranch.trim(),
            startPointRef,
          })
        : await createStream({
            title: title.trim(),
            summary: summary.trim(),
            source: "worktree",
            worktreePath: selectedWorktreePath,
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
        <div className="oxplow-rail-scroll" style={{ display: "flex", gap: 2, overflowX: "auto", flex: 1, minWidth: 0, alignItems: "flex-end" }}>
          {orderedStreams.map((candidate) => {
            const active = candidate.id === stream?.id;
            const status = streamStatuses[candidate.id] ?? "idle";
            const canDrop = !!onDropWorkItemOnStream && !!streamActiveThreadIds?.[candidate.id];
            const isDragOver = dragOverStreamId === candidate.id;
            const isStreamDragTarget = isDragOver && draggingStreamId !== null && draggingStreamId !== candidate.id;
            const isPrimary = candidate.kind === "primary";
            const canDrag = !!onReorderStreams && !isPrimary;
            const showBranchInTitle = isPrimary || candidate.title !== candidate.branch;
            return (
              <button type="button"
                key={candidate.id}
                draggable={canDrag}
                onClick={() => onSwitch(candidate.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  setContextMenu({ x: event.clientX, y: event.clientY, stream: candidate });
                }}
                onDragStart={canDrag ? (event) => {
                  event.dataTransfer.setData(STREAM_DRAG_MIME, JSON.stringify({ streamId: candidate.id }));
                  event.dataTransfer.effectAllowed = "move";
                  setDraggingStreamId(candidate.id);
                } : undefined}
                onDragEnd={onReorderStreams ? () => {
                  setDraggingStreamId(null);
                  setDragOverStreamId(null);
                } : undefined}
                onDragOver={(event) => {
                  const types = Array.from(event.dataTransfer.types);
                  if (types.includes(STREAM_DRAG_MIME)) {
                    event.preventDefault();
                    event.dataTransfer.dropEffect = "move";
                    if (dragOverStreamId !== candidate.id) setDragOverStreamId(candidate.id);
                    return;
                  }
                  if (!canDrop) return;
                  if (!types.includes(WORK_ITEM_DRAG_MIME) && !types.includes(THREAD_DRAG_MIME)) return;
                  if (!types.includes(WORK_ITEM_DRAG_MIME)) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  if (dragOverStreamId !== candidate.id) setDragOverStreamId(candidate.id);
                }}
                onDragLeave={() => {
                  if (dragOverStreamId === candidate.id) setDragOverStreamId(null);
                }}
                onDrop={(event) => {
                  const types = Array.from(event.dataTransfer.types);
                  if (types.includes(STREAM_DRAG_MIME) && onReorderStreams) {
                    event.preventDefault();
                    setDragOverStreamId(null);
                    if (!draggingStreamId || draggingStreamId === candidate.id) return;
                    // Never drop anything ahead of the primary tab.
                    if (candidate.kind === "primary") return;
                    const ids = orderedStreams.map((s) => s.id);
                    const fromIdx = ids.indexOf(draggingStreamId);
                    const toIdx = ids.indexOf(candidate.id);
                    if (fromIdx < 0 || toIdx < 0) return;
                    const next = ids.slice();
                    const [moved] = next.splice(fromIdx, 1);
                    next.splice(toIdx, 0, moved);
                    void onReorderStreams(next);
                    setDraggingStreamId(null);
                    return;
                  }
                  if (!canDrop) return;
                  const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
                  if (!raw) return;
                  event.preventDefault();
                  setDragOverStreamId(null);
                  try {
                    const payload = JSON.parse(raw) as {
                      itemId?: string;
                      itemIds?: string[];
                      fromThreadId?: string | null;
                    };
                    const ids = payload.itemIds && payload.itemIds.length > 0
                      ? payload.itemIds
                      : payload.itemId ? [payload.itemId] : [];
                    for (const id of ids) {
                      onDropWorkItemOnStream?.(candidate.id, id, payload.fromThreadId ?? null);
                    }
                  } catch {
                    // ignore malformed payload
                  }
                }}
                style={{
                  ...tabStyle,
                  background: active ? "var(--bg-2)" : "transparent",
                  color: active ? "var(--fg)" : "var(--muted)",
                  fontWeight: active ? 600 : 400,
                  borderBottom: active
                    ? "2px solid var(--accent)"
                    : "2px solid transparent",
                  borderLeft: isStreamDragTarget ? "2px solid var(--accent)" : undefined,
                  boxShadow: isDragOver && !isStreamDragTarget ? "inset 0 0 0 2px var(--accent)" : undefined,
                }}
                title={showBranchInTitle ? `${candidate.title} (${candidate.branch})` : candidate.title}
              >
                <AgentStatusDot status={status} />
                <span>{candidate.title}</span>
              </button>
            );
          })}
        </div>
        <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, padding: "6px 0 6px 8px" }}>
          <button type="button"
            onClick={openCreate}
            title={gitEnabled ? "Create a new stream" : "Disabled: this workspace root does not contain its own .git directory"}
            style={{ ...buttonStyle, opacity: gitEnabled ? 1 : 0.6, cursor: gitEnabled ? "pointer" : "not-allowed" }}
            disabled={!gitEnabled}
          >
            + New stream
          </button>
          {onOpenSettings ? (
            <button type="button"
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
        <div style={backdropStyle}>
          <form
            onSubmit={(e) => { e.preventDefault(); void handleCreate(); }}
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
            <select value={mode} onChange={(e) => setMode(e.target.value as "existing" | "new" | "worktree")} style={selectStyle}>
              <option value="existing">Existing branch</option>
              <option value="new">Create new branch</option>
              <option value="worktree" disabled={worktrees.length === 0}>
                {worktrees.length === 0 ? "Existing worktree (none available)" : "Existing worktree"}
              </option>
            </select>
          </label>
          {mode === "existing" ? (
            <label style={labelStyle}>
              <span>Existing branch</span>
              <BranchPicker
                label={<span>{selectedRefLabel || "Select branch…"}</span>}
                anchor="bottom"
                align="left"
                currentBranch={null}
                disabled={loadingBranches}
                buttonStyle={pickerButtonStyle}
                onPick={(target) => {
                  const { ref, label } = resolvePickedRef(target);
                  setSelectedRef(ref);
                  setSelectedRefLabel(label);
                }}
              />
            </label>
          ) : mode === "new" ? (
            <>
              <label style={labelStyle}>
                <span>New branch</span>
                <input value={newBranch} onChange={(e) => setNewBranch(e.target.value)} style={inputStyle} />
              </label>
              <label style={labelStyle}>
                <span>Start point</span>
                <BranchPicker
                  label={<span>{startPointLabel || "Select starting ref…"}</span>}
                  anchor="bottom"
                  align="left"
                  currentBranch={null}
                  disabled={loadingBranches}
                  buttonStyle={pickerButtonStyle}
                  onPick={(target) => {
                    const { ref, label } = resolvePickedRef(target);
                    setStartPointRef(ref);
                    setStartPointLabel(label);
                  }}
                />
              </label>
            </>
          ) : (
            <label style={{ ...labelStyle, gridColumn: "1 / 3" }}>
              <span>Existing worktree</span>
              <select
                value={selectedWorktreePath}
                onChange={(e) => setSelectedWorktreePath(e.target.value)}
                style={selectStyle}
                disabled={loadingBranches}
              >
                {worktrees.map((wt) => (
                  <option key={wt.path} value={wt.path}>
                    {wt.branch ? `[${wt.branch}]` : "[detached]"} {wt.path}
                  </option>
                ))}
              </select>
            </label>
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
              id: "stream.switch-branch",
              label: "Switch branch…",
              enabled: gitEnabled,
              run: () => { void openSwitchBranch(contextMenu.stream); },
            },
            {
              id: "stream.settings",
              label: "Settings",
              enabled: true,
              run: () => openStreamSettings(contextMenu.stream),
            },
            {
              id: "stream.add-stream",
              label: "Add stream",
              enabled: gitEnabled,
              run: () => void openCreate(),
            },
            {
              id: "stream.add-thread",
              label: "Add thread",
              enabled: !!onRequestCreateThread,
              run: () => onRequestCreateThread?.(),
            },
          ]}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={180}
        />
      ) : null}
      {switchBranchStream ? (
        <div style={backdropStyle}>
          <form
            onSubmit={(e) => { e.preventDefault(); void handleSwitchBranch(); }}
            style={{ ...modalStyle, minWidth: 420 }}
          >
            <div style={modalHeaderStyle}>
              <span>Switch branch — {switchBranchStream.title}</span>
              <button type="button" onClick={() => setSwitchBranchStream(null)} style={closeBtnStyle} aria-label="Close">×</button>
            </div>
            <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <label style={labelStyle}>
                <span>Branch</span>
                <select
                  value={switchBranchRef}
                  onChange={(e) => setSwitchBranchRef(e.target.value)}
                  style={selectStyle}
                  disabled={loadingBranches}
                >
                  {branches.map((branch) => (
                    <option key={branch.ref} value={branch.ref}>[{branch.kind}] {branch.name}</option>
                  ))}
                </select>
              </label>
              <div style={{ color: switchBranchError ? "#ff6b6b" : "var(--muted)", fontSize: 12, whiteSpace: "pre-wrap" }}>
                {switchBranchError ?? `Currently on ${switchBranchStream.branch}. Git will reject switches that conflict (dirty tree, missing branch, or branch already checked out in another worktree).`}
              </div>
              <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
                <button type="button" onClick={() => setSwitchBranchStream(null)} style={buttonStyle}>Cancel</button>
                <button type="submit" style={buttonStyle} disabled={switchBranchBusy || loadingBranches || !switchBranchRef}>
                  {switchBranchBusy ? "Switching…" : "Switch"}
                </button>
              </div>
            </div>
          </form>
        </div>
      ) : null}
      {settingsStream ? (
        <div style={backdropStyle}>
          <div style={modalStyle}>
            <div style={modalHeaderStyle}>
              <span>Stream settings — {settingsStream.title}</span>
              <button type="button" onClick={() => setSettingsStream(null)} style={closeBtnStyle} aria-label="Close">×</button>
            </div>
            <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <label style={labelStyle}>
                <span>Custom prompt</span>
                <span style={{ color: "var(--muted)", fontSize: 11 }}>
                  This prompt is appended to the agent's system prompt for this stream.
                </span>
                <textarea
                  value={settingsPrompt}
                  onChange={(e) => setSettingsPrompt(e.target.value)}
                  rows={6}
                  style={{ ...inputStyle, resize: "vertical", fontFamily: "inherit" }}
                  placeholder="Enter standing instructions for this stream…"
                />
              </label>
              <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
                <button type="button" onClick={() => setSettingsStream(null)} style={buttonStyle}>Cancel</button>
                <button type="button" onClick={() => void saveStreamSettings()} style={buttonStyle} disabled={settingsSaving}>
                  {settingsSaving ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
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
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 24px 60px rgba(0,0,0,0.5)",
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

const pickerButtonStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "6px 8px",
  borderRadius: 4,
  fontFamily: "inherit",
  fontSize: 12,
  minWidth: 220,
  justifyContent: "flex-start",
  height: "auto",
};

function resolvePickedRef(target: PickedRef): { ref: string; label: string } {
  if (target.kind === "tag") {
    return { ref: `refs/tags/${target.name}`, label: `tag: ${target.name}` };
  }
  const branch = target.branch;
  if (!branch) return { ref: "", label: target.name };
  return { ref: branch.ref, label: `[${branch.kind}] ${branch.name}` };
}
