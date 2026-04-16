import type { PaneKind } from "../persistence/stream-store.js";

type ResumeUpdate =
  | { type: "set"; sessionId: string }
  | { type: "clear" };

interface PaneState {
  launchedWithResume: boolean;
  sawSessionStart: boolean;
}

export class ResumeTracker {
  private panes = new Map<string, PaneState>();

  noteSessionLaunch(sessionKey: string, launchedWithResume: boolean): void {
    this.panes.set(sessionKey, {
      launchedWithResume,
      sawSessionStart: false,
    });
  }

  notePaneLaunch(streamId: string, pane: PaneKind, launchedWithResume: boolean): void {
    this.noteSessionLaunch(key(streamId, pane), launchedWithResume);
  }

  recordSessionHookEvent(
    sessionKey: string,
    eventName: string,
    sessionId?: string,
  ): ResumeUpdate | null {
    if (!sessionId) return null;
    const state = this.panes.get(sessionKey);

    if (eventName === "SessionStart") {
      this.panes.set(sessionKey, {
        launchedWithResume: state?.launchedWithResume ?? false,
        sawSessionStart: true,
      });
      return { type: "set", sessionId };
    }

    if (eventName === "SessionEnd" && state?.launchedWithResume && !state.sawSessionStart) {
      this.panes.delete(sessionKey);
      return { type: "clear" };
    }

    return null;
  }

  recordHookEvent(
    streamId: string,
    pane: PaneKind,
    eventName: string,
    sessionId?: string,
  ): ResumeUpdate | null {
    return this.recordSessionHookEvent(key(streamId, pane), eventName, sessionId);
  }
}

function key(streamId: string, pane: PaneKind): string {
  return `${streamId}:${pane}`;
}
