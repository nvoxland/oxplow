import { useEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import type { Node as PMNode } from "@tiptap/pm/model";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Table from "@tiptap/extension-table";
import TableRow from "@tiptap/extension-table-row";
import TableHeader from "@tiptap/extension-table-header";
import TableCell from "@tiptap/extension-table-cell";
import { Markdown } from "tiptap-markdown";
import { Pencil } from "lucide-react";
import { InternalLink } from "./InternalLink.js";
import { MermaidBlock } from "./MermaidBlock.js";
import {
  CommentDecorations,
  commentDecorationsKey,
  findCommentRange,
  flatten,
  type CommentRange,
} from "./CommentDecorations.js";
import { createComment, relinkComment, setCommentAnchor } from "../../api.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { extractContext } from "../Comments/anchor.js";
import { partitionPageComments, stepComment } from "../Comments/pageCommentNav.js";
import { refsInRange } from "../Comments/domAnchor.js";
import type { RefNode } from "../Comments/contextNodes.js";
import { useCommentsForTarget } from "../Comments/useCommentsForTarget.js";
import {
  clearCommentReveal,
  peekPendingCommentReveal,
  requestCommentReveal,
  subscribeCommentReveal,
} from "../../comment-reveal-bus.js";
import { CommentPopover } from "../Comments/CommentPopover.js";
import { NewCommentPopover } from "../Comments/NewCommentPopover.js";
import { SelectionCommentToolbar } from "../Comments/SelectionCommentToolbar.js";
import { selectionToolbarVisible } from "./selectionToolbar.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { parseMarkdownLink } from "../Wiki/MarkdownView.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef, directoryRef, gitCommitRef, wikiPageRef } from "../../tabs/pageRefs.js";
import { DISK } from "../../file-version.js";

/// Comment integration bundle. When provided, the field highlights
/// anchored ranges and exposes "Add comment" via both the floating
/// selection toolbar (mirroring the plain-DOM surfaces, which this
/// contenteditable region is carved out of) and the right-click menu;
/// "Open comment" stays right-click-only so existing comments don't
/// fight selection/cursor. `targetId` identifies the
/// page (wiki slug / task id); `streamId` is the hard scope and
/// `threadId` the origin thread (null for non-thread-bound surfaces).
export interface RichTextCommentConfig {
  streamId: string;
  threadId: string | null;
  targetKind: string;
  targetId: string;
  author?: string;
}

/// Build the enriched `anchor_json` for a resolved `[from, to)` doc
/// range: the from/to fast-path hint plus prefix/suffix/textOffset
/// (recomputed from the same flattened text the resolver searches) and
/// whether this was a fuzzy (approximate) match.
function buildAnchorJson(doc: PMNode, from: number, to: number, approx: boolean): string {
  const { text, map } = flatten(doc);
  let startOff = map.findIndex((p) => p >= from);
  if (startOff === -1) startOff = map.length;
  let endOff = map.findIndex((p) => p >= to);
  if (endOff === -1) endOff = map.length;
  const { prefix, suffix } = extractContext(text, startOff, endOff);
  return JSON.stringify({ from, to, prefix, suffix, textOffset: startOff, approx });
}

interface PendingSelection {
  quote: string;
  from: number;
  to: number;
  rect: DOMRect;
  /// Enriched anchor_json (from/to + prefix/suffix/textOffset) captured
  /// from the same flattened text the resolver searches.
  anchorJson: string;
  /// Canonical refs the selection's rendered links point at (e.g. an
  /// internal `[[wikilink]]` Tiptap rendered as an `<a>`). Captured from
  /// the live DOM selection at compose time so the agent sees typed
  /// context for what the highlighted prose links to. The backend
  /// additionally unions refs it can parse out of the quote text itself.
  referencedRefs: RefNode[];
}

/**
 * Shared rich-text editor surface. One instance per editable region
 * (title saves to one field, description to another, etc.) — the page
 * composes them at the React level.
 *
 * Storage stays markdown. tiptap-markdown handles GFM round-trip on
 * mount and on save; the `MermaidBlock` NodeView paints rendered SVG
 * over the editable fenced code, so users see the diagram unless they
 * click into it.
 *
 * Save model: debounced 300ms while typing, and immediate on blur. The
 * `onCommit` callback is responsible for the actual persistence.
 *
 * Pencil affordance: a small `Pencil` icon sits in the top-right of
 * the editor surface, opacity ~0.4 by default, full opacity on hover
 * or focus. Read-only blocks elsewhere on the page must not show this
 * — that's the visual signal "this is for reading."
 */
export interface RichTextFieldProps {
  value: string;
  onCommit: (markdown: string) => void;
  placeholder?: string;
  /** Disable headings/blocks for inline-only fields (e.g. a wiki page
   *  title). Default false. */
  inlineOnly?: boolean;
  /** Optional className applied to the wrapper. */
  className?: string;
  style?: CSSProperties;
  /** When true, no pencil affordance (e.g. effort summaries — but
   *  those should use MarkdownView, not this field). Default false. */
  hidePencil?: boolean;
  /** When set, the field becomes comment-enabled (highlights, the
   *  selection affordance, and the thread popover). */
  comments?: RichTextCommentConfig;
}

export function RichTextField({
  value,
  onCommit,
  placeholder,
  inlineOnly = false,
  className,
  style,
  hidePencil,
  comments,
}: RichTextFieldProps) {
  const lastCommittedRef = useRef(value);
  const debounceRef = useRef<number | null>(null);

  // Comment state. The hook is always called (empty target → no fetch).
  const { threads } = useCommentsForTarget(comments?.targetKind ?? "", comments?.targetId ?? "");
  const [activeComment, setActiveComment] = useState<{ id: string; rect: DOMRect } | null>(null);
  const [pendingSel, setPendingSel] = useState<PendingSelection | null>(null);
  const [commentMenu, setCommentMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(
    null,
  );
  // Floating "Add comment" toolbar over a fresh selection (mouse-driven,
  // like the plain-DOM layer). Null when hidden.
  const [selToolbarRect, setSelToolbarRect] = useState<DOMRect | null>(null);

  const editor = useEditor({
    // Defer the first editor render out of React's render phase. With
    // the default (true), the initial document — including the
    // MermaidBlock React NodeView, which flushes via flushSync — renders
    // synchronously during RichTextField's render, triggering React's
    // "flushSync was called from inside a lifecycle method" warning. No
    // SSR here, so deferring to a post-commit effect is invisible.
    immediatelyRender: false,
    extensions: [
      StarterKit.configure({
        // Replaced by MermaidBlock (which `extend`s CodeBlock under the
        // same name "codeBlock"). Avoid the duplicate name warning.
        codeBlock: false,
        // Inline-only fields skip block features at the schema level.
        heading: inlineOnly ? false : undefined,
        bulletList: inlineOnly ? false : undefined,
        orderedList: inlineOnly ? false : undefined,
        blockquote: inlineOnly ? false : undefined,
        horizontalRule: inlineOnly ? false : undefined,
      }),
      MermaidBlock,
      InternalLink,
      // GFM tables (block content — off for inline-only fields). tiptap-markdown
      // ships the GFM table serializer + markdown-it parses tables, so adding
      // the standard table nodes is all the round-trip needs. Without these the
      // editor flattens any table in a wiki page to run-on text on autosave.
      ...(inlineOnly
        ? []
        : [Table.configure({ resizable: true }), TableRow, TableHeader, TableCell]),
      // Decorations only — opening is via the right-click menu, not click.
      CommentDecorations.configure({ onClickComment: null }),
      Placeholder.configure({ placeholder: placeholder ?? "" }),
      Markdown.configure({
        html: false,
        linkify: false,
        breaks: false,
        transformPastedText: true,
        transformCopiedText: false,
      }),
    ],
    content: value,
    editorProps: {
      attributes: {
        class: "oxplow-md oxplow-rt-editor",
      },
    },
    onUpdate({ editor }) {
      if (debounceRef.current != null) window.clearTimeout(debounceRef.current);
      debounceRef.current = window.setTimeout(() => {
        const md = editor.storage.markdown?.getMarkdown?.() ?? "";
        if (md !== lastCommittedRef.current) {
          lastCommittedRef.current = md;
          onCommit(md);
        }
      }, 300);
    },
    onBlur({ editor }) {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      const md = editor.storage.markdown?.getMarkdown?.() ?? "";
      if (md !== lastCommittedRef.current) {
        lastCommittedRef.current = md;
        onCommit(md);
      }
    },
  });

  // Keep the editor in sync when the upstream value changes from
  // outside (e.g. another tab edited the same task). Don't clobber
  // the user's in-progress typing — skip the sync while the editor
  // has focus.
  useEffect(() => {
    if (!editor) return;
    if (editor.isFocused) return;
    if (value === lastCommittedRef.current) return;
    lastCommittedRef.current = value;
    editor.commands.setContent(value, false);
  }, [editor, value]);

  // On unmount, flush any pending debounce.
  useEffect(() => {
    return () => {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
      }
    };
  }, []);

  // Re-anchor each comment's quote against the current doc and push the
  // resolved ranges into the decoration plugin. Recomputes when the
  // thread list changes or the document content is re-synced; live
  // typing in between is handled by the plugin mapping its set forward.
  // A corrected/orphaned anchor is persisted via `setCommentAnchor`,
  // which deliberately emits no event so this doesn't loop.
  // Bumped (debounced) on every doc-changing transaction so the
  // re-anchor effect re-runs on live edits — not just on the debounced
  // `value` commit. Without this, deleting then retyping the quoted text
  // while focused would leave the comment stuck orphaned until blur.
  const [docVersion, setDocVersion] = useState(0);
  useEffect(() => {
    if (!editor) return;
    let timer: number | null = null;
    const onTx = () => {
      if (timer != null) window.clearTimeout(timer);
      timer = window.setTimeout(() => setDocVersion((v) => v + 1), 150);
    };
    editor.on("update", onTx);
    return () => {
      if (timer != null) window.clearTimeout(timer);
      editor.off("update", onTx);
    };
  }, [editor]);

  useEffect(() => {
    if (!editor || !comments) return;
    const doc = editor.state.doc;
    const ranges: CommentRange[] = [];
    for (const thread of threads) {
      const c = thread.comment;
      let hintFrom: number | undefined;
      let hintTo: number | undefined;
      let prefix: string | undefined;
      let suffix: string | undefined;
      try {
        const parsed = JSON.parse(c.selectors_json) as {
          from?: number;
          to?: number;
          prefix?: string;
          suffix?: string;
        };
        hintFrom = typeof parsed.from === "number" ? parsed.from : undefined;
        hintTo = typeof parsed.to === "number" ? parsed.to : undefined;
        prefix = typeof parsed.prefix === "string" ? parsed.prefix : undefined;
        suffix = typeof parsed.suffix === "string" ? parsed.suffix : undefined;
      } catch {
        // Malformed hint — fall back to a pure quote search.
      }
      const range = findCommentRange(doc, c.quote, { hintFrom, hintTo, prefix, suffix });
      if (range) {
        ranges.push({ id: c.id, from: range.from, to: range.to, approx: range.approx });
        // Re-persist the enriched anchor recomputed from the resolved
        // location so the hint + context self-heal (and old comments
        // upgrade in place); guard keeps DB churn down.
        const aj = buildAnchorJson(doc, range.from, range.to, range.approx);
        if (c.orphaned || c.selectors_json !== aj) void setCommentAnchor(c.id, aj, false);
      } else if (!c.orphaned) {
        void setCommentAnchor(c.id, c.selectors_json, true);
      }
    }
    editor.view.dispatch(editor.state.tr.setMeta(commentDecorationsKey, ranges));
  }, [editor, threads, comments, value, docVersion]);

  // Honor cross-page "go to location" requests from the Comments
  // dashboard. The decoration plugin renders each anchored range as a
  // `[data-comment-id]` span (the re-anchor effect above runs first), so
  // we scroll that node into view and open its popover. The request
  // stays on the bus until the node exists, surviving the async mount +
  // threads fetch after navigation.
  const [revealTick, setRevealTick] = useState(0);
  useEffect(() => subscribeCommentReveal(() => setRevealTick((t) => t + 1)), []);
  useEffect(() => {
    if (!editor || !comments) return;
    const id = peekPendingCommentReveal();
    if (id == null) return;
    const target = threads.find((t) => t.comment.id === id);
    if (!target) return;
    const dom = editor.view.dom as HTMLElement;
    const el = dom.querySelector(`[data-comment-id="${id}"]`) as HTMLElement | null;
    if (el) {
      el.scrollIntoView({ block: "center" });
      setPendingSel(null);
      setActiveComment({ id, rect: el.getBoundingClientRect() });
      clearCommentReveal(id);
    } else if (target.comment.orphaned) {
      // Orphaned: no decoration to scroll to — open the popover anyway
      // (near the editor's top-left) so the user can read it and relink.
      const r = dom.getBoundingClientRect();
      setPendingSel(null);
      setActiveComment({ id, rect: new DOMRect(r.left + 24, r.top + 24, 0, 0) });
      clearCommentReveal(id);
    }
    // else: anchored but decoration not painted yet — retry on next deps change.
  }, [revealTick, threads, comments, value, editor]);

  // Open the new-comment composer anchored to the current selection.
  // Anchors to the caret at the END of the selection (coordsAtPos), not
  // the selection's bounding box — a multi-line / mid-paragraph box-left
  // lands the composer far from where the user is looking.
  // Capture the quote + enriched anchor (prefix/suffix/textOffset) from
  // the SAME flattened text the resolver searches, so stored context
  // lines up with re-anchor. Block separators let cross-block selections
  // round-trip.
  const captureAnchor = (from: number, to: number): { quote: string; anchorJson: string } | null => {
    if (!editor) return null;
    const { text, map } = flatten(editor.state.doc);
    let startOff = map.findIndex((p) => p >= from);
    if (startOff === -1) startOff = map.length;
    let endOff = map.findIndex((p) => p >= to);
    if (endOff === -1) endOff = map.length;
    const raw = text.slice(startOff, endOff);
    const quote = raw.trim();
    if (!quote) return null;
    const lead = raw.length - raw.trimStart().length;
    const qStart = startOff + lead;
    const { prefix, suffix } = extractContext(text, qStart, qStart + quote.length);
    return {
      quote,
      anchorJson: JSON.stringify({ from, to, prefix, suffix, textOffset: qStart, approx: false }),
    };
  };

  // Hide the selection toolbar whenever the selection collapses
  // (keyboard or programmatic) — the mouseup path below only covers
  // mouse-driven selections.
  useEffect(() => {
    if (!editor) return;
    const onSelectionUpdate = () => {
      if (editor.state.selection.empty) setSelToolbarRect(null);
    };
    editor.on("selectionUpdate", onSelectionUpdate);
    return () => {
      editor.off("selectionUpdate", onSelectionUpdate);
    };
  }, [editor]);

  // Mirror the plain-DOM selection affordance: on mouseup with a fresh
  // non-collapsed selection, float the "Add comment" toolbar at the
  // selection's end. Deferred a tick so the browser finalizes the
  // selection first (same trick as useDomAnnotations).
  const handleSelectionMouseUp = () => {
    if (!editor || !comments) return;
    window.setTimeout(() => {
      if (!editor || editor.isDestroyed) return;
      const { to, empty } = editor.state.selection;
      const visible = selectionToolbarVisible({
        commentsEnabled: !!comments,
        selectionEmpty: empty,
        composerOpen: !!pendingSel,
        popoverOpen: !!activeComment,
      });
      if (!visible) {
        setSelToolbarRect(null);
        return;
      }
      try {
        const c = editor.view.coordsAtPos(to);
        setSelToolbarRect(new DOMRect(c.left, c.top, 0, c.bottom - c.top));
      } catch {
        const domSel = window.getSelection();
        if (!domSel || domSel.rangeCount === 0) return;
        setSelToolbarRect(domSel.getRangeAt(0).getBoundingClientRect());
      }
    }, 0);
  };

  const startCommentForSelection = () => {
    if (!editor || !comments) return;
    setSelToolbarRect(null);
    const { from, to, empty } = editor.state.selection;
    if (empty) return;
    const captured = captureAnchor(from, to);
    if (!captured) return;
    // Capture the refs any rendered links inside the selection point at
    // (e.g. an internal [[wikilink]]) from the live DOM selection, before
    // the composer steals focus and collapses it.
    const domSel = window.getSelection();
    const referencedRefs =
      domSel && domSel.rangeCount > 0 ? refsInRange(domSel.getRangeAt(0)) : [];
    let rect: DOMRect;
    try {
      const c = editor.view.coordsAtPos(to);
      rect = new DOMRect(c.left, c.top, 0, c.bottom - c.top);
    } catch {
      if (!domSel || domSel.rangeCount === 0) return;
      rect = domSel.getRangeAt(0).getBoundingClientRect();
    }
    setActiveComment(null);
    setPendingSel({ quote: captured.quote, from, to, rect, anchorJson: captured.anchorJson, referencedRefs });
  };

  // Right-click menu. Always shown (the native webview menu is never
  // allowed in our editor); carries Cut/Copy/Paste plus, when
  // comment-enabled, "Add Comment" (on a selection) / "Open Comment" (on
  // a commented range). Clipboard ops use positions captured here at
  // menu-open so they survive the menu click moving focus.
  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (!editor) return;
    event.preventDefault();
    const { from, to, empty } = editor.state.selection;
    const text = empty ? "" : editor.state.doc.textBetween(from, to);
    const targetEl = event.target as HTMLElement | null;
    const commentEl = comments
      ? (targetEl?.closest?.("[data-comment-id]") as HTMLElement | null)
      : null;
    const commentId = commentEl ? commentEl.getAttribute("data-comment-id") : null;

    const items: MenuItem[] = [
      {
        id: "cut",
        label: "Cut",
        enabled: !empty,
        run: async () => {
          if (text) await navigator.clipboard.writeText(text);
          editor.chain().focus().deleteRange({ from, to }).run();
        },
      },
      {
        id: "copy",
        label: "Copy",
        enabled: !empty,
        run: () => {
          if (text) void navigator.clipboard.writeText(text);
        },
      },
      {
        id: "paste",
        label: "Paste",
        enabled: true,
        run: async () => {
          const t = await navigator.clipboard.readText();
          if (t) editor.chain().focus().insertContentAt(empty ? from : { from, to }, t).run();
        },
      },
    ];
    if (comments && !empty) {
      items.push({
        id: "comment.add",
        label: "Add Comment",
        enabled: true,
        run: () => startCommentForSelection(),
      });
      // Re-attach any orphaned comment to this fresh selection — the
      // escape hatch for a quote that drifted past fuzzy tolerance.
      for (const t of threads) {
        if (!t.comment.orphaned) continue;
        const snip = t.comment.quote.length > 24 ? `${t.comment.quote.slice(0, 24)}…` : t.comment.quote;
        items.push({
          id: `comment.relink.${t.comment.id}`,
          label: `Relink orphaned: “${snip}”`,
          enabled: true,
          run: () => {
            const captured = captureAnchor(from, to);
            if (captured) void relinkComment(t.comment.id, captured.quote, captured.anchorJson);
          },
        });
      }
    }
    if (comments && commentId != null) {
      const rect = commentEl!.getBoundingClientRect();
      items.push({
        id: "comment.open",
        label: "Open Comment",
        enabled: true,
        run: () => {
          setPendingSel(null);
          setActiveComment({ id: commentId, rect });
        },
      });
    }
    setCommentMenu({ x: event.clientX, y: event.clientY, items });
  };

  const handleCreateComment = async (input: { body: string; intent: CommentIntent }) => {
    if (!comments || !pendingSel) return;
    await createComment({
      streamId: comments.streamId,
      threadId: comments.threadId,
      targetKind: comments.targetKind,
      targetId: comments.targetId,
      quote: pendingSel.quote,
      selectorsJson: pendingSel.anchorJson,
      referencedRefs: pendingSel.referencedRefs,
      intent: input.intent,
      author: comments.author ?? "user",
      body: input.body,
    });
    setPendingSel(null);
  };

  const activeThread =
    activeComment != null ? threads.find((t) => t.comment.id === activeComment.id) : undefined;

  const wrapperStyle: CSSProperties = {
    position: "relative",
    padding: "6px 8px",
    borderRadius: 6,
    transition: "background-color 120ms ease",
    ...style,
  };

  // Plain-click on a wikilink / file: / dir: / gitcommit: anchor inside
  // the editable surface should follow the link, not place a cursor.
  // Mirrors `MarkdownView`'s click semantics so the read-only and
  // editable surfaces feel the same: in-tab navigate via
  // `PageNavigationContext`, modifier/middle/right click escapes to a
  // new tab. Cursor placement inside link text is sacrificed — arrow
  // in from adjacent text — which is fine for wikilinks since the
  // visible label is rarely the cursor target.
  const ctxNav = useOptionalPageNavigation();
  const handleAnchorIntent = (event: ReactMouseEvent<HTMLDivElement>, isAux: boolean): boolean => {
    const target = event.target as HTMLElement | null;
    const anchor = target?.closest?.("a");
    if (!anchor) return false;
    const href = anchor.getAttribute("href") ?? "";
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "anchor" || parsed.kind === "empty") return false;
    event.preventDefault();
    event.stopPropagation();
    const newTab = isAux || event.metaKey || event.ctrlKey || event.button === 1;
    if (parsed.kind === "external") {
      window.open(href, "_blank", "noopener,noreferrer");
      return true;
    }
    if (parsed.kind === "file") {
      const version = parsed.version ?? DISK;
      ctxNav?.navigate(fileRef(parsed.path, version), { newTab });
      return true;
    }
    if (parsed.kind === "directory") {
      ctxNav?.navigate(directoryRef(parsed.path), { newTab });
      return true;
    }
    if (parsed.kind === "git-commit") {
      ctxNav?.navigate(gitCommitRef(parsed.sha), { newTab });
      return true;
    }
    if (parsed.kind === "internal") {
      ctxNav?.navigate(wikiPageRef(parsed.slug), { newTab });
      return true;
    }
    return false;
  };

  return (
    <div
      className={`oxplow-rt-field ${className ?? ""}`.trim()}
      style={wrapperStyle}
      onClick={(event) => {
        if (handleAnchorIntent(event, false)) return;
        // Clicking anywhere on the wrapper focuses the editor — keeps
        // the "the whole block is editable" feel from Linear.
        if (editor && !editor.isFocused) editor.commands.focus("end");
      }}
      onAuxClick={(event) => {
        // Middle-click on a link → new-tab navigate.
        if (event.button === 1) handleAnchorIntent(event, true);
      }}
      onMouseUp={handleSelectionMouseUp}
      onContextMenu={handleContextMenu}
    >
      {!hidePencil ? (
        <Pencil
          size={12}
          aria-hidden
          className="oxplow-rt-pencil"
          style={{
            position: "absolute",
            top: 6,
            right: 6,
            color: "var(--text-secondary)",
            opacity: 0.35,
            pointerEvents: "none",
            transition: "opacity 120ms ease",
          }}
        />
      ) : null}
      <EditorContent editor={editor} />
      {comments && selToolbarRect && !pendingSel && !activeComment && (
        <SelectionCommentToolbar
          rect={selToolbarRect}
          onAdd={() => startCommentForSelection()}
        />
      )}
      {comments && pendingSel && (
        <NewCommentPopover
          rect={pendingSel.rect}
          onCreate={handleCreateComment}
          onDismiss={() => setPendingSel(null)}
        />
      )}
      {comments && activeComment && activeThread && (
        <CommentPopover
          thread={activeThread}
          author={comments.author ?? "user"}
          anchorRect={activeComment.rect}
          onClose={() => setActiveComment(null)}
          onStep={
            partitionPageComments(threads).jumpable.length >= 2
              ? (dir) => {
                  const next = stepComment(threads, activeComment.id, dir);
                  if (next != null) requestCommentReveal(next);
                }
              : undefined
          }
          onRelink={
            activeThread.comment.orphaned
              ? () => {
                  if (!editor) return;
                  const { from, to, empty } = editor.state.selection;
                  if (empty) return; // hint tells the user to select text first
                  const captured = captureAnchor(from, to);
                  if (captured) {
                    void relinkComment(activeComment.id, captured.quote, captured.anchorJson);
                    setActiveComment(null);
                  }
                }
              : undefined
          }
        />
      )}
      {commentMenu && (
        <ContextMenu
          items={commentMenu.items}
          position={{ x: commentMenu.x, y: commentMenu.y }}
          onClose={() => setCommentMenu(null)}
        />
      )}
    </div>
  );
}
