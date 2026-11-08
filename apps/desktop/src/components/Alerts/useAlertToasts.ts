/// A toast as each new thing needs the person: a proposal or a
/// failed operation as it arrives, delivery or a reaction as it starts
/// failing, a badge as it starts firing. Once per item; its only action
/// is Review (the Alerts page, or a badge's page) — never Approve: a
/// decision is made where its preview is. What's already there in the first moments after the
/// app opens is taken as seen, not announced.

import { useEffect, useRef } from "react";

import { inlineClaims, type InlineClaims } from "../../commands/actionFailures.js";
import { showToast } from "../toastStore.js";
import { alertKeys, newAlerts, type AlertItems } from "./alertsModel.js";

/** How long after mount what loads counts as already there. */
export const SETTLE_MS = 3000;

/** `open`: the URI a badge opens (its page), when the alert is one. A
 *  failed operation shown beside the control that ran it (`claims`,
 *  `commands/runForPerson.ts`) is not toasted: the toast is for one whose
 *  control is gone. */
export function useAlertToasts(
  items: AlertItems,
  onReview: (open?: string) => void,
  now: () => number = Date.now,
  claims: InlineClaims = inlineClaims,
) {
  const mounted = useRef(now());
  const seen = useRef<Set<string>>(new Set());
  const review = useRef(onReview);
  review.current = onReview;
  useEffect(() => {
    if (now() - mounted.current < SETTLE_MS) {
      for (const k of alertKeys(items)) seen.current.add(k);
      return;
    }
    for (const a of newAlerts(seen.current, items)) {
      seen.current.add(a.key);
      const toast = () => showToast({ message: a.message, actionLabel: "Review", onUndo: () => review.current(a.open) });
      const op = a.key.startsWith("op:") ? a.key.slice(3) : null;
      if (op === null) toast();
      else void claims.settled().then(() => (claims.shown(op) ? undefined : toast()));
    }
    // A key that's gone (a proposal decided, delivery recovered) may be
    // announced again if it comes back.
    seen.current = new Set([...seen.current].filter((k) => alertKeys(items).has(k)));
  }, [items, now, claims]);
}
