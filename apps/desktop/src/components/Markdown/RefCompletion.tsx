import { forwardRef, useEffect, useImperativeHandle, useState } from "react";
import { Extension } from "@tiptap/core";
import { ReactRenderer } from "@tiptap/react";
import Suggestion, { type SuggestionKeyDownProps, type SuggestionProps } from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";

import { PageKindIcon } from "../../pageKinds.js";
import type { MentionRef } from "./MarkdownView.js";
import { refLinkOf } from "./MarkdownView.js";
import type { RefOption, RefSearch } from "./refOptions.js";

export interface RefCompletionOptions {
  /** Where `[[` looks; none, and `[[` is plain text. */
  search: RefSearch | null;
  /** A work item id that is the active list's (`refLinkOf`). */
  mentionRef: MentionRef;
}

export interface RefCompletionStorage {
  /** The list is showing: Escape and Enter are its. */
  open: boolean;
}

declare module "@tiptap/core" {
  interface Storage {
    refCompletion: RefCompletionStorage;
  }
}

/**
 * `[[` completes over resources (`.context/markdown.md`): typing it opens a
 * list under the caret, filtered as the person types (`search`, debounced;
 * a stale search is aborted). ↑/↓ move, Enter or Tab picks, Escape closes
 * and keeps the text. Picking replaces `[[query` with the ref as its link
 * (`refLinkOf`: saved as the bare wikilink), and a space. `]]` typed by
 * hand closes the list: the text is a wikilink already.
 */
export const RefCompletion = Extension.create<RefCompletionOptions, RefCompletionStorage>({
  name: "refCompletion",
  addOptions() {
    return { search: null, mentionRef: () => null };
  },
  addStorage() {
    return { open: false };
  },
  addProseMirrorPlugins() {
    const ext = this;
    return [
      Suggestion<RefOption, RefOption>({
        pluginKey: new PluginKey("refCompletion"),
        editor: this.editor,
        char: "[[",
        allowSpaces: true,
        allowedPrefixes: null,
        debounce: 150,
        allow: ({ state, range }) => ext.options.search !== null && !state.doc.textBetween(range.from, range.to).includes("]]"),
        items: ({ query, signal }) => (ext.options.search ? ext.options.search(query, signal) : []),
        command: ({ editor, range, props }) => {
          const { href, text } = refLinkOf(props.uri, props.title, ext.options.mentionRef);
          editor
            .chain()
            .focus()
            .deleteRange(range)
            .insertContent([
              { type: "text", text, marks: [{ type: "link", attrs: { href } }] },
              { type: "text", text: " " },
            ])
            .run();
        },
        render: () => {
          let list: ReactRenderer<RefListHandle, RefListProps> | null = null;
          let unmount: (() => void) | null = null;
          return {
            onStart: (props) => {
              ext.storage.open = true;
              list = new ReactRenderer(RefList, { props, editor: props.editor });
              unmount = props.mount(list.element as HTMLElement);
            },
            onUpdate: (props) => list?.updateProps(props),
            onKeyDown: (props) => list?.ref?.onKeyDown(props) ?? false,
            onExit: () => {
              ext.storage.open = false;
              unmount?.();
              list?.destroy();
              list = null;
            },
          };
        },
      }),
    ];
  },
});

type RefListProps = SuggestionProps<RefOption, RefOption>;
interface RefListHandle {
  onKeyDown(props: SuggestionKeyDownProps): boolean;
}

const RefList = forwardRef<RefListHandle, RefListProps>(function RefList({ items, command, loading, query }, ref) {
  const [cursor, setCursor] = useState(0);
  useEffect(() => setCursor(0), [items]);
  useImperativeHandle(
    ref,
    () => ({
      onKeyDown: ({ event }) => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          if (items.length === 0) return true;
          const step = event.key === "ArrowDown" ? 1 : -1;
          setCursor((c) => (c + step + items.length) % items.length);
          return true;
        }
        if (event.key === "Enter" || event.key === "Tab") {
          const picked = items[cursor];
          if (!picked) return false;
          command(picked);
          return true;
        }
        return false;
      },
    }),
    [items, cursor, command],
  );
  return (
    <div data-testid="ref-completion" role="listbox" className="ox-ref-completion">
      {items.length === 0 ? (
        <div className="ox-ref-completion-empty">{loading ? "Searching…" : query.trim() ? "Nothing matches." : "Type to search."}</div>
      ) : (
        items.map((item, i) => (
          <div
            key={item.uri}
            role="option"
            aria-selected={i === cursor}
            data-testid={`ref-completion-row-${item.uri}`}
            className="ox-ref-completion-row"
            onMouseEnter={() => setCursor(i)}
            // The editor keeps focus: pick on the press, before a blur.
            onMouseDown={(e) => {
              e.preventDefault();
              command(item);
            }}
          >
            <PageKindIcon kind={item.kind} size={13} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
            <span className="ox-ref-completion-title">{item.title}</span>
          </div>
        ))
      )}
    </div>
  );
});
