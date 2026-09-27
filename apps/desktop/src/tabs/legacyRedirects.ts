/**
 * Page kinds that moved out of core into an extension's lenses. A tab,
 * bookmark or history entry saved before the move reopens the lens
 * instead of an empty tab. Keys are an exact tab id (`dashboard:quality`)
 * or a whole scheme/kind (`usage`); an exact id wins. Values are lens ids,
 * or `{ lens, param }` to carry the old id's row id (`effort-coverage:eff12`
 * → `effort_id=12`) into that lens param.
 * If the extension is disabled, the lens page says so. See
 * `.context/pages-and-tabs.md`.
 */
import { lensRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

export type LegacyRedirect = string | { lens: string; param: string };

export const LEGACY_PAGE_REDIRECTS: Readonly<Record<string, LegacyRedirect>> = {
  usage: "oxplow-analytics/usage",
  "page-analytics": "oxplow-analytics/usage",
  "dashboard:planning": "oxplow-analytics/planning",
  "dashboard:review": "oxplow-analytics/review",
  "dashboard:quality": "oxplow-analytics/quality",
  finding: "oxplow-analytics/findings",
  "effort-coverage": { lens: "oxplow-analytics/effort-tests", param: "effort_id" },
};

export function redirectLegacyRef(
  ref: TabRef,
  table: Readonly<Record<string, LegacyRedirect>> = LEGACY_PAGE_REDIRECTS,
): TabRef {
  const colon = ref.id.indexOf(":");
  const scheme = colon === -1 ? ref.id : ref.id.slice(0, colon);
  const target = table[ref.id] ?? table[scheme];
  if (!target) return ref;
  if (typeof target === "string") return lensRef(target);
  const rest = colon === -1 ? "" : ref.id.slice(colon + 1);
  const m = /^[a-z]*(\d+)$/.exec(rest);
  return lensRef(target.lens, rest === "" ? {} : { [target.param]: m ? Number(m[1]) : rest });
}
