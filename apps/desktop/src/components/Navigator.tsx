import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { archiveStream, type AgentKind, type Stream, type Thread, type ThreadState } from "../api.js";
import { agentLabel } from "../agentKinds.js";
import { subscribeNewThreadRequests } from "../new-thread-bus.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { Kebab } from "./Kebab.js";
import type { MenuItem } from "../menu.js";
import { Slideover } from "./Slideover.js";
import { titleInitials } from "../initials.js";

interface NavigatorProps {
  streams: Stream[];
  currentStreamId: string | null;
  threadStates: Record<string, ThreadState>;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  enabledAgents: AgentKind[];
  onSwitchStream(id: string): void | Promise<void>;
  onSelectThread(streamId: string, threadId: string): void | Promise<void>;
  onCreateThread(streamId: string, title: string, agent?: AgentKind): Promise<void>;
  onOpenNewStreamPage?(): void;
  onRenameStream?(streamId: string, title: string): void | Promise<void>;
  onRenameThread?(threadId: string, title: string): void | Promise<void>;
  onPromoteThread?(threadId: string): void | Promise<void>;
  onCloseThread?(threadId: string): void | Promise<void>;
  onOpenStreamSettings?(streamId: string): void;
  onOpenThreadSettings?(threadId: string): void;
  gitEnabled: boolean;
}

type RenameTarget = { kind: "stream" | "thread"; id: string };

/**
 * Combined stream + thread navigator.
 *
 * Layout: a thin always-visible vertical strip on the left holding a
 * letter glyph per stream and per thread. Clicking a glyph navigates
 * directly. Hovering the strip slides an overlay panel out to the
 * right that re-renders the same rows with full titles — y-positions
 * are identical between the strip and the overlay so items don't move
 * when switching modes. The overlay closes when the pointer leaves
 * the wrapper (with a short grace delay), on Escape, or on any
 * pointerdown outside the overlay. That last rule matters: the expanded
 * overlay covers the rail HUD to its right, and because it's part of the
 * wrapper's DOM subtree the `mouseleave` path can't fire while the
 * pointer sits over the rail-covered region — so an outward pointerdown
 * is what frees a click meant for the rail beneath it (tsk131).
 *
 * Visual hierarchy:
 *   - Stream rows: two-letter glyph, weight 700, with a subtle
 *     underline (border-bottom) so they read as section headers for the
 *     threads beneath them.
 *   - Thread rows: two-letter glyph, weight 600, slightly indented in
 *     the overlay; in the strip they share the same column.
 *   - The active stream's writer thread renders its letter inside an
 *     accent-soft-bg pill with an accent border. Activity dot is
 *     overlaid in the top-right corner of the icon cell — same
 *     position in strip and overlay.
 *   - Selection treatment: accent-soft-bg row background + 3px accent
 *     left stripe.
 *   - 8px gap separates the last thread of one stream from the next
 *     stream's row.
 */
export function Navigator({
  streams,
  currentStreamId,
  threadStates,
  streamStatuses,
  agentStatuses,
  enabledAgents,
  onSwitchStream,
  onSelectThread,
  onCreateThread,
  onOpenNewStreamPage,
  onRenameStream,
  onRenameThread,
  onPromoteThread,
  onCloseThread,
  onOpenStreamSettings,
  onOpenThreadSettings,
  gitEnabled,
}: NavigatorProps) {
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [pendingNewThreadFor, setPendingNewThreadFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<RenameTarget | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  // Remove-stream confirm flow
  const [removeStream, setRemoveStream] = useState<Stream | null>(null);
  const [removeWorktree, setRemoveWorktree] = useState(false);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);

  // Command-palette "New Thread…" (and any other external surface)
  // requests land here: open the overlay and show the inline creator
  // for the requested stream — the same flow as the kebab menu item.
  useEffect(
    () =>
      subscribeNewThreadRequests((streamId) => {
        setOverlayOpen(true);
        setPendingNewThreadFor(streamId);
      }),
    [],
  );

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

  // Hover-to-open behavior: opening on mouse-enter to the strip,
  // closing when the pointer leaves the whole nav (strip + overlay).
  // A short delay on close keeps the overlay open across small gaps
  // (e.g., when crossing into the kebab menu portal) and gives users
  // a moment to slide back if they overshoot.
  const cancelClose = () => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  };
  const scheduleClose = () => {
    cancelClose();
    closeTimerRef.current = window.setTimeout(() => {
      // Don't snap shut while the user is mid-rename or mid-new-thread —
      // they're committed to an action inside the overlay.
      setOverlayOpen(false);
      closeTimerRef.current = null;
    }, 180);
  };
  useEffect(() => () => cancelClose(), []);

  const orderedStreams = useMemo(() => {
    return streams.slice().sort((a, b) => {
      if (a.kind === "primary" && b.kind !== "primary") return -1;
      if (b.kind === "primary" && a.kind !== "primary") return 1;
      return 0;
    });
  }, [streams]);

  // Dismissal while the overlay is open. `mouseleave` on the wrapper is
  // NOT sufficient on its own: the expanded overlay is absolutely
  // positioned and ~280px wide, so it covers the rail HUD to its right.
  // Because the overlay is part of the wrapper's DOM subtree, the
  // pointer never "leaves" the wrapper while it's parked over the
  // rail-covered region — so the overlay lingers and swallows clicks
  // meant for the rail beneath it (tsk131). We therefore also dismiss on
  // any outward pointer interaction:
  //   - Escape, and
  //   - a pointerdown anywhere outside the overlay element. A press on
  //     the still-visible rail / center / tab bar collapses the overlay
  //     immediately, so the very next click lands on the rail instead of
  //     being intercepted. (Presses on the overlay's own interactive
  //     rows are inside `overlayRef` and keep working.)
  useEffect(() => {
    if (!overlayOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOverlayOpen(false);
    };
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      // The kebab popover (ContextMenu) renders inline as a descendant of
      // the overlay row, so `contains` already covers menu presses.
      if (target && overlayRef.current?.contains(target)) return;
      cancelClose();
      setOverlayOpen(false);
    };
    document.addEventListener("keydown", onKey);
    // Capture phase so we collapse before the press reaches (and is
    // consumed by) whatever is beneath the overlay.
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onPointerDown, true);
    };
  }, [overlayOpen]);

  const handleSelectThread = (streamId: string, threadId: string) => {
    // App.handleSelectThread switches the stream first when streamId
    // differs from the current one, so we don't dispatch onSwitchStream
    // separately here — doing both in parallel races their thread-state
    // writes and can leave the old thread selected.
    void onSelectThread(streamId, threadId);
    setOverlayOpen(false);
  };

  // Build the list of "rows" so the strip and overlay can both walk
  // the same sequence — guaranteeing matching y-positions row-by-row.
  // `add-thread` rows are flyout-only (skipped in the strip render).
  type Row =
    | { kind: "stream"; stream: Stream }
    | { kind: "thread"; stream: Stream; thread: Thread; isWriter: boolean }
    | { kind: "add-thread"; stream: Stream }
    | { kind: "gap"; afterStreamId: string };
  const rows: Row[] = useMemo(() => {
    const out: Row[] = [];
    for (const s of orderedStreams) {
      out.push({ kind: "stream", stream: s });
      const ts = threadStates[s.id];
      const writerId = ts?.activeThreadId ?? null;
      const threads = ts?.threads ?? [];
      for (const t of threads) {
        out.push({ kind: "thread", stream: s, thread: t, isWriter: t.id === writerId });
      }
      out.push({ kind: "add-thread", stream: s });
      out.push({ kind: "gap", afterStreamId: s.id });
    }
    return out;
  }, [orderedStreams, threadStates]);

  return (
    <div
      ref={overlayRef}
      onMouseEnter={() => {
        cancelClose();
        setOverlayOpen(true);
      }}
      onMouseLeave={scheduleClose}
      style={{
        position: "relative",
        display: "flex",
        height: "100%",
        flexShrink: 0,
      }}
    >
      {/* Always-visible strip */}
      <aside
        data-testid="navigator-strip"
        style={{
          width: STRIP_WIDTH,
          flexShrink: 0,
          height: "100%",
          // Darker than the HUD rail so the gutter reads as a
          // separate, recessed control surface — not the same
          // background as the pane to its right.
          background: "var(--surface-app)",
          borderRight: "1px solid var(--border-strong)",
          boxShadow: "inset -1px 0 0 rgba(0, 0, 0, 0.35)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          minHeight: 0,
        }}
      >
        <div
          style={{ flex: 1, overflowY: "auto", paddingTop: 0 }}
        >
          {rows.map((row, idx) => {
            if (row.kind === "gap") {
              return <div key={`gap-${row.afterStreamId}-${idx}`} style={{ height: GAP_HEIGHT }} />;
            }
            if (row.kind === "add-thread") {
              // The slot only takes vertical space when the inline
              // title input is rendered in the overlay; keep the
              // strip in lock-step so subsequent glyphs stay aligned.
              if (pendingNewThreadFor !== row.stream.id) return null;
              return (
                <div
                  key={`strip-addthread-${row.stream.id}`}
                  style={{ height: ADD_ROW_HEIGHT }}
                />
              );
            }
            if (row.kind === "stream") {
              return (
                <StripRow
                  key={`s-${row.stream.id}`}
                  letter={titleInitials(row.stream.title)}
                  isStream
                  isFirstStream={row.stream.id === orderedStreams[0]?.id}
                  isWriter={false}
                  selected={false}
                  status={undefined}
                />
              );
            }
            const isSelected =
              row.stream.id === currentStreamId &&
              threadStates[row.stream.id]?.selectedThreadId === row.thread.id;
            return (
              <StripRow
                key={`t-${row.thread.id}`}
                letter={titleInitials(row.thread.title)}
                isStream={false}
                isWriter={row.isWriter}
                selected={isSelected}
                status={agentStatuses[row.thread.id]}
                onClick={() => handleSelectThread(row.stream.id, row.thread.id)}
              />
            );
          })}
        </div>
      </aside>

      {/* Overlay panel — anchored at left:0 so it covers the strip
          rather than sitting beside it. The icon column inside each
          overlay row is sized identically to the strip's width, so the
          glyphs render in the same x-position before and after open. */}
      {overlayOpen ? (
        <div
          data-testid="navigator-overlay"
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            height: "100%",
            width: STRIP_WIDTH + OVERLAY_WIDTH,
            background: "var(--surface-rail)",
            borderRight: "1px solid var(--border-strong)",
            boxShadow: "8px 0 24px rgba(0, 0, 0, 0.45)",
            display: "flex",
            flexDirection: "column",
            zIndex: 30,
          }}
        >
          <div style={{ flex: 1, overflowY: "auto", paddingTop: 0 }}>
            {rows.map((row, idx) => {
              if (row.kind === "gap") {
                return <div key={`o-gap-${row.afterStreamId}-${idx}`} style={{ height: GAP_HEIGHT }} />;
              }
              if (row.kind === "stream") {
                const isPrimary = row.stream.kind === "primary";
                const isWorking = streamStatuses[row.stream.id] === "working";
                const streamMenu: MenuItem[] = [
                  {
                    id: "stream.add-thread",
                    label: "Add thread",
                    enabled: true,
                    run: () => setPendingNewThreadFor(row.stream.id),
                  },
                  {
                    id: "stream.rename",
                    label: "Rename…",
                    enabled: !!onRenameStream,
                    run: () => setRenaming({ kind: "stream", id: row.stream.id }),
                  },
                  {
                    id: "stream.settings",
                    label: "Settings…",
                    enabled: !!onOpenStreamSettings,
                    run: () => onOpenStreamSettings?.(row.stream.id),
                  },
                ];
                if (!isPrimary) {
                  streamMenu.push({
                    id: "stream.remove",
                    label: "Remove…",
                    // Disable when an agent is currently running in
                    // any of this stream's threads — the IPC also
                    // rejects, but disabling avoids a useless prompt.
                    enabled: !isWorking,
                    run: () => {
                      setRemoveStream(row.stream);
                      setRemoveWorktree(false);
                      setRemoveError(null);
                    },
                  });
                }
                return (
                  <OverlayRow
                    key={`o-s-${row.stream.id}`}
                    letter={titleInitials(row.stream.title)}
                    label={row.stream.title}
                    isStream
                    isFirstStream={row.stream.id === orderedStreams[0]?.id}
                    isWriter={false}
                    selected={false}
                    status={undefined}
                    renaming={renaming?.kind === "stream" && renaming.id === row.stream.id}
                    onCommitRename={async (next) => {
                      setRenaming(null);
                      if (next && next !== row.stream.title) {
                        await onRenameStream?.(row.stream.id, next);
                      }
                    }}
                    onCancelRename={() => setRenaming(null)}
                    menu={streamMenu}
                    menuTestId={`navigator-stream-kebab-${row.stream.id}`}
                  />
                );
              }
              if (row.kind === "add-thread") {
                // Only show the inline title input when the user has
                // explicitly chosen "Add thread" from the stream's
                // kebab. Otherwise the slot is empty in the overlay
                // (and the strip).
                if (pendingNewThreadFor !== row.stream.id) return null;
                return (
                  <InlineNewThread
                    key={`o-addinput-${row.stream.id}`}
                    enabledAgents={enabledAgents}
                    onSubmit={async (title, agent) => {
                      await onCreateThread(row.stream.id, title, agent);
                      setPendingNewThreadFor(null);
                    }}
                    onCancel={() => setPendingNewThreadFor(null)}
                  />
                );
              }
              const isSelected =
                row.stream.id === currentStreamId &&
                threadStates[row.stream.id]?.selectedThreadId === row.thread.id;
              const threadMenu: MenuItem[] = [
                {
                  id: "thread.rename",
                  label: "Rename…",
                  enabled: !!onRenameThread,
                  run: () => setRenaming({ kind: "thread", id: row.thread.id }),
                },
                {
                  id: "thread.promote",
                  label: row.isWriter ? "Already the writer" : "Make writer",
                  enabled: !!onPromoteThread && !row.isWriter,
                  run: () => onPromoteThread?.(row.thread.id),
                },
                {
                  id: "thread.settings",
                  label: "Settings…",
                  enabled: !!onOpenThreadSettings,
                  run: () => onOpenThreadSettings?.(row.thread.id),
                },
                {
                  id: "thread.close",
                  label: "Close thread",
                  enabled: !!onCloseThread,
                  run: () => onCloseThread?.(row.thread.id),
                },
              ];
              return (
                <OverlayRow
                  key={`o-t-${row.thread.id}`}
                  letter={titleInitials(row.thread.title)}
                  label={row.thread.title}
                  isStream={false}
                  isWriter={row.isWriter}
                  selected={isSelected}
                  status={agentStatuses[row.thread.id]}
                  onClick={() => handleSelectThread(row.stream.id, row.thread.id)}
                  renaming={renaming?.kind === "thread" && renaming.id === row.thread.id}
                  onCommitRename={async (next) => {
                    setRenaming(null);
                    if (next && next !== row.thread.title) {
                      await onRenameThread?.(row.thread.id, next);
                    }
                  }}
                  onCancelRename={() => setRenaming(null)}
                  menu={threadMenu}
                  menuTestId={`navigator-thread-kebab-${row.thread.id}`}
                />
              );
            })}
            <AddStreamButton
              gitEnabled={gitEnabled && !!onOpenNewStreamPage}
              onClick={() => onOpenNewStreamPage?.()}
            />
          </div>
        </div>
      ) : null}
      <Slideover
        open={!!removeStream}
        onClose={() => { if (!removeBusy) setRemoveStream(null); }}
        title={removeStream ? `Remove stream — ${removeStream.title}` : "Remove stream"}
        testId="stream-remove-slideover"
        footer={(
          <>
            <button
              type="button"
              onClick={() => setRemoveStream(null)}
              disabled={removeBusy}
              style={{
                background: "var(--surface-card)",
                color: "var(--text-primary)",
                border: "1px solid var(--border-subtle)",
                padding: "6px 12px",
                borderRadius: 6,
                cursor: removeBusy ? "not-allowed" : "pointer",
                fontFamily: "inherit",
                fontSize: "var(--text-xs)",
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              data-testid="stream-remove-confirm"
              onClick={() => { void handleRemoveStream(); }}
              disabled={removeBusy}
              style={{
                background: "#b32a2a",
                color: "#fff",
                border: "1px solid transparent",
                padding: "6px 12px",
                borderRadius: 6,
                cursor: removeBusy ? "not-allowed" : "pointer",
                fontFamily: "inherit",
                fontSize: "var(--text-xs)",
                fontWeight: "var(--weight-medium)",
              }}
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
    </div>
  );
}

/** Single row inside the strip — letter + status, fixed height.
 *  No tooltip: hovering the strip pops the overlay open, which shows
 *  the full title in-line, so a hover label here would be redundant. */
function StripRow({
  letter,
  isStream,
  isFirstStream = false,
  isWriter,
  selected,
  status,
  onClick,
}: {
  letter: string;
  isStream: boolean;
  isFirstStream?: boolean;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  onClick?(): void;
}) {
  const interactive = !!onClick;
  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={
        interactive
          ? (e) => {
              e.stopPropagation();
              onClick!();
            }
          : undefined
      }
      onKeyDown={
        interactive
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                onClick!();
              }
            }
          : undefined
      }
      style={{
        height: ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor: interactive ? "pointer" : "default",
        background: selected
          ? "var(--accent-soft-bg)"
          : isStream
            ? "var(--surface-app)"
            : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        borderTop: isStream && !isFirstStream
          ? "2px solid var(--border-strong)"
          : "1px solid transparent",
        borderBottom: "1px solid transparent",
        position: "relative",
        transition: "background 120ms ease",
      }}
    >
      <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} />
    </div>
  );
}

/** Row inside the slide-over overlay — same row height + letter cell
 *  as the strip, plus the full title to the right. */
function OverlayRow({
  letter,
  label,
  isStream,
  isFirstStream = false,
  isWriter,
  selected,
  status,
  onClick,
  renaming = false,
  onCommitRename,
  onCancelRename,
  menu,
  menuTestId,
}: {
  letter: string;
  label: string;
  isStream: boolean;
  isFirstStream?: boolean;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  onClick?(): void;
  renaming?: boolean;
  onCommitRename?(next: string): void | Promise<void>;
  onCancelRename?(): void;
  menu?: MenuItem[];
  menuTestId?: string;
}) {
  const interactive = !!onClick && !renaming;
  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={
        interactive
          ? (e) => {
              // Suppress row navigation when the click originated inside the
              // kebab — the menu manages its own click flow.
              if ((e.target as HTMLElement).closest("[data-navigator-row-kebab]")) return;
              onClick!();
            }
          : undefined
      }
      onKeyDown={
        interactive
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onClick!();
              }
            }
          : undefined
      }
      title={label}
      style={{
        height: ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        gap: 8,
        cursor: interactive ? "pointer" : "default",
        background: selected
          ? "var(--accent-soft-bg)"
          : isStream
            ? "var(--surface-app)"
            : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        borderTop: isStream && !isFirstStream
          ? "2px solid var(--border-strong)"
          : "1px solid transparent",
        borderBottom: "1px solid transparent",
        paddingRight: 6,
        transition: "background 120ms ease",
      }}
    >
      <div style={{ width: STRIP_WIDTH - 3 /* keep the icon column the same width as the strip */, display: "flex", justifyContent: "center" }}>
        <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} />
      </div>
      {renaming ? (
        <RenameInput
          initial={label}
          paddingLeft={0}
          onCommit={(next) => onCommitRename?.(next)}
          onCancel={() => onCancelRename?.()}
        />
      ) : (
        <span
          style={{
            flex: 1,
            fontSize: "var(--text-sm)",
            fontWeight: isStream ? 700 : 400,
            color: isStream
              ? "var(--text-primary)"
              : selected
                ? "var(--text-primary)"
                : "var(--text-secondary)",
            paddingLeft: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </span>
      )}
      {menu && !renaming ? (
        <span data-navigator-row-kebab style={{ flexShrink: 0 }}>
          <Kebab items={menu} testId={menuTestId} size={14} />
        </span>
      ) : null}
    </div>
  );
}

function RenameInput({
  initial,
  paddingLeft,
  onCommit,
  onCancel,
}: {
  initial: string;
  paddingLeft: number;
  onCommit(next: string): void | Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <input
      autoFocus
      value={value}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => {
        const next = value.trim();
        if (!next || next === initial) onCancel();
        else void onCommit(next);
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          const next = value.trim();
          if (!next || next === initial) onCancel();
          else void onCommit(next);
        } else if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
      style={{
        flex: 1,
        background: "var(--surface-card)",
        color: "var(--text-primary)",
        border: "1px solid var(--accent)",
        borderRadius: 4,
        padding: "3px 6px",
        fontSize: "var(--text-sm)",
        marginLeft: paddingLeft,
      }}
    />
  );
}

/** The shared icon cell that appears in both strip and overlay. The
 *  letter is centered in a 22px box; for writer threads the box is
 *  filled with `--accent-soft-bg` and bordered with `--accent`. The
 *  activity dot is absolutely positioned in the top-right corner so
 *  its location is identical regardless of writer/non-writer. */
function IconCell({
  letter,
  isStream,
  isWriter,
  status,
}: {
  letter: string;
  isStream: boolean;
  isWriter: boolean;
  status: AgentStatusDotState | undefined;
}) {
  // Writer pill: a soft, dim accent wash + translucent ring instead
  // of the full --accent-soft-bg + --accent treatment, so "writer"
  // still reads as a deliberate state without dominating the icon.
  const writerStyles: CSSProperties = isWriter
    ? {
        background: "rgba(107, 156, 246, 0.08)",
        border: "1px solid rgba(107, 156, 246, 0.35)",
      }
    : {
        background: "transparent",
        border: "1px solid transparent",
      };
  return (
    <span
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: ICON_BOX,
        height: ICON_BOX,
        borderRadius: 6,
        fontSize: LETTER_FONT,
        lineHeight: 1,
        fontWeight: isStream ? 700 : 600,
        color: "var(--text-primary)",
        ...writerStyles,
      }}
    >
      {letter}
      {/* Threads always show the agent's activity indicator. Mirrors
          the fallback the Agent tab uses (App.tsx — `agentStatuses[id]
          ?? "waiting"`) so a never-attached thread still reads as the
          same red "waiting" dot here as it does on the tab. Stream
          rows omit the dot entirely. */}
      {!isStream ? (
        <span
          style={{
            position: "absolute",
            top: -2,
            right: -2,
          }}
        >
          <AgentStatusDot status={status ?? "waiting"} size={8} />
        </span>
      ) : null}
    </span>
  );
}

function AddStreamButton({ gitEnabled, onClick }: { gitEnabled: boolean; onClick(): void }) {
  return (
    <div
      style={{
        padding: "12px 12px 8px",
        marginTop: GAP_HEIGHT,
        borderTop: "1px solid var(--border-subtle)",
      }}
    >
      <button
        type="button"
        data-testid="navigator-new-stream"
        onClick={() => { if (gitEnabled) onClick(); }}
        disabled={!gitEnabled}
        title={gitEnabled ? "Create a new stream" : "Disabled: workspace root is not its own git repo"}
        style={{
          width: "100%",
          textAlign: "center",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6,
          padding: "6px 10px",
          cursor: gitEnabled ? "pointer" : "not-allowed",
          fontFamily: "inherit",
          fontSize: "var(--text-xs)",
          fontWeight: "var(--weight-medium)",
          letterSpacing: 0.2,
          opacity: gitEnabled ? 1 : 0.5,
        }}
      >
        + Add stream
      </button>
    </div>
  );
}

function InlineNewThread({
  enabledAgents,
  onSubmit,
  onCancel,
}: {
  enabledAgents: AgentKind[];
  onSubmit(title: string, agent?: AgentKind): Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState("");
  const [agent, setAgent] = useState<AgentKind>(enabledAgents[0] ?? "claude");
  const [busy, setBusy] = useState(false);
  const choices = enabledAgents.length > 0 ? enabledAgents : ["claude" as AgentKind];
  return (
    <form
      onSubmit={async (e) => {
        e.preventDefault();
        const t = value.trim();
        if (!t) return onCancel();
        setBusy(true);
        try {
          await onSubmit(t, agent);
        } finally {
          setBusy(false);
        }
      }}
      style={{
        height: ADD_ROW_HEIGHT,
        padding: "4px 12px",
        display: "flex",
        alignItems: "center",
        gap: 6,
      }}
    >
      <input
        autoFocus
        value={value}
        disabled={busy}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => {
          const t = value.trim();
          if (!t) onCancel();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        placeholder="New thread title"
        style={{
          width: "100%",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 4,
          padding: "4px 6px",
          fontSize: "var(--text-xs)",
        }}
      />
      {choices.length > 1 ? (
        <select
          value={agent}
          disabled={busy}
          onChange={(e) => setAgent(e.target.value as AgentKind)}
          style={{
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 4,
            padding: "4px 6px",
            fontSize: "var(--text-xs)",
          }}
        >
          {choices.map((choice) => (
            <option key={choice} value={choice}>
              {agentLabel(choice)}
            </option>
          ))}
        </select>
      ) : null}
    </form>
  );
}

// Glyph sizing. The two-letter glyph runs a touch above body text
// (15px vs 14px) so the strip still reads as a real navigation
// indicator. Strip width and row height stay proportional so the icon
// cell sits comfortably with breathing room on both sides.
const LETTER_FONT = 15;
const ICON_BOX = 30;
const STRIP_WIDTH = 40;
const STRIP_PADDING_Y = 6;
const ROW_HEIGHT = 36;
const GAP_HEIGHT = 14;
// Height reserved (strip) and matched (overlay) for the per-stream
// "+ New thread" row so subsequent items stay y-aligned across the
// two views. Must match the rendered AddThreadRow height.
const ADD_ROW_HEIGHT = 40;
const OVERLAY_WIDTH = 240;
