import type { CSSProperties, ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useSlideoutStrip } from "./useSlideoutStrip.js";
import { SlideoutChevron } from "./SlideoutChevron.js";
import {
  subscribeNavigatorMenuRequests,
  subscribeNavigatorOpenRequests,
  type NavigatorMenuRequest,
} from "../navigator-bus.js";
import { archiveStream, type Stream, type Thread, type ThreadState } from "../api.js";
import type { AcpAgentListing } from "../tauri-bridge/generated/bindings.js";
import { subscribeNewThreadRequests } from "../new-thread-bus.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { useContextMenu, useRowContextMenu } from "./useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { Slideover } from "./Slideover.js";
import { titleInitials } from "../initials.js";

interface NavigatorProps {
  streams: Stream[];
  currentStreamId: string | null;
  threadStates: Record<string, ThreadState>;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  /// Per-thread question the agent is waiting on, shown as the rail dot's tooltip
  /// while that thread's status is "awaiting". Absent for every other
  /// state. Keyed by thread id, parallel to agentStatuses.
  agentQuestions?: Record<string, string | undefined>;
  onSwitchStream(id: string): void | Promise<void>;
  onSelectThread(streamId: string, threadId: string): void | Promise<void>;
  onCreateThread(streamId: string, title: string): Promise<void>;
  onOpenNewStreamPage?(): void;
  onRenameStream?(streamId: string, title: string): void | Promise<void>;
  onRenameThread?(threadId: string, title: string): void | Promise<void>;
  onPromoteThread?(threadId: string): void | Promise<void>;
  onCloseThread?(threadId: string): void | Promise<void>;
  onOpenStreamSettings?(streamId: string): void;
  onOpenThreadSettings?(threadId: string): void;
  vcsEnabled: boolean;
}

type RenameTarget = { kind: "stream" | "thread"; id: string };

/**
 * Combined stream + thread navigator.
 *
 * Layout: a thin always-visible vertical strip on the left holding a
 * letter glyph per stream and per thread. Clicking a glyph navigates
 * directly — a thread glyph selects that thread, a stream glyph switches
 * to that stream. Hovering a glyph shows its full title as a tooltip and
 * nothing else.
 *
 * The panel expands only on an explicit click: the bottom-pinned chevron,
 * dead space in the strip, a stream glyph (which also switches to it) or
 * the selected thread's glyph. It re-renders the same rows with full
 * titles — y-positions are identical between the strip and the panel so
 * items don't move when switching modes.
 *
 * Hover used to expand it, and that was the problem (tsk269): the panel
 * is ~280px wide over a rail HUD that starts at x=40, so it covers
 * essentially all of it, and a zero-dwell hover-open fired whenever the
 * pointer merely drifted left en route to that rail — burying the click
 * the user was lining up.
 *
 * It closes on a click in its own dead background, on the pointer
 * leaving its bounds (with a short grace delay), on Escape, and on any
 * pointerdown outside it. The last two are explicit dismissals and beat
 * the mid-rename / mid-new-thread guard; the first two don't. Pointer
 * departure is measured GEOMETRICALLY, not via `mouseleave`: the panel
 * covers the rail and lives in the same DOM subtree as the strip, so the
 * pointer never "leaves" the wrapper while parked over the covered
 * region — which is what used to strand it open on top of the rail and
 * swallow clicks meant for it (tsk131).
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
  agentQuestions,
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
  vcsEnabled,
}: NavigatorProps) {
  const [pendingNewThreadFor, setPendingNewThreadFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<RenameTarget | null>(null);

  // A rename or a new-thread entry is in flight: the user is committed to
  // an action inside the panel, so neither passive close path (pointer
  // drifting off, a click on dead background) may throw their typing away.
  // Only an explicit dismissal — Escape, the collapse chevron, a press
  // outside the nav entirely — gets through.
  const formActive = renaming !== null || pendingNewThreadFor !== null;

  // The whole open/close state machine — click-to-open, geometric
  // pointer-leave, Escape, outside-press, background-click, and the
  // passive-vs-explicit guard — lives in the shared hook. See
  // `useSlideoutStrip.ts` for why each rule is the way it is; the
  // Terminal page's tab strip runs on the same machine.
  const strip = useSlideoutStrip({ guard: formActive });
  const overlayOpen = strip.open;
  // Remove-stream confirm flow
  const [removeStream, setRemoveStream] = useState<Stream | null>(null);
  const [removeWorktree, setRemoveWorktree] = useState(false);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);

  // The title bar's stream name (and any other surface pointing here)
  // opens the panel.
  useEffect(() => subscribeNavigatorOpenRequests(() => strip.openPanel()), [strip]);

  // Command-palette "New Thread…" (and any other external surface)
  // requests land here: open the overlay and show the inline creator
  // for the requested stream — the same flow as the kebab menu item.
  useEffect(
    () =>
      subscribeNewThreadRequests((streamId) => {
        strip.openPanel();
        setPendingNewThreadFor(streamId);
      }),
    [strip],
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

  const orderedStreams = useMemo(() => {
    return streams.slice().sort((a, b) => {
      if (a.kind === "primary" && b.kind !== "primary") return -1;
      if (b.kind === "primary" && a.kind !== "primary") return 1;
      return 0;
    });
  }, [streams]);

  const handleSelectThread = (streamId: string, threadId: string) => {
    // App.handleSelectThread switches the stream first when streamId
    // differs from the current one, so we don't dispatch onSwitchStream
    // separately here — doing both in parallel races their thread-state
    // writes and can leave the old thread selected.
    void onSelectThread(streamId, threadId);
    strip.closePanel();
  };

  // Clicking a stream glyph switches to that stream — App restores that
  // stream's own selected thread. For the same race reason as above this
  // dispatches `onSwitchStream` ALONE; pairing it with onSelectThread
  // would have the two handlers fight over the thread-state write.
  const handleSwitchStream = (streamId: string) => {
    void onSwitchStream(streamId);
    strip.closePanel();
  };

  // In the collapsed strip a stream glyph heads its threads, so it
  // switches to the stream and opens the panel to show them; the selected
  // thread's glyph (selecting it again would do nothing) opens the panel,
  // as a click in the strip's empty space does.
  const handleStripStream = (streamId: string) => {
    void onSwitchStream(streamId);
    strip.openPanel();
  };
  const handleStripThread = (streamId: string, threadId: string, isSelected: boolean) => {
    if (isSelected) strip.openPanel();
    else handleSelectThread(streamId, threadId);
  };

  // One menu per stream and per thread, the same whether it's opened from
  // the strip's icon or the panel's row. Rename and Add thread work in the
  // panel (their inline fields live there), so they open it.
  const streamMenu = (s: Stream): MenuItem[] => {
    const items: MenuItem[] = [
      {
        id: "stream.add-thread",
        label: "Add thread",
        enabled: true,
        run: () => {
          strip.openPanel();
          setPendingNewThreadFor(s.id);
        },
      },
      {
        id: "stream.rename",
        label: "Rename…",
        enabled: !!onRenameStream,
        run: () => {
          strip.openPanel();
          setRenaming({ kind: "stream", id: s.id });
        },
      },
      {
        id: "stream.settings",
        label: "Settings…",
        enabled: !!onOpenStreamSettings,
        run: () => onOpenStreamSettings?.(s.id),
      },
    ];
    if (s.kind !== "primary") {
      items.push({
        id: "stream.remove",
        label: "Remove…",
        // Disable when an agent is currently running in any of this
        // stream's threads — the IPC also rejects, but disabling avoids a
        // useless prompt.
        enabled: streamStatuses[s.id] !== "working",
        run: () => {
          setRemoveStream(s);
          setRemoveWorktree(false);
          setRemoveError(null);
        },
      });
    }
    return items;
  };
  const threadMenu = (thread: Thread, isWriter: boolean): MenuItem[] => {
    const items: MenuItem[] = [];
    // "Make writer" is the headline action for a read-only thread: only
    // the stream's single active thread can edit files, so a queued thread
    // is "edits blocked" until promoted. Show it FIRST, and only when this
    // thread isn't already the writer (tsk132).
    if (!isWriter) {
      items.push({
        id: "thread.promote",
        label: "Make writer",
        enabled: !!onPromoteThread,
        run: () => onPromoteThread?.(thread.id),
      });
    }
    items.push(
      {
        id: "thread.rename",
        label: "Rename…",
        enabled: !!onRenameThread,
        run: () => {
          strip.openPanel();
          setRenaming({ kind: "thread", id: thread.id });
        },
      },
      {
        id: "thread.settings",
        label: "Settings…",
        enabled: !!onOpenThreadSettings,
        run: () => onOpenThreadSettings?.(thread.id),
      },
      {
        id: "thread.close",
        label: "Close thread",
        enabled: !!onCloseThread,
        run: () => onCloseThread?.(thread.id),
      },
    );
    return items;
  };

  // Build the list of "rows" so the strip and overlay can both walk
  // the same sequence — guaranteeing matching y-positions row-by-row.
  // `add-thread` rows are flyout-only (skipped in the strip render).
  // Each stream + its threads renders as one panel (a surface-card box,
  // rounded on the right, flush on the left), with a gap between groups —
  // mirroring the main rail's panel look. The strip and the slide-over
  // overlay both map this same structure so glyph y-positions stay in
  // lock-step when the overlay opens.
  // A stream's or thread's menu asked for from outside (the title bar's
  // names): the same menu, headed the same way, at the asker's point.
  const requested = useContextMenu();
  const menuFor = (r: NavigatorMenuRequest): { items: MenuItem[]; header: string } | null => {
    if (r.kind === "stream") {
      const s = streams.find((x) => x.id === r.id);
      return s ? { items: streamMenu(s), header: s.title } : null;
    }
    for (const [streamId, ts] of Object.entries(threadStates)) {
      const t = ts.threads.find((x) => x.id === r.id);
      if (t) return { items: threadMenu(t, threadStates[streamId]?.activeThreadId === t.id), header: t.title };
    }
    return null;
  };
  const menuForRef = useRef(menuFor);
  menuForRef.current = menuFor;
  const openAt = requested.openAt;
  useEffect(
    () =>
      subscribeNavigatorMenuRequests((r) => {
        const menu = menuForRef.current(r);
        if (menu) openAt({ x: r.x, y: r.y }, menu.items, menu.header);
      }),
    [openAt],
  );

  const streamGroups = useMemo(
    () =>
      orderedStreams.map((s) => {
        const ts = threadStates[s.id];
        const writerId = ts?.activeThreadId ?? null;
        return {
          stream: s,
          threads: (ts?.threads ?? []).map((t) => ({ thread: t, isWriter: t.id === writerId })),
        };
      }),
    [orderedStreams, threadStates],
  );

  return (
    <div
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
          // Part of the lighter chrome frame, like the HUD rail to its
          // right; its own edge is where the tabs end, so they don't read
          // as cut off against the rail.
          background: "var(--surface-chrome)",
          borderRightWidth: 1,
          borderRightStyle: "solid",
          // The same 1px line that frames the content area.
          borderRightColor: "var(--border-strong)",
          boxSizing: "border-box",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          minHeight: 0,
        }}
      >
        <div
          data-testid="navigator-strip-empty"
          // Dead space in the strip — below the last panel, and the gaps
          // between panels — is a bonus way to expand. It can't be the
          // only one: once the list scrolls there is no dead space left,
          // and `+ Add stream` lives only in the panel. The chevron below
          // is the affordance that's always there.
          {...strip.deadSpaceProps}
          style={{ flex: 1, overflowY: "auto", paddingTop: 0 }}
        >
          {streamGroups.map((g) => (
            <div key={g.stream.id} style={STRIP_PANEL_STYLE}>
              <StripRow
                letter={titleInitials(g.stream.title)}
                label={g.stream.title}
                isStream
                guide={g.threads.length > 0 ? "stream" : "none"}
                isWriter={false}
                selected={false}
                status={undefined}
                onClick={() => handleStripStream(g.stream.id)}
                menu={streamMenu(g.stream)}
                testId={`navigator-strip-stream-${g.stream.id}`}
              />
              {g.threads.map(({ thread, isWriter }, i) => {
                const isSelected =
                  g.stream.id === currentStreamId &&
                  threadStates[g.stream.id]?.selectedThreadId === thread.id;
                return (
                  <StripRow
                    key={thread.id}
                    letter={titleInitials(thread.title)}
                    label={thread.title}
                    isStream={false}
                    guide={i === g.threads.length - 1 ? "last" : "mid"}
                    isWriter={isWriter}
                    selected={isSelected}
                    status={agentStatuses[thread.id]}
                    question={agentQuestions?.[thread.id]}
                    onClick={() => handleStripThread(g.stream.id, thread.id, isSelected)}
                    menu={threadMenu(thread, isWriter)}
                    testId={`navigator-strip-thread-${thread.id}`}
                  />
                );
              })}
              {/* Keep the strip in lock-step with the overlay's inline
                  add-thread input so glyph y-positions stay aligned. */}
              {pendingNewThreadFor === g.stream.id ? <div style={{ height: ADD_ROW_HEIGHT }} /> : null}
            </div>
          ))}
        </div>
        <SlideoutChevron
          direction="expand"
          stripWidth={STRIP_WIDTH}
          testId="navigator-expand"
          title="Show streams and threads"
          onClick={strip.openPanel}
        />
      </aside>

      {/* Overlay panel — anchored at left:0 so it covers the strip
          rather than sitting beside it. The icon column inside each
          overlay row is sized identically to the strip's width, so the
          glyphs render in the same x-position before and after open. */}
      {overlayOpen ? (
        <div
          ref={strip.panelRef}
          data-testid="navigator-overlay"
          {...strip.panelProps}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            height: "100%",
            width: STRIP_WIDTH + OVERLAY_WIDTH,
            background: "var(--surface-chrome)",
            borderRight: "1px solid var(--border-strong)",
            boxShadow: "8px 0 24px rgba(0, 0, 0, 0.45)",
            display: "flex",
            flexDirection: "column",
            zIndex: 30,
          }}
        >
          <div style={{ flex: 1, overflowY: "auto", paddingTop: 0 }}>
            {streamGroups.map((g) => {
              return (
                <div key={g.stream.id} style={STREAM_PANEL_STYLE}>
                  <OverlayRow
                    letter={titleInitials(g.stream.title)}
                    label={g.stream.title}
                    isStream
                    guide={g.threads.length > 0 ? "stream" : "none"}
                    isWriter={false}
                    selected={false}
                    status={undefined}
                    onClick={() => handleSwitchStream(g.stream.id)}
                    renaming={renaming?.kind === "stream" && renaming.id === g.stream.id}
                    onCommitRename={async (next) => {
                      setRenaming(null);
                      if (next && next !== g.stream.title) {
                        await onRenameStream?.(g.stream.id, next);
                      }
                    }}
                    onCancelRename={() => setRenaming(null)}
                    menu={streamMenu(g.stream)}
                    testId={`navigator-stream-row-${g.stream.id}`}
                  />
                  {g.threads.map(({ thread, isWriter }, i) => {
                    const isSelected =
                      g.stream.id === currentStreamId &&
                      threadStates[g.stream.id]?.selectedThreadId === thread.id;
                    return (
                      <OverlayRow
                        key={thread.id}
                        letter={titleInitials(thread.title)}
                        label={thread.title}
                        isStream={false}
                        guide={i === g.threads.length - 1 ? "last" : "mid"}
                        isWriter={isWriter}
                        selected={isSelected}
                        status={agentStatuses[thread.id]}
                        question={agentQuestions?.[thread.id]}
                        onClick={() => handleSelectThread(g.stream.id, thread.id)}
                        renaming={renaming?.kind === "thread" && renaming.id === thread.id}
                        onCommitRename={async (next) => {
                          setRenaming(null);
                          if (next && next !== thread.title) {
                            await onRenameThread?.(thread.id, next);
                          }
                        }}
                        onCancelRename={() => setRenaming(null)}
                        menu={threadMenu(thread, isWriter)}
                        testId={`navigator-thread-row-${thread.id}`}
                      />
                    );
                  })}
                  {/* Inline "Add thread" title input, shown when chosen from
                      the stream's menu. */}
                  {pendingNewThreadFor === g.stream.id ? (
                    <InlineNewThread
                      onSubmit={async (title) => {
                        await onCreateThread(g.stream.id, title);
                        setPendingNewThreadFor(null);
                      }}
                      onCancel={() => setPendingNewThreadFor(null)}
                    />
                  ) : null}
                </div>
              );
            })}
            <AddStreamButton
              vcsEnabled={vcsEnabled && !!onOpenNewStreamPage}
              onClick={() => onOpenNewStreamPage?.()}
            />
          </div>
          <SlideoutChevron
            direction="collapse"
            stripWidth={STRIP_WIDTH}
            testId="navigator-collapse"
            title="Hide streams and threads"
            onClick={strip.closePanel}
          />
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
      {requested.menu}
    </div>
  );
}

/** Single row inside the strip — letter + status, fixed height.
 *  Carries the full title as a native tooltip: since hover no longer
 *  expands the panel (tsk269), the tooltip is what answers "which stream
 *  / thread is this glyph?" without moving a single pixel of layout. The
 *  status dot keeps its own `awaiting` question tooltip via `question`. */
function StripRow({
  letter,
  label,
  isStream,
  guide,
  isWriter,
  selected,
  status,
  question,
  onClick,
  menu,
  testId,
}: {
  letter: string;
  label: string;
  isStream: boolean;
  guide: Guide;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
  onClick?(): void;
  /** Its right-click menu — the panel row's, headed by the full title
   *  since the glyph shows only initials. */
  menu?: MenuItem[];
  testId?: string;
}) {
  const interactive = !!onClick;
  const cm = useRowContextMenu(menu ?? [], label);
  return (
    <div
      data-testid={testId}
      title={label}
      onContextMenu={cm.onContextMenu}
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
              cm.onKeyDown(e);
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                onClick!();
              }
            }
          : undefined
      }
      style={{
        height: isStream ? ROW_HEIGHT : THREAD_ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor: interactive ? "pointer" : "default",
        // The stream row is the panel header (muted-accent tint); thread
        // rows are transparent. Selection is marked by the accent left
        // line only — no background fill.
        background: isStream ? "var(--panel-header-bg)" : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        position: "relative",
        transition: "background 120ms ease",
      }}
    >
      <IconColumn guide={guide}>
        <IconCell
          letter={letter}
          isStream={isStream}
          isLast={guide === "last"}
          hasThreads={guide === "stream"}
          isWriter={isWriter}
          status={status}
          question={question}
        />
      </IconColumn>
      {cm.menu}
    </div>
  );
}

/** Row inside the slide-over overlay — same row height + letter cell
 *  as the strip, plus the full title to the right. */
function OverlayRow({
  letter,
  label,
  isStream,
  guide,
  isWriter,
  selected,
  status,
  question,
  onClick,
  renaming = false,
  onCommitRename,
  onCancelRename,
  menu,
  testId,
}: {
  letter: string;
  label: string;
  isStream: boolean;
  guide: Guide;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
  onClick?(): void;
  renaming?: boolean;
  onCommitRename?(next: string): void | Promise<void>;
  onCancelRename?(): void;
  menu?: MenuItem[];
  testId?: string;
}) {
  const interactive = !!onClick && !renaming;
  const cm = useRowContextMenu(menu ?? [], label);
  return (
    <div
      data-testid={testId}
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={interactive ? () => onClick!() : undefined}
      onContextMenu={renaming ? undefined : cm.onContextMenu}
      onKeyDown={
        interactive
          ? (e) => {
              cm.onKeyDown(e);
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onClick!();
              }
            }
          : undefined
      }
      title={label}
      style={{
        height: isStream ? ROW_HEIGHT : THREAD_ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        cursor: interactive ? "pointer" : "default",
        // The row's tab carries its title (below), so the stream's tile
        // colour and a thread's tab run on across the row. Selection is
        // the accent left line.
        background: isStream ? "var(--panel-header-bg)" : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        transition: "background 120ms ease",
      }}
    >
      <IconColumn guide={guide} wide>
        <IconCell
          letter={letter}
          isStream={isStream}
          isLast={guide === "last"}
          hasThreads={guide === "stream"}
          isWriter={isWriter}
          status={status}
          question={question}
          label={
            renaming ? (
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
                  minWidth: 0,
                  fontSize: "var(--text-sm)",
                  fontWeight: isStream ? 700 : 400,
                  color: isStream
                    ? "var(--text-primary)"
                    : selected
                      ? "var(--text-primary)"
                      : "var(--text-secondary)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {label}
              </span>
            )
          }
        />
      </IconColumn>
      {cm.menu}
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
/** Where a row sits on its stream's guide line: the stream that starts it
 *  (with threads under it), a thread it runs past, the thread it ends at. */
type Guide = "none" | "stream" | "mid" | "last";

/** A row's icon column, the same in the strip and the panel so their rows
 *  line up: the glyph, flush with the column's right edge like a tab, and
 *  the guide line that ties a stream's threads to it — down the column's
 *  left from under the stream's tile, with a tick into each thread's tab,
 *  ending at the last one. */
function IconColumn({
  guide,
  wide = false,
  children,
}: {
  guide: Guide;
  /** The panel's row: the column runs the row's width, its tab with it. */
  wide?: boolean;
  children: ReactNode;
}) {
  const line = (style: CSSProperties) => (
    <span aria-hidden style={{ position: "absolute", background: "var(--text-muted)", ...style }} />
  );
  const isThread = guide === "mid" || guide === "last";
  const height = isThread ? THREAD_ROW_HEIGHT : ROW_HEIGHT;
  const mid = height / 2;
  return (
    <span
      style={{
        position: "relative",
        flexShrink: 0,
        ...(wide ? { flex: 1, minWidth: 0 } : { width: ICON_COLUMN }),
        height,
        display: "flex",
        alignItems: "center",
        justifyContent: "flex-start",
        paddingLeft: isThread ? THREAD_LEFT : STREAM_LEFT,
        boxSizing: "border-box",
      }}
    >
      {guide !== "none" ? (
        <span data-guide={guide}>
          {guide === "stream"
            ? line({ left: GUIDE_X, width: 2, top: (ROW_HEIGHT + ICON_BOX) / 2, bottom: 0 })
            : null}
          {isThread ? line({ left: GUIDE_X, width: 2, top: 0, height: guide === "last" ? mid + 1 : height }) : null}
          {isThread ? line({ left: GUIDE_X, height: 2, top: mid - 1, width: THREAD_LEFT - GUIDE_X }) : null}
        </span>
      ) : null}
      {children}
    </span>
  );
}

function IconCell({
  letter,
  isStream,
  isLast,
  hasThreads,
  isWriter,
  status,
  question,
  label,
}: {
  letter: string;
  isStream: boolean;
  /** The panel's title: the tab runs on across the row carrying it. */
  label?: ReactNode;
  /** The last of its stream's threads: its tab closes the stack. */
  isLast: boolean;
  /** A stream with threads under it: its tab is the stack's top. */
  hasThreads: boolean;
  isWriter: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
}) {
  // A stream is an inverted tile (light, dark letters) heading its
  // threads; its threads are tabs indented off the stream's guide line and
  // butted up against each other — each fills its row, drawing its top
  // edge, and the last one also the bottom — a faint fill, the writer's in
  // the accent. Both are rounded on the left only and run to the column's
  // right edge, like tabs. In the panel a tab runs on across the row
  // carrying the title, its glyph kept at the strip's width so the two
  // line up.
  const glyphWidth = isStream ? ICON_COLUMN - STREAM_LEFT : ICON_COLUMN - THREAD_LEFT;
  const shape: CSSProperties = isStream
    ? {
        width: glyphWidth,
        height: ICON_BOX,
        // Rounded only where no tab sits next to it: the stack's top, and
        // its bottom when the stream has no threads.
        borderRadius: hasThreads ? "6px 0 0 0" : "6px 0 0 6px",
        // The tab strip's own header tint, so the rail matches the tabs.
        backgroundImage: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg))",
        backgroundColor: "var(--surface-card)",
        color: "var(--text-primary)",
        fontSize: LETTER_FONT,
        fontWeight: 700,
      }
    : {
        width: glyphWidth,
        height: THREAD_ROW_HEIGHT,
        // Tabs next to each other are flat; only the stack's last is
        // rounded at the bottom.
        borderRadius: isLast ? "0 0 0 6px" : 0,
        background: isWriter ? "var(--accent-soft-bg)" : "transparent",
        color: "var(--text-primary)",
        borderWidth: isLast ? "1px 0 1px 1px" : "1px 0 0 1px",
        borderStyle: "solid",
        borderColor: isWriter ? "var(--accent)" : "var(--border-strong)",
        fontSize: THREAD_LETTER_FONT,
        fontWeight: 600,
      };
  return (
    <span
      data-glyph={isStream ? "stream" : "thread"}
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        boxSizing: "border-box",
        lineHeight: 1,
        ...shape,
        ...(label !== undefined ? { width: undefined, flex: 1, minWidth: 0, justifyContent: "flex-start" } : {}),
      }}
    >
      {label !== undefined ? (
        <>
          {/* The glyph keeps its strip width (less a thread tab's left edge). */}
          <span style={{ width: glyphWidth - (isStream ? 0 : 1), flexShrink: 0, textAlign: "center" }}>{letter}</span>
          {label}
        </>
      ) : (
        letter
      )}
      {/* Threads always show the agent's activity indicator. Mirrors
          the fallback the Agent tab uses (App.tsx — `agentStatuses[id]
          ?? "waiting"`) so a never-attached thread still reads as the
          same red "waiting" dot here as it does on the tab. Stream
          rows omit the dot entirely. */}
      {!isStream ? (
        <span
          style={{
            position: "absolute",
            // On the tab's left edge, where the guide's tick meets it.
            top: "50%",
            left: -5,
            transform: "translateY(-50%)",
            display: "flex",
          }}
        >
          <AgentStatusDot status={status ?? "waiting"} size={8} question={question} />
        </span>
      ) : null}
    </span>
  );
}

function AddStreamButton({ vcsEnabled, onClick }: { vcsEnabled: boolean; onClick(): void }) {
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
        onClick={() => { if (vcsEnabled) onClick(); }}
        disabled={!vcsEnabled}
        title={vcsEnabled ? "Create a new stream" : "Disabled: workspace root is not its own git repo"}
        style={{
          width: "100%",
          textAlign: "center",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6,
          padding: "6px 10px",
          cursor: vcsEnabled ? "pointer" : "not-allowed",
          fontFamily: "inherit",
          fontSize: "var(--text-xs)",
          fontWeight: "var(--weight-medium)",
          letterSpacing: 0.2,
          opacity: vcsEnabled ? 1 : 0.5,
        }}
      >
        + Add stream
      </button>
    </div>
  );
}

/// The new-thread title strip. A thread names no agent: it opens on the
/// session picker.
function InlineNewThread({
  onSubmit,
  onCancel,
}: {
  onSubmit(title: string): Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <form
      onSubmit={async (e) => {
        e.preventDefault();
        const t = value.trim();
        if (!t) return onCancel();
        setBusy(true);
        try {
          await onSubmit(t);
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
        data-testid="navigator-new-thread-input"
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
    </form>
  );
}

// Glyph sizing. The two-letter glyph runs a touch above body text
// (15px vs 14px) so the strip still reads as a real navigation
// indicator. Strip width and row height stay proportional so the icon
// cell sits comfortably with breathing room on both sides.
const LETTER_FONT = 15;
const ICON_BOX = 30;
// A thread's row is its tab: the tabs of a stream's threads touch, a touch
// shorter than the stream's tile so the two read as parent and child.
const THREAD_ROW_HEIGHT = 28;
const THREAD_LETTER_FONT = 13;
const STRIP_WIDTH = 40;
// The icon column: the strip's width less the selection line's 3px and the
// strip's 1px edge.
const ICON_COLUMN = STRIP_WIDTH - 4;
// Where a stream's tile starts (over the guide line's top), the guide
// line's x, and where a thread's tab starts. Both run to the column's
// right edge.
const STREAM_LEFT = 2;
const GUIDE_X = 5;
const THREAD_LEFT = 9;
const STRIP_PADDING_Y = 6;
const ROW_HEIGHT = 36;
const GAP_HEIGHT = 14;
// Height reserved (strip) and matched (overlay) for the per-stream
// "+ New thread" row so subsequent items stay y-aligned across the
// two views. Must match the rendered AddThreadRow height.
const ADD_ROW_HEIGHT = 40;
const OVERLAY_WIDTH = 240;

// Each stream + its threads renders inside this box: a surface-card panel
// flush along the left window edge, square-cornered so the tabs inside it
// keep their square right edges, with a gap below it. Shared by the strip
// and the slide-over overlay.
const STREAM_PANEL_STYLE: CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  marginBottom: 6,
  overflow: "hidden",
};
// In the strip the strip's own edge is the panel's right side.
const STRIP_PANEL_STYLE: CSSProperties = { ...STREAM_PANEL_STYLE, borderRight: "none" };
