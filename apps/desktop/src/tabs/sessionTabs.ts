/**
 * A thread's agent-session tabs follow its `agent_session` rows: every
 * open session's tab leads the thread's tabs, in the order the sessions
 * opened, and a closed one's goes. The rows are the truth — the tabs are
 * never a copy the UI keeps on its own (`.context/pages-and-tabs.md`).
 */
import { parseAgentChoice } from "../agentKinds.js";
import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { agentSessionRef, newSessionRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

/** `tabs` with `openSessions`' tabs leading (and only theirs). The session
 *  picker is an ordinary tab here: it opens for a new thread
 *  (`newThreadTabs`) and stays closed once closed. The same array when
 *  nothing changed. */
export function reconcileSessionTabs(tabs: TabRef[], openSessions: ReadonlyArray<{ id: string }>): TabRef[] {
  const rest = tabs.filter((t) => t.kind !== "agent_session");
  const leading = openSessions.map((s) => agentSessionRef(s.id));
  const next = [...leading, ...rest];
  const same = next.length === tabs.length && next.every((t, i) => t.id === tabs[i]!.id);
  return same ? tabs : next;
}

/** What a new thread opens with, by the person's `newThreadSession`: `ask`
 *  opens the session picker, `none` nothing, and an agent
 *  (`<harness>` / `<harness>:<acp agent>`) starts that agent's session —
 *  unless it can't start any more (disabled, gone, or a chat harness
 *  without its agent), when it asks instead. */
export function newThreadTabs(
  setting: string,
  harnesses: ReadonlyArray<HarnessListing>,
): { tabs: TabRef[]; start: { harness: string; acpAgent: string | null } | null } {
  const ask = { tabs: [newSessionRef()], start: null };
  if (setting === "none") return { tabs: [], start: null };
  if (setting === "ask") return ask;
  const start = parseAgentChoice(setting);
  const harness = harnesses.find((h) => h.id === start.harness && h.enabled);
  if (!harness || harness.chat !== (start.acpAgent !== null)) return ask;
  return { tabs: [], start };
}
