import type { PaneKind } from "./stream-store.js";

type ResumeUpdate =
  | { type: "set"; sessionId: string }
  | { type: "clear" };

interface PaneState {
  launchedWithResume: boolean;
  sawSessionStart: boolean;
}

export class ResumeTracker {
  private panes = new Map<string, PaneState>();

  notePaneLaunch(streamId: string, pane: PaneKind, launchedWithResume: boolean): void {
    this.panes.set(key(streamId, pane), {
      launchedWithResume,
      sawSessionStart: false,
    });
  }

  recordHookEvent(
    streamId: string,
    pane: PaneKind,
    eventName: string,
    sessionId?: string,
  ): ResumeUpdate | null {
    if (!sessionId) return null;
    const paneKey = key(streamId, pane);
    const state = this.panes.get(paneKey);

    if (eventName === "SessionStart") {
      this.panes.set(paneKey, {
        launchedWithResume: state?.launchedWithResume ?? false,
        sawSessionStart: true,
      });
      return { type: "set", sessionId };
    }

    if (eventName === "SessionEnd" && state?.launchedWithResume && !state.sawSessionStart) {
      this.panes.delete(paneKey);
      return { type: "clear" };
    }

    return null;
  }
}

function key(streamId: string, pane: PaneKind): string {
  return `${streamId}:${pane}`;
}
