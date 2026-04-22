import type { Logger } from "./logger.js";

export type WorkspaceChangeKind = "created" | "updated" | "deleted";
export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";
export type ThreadLifecycleKind = "created" | "selected" | "reordered" | "promoted" | "completed" | "resume-updated" | "summary-updated" | "renamed" | "auto-commit-changed" | "auto-committed" | "prompt-changed";
export type AgentStatus = "idle" | "working" | "waiting" | "done";
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

export interface TurnChangedEvent {
  type: "turn.changed";
  streamId: string;
  threadId: string;
  turnId: string;
  kind: "opened" | "closed";
}

export interface WorkspaceContextChangedEvent {
  type: "workspace-context.changed";
  gitEnabled: boolean;
}

export interface CommitPointChangedEvent {
  type: "commit-point.changed";
  streamId: string | null;
  threadId: string;
  id: string | null;
  kind: "created" | "updated" | "deleted" | "reordered";
}

export interface GitRefsChangedEvent {
  type: "git-refs.changed";
  streamId: string;
  t: number;
}

export interface StreamChangedEvent {
  type: "stream.changed";
  kind: "reordered" | "prompt-changed";
  streamId?: string;
}

export interface ConfigChangedEvent {
  type: "config.changed";
}

export interface WaitPointChangedEvent {
  type: "wait-point.changed";
  streamId: string | null;
  threadId: string;
  id: string | null;
  kind: "created" | "updated" | "deleted";
}

export interface FileSnapshotCreatedEvent {
  type: "file-snapshot.created";
  streamId: string;
  snapshotId: string;
  kind: "task-start" | "task-end" | "turn-start" | "turn-end" | "startup";
  turnId: string | null;
  threadId: string | null;
}

export type NewdeEvent =
  | WorkspaceChangedEvent
  | HookRecordedEvent
  | WorkItemChangedEvent
  | BacklogChangedEvent
  | ThreadChangedEvent
  | AgentStatusChangedEvent
  | TurnChangedEvent
  | FileSnapshotCreatedEvent
  | WorkspaceContextChangedEvent
  | CommitPointChangedEvent
  | WaitPointChangedEvent
  | GitRefsChangedEvent
  | StreamChangedEvent
  | ConfigChangedEvent;

export type NewdeEventType = NewdeEvent["type"];

export type NewdeEventOf<T extends NewdeEventType> = Extract<NewdeEvent, { type: T }>;

export class EventBus {
  private readonly listeners = new Set<(event: NewdeEvent) => void>();

  constructor(private readonly logger?: Logger) {}

  publish(event: NewdeEvent): void {
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

  subscribe(listener: (event: NewdeEvent) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  subscribeByType<T extends NewdeEventType>(
    type: T,
    listener: (event: NewdeEventOf<T>) => void,
  ): () => void {
    return this.subscribe((event) => {
      if (event.type === type) listener(event as NewdeEventOf<T>);
    });
  }
}
