/// Which stream a thread is in, for what the window does in a thread it
/// may not show (an agent's `oxplow.tab.*`, `clientHost.ts`).
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
