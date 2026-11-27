import type { MenuGroup, MenuItem } from "../menu.js";

/** The menu bar's groups (`menuBar.ts`) as one menu, for a shell with no
 *  menu bar: each group a submenu, without the OS's own edit items
 *  (`native.*`: the OS runs them, never a tap) and the separators they leave
 *  at an end or doubled; a group left empty is gone. Pure. */
export function shellMenu(groups: readonly MenuGroup[]): MenuItem[] {
  return groups.flatMap((g) => {
    const items = tidy(g.items.filter((i) => !i.id.startsWith("native.")));
    return items.length === 0 ? [] : [{ id: `shell.menu.${g.id}`, label: g.label, enabled: true, submenu: items }];
  });
}

function tidy(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const i of items) {
    if (i.separator && (out.length === 0 || out[out.length - 1]!.separator)) continue;
    out.push(i);
  }
  while (out.length > 0 && out[out.length - 1]!.separator) out.pop();
  return out;
}

/** The menu bar's groups with the layout choice at the end of View (on a
 *  device that has one to make). */
export function withLayouts<G extends MenuGroup>(groups: G[], layouts: MenuItem[]): G[] {
  if (layouts.length === 0) return groups;
  return groups.map((g) => (g.id === "view" ? { ...g, items: [...g.items, { id: "view.layout-sep", label: "", enabled: false, separator: true }, ...layouts] } : g));
}
