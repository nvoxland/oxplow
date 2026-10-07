import type { CSSProperties } from "react";
import { ChevronRight, Search } from "lucide-react";

import { vcsCheckoutBranch } from "../api.js";
import { requestNavigatorOpen } from "../navigator-bus.js";
import type { Stream } from "../tauri-bridge/index.js";
import { BranchPicker, type PickedRef } from "./BranchPicker.js";

/** The search field's testid. The search overlay opens over it (finding it
 *  by this id), so it reads as the field expanding in place. */
export const SEARCH_TRIGGER_TESTID = "title-bar-search";

interface Props {
  stream: Stream | null;
  thread: { id: string; title: string } | null;
  vcsEnabled: boolean;
  /** Room left at the start for the window's own controls (macOS traffic
   *  lights float over this bar). */
  leftInset: number;
  onOpenSearch(): void;
}

/**
 * The bar across the top of the window, over both the left nav and the
 * content: where you are (`stream › thread` at the start) and, at the
 * right end, the stream's branch and the global search beside it. The
 * stream and thread names open the navigator below them, the branch the
 * branch picker.
 *
 * Its empty space drags the window (`data-tauri-drag-region` — Tauri drags
 * only from elements that carry it, so the controls stay clickable).
 * Alerts and background tasks stay in the bottom bar.
 */
export function TitleBar({ stream, thread, vcsEnabled, leftInset, onOpenSearch }: Props) {
  return (
    <div
      data-testid="title-bar"
      data-tauri-drag-region
      style={{
        height: 30,
        flexShrink: 0,
        display: "grid",
        // Context (and the drag space after it) | branch | search, the
        // last two pinned right.
        gridTemplateColumns: "minmax(0, 1fr) auto clamp(180px, 28vw, 320px)",
        alignItems: "center",
        gap: 12,
        padding: `0 10px 0 ${leftInset}px`,
        background: "var(--surface-chrome)",
        fontSize: "var(--text-xs)",
      }}
    >
      <div data-tauri-drag-region style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0, height: "100%" }}>
        {stream ? (
          <>
            <button
              type="button"
              data-testid="title-bar-stream"
              title={`${stream.title} — show streams and threads`}
              onClick={() => requestNavigatorOpen()}
              style={{ ...nameStyle, fontWeight: 600, color: "var(--text-primary)" }}
            >
              {stream.title}
            </button>
            {thread ? (
              <>
                <ChevronRight size={12} aria-hidden style={{ flexShrink: 0, color: "var(--text-muted)" }} />
                <button
                  type="button"
                  data-testid="title-bar-thread"
                  title={`${thread.title} — show streams and threads`}
                  onClick={() => requestNavigatorOpen()}
                  style={{ ...nameStyle, color: "var(--text-secondary)" }}
                >
                  {thread.title}
                </button>
              </>
            ) : null}
          </>
        ) : null}
      </div>
      <div style={{ display: "flex", alignItems: "center" }}>
        {stream ? <BranchChip stream={stream} vcsEnabled={vcsEnabled} /> : null}
      </div>
      <button
        type="button"
        data-testid={SEARCH_TRIGGER_TESTID}
        onClick={onOpenSearch}
        className="oxplow-title-search"
        style={searchStyle}
      >
        <Search size={13} aria-hidden style={{ flexShrink: 0 }} />
        <span style={{ flex: 1, textAlign: "left" }}>Search…</span>
        <kbd style={kbdStyle}>⌘K</kbd>
      </button>
    </div>
  );
}

/** The stream's branch, opening the branch picker (checkout, manage). */
function BranchChip({ stream, vcsEnabled }: { stream: Stream; vcsEnabled: boolean }) {
  const title = vcsEnabled ? `Branch: ${stream.branch} (click to switch)` : "Git not enabled for this workspace";
  async function handlePick(target: PickedRef) {
    // Tags and remote refs are checked out via their local-name form; git
    // creates a tracking branch / detached HEAD as appropriate. The branch
    // reconciler records the new branch on the stream.
    await vcsCheckoutBranch(stream.id, target.name);
  }
  return (
    <BranchPicker
      label={
        <>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ display: "block" }}>
            <line x1="6" y1="3" x2="6" y2="15" />
            <circle cx="18" cy="6" r="3" />
            <circle cx="6" cy="18" r="3" />
            <path d="M18 9a9 9 0 0 1-9 9" />
          </svg>
          <span style={{ marginLeft: 6 }}>{stream.branch}</span>
        </>
      }
      title={title}
      currentBranch={stream.branch}
      disabled={!vcsEnabled}
      anchor="bottom"
      align="right"
      mode="manage"
      streamId={stream.id}
      onPick={handlePick}
    />
  );
}

const nameStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  padding: "2px 4px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "inherit",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  minWidth: 0,
};

// Background, color and border come from `.oxplow-title-search` so its
// `:hover` can change them.
const searchStyle: CSSProperties = {
  height: 22,
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "0 8px",
  borderRadius: 6,
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
  minWidth: 0,
};

const kbdStyle: CSSProperties = {
  fontSize: 10,
  color: "var(--text-muted)",
  background: "var(--surface-tab-inactive)",
  padding: "0 5px",
  borderRadius: 3,
  border: "1px solid var(--border-subtle)",
  fontFamily: "inherit",
};
