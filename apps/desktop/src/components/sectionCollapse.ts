// Pure state for the collapsible page sections. React-free
// so the rules are unit-testable without mounting a page.
//
// The state is stored as the set of **collapsed** ids, not expanded ones: a
// section defaults to expanded, so "collapsed" is the exception worth recording
// and a brand-new section needs no migration to appear open.

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
