import { createContext } from "react";

/** The page body's own scope, including when its tab is hidden. Inline
 * views inherit this rather than following the window's selected thread. */
export interface PageViewScope {
  streamId: string | null;
  threadId: string | null;
  /** The page's URI (its tab's identity): what text selected on it is
   *  added to the context basket on. */
  uri?: string | null;
}

export const PageViewContext = createContext<PageViewScope | null>(null);
