import { diskFilePath, fileRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

/**
 * Resolve the active center-tab id back to a structured TabRef.
 *
 * The desktop's central page-visit recorder watches `effectiveCenterActive`
 * (a string id) and needs the corresponding TabRef to record the kind +
 * payload. Most kinds live in `pageTabs` (agent sessions' tabs too);
 * working-tree file tabs live in a separate `fileSessions.openOrder`
 * array keyed by path (`file:<path>` with no revision).
 *
 * Returns `null` when the id can't be resolved (e.g. mid-snap-back, or a
 * stale id from before the active set settled). Callers should treat
 * `null` as "skip recording this transition".
 */
export function resolveActiveTabRef(
  activeId: string,
  pageTabs: TabRef[],
  openFilePaths: string[],
): TabRef | null {
  const fromPage = pageTabs.find((t) => t.id === activeId);
  if (fromPage) return fromPage;
  const path = diskFilePath(activeId);
  if (path !== null && openFilePaths.includes(path)) return fileRef(path);
  return null;
}
