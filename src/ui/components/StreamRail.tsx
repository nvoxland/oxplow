import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import { checkoutStreamBranch, listBranches, type AgentStatus, type BranchRef, type Stream } from "../api.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { Kebab } from "./Kebab.js";
import type { MenuItem } from "../menu.js";
import { WORK_ITEM_DRAG_MIME, THREAD_DRAG_MIME } from "./ThreadRail.js";
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
  /** Open the per-stream settings page. The kebab "Settings" item
   *  routes here. */
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

export function StreamRail({ stream, streams, streamStatuses, streamActiveThreadIds, gitEnabled, onSwitch, onRenameStream, onRequestCreateThread, onOpenStreamSettings, onOpenNewStreamPage, onDropWorkItemOnStream, onReorderStreams, createRequest }: Props) {
  const [dragOverStreamId, setDragOverStreamId] = useState<string | null>(null);
  const [draggingStreamId, setDraggingStreamId] = useState<string | null>(null);
  // Inline rename state — set to a stream id to swap the tab title for an
  // input. Mirrors the thread-chip rename pattern (Enter commits, Escape
  // reverts, blur commits).
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [switchBranchStream, setSwitchBranchStream] = useState<Stream | null>(null);
  const [switchBranchRef, setSwitchBranchRef] = useState<string>("");
  const [switchBranchError, setSwitchBranchError] = useState<string | null>(null);
  const [switchBranchBusy, setSwitchBranchBusy] = useState(false);

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
                {renamingId === candidate.id ? (
                  <input
                    autoFocus
                    defaultValue={candidate.title}
                    data-testid="stream-rename-input"
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Enter") {
                        const next = (e.target as HTMLInputElement).value.trim();
                        setRenamingId(null);
                        if (next && next !== candidate.title) {
                          void onRenameStream?.(candidate.id, next);
                        }
                      } else if (e.key === "Escape") {
                        setRenamingId(null);
                      }
                    }}
                    onBlur={(e) => {
                      const next = e.target.value.trim();
                      setRenamingId(null);
                      if (next && next !== candidate.title) {
                        void onRenameStream?.(candidate.id, next);
                      }
                    }}
                    style={renameInputStyle}
                  />
                ) : (
                  <span>{candidate.title}</span>
                )}
                <Kebab
                  items={buildStreamMenu(candidate, {
                    gitEnabled,
                    onRename: () => {
                      if (!onRenameStream) return;
                      setRenamingId(candidate.id);
                    },
                    onSwitchBranch: () => { void openSwitchBranch(candidate); },
                    onSettings: () => onOpenStreamSettings?.(candidate.id),
                    onAddStream: () => onOpenNewStreamPage?.(),
                    onAddThread: () => onRequestCreateThread?.(),
                    canRename: !!onRenameStream,
                    canSettings: !!onOpenStreamSettings,
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

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: 12 };

const renameInputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "2px 6px",
  fontFamily: "inherit",
  fontSize: 12,
  minWidth: 140,
};

function buildStreamMenu(_stream: Stream, opts: {
  gitEnabled: boolean;
  onRename(): void;
  onSwitchBranch(): void;
  onSettings(): void;
  onAddStream(): void;
  onAddThread(): void;
  canRename: boolean;
  canSettings: boolean;
  canAddThread: boolean;
}): MenuItem[] {
  return [
    { id: "stream.rename", label: "Rename…", enabled: opts.canRename, run: opts.onRename },
    { id: "stream.switch-branch", label: "Switch branch…", enabled: opts.gitEnabled, run: opts.onSwitchBranch },
    { id: "stream.settings", label: "Settings", enabled: opts.canSettings, run: opts.onSettings },
    { id: "stream.add-stream", label: "Add stream", enabled: opts.gitEnabled, run: opts.onAddStream },
    { id: "stream.add-thread", label: "Add thread", enabled: opts.canAddThread, run: opts.onAddThread },
  ];
}

