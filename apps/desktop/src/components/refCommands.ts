/// A ref's commands (`.context/commands.md` "Offering a command to a
/// person"): the commands about its kind (`ui.about`) — what its page's
/// nav-bar menu and a row's right-click menu offer — grouped by their
/// `ui.group`, each run the way the window runs any offer. The window
/// says how (`setRefOfferHost`, from `App`); until it has, there are none.
import { useCallback } from "react";

import { refOffers, type OfferContext, type OfferDeps } from "../commandOffers.js";
import type { MenuItem } from "../menu.js";
import { usePersonCommands } from "../personCommandsStore.js";
import type { CommandEntry } from "./quickOpenResults.js";

let host: { ctx: OfferContext; deps: OfferDeps } | null = null;

/** How the window runs an offer, and where it is (`App`). */
export function setRefOfferHost(next: { ctx: OfferContext; deps: OfferDeps } | null): void {
  host = next;
}

/** `ref`'s commands, by the person's command listing. */
export function useRefOffers(): (ref: string) => CommandEntry[] {
  const specs = usePersonCommands();
  return useCallback((ref: string) => (host ? refOffers(specs, ref, host.ctx, host.deps) : []), [specs]);
}

/** `entries` by group, in the order groups first appear. */
export function groupOffers(entries: CommandEntry[]): { group: string; entries: CommandEntry[] }[] {
  const out: { group: string; entries: CommandEntry[] }[] = [];
  for (const e of entries) {
    const g = out.find((x) => x.group === e.group);
    if (g) g.entries.push(e);
    else out.push({ group: e.group, entries: [e] });
  }
  return out;
}

/** A row menu's tail for a ref's `entries`: a separator, then one submenu
 *  per group. Empty when there's nothing to offer. */
export function refCommandMenuItems(entries: CommandEntry[]): MenuItem[] {
  if (entries.length === 0) return [];
  return [
    { id: "ref-commands-separator", label: "", enabled: false, separator: true },
    ...groupOffers(entries).map((g) => ({
      id: `ref-commands-${g.group}`,
      label: g.group,
      enabled: true,
      submenu: g.entries.map((e) => ({
        id: `ref-command-${e.id}`,
        label: e.label,
        enabled: e.enabled ?? true,
        run: e.run,
      })),
    })),
  ];
}
