/// A thread's tabs changed by id, whichever thread the window shows — what
/// an agent's `oxplow.tab.*` does in its own thread (`clientHost.ts`).
/// Pure over the per-thread tab lists `useThreadPageTabs` owns.
import type { TabRef } from "./tabState.js";

/** `tabs` with `ref` among `threadId`'s (appended); the same object when
 *  it's already there. */
export function withTab(tabs: Record<string, TabRef[]>, threadId: string, ref: TabRef): Record<string, TabRef[]> {
  const existing = tabs[threadId] ?? [];
  if (existing.some((t) => t.id === ref.id)) return tabs;
  return { ...tabs, [threadId]: [...existing, ref] };
}

/** `tabs` without tab `id` among `threadId`'s; the same object when it
 *  isn't there. */
export function withoutTab(tabs: Record<string, TabRef[]>, threadId: string, id: string): Record<string, TabRef[]> {
  const existing = tabs[threadId] ?? [];
  if (!existing.some((t) => t.id === id)) return tabs;
  return { ...tabs, [threadId]: existing.filter((t) => t.id !== id) };
}

/** The stream thread `threadId` is in, by the window's thread lists. */
export function streamOfThread(
  threadStates: Record<string, { threads: Array<{ id: string }> } | undefined>,
  threadId: string,
): string | null {
  for (const [streamId, state] of Object.entries(threadStates)) {
    if (state?.threads.some((t) => t.id === threadId)) return streamId;
  }
  return null;
}
