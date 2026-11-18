import { useEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import type { Editor } from "@tiptap/core";
import { Pencil } from "lucide-react";
import { getMarkdown, richTextExtensions } from "./richTextExtensions.js";
import { addToContext } from "../../contextBasket.js";
import { SelectionToolbar } from "./SelectionToolbar.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { followLink, linkMenu, type OpenRef } from "./links.js";
import { refLinkOf } from "./markdownLinks.js";
import { pastedLink, type RefLink } from "./refPaste.js";
import { linkTarget } from "../../uri/uri.js";
import type { RefSearch } from "./refOptions.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";

/** Put `refs` in at `pos` as their links (`refLinkOf`), a space between
 *  them and before them when the text there doesn't end in one. */
export function insertRefLinks(editor: Editor, pos: number, refs: readonly RefLink[]): void {
  if (refs.length === 0) return;
  const before = pos > 0 ? editor.state.doc.textBetween(Math.max(0, pos - 1), pos, "\n", "\n") : "";
  const content: Array<{ type: "text"; text: string; marks?: Array<{ type: "link"; attrs: { href: string } }> }> = [];
  refs.forEach((ref, i) => {
    if (i > 0 || (before !== "" && !/\s/.test(before))) content.push({ type: "text", text: " " });
    const { href, text } = refLinkOf(ref.uri, ref.title);
    content.push({ type: "text", text, marks: [{ type: "link", attrs: { href } }] });
  });
  editor.chain().focus().insertContentAt(pos, content).run();
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
 * saves on a button: a form, a composer, the page editor). Either may be
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
  /** Disable headings/blocks for inline-only fields (e.g. a knowledge page
   *  title). Default false. */
  inlineOnly?: boolean;
  /** Optional className applied to the wrapper. */
  className?: string;
  style?: CSSProperties;
  /** When true, no pencil affordance (e.g. a read-only summary — but
   *  that should use MarkdownView, not this field). Default false. */
  hidePencil?: boolean;
  /** The URI the field's text is on (the page it edits): what Add
   *  Selection to Context names with the selected text. */
  contextUri?: string | null;
  /** Where `[[` looks (`RefCompletion`); none, and `[[` is plain text.
   *  Read at mount: a field has it or doesn't. */
  refSearch?: RefSearch;
  /** A task id that is the active list's: a picked item saves as it. */
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
  contextUri,
  refSearch,
}: RichTextFieldProps) {
  const lastCommittedRef = useRef(value);
  const debounceRef = useRef<number | null>(null);
  // The owner's latest callbacks: the editor is made once.
  const owner = useRef({ onCommit, onChange, onSubmitShortcut, onSaveShortcut, onEscape, refSearch });
  owner.current = { onCommit, onChange, onSubmitShortcut, onSaveShortcut, onEscape, refSearch };
  // A popover of the field's is open: Escape is the popover's.
  const popoverOpenRef = useRef(false);
  const commit = (md: string) => {
    if (md === lastCommittedRef.current) return;
    lastCommittedRef.current = md;
    owner.current.onCommit?.(md);
  };
  // The right-click menu, when open.
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  // The floating format toolbar over a fresh selection. Null when hidden.
  const [selToolbarRect, setSelToolbarRect] = useState<DOMRect | null>(null);
  // A mouse button is down in the field: the toolbar waits for the mouseup.
  const mouseDownRef = useRef(false);
  popoverOpenRef.current = menu !== null;

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
        insertRefLinks(ed, from, [{ uri: link.uri, title: link.uri }]);
        return true;
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
  // lost to a closed tab).
  useEffect(() => {
    return () => {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
        const ed = editorRef.current;
        if (ed && !ed.isDestroyed) commit(getMarkdown(ed));
      }
    };
    // commit reads only refs.
  }, []);

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
    if (empty) {
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
  // This render's, for the listener below.
  const placeRef = useRef(placeSelectionToolbar);
  placeRef.current = placeSelectionToolbar;
  // On mouseup with a fresh selection, float the toolbar. Deferred a tick
  // so the browser finalizes the selection first.
  const handleSelectionMouseUp = () => {
    mouseDownRef.current = false;
    window.setTimeout(() => placeRef.current(), 0);
  };

  // Right-click menu. Always shown (the native webview menu is never
  // allowed in our editor); carries Cut/Copy/Paste plus, on a selection,
  // Add Selection to Context. Clipboard ops use positions captured here at
  // menu-open so they survive the menu click moving focus.
  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (!editor) return;
    event.preventDefault();
    const { from, to, empty } = editor.state.selection;
    const text = empty ? "" : editor.state.doc.textBetween(from, to);
    const targetEl = event.target as HTMLElement | null;

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
        run: () => void addToContext({ uri: contextUri, text }),
      });
    }
    setMenu({ x: event.clientX, y: event.clientY, items });
  };

  const wrapperStyle: CSSProperties = {
    position: "relative",
    padding: "6px 8px",
    borderRadius: 6,
    transition: "background-color 120ms ease",
    ...style,
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
      {editor && selToolbarRect && <SelectionToolbar editor={editor} rect={selToolbarRect} inlineOnly={inlineOnly} />}
      {menu && (
        <ContextMenu
          items={menu.items}
          position={{ x: menu.x, y: menu.y }}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
