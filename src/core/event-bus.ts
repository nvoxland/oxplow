import type { Logger } from "./logger.js";

export type WorkspaceChangeKind = "created" | "updated" | "deleted";
export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";
export type ThreadLifecycleKind = "created" | "selected" | "reordered" | "promoted" | "closed" | "reopened" | "resume-updated" | "summary-updated" | "renamed" | "prompt-changed";
export type AgentStatus = "working" | "waiting";
export type PaneKind = "working" | "talking";

export interface WorkspaceChangedEvent {
  type: "workspace.changed";
  id: number;
  streamId: string;
  kind: WorkspaceChangeKind;
  path: string;
  t: number;
}

export interface HookRecordedEvent {
  type: "hook.recorded";
  streamId: string;
  threadId?: string;
  pane?: PaneKind;
  // Kept opaque at the bus level; the UI re-hydrates via the `StoredEvent`
  // type exported from the hook-ingest module. Using `unknown` here avoids a
  // cross-module import cycle (event-bus is in core/).
  event: unknown;
}

export interface WorkItemChangedEvent {
  type: "work-item.changed";
  streamId: string;
  threadId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export interface BacklogChangedEvent {
  type: "backlog.changed";
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export interface ThreadChangedEvent {
  type: "thread.changed";
  streamId: string;
  threadId: string;
  kind: ThreadLifecycleKind;
}

export interface AgentStatusChangedEvent {
  type: "agent-status.changed";
  streamId: string;
  threadId: string;
  status: AgentStatus;
}

export interface WorkspaceContextChangedEvent {
  type: "workspace-context.changed";
  gitEnabled: boolean;
}

export interface GitRefsChangedEvent {
  type: "git-refs.changed";
  streamId: string;
  t: number;
}

export interface StreamChangedEvent {
  type: "stream.changed";
  kind: "reordered" | "prompt-changed" | "branch-changed";
  streamId?: string;
}

export interface ConfigChangedEvent {
  type: "config.changed";
}

export interface WikiNoteChangedEvent {
  type: "wiki-note.changed";
  kind: "upserted" | "deleted";
  slug: string | null;
}

export interface FollowupChangedEvent {
  type: "followup.changed";
  threadId: string;
  kind: "added" | "removed" | "cleared";
  id: string | null;
}

export interface BackgroundTaskChangedEvent {
  type: "background-task.changed";
  kind: "started" | "updated" | "ended";
  id: string;
}

export interface UsageRecordedEvent {
  type: "usage.recorded";
  kind: string;
  key: string;
  streamId: string | null;
  threadId: string | null;
}

export interface CodeQualityScannedEvent {
  type: "code-quality.scanned";
  streamId: string;
  scanId: number;
  tool: "lizard" | "jscpd";
  scope: "codebase" | "diff";
  status: "running" | "completed" | "failed";
}

export interface FileSnapshotCreatedEvent {
  type: "file-snapshot.created";
  streamId: string;
  snapshotId: string;
  kind: "task-start" | "task-end" | "task-event" | "startup";
  effortId: string | null;
  threadId: string | null;
}

export type OxplowEvent =
  | WorkspaceChangedEvent
  | HookRecordedEvent
  | WorkItemChangedEvent
  | BacklogChangedEvent
  | ThreadChangedEvent
  | AgentStatusChangedEvent
  | FileSnapshotCreatedEvent
  | WorkspaceContextChangedEvent
  | GitRefsChangedEvent
  | StreamChangedEvent
  | ConfigChangedEvent
  | WikiNoteChangedEvent
  | UsageRecordedEvent
  | CodeQualityScannedEvent
  | FollowupChangedEvent
  | BackgroundTaskChangedEvent;

export type OxplowEventType = OxplowEvent["type"];

export type OxplowEventOf<T extends OxplowEventType> = Extract<OxplowEvent, { type: T }>;

export class EventBus {
  private readonly listeners = new Set<(event: OxplowEvent) => void>();

  constructor(private readonly logger?: Logger) {}

  publish(event: OxplowEvent): void {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (error) {
        this.logger?.warn("event bus listener threw", {
          type: event.type,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  subscribe(listener: (event: OxplowEvent) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  subscribeByType<T extends OxplowEventType>(
    type: T,
    listener: (event: OxplowEventOf<T>) => void,
  ): () => void {
    return this.subscribe((event) => {
      if (event.type === type) listener(event as OxplowEventOf<T>);
    });
  }
}
