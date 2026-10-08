/**
 * A thread's agent-session tabs follow its `agent_session` rows: every
 * open session's tab leads the thread's tabs, in the order the sessions
 * opened, and a closed one's goes. The rows are the truth — the tabs are
 * never a copy the UI keeps on its own (`.context/pages-and-tabs.md`).
 */
import { agentSessionRef, newSessionRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

/** `tabs` with `openSessions`' tabs leading (and only theirs), and — for a
 *  thread with no session — the session picker first. The same array when
 *  nothing changed. */
export function reconcileSessionTabs(tabs: TabRef[], openSessions: ReadonlyArray<{ id: string }>): TabRef[] {
  const rest = tabs.filter((t) => t.kind !== "agent_session");
  const leading = openSessions.map((s) => agentSessionRef(s.id));
  const picker = openSessions.length === 0 && !rest.some((t) => t.kind === "new-session") ? [newSessionRef()] : [];
  const next = [...picker, ...leading, ...rest];
  const same = next.length === tabs.length && next.every((t, i) => t.id === tabs[i]!.id);
  return same ? tabs : next;
}
