/// Everything that needs the person, live: proposals, failed
/// operations, undelivered events and failed reactions, and firing panel
/// badges. The bell, the Alerts page and the toasts all read it.

import { useMemo, useSyncExternalStore } from "react";

import { useFailedReactions, useUndelivered } from "../../delivery.js";
import { useProposals } from "../../proposals.js";
import { usePanelRuns } from "../Panels/PanelRunsContext.js";
import { getOpErrorsStore } from "../opErrorsStore.js";
import type { AlertItems } from "./alertsModel.js";

export function useAlerts() {
  const proposals = useProposals();
  const store = getOpErrorsStore();
  const opErrors = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const undelivered = useUndelivered();
  const failedReactions = useFailedReactions();
  const { alerts: badges } = usePanelRuns();
  const items: AlertItems = useMemo(
    () => ({
      proposals: proposals.map((p) => ({
        id: p.ref,
        title: `${p.threadTitle ? `“${p.threadTitle}”` : "An agent"} wants to run ${p.command}`,
      })),
      opErrors: opErrors.map((e) => ({ id: e.id, label: e.label })),
      undelivered: undelivered.length,
      failedReactions: failedReactions.length,
      badges: badges.map((b) => ({ id: b.id, title: b.title, message: b.message })),
    }),
    [proposals, opErrors, undelivered, failedReactions, badges],
  );
  return { items, proposals, opErrors, badges };
}
