import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import { archiveStream, checkoutStreamBranch, listBranches, type BranchRef, type Stream } from "../api.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { useContextMenu } from "./useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { TASK_DRAG_MIME, THREAD_DRAG_MIME } from "./ThreadRail.js";
import { Slideover } from "./Slideover.js";

interface Props {
  stream: Stream | null;
  streams: Stream[];
  streamStatuses: Record<string, AgentStatusDotState>;
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
  onDroptasksOnStream?(targetStreamId: string, itemId: number, fromThreadId: string | null): void;
  onReorderStreams?(orderedStreamIds: string[]): Promise<void> | void;
  /** Bumping this number opens the New-stream page via
   *  `onOpenNewStreamPage`. The legacy in-rail modal was retired in
   *  the IA redesign; this prop now does nothing without that handler. */
  createRequest?: number;
}

export const STREAM_DRAG_MIME = "application/x-oxplow-stream";

export function StreamRail({ stream, streams, streamStatuses, streamActiveThreadIds, gitEnabled, onSwitch, onRenameStream, onRequestCreateThread, onOpenStreamSettings, onOpenNewStreamPage, onDroptasksOnStream, onReorderStreams, createRequest }: Props) {
  const cm = useContextMenu();
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
  // Remove flow state
  const [removeStream, setRemoveStream] = useState<Stream | null>(null);
  const [removeWorktree, setRemoveWorktree] = useState(false);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);

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

  function openRemoveStream(target: Stream) {
    setRemoveStream(target);
    setRemoveWorktree(false);
    setRemoveError(null);
  }

  async function handleRemoveStream() {
    if (!removeStream) return;
    try {
      setRemoveBusy(true);
      setRemoveError(null);
      await archiveStream(removeStream.id, removeWorktree);
      setRemoveStream(null);
    } catch (e) {
      setRemoveError(String(e));
    } finally {
      setRemoveBusy(false);
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
    <div style={{ display: "flex", flexDirection: "column", background: "var(--surface-rail)", borderBottom: "1px solid var(--border-subtle)" }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 0, padding: "0 12px", overflow: "hidden" }}>
        <div className="oxplow-rail-scroll" style={{ display: "flex", gap: 2, overflowX: "auto", flex: 1, minWidth: 0, alignItems: "flex-end" }}>
          {orderedStreams.map((candidate) => {
            const active = candidate.id === stream?.id;
            const status = streamStatuses[candidate.id] ?? "waiting";
            const canDrop = !!onDroptasksOnStream && !!streamActiveThreadIds?.[candidate.id];
            const isDragOver = dragOverStreamId === candidate.id;
            const isStreamDragTarget = isDragOver && draggingStreamId !== null && draggingStreamId !== candidate.id;
            const isPrimary = candidate.kind === "primary";
            const canDrag = !!onReorderStreams && !isPrimary;
            const showBranchInTitle = isPrimary || candidate.title !== candidate.branch;
            const streamMenu = buildStreamMenu(candidate, {
              gitEnabled,
              isPrimary,
              isWorking: status === "working",
              onRename: () => {
                if (!onRenameStream) return;
                setRenamingId(candidate.id);
              },
              onSwitchBranch: () => { void openSwitchBranch(candidate); },
              onSettings: () => onOpenStreamSettings?.(candidate.id),
              onAddStream: () => onOpenNewStreamPage?.(),
              onAddThread: () => onRequestCreateThread?.(),
              onRemove: () => openRemoveStream(candidate),
              canRename: !!onRenameStream,
              canSettings: !!onOpenStreamSettings,
              canAddThread: !!onRequestCreateThread,
            });
            return (
              <div
                role="button"
                tabIndex={0}
                key={candidate.id}
                draggable={canDrag}
                onClick={() => onSwitch(candidate.id)}
                onContextMenu={(e) => cm.open(e, streamMenu)}
                onKeyDown={(e) => {
                  cm.openForKey(e, streamMenu);
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
                  if (!types.includes(TASK_DRAG_MIME) && !types.includes(THREAD_DRAG_MIME)) return;
                  if (!types.includes(TASK_DRAG_MIME)) return;
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
                  const raw = event.dataTransfer.getData(TASK_DRAG_MIME);
                  if (!raw) return;
                  event.preventDefault();
                  setDragOverStreamId(null);
                  try {
                    const payload = JSON.parse(raw) as {
                      itemId?: number;
                      itemIds?: number[];
                      fromThreadId?: string | null;
                    };
                    const ids = payload.itemIds && payload.itemIds.length > 0
                      ? payload.itemIds
                      : payload.itemId ? [payload.itemId] : [];
                    for (const id of ids) {
                      onDroptasksOnStream?.(candidate.id, id, payload.fromThreadId ?? null);
                    }
                  } catch {
                    // ignore malformed payload
                  }
                }}
                style={{
                  ...tabStyle,
                  background: active ? "var(--surface-tab-active)" : "transparent",
                  color: active ? "var(--text-primary)" : "var(--text-secondary)",
                  fontWeight: active ? 600 : 500,
                  borderBottom: active
                    ? "2px solid var(--accent)"
                    : "2px solid transparent",
                  borderLeft: isStreamDragTarget ? "2px solid var(--accent)" : undefined,
                  boxShadow: isDragOver && !isStreamDragTarget ? "inset 0 0 0 2px var(--accent)" : undefined,
                  transition: "background 120ms ease, color 120ms ease",
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
              </div>
            );
          })}
        </div>
        {cm.menu}
        <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, padding: "6px 0 6px 8px" }}>
          <button type="button"
            onClick={() => onOpenNewStreamPage?.()}
            title={gitEnabled ? "Create a new stream" : "Disabled: this workspace root does not contain its own .git directory"}
            style={{ ...primaryButtonStyle, opacity: gitEnabled && onOpenNewStreamPage ? 1 : 0.5, cursor: gitEnabled && onOpenNewStreamPage ? "pointer" : "not-allowed" }}
            disabled={!gitEnabled || !onOpenNewStreamPage}
          >
            + New stream
          </button>
        </span>
      </div>
      <Slideover
        open={!!removeStream}
        onClose={() => { if (!removeBusy) setRemoveStream(null); }}
        title={removeStream ? `Remove stream — ${removeStream.title}` : "Remove stream"}
        testId="stream-remove-slideover"
        footer={(
          <>
            <button type="button" onClick={() => setRemoveStream(null)} style={buttonStyle} disabled={removeBusy}>Cancel</button>
            <button
              type="button"
              data-testid="stream-remove-confirm"
              onClick={() => { void handleRemoveStream(); }}
              style={{ ...primaryButtonStyle, background: "#b32a2a" }}
              disabled={removeBusy}
            >
              {removeBusy ? "Removing…" : "Remove"}
            </button>
          </>
        )}
      >
        {removeStream ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 12, fontSize: "var(--text-xs)" }}>
            <p style={{ margin: 0, color: "var(--text-secondary)", lineHeight: 1.5 }}>
              The stream and every thread under it will be archived (hidden from the rail). History — closed efforts, snapshots, and visit logs — stays intact.
            </p>
            <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <input
                type="checkbox"
                data-testid="stream-remove-delete-worktree"
                checked={removeWorktree}
                onChange={(e) => setRemoveWorktree(e.target.checked)}
                disabled={removeBusy || removeStream.kind !== "worktree"}
              />
              <span>
                Also delete the on-disk worktree
                {removeStream.kind === "worktree" ? (
                  <span style={{ color: "var(--text-secondary)" }}> ({removeStream.worktree_path})</span>
                ) : (
                  <span style={{ color: "var(--text-secondary)" }}> — primary stream has no worktree to delete</span>
                )}
              </span>
            </label>
            {removeError ? (
              <div style={{ color: "#ff6b6b", whiteSpace: "pre-wrap" }}>{removeError}</div>
            ) : null}
          </div>
        ) : null}
      </Slideover>
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
            <div style={{ color: switchBranchError ? "#ff6b6b" : "var(--muted)", fontSize: "var(--text-xs)", whiteSpace: "pre-wrap" }}>
              {switchBranchError ?? `Currently on ${switchBranchStream.branch}. Git will reject switches that conflict (dirty tree, missing branch, or branch already checked out in another worktree).`}
            </div>
          </form>
        ) : null}
      </Slideover>
    </div>
  );
}

const buttonStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "6px 12px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
};

const primaryButtonStyle: CSSProperties = {
  background: "var(--button-primary-bg)",
  color: "var(--button-primary-fg)",
  border: "1px solid transparent",
  padding: "6px 12px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  fontWeight: "var(--weight-medium)",
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

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-xs)" };

const renameInputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "2px 6px",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  minWidth: 140,
};

function buildStreamMenu(_stream: Stream, opts: {
  gitEnabled: boolean;
  isPrimary: boolean;
  isWorking: boolean;
  onRename(): void;
  onSwitchBranch(): void;
  onSettings(): void;
  onAddStream(): void;
  onAddThread(): void;
  onRemove(): void;
  canRename: boolean;
  canSettings: boolean;
  canAddThread: boolean;
}): MenuItem[] {
  const items: MenuItem[] = [
    { id: "stream.rename", label: "Rename…", enabled: opts.canRename, run: opts.onRename },
    { id: "stream.switch-branch", label: "Switch branch…", enabled: opts.gitEnabled, run: opts.onSwitchBranch },
    { id: "stream.settings", label: "Settings", enabled: opts.canSettings, run: opts.onSettings },
    { id: "stream.add-stream", label: "Add stream", enabled: opts.gitEnabled, run: opts.onAddStream },
    { id: "stream.add-thread", label: "Add thread", enabled: opts.canAddThread, run: opts.onAddThread },
  ];
  if (!opts.isPrimary) {
    items.push({
      id: "stream.remove",
      label: "Remove…",
      // Disable while any thread in this stream has a running agent —
      // the IPC also rejects, but disabling avoids a useless prompt.
      enabled: !opts.isWorking,
      run: opts.onRemove,
    });
  }
  return items;
}

