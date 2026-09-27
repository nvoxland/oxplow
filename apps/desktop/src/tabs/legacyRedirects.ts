/**
 * Page kinds that moved out of core into an extension's lenses. A tab,
 * bookmark or history entry saved before the move reopens the lens
 * instead of an empty tab. Keys are an exact tab id (`dashboard:quality`)
 * or a whole scheme/kind (`usage`); an exact id wins. Values are lens ids.
 * If the extension is disabled, the lens page says so. See
 * `.context/pages-and-tabs.md`.
 */
import { lensRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

export const LEGACY_PAGE_REDIRECTS: Readonly<Record<string, string>> = {};

export function redirectLegacyRef(
  ref: TabRef,
  table: Readonly<Record<string, string>> = LEGACY_PAGE_REDIRECTS,
): TabRef {
  const colon = ref.id.indexOf(":");
  const scheme = colon === -1 ? ref.id : ref.id.slice(0, colon);
  const target = table[ref.id] ?? table[scheme];
  return target ? lensRef(target) : ref;
}
