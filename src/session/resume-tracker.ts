// Claude Code silently drops HTTP hooks for `SessionStart` ("HTTP hooks are
// not supported for SessionStart" appears in its debug log). So we can't
// learn a session id from that hook — instead we adopt whichever session id
// shows up on *any* hook that carries one (UserPromptSubmit, PreToolUse,
// Stop, SessionEnd, …). `decideResumeUpdate` returns a directive when the
// observed id is new or different from the one already persisted for the
// thread, and null otherwise so callers avoid a no-op DB write.
export type ResumeUpdate = { type: "set"; sessionId: string };

export function decideResumeUpdate(
  currentResumeId: string,
  observedSessionId: string | undefined,
): ResumeUpdate | null {
  if (!observedSessionId) return null;
  if (observedSessionId === currentResumeId) return null;
  return { type: "set", sessionId: observedSessionId };
}
