/**
 * Markdown, the one component every surface draws or edits markdown with
 * (`.context/markdown.md`): a wiki page, a work item's body, an effort's
 * summary, a comment, a form's markdown field, a lens's `markdown` viz.
 * Given `onChange` or `onCommit` it is the editor, else the viewer; both
 * have the one feature set — wikilinks and ref links that open through
 * the page (Cmd/Ctrl-click: a new tab; right-click: Open, Open in new
 * tab, Copy), web links in the app when the allowlist takes them, mermaid
 * diagrams, GFM tables, code blocks with Copy, the headings a page's
 * Contents lists, and comments on the text.
 *
 * The editor reads and writes markdown as it is stored: `[[…]]` links go
 * in as links and come out as `[[…]]` again, so an owner never sees the
 * editor's own spelling.
 */
import { useCallback, useMemo, useRef } from "react";
import type { CSSProperties } from "react";

import { searchSite } from "../../api.js";
import { useLensStream } from "../../lens/LensStream.js";
import { useResources } from "../../resources.js";
import { readWorkspaceIndex } from "../../workspaceFiles.js";
import { useWorkListProfile } from "../../useWorkListProfile.js";
import { workItemRefOfMention } from "../../workItems.js";
import { MarkdownView, postprocessWikilinks, preprocessWikilinks } from "./MarkdownView.js";
import { RichTextField, type RichTextCommentConfig } from "./RichTextField.js";
import { fileOptions, mergeRefOptions, type RefSearch } from "./refOptions.js";

/** Comments on the editor's text: the page they are on (`target`, a URI),
 *  the stream they belong to and the thread they come from. */
export type MarkdownComments = RichTextCommentConfig;

export interface MarkdownProps {
  /** The markdown, as stored. */
  value: string;
  /** Edit: every edit's text (a draft the owner keeps). */
  onChange?(markdown: string): void;
  /** Edit: the text 300ms after typing stops, on blur and on unmount (an
   *  owner that saves as the person types). */
  onCommit?(markdown: string): void;
  /** Edit: Cmd/Ctrl+Enter, with the text as it is now. */
  onSubmitShortcut?(markdown: string): void;
  /** Edit: Cmd/Ctrl+S, with the text as it is now. */
  onSaveShortcut?(markdown: string): void;
  /** Edit: Escape, when none of the editor's popovers is open. */
  onEscape?(): void;
  /** Edit: what an empty editor says. */
  placeholder?: string;
  /** Edit: one paragraph's marks only (no headings, lists, blocks). */
  inlineOnly?: boolean;
  /** Edit: focus it when it mounts. */
  autofocus?: boolean;
  /** Edit: no pencil (a field whose frame already says it's editable). */
  hidePencil?: boolean;
  /** Edit: its text takes comments (the viewer's text takes them through
   *  its page: `data-ref`). */
  comments?: MarkdownComments;
  /** `data-testid` on the editable surface, or the viewer's root. */
  testId?: string;
  /** View: a height past which it scrolls. */
  maxHeight?: number | string;
  className?: string;
  style?: CSSProperties;
}

export function Markdown(props: MarkdownProps) {
  if (props.onChange || props.onCommit) return <MarkdownEditor {...props} />;
  const view = <MarkdownView body={props.value} maxHeight={props.maxHeight} className={props.className} style={props.style} />;
  return props.testId ? <div data-testid={props.testId}>{view}</div> : view;
}

function MarkdownEditor({
  value,
  onChange,
  onCommit,
  onSubmitShortcut,
  onSaveShortcut,
  onEscape,
  placeholder,
  inlineOnly,
  autofocus,
  hidePencil,
  comments,
  testId,
  className,
  style,
}: MarkdownProps) {
  const refKinds = useResources();
  const profile = useWorkListProfile();
  const refSearch = useRefSearch();
  const mentionRef = useCallback((id: string) => workItemRefOfMention(profile, id), [profile]);
  // The last text handed out, as stored and as the editor spelled it: the
  // owner's echo of it is the editor's own text, not a change to apply.
  const lastOut = useRef<{ stored: string; editor: string } | null>(null);
  const shown = useMemo(
    () =>
      lastOut.current && value === lastOut.current.stored
        ? lastOut.current.editor
        : preprocessWikilinks(value, (id) => workItemRefOfMention(profile, id)),
    // The ref kinds change what a `[[…]]` names.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [value, refKinds, profile],
  );
  const out = useCallback(
    (handler: ((md: string) => void) | undefined) =>
      handler
        ? (md: string) => {
            const stored = postprocessWikilinks(md);
            lastOut.current = { stored, editor: md };
            handler(stored);
          }
        : undefined,
    [],
  );
  return (
    <RichTextField
      value={shown}
      onChange={out(onChange)}
      onCommit={out(onCommit)}
      onSubmitShortcut={out(onSubmitShortcut)}
      onSaveShortcut={out(onSaveShortcut)}
      onEscape={onEscape}
      placeholder={placeholder}
      inlineOnly={inlineOnly}
      autofocus={autofocus}
      hidePencil={hidePencil}
      comments={comments}
      refSearch={refSearch}
      mentionRef={mentionRef}
      testId={testId}
      className={className}
      style={style}
    />
  );
}

/** How many rows `[[` lists. */
const REF_LIMIT = 12;

/** Where `[[` looks: the site search (project-wide, as the quick open) and,
 *  on a page of a stream, its files by name — the index read once, on the
 *  first `[[`. */
function useRefSearch(): RefSearch {
  const stream = useLensStream();
  const files = useRef<{ stream: string; paths: Promise<string[]> } | null>(null);
  const streamRef = useRef(stream?.id ?? null);
  streamRef.current = stream?.id ?? null;
  return useCallback<RefSearch>(async (query, signal) => {
    const q = query.trim();
    if (!q) return [];
    const streamId = streamRef.current;
    if (streamId && files.current?.stream !== streamId) {
      files.current = {
        stream: streamId,
        paths: readWorkspaceIndex(streamId).then(
          (r) => r.files.map((f) => f.path),
          () => [],
        ),
      };
    }
    const [hits, paths] = await Promise.all([
      q.length >= 2 ? searchSite(q, streamId, null, REF_LIMIT).catch(() => []) : Promise.resolve([]),
      streamId && files.current ? files.current.paths : Promise.resolve([]),
    ]);
    if (signal.aborted) return [];
    return mergeRefOptions(hits, fileOptions(paths, q, REF_LIMIT), REF_LIMIT);
  }, []);
}
