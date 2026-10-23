import type { Dispatch, SetStateAction } from "react";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { TabRef } from "../tabs/tabState.js";
import type { AgentStatus, Stream, ThreadState } from "../api.js";
import type { useAgentHarnesses } from "../useAgentHarnesses.js";
import type { EditorNavigationTarget } from "../lsp.js";
import type { Origin } from "../tabs/session.js";

/** Navigation bound to one tab slot: in-tab navigation mutates the slot. */
export type SlotNav = {
  /** The slot's current ref id. */
  slotId: string;
  navOpen: (newRef: TabRef, opts?: { newTab?: boolean; origin?: Origin }) => void;
  navOpenFile: (path: string, opts?: { newTab?: boolean }) => void;
  navOpenDiff: (spec: DiffSpec, origin?: Origin) => void;
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
  agentStatus: AgentStatus;
  harnesses: ReturnType<typeof useAgentHarnesses>;
  /** The tab shown in the center now (a page uses it to know it's visible). */
  effectiveCenterActive: string;
  setCenterActive: (next: string | ((prev: string) => string)) => void;
  /** Whether the workspace is under version control. */
  vcsEnabled: boolean;
  /** The file selected in the current session. */
  selectedFilePath: string | null;
  /** Opens a page in a new tab. */
  handleOpenPage: (ref: TabRef) => void;
  /** Navigates the slot with this tab id to `ref`. */
  handleNavigateInTab: (currentTabId: string, ref: TabRef, origin?: Origin, replace?: boolean) => void;
  closePageTab: (id: string) => void;
  handleOpenFile: (path: string) => Promise<void>;
  handleNavigateToLocation: (target: EditorNavigationTarget) => Promise<void>;
  handleStreamCreated: (next: Stream) => Promise<void>;
}
