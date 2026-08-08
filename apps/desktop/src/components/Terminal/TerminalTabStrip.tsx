import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import { titleInitials } from "../../initials.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { useSlideoutStrip } from "../useSlideoutStrip.js";
import { SlideoutChevron } from "../SlideoutChevron.js";
import type { MenuItem } from "../../menu.js";
import type { TerminalTab } from "./terminalTabs.js";

/**
 * Vertical tab strip for the Terminal page, sharing its open/close state
 * machine with the far-left `Navigator` via {@link useSlideoutStrip}: a
 * thin always-visible strip shows a two-letter initial glyph per terminal
 * (via {@link titleInitials}), and a panel slides over it re-rendering the
 * same rows with their full titles. Glyph y-positions match between strip
 * and panel so nothing jumps when it opens.
 *
 * Interactions (per `.context/usability.md` → "Hover reveals, click
 * rearranges"):
 * - Hover a glyph → tooltip with the full title. Hover does NOT expand;
 *   the panel covers the xterm surface, so an involuntary open would bury
 *   a live click target.
 * - Click a glyph (strip or panel row) → activate.
 * - The bottom-pinned chevron expands / collapses; clicking the strip's
 *   dead space expands too.
 * - Per-row actions live on the right-click menu (Menu key / Shift+F10
 *   for keyboard): Rename… opens an inline input in the panel row (Enter
 *   commits, Escape cancels, blur commits unless Escape was pressed);
 *   Close terminal kills the shell (disabled when only one remains).
 * - "+ New" appends a terminal.
 *
 * An open rename passes `guard`, so drifting the pointer off the panel
 * can't discard it — that used to silently throw the edit away.
 */
export function TerminalTabStrip({
  tabs,
  activeId,
  onActivate,
  onNew,
  onClose,
  onRename,
}: {
  tabs: TerminalTab[];
  activeId: string;
  onActivate(id: string): void;
  onNew(): void;
  onClose(id: string): void;
  onRename(id: string, title: string): void;
}) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const canClose = tabs.length > 1;
  const cm = useContextMenu();
  const strip = useSlideoutStrip({
    guard: renamingId !== null,
    onClose: () => setRenamingId(null),
  });

  const activate = (id: string) => {
    onActivate(id);
    strip.closePanel();
  };

  return (
    <div style={{ position: "relative", display: "flex", height: "100%", flexShrink: 0 }}>
      {/* Always-visible strip */}
      <div data-testid="terminal-tab-strip" style={stripStyle}>
        <div
          data-testid="terminal-tab-strip-empty"
          style={stripScrollStyle}
          {...strip.deadSpaceProps}
        >
          {tabs.map((tab) => {
            const active = tab.id === activeId;
            return (
              <button
                key={tab.id}
                type="button"
                data-testid={`terminal-tab-${tab.id}`}
                title={tab.title}
                onClick={() => onActivate(tab.id)}
                style={glyphButtonStyle(active)}
              >
                {titleInitials(tab.title)}
              </button>
            );
          })}
          <button
            type="button"
            data-testid="terminal-tab-new"
            title="New terminal"
            onClick={onNew}
            style={newButtonStyle}
          >
            +
          </button>
        </div>
        <SlideoutChevron
          direction="expand"
          stripWidth={STRIP_WIDTH}
          testId="terminal-tab-expand"
          title="Show terminals"
          onClick={strip.openPanel}
        />
      </div>

      {/* Slide-over panel — anchored at left:0 so it covers the strip; the
          icon column matches the strip width, so glyphs don't shift. */}
      {strip.open ? (
        <div
          ref={strip.panelRef}
          data-testid="terminal-tab-overlay"
          style={overlayStyle}
          {...strip.panelProps}
        >
          <div style={overlayScrollStyle}>
            {tabs.map((tab) => {
              const active = tab.id === activeId;
              const renaming = renamingId === tab.id;
              const menu: MenuItem[] = [
                {
                  id: "terminal.rename",
                  label: "Rename…",
                  enabled: true,
                  run: () => setRenamingId(tab.id),
                },
                {
                  id: "terminal.close",
                  label: "Close terminal",
                  enabled: canClose,
                  run: () => onClose(tab.id),
                },
              ];
              return (
                <div
                  key={tab.id}
                  data-testid={`terminal-tab-row-${tab.id}`}
                  role={renaming ? undefined : "button"}
                  tabIndex={renaming ? undefined : 0}
                  onClick={renaming ? undefined : () => activate(tab.id)}
                  onContextMenu={renaming ? undefined : (e) => cm.open(e, menu)}
                  // Keyboard parity for the right-click menu — Menu key /
                  // Shift+F10 on the focused row (`.context/usability.md`).
                  onKeyDown={renaming ? undefined : (e) => cm.openForKey(e, menu)}
                  title={tab.title}
                  style={overlayRowStyle(active)}
                >
                  <div style={overlayIconColStyle}>
                    <span style={overlayGlyphStyle}>{titleInitials(tab.title)}</span>
                  </div>
                  {renaming ? (
                    <RenameInput
                      defaultValue={tab.title}
                      testId={`terminal-tab-rename-input-${tab.id}`}
                      onCommit={(next) => {
                        setRenamingId(null);
                        onRename(tab.id, next);
                      }}
                      onCancel={() => setRenamingId(null)}
                    />
                  ) : (
                    <span
                      onDoubleClick={() => setRenamingId(tab.id)}
                      style={{
                        flex: 1,
                        fontSize: "var(--text-sm)",
                        color: active ? "var(--text-primary)" : "var(--text-secondary)",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {tab.title}
                    </span>
                  )}
                </div>
              );
            })}
            <button
              type="button"
              data-testid="terminal-tab-new-overlay"
              title="New terminal"
              onClick={onNew}
              style={overlayNewButtonStyle}
            >
              + New terminal
            </button>
          </div>
          <SlideoutChevron
            direction="collapse"
            stripWidth={STRIP_WIDTH}
            testId="terminal-tab-collapse"
            title="Hide terminals"
            onClick={strip.closePanel}
          />
        </div>
      ) : null}
      {cm.menu}
    </div>
  );
}

function RenameInput({
  defaultValue,
  testId,
  onCommit,
  onCancel,
}: {
  defaultValue: string;
  testId: string;
  onCommit(next: string): void;
  onCancel(): void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  // A blur fires on the same tick as the Escape keydown; latch the cancel
  // so blur doesn't commit after an Escape (mirrors InlineEdit).
  const canceledRef = useRef(false);
  useEffect(() => {
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      autoFocus
      data-testid={testId}
      defaultValue={defaultValue}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        // Keep Escape local: it cancels the rename, and must not also
        // reach the panel's document-level dismissal listener.
        e.stopPropagation();
        if (e.key === "Enter") {
          onCommit((e.target as HTMLInputElement).value);
        } else if (e.key === "Escape") {
          canceledRef.current = true;
          onCancel();
        }
      }}
      onBlur={(e) => {
        if (canceledRef.current) return;
        onCommit(e.target.value);
      }}
      style={renameInputStyle}
    />
  );
}

const STRIP_WIDTH = 52;
const OVERLAY_WIDTH = 220;
const ROW_HEIGHT = 40;

// The strip is a column with a pinned chevron footer; only the inner
// region scrolls, so the toggle stays reachable however long the list is.
const stripStyle: CSSProperties = {
  width: STRIP_WIDTH,
  flexShrink: 0,
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
  // A vertical control strip — neutral chrome surface (same as the
  // Navigator strip), not the blue header tint and not the near-black
  // content tier.
  background: "var(--surface-chrome)",
  borderRight: "1px solid var(--border-subtle)",
};

const stripScrollStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  display: "flex",
  flexDirection: "column",
  gap: 4,
  // No top/right padding: tiles sit flush against the top edge and the
  // right divider. A little bottom padding keeps the `+` off the floor.
  padding: "0 0 6px 0",
  overflowY: "auto",
};

function glyphButtonStyle(active: boolean): CSSProperties {
  return {
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: "100%",
    height: ROW_HEIGHT,
    border: "none",
    borderLeft: active ? "3px solid var(--accent)" : "3px solid transparent",
    // Square tabs — they fill the strip width, so no rounded right edge.
    borderRadius: 0,
    cursor: "pointer",
    fontFamily: "inherit",
    fontSize: "var(--text-sm)",
    letterSpacing: 0.3,
    background: active ? "var(--accent-soft-bg)" : "transparent",
    color: active ? "var(--text-primary)" : "var(--text-secondary)",
    fontWeight: active ? 700 : 500,
  };
}

const newButtonStyle: CSSProperties = {
  flexShrink: 0,
  width: "100%",
  height: 32,
  border: "1px dashed var(--border-subtle)",
  borderRadius: 6,
  background: "transparent",
  color: "var(--text-secondary)",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};

const overlayStyle: CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  height: "100%",
  width: STRIP_WIDTH + OVERLAY_WIDTH,
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
  background: "var(--surface-chrome)",
  borderRight: "1px solid var(--border-strong)",
  boxShadow: "8px 0 24px rgba(0, 0, 0, 0.45)",
  zIndex: 30,
};

const overlayScrollStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  display: "flex",
  flexDirection: "column",
  gap: 4,
  padding: "0 6px 6px 0",
  overflowY: "auto",
};

function overlayRowStyle(active: boolean): CSSProperties {
  return {
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    gap: 8,
    height: ROW_HEIGHT,
    cursor: "pointer",
    paddingRight: 6,
    borderLeft: active ? "3px solid var(--accent)" : "3px solid transparent",
    borderRadius: 0,
    background: active ? "var(--accent-soft-bg)" : "transparent",
    transition: "background 120ms ease",
  };
}

// Width matches the strip so the glyph renders at the same x before and
// after the panel opens (minus the 3px active stripe the row owns).
const overlayIconColStyle: CSSProperties = {
  width: STRIP_WIDTH - 3,
  flexShrink: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const overlayGlyphStyle: CSSProperties = {
  fontSize: "var(--text-sm)",
  letterSpacing: 0.3,
  color: "var(--text-primary)",
};

const renameInputStyle: CSSProperties = {
  flex: 1,
  boxSizing: "border-box",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--accent)",
  borderRadius: 4,
  padding: "3px 6px",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};

const overlayNewButtonStyle: CSSProperties = {
  flexShrink: 0,
  marginLeft: STRIP_WIDTH - 3,
  marginRight: 6,
  height: 32,
  border: "1px dashed var(--border-subtle)",
  borderRadius: 6,
  background: "transparent",
  color: "var(--text-secondary)",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};
