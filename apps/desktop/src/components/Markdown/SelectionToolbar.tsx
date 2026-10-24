import { useState } from "react";
import type { ReactNode } from "react";
import type { Editor } from "@tiptap/core";
import { useEditorState } from "@tiptap/react";
import { Bold, Code, Heading2, Heading3, Italic, Link2, MessageSquarePlus } from "lucide-react";

import { type MentionRef, preprocessWikilinks } from "./MarkdownView.js";

/**
 * The editor's selection toolbar (`.context/markdown.md`): floated over a
 * fresh selection, it formats it — Bold, Italic, Code, Link and the
 * headings (none in a one-line field) — and, where the text takes
 * comments, adds one. Each button says whether it is on. Buttons act on
 * the press, so the selection never collapses first. Link opens a box: a
 * web URL or an oxplow URI as it is, anything else read as a wikilink's
 * target (`local-snapshots`, `src/a.ts:12`, `tsk42`); Enter applies it,
 * nothing removes the link, Escape leaves the text as it was.
 */
export function SelectionToolbar({
  editor,
  rect,
  inlineOnly,
  mentionRef,
  onComment,
}: {
  editor: Editor;
  rect: DOMRect;
  inlineOnly: boolean;
  mentionRef: MentionRef;
  onComment?: () => void;
}) {
  const on = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      code: e.isActive("code"),
      link: e.isActive("link"),
      h2: e.isActive("heading", { level: 2 }),
      h3: e.isActive("heading", { level: 3 }),
      href: (e.getAttributes("link").href as string | undefined) ?? "",
    }),
  });
  // The selection the link box applies to: the box takes the focus.
  const [linking, setLinking] = useState<{ from: number; to: number; value: string } | null>(null);
  const run = (fn: (chain: ReturnType<Editor["chain"]>) => ReturnType<Editor["chain"]>) => fn(editor.chain().focus()).run();

  const applyLink = () => {
    if (!linking) return;
    const href = linkHref(linking.value, mentionRef);
    const chain = editor.chain().focus().setTextSelection({ from: linking.from, to: linking.to });
    (href ? chain.extendMarkRange("link").setLink({ href }) : chain.extendMarkRange("link").unsetLink()).run();
    setLinking(null);
  };

  const left = Math.min(Math.max(8, rect.left), window.innerWidth - 300);
  const top = Math.max(8, rect.top - 40);
  return (
    <div
      data-testid="selection-toolbar"
      data-selection-comment-ui
      role="toolbar"
      aria-label="Format"
      className="ox-selection-toolbar"
      style={{ position: "fixed", left, top }}
    >
      {linking ? (
        <input
          data-testid="selection-toolbar-link-input"
          className="ox-selection-toolbar-input"
          autoFocus
          placeholder="URL, page or file…"
          value={linking.value}
          onChange={(e) => setLinking({ ...linking, value: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              applyLink();
            } else if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              setLinking(null);
              editor.chain().focus().setTextSelection({ from: linking.from, to: linking.to }).run();
            }
          }}
        />
      ) : (
        <>
          <ToolButton id="bold" label="Bold (⌘B)" active={on.bold} onPress={() => run((c) => c.toggleBold())}>
            <Bold size={14} />
          </ToolButton>
          <ToolButton id="italic" label="Italic (⌘I)" active={on.italic} onPress={() => run((c) => c.toggleItalic())}>
            <Italic size={14} />
          </ToolButton>
          <ToolButton id="code" label="Code (⌘E)" active={on.code} onPress={() => run((c) => c.toggleCode())}>
            <Code size={14} />
          </ToolButton>
          <ToolButton
            id="link"
            label="Link"
            active={on.link}
            onPress={() => {
              const { from, to } = editor.state.selection;
              setLinking({ from, to, value: on.href });
            }}
          >
            <Link2 size={14} />
          </ToolButton>
          {inlineOnly ? null : (
            <>
              <ToolButton id="h2" label="Heading" active={on.h2} onPress={() => run((c) => c.toggleHeading({ level: 2 }))}>
                <Heading2 size={14} />
              </ToolButton>
              <ToolButton id="h3" label="Subheading" active={on.h3} onPress={() => run((c) => c.toggleHeading({ level: 3 }))}>
                <Heading3 size={14} />
              </ToolButton>
            </>
          )}
          {onComment ? (
            <ToolButton id="comment" label="Add comment" active={false} onPress={onComment}>
              <MessageSquarePlus size={14} />
              <span style={{ marginLeft: 4 }}>Comment</span>
            </ToolButton>
          ) : null}
        </>
      )}
    </div>
  );
}

function ToolButton({
  id,
  label,
  active,
  onPress,
  children,
}: {
  id: string;
  label: string;
  active: boolean;
  onPress(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      data-testid={`selection-toolbar-${id}`}
      className="ox-selection-toolbar-button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      // On the press: a click would collapse the selection first.
      onMouseDown={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onPress();
      }}
    >
      {children}
    </button>
  );
}

/** What the link box's text links to: a URL as it is, else the target a
 *  wikilink of it names (`preprocessWikilinks`); null for nothing. */
export function linkHref(value: string, mentionRef: MentionRef): string | null {
  const v = value.trim();
  if (!v) return null;
  if (/^(https?|mailto|oxplow):/i.test(v)) return v;
  const interior = v.replace(/^\[\[/, "").replace(/\]\]$/, "");
  const md = preprocessWikilinks(`[[${interior}]]`, mentionRef);
  return /\]\(([^)]+)\)$/.exec(md)?.[1] ?? null;
}

/// Gating for the editor's floating selection toolbar (`SelectionToolbar`)
/// over a fresh text selection. The Tiptap surface is deliberately carved
/// out of the plain-DOM selection layer (`useDomAnnotations` skips
/// contenteditable), so the field shows its own — this predicate keeps the
/// rules in one testable spot.
export interface SelectionToolbarInput {
  /// The editor selection is collapsed.
  selectionEmpty: boolean;
  /// The new-comment composer is open for a pending selection.
  composerOpen: boolean;
  /// An existing comment-thread popover is open.
  popoverOpen: boolean;
}

export function selectionToolbarVisible(input: SelectionToolbarInput): boolean {
  return !input.selectionEmpty && !input.composerOpen && !input.popoverOpen;
}
