import { useEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import type { Editor } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { Pencil } from "lucide-react";
import { getMarkdown, richTextExtensions } from "./richTextExtensions.js";
import {
  commentDecorationsKey,
  findCommentRange,
  flatten,
  type CommentRange,
} from "./CommentDecorations.js";
import { addToContext } from "../../contextBasket.js";
import { createComment, relinkComment, relocateComment } from "../../api.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { extractContext } from "../Comments/anchor.js";
import { partitionPageComments, stepComment } from "../Comments/pageCommentNav.js";
import { refsInRange } from "../Comments/domAnchor.js";
import { useCommentsForTarget } from "../Comments/useCommentsForTarget.js";
import {
  clearCommentReveal,
  peekPendingCommentReveal,
  requestCommentReveal,
  subscribeCommentReveal,
} from "../../comment-reveal-bus.js";
import { CommentPopover } from "../Comments/CommentPopover.js";
import { NewCommentPopover } from "../Comments/NewCommentPopover.js";
import { SelectionToolbar, selectionToolbarVisible } from "./SelectionToolbar.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { followLink, linkMenu, type OpenRef } from "./links.js";
import { refLinkOf, type MentionRef } from "./MarkdownView.js";
import { dragCarriesRefs, droppedRefs, pastedLink, type DroppedRef } from "./refDrops.js";
import { linkTarget } from "../../uri/uri.js";
import type { RefSearch } from "./refOptions.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";

/// Comment integration bundle. When provided, the field highlights
/// anchored ranges and exposes "Add comment" via both the floating
/// selection toolbar (mirroring the plain-DOM surfaces, which this
/// contenteditable region is carved out of) and the right-click menu;
/// "Open comment" stays right-click-only so existing comments don't
/// fight selection/cursor. `target` (a URI) identifies the
/// page (wiki slug / task id); `streamId` is the hard scope and
/// `threadId` the origin thread (null for non-thread-bound surfaces).
export interface RichTextCommentConfig {
  streamId: string;
  threadId: string | null;
  target: string;
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

/** Put `refs` in at `pos` as their links (`refLinkOf`), a space between
 *  them and before them when the text there doesn't end in one. */
export function insertRefLinks(editor: Editor, pos: number, refs: readonly DroppedRef[], mentionRef: MentionRef): void {
  if (refs.length === 0) return;
  const before = pos > 0 ? editor.state.doc.textBetween(Math.max(0, pos - 1), pos, "\n", "\n") : "";
  const content: Array<{ type: "text"; text: string; marks?: Array<{ type: "link"; attrs: { href: string } }> }> = [];
  refs.forEach((ref, i) => {
    if (i > 0 || (before !== "" && !/\s/.test(before))) content.push({ type: "text", text: " " });
    const { href, text } = refLinkOf(ref.uri, ref.title, mentionRef);
    content.push({ type: "text", text, marks: [{ type: "link", attrs: { href } }] });
  });
  editor.chain().focus().insertContentAt(pos, content).run();
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
  referenced: string[];
}

/**
 * Shared rich-text editor surface. One instance per editable region
 * (title saves to one field, description to another, etc.) — the page
 * composes them at the React level.
 *
 * Storage stays markdown. tiptap-markdown handles GFM round-trip on
 * mount and on save; the `FencedBlock` NodeView paints rendered SVG
 * over the editable fenced code, so users see the diagram unless they
 * click into it.
 *
 * Two owners: `onCommit` hears the text 300ms after typing stops, on blur
 * and on unmount (an owner that saves as the person types: the task
 * body); `onChange` hears every edit (an owner that keeps a draft and
 * saves on a button: a form, a composer, the lens editor). Either may be
 * given, or both. Cmd/Ctrl+Enter, Cmd/Ctrl+S and Escape reach the owner
 * with the text as it is now (`onSubmitShortcut`, `onSaveShortcut`,
 * `onEscape`).
 *
 * Pencil affordance: a small `Pencil` icon sits in the top-right of
 * the editor surface, opacity ~0.4 by default, full opacity on hover
 * or focus. Read-only blocks elsewhere on the page must not show this
 * — that's the visual signal "this is for reading."
 */
export interface RichTextFieldProps {
  value: string;
  /** The text, 300ms after typing stops, on blur and on unmount. */
  onCommit?: (markdown: string) => void;
  /** The text, on every edit. */
  onChange?: (markdown: string) => void;
  /** Cmd/Ctrl+Enter, with the text as it is now. */
  onSubmitShortcut?: (markdown: string) => void;
  /** Cmd/Ctrl+S, with the text as it is now. */
  onSaveShortcut?: (markdown: string) => void;
  /** Escape, when no popover of the field's is open. */
  onEscape?: () => void;
  /** Focus the editor when it mounts. */
  autofocus?: boolean;
  /** `data-testid` on the editable surface. */
  testId?: string;
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
  /** Where `[[` looks (`RefCompletion`); none, and `[[` is plain text.
   *  Read at mount: a field has it or doesn't. */
  refSearch?: RefSearch;
  /** A work item id that is the active list's: a picked item saves as it. */
  mentionRef?: MentionRef;
}

export function RichTextField({
  value,
  onCommit,
  onChange,
  onSubmitShortcut,
  onSaveShortcut,
  onEscape,
  autofocus = false,
  testId,
  placeholder,
  inlineOnly = false,
  className,
  style,
  hidePencil,
  comments,
  refSearch,
  mentionRef,
}: RichTextFieldProps) {
  const lastCommittedRef = useRef(value);
  const debounceRef = useRef<number | null>(null);
  // The owner's latest callbacks: the editor is made once.
  const owner = useRef({ onCommit, onChange, onSubmitShortcut, onSaveShortcut, onEscape, refSearch, mentionRef });
  owner.current = { onCommit, onChange, onSubmitShortcut, onSaveShortcut, onEscape, refSearch, mentionRef };
  // A popover of the field's is open: Escape is the popover's.
  const popoverOpenRef = useRef(false);
  const commit = (md: string) => {
    if (md === lastCommittedRef.current) return;
    lastCommittedRef.current = md;
    owner.current.onCommit?.(md);
  };
  // Re-found comment anchors waiting to be stored (tsk902): written when
  // the document settles — on blur, on leaving, or at once when the
  // person isn't typing — never per keystroke pause, since each is a
  // command run on the person's behalf.
  const pendingAnchorsRef = useRef(new Map<string, { anchor: string; orphaned: boolean }>());
  const flushAnchors = () => {
    const pending = pendingAnchorsRef.current;
    for (const [id, { anchor, orphaned }] of pending) {
      relocateComment(id, anchor, orphaned).catch(() => {
        // Deleted meanwhile, or the run failed (the bus audited it):
        // there is nothing left to re-anchor.
      });
    }
    pending.clear();
  };

  // Comment state. The hook is always called (empty target → no fetch).
  const { threads } = useCommentsForTarget(comments?.target ?? "");
  const [activeComment, setActiveComment] = useState<{ id: string; rect: DOMRect } | null>(null);
  const [pendingSel, setPendingSel] = useState<PendingSelection | null>(null);
  const [commentMenu, setCommentMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(
    null,
  );
  // The floating selection toolbar (format, and Add comment where comments
  // are) over a fresh selection. Null when hidden.
  const [selToolbarRect, setSelToolbarRect] = useState<DOMRect | null>(null);
  // A mouse button is down in the field: the toolbar waits for the mouseup.
  const mouseDownRef = useRef(false);
  // A drag of refs is over the field: it shows it takes the drop.
  const [dropping, setDropping] = useState(false);
  popoverOpenRef.current = pendingSel !== null || activeComment !== null || commentMenu !== null;

  const editor = useEditor({
    // Defer the first editor render out of React's render phase. With
    // the default (true), the initial document — including the
    // FencedBlock React NodeView, which flushes via flushSync — renders
    // synchronously during RichTextField's render, triggering React's
    // "flushSync was called from inside a lifecycle method" warning. No
    // SSR here, so deferring to a post-commit effect is invisible.
    immediatelyRender: false,
    extensions: richTextExtensions({
      inlineOnly,
      placeholder,
      refCompletion: refSearch
        ? {
            search: (query, signal) => owner.current.refSearch?.(query, signal) ?? Promise.resolve([]),
            mentionRef: (id) => owner.current.mentionRef?.(id) ?? null,
          }
        : undefined,
    }),
    content: value,
    autofocus: autofocus ? "end" : false,
    editorProps: {
      attributes: {
        class: "oxplow-md oxplow-rt-editor",
        ...(testId ? { "data-testid": testId } : {}),
      },
      handleKeyDown(view, event) {
        const mod = event.metaKey || event.ctrlKey;
        const md = () => getMarkdown(editorRef.current!);
        if (mod && event.key === "Enter" && owner.current.onSubmitShortcut) {
          event.preventDefault();
          owner.current.onSubmitShortcut(md());
          return true;
        }
        if (mod && event.key.toLowerCase() === "s" && owner.current.onSaveShortcut) {
          event.preventDefault();
          owner.current.onSaveShortcut(md());
          return true;
        }
        // The `[[` list's keys are its own.
        if (editorRef.current?.storage.refCompletion?.open) return false;
        if (event.key === "Escape" && owner.current.onEscape && !popoverOpenRef.current) {
          event.preventDefault();
          owner.current.onEscape();
          return true;
        }
        void view;
        return false;
      },
      // A pasted oxplow URI goes in as its link; a URI or a web URL over
      // a selection links it. Any other paste is the editor's own.
      handlePaste(view, event) {
        const ed = editorRef.current;
        const link = pastedLink(event.clipboardData?.getData("text/plain") ?? "");
        if (!ed || !link) return false;
        const { empty, from } = view.state.selection;
        if (!empty) {
          ed.chain().focus().setLink({ href: link.kind === "ref" ? linkTarget(link.uri) : link.url }).run();
          return true;
        }
        if (link.kind !== "ref") return false;
        insertRefLinks(ed, from, [{ uri: link.uri, title: link.uri }], (id) => owner.current.mentionRef?.(id) ?? null);
        return true;
      },
      handleDOMEvents: {
        // An "Add to agent context" or work-item drag goes in as links where
        // it lands — at the caret when the point names no place (ProseMirror's
        // own drop gives up there, and the browser would paste its text).
        drop(view, event) {
          setDropping(false);
          const ed = editorRef.current;
          if (!ed || !event.dataTransfer) return false;
          const refs = droppedRefs(event.dataTransfer);
          if (refs.length === 0) return false;
          event.preventDefault();
          const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos ?? view.state.selection.from;
          insertRefLinks(ed, pos, refs, (id) => owner.current.mentionRef?.(id) ?? null);
          return true;
        },
      },
    },
    onUpdate({ editor }) {
      if (owner.current.onChange) {
        owner.current.onChange(getMarkdown(editor));
      }
      if (!owner.current.onCommit) return;
      if (debounceRef.current != null) window.clearTimeout(debounceRef.current);
      debounceRef.current = window.setTimeout(() => {
        debounceRef.current = null;
        commit(getMarkdown(editor));
      }, 300);
    },
    onBlur({ editor, event }) {
      // Focus left for anything but the toolbar (its link box): hide it.
      const to = (event as FocusEvent).relatedTarget as HTMLElement | null;
      if (!to?.closest?.("[data-testid='selection-toolbar']")) setSelToolbarRect(null);
      flushAnchors();
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      if (owner.current.onCommit) commit(getMarkdown(editor));
    },
  });
  const editorRef = useRef(editor);
  editorRef.current = editor;

  // Keep the editor in sync when the upstream value changes from
  // outside (e.g. another tab edited the same task). Don't clobber
  // the user's in-progress typing — skip the sync while the editor
  // has focus.
  //
  // Deferred to a microtask: the new content can create React NodeViews
  // (FencedBlock), which Tiptap renders with flushSync — illegal while
  // React is still committing this effect. Guards re-run at apply time
  // since focus or the editor itself may have changed in between.
  useEffect(() => {
    if (!editor) return;
    queueMicrotask(() => {
      if (editor.isDestroyed) return;
      // Already what the editor holds (an echo of its own text).
      if (value === getMarkdown(editor)) return;
      // While the person types, a value from outside is skipped for an
      // owner that saves on commit (a late echo mustn't clobber typing);
      // an owner that hears every change echoes exactly what it heard, so
      // anything else is a reset it means (Discard, Escape).
      if (editor.isFocused && !owner.current.onChange) return;
      lastCommittedRef.current = value;
      editor.commands.setContent(value, { emitUpdate: false });
    });
  }, [editor, value]);

  // On unmount, commit what the last pause didn't reach (an edit is never
  // lost to a closed tab), and store the anchors the last edits moved.
  useEffect(() => {
    return () => {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
        const ed = editorRef.current;
        if (ed && !ed.isDestroyed) commit(getMarkdown(ed));
      }
      flushAnchors();
    };
    // commit and flushAnchors read only refs.
  }, []);

  // Re-anchor each comment's quote against the current doc and push the
  // resolved ranges into the decoration plugin. Recomputes when the
  // thread list changes or the document content is re-synced; live
  // typing in between is handled by the plugin mapping its set forward.
  // A corrected/orphaned anchor is queued and stored via
  // `relocateComment` once the document settles (`flushAnchors`); the
  // stored anchor then equals the recomputed one, so the re-read it
  // causes stops here.
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
        if (c.orphaned || c.selectors_json !== aj) {
          pendingAnchorsRef.current.set(c.id, { anchor: aj, orphaned: false });
        } else {
          pendingAnchorsRef.current.delete(c.id);
        }
      } else if (!c.orphaned) {
        pendingAnchorsRef.current.set(c.id, { anchor: c.selectors_json, orphaned: true });
      } else {
        pendingAnchorsRef.current.delete(c.id);
      }
    }
    editor.view.dispatch(editor.state.tr.setMeta(commentDecorationsKey, ranges));
    // Not being typed in: the document is settled now.
    if (!editor.isFocused) flushAnchors();
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

  // The toolbar follows the selection: hidden when it collapses, placed
  // when the keyboard makes one (a mouse selection waits for its mouseup,
  // below).
  useEffect(() => {
    if (!editor) return;
    const onSelectionUpdate = () => {
      if (editor.state.selection.empty) setSelToolbarRect(null);
      else if (!mouseDownRef.current) placeRef.current();
    };
    editor.on("selectionUpdate", onSelectionUpdate);
    return () => {
      editor.off("selectionUpdate", onSelectionUpdate);
    };
  }, [editor]);

  // Float the toolbar at the selection's end, when it shows at all.
  const placeSelectionToolbar = () => {
    if (!editor || editor.isDestroyed) return;
    const { to, empty } = editor.state.selection;
    const visible = selectionToolbarVisible({
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
  };
  // This render's (it reads the popovers' state), for the listener below.
  const placeRef = useRef(placeSelectionToolbar);
  placeRef.current = placeSelectionToolbar;
  // Mirror the plain-DOM selection affordance: on mouseup with a fresh
  // selection, float the toolbar. Deferred a tick so the browser finalizes
  // the selection first (same trick as useDomAnnotations).
  const handleSelectionMouseUp = () => {
    mouseDownRef.current = false;
    window.setTimeout(() => placeRef.current(), 0);
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
    const referenced =
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
    setPendingSel({ quote: captured.quote, from, to, rect, anchorJson: captured.anchorJson, referenced });
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

    // On a link: the viewer's link menu first.
    const href = targetEl?.closest?.("a")?.getAttribute("href") ?? null;
    const items: MenuItem[] = [
      ...(href ? linkMenu(href, open) : []),
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
    if (!empty) {
      items.push({
        id: "context.add",
        label: "Add Selection to Context",
        enabled: true,
        run: () => void addToContext({ uri: comments?.target, text }),
      });
    }
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
      target: comments.target,
      quote: pendingSel.quote,
      selectorsJson: pendingSel.anchorJson,
      referenced: pendingSel.referenced,
      intent: input.intent,
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
    ...(dropping ? { outline: "2px dashed var(--accent)", background: "var(--accent-soft-bg)" } : {}),
  };

  // Plain-click on a wikilink / file: / dir: / commit: anchor inside
  // the editable surface should follow the link, not place a cursor.
  // Mirrors `MarkdownView`'s click semantics so the read-only and
  // editable surfaces feel the same: in-tab navigate via
  // `PageNavigationContext`, modifier/middle/right click escapes to a
  // new tab. Cursor placement inside link text is sacrificed — arrow
  // in from adjacent text — which is fine for wikilinks since the
  // visible label is rarely the cursor target.
  const ctxNav = useOptionalPageNavigation();
  const open: OpenRef = ctxNav?.openRef ? ctxNav.openRef.bind(ctxNav) : null;
  const handleAnchorIntent = (event: ReactMouseEvent<HTMLDivElement>, isAux: boolean): boolean => {
    const target = event.target as HTMLElement | null;
    const anchor = target?.closest?.("a");
    if (!anchor) return false;
    const href = anchor.getAttribute("href") ?? "";
    const newTab = isAux || event.metaKey || event.ctrlKey || event.button === 1;
    // The viewer's links, the viewer's way (`links.ts`).
    if (!followLink(href, open, newTab)) return false;
    event.preventDefault();
    event.stopPropagation();
    return true;
  };

  return (
    <div
      className={`oxplow-rt-field ${className ?? ""}`.trim()}
      style={wrapperStyle}
      onClick={(event) => {
        if (handleAnchorIntent(event, false)) return;
        // Clicking the wrapper around the text focuses the editor — keeps
        // the "the whole block is editable" feel from Linear. A click in
        // the text placed the caret itself: focusing the end (a frame
        // later) would move it under keys already typed.
        const inText = (event.target as HTMLElement | null)?.closest?.(".ProseMirror");
        if (editor && !editor.isFocused && !inText) editor.commands.focus("end");
      }}
      onAuxClick={(event) => {
        // Middle-click on a link → new-tab navigate.
        if (event.button === 1) handleAnchorIntent(event, true);
      }}
      onMouseDown={() => {
        mouseDownRef.current = true;
      }}
      onDragOver={(event) => {
        if (dragCarriesRefs(Array.from(event.dataTransfer.types))) setDropping(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropping(false);
      }}
      onDrop={() => setDropping(false)}
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
      {editor && selToolbarRect && !pendingSel && !activeComment && (
        <SelectionToolbar
          editor={editor}
          rect={selToolbarRect}
          inlineOnly={inlineOnly}
          mentionRef={(id) => owner.current.mentionRef?.(id) ?? null}
          onComment={comments ? () => startCommentForSelection() : undefined}
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
