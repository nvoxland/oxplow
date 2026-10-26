/**
 * Comments are a page's capability, not a page's code: a page file
 * with `comments: true` (`Page.comments`) gives its vizes this
 * context — the comments' target (the page's URI, its tab id), the stream
 * they belong to and the thread they come from. A markdown editor anchors
 * comments to its text through it, a `code` viz to its lines; plain text
 * and the markdown viewer take them through the app's DOM layer, since
 * the page body carries `data-ref` (`PageHost`). Outside such a page it is
 * null and nothing is commentable. See `.context/usability.md` "Comments
 * are a page's".
 */
import { createContext, useContext } from "react";

export interface PageComments {
  /** The page's URI: what its comments are on. */
  target: string;
  streamId: string;
  threadId: string | null;
}

export const PageCommentsContext = createContext<PageComments | null>(null);

/** The page's comments, when the page takes them. */
export function usePageComments(): PageComments | null {
  return useContext(PageCommentsContext);
}
