/// A ref's commands (`.context/commands.md` "Offering a command to a
/// person"): the commands about its kind (`ui.about`) whose `ui.about_when`
/// holds on its row — what its page's nav-bar menu and a row's right-click
/// menu offer, the daemon's answer (`ref_commands`) asked when a menu
/// opens — each run the way the window runs any offer. A row menu groups
/// them by their `ui.group`. The window says how (`setRefOfferHost`, from
/// `App`); until it has, there are none.
import { useCallback } from "react";

import { refCommands } from "../api.js";
import { refOffers, type OfferContext, type OfferDeps } from "../commandOffers.js";
import type { MenuItem } from "../menu.js";
import { recordOpError } from "../opErrors.js";
import { personCommandsNow, usePersonCommands } from "../personCommandsStore.js";
import type { CommandEntry } from "./quickOpenResults.js";
import type { CommandSpec } from "../tauri-bridge/generated/bindings.js";

let host: { ctx: OfferContext; deps: OfferDeps } | null = null;

/** How the window runs an offer, and where it is (`App`). */
export function setRefOfferHost(next: { ctx: OfferContext; deps: OfferDeps } | null): void {
  host = next;
}

/** The window's offer host, for what draws offers outside a menu (an
 *  action point); `null` until `App` says how. */
export function refOfferHost(): { ctx: OfferContext; deps: OfferDeps } | null {
  return host;
}

/** `ref`'s commands as of now, from `specs` (the person's command
 *  listing, which says how each is run): the daemon says which apply to
 *  it. Asked as a menu opens, so a ref's state (a session closed a moment
 *  ago) is current. */
async function offersWith(specs: CommandSpec[], ref: string): Promise<CommandEntry[]> {
  if (!host) return [];
  // A menu without them still opens, and the failure is reported.
  const applies = await refCommands(ref).catch((e: unknown) => {
    recordOpError({ label: "Load commands for a ref", message: e instanceof Error ? e.message : String(e) });
    return [];
  });
  return refOffers(specs, ref, applies, host.ctx, host.deps);
}

/** `ref`'s commands as of now (`offersWith`), for a component. */
export function useRefOffers(): (ref: string) => Promise<CommandEntry[]> {
  const specs = usePersonCommands();
  return useCallback((ref: string) => offersWith(specs, ref), [specs]);
}

/** `ref`'s commands as of now, outside React (a menu built as it opens). */
export function offersForRef(ref: string): Promise<CommandEntry[]> {
  return offersWith(personCommandsNow(), ref);
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
