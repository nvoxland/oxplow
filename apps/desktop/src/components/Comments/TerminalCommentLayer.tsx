//! Comment layer for the xterm.js terminal / agent pane.
//!
//! The terminal owns its own selection UX (it's in the editor/terminal
//! carve-out, so the app-level `DomCommentLayer` ignores it). This layer
//! mirrors the Monaco layer's shape for a different coordinate space:
//!   - capture a buffer-text selection into a comment (quote + W3C
//!     selectors + a `TerminalBufferSelector`, target = the terminal/agent
//!     page ref);
//!   - re-anchor each comment's quote in the live buffer and paint a
//!     best-effort xterm decoration on its line(s); clicking it reopens
//!     the thread.
//!
//! Scrollback is ephemeral and wraps/evicts, so terminal anchors orphan
//! readily — that's expected. Decoration placement is wrapped in
//! try/catch so a coordinate edge case degrades to "no highlight" rather
//! than disturbing the terminal.

import { useCallback, useEffect, useRef, useState } from "react";
import type { IDecoration, Terminal } from "@xterm/xterm";

import { createComment } from "../../api.js";
import { timed } from "../../logger.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { CommentPopover } from "./CommentPopover.js";
import { NewCommentPopover } from "./NewCommentPopover.js";
import { SelectionCommentToolbar } from "./SelectionCommentToolbar.js";
import { planRepaint, REPAINT_MIN_INTERVAL_MS } from "./terminalRepaintSchedule.js";
import {
  buildTerminalSelectorsJson,
  coordToOffset,
  reanchorInBuffer,
  serializeBuffer,
  type SerializedBuffer,
} from "./terminalAnchor.js";
import { useCommentsForTarget } from "./useCommentsForTarget.js";

/// Monotonic-ish clock for repaint throttling; falls back to Date.now
/// where performance.now is unavailable (e.g. non-browser test env).
function nowMs(): number {
  return globalThis.performance?.now?.() ?? Date.now();
}

/// Flatten the terminal's active buffer (scrollback + viewport) into a
/// [`SerializedBuffer`]. `getLine(i)` is absolute over the whole buffer,
/// matching the coordinates `getSelectionPosition` returns.
function serializeTermBuffer(term: Terminal): SerializedBuffer {
  const buf = term.buffer.active;
  const lines: string[] = [];
  for (let i = 0; i < buf.length; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) ?? "");
  }
  return serializeBuffer(lines);
}

/// Viewport rect of the current DOM selection (xterm uses the DOM
/// renderer, so selections produce a real Range). Falls back to a corner
/// of the terminal element when unavailable.
function selectionRect(term: Terminal): DOMRect {
  const sel = window.getSelection();
  if (sel && sel.rangeCount > 0) {
    const r = sel.getRangeAt(0).getBoundingClientRect();
    if (r.width || r.height) return r;
  }
  const host = term.element?.getBoundingClientRect();
  return host ? new DOMRect(host.left + 24, host.top + 24, 0, 0) : new DOMRect(80, 80, 0, 0);
}

interface Pending {
  quote: string;
  selectorsJson: string;
  rect: DOMRect;
}

interface Painted {
  commentId: string;
  decoration: IDecoration;
}

export function TerminalCommentLayer({
  term,
  streamId,
  threadId,
  targetKind,
  targetId,
}: {
  term: Terminal | null;
  streamId: string;
  threadId: string | null;
  targetKind: string;
  targetId: string;
}) {
  const { threads } = useCommentsForTarget(targetKind, targetId);
  const threadsRef = useRef(threads);
  threadsRef.current = threads;

  const [pending, setPending] = useState<Pending | null>(null);
  const [composing, setComposing] = useState(false);
  const [openId, setOpenId] = useState<{ id: string; rect: DOMRect } | null>(null);
  const paintedRef = useRef<Painted[]>([]);

  // Capture a buffer-text selection into a pending comment.
  useEffect(() => {
    if (!term) return;
    const disp = term.onSelectionChange(() => {
      if (composing) return;
      const raw = term.getSelection();
      if (!raw.trim()) {
        setPending(null);
        return;
      }
      const pos = term.getSelectionPosition();
      if (!pos) return;
      const sbuf = serializeTermBuffer(term);
      const start = coordToOffset(sbuf, pos.start.y, pos.start.x);
      const end = coordToOffset(sbuf, pos.end.y, pos.end.x);
      // Derive the durable quote from the serialized buffer slice (not
      // term.getSelection(), which de-wraps) so the stored quote uses the
      // same wrapping convention the re-anchor search text does.
      const quote = sbuf.text.slice(start, end).trim();
      if (!quote) return;
      const selectorsJson = buildTerminalSelectorsJson(sbuf, start, end);
      setPending({ quote, selectorsJson, rect: selectionRect(term) });
    });
    return () => disp.dispose();
  }, [term, composing]);

  // Re-anchor every comment's quote in the current buffer and paint a
  // best-effort decoration on its starting line. Re-runs on buffer
  // writes and scroll so highlights track the live scrollback.
  const repaint = useCallback(() => {
    if (!term) return;
    for (const p of paintedRef.current) {
      try {
        p.decoration.dispose();
      } catch {
        /* already disposed */
      }
    }
    paintedRef.current = [];

    // Nothing to anchor → skip the whole-buffer serialize + reanchor. This
    // is the common case on the agent pane (a streaming terminal with no
    // comments), and serializing a ~5000-line scrollback every frame is
    // what stalls the main thread while output streams.
    if (threads.length === 0) return;

    const active = term.buffer.active;
    // Serialize + reanchor is the expensive part (whole-scrollback string +
    // a scan per comment). Self-time it so a future regression, or a
    // pathological buffer/comment count, self-reports as a `slow operation`
    // WARN naming the buffer + comment sizes — the drift watchdog can only
    // report that *something* froze, never what.
    timed(
      "terminal-repaint",
      () => {
        const sbuf = serializeTermBuffer(term);
        const cursorAbs = active.baseY + active.cursorY;
        const painted: Painted[] = [];
        for (const t of threads) {
          const c = t.comment;
          const anchor = reanchorInBuffer(sbuf, c.selectors_json, c.quote);
          if (!anchor) continue;
          try {
            // registerMarker is relative to the cursor's absolute line.
            const marker = term.registerMarker(anchor.startLine - cursorAbs);
            if (!marker) continue;
            const width = Math.max(
              1,
              (anchor.endLine > anchor.startLine ? term.cols : anchor.endCol) - anchor.startCol,
            );
            const decoration = term.registerDecoration({
              marker,
              x: anchor.startCol,
              width,
            });
            if (!decoration) {
              marker.dispose();
              continue;
            }
            const commentId = c.id;
            const approx = anchor.confidence === "fuzzy";
            decoration.onRender((el) => {
              el.classList.add("oxplow-terminal-comment");
              if (approx) el.classList.add("oxplow-terminal-comment--approx");
              el.style.cursor = "pointer";
              el.onclick = () => setOpenId({ id: commentId, rect: el.getBoundingClientRect() });
            });
            painted.push({ commentId, decoration });
          } catch {
            /* coordinate out of range — skip painting this one */
          }
        }
        paintedRef.current = painted;
      },
      { context: () => ({ bufferLines: active.length, commentThreads: threads.length }) },
    );
  }, [term, threads]);

  useEffect(() => {
    if (!term) return;
    repaint();
    // Throttle the write firehose (agent output streams a line at a time) to
    // at most one repaint per REPAINT_MIN_INTERVAL_MS. Coalescing to one
    // repaint *per frame* still let a busy terminal thrash — each repaint
    // re-serializes the full ~5000-line scrollback and re-anchors every
    // comment, which alone can exceed a frame budget, so per-frame repaints
    // ran back-to-back and stalled the main thread. A trailing repaint keeps
    // the final state correct once the stream quiets.
    let lastRun = 0;
    let trailing: ReturnType<typeof setTimeout> | null = null;
    const run = () => {
      trailing = null;
      lastRun = nowMs();
      repaint();
    };
    const schedule = () => {
      if (trailing) return; // a trailing repaint is already queued
      const plan = planRepaint(lastRun, nowMs(), REPAINT_MIN_INTERVAL_MS);
      if (plan.run === "now") run();
      else trailing = setTimeout(run, plan.waitMs);
    };
    const onData = term.onWriteParsed(schedule);
    const onScroll = term.onScroll(schedule);
    return () => {
      if (trailing) clearTimeout(trailing);
      onData.dispose();
      onScroll.dispose();
      for (const p of paintedRef.current) {
        try {
          p.decoration.dispose();
        } catch {
          /* ignore */
        }
      }
      paintedRef.current = [];
    };
  }, [term, repaint]);

  const handleCreate = async (input: { body: string; intent: CommentIntent }) => {
    if (!pending) return;
    await createComment({
      streamId,
      threadId,
      targetKind,
      targetId,
      quote: pending.quote,
      selectorsJson: pending.selectorsJson,
      intent: input.intent,
      author: "user",
      body: input.body,
    });
    setComposing(false);
    setPending(null);
  };

  const openThread = openId ? threads.find((t) => t.comment.id === openId.id) : undefined;

  return (
    <div data-selection-comment-ui>
      {pending && !composing && (
        <SelectionCommentToolbar rect={pending.rect} onAdd={() => setComposing(true)} />
      )}
      {pending && composing && (
        <NewCommentPopover
          rect={pending.rect}
          onCreate={handleCreate}
          onDismiss={() => {
            setComposing(false);
            setPending(null);
          }}
        />
      )}
      {openId && openThread && (
        <CommentPopover
          thread={openThread}
          anchorRect={openId.rect}
          onClose={() => setOpenId(null)}
        />
      )}
    </div>
  );
}
