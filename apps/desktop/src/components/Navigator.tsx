import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import { useSlideoutStrip } from "./useSlideoutStrip.js";
import { SlideoutChevron } from "./SlideoutChevron.js";
import { archiveStream, type AgentKind, type Stream, type Thread, type ThreadState } from "../api.js";
import { agentChoices, parseAgentChoice } from "../agentKinds.js";
import { listAcpAgents } from "../api.js";
import type { AcpAgentListing } from "../tauri-bridge/generated/bindings.js";
import { subscribeNewThreadRequests } from "../new-thread-bus.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { useRowContextMenu } from "./useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { Slideover } from "./Slideover.js";
import { titleInitials } from "../initials.js";

interface NavigatorProps {
  streams: Stream[];
  currentStreamId: string | null;
  threadStates: Record<string, ThreadState>;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  /// Per-thread await_user question, shown as the rail dot's tooltip
  /// while that thread's status is "awaiting". Absent for every other
  /// state. Keyed by thread id, parallel to agentStatuses.
  agentQuestions?: Record<string, string | undefined>;
  enabledAgents: AgentKind[];
  onSwitchStream(id: string): void | Promise<void>;
  onSelectThread(streamId: string, threadId: string): void | Promise<void>;
  onCreateThread(streamId: string, title: string, agent?: AgentKind, acpAgent?: string | null): Promise<void>;
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
 * directly — a thread glyph selects that thread, a stream glyph switches
 * to that stream. Hovering a glyph shows its full title as a tooltip and
 * nothing else.
 *
 * The panel expands only on an explicit click: the bottom-pinned chevron,
 * or dead space in the strip. It re-renders the same rows with full
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

  // Build the list of "rows" so the strip and overlay can both walk
  // the same sequence — guaranteeing matching y-positions row-by-row.
  // `add-thread` rows are flyout-only (skipped in the strip render).
  // Each stream + its threads renders as one panel (a surface-card box,
  // rounded on the right, flush on the left), with a gap between groups —
  // mirroring the main rail's panel look. The strip and the slide-over
  // overlay both map this same structure so glyph y-positions stay in
  // lock-step when the overlay opens.
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
          // Part of the lighter chrome frame — matches the HUD rail to its
          // right so the whole left edge reads as one seamless surface (no
          // divider between the strip and the rail).
          background: "var(--surface-chrome)",
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
                isWriter={false}
                selected={false}
                status={undefined}
                onClick={() => handleSwitchStream(g.stream.id)}
                testId={`navigator-strip-stream-${g.stream.id}`}
              />
              {g.threads.map(({ thread, isWriter }) => {
                const isSelected =
                  g.stream.id === currentStreamId &&
                  threadStates[g.stream.id]?.selectedThreadId === thread.id;
                return (
                  <StripRow
                    key={thread.id}
                    letter={titleInitials(thread.title)}
                    label={thread.title}
                    isStream={false}
                    isWriter={isWriter}
                    selected={isSelected}
                    status={agentStatuses[thread.id]}
                    question={agentQuestions?.[thread.id]}
                    onClick={() => handleSelectThread(g.stream.id, thread.id)}
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
              const isPrimary = g.stream.kind === "primary";
              const isWorking = streamStatuses[g.stream.id] === "working";
              const streamMenu: MenuItem[] = [
                {
                  id: "stream.add-thread",
                  label: "Add thread",
                  enabled: true,
                  run: () => setPendingNewThreadFor(g.stream.id),
                },
                {
                  id: "stream.rename",
                  label: "Rename…",
                  enabled: !!onRenameStream,
                  run: () => setRenaming({ kind: "stream", id: g.stream.id }),
                },
                {
                  id: "stream.settings",
                  label: "Settings…",
                  enabled: !!onOpenStreamSettings,
                  run: () => onOpenStreamSettings?.(g.stream.id),
                },
              ];
              if (!isPrimary) {
                streamMenu.push({
                  id: "stream.remove",
                  label: "Remove…",
                  // Disable when an agent is currently running in any of
                  // this stream's threads — the IPC also rejects, but
                  // disabling avoids a useless prompt.
                  enabled: !isWorking,
                  run: () => {
                    setRemoveStream(g.stream);
                    setRemoveWorktree(false);
                    setRemoveError(null);
                  },
                });
              }
              return (
                <div key={g.stream.id} style={STREAM_PANEL_STYLE}>
                  <OverlayRow
                    letter={titleInitials(g.stream.title)}
                    label={g.stream.title}
                    isStream
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
                    menu={streamMenu}
                    testId={`navigator-stream-row-${g.stream.id}`}
                  />
                  {g.threads.map(({ thread, isWriter }) => {
                    const isSelected =
                      g.stream.id === currentStreamId &&
                      threadStates[g.stream.id]?.selectedThreadId === thread.id;
                    const threadMenu: MenuItem[] = [];
                    // "Make writer" is the headline action for a read-only
                    // thread: only the stream's single active thread can
                    // edit files, so a queued thread is "edits blocked"
                    // until promoted. Show it FIRST, and only when this
                    // thread isn't already the writer (tsk132).
                    if (!isWriter) {
                      threadMenu.push({
                        id: "thread.promote",
                        label: "Make writer",
                        enabled: !!onPromoteThread,
                        run: () => onPromoteThread?.(thread.id),
                      });
                    }
                    threadMenu.push(
                      {
                        id: "thread.rename",
                        label: "Rename…",
                        enabled: !!onRenameThread,
                        run: () => setRenaming({ kind: "thread", id: thread.id }),
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
                    return (
                      <OverlayRow
                        key={thread.id}
                        letter={titleInitials(thread.title)}
                        label={thread.title}
                        isStream={false}
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
                        menu={threadMenu}
                        testId={`navigator-thread-row-${thread.id}`}
                      />
                    );
                  })}
                  {/* Inline "Add thread" title input, shown when chosen from
                      the stream's menu. */}
                  {pendingNewThreadFor === g.stream.id ? (
                    <InlineNewThread
                      enabledAgents={enabledAgents}
                      onSubmit={async (title, agent, acpAgent) => {
                        await onCreateThread(g.stream.id, title, agent, acpAgent);
                        setPendingNewThreadFor(null);
                      }}
                      onCancel={() => setPendingNewThreadFor(null)}
                    />
                  ) : null}
                </div>
              );
            })}
            <AddStreamButton
              gitEnabled={gitEnabled && !!onOpenNewStreamPage}
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
  isWriter,
  selected,
  status,
  question,
  onClick,
  testId,
}: {
  letter: string;
  label: string;
  isStream: boolean;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
  onClick?(): void;
  testId?: string;
}) {
  const interactive = !!onClick;
  return (
    <div
      data-testid={testId}
      title={label}
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
        // The stream row is the panel header (muted-accent tint); thread
        // rows are transparent. Selection is marked by the accent left
        // line only — no background fill.
        background: isStream ? "var(--panel-header-bg)" : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        position: "relative",
        transition: "background 120ms ease",
      }}
    >
      <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} question={question} />
    </div>
  );
}

/** Row inside the slide-over overlay — same row height + letter cell
 *  as the strip, plus the full title to the right. */
function OverlayRow({
  letter,
  label,
  isStream,
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
  const cm = useRowContextMenu(menu ?? []);
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
        height: ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        gap: 8,
        cursor: interactive ? "pointer" : "default",
        // The stream row is the panel header (muted-accent tint); thread
        // rows are transparent. Selection is marked by the accent left
        // line only — no background fill.
        background: isStream ? "var(--panel-header-bg)" : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        paddingRight: 6,
        transition: "background 120ms ease",
      }}
    >
      <div style={{ width: STRIP_WIDTH - 3 /* keep the icon column the same width as the strip */, display: "flex", justifyContent: "center" }}>
        <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} question={question} />
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
function IconCell({
  letter,
  isStream,
  isWriter,
  status,
  question,
}: {
  letter: string;
  isStream: boolean;
  isWriter: boolean;
  status: AgentStatusDotState | undefined;
  question?: string;
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
          <AgentStatusDot status={status ?? "waiting"} size={8} question={question} />
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
  onSubmit(title: string, agent?: AgentKind, acpAgent?: string | null): Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [acpAgents, setAcpAgents] = useState<AcpAgentListing[]>([]);
  const acpEnabled = enabledAgents.includes("acp");
  useEffect(() => {
    if (!acpEnabled) return;
    void listAcpAgents()
      .then(setAcpAgents)
      .catch(() => setAcpAgents([]));
  }, [acpEnabled]);
  const choices = agentChoices(enabledAgents.length > 0 ? enabledAgents : ["claude"], acpAgents);
  const [choice, setChoice] = useState<string>(choices[0]?.value ?? "claude");
  const picked = choices.some((c) => c.value === choice) ? choice : (choices[0]?.value ?? "claude");
  return (
    <form
      onSubmit={async (e) => {
        e.preventDefault();
        const t = value.trim();
        if (!t) return onCancel();
        setBusy(true);
        try {
          const { agent, acpAgent } = parseAgentChoice(picked);
          await onSubmit(t, agent, acpAgent);
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
      {choices.length > 1 ? (
        <select
          data-testid="navigator-new-thread-agent"
          value={picked}
          disabled={busy}
          onChange={(e) => setChoice(e.target.value)}
          style={{
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 4,
            padding: "4px 6px",
            fontSize: "var(--text-xs)",
          }}
        >
          {choices.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
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

// Each stream + its threads renders inside this box: a surface-card panel
// flush along the left window edge, rounded on the right, with a gap below
// it (matching the main rail's panel look). Shared by the strip and the
// slide-over overlay.
const STREAM_PANEL_STYLE: CSSProperties = {
  background: "var(--surface-card)",
  // Same bordered-card treatment as the left rail's RailSection panels
  // (--surface-card + --border-subtle + radius 6). Flush along the left
  // window edge, so only the right corners round.
  border: "1px solid var(--border-subtle)",
  borderTopRightRadius: 6,
  borderBottomRightRadius: 6,
  marginBottom: 6,
  overflow: "hidden",
};
