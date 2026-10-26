import { createContext } from "react";

/** The page body's own scope, including when its tab is hidden. Inline
 * views inherit this rather than following the window's selected thread. */
export interface PageViewScope {
  streamId: string | null;
  threadId: string | null;
}

export const PageViewContext = createContext<PageViewScope | null>(null);
