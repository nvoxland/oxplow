/**
 * Ref kinds that should NOT be recorded as page visits. An agent
 * session's tab is always there while it's open, and creation pages
 * (the session picker among them) have throwaway ids.
 */
export const NON_TRACKED_KINDS: ReadonlySet<string> = new Set([
  "agent_session",
  "new-session",
  "new-stream",
  "new-task",
]);

/** Kinds excluded from the rail History display (still recorded for analytics).
 *  An agent session's tab leads the strip while it's open, so it would be
 *  noise in History; everything else (including pages pinned in the
 *  curated "Pages" section) is allowed through. */
export const RAIL_HISTORY_EXCLUDE_KINDS: string[] = [
  "agent_session",
];
