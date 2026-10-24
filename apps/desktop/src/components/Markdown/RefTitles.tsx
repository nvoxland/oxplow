import { Extension, type Decoration } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { ReactWidgetRenderer } from "@tiptap/react";

import { PageKindIcon } from "../../pageKinds.js";
import { BrokenLink, parseMarkdownLink, parsedLinkIconKind, refLinkNames, useRefLinkTitle } from "./MarkdownView.js";

/**
 * A ref link's title in the editor, as the viewer shows it
 * (`useRefLinkTitle`): a widget drawn before each link that names a titled
 * ref (a wiki page, a work item, an extension's ref). Once its title
 * resolves for a bare link, the widget shows the kind's icon and the title,
 * and CSS hides the link beside it (`.oxplow-ref-title:has([data-ref-title])
 * + a`, in `index.html`) — the stored text stays the slug. With the caret
 * inside the link, there's no widget: the slug shows, to edit (the rule
 * `MermaidBlock` keeps for a diagram's source). A decoration, never a mark,
 * so nothing of it is saved.
 */
export const RefTitles = Extension.create({
  name: "refTitles",
  addDecorations() {
    return {
      create: ({ editor, state }) => {
        const out: Decoration[] = [];
        const { from: selFrom, to: selTo } = state.selection;
        const seen = new Map<string, number>();
        for (const run of linkRuns(state.doc)) {
          // The caret (or a selection) strictly inside: edit the slug.
          if (selTo > run.from && selFrom < run.to) continue;
          const parsed = parseMarkdownLink(run.href);
          const names = refLinkNames(parsed);
          if (!names.internalSlug && !names.itemRef && !names.extensionRef) continue;
          const n = seen.get(run.href) ?? 0;
          seen.set(run.href, n + 1);
          out.push(
            ReactWidgetRenderer(RefTitle, {
              editor,
              pos: run.from,
              side: -1,
              key: `ref-title:${run.href}:${run.text}:${n}`,
              className: "oxplow-ref-title",
              props: { href: run.href, text: run.text },
            }),
          );
        }
        return out;
      },
      shouldUpdate: ({ tr, oldState, newState }) => tr.docChanged || !oldState.selection.eq(newState.selection),
    };
  },
});

/** Each stretch of text under one link: where it starts and ends, its
 *  href and its text. */
export function linkRuns(doc: PMNode): Array<{ from: number; to: number; href: string; text: string }> {
  const runs: Array<{ from: number; to: number; href: string; text: string }> = [];
  doc.descendants((node, pos) => {
    if (!node.isText) return;
    const href = node.marks.find((m) => m.type.name === "link")?.attrs.href as string | undefined;
    if (!href) return;
    const last = runs[runs.length - 1];
    if (last && last.href === href && last.to === pos) {
      last.to = pos + node.nodeSize;
      last.text += node.text ?? "";
    } else {
      runs.push({ from: pos, to: pos + node.nodeSize, href, text: node.text ?? "" });
    }
  });
  return runs;
}

function RefTitle({ href, text }: { href: string; text: string }) {
  const parsed = parseMarkdownLink(href);
  const { title, brokenReason } = useRefLinkTitle(refLinkNames(parsed), text);
  if (brokenReason) {
    return (
      <span data-ref-title="">
        <BrokenLink reason={brokenReason}>{text}</BrokenLink>
      </span>
    );
  }
  if (!title) return null;
  const kind = parsedLinkIconKind(parsed);
  return (
    <a href={href} data-ref-title="" className="oxplow-md-link" contentEditable={false}>
      {kind ? (
        <PageKindIcon kind={kind} size={12} style={{ color: "var(--text-secondary)", marginRight: 3, verticalAlign: "middle" }} />
      ) : null}
      {title}
    </a>
  );
}
