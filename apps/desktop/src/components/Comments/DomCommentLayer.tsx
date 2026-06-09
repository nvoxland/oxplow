//! Generic comment layer for plain-DOM pages.
//!
//! Mounted ONCE at the app level (see `App.tsx`), not per page: it serves
//! every plain-DOM surface whose regions declare `data-ref-*` context
//! nodes (see `contextNodes.tsx`). A selection only becomes a comment
//! when it lands inside such a region and outside the editor/terminal
//! carve-out, so a single instance is safe everywhere. It:
//!   - captures selections into comments (via `useDomAnnotations`);
//!   - paints existing comments back onto their context-node element's
//!     text using the CSS Custom Highlight API (no DOM mutation);
//!   - opens a thread popover when a highlight is clicked.
//!
//! It is target-agnostic: it shows every stream comment whose
//! `(target_kind, target_id)` matches a context node currently on the
//! page, so the same component serves the tasks list, the file list, the
//! git dashboard, etc. Editor surfaces (Monaco, ProseMirror) keep their
//! own layers — this one deliberately skips their subtrees.

import { useCallback, useEffect, useRef, useState } from "react";

import { createComment, listCommentsForStream, subscribeCommentEvents } from "../../api.js";
import type { CommentIntent, CommentThread } from "../../tauri-bridge/generated/bindings.js";
import { resolveAnchor } from "./anchor.js";
import { CommentPopover } from "./CommentPopover.js";
import { clearCommentHighlights, paintCommentHighlights } from "./cssHighlight.js";
import { textContentRange } from "./domAnchor.js";
import { NewCommentPopover } from "./NewCommentPopover.js";
import { anchorInputFromSelectors } from "./selectors.js";
import { SelectionCommentToolbar } from "./SelectionCommentToolbar.js";
import { useDomAnnotations } from "./useDomAnnotations.js";

function cssEscape(value: string): string {
  const fn = (globalThis as { CSS?: { escape?: (v: string) => string } }).CSS?.escape;
  return fn ? fn(value) : value.replace(/["\\]/g, "\\$&");
}

/// Find the on-page element that declares a comment's target context
/// node, or null when that target isn't currently rendered.
function targetElement(kind: string, id: string): Element | null {
  return document.querySelector(
    `[data-ref-kind="${cssEscape(kind)}"][data-ref-id="${cssEscape(id)}"]`,
  );
}

interface Painted {
  commentId: string;
  range: Range;
}

export function DomCommentLayer({
  streamId,
  threadId,
}: {
  streamId: string;
  threadId: string | null;
}) {
  const { pending, composing, beginCompose, cancel } = useDomAnnotations();
  const [threads, setThreads] = useState<CommentThread[]>([]);
  const [openId, setOpenId] = useState<{ id: string; rect: DOMRect } | null>(null);
  const paintedRef = useRef<Painted[]>([]);

  // Load + keep stream comments live.
  const reload = useCallback(async () => {
    setThreads(await listCommentsForStream(streamId));
  }, [streamId]);
  useEffect(() => {
    void reload();
    return subscribeCommentEvents(() => void reload());
  }, [reload]);

  // Re-resolve each comment's quote against its on-page context node and
  // repaint. Runs on comment changes and whenever the page DOM mutates
  // (rows re-render, the list scrolls/virtualizes) so highlights track
  // live React content — the same discipline as Monaco's contentTick.
  const repaint = useCallback(() => {
    const exact: Range[] = [];
    const approx: Range[] = [];
    const painted: Painted[] = [];
    for (const t of threads) {
      const c = t.comment;
      const el = targetElement(c.target_kind, c.target_id);
      if (!el) continue;
      const text = el.textContent ?? "";
      const res = resolveAnchor(text, anchorInputFromSelectors(c.selectors_json, c.quote));
      if (res.offset === null) continue;
      const range = textContentRange(el, res.offset, res.length);
      if (!range) continue;
      (res.confidence === "fuzzy" ? approx : exact).push(range);
      painted.push({ commentId: c.id, range });
    }
    paintedRef.current = painted;
    paintCommentHighlights(exact, approx);
  }, [threads]);

  useEffect(() => {
    repaint();
    // Coalesce the firehose of DOM mutations (terminal output, spinners,
    // list re-renders) into one repaint per animation frame.
    let frame = 0;
    const schedule = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        repaint();
      });
    };
    const obs = new MutationObserver(schedule);
    obs.observe(document.body, { childList: true, characterData: true, subtree: true });
    window.addEventListener("scroll", schedule, true);
    window.addEventListener("resize", schedule);
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      obs.disconnect();
      window.removeEventListener("scroll", schedule, true);
      window.removeEventListener("resize", schedule);
      clearCommentHighlights();
    };
  }, [repaint]);

  // Open a thread when its highlight is clicked. CSS highlights aren't
  // hit-testable, so we test the click point against each painted range's
  // client rects.
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if ((e.target as Element | null)?.closest?.("[data-selection-comment-ui]")) return;
      for (const p of paintedRef.current) {
        for (const r of Array.from(p.range.getClientRects())) {
          if (
            e.clientX >= r.left &&
            e.clientX <= r.right &&
            e.clientY >= r.top &&
            e.clientY <= r.bottom
          ) {
            setOpenId({ id: p.commentId, rect: new DOMRect(e.clientX, e.clientY, 0, 0) });
            return;
          }
        }
      }
    };
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, []);

  const handleCreate = async (input: { body: string; intent: CommentIntent }) => {
    if (!pending) return;
    await createComment({
      streamId,
      threadId,
      targetKind: pending.targetKind,
      targetId: pending.targetId,
      quote: pending.quote,
      selectorsJson: pending.selectorsJson,
      contextChain: pending.contextChain,
      referencedRefs: pending.referencedRefs,
      intent: input.intent,
      author: "user",
      body: input.body,
    });
    cancel();
  };

  const openThread = openId ? threads.find((t) => t.comment.id === openId.id) : undefined;

  return (
    <div data-selection-comment-ui>
      {pending && !composing && (
        <SelectionCommentToolbar rect={pending.rect} onAdd={beginCompose} />
      )}
      {pending && composing && (
        <NewCommentPopover rect={pending.rect} onCreate={handleCreate} onDismiss={cancel} />
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
