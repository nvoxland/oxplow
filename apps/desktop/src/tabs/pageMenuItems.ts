/// The page nav bar's ☰ menu, as data: everything a page can be acted on
/// with that isn't how it is navigated — the page's own actions (bar a
/// primary and prominent ones, which stay buttons), Ask, the commands about its
/// resource (their `ui.about`, one submenu per `ui.group`), Open With, and
/// the panels (Backlinks, Outbound, Pin to Dashboard) it opens under the
/// button. A section with nothing in it is left out, and so is the menu.
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

export function pageMenuItems(source: PageMenuSource): MenuItem[] {
  const sections: MenuItem[][] = [];

  sections.push(
    (source.actions ?? [])
      .filter((a) => !a.primary && !a.prominent)
      .map((a) => ({
        id: `action-${a.id}`,
        label: a.busy ? `${a.label}…` : a.label,
        enabled: !a.disabled && !a.busy,
        run: a.run,
      })),
  );

  const extension: MenuItem[] = [];
  if (source.ask) {
    const ask = source.ask;
    extension.push({
      id: "page-ask",
      label: "Ask",
      enabled: true,
      submenuHeader: "Fills the agent's input; nothing is sent",
      submenu: [
        { id: "page-ask-this", label: "Ask About This", enabled: true, run: ask.askThis },
        ...ask.prompts.map((p, i) => ({ id: `page-ask-prompt-${i}`, label: p, enabled: true, run: () => ask.askPrompt(p) })),
      ],
    });
  }
  const groups: Array<{ group: string; entries: NonNullable<PageMenuSource["commands"]> }> = [];
  for (const e of source.commands ?? []) {
    const g = groups.find((x) => x.group === e.group);
    if (g) g.entries.push(e);
    else groups.push({ group: e.group, entries: [e] });
  }
  for (const g of groups) {
    extension.push({
      id: `ref-commands-${g.group}`,
      label: g.group,
      enabled: true,
      submenu: g.entries.map((e) => ({ id: `ref-command-${e.id}`, label: e.label, enabled: e.enabled ?? true, run: e.run })),
    });
  }
  sections.push(extension);

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
