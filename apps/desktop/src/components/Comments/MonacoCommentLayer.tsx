/* eslint-disable @typescript-eslint/no-explicit-any */
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";

import { createComment, relinkComment, setCommentAnchor } from "../../api.js";
import {
  clearCommentReveal,
  peekPendingCommentReveal,
  requestCommentReveal,
  subscribeCommentReveal,
} from "../../comment-reveal-bus.js";
import { partitionPageComments, stepComment } from "./pageCommentNav.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { extractContext, resolveAnchor } from "./anchor.js";
import { CommentPopover } from "./CommentPopover.js";
import { NewCommentPopover } from "./NewCommentPopover.js";
import { useCommentsForTarget } from "./useCommentsForTarget.js";

interface PendingSel {
  quote: string;
  anchorJson: string;
  rect: DOMRect;
}

/// Imperative surface EditorPane drives from its right-click menu.
export interface MonacoCommentHandle {
  /// The comment id whose decoration covers `position`, or null.
  commentIdAt(position: any): string | null;
  /// Open the composer for the current editor selection (if non-empty).
  addCommentForSelection(): void;
  /// Open the thread popover for `commentId`.
  openComment(commentId: string): void;
  /// Orphaned comments for this file — drives "Relink orphaned" menu
  /// entries (the escape hatch when a quote drifted past fuzzy tolerance).
  relinkTargets(): { id: string; quote: string }[];
  /// Re-attach `commentId` to the editor's current selection.
  relinkToSelection(commentId: string): void;
}

/// Build the enriched `anchor_json` for a resolved Monaco range: the
/// line/col fast-path hint plus the text offset and prefix/suffix
/// context the resolver uses, and whether this was a fuzzy (approximate)
/// match. Recomputed from the resolved location on every re-anchor so
/// the stored hint + context self-heal.
function buildAnchorJson(model: any, text: string, range: any, approx: boolean): string {
  const startOffset: number = model.getOffsetAt(range.getStartPosition());
  const endOffset: number = model.getOffsetAt(range.getEndPosition());
  const { prefix, suffix } = extractContext(text, startOffset, endOffset);
  return JSON.stringify({
    startLine: range.startLineNumber,
    startColumn: range.startColumn,
    endLine: range.endLineNumber,
    endColumn: range.endColumn,
    prefix,
    suffix,
    textOffset: startOffset,
    approx,
  });
}

/// Comment overlay for the Monaco editor. Renders inline highlights and
/// owns the composer + thread popover, but no longer reacts to plain
/// clicks or selection — creation/opening are driven by EditorPane's
/// right-click menu via the imperative handle (so comments don't fight
/// cursor placement or text selection).
export const MonacoCommentLayer = forwardRef<
  MonacoCommentHandle,
  {
    editor: any;
    monaco: any;
    ready: boolean;
    streamId: string;
    threadId: string | null;
    filePath: string;
  }
>(function MonacoCommentLayer({ editor, monaco, ready, streamId, threadId, filePath }, ref) {
  const { threads } = useCommentsForTarget("file", filePath);
  const threadsRef = useRef(threads);
  threadsRef.current = threads;
  const decoIdsRef = useRef<string[]>([]);
  const decoMapRef = useRef<{ decoId: string; commentId: string }[]>([]);
  const [active, setActive] = useState<{ id: string; rect: DOMRect } | null>(null);
  const [pending, setPending] = useState<PendingSel | null>(null);
  // Bumped when the model's content or the model itself changes. The
  // file's content loads asynchronously after the editor mounts, so
  // without re-running the paint on content arrival a comment resolved
  // against an empty model would (wrongly) orphan and never re-anchor.
  const [contentTick, setContentTick] = useState(0);
  useEffect(() => {
    if (!ready || !editor) return;
    let timer: number | null = null;
    const bump = () => {
      if (timer != null) window.clearTimeout(timer);
      timer = window.setTimeout(() => setContentTick((t) => t + 1), 80);
    };
    const d1 = editor.onDidChangeModelContent?.(bump);
    const d2 = editor.onDidChangeModel?.(bump);
    return () => {
      if (timer != null) window.clearTimeout(timer);
      d1?.dispose?.();
      d2?.dispose?.();
    };
  }, [ready, editor]);

  // Re-anchor each comment to a Monaco range and paint inline highlights.
  useEffect(() => {
    if (!ready || !editor || !monaco) return;
    const model = editor.getModel?.();
    if (!model) return;
    const text: string = model.getValue();
    const decos: any[] = [];
    const map: { decoId: string; commentId: string }[] = [];

    for (const thread of threads) {
      const c = thread.comment;
      let range: any = null;
      let approx = false;
      let parsed: Record<string, unknown> = {};
      try {
        parsed = JSON.parse(c.selectors_json) as Record<string, unknown>;
        // Tier A: the stored range still spells the quote exactly.
        if (typeof parsed.startLine === "number") {
          const r = new monaco.Range(
            parsed.startLine as number,
            parsed.startColumn as number,
            parsed.endLine as number,
            parsed.endColumn as number,
          );
          if (model.getValueInRange(r) === c.quote) range = r;
        }
      } catch {
        // fall through to the resolver
      }
      if (!range) {
        // Tiers B/C: exact (disambiguated by context + proximity) then
        // bounded fuzzy near the stored offset.
        const res = resolveAnchor(text, {
          quote: c.quote,
          prefix: typeof parsed.prefix === "string" ? parsed.prefix : undefined,
          suffix: typeof parsed.suffix === "string" ? parsed.suffix : undefined,
          hintOffset: typeof parsed.textOffset === "number" ? parsed.textOffset : undefined,
        });
        if (res.offset !== null) {
          const start = model.getPositionAt(res.offset);
          const end = model.getPositionAt(res.offset + res.length);
          range = new monaco.Range(start.lineNumber, start.column, end.lineNumber, end.column);
          approx = res.confidence === "fuzzy";
        }
      }
      if (range) {
        decos.push({
          range,
          options: {
            inlineClassName: approx
              ? "oxplow-comment-highlight oxplow-comment-highlight--approx"
              : "oxplow-comment-highlight",
            stickiness: 1,
          },
        });
        // Persist the enriched anchor recomputed from the resolved
        // location so the position hint + context self-heal (and old
        // comments upgrade in place). The equality guard keeps churn down.
        const aj = buildAnchorJson(model, text, range, approx);
        if (c.orphaned || c.selectors_json !== aj) void setCommentAnchor(c.id, aj, false);
        map.push({ decoId: "", commentId: c.id });
      } else if (!c.orphaned) {
        void setCommentAnchor(c.id, c.selectors_json, true);
      }
    }

    const ids: string[] = editor.deltaDecorations(decoIdsRef.current, decos);
    decoIdsRef.current = ids;
    decoMapRef.current = ids.map((decoId, i) => ({ decoId, commentId: map[i]?.commentId ?? "" }));
  }, [ready, editor, monaco, threads, filePath, contentTick]);

  // Build a viewport rect from a Monaco position (read model/editor live).
  const rectAtPosition = (position: any): DOMRect | null => {
    const vis = editor?.getScrolledVisiblePosition?.(position);
    const dom = editor?.getDomNode?.();
    if (!vis || !dom) return null;
    const host = dom.getBoundingClientRect();
    return new DOMRect(host.left + vis.left, host.top + vis.top, 0, vis.height);
  };

  // Capture the current selection as a quote + enriched anchor_json.
  // Shared by "Add comment" and "Relink orphaned".
  const captureSelection = (): { quote: string; anchorJson: string } | null => {
    const sel = editor?.getSelection?.();
    const model = editor?.getModel?.();
    if (!sel || !model || sel.isEmpty()) return null;
    const raw: string = model.getValueInRange(sel);
    const quote = raw.trim();
    if (!quote) return null;
    // Locate the trimmed quote inside the raw selection so context/offset
    // line up with the text the resolver searches.
    const text: string = model.getValue();
    const lead = raw.length - raw.trimStart().length;
    const quoteStart = model.getOffsetAt(sel.getStartPosition()) + lead;
    const startPos = model.getPositionAt(quoteStart);
    const endPos = model.getPositionAt(quoteStart + quote.length);
    const { prefix, suffix } = extractContext(text, quoteStart, quoteStart + quote.length);
    return {
      quote,
      anchorJson: JSON.stringify({
        startLine: startPos.lineNumber,
        startColumn: startPos.column,
        endLine: endPos.lineNumber,
        endColumn: endPos.column,
        prefix,
        suffix,
        textOffset: quoteStart,
        approx: false,
      }),
    };
  };

  // Honor cross-page "go to location" requests from the Comments
  // dashboard. We only act when the pending comment is one of ours; the
  // request stays on the bus until the decoration is painted (the
  // re-anchor effect above runs first), so a slow threads fetch after
  // navigation still resolves. `revealTick` re-runs this when a new
  // request arrives even if the thread list is unchanged.
  const [revealTick, setRevealTick] = useState(0);
  useEffect(() => subscribeCommentReveal(() => setRevealTick((t) => t + 1)), []);
  useEffect(() => {
    const id = peekPendingCommentReveal();
    if (id == null || !ready) return;
    const target = threadsRef.current.find((t) => t.comment.id === id);
    if (!target) return;
    const model = editor?.getModel?.();
    const entry = decoMapRef.current.find((e) => e.commentId === id);
    const range = entry && model ? model.getDecorationRange(entry.decoId) : null;
    if (range) {
      editor.revealRangeInCenter(range);
      setActive({ id, rect: rectAtPosition(range.getStartPosition()) ?? new DOMRect(120, 120, 0, 0) });
      clearCommentReveal(id);
    } else if (target.comment.orphaned) {
      // Orphaned: no decoration to scroll to — open the popover anyway so
      // the user can read it and relink.
      const host = editor?.getDomNode?.()?.getBoundingClientRect?.();
      setActive({
        id,
        rect: host ? new DOMRect(host.left + 24, host.top + 24, 0, 0) : new DOMRect(120, 120, 0, 0),
      });
      clearCommentReveal(id);
    }
    // else: anchored but decoration not painted yet — retry on next deps change.
  }, [revealTick, threads, ready, editor, contentTick]);

  useImperativeHandle(
    ref,
    (): MonacoCommentHandle => ({
      commentIdAt(position) {
        const model = editor?.getModel?.();
        if (!model || !position) return null;
        for (const { decoId, commentId } of decoMapRef.current) {
          const r = model.getDecorationRange(decoId);
          if (r && r.containsPosition(position)) return commentId;
        }
        return null;
      },
      addCommentForSelection() {
        const sel = editor?.getSelection?.();
        if (!sel) return;
        const captured = captureSelection();
        if (!captured) return;
        const rect = rectAtPosition(sel.getEndPosition());
        if (!rect) return;
        setActive(null);
        setPending({ quote: captured.quote, anchorJson: captured.anchorJson, rect });
      },
      openComment(commentId) {
        const model = editor?.getModel?.();
        const entry = decoMapRef.current.find((e) => e.commentId === commentId);
        const r = entry && model ? model.getDecorationRange(entry.decoId) : null;
        const rect = r ? rectAtPosition(r.getStartPosition()) : null;
        setPending(null);
        setActive({ id: commentId, rect: rect ?? new DOMRect(120, 120, 0, 0) });
      },
      relinkTargets() {
        return threadsRef.current
          .filter((t) => t.comment.orphaned)
          .map((t) => ({ id: t.comment.id, quote: t.comment.quote }));
      },
      relinkToSelection(commentId) {
        const captured = captureSelection();
        if (captured) void relinkComment(commentId, captured.quote, captured.anchorJson);
      },
    }),
    [editor],
  );

  const handleCreate = async (input: { body: string; intent: CommentIntent }) => {
    if (!pending) return;
    // No explicit context_chain/referenced_refs: a file editor has no
    // typed ancestors above the file itself, and the backend already
    // unions any refs it can parse out of the quoted code (paths,
    // `task:N`, …) into referenced_refs at create time — so we don't
    // reimplement ref parsing here.
    await createComment({
      streamId,
      threadId,
      targetKind: "file",
      targetId: filePath,
      quote: pending.quote,
      selectorsJson: pending.anchorJson,
      intent: input.intent,
      author: "user",
      body: input.body,
    });
    setPending(null);
  };

  const activeThread = active != null ? threads.find((t) => t.comment.id === active.id) : undefined;

  return (
    <>
      {pending && (
        <NewCommentPopover
          rect={pending.rect}
          onCreate={handleCreate}
          onDismiss={() => setPending(null)}
        />
      )}
      {active && activeThread && (
        <CommentPopover
          thread={activeThread}
          anchorRect={active.rect}
          onClose={() => setActive(null)}
          onStep={
            partitionPageComments(threads).jumpable.length >= 2
              ? (dir) => {
                  const next = stepComment(threads, active.id, dir);
                  if (next != null) requestCommentReveal(next);
                }
              : undefined
          }
          onRelink={
            activeThread.comment.orphaned
              ? () => {
                  const captured = captureSelection();
                  if (captured) {
                    void relinkComment(active.id, captured.quote, captured.anchorJson);
                    setActive(null);
                  }
                }
              : undefined
          }
        />
      )}
    </>
  );
});
