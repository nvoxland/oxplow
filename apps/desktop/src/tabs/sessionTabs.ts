import { typeOf } from "../uri/uri.js";
/**
 * A thread's agent-session tabs follow its `agent_session` rows: every
 * open session's tab leads the thread's tabs, in the order the sessions
 * opened, and a closed one's goes. The rows are the truth — the tabs are
 * never a copy the UI keeps on its own (`.context/pages-and-tabs.md`).
 */
import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { agentSessionRef, newSessionRef } from "./pageRefs.js";
import { refsOf, type Session } from "./session.js";
import type { TabRef } from "./tabState.js";

/** `tabs` with `openSessions`' tabs leading (and only theirs). The session
 *  picker is an ordinary tab here: it opens for a new thread
 *  (`newThreadTabs`) and stays closed once closed. The same array when
 *  nothing changed. */
export function reconcileSessionTabs(tabs: TabRef[], openSessions: ReadonlyArray<{ id: string }>): TabRef[] {
  const rest = tabs.filter((t) => typeOf(t.id) !== "oxplow/agent_session");
  const leading = openSessions.map((s) => agentSessionRef(s.id));
  const next = [...leading, ...rest];
  const same = next.length === tabs.length && next.every((t, i) => t.id === tabs[i]!.id);
  return same ? tabs : next;
}

/** What a new thread opens with, by the person's `newThreadSession`: `ask`
 *  opens the session picker, `none` nothing, and an agent
 *  (a harness's key) starts that agent's session — unless it can't start
 *  any more (disabled, or gone), when it asks instead. */
export function newThreadTabs(
  setting: string,
  harnesses: ReadonlyArray<HarnessListing>,
): { tabs: TabRef[]; start: { harness: string } | null } {
  const ask = { tabs: [newSessionRef()], start: null };
  if (setting === "none") return { tabs: [], start: null };
  if (setting === "ask") return ask;
  const harness = harnesses.find((h) => h.id === setting && h.enabled);
  return harness ? { tabs: [], start: { harness: harness.id } } : ask;
}

/** Whether a thread's stored tabs (`tabs/useTabSessions.ts`) are left from a
 *  thread that is gone. The browser keeps them across a reset of the project,
 *  and a new thread can get an old one's id; tabs of agent sessions, none of
 *  which the thread has open now, then name sessions that can't load. An entry
 *  with no agent-session tab is not judged (a thread whose sessions were all
 *  closed keeps its page tabs). Asked once, when the window first sees the
 *  thread's open sessions: later, a closed session's tab is `reconcileSessionTabs`'s. */
export function storedSessionIsStale(stored: Session, openSessions: ReadonlyArray<{ id: string }>): boolean {
  const named = refsOf(stored).filter((t) => typeOf(t.id) === "oxplow/agent_session");
  if (named.length === 0) return false;
  return !named.some((t) => openSessions.some((s) => agentSessionRef(s.id).id === t.id));
}
