/// The page nav bar's ☰ menu, as data: everything a page can be acted on
/// with that isn't how it is navigated, in a fixed order — oxplow's own
/// first (Ask, then the standard actions, in their order), then the page's
/// and its extensions' own sorted by label (an action, or a submenu its
/// `submenu` / a command's `ui.group` names, merged by name), then Open
/// With, then the panels (Backlinks, Outbound, Pin to Dashboard) it opens
/// under the button. The primary and prominent actions are buttons in the
/// bar instead. A section with nothing in it is left out, and so is the
/// menu.
import type { PageAction } from "../kit/actions.js";
import type { MenuItem } from "../menu.js";
import type { CommandEntry } from "../components/quickOpenResults.js";

/// "Open With": the pages that render the shown URI, when there's more
/// than one, and which of them draws it now. Picking one writes the URI's
/// view (`#oxplow.renderer=<extension>/<page>`), so the same URI stays in
/// the tab and Back returns to how it was shown.
export interface OpenWithProps {
  /** The renderers, by name (`<extension>/<page>`), with their titles. */
  options: Array<{ name: string; title: string }>;
  /** The one drawing the page now. */
  current: string;
  pick(name: string): void;
}

/** What the ☰ opens under itself rather than as a menu. */
export type PagePanel = "backlinks" | "outbound" | "pin";

export interface PageMenuSource {
  actions?: readonly PageAction[];
  /** Ask About This and the catalog's prompts about the page's kind. */
  ask?: { prompts: string[]; askThis(): void; askPrompt(prompt: string): void };
  /** The commands about the page's resource. */
  commands?: Array<Pick<CommandEntry, "id" | "label" | "group" | "run" | "enabled">>;
  openWith?: OpenWithProps;
  /** How many pages link here / this page links to. */
  backlinks?: number;
  outbound?: number;
  /** The page can be pinned; with a reason, not now. */
  pin?: { disabledReason?: string | null };
  openPanel(panel: PagePanel): void;
}

/** The actions that stay buttons in the bar: the primary one and the
 *  prominent ones. */
export function barActions(actions: readonly PageAction[] | undefined): PageAction[] {
  return (actions ?? []).filter((a) => a.primary || a.prominent);
}

const separator = (id: string): MenuItem => ({ id, label: "", enabled: false, separator: true });

/** By label, as a person reads them: case and accents aside. */
const byLabel = (a: { label: string }, b: { label: string }) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" });

export function pageMenuItems(source: PageMenuSource): MenuItem[] {
  const sections: MenuItem[][] = [];
  const inMenu = (source.actions ?? []).filter((a) => !a.primary && !a.prominent);
  const item = (a: PageAction): MenuItem => ({
    id: `action-${a.id}`,
    label: a.busy ? `${a.label}…` : a.label,
    enabled: !a.disabled && !a.busy,
    icon: a.icon,
    run: a.run,
  });

  // oxplow's own: Ask, then the standard actions, always here and in
  // their own order, so they are in the same spot on every page.
  const standard: MenuItem[] = [];
  if (source.ask) {
    const ask = source.ask;
    standard.push({
      id: "page-ask",
      label: "Ask",
      enabled: true,
      icon: "message-square",
      submenuHeader: "Fills the agent's input; nothing is sent",
      submenu: [
        { id: "page-ask-this", label: "Ask About This", enabled: true, run: ask.askThis },
        ...ask.prompts.map((p, i) => ({ id: `page-ask-prompt-${i}`, label: p, enabled: true, run: () => ask.askPrompt(p) })),
      ],
    });
  }
  standard.push(...inMenu.filter((a) => a.standard).map(item));
  sections.push(standard);

  // The page's and its extensions' own, sorted by label: an action alone,
  // or a submenu (by its name) of the actions and commands that share it.
  const loose: MenuItem[] = [];
  const submenus = new Map<string, MenuItem[]>();
  const into = (name: string, entry: MenuItem) => {
    const list = submenus.get(name);
    if (list) list.push(entry);
    else submenus.set(name, [entry]);
  };
  for (const a of inMenu.filter((x) => !x.standard)) {
    if (a.submenu) into(a.submenu, item(a));
    else loose.push(item(a));
  }
  for (const e of source.commands ?? []) {
    into(e.group, { id: `ref-command-${e.id}`, label: e.label, enabled: e.enabled ?? true, run: e.run });
  }
  sections.push(
    [
      ...loose,
      ...[...submenus].map(([name, entries]) => ({
        id: `page-submenu-${name}`,
        label: name,
        enabled: true,
        submenu: entries.sort(byLabel),
      })),
    ].sort(byLabel),
  );

  const openWith = source.openWith;
  sections.push(
    openWith && openWith.options.length > 1
      ? [
          {
            id: "page-open-with",
            label: "Open With",
            enabled: true,
            submenu: openWith.options.map((o) => ({
              id: `page-open-with-${o.name}`,
              label: o.title,
              enabled: true,
              checked: o.name === openWith.current,
              run: () => openWith.pick(o.name),
            })),
          },
        ]
      : [],
  );

  const panels: MenuItem[] = [];
  if (source.backlinks !== undefined) {
    panels.push({
      id: "page-backlinks",
      label: `Backlinks (${source.backlinks})…`,
      enabled: source.backlinks > 0,
      run: () => source.openPanel("backlinks"),
    });
  }
  if (source.outbound !== undefined) {
    panels.push({
      id: "page-outbound",
      label: `Outbound (${source.outbound})…`,
      enabled: source.outbound > 0,
      run: () => source.openPanel("outbound"),
    });
  }
  if (source.pin) {
    const why = source.pin.disabledReason;
    panels.push({
      id: "page-pin",
      label: why ? `Pin to Dashboard… (${why})` : "Pin to Dashboard…",
      enabled: !why,
      run: () => source.openPanel("pin"),
    });
  }
  sections.push(panels);

  return sections
    .filter((s) => s.length > 0)
    .flatMap((s, i) => (i === 0 ? s : [separator(`page-menu-separator-${i}`), ...s]));
}
