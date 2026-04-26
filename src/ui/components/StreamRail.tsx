import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { checkoutStreamBranch, listBranches, setStreamPrompt, type AgentStatus, type BranchRef, type Stream } from "../api.js";
import { logUi } from "../logger.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { Kebab } from "./Kebab.js";
import type { MenuItem } from "../menu.js";
import { WORK_ITEM_DRAG_MIME, THREAD_DRAG_MIME } from "./ThreadRail.js";
import { ThemeToggle } from "./ThemeToggle.js";
import { Slideover } from "./Slideover.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  streamStatuses: Record<string, AgentStatus>;
  streamActiveThreadIds?: Record<string, string | null>;
  gitEnabled: boolean;
  onSwitch(id: string): void;
  onRenameStream?(streamId: string, newTitle: string): Promise<void> | void;
  onRequestCreateThread?(): void;
  onOpenSettings?(): void;
  /** Open the per-stream settings page. When provided, the kebab
   *  "Settings" item routes here instead of the legacy modal. */
  onOpenStreamSettings?(streamId: string): void;
  /** Open the New-stream page (replaces the in-rail modal when wired). */
  onOpenNewStreamPage?(): void;
  onDropWorkItemOnStream?(targetStreamId: string, itemId: string, fromThreadId: string | null): void;
  onReorderStreams?(orderedStreamIds: string[]): Promise<void> | void;
  /** Bumping this number opens the New-stream page via
   *  `onOpenNewStreamPage`. The legacy in-rail modal was retired in
   *  the IA redesign; this prop now does nothing without that handler. */
  createRequest?: number;
}

export const STREAM_DRAG_MIME = "application/x-oxplow-stream";

export function StreamRail({ stream, streams, streamStatuses, streamActiveThreadIds, gitEnabled, onSwitch, onRenameStream, onRequestCreateThread, onOpenSettings, onOpenStreamSettings, onOpenNewStreamPage, onDropWorkItemOnStream, onReorderStreams, createRequest }: Props) {
  const [dragOverStreamId, setDragOverStreamId] = useState<string | null>(null);
  const [draggingStreamId, setDraggingStreamId] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameBusy, setRenameBusy] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  const [settingsStream, setSettingsStream] = useState<Stream | null>(null);
  const [settingsPrompt, setSettingsPrompt] = useState("");
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [switchBranchStream, setSwitchBranchStream] = useState<Stream | null>(null);
  const [switchBranchRef, setSwitchBranchRef] = useState<string>("");
  const [switchBranchError, setSwitchBranchError] = useState<string | null>(null);
  const [switchBranchBusy, setSwitchBranchBusy] = useState(false);

  function openStreamSettings(candidate: Stream) {
    if (onOpenStreamSettings) {
      onOpenStreamSettings(candidate.id);
      return;
    }
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

  // Primary is always the leftmost tab regardless of persisted sort_index.
  const orderedStreams = useMemo(() => {
    const primary = streams.filter((s) => s.kind === "primary");
    const rest = streams.filter((s) => s.kind !== "primary");
    return [...primary, ...rest];
  }, [streams]);

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    onOpenNewStreamPage?.();
  }, [createRequest, onOpenNewStreamPage]);

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
              <div
                role="button"
                tabIndex={0}
                key={candidate.id}
                draggable={canDrag}
                onClick={() => onSwitch(candidate.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSwitch(candidate.id);
                  }
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
                <Kebab
                  items={buildStreamMenu(candidate, {
                    gitEnabled,
                    onRename: () => {
                      if (!onRenameStream) return;
                      setRenaming({ id: candidate.id, title: candidate.title });
                      setRenameValue(candidate.title);
                      setRenameError(null);
                      setTimeout(() => renameInputRef.current?.select(), 0);
                    },
                    onSwitchBranch: () => { void openSwitchBranch(candidate); },
                    onSettings: () => openStreamSettings(candidate),
                    onAddStream: () => onOpenNewStreamPage?.(),
                    onAddThread: () => onRequestCreateThread?.(),
                    canRename: !!onRenameStream,
                    canAddThread: !!onRequestCreateThread,
                  })}
                  testId={`stream-tab-kebab-${candidate.id}`}
                  size={14}
                />
              </div>
            );
          })}
        </div>
        <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, padding: "6px 0 6px 8px" }}>
          <button type="button"
            onClick={() => onOpenNewStreamPage?.()}
            title={gitEnabled ? "Create a new stream" : "Disabled: this workspace root does not contain its own .git directory"}
            style={{ ...buttonStyle, opacity: gitEnabled && onOpenNewStreamPage ? 1 : 0.6, cursor: gitEnabled && onOpenNewStreamPage ? "pointer" : "not-allowed" }}
            disabled={!gitEnabled || !onOpenNewStreamPage}
          >
            + New stream
          </button>
          <ThemeToggle variant="compact" />
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
      <Slideover
        open={!!switchBranchStream}
        onClose={() => setSwitchBranchStream(null)}
        title={switchBranchStream ? `Switch branch — ${switchBranchStream.title}` : "Switch branch"}
        testId="stream-switch-branch-slideover"
        footer={(
          <>
            <button type="button" onClick={() => setSwitchBranchStream(null)} style={buttonStyle}>Cancel</button>
            <button
              type="button"
              onClick={() => { void handleSwitchBranch(); }}
              style={buttonStyle}
              disabled={switchBranchBusy || loadingBranches || !switchBranchRef}
            >
              {switchBranchBusy ? "Switching…" : "Switch"}
            </button>
          </>
        )}
      >
        {switchBranchStream ? (
          <form
            onSubmit={(e) => { e.preventDefault(); void handleSwitchBranch(); }}
            style={{ display: "flex", flexDirection: "column", gap: 10 }}
          >
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
          </form>
        ) : null}
      </Slideover>
      {renaming ? (
        <div style={backdropStyle}>
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              const next = renameValue.trim();
              if (!next || next === renaming.title) { setRenaming(null); return; }
              setRenameBusy(true);
              setRenameError(null);
              try {
                await onRenameStream?.(renaming.id, next);
                setRenaming(null);
              } catch (err) {
                setRenameError(String(err));
              } finally {
                setRenameBusy(false);
              }
            }}
            style={{ ...modalStyle, minWidth: 360 }}
          >
            <div style={modalHeaderStyle}>
              <span>Rename stream</span>
              <button type="button" onClick={() => setRenaming(null)} style={closeBtnStyle} aria-label="Close">×</button>
            </div>
            <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <label style={labelStyle}>
                <span>Title</span>
                <input
                  ref={renameInputRef}
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Escape") setRenaming(null); }}
                  style={inputStyle}
                  autoFocus
                  data-testid="stream-rename-input"
                />
              </label>
              {renameError ? <div style={{ color: "#ff6b6b", fontSize: 12 }}>{renameError}</div> : null}
              <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
                <button type="button" onClick={() => setRenaming(null)} style={buttonStyle}>Cancel</button>
                <button type="submit" style={buttonStyle} disabled={renameBusy || !renameValue.trim()}>
                  {renameBusy ? "Renaming…" : "Rename"}
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

function buildStreamMenu(_stream: Stream, opts: {
  gitEnabled: boolean;
  onRename(): void;
  onSwitchBranch(): void;
  onSettings(): void;
  onAddStream(): void;
  onAddThread(): void;
  canRename: boolean;
  canAddThread: boolean;
}): MenuItem[] {
  return [
    { id: "stream.rename", label: "Rename…", enabled: opts.canRename, run: opts.onRename },
    { id: "stream.switch-branch", label: "Switch branch…", enabled: opts.gitEnabled, run: opts.onSwitchBranch },
    { id: "stream.settings", label: "Settings", enabled: true, run: opts.onSettings },
    { id: "stream.add-stream", label: "Add stream", enabled: opts.gitEnabled, run: opts.onAddStream },
    { id: "stream.add-thread", label: "Add thread", enabled: opts.canAddThread, run: opts.onAddThread },
  ];
}

