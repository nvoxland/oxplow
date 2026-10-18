// Pure state + persistence for the collapsible page sections (tsk84). React-free
// so the rules are unit-testable without mounting a page.
//
// The state is stored as the set of **collapsed** ids, not expanded ones: a
// section defaults to expanded, so "collapsed" is the exception worth recording
// and a brand-new section needs no migration to appear open.

/** localStorage key. `{ [pageKey]: collapsedId[] }` — one entry per page. */
export const SECTIONS_COLLAPSED_KEY = "oxplow.page.sectionsCollapsed.v1";

/** Sections default to **expanded** — only an explicit collapse is recorded.
 *  A section that `defaultCollapsed` is the mirror image: it starts closed
 *  and only an explicit opening (`opened`) is recorded. */
export function isExpanded(
  collapsed: ReadonlySet<string>,
  id: string,
  opened: ReadonlySet<string> = EMPTY,
  defaultCollapsed = false,
): boolean {
  return defaultCollapsed ? opened.has(id) : !collapsed.has(id);
}

const EMPTY: ReadonlySet<string> = new Set();

/** Where a page's opened-by-the-person ids live in the blob (beside its
 *  collapsed ones, under the same key plus this suffix). */
export const OPENED_SUFFIX = "#opened";

/** Flip one section in whichever set records its exceptions: `collapsed`
 *  for a section that starts open, `opened` for one that starts closed. */
export function toggleSection(
  state: { collapsed: ReadonlySet<string>; opened: ReadonlySet<string> },
  id: string,
  defaultCollapsed: boolean,
): { collapsed: ReadonlySet<string>; opened: ReadonlySet<string> } {
  return defaultCollapsed
    ? { collapsed: state.collapsed, opened: toggleCollapsed(state.opened, id) }
    : { collapsed: toggleCollapsed(state.collapsed, id), opened: state.opened };
}

/** Open (or close) every rendered section, leaving the ones that aren't
 *  rendered as they were. `rendered` maps an id to whether it starts closed. */
export function setAll(
  state: { collapsed: ReadonlySet<string>; opened: ReadonlySet<string> },
  rendered: ReadonlyMap<string, boolean>,
  expand: boolean,
): { collapsed: Set<string>; opened: Set<string> } {
  const collapsed = new Set(state.collapsed);
  const opened = new Set(state.opened);
  for (const [id, startsClosed] of rendered) {
    const exceptions = startsClosed ? opened : collapsed;
    // A section is an exception when it is in the state the other way.
    if (startsClosed === expand) exceptions.add(id);
    else exceptions.delete(id);
  }
  return { collapsed, opened };
}

/** Flip one section, leaving the rest alone. Returns a new set (never mutates). */
export function toggleCollapsed(collapsed: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(collapsed);
  if (!next.delete(id)) next.add(id);
  return next;
}

/** True when every **currently rendered** section is expanded. Ids that aren't
 *  rendered (hidden by a search) are ignored — they must not decide whether the
 *  toolbar's buttons have anything to act on. `[]` is neither all-expanded nor
 *  all-collapsed: there's nothing to expand or collapse. */
export function allExpanded(
  ids: readonly string[],
  collapsed: ReadonlySet<string>,
  opened: ReadonlySet<string> = EMPTY,
  startsClosed: ReadonlySet<string> = EMPTY,
): boolean {
  return ids.length > 0 && ids.every((id) => isExpanded(collapsed, id, opened, startsClosed.has(id)));
}

/** True when every currently rendered section is collapsed. See `allExpanded`. */
export function allCollapsed(
  ids: readonly string[],
  collapsed: ReadonlySet<string>,
  opened: ReadonlySet<string> = EMPTY,
  startsClosed: ReadonlySet<string> = EMPTY,
): boolean {
  return ids.length > 0 && ids.every((id) => !isExpanded(collapsed, id, opened, startsClosed.has(id)));
}

/** One page's collapsed ids out of the raw storage blob. Tolerates missing,
 *  malformed, and wrong-shaped values by reading them as "nothing collapsed" —
 *  a corrupt entry costs a lost preference, never a crash.
 *
 *  Deliberately **not** reconciled against the sections that exist right now
 *  (unlike the rail's section ORDER, which drops unknown ids): a section hidden
 *  by a search filter must come back still collapsed, so an id we don't
 *  currently recognize is remembered rather than pruned. */
export function parseCollapsed(raw: string | null, pageKey: string): Set<string> {
  if (!raw) return new Set();
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return new Set();
    const entry = (parsed as Record<string, unknown>)[pageKey];
    if (!Array.isArray(entry)) return new Set();
    return new Set(entry.filter((v): v is string => typeof v === "string"));
  } catch {
    return new Set();
  }
}

/** Write one page's collapsed ids back into the raw storage blob, preserving
 *  every other page's entry. */
export function serializeCollapsed(
  raw: string | null,
  pageKey: string,
  collapsed: ReadonlySet<string>,
): string {
  let base: Record<string, unknown> = {};
  if (raw) {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        base = parsed as Record<string, unknown>;
      }
    } catch {
      // Malformed blob — start clean rather than lose this page's write too.
    }
  }
  return JSON.stringify({ ...base, [pageKey]: [...collapsed] });
}
