import type { CSSProperties, ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useSlideoutStrip } from "./useSlideoutStrip.js";
import { SlideoutChevron } from "./SlideoutChevron.js";
import {
  subscribeNavigatorMenuRequests,
  subscribeNavigatorOpenRequests,
  type NavigatorMenuRequest,
} from "../navigator-bus.js";
import { archiveStream, type Stream, type Thread, type ThreadMark, type ThreadState } from "../api.js";
import { subscribeNewThreadRequests } from "../new-thread-bus.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { useContextMenu, useRowContextMenu, type MenuItems } from "./useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { threadUri } from "../uri/uri.js";
import { refCommandMenuItems, useRefOffers } from "./refCommands.js";
import type { OpenHandoffs } from "../handoffs.js";
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
  /** Open handoffs to each thread, by thread id: a count on its row. */
  openHandoffs?: Record<string, OpenHandoffs>;
  onSwitchStream(id: string): void | Promise<void>;
  onSelectThread(streamId: string, threadId: string): void | Promise<void>;
  /** Whether it was made (a failure is the runner's: recorded, toasted). */
  onCreateThread(streamId: string, title: string): Promise<boolean>;
  onOpenNewStreamPage?(): void;
  onRenameStream?(streamId: string, title: string): void | Promise<void>;
  onRenameThread?(threadId: string, title: string): void | Promise<void>;
  /** Mark a thread's tab with a colour, or clear it (`null`). */
  onMarkThread?(threadId: string, mark: ThreadMark | null): void | Promise<void>;
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
 *   - A thread the person marked (thread menu → Mark with…) carries a
 *     corner fold (a small right triangle in its `--thread-mark-<colour>`)
 *     on its tab's top-left corner — not an accent outline, which reads as
 *     selected. The activity dot sits on the right end at half height,
 *     clear of it; both are in the same place in strip and overlay.
 *   - Selection treatment: a 3px accent bar in the dark gap right of the
 *     tab, against it. The overlay draws the strip's tab and gap unchanged,
 *     then the same again for the title, so it has a bar after each.
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
  openHandoffs,
  onSwitchStream,
  onSelectThread,
  onCreateThread,
  onOpenNewStreamPage,
  onRenameStream,
  onRenameThread,
  onMarkThread,
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
  const refOffersFor = useRefOffers();
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
  const threadMenu = async (thread: Thread): Promise<MenuItem[]> => {
    const markItem = (mark: ThreadMark | null): MenuItem => ({
      id: `thread.mark.${mark ?? "none"}`,
      label: mark ? MARK_LABELS[mark] : "None",
      enabled: true,
      checked: (thread.mark ?? null) === mark,
      swatch: mark ? markColour(mark) : undefined,
      run: () => onMarkThread?.(thread.id, mark),
    });
    const items: MenuItem[] = [];
    items.push(
      {
        id: "thread.mark",
        label: "Mark with…",
        enabled: !!onMarkThread,
        submenuHeader: "A colour on the thread's tab, for grouping threads your way. It changes nothing else.",
        submenu: [...THREAD_MARKS.map(markItem), markItem(null)],
      },
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
      // Then the commands about the thread (usability.md "Menus a page or
      // row gets from extensions").
      ...refCommandMenuItems(await refOffersFor(threadUri(thread.id))),
    );
    return items;
  };

  // Build the list of "rows" so the strip and overlay can both walk
  // the same sequence — guaranteeing matching y-positions row-by-row.
  // `add-thread` rows are flyout-only (skipped in the strip render).
  // Each stream + its threads renders as one panel (a transparent box,
  // flush on the left), with a gap between groups —
  // mirroring the main rail's panel look. The strip and the slide-over
  // overlay both map this same structure so glyph y-positions stay in
  // lock-step when the overlay opens.
  // A stream's or thread's menu asked for from outside (the title bar's
  // names): the same menu, headed the same way, at the asker's point.
  const requested = useContextMenu();
  const menuFor = (r: NavigatorMenuRequest): { items: MenuItems; header: string } | null => {
    if (r.kind === "stream") {
      const s = streams.find((x) => x.id === r.id);
      return s ? { items: streamMenu(s), header: s.title } : null;
    }
    for (const ts of Object.values(threadStates)) {
      const t = ts.threads.find((x) => x.id === r.id);
      if (t) return { items: threadMenu(t), header: t.title };
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
        return { stream: s, threads: ts?.threads ?? [] };
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
          // The panel colour that sits left of the tabs runs the rail's
          // whole height, below the tabs too, with no line between it and
          // the HUD rail. The app's dark, so the tabs stand out the way
          // the centre tab strip does against the content.
          background: "var(--surface-app)",

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
            <div key={g.stream.id} style={STREAM_PANEL_STYLE}>
              <StripRow
                letter={titleInitials(g.stream.title)}
                label={g.stream.title}
                isStream
                guide={g.threads.length > 0 ? "stream" : "none"}
                mark={null}
                selected={false}
                status={undefined}
                onClick={() => handleStripStream(g.stream.id)}
                menu={() => streamMenu(g.stream)}
                testId={`navigator-strip-stream-${g.stream.id}`}
              />
              {g.threads.map((thread, i) => {
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
                    mark={thread.mark ?? null}
                    selected={isSelected}
                    status={agentStatuses[thread.id]}
                    question={agentQuestions?.[thread.id]}
                    onClick={() => handleStripThread(g.stream.id, thread.id, isSelected)}
                    menu={() => threadMenu(thread)}
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
            // The strip's dark, so the gaps between its tabs look the same.
            background: "var(--surface-app)",
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
                    mark={null}
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
                    menu={() => streamMenu(g.stream)}
                    testId={`navigator-stream-row-${g.stream.id}`}
                  />
                  {g.threads.map((thread, i) => {
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
                        mark={thread.mark ?? null}
                        selected={isSelected}
                        status={agentStatuses[thread.id]}
                        question={agentQuestions?.[thread.id]}
                        handoffs={openHandoffs?.[thread.id]}
                        onClick={() => handleSelectThread(g.stream.id, thread.id)}
                        renaming={renaming?.kind === "thread" && renaming.id === thread.id}
                        onCommitRename={async (next) => {
                          setRenaming(null);
                          if (next && next !== thread.title) {
                            await onRenameThread?.(thread.id, next);
                          }
                        }}
                        onCancelRename={() => setRenaming(null)}
                        menu={() => threadMenu(thread)}
                        testId={`navigator-thread-row-${thread.id}`}
                      />
                    );
                  })}
                  {/* Inline "Add thread" title input, shown when chosen from
                      the stream's menu. */}
                  {pendingNewThreadFor === g.stream.id ? (
                    <InlineNewThread
                      onSubmit={async (title) => {
                        // A failure keeps the title, to try again.
                        if (await onCreateThread(g.stream.id, title)) setPendingNewThreadFor(null);
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
  mark,
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
  /** The person's colour for a thread's tab (`null` unmarked, and for a
   *  stream). */
  mark: ThreadMark | null;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
  onClick?(): void;
  /** Its right-click menu — the panel row's, headed by the full title
   *  since the glyph shows only initials. */
  menu?: () => MenuItems;
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
        position: "relative",
        transition: "background 120ms ease",
      }}
    >
      <IconColumn guide={guide} selected={selected}>
        <IconCell
          selected={selected}
          letter={letter}
          isStream={isStream}
          isLast={guide === "last"}
          hasThreads={guide === "stream"}
          mark={mark}
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
  mark,
  selected,
  status,
  question,
  handoffs,
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
  /** The person's colour for a thread's tab (`null` unmarked, and for a
   *  stream). */
  mark: ThreadMark | null;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
  /** Open handoffs to the thread. */
  handoffs?: OpenHandoffs;
  onClick?(): void;
  renaming?: boolean;
  onCommitRename?(next: string): void | Promise<void>;
  onCancelRename?(): void;
  menu?: () => MenuItems;
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
        // The row's tab carries its title (below), so the stream's colour
        // and a thread's tab run on across the row (the row itself has no
        // fill, so the stream tab's rounded corner shows). Selection is the
        // accent left line.
        background: "transparent",
        transition: "background 120ms ease",
      }}
    >
      <IconColumn guide={guide} selected={selected} wide>
        <IconCell
          selected={selected}
          letter={letter}
          isStream={isStream}
          isLast={guide === "last"}
          hasThreads={guide === "stream"}
          mark={mark}
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
              <>
                <span
                  style={{
                    flex: 1,
                  minWidth: 0,
                  fontSize: "var(--text-sm)",
                  fontWeight: isStream ? 700 : 400,
                  color: isStream
                    ? "var(--text-on-stream-tile)"
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
                {handoffs ? (
                  <span
                    data-testid={testId ? `${testId}-handoffs` : undefined}
                    title={`${handoffs.count} open handoff${handoffs.count === 1 ? "" : "s"}; the newest from ${handoffs.newestFrom}`}
                    style={handoffBadgeStyle}
                  >
                    {handoffs.count}
                  </span>
                ) : null}
              </>
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

/** Where a row sits on its stream's guide line: the stream that starts it
 *  (with threads under it), a thread it runs past, the thread it ends at. */
type Guide = "none" | "stream" | "mid" | "last";

/** A row's icon column, the same in the strip and the panel so their rows
 *  line up: the glyph, a tab coming out of the window's left edge, rounded
 *  on the right — the stream's reaching the guide line, its threads' short
 *  of it — and the grey guide line that ties a stream's threads to it,
 *  down the column's right from under the stream's tab with a tick from
 *  each thread's tab, ending at the last. */
function IconColumn({
  guide,
  selected = false,
  wide = false,
  children,
}: {
  guide: Guide;
  /** The selected thread: an accent bar down the right of its tab. */
  selected?: boolean;
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
        paddingRight: isThread ? THREAD_RIGHT : STREAM_RIGHT,
        boxSizing: "border-box",
      }}
    >
      {guide !== "none" ? (
        <span data-guide={guide}>
          {guide === "stream"
            ? line({ right: GUIDE_RIGHT, width: 2, top: ROW_HEIGHT, bottom: 0 })
            : null}
          {isThread ? line({ right: GUIDE_RIGHT, width: 2, top: 0, height: guide === "last" ? mid + 1 : height }) : null}
          {isThread ? line({ right: GUIDE_RIGHT + 2, height: 2, top: mid - 1, width: TAB_GAP }) : null}
        </span>
      ) : null}
      {selected && isThread ? <SelectionBar right={THREAD_RIGHT - SELECTION_BAR} /> : null}
      {children}
    </span>
  );
}

/** The selected thread's mark, in the dark gap against its tab: as thick as
 *  a selected centre tab's underline, in the same colour. */
function SelectionBar(position: { left: number } | { right: number }) {
  return (
    <span
      aria-hidden
      data-testid="navigator-selected-fill"
      style={{
        position: "absolute",
        background: "var(--accent)",
        width: SELECTION_BAR,
        top: 0,
        bottom: 0,
        ...position,
      }}
    />
  );
}

function IconCell({
  letter,
  isStream,
  isLast,
  hasThreads,
  mark,
  selected,
  status,
  question,
  label,
}: {
  letter: string;
  isStream: boolean;
  /** The panel's title: for a thread a second tab across the row, after the
   *  gap; for a stream, its one tab runs on across the row. */
  label?: ReactNode;
  /** The last of its stream's threads: its tab closes the stack. */
  isLast: boolean;
  /** A stream with threads under it: its tab is the stack's top. */
  hasThreads: boolean;
  /** The person's colour for a thread's tab (`null` unmarked, and for a
   *  stream). */
  mark: ThreadMark | null;
  /** The selected thread: in the panel, a bar in the gap after its
   *  abbreviation (`IconColumn` draws the one at the row's end). */
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
}) {
  // Tabs coming out of the window's left edge: a stream's in the tab
  // strip's header tint, its threads' stopping short of the guide line on
  // the right and butted up
  // against each other — each fills its row, drawing its top edge, and the
  // last one also the bottom.
  // Rounded on the right only. In the panel a thread's abbreviation keeps
  // the strip's tab, squared off, and a second tab of the same shape carries
  // the title, after the gap; a stream's tab runs on across the row.
  const glyphWidth = ICON_COLUMN - (isStream ? STREAM_RIGHT : THREAD_RIGHT);
  const shape: CSSProperties = isStream
    ? {
        width: glyphWidth,
        height: ROW_HEIGHT,
        // Rounded only where no tab sits next to it: the stack's top, and
        // its bottom when the stream has no threads.
        borderRadius: hasThreads ? "0 6px 0 0" : "0 6px 6px 0",
        // The guide line's grey, with dark text to stand out against it.
        background: "var(--text-muted)",
        color: "var(--text-on-stream-tile)",
        fontSize: LETTER_FONT,
        fontWeight: 700,
      }
    : {
        width: glyphWidth,
        height: THREAD_ROW_HEIGHT,
        // Tabs next to each other are flat; only the stack's last is
        // rounded at the bottom.
        borderRadius: isLast ? "0 0 6px 0" : 0,
        // The centre tab strip's header tint. A marked thread looks like
        // any other tab (the accent outline and tint read as "selected");
        // a corner fold in its colour marks it instead.
        backgroundImage: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg))",
        backgroundColor: "var(--surface-card)",
        color: "var(--text-primary)",
        // Each tab but the last draws a divider along its bottom, in the
        // guide line's grey, so it stands out between two tabs (a
        // frame-coloured one vanished at this size) and none falls under
        // the stream's tab; the last's bottom and every right edge keep
        // the frame.
        borderWidth: isLast ? "0 1px 1px 0" : `0 1px ${THREAD_DIVIDER}px 0`,
        borderStyle: "solid",
        borderColor: isLast ? "var(--border-strong)" : "transparent var(--border-strong) var(--text-muted)",
        fontSize: THREAD_LETTER_FONT,
        fontWeight: 600,
      };
  const tabBase: CSSProperties = {
    position: "relative",
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    boxSizing: "border-box",
    lineHeight: 1,
    ...shape,
  };
  // The agent's dot belongs to the thread's outermost tab, on its right end
  // where the guide's tick meets it.
  const dot = !isStream ? (
    <span
      style={{
        position: "absolute",
        top: "50%",
        right: -5,
        transform: "translateY(-50%)",
        display: "flex",
      }}
    >
      {/* Mirrors the fallback the Agent tab uses (App.tsx — `agentStatuses[id]
          ?? "waiting"`) so a never-attached thread still reads as the same
          red "waiting" dot here as it does on the tab. */}
      <AgentStatusDot status={status ?? "waiting"} size={8} question={question} />
    </span>
  ) : null;
  const tab = (
    <span data-glyph={isStream ? "stream" : "thread"} style={tabBase}>
      {letter}
      {/* At the window's edge, so the strip's and the panel's line up. */}
      {mark && !isStream ? <MarkFold mark={mark} /> : null}
      {dot}
    </span>
  );
  if (label === undefined) return tab;
  // A stream's row is one tab, its abbreviation kept at the strip's width.
  if (isStream) {
    return (
      <span
        data-glyph="stream"
        data-testid="navigator-tab-title"
        style={{ ...tabBase, width: undefined, flex: 1, minWidth: 0, justifyContent: "flex-start" }}
      >
        <span
          data-testid="navigator-stream-abbreviation"
          style={{ width: glyphWidth, flexShrink: 0, textAlign: "center" }}
        >
          {letter}
        </span>
        {/* The thread rows' dark gap carried up through the header, thin. */}
        <span
          aria-hidden
          data-testid="navigator-stream-divider"
          style={{
            position: "absolute",
            left: ICON_COLUMN - THREAD_RIGHT + (TAB_GAP - 1) / 2,
            width: 1,
            top: 0,
            bottom: 0,
            background: "var(--surface-app)",
          }}
        />
        <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", paddingLeft: TITLE_LEFT - glyphWidth }}>
          {label}
        </span>
      </span>
    );
  }
  return (
    <span style={{ display: "flex", flex: 1, minWidth: 0, height: "100%", alignItems: "center" }}>
      {/* Its curve is at the row's end, so this end is square. */}
      <span data-glyph="thread" style={{ ...tabBase, borderRadius: 0 }}>
        {letter}
        {/* At the window's edge, so the strip's and the panel's line up. */}
        {mark ? <MarkFold mark={mark} /> : null}
      </span>
      {/* The strip's dark gap, with its bar. Only the row's end has a guide. */}
      <span
        data-testid="navigator-tab-gap"
        style={{
          position: "relative",
          width: TAB_GAP,
          flexShrink: 0,
          alignSelf: "stretch",
          boxSizing: "border-box",
          // The line between stacked tabs runs on through the gap.
          ...(isLast
            ? {}
            : { borderBottomWidth: THREAD_DIVIDER, borderBottomStyle: "solid", borderBottomColor: "var(--text-muted)" }),
        }}
      >
        {selected ? <SelectionBar left={0} /> : null}
      </span>
      <span
        data-testid="navigator-tab-title"
        style={{
          ...tabBase,
          width: undefined,
          flex: 1,
          minWidth: 0,
          justifyContent: "flex-start",
          paddingLeft: TITLE_LEFT - glyphWidth - TAB_GAP,
        }}
      >
        {label}
        {dot}
      </span>
    </span>
  );
}

/** The colours a person marks a thread with, in menu order. */
const THREAD_MARKS: ThreadMark[] = ["amber", "red", "green", "blue", "violet"];

const MARK_LABELS: Record<ThreadMark, string> = {
  amber: "Amber",
  red: "Red",
  green: "Green",
  blue: "Blue",
  violet: "Violet",
};

/** A mark's colour token (`--thread-mark-<colour>`, theming.md). */
function markColour(mark: ThreadMark): string {
  return `var(--thread-mark-${mark})`;
}

/** A marked thread's corner fold: a right triangle, its right angle in
 *  the tab's top-left corner, in its mark's colour. */
function MarkFold({ mark }: { mark: ThreadMark }) {
  return (
    <span
      data-testid="navigator-thread-mark"
      title={`Marked ${mark}`}
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        display: "flex",
        color: markColour(mark),
      }}
    >
      <svg width="7" height="7" viewBox="0 0 7 7" aria-hidden fill="currentColor">
        <path d="M0 0H7L0 7Z" />
      </svg>
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
// A thread's row is its tab: the tabs of a stream's threads touch, a touch
// shorter than the stream's tile so the two read as parent and child.
const THREAD_ROW_HEIGHT = 28;
const THREAD_LETTER_FONT = 13;
const STRIP_WIDTH = 40;
// The icon column is the strip's width.
const ICON_COLUMN = STRIP_WIDTH;
const SELECTION_BAR = 3;
// The tabs come out of the window's left edge. The guide line runs down the
// column's right, flush with the stream tab's right edge; a thread's tab
// ends short of it (a tick joins them).
const STREAM_RIGHT = 2;
const GUIDE_RIGHT = STREAM_RIGHT;
const THREAD_RIGHT = 9;
// The dark gap right of a thread's tab, up to the guide; the panel draws the
// same gap between the abbreviation and the title.
const TAB_GAP = THREAD_RIGHT - GUIDE_RIGHT - 2;
// Where a title starts in the panel's rows, measured from the window's edge,
// so stream and thread titles line up.
const TITLE_LEFT = ICON_COLUMN + 13;
// The divider drawn along a thread tab's bottom, between stacked tabs.
const THREAD_DIVIDER = 1;
const ROW_HEIGHT = 36;
const GAP_HEIGHT = 14;
// Height reserved (strip) and matched (overlay) for the per-stream
// "+ New thread" row so subsequent items stay y-aligned across the
// two views. Must match the rendered AddThreadRow height.
const ADD_ROW_HEIGHT = 40;
const OVERLAY_WIDTH = 240;

// Each stream + its threads renders inside this box: a transparent panel
// (the strip's dark or the overlay's chrome shows through) flush along the left window edge, with no outline of its own (the tabs
// carry the borders), square-cornered so the tabs inside it
// keep their square right edges, with a gap below it. Shared by the strip
// and the slide-over overlay.
const STREAM_PANEL_STYLE: CSSProperties = {
  marginBottom: 6,
  overflow: "hidden",
};

/** The count of a thread's open handoffs, after its title. */
const handoffBadgeStyle: CSSProperties = {
  flexShrink: 0,
  marginLeft: 4,
  marginRight: 6,
  minWidth: 16,
  padding: "0 4px",
  borderRadius: 8,
  fontSize: "var(--text-xs)",
  lineHeight: "16px",
  textAlign: "center",
  color: "var(--text-on-accent)",
  background: "var(--accent)",
};
