import type { MenuItem } from "../menu.js";
import { current, parseSession } from "../tabs/session.js";
import type { TabRef } from "../tabs/tabState.js";
import type { OtherDeviceTabs } from "../tauri-bridge/generated/bindings.js";

/** What Continue from a device opens: its page tabs of the thread, in its
 *  order, the one it shows last so it is shown here too. Its agent
 *  sessions (pinned) are left out: they show on every device already. A
 *  session that doesn't parse opens nothing. Pure. */
export function continueRefs(sessionJson: string): TabRef[] {
  const s = parseSession(sessionJson);
  if (!s) return [];
  const pages = s.tabs.filter((t) => !t.pinned);
  const shown = pages.find((t) => t.slot === s.active);
  return [...pages.filter((t) => t !== shown), ...(shown ? [shown] : [])].map((t) => current(t).ref);
}

/** One menu item per other device with pages open in the thread: its name
 *  (or "Another device"), opening its pages here. The two devices share
 *  nothing after: this reads the other's place once. */
export function continueItems(others: readonly OtherDeviceTabs[], open: (ref: TabRef) => void): MenuItem[] {
  return others.flatMap((o) => {
    const refs = continueRefs(o.sessionJson);
    if (refs.length === 0) return [];
    return [
      {
        id: `continue.${o.device}`,
        label: o.name ?? "Another device",
        enabled: true,
        run: () => {
          for (const ref of refs) open(ref);
        },
      },
    ];
  });
}
