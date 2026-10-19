import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { TabRef } from "../tabs/tabState.js";
import type { AgentStatus, Stream, ThreadState } from "../api.js";
import type { ItemChange, NewWorkItem, WorkList } from "../workItems.js";
import type { useAgentHarnesses } from "../useAgentHarnesses.js";
import type { useWorkListProfile } from "../useWorkListProfile.js";
import type { EditorNavigationTarget } from "../lsp.js";
import type { NavSiblings } from "../tabs/PageNavigationContext.js";

/** Navigation bound to one tab slot: in-tab navigation mutates the slot. */
export type SlotNav = {
  /** The slot's current ref id. */
  slotId: string;
  navOpen: (newRef: TabRef, opts?: { newTab?: boolean; siblings?: NavSiblings }) => void;
  navOpenFile: (path: string, opts?: { newTab?: boolean }) => void;
  navOpenDiff: (spec: DiffSpec, siblings?: NavSiblings) => void;
  navRevealCommit: (sha: string) => void;
};

/**
 * What core offers the built-in pages (`pages/builtInPages.tsx`): the
 * selection, the work state and the handlers their bodies use, and
 * nothing else. A page reads from here, never from App's locals.
 */
export interface AppScope {
  /** The selected stream. */
  stream: Stream | null;
  streams: Stream[];
  setStreams: Dispatch<SetStateAction<Stream[]>>;
  /** The selected stream's threads. */
  threads: ThreadState["threads"];
  setThreadStates: Dispatch<SetStateAction<Record<string, ThreadState>>>;
  /** The selected thread, `null` when none. */
  thread: ThreadState["threads"][number] | null;
  threadId: string | null;
  threadWork: WorkList | null;
  agentStatus: AgentStatus;
  /** The backlog, `null` until it loads. */
  backlog: WorkList | null;
  workListProfile: ReturnType<typeof useWorkListProfile>;
  /** Registers the callback that opens the task list's create row. */
  planOpenCreateRef: MutableRefObject<(() => void) | null>;
  harnesses: ReturnType<typeof useAgentHarnesses>;
  /** The diff-spec registry, indexed by tab id. */
  diffTabs: Array<{ id: string; spec: DiffSpec }>;
  /** The tab shown in the center now (a page uses it to know it's visible). */
  effectiveCenterActive: string;
  setCenterActive: (next: string | ((prev: string) => string)) => void;
  /** Whether the workspace is under version control. */
  vcsEnabled: boolean;
  /** The file selected in the current session. */
  selectedFilePath: string | null;
  /** Paths the Files page marks as generated. */
  generatedExclude: string[];
  /** Counter that asks the Files page to open its commit flow. */
  commitFilesRequest: number;
  /** Opens a page in a new tab. */
  handleOpenPage: (ref: TabRef) => void;
  /** Navigates the slot with this tab id to `ref`. */
  handleNavigateInTab: (currentTabId: string, ref: TabRef, siblings?: NavSiblings, replace?: boolean) => void;
  closePageTab: (id: string) => void;
  handleOpenFile: (path: string) => Promise<void>;
  handleNavigateToLocation: (target: EditorNavigationTarget) => Promise<void>;
  handleStreamCreated: (next: Stream) => Promise<void>;
  handleCreateFile: (path: string) => Promise<void>;
  handleCreateDirectory: (path: string) => Promise<void>;
  handleRenamePath: (fromPath: string, toPath: string) => Promise<void>;
  handleDeletePath: (path: string) => Promise<void>;
  handleToggleGenerated: (entry: string, mark: boolean) => Promise<void>;
  handleCreateTask: (input: NewWorkItem) => Promise<void>;
  handleUpdateTask: (ref: string, change: ItemChange) => Promise<void>;
  handleDeleteTask: (ref: string) => Promise<void>;
  handleReorderTasks: (orderedRefs: string[]) => Promise<void>;
  handleReorderBacklog: (orderedRefs: string[]) => Promise<void>;
  handleMoveItemToBacklog: (ref: string) => Promise<void>;
}
